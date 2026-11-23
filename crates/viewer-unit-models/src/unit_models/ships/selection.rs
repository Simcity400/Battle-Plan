
use super::*;
use content_reader_parser::parser::{c_string, text};
mod pass;
pub(super) use pass::Pass;
#[cfg(test)]
use std::sync::Arc;

pub(super) trait Names<'rows> {
    fn name<'tag>(
        &mut self,
        tag: impl FnOnce() -> Result<&'tag str, String>,
        row: &'rows replay_data::rows::ShipModelRow,
        graphics: &[u8],
        definition: Option<(u32, u32)>,
    ) -> Result<Option<String>, String>;
}

pub(super) fn name<'t>(
    assets: &UnitModelAssets,
    tag: impl FnOnce() -> Result<&'t str, String>,
    row: &replay_data::rows::ShipModelRow,
    graphics: &[u8],
    definition: Option<(u32, u32)>,
    hour: Option<&Hour<'_>>,
) -> Result<Option<String>, String> {
    name_with_generated(assets, tag, row, graphics, || {
        generated(assets, row, definition, hour)
    })
}

fn name_with_generated<'t>(
    assets: &UnitModelAssets,
    tag: impl FnOnce() -> Result<&'t str, String>,
    row: &replay_data::rows::ShipModelRow,
    graphics: &[u8],
    generated: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    choose(row, graphics, tag, generated, |name| {
        if let Some(reason) = assets.roster.unreadable_names.get(name) {
            return Err(format!(
                "naval_entity: selected {name} is unreadable: {reason}"
            ));
        }
        Ok(assets.registered(name))
    })
}

fn generated(
    assets: &UnitModelAssets,
    row: &replay_data::rows::ShipModelRow,
    definition: Option<(u32, u32)>,
    hour: Option<&Hour<'_>>,
) -> Result<Option<String>, String> {
    let hour = hour.ok_or("naval_entity: hourly graphics inputs are unavailable")?;
    let variant = hour
        .variants
        .get(row.variant)
        .ok_or_else(|| format!("naval_entity: variant {} is unavailable", row.variant))?;
    let country = creator_country(variant.creator_link)?;
    let (owner, token) =
        definition.ok_or("naval_entity: ship's roster owner or Ships.class_id is unavailable")?;
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
    let requirements = assets
        .sub_units
        .requirements(identity, subunit, &hour.sources)
        .map_err(|error| format!("naval_entity: {error}"))?;
    let first = requirements.first().map(|(key, _)| key.as_str());
    let mut effective = subunit.clone();
    effective.category = row.formation_flags;
    if first.is_some() {
        effective.sprite = identifier(&row.fallback_sprite, "embedded sub-unit sprite")?.to_owned();
    }
    let pair = SelectedPair {
        variant,
        subunit: &effective,
        identity,
    };
    graphics_model_for_country(assets, hour, pair, first, country)
        .map(|model| model.map(|(_, name)| name))
}

