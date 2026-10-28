//! Cache-independent rebuilding from witnessed naming operands. Never allocates an order.
use super::*;
use installed_content_division_names::division_names::{self, Database};

type Groups = (Database, Vec<String>);
pub(super) fn groups(cat: &Catalogue) -> io::Result<Arc<Groups>> {
    cat.content
        .layers
        .derived::<catalogue::CachedResult<Groups>>()
        .get_or_init(|| division_names::read(&cat.content.layers))
}

/// The variant whose definition names a ship's group, if any.
pub(super) fn variant(
    naming: &rows::ShipNamingOperands,
    model: Option<&rows::ShipModelRow>,
) -> Option<u64> {
    naming
        .equipment
        .or_else(|| model.map(|model| model.variant))
        .filter(|handle| *handle != 0)
}

pub(super) fn group(
    frame: &DecodedFrame,
    naming: &rows::ShipNamingOperands,
    model: Option<&rows::ShipModelRow>,
) -> Result<Option<Vec<u8>>, String> {
    let variant = variant(naming, model)
        .ok_or("Ship naming group unavailable: no recorded naming or selected pool variant")?;
    let definition = find(
        frame.complete::<tables::VariantDefinitions>().ok(),
        variant,
        |row| row.id,
    )
    .ok_or_else(|| format!("Ship naming variant {variant}: definition not recorded"))?;
    let design = definition.design.require("VariantDefinitions.design")?;
    Ok(design.division_names_group.clone())
}

fn override_applies(
    naming: &rows::ShipNamingOperands,
    group: &Result<Option<Vec<u8>>, String>,
) -> bool {
    !naming.override_name.is_empty() && (naming.programmatic == 0 || matches!(group, Ok(None)))
}

pub(super) fn needs_groups(
    naming: &rows::ShipNamingOperands,
    group: &Result<Option<Vec<u8>>, String>,
) -> bool {
    !override_applies(naming, group) && naming.kind == 1 && matches!(group, Ok(Some(_)))
}

