//! Content rosters used by effect subreaders: the same recorded-stack
//! database loaders as their other consumers, with an unread suffix kept
//! as a named gap instead of treating an unknown key as a miss.

use std::{collections::BTreeSet, io};

use crate::{ResolvedLayers, equipment, modifier_definitions, terrain, tokens::Token, unit_leader};

use super::KeyNames;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EffectNames {
    pub terrains: BTreeSet<Vec<u8>>,
    pub terrains_gap: Option<String>,
    pub equipment: BTreeSet<Vec<u8>>,
    pub equipment_gap: Option<String>,
    pub equipment_upgrades: BTreeSet<Vec<u8>>,
    pub equipment_upgrades_gap: Option<String>,
    pub modifiers: BTreeSet<Vec<u8>>,
    pub modifiers_gap: Option<String>,
    pub early_modifiers: BTreeSet<Vec<u8>>,
    pub early_modifiers_gap: Option<String>,
}

impl EffectNames {
    /// 140636690 first finds a sub-unit by token (140AB5EE0), then a
    /// category (140AB5BB0). SSubUnitStats finds terrain types by token
    /// (141415B80 -> singleton 14022F8F0); `need` and `need_equipment`
    /// use GetTypeByToken (140FFC070 -> 1409EB9F0 -> 1409DC660), which
    /// converts the token to text and includes equipment aliases.
    pub(super) fn read(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Self> {
        let fallback;
        let sub_units = if let Some(sub_units) = &names.sub_units {
            sub_units
        } else {
            fallback = unit_leader::sub_units(layers)?;
            &fallback
        };
        let (modifiers, _) = modifier_definitions::read(layers)?;
        let early_modifiers = token_names(modifiers.registered_names());
        let builtin_gap = "built-in modifier token registry 140552CB0/14054A990 is not in the recorded content stack";
        let mut result = Self {
            modifiers: early_modifiers.clone(),
            modifiers_gap: Some(
                modifiers
                    .gap()
                    .or(sub_units.gap())
                    .unwrap_or(builtin_gap)
                    .to_owned(),
            ),
            early_modifiers,
            early_modifiers_gap: Some(modifiers.gap().unwrap_or(builtin_gap).to_owned()),
            ..Self::default()
        };
        result
            .modifiers
            .extend(sub_units.modifiers().iter().cloned());
        // CTerrainDatabase::load_directory 140AC7AF0 / 140AC6EB0 appends
        // categories before later effect files are read. An absent directory
        // is an empty database; the full map projection's requirement for
        // at least one category does not apply to token lookup.
        let mut definitions = terrain::TerrainDefinitions::default();
        for path in terrain::definition_paths(layers)? {
            if let Err(error) = definitions.append(&layers.read(&path)?) {
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
        let database =
            equipment::Database::load_with_names(layers, &names.at(super::Point::BeforeBuildings))?;
        result.equipment = crate::technology::equipment_pool_roster(&database);
        result.equipment_upgrades = token_names(database.registered_upgrades());
        result.equipment_gap = database.lost.clone();
        result.equipment_upgrades_gap = database.lost;
        Ok(result)
    }
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
    use crate::parser::{
        Operator,
        native::{Field, Kind, Shape, Token as LexToken},
    };

    #[test]
    fn sub_unit_category_lists_register_raw_values_without_definitions() {
        let mut units = unit_leader::SubUnits::default();
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
        let mut units = unit_leader::SubUnits::default();
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
        let mut units = unit_leader::SubUnits::default();
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
        let root = std::env::temp_dir().join(format!(
            "installed_content_effect_names_{}_{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
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
        let names = EffectNames::read(&layers, &KeyNames::default()).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(names.terrains, [b"active_terrain".to_vec()].into());
        assert_eq!(
            names.equipment,
            [b"active_equipment".to_vec(), b"active_alias".to_vec()].into()
        );
        // RegisterType (1409ECF30) installs Active_Alias in the same index
        // that GetTypeByToken (1409EB9F0) uses for Pool (140FFC070).
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
            crate::technology::equipment_pool_shape(&field(&alias), Some((&names.equipment, None))),
            Shape::One
        ));
        assert!(matches!(
            crate::technology::equipment_pool_shape(
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
        assert!(names.modifiers_gap.is_some());
        assert!(names.early_modifiers_gap.is_some());
    }
}
