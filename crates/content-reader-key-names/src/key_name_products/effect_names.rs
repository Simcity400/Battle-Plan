
use std::{collections::BTreeSet, io, sync::Arc};

use content_reader_layers::ResolvedLayers;
use crate::{equipment, modifier_definitions, terrain};
use content_reader_parser::tokens::Token;

use super::KeyNames;
use content_reader_script::effect_names::EffectNames;

pub(super) fn read(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<(EffectNames, Arc<equipment::Database>)> {
        let fallback;
        let sub_units = if let Some(sub_units) = &names.sub_units {
            sub_units
        } else {
            fallback = crate::unit_leader::sub_units(layers)?;
            &fallback
        };
        let (modifiers, _) = modifier_definitions::read(layers)?;
        let early_modifiers = token_names(modifiers.registered_names());
        let mut result = EffectNames {
            modifiers: early_modifiers.clone(),
            modifiers_gap: modifiers.gap().or(sub_units.gap()).map(str::to_owned),
            early_modifiers,
            early_modifiers_gap: modifiers.gap().map(str::to_owned),
            ..EffectNames::default()
        };
        result
            .modifiers
            .extend(sub_units.modifiers().iter().cloned());
        let mut definitions = terrain::TerrainDefinitions::default();
        for path in terrain::definition_paths(layers)? {
            if let Err(error) = definitions.append(&path.read(layers)?) {
                result.terrains_gap = Some(format!("{path}: {error}"));
                break;
            }
        }
        result.terrains = token_names(
            definitions
                .categories
                .iter()
                .map(|category| category.name.as_slice()),
        );
        let database = layers.product("equipment database", 1, &[], |layers| {
            equipment::Database::load_with_names(layers, &names.at(content_reader_script::key_names::Point::BeforeBuildings))
        })?;
        result.equipment = crate::equipment::equipment_pool_roster(&database);
        result.equipment_upgrades = token_names(database.registered_upgrades());
        result.equipment_gap = database.lost.clone();
        result.equipment_upgrades_gap = database.lost.clone();
        Ok((result, database))
}

fn token_names<'a>(names: impl IntoIterator<Item = &'a [u8]>) -> BTreeSet<Vec<u8>> {
    names
        .into_iter()
        .filter_map(|name| match Token::c_str(name) {
            Token::Name(name) => Some(name),
            Token::Rejected => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "test-support")]
    use content_reader_parser::parser::Operator;
    #[cfg(feature = "test-support")]
    use content_reader_script::native::{Field, Kind, Shape, Token as LexToken};
    #[cfg(feature = "test-support")]
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(feature = "test-support")]
    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn sub_unit_category_lists_register_raw_values_without_definitions() {
        content_reader_script::test_game::enter();
        let mut units = content_reader_script::sub_units::SubUnits::default();
        units.read(
            b"@cat = Group_A SUB_UNIT_CATEGORIES = { @cat \"GROUP_B\" @inside = NeverDefined } \
              \"sub_unit_categories\" = { QuotedSkip } \
              sub_units = { Unit_A = { } } sub_unit_categories = { @inside Group_C }",
            "common/unit_tags/a.txt",
        );
        assert_eq!(
            units.categories(),
            &[
                b"group_a".to_vec(),
                b"group_b".to_vec(),
                b"@inside".to_vec(),
                b"=".to_vec(),
                b"neverdefined".to_vec(),
                b"group_c".to_vec(),
            ]
            .into()
        );
        assert_eq!(
            units.registered().collect::<Vec<_>>(),
            [b"Unit_A".as_slice()]
        );
        assert_eq!(units.gap(), None);
    }

    #[test]
    fn effect_rosters_follow_dynamic_token_identity() {
        assert_eq!(
            token_names([
                b"UNIT_A".as_slice(),
                b"unit_a",
                b"Terrain\0suffix",
                b"",
                b"1unit",
                b"-unit",
            ]),
            [b"unit_a".to_vec(), b"terrain".to_vec()].into()
        );
    }

    #[test]
    fn sub_unit_modifier_lists_use_native_string_conversion_and_register_no_definitions() {
        content_reader_script::test_game::enter();
        let mut units = content_reader_script::sub_units::SubUnits::default();
        units.read(b"@mod = Active_Mod SUB_UNIT_MODIFIERS = { @mod \"two\nlines\" @inside = Undefined } sub_unit_modifiers = { @inside }", "common/units/unit_modifiers/a.txt");
        assert_eq!(
            units.modifiers(),
            &[
                b"active_mod".to_vec(),
                b"unreadable string".to_vec(),
                b"@inside".to_vec(),
                b"=".to_vec(),
                b"undefined".to_vec(),
            ]
            .into()
        );
        assert_eq!(units.gap(), None);
    }

    #[test]
    fn an_unread_sub_unit_suffix_remains_a_roster_gap() {
        content_reader_script::test_game::enter();
        let mut units = content_reader_script::sub_units::SubUnits::default();
        units.read(b"sub_unit_categories = { first } sub_units = { known = { } broken = { weights = { x @v = 1 } } later = { } }", "common/units/a.txt");
        assert!(units.categories().contains(b"first".as_slice()));
        assert!(units.registered().any(|name| name == b"known"));
        assert!(
            units
                .gap()
                .is_some_and(|why| why.contains("common/units/a.txt"))
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn effect_rosters_use_layer_replacement_and_equipment_aliases() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "content_reader_effect_names_{}_{:?}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let base = root.join("base");
        let overlay = root.join("overlay");
        for (folder, path, text) in [
            (
                &base,
                "common/unit_tags/a.txt",
                "sub_unit_categories = { replaced_category }",
            ),
            (
                &overlay,
                "common/unit_tags/b.txt",
                "sub_unit_categories = { Active_Category }",
            ),
            (
                &base,
                "common/units/a.txt",
                "sub_units = { Active_Unit = { categories = { Active_Category } } }",
            ),
            (
                &base,
                "common/units/unit_modifiers/a.txt",
                "sub_unit_modifiers = { Active_Unit_Modifier }",
            ),
            (
                &base,
                "common/units/nested/a.txt",
                "sub_units = { unseen = { } }",
            ),
            (
                &base,
                "common/terrain/a.txt",
                "categories = { Active_Terrain = { } }",
            ),
            (
                &base,
                "common/units/equipment/a.txt",
                "equipments = { Active_Equipment = { alias = { Active_Alias } } }",
            ),
            (
                &base,
                "common/units/equipment/upgrades/a.txt",
                "upgrades = { Active_Upgrade = { max_level = 3 alias = { skipped_alias } } }",
            ),
            (
                &base,
                "common/modifier_definitions/a.txt",
                "none = { } Active_Script_Modifier = { }",
            ),
        ] {
            let path = folder.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let replace = ["common/unit_tags"];
        let layers =
            ResolvedLayers::from_folders(&[(&base, &[][..]), (&overlay, &replace)]).unwrap();
        let (names, database) = read(&layers, &KeyNames::default()).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(names.terrains, [b"active_terrain".to_vec()].into());
        assert_eq!(
            names.equipment,
            [b"active_equipment".to_vec(), b"active_alias".to_vec()].into()
        );
        let token = |kind, text: &[u8]| LexToken {
            kind,
            text: text.to_vec(),
            line: 1,
            quoted: false,
            byte_end: 0,
        };
        let value = token(Kind::Open, b"{");
        let alias = token(Kind::Bare, b"Active_Alias");
        let missing = token(Kind::Bare, b"not_registered");
        let field = |key| Field {
            key,
            value: &value,
            fresh: true,
            operator: Operator::Equals,
            current: &value,
            effect_scope: None,
        };
        assert!(matches!(
            content_reader_script::unit_stats::equipment_pool_shape(&field(&alias), Some((&names.equipment, None))),
            Shape::One
        ));
        assert!(matches!(
            content_reader_script::unit_stats::equipment_pool_shape(
                &field(&missing),
                Some((&names.equipment, None))
            ),
            Shape::Skip
        ));
        assert_eq!(
            names.equipment_upgrades,
            [b"active_upgrade".to_vec()].into()
        );
        assert!(names.modifiers.contains(b"none".as_slice()));
        assert!(
            names
                .modifiers
                .contains(b"active_script_modifier".as_slice())
        );
        assert!(names.modifiers.contains(b"active_unit_modifier".as_slice()));
        assert!(names.early_modifiers.contains(b"none".as_slice()));
        assert!(
            names
                .early_modifiers
                .contains(b"active_script_modifier".as_slice())
        );
        assert!(
            !names
                .early_modifiers
                .contains(b"active_unit_modifier".as_slice())
        );
        assert_eq!(names.terrains_gap, None);
        assert_eq!(names.equipment_gap, None);
        assert_eq!(names.equipment_upgrades_gap, None);
        assert_eq!(names.modifiers_gap, None);
        assert_eq!(names.early_modifiers_gap, None);
        let lookup = KeyNames {
            effects: names,
            ..KeyNames::default()
        };
        assert!(std::ptr::eq(
            database.named("Active_Alias").unwrap(),
            database.named("Active_Equipment").unwrap()
        ));
        let product = super::super::KeyNameProducts {
            names: lookup,
            buildings: None,
            equipment_database: Some(database),
        };
        let cloned = product.clone();
        assert!(Arc::ptr_eq(product.equipment_database().unwrap(), cloned.equipment_database().unwrap()));
        let lookup = &product.names;
        assert!(lookup.at(content_reader_script::key_names::Point::IdeaTags).effects.equipment.is_empty());
        assert_eq!(
            content_reader_script::modifiers::registered(b"ACTIVE_UNIT_MODIFIER", Some(lookup)),
            Some(true)
        );
        assert_eq!(
            content_reader_script::modifiers::registered(b"Active_Script_Modifier", Some(lookup)),
            Some(true)
        );
        assert_eq!(
            content_reader_script::modifiers::registered(b"ARMY_ATTACK_FACTOR", Some(lookup)),
            Some(true)
        );
        assert_eq!(
            content_reader_script::modifiers::registered(b"absent_modifier", Some(lookup)),
            Some(false)
        );
        assert_eq!(
            content_reader_script::modifiers::registered(
                b"ACTIVE_UNIT_MODIFIER",
                Some(&lookup.at(content_reader_script::key_names::Point::IdeaTags))
            ),
            Some(false)
        );
    }
}