pub(super) fn rebuild(
    cat: &Catalogue,
    naming: &rows::ShipNamingOperands,
    group: &Result<Option<Vec<u8>>, String>,
) -> Result<String, String> {
    if override_applies(naming, group) {
        return scripted_text::stored_text(
            &cat.localisation,
            &cat.recorded_language,
            &naming.override_name,
            "ship name override",
        );
    }
    if naming.kind != 1 {
        return Err(format!(
            "Ship naming kind {} is not supported for rebuilding",
            naming.kind
        ));
    }
    let key =
        group.as_ref().map_err(Clone::clone)?.as_ref().ok_or(
            "Ship name has no group or usable override; legacy generation is not reproduced",
        )?;
    groups(cat)
        .map_err(|why| why.to_string())?
        .0
        .resolve_any(key, naming.order, naming.ordered != 0)
        .map_err(|why| format!("Ship name group: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalogue() -> (Catalogue, catalogue::tests::Folder) {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        catalogue::tests::folder_catalogue(&[(
            "common/units/names_ships/synthetic.txt",
            r#"
            MOD_GROUP={ type=ship prefix="P "
                ordered={ 2={"Hull %d"} 7={"Fleet %s / %d"} }
                unordered={ a={"Unordered %d"} }
                unique={"Unique %s"}
                fallback_name="Fallback %d"
            }
        "#,
        )])
    }
    #[test]
    fn rebuilt_ship_names_equal_synthetic_cache_for_selected_native_rules() {
        let (cat, _folder) = catalogue();
        let cat = Arc::new(cat);
        for (order, ordered, programmatic, override_name, expected) in [
            (2, 1, 0, "", "P Hull 2"),
            (7, 2, 0, "", "P Fleet VII / %d"),
            (0, 0, 0, "", "P Unordered %d"),
            (1, 0, 0, "", "P Unique %s"),
            (4, 1, 0, "", "Fallback 4"),
            (-1, 0, 0, "", "Fallback 1"),
            (
                2,
                1,
                0,
                "synthetic_key_looking_literal",
                "synthetic_key_looking_literal",
            ),
            (2, 1, 0xfe, "hidden old override", "P Hull 2"),
        ] {
            let naming = rows::ShipNamingOperands {
                kind: 1,
                order,
                ordered,
                programmatic,
                override_name: override_name.as_bytes().to_vec(),
                equipment: Some(70),
            };
            let rebuilt = Operand::ShipRebuilt {
                handle: 9,
                naming,
                group: Ok(Some(b"MOD_GROUP".to_vec())),
            };
            let cache = Operand::ShipCache {
                handle: 9,
                bytes: expected.as_bytes().to_vec(),
            };
            assert_eq!(
                super::super::resolve(&rebuilt, Some(&cat)).unwrap().name,
                expected
            );
            assert_eq!(
                rebuild(
                    &cat,
                    match &rebuilt {
                        Operand::ShipRebuilt { naming, .. } => naming,
                        _ => unreachable!(),
                    },
                    &Ok(Some(b"MOD_GROUP".to_vec()))
                )
                .unwrap(),
                scripted_text::stored_text(
                    &cat.localisation,
                    &cat.recorded_language,
                    match &cache {
                        Operand::ShipCache { bytes, .. } => bytes,
                        _ => unreachable!(),
                    },
                    "cache"
                )
                .unwrap()
            );
        }
    }
    #[test]
    fn ship_name_rebuild_refuses_unassigned_missing_empty_and_unknown_inputs() {
        let (cat, _folder) = catalogue();
        let mut naming = rows::ShipNamingOperands {
            kind: 1,
            order: 0,
            ordered: 1,
            ..Default::default()
        };
        assert!(
            rebuild(&cat, &naming, &Ok(Some(b"MOD_GROUP".to_vec())))
                .unwrap_err()
                .contains("not yet assigned")
        );
        for group in [
            Ok(None),
            Ok(Some(Vec::new())),
            Ok(Some(b"missing".to_vec())),
            Err("variant not recorded".into()),
        ] {
            assert!(rebuild(&cat, &naming, &group).is_err());
        }
        naming.override_name = b"Programmatic".to_vec();
        naming.programmatic = 1;
        assert_eq!(rebuild(&cat, &naming, &Ok(None)).unwrap(), "Programmatic");
        assert!(rebuild(&cat, &naming, &Err("group unknown".into())).is_err());
        naming.programmatic = 0;
        assert_eq!(
            rebuild(&cat, &naming, &Err("group unknown".into())).unwrap(),
            "Programmatic"
        );
        naming.override_name.clear();
        naming.kind = 9;
        assert!(
            rebuild(&cat, &naming, &Ok(Some(b"MOD_GROUP".to_vec())))
                .unwrap_err()
                .contains("kind 9")
        );
        let empty = Operand::ShipCache {
            handle: 9,
            bytes: Vec::new(),
        };
        let cat = Arc::new(cat);
        assert_eq!(super::super::resolve(&empty, Some(&cat)).unwrap().name, "");
        let invalid = Operand::ShipCache {
            handle: 9,
            bytes: vec![0xff],
        };
        assert!(super::super::resolve(&invalid, Some(&cat)).is_err());
    }
    #[test]
    fn ship_name_groups_use_layer_replacement_and_stack_scoped_caches() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (base_cat, base) = catalogue::tests::folder_catalogue(&[(
            "common/units/names_ships/base.txt",
            "MOD_GROUP={type=ship prefix=\"Base \" ordered={2={\"Hull %d\"}}}",
        )]);
        let (_, overlay) = catalogue::tests::folder_catalogue(&[(
            "common/units/names_ships/mod.txt",
            "MOD_GROUP={type=ship prefix=\"Mod \" ordered={2={\"Hull %d\"}}}",
        )]);
        let replace = ["common/units/names_ships"];
        let layers = installed_content_core::ResolvedLayers::from_folders(&[
            (&base.0, &[][..]),
            (&overlay.0, &replace),
        ])
        .unwrap();
        let mod_cat = catalogue::tests::layers_catalogue(layers);
        let naming = rows::ShipNamingOperands {
            kind: 1,
            order: 2,
            ordered: 1,
            ..Default::default()
        };
        let key = Ok(Some(b"MOD_GROUP".to_vec()));
        assert_eq!(rebuild(&base_cat, &naming, &key).unwrap(), "Base Hull 2");
        assert_eq!(rebuild(&mod_cat, &naming, &key).unwrap(), "Mod Hull 2");
        assert_eq!(rebuild(&base_cat, &naming, &key).unwrap(), "Base Hull 2");
    }
    #[test]
    fn ship_cache_keeps_empty_language_independent_but_refuses_opaque_translation() {
        let (mut cat, _folder) = catalogue();
        cat.recorded_language = "synthetic-other-language".into();
        let cat = Arc::new(cat);
        for bytes in [Vec::new(), b"literal_token_looking_name".to_vec()] {
            let result = super::super::resolve(
                &Operand::ShipCache {
                    handle: 9,
                    bytes: bytes.clone(),
                },
                Some(&cat),
            );
            if bytes.is_empty() {
                assert_eq!(result.unwrap().name, "");
            } else {
                assert!(result.is_err());
            }
        }
    }
}