fn creator_country(raw: u32) -> Result<u32, String> {
    match raw as i32 {
        1.. => Ok(raw),
        0 => Err("naval_entity: generated variant model needs unsupported null-country ideology/culture operands".into()),
        _ => Err(format!("naval_entity: generated variant model has unsafe signed creator country {}", raw as i32)),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::ShipCounterInputs, rows::ShipModelRow};

    fn valid() -> ShipModelRow {
        ShipModelRow {
            variant: 7,
            counter: Some(ShipCounterInputs {
                definition_valid: 1,
                ..Default::default()
            }),
            hull_sprite: b"hull".to_vec(),
            fallback_sprite: b"fallback".to_vec(),
            model_number: -3,
            ..Default::default()
        }
    }

    #[test]
    fn invalid_definition_needs_only_placeholder_registration() {
        let row = ShipModelRow {
            counter: Some(ShipCounterInputs::default()),
            ..valid()
        };
        let name = choose(
            &row,
            b"\xff",
            || panic!("country read"),
            || panic!("graphics read"),
            |name| Ok(name == PLACEHOLDER_ENTITY),
        )
        .unwrap();
        assert_eq!(name.as_deref(), Some(PLACEHOLDER_ENTITY));
        assert!(
            choose(
                &ShipModelRow::default(),
                b"",
                || panic!(),
                || panic!(),
                |_| panic!()
            )
            .unwrap_err()
            .contains("definition-valid")
        );
    }

    #[test]
    fn explicit_winner_bypasses_generated_inputs_and_fallback_country() {
        let name = choose(
            &valid(),
            b"saved\0\xff",
            || panic!("country read"),
            || panic!("graphics read"),
            |name| Ok(name == "saved"),
        )
        .unwrap();
        assert_eq!(name.as_deref(), Some("saved"));
    }

    #[test]
    fn generated_winner_uses_primary_stage_before_sprite_ladders() {
        let name = choose(
            &valid(),
            b"",
            || panic!("country read"),
            || Ok(Some("graphic".into())),
            |name| Ok(name == "graphic"),
        )
        .unwrap();
        assert_eq!(name.as_deref(), Some("graphic"));
        assert_eq!(creator_country(123).unwrap(), 123);
        assert!(creator_country(0).unwrap_err().contains("null-country"));
        assert!(
            creator_country(u32::MAX)
                .unwrap_err()
                .contains("signed creator country -1")
        );
    }

    #[test]
    fn unsupported_primary_or_registered_winner_does_not_descend() {
        let error = choose(
            &valid(),
            b"",
            || panic!("country read"),
            || Err("primary operands".into()),
            |_| panic!(),
        )
        .unwrap_err();
        assert_eq!(error, "primary operands");
        let error = choose(
            &valid(),
            b"saved",
            || panic!("country read"),
            || panic!(),
            |_| Err("registered but unreadable".into()),
        )
        .unwrap_err();
        assert_eq!(error, "registered but unreadable");
    }

    #[test]
    fn fallback_country_is_reached_after_explicit_registry_miss() {
        let mut lookups = Vec::new();
        let error = choose(
            &valid(),
            b"missing",
            || Err("country unavailable".into()),
            || panic!("nonempty explicit generated"),
            |name| {
                lookups.push(name.to_owned());
                Ok(false)
            },
        )
        .unwrap_err();
        assert_eq!(error, "country unavailable");
        assert_eq!(lookups, ["missing"]);
    }

    #[test]
    fn fallback_order_preserves_signed_model_and_first_registered_definition() {
        let mut lookups = Vec::new();
        let name = choose(
            &valid(),
            b"",
            || Ok("TAG"),
            || Ok(None),
            |name| {
                lookups.push(name.to_owned());
                Ok(name == "hull_entity" || name == "TAG_fallback_-3_entity")
            },
        )
        .unwrap();
        assert_eq!(name.as_deref(), Some("hull_entity"));
        assert_eq!(
            lookups,
            [
                "TAG_hull_-3_entity",
                "TAG_hull_entity",
                "hull_-3_entity",
                "hull_entity"
            ]
        );
    }

    #[test]
    fn null_variant_skips_variant_hull_but_still_tries_empty_fallback_sprite() {
        let row = ShipModelRow {
            variant: 0,
            fallback_sprite: Vec::new(),
            ..valid()
        };
        let name = choose(
            &row,
            b"",
            || Ok("TAG"),
            || panic!("invalid variant"),
            |name| Ok(name == "TAG__-3_entity"),
        )
        .unwrap();
        assert_eq!(name.as_deref(), Some("TAG__-3_entity"));
    }

    #[test]
    fn stored_nul_explicit_does_not_generate_or_decode_unreached_sprite() {
        let row = ShipModelRow {
            hull_sprite: vec![255],
            ..valid()
        };
        let name = choose(
            &row,
            b"\0",
            || panic!(),
            || panic!("stored override generated"),
            |name| Ok(name.is_empty()),
        )
        .unwrap();
        assert_eq!(name.as_deref(), Some(""));
    }

    #[test]
    fn a_failed_registered_winner_does_not_use_a_cached_later_hull() {
        let mut assets = UnitModelAssets::off(String::new());
        assets.registered.extend([
            "TAG_fallback_-3_entity".into(),
            "TAG_fallback_entity".into(),
        ]);
        assets.bakes.lock().unwrap().insert(
            "TAG_fallback_-3_entity".into(),
            std::sync::Arc::new(std::sync::Mutex::new(true)),
        );
        assets
            .library
            .write()
            .unwrap()
            .index
            .insert("TAG_fallback_entity".into(), 0);
        let row = ShipModelRow {
            variant: 0,
            ..valid()
        };
        let error = assets
            .ship_slot(|| Ok("TAG"), &row, b"", None, None)
            .unwrap_err();
        assert!(error.contains("TAG_fallback_-3_entity cannot be baked"));
    }

    #[test]
    fn an_unreadable_registered_winner_refuses_even_with_a_cached_bake() {
        let mut assets = UnitModelAssets::off(String::new());
        assets.registered.insert("saved".into());
        Arc::make_mut(&mut assets.roster)
            .unreadable_names
            .insert("saved".into(), "unsupported registration".into());
        assets
            .library
            .write()
            .unwrap()
            .index
            .insert("saved".into(), 0);
        let error = assets
            .ship_slot(|| panic!(), &valid(), b"saved", None, None)
            .unwrap_err();
        assert!(error.contains("selected saved is unreadable: unsupported registration"));
    }

    #[test]
    fn owner_local_first_requirement_preserves_zero_and_differs_from_global() {
        content_reader_script::test_game::enter();
        use replay_data::{
            rows::CountryLinkRow, rows::CountryLocalSubunitRow, rows::LexicalTokenRow,
            rows::LocalSubunitRequirement, rows::TableRows, rows::TableSchema,
        };
        use replay_frame::{frame::Metrics, frame::TableView};
        let content = model_selection::Content::from_test_documents(&[br"
            sub_units = { ship = { type = infantry need = { base = 1 } } }
            equipments = { base = { is_archetype = yes } alternate = { is_archetype = yes } }
        "]);
        let mut views = std::array::from_fn(|_| TableView::Unavailable {
            reason: "unused".into(),
        });
        for table in [
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 100,
                canonical_slot: 0,
            }]),
            TableRows::LexicalTokens(
                [b"ship".as_slice(), b"base", b"alternate"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, name)| LexicalTokenRow {
                        token: index as u32 + 1,
                        literal: name.to_vec(),
                    })
                    .collect(),
            ),
            TableRows::CountryLocalSubunits(vec![CountryLocalSubunitRow {
                country: 100,
                token: 1,
                requirements: vec![
                    LocalSubunitRequirement {
                        archetype_token: 3,
                        amount_raw: 0,
                    },
                    LocalSubunitRequirement {
                        archetype_token: 2,
                        amount_raw: -1,
                    },
                ],
            }]),
        ] {
            let index = table.table().index();
            views[index] = TableView::Complete(table);
        }
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            tables: views.map(Into::into),
            metrics: Metrics::default(),
        };
        let sources = model_selection::Sources::read(&frame).unwrap();
        let subunit = content.subunit_of_token(sources.tokens, 1).unwrap();
        let global = content
            .requirements(Definition::Global(1), subunit, &sources)
            .unwrap();
        let local = content
            .requirements(
                Definition::Local {
                    country: 0,
                    token: 1,
                },
                subunit,
                &sources,
            )
            .unwrap();
        assert_eq!(global.first().unwrap().0, "base");
        assert_eq!(
            local.as_ref(),
            [("alternate".to_owned(), 0), ("base".to_owned(), -1)]
        );
        frame.tables[tables::CountryLocalSubunits::ID.index()] = TableView::Unavailable {
            reason: "gap".into(),
        }
        .into();
        let sources = model_selection::Sources::read(&frame).unwrap();
        assert!(
            content
                .requirements(
                    Definition::Local {
                        country: 0,
                        token: 1
                    },
                    subunit,
                    &sources
                )
                .unwrap_err()
                .to_string()
                .contains("incomplete census")
        );
    }
}
