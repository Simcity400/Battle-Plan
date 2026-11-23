use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    ops::Deref,
    sync::Arc,
};

use crate::{
    buildings,
    equipment::Database,
    ideologies::Ideologies,
    resources::{self, Resources},
};
use content_reader_layers::ResolvedLayers;
use content_reader_script::{
    key_names::{KeyNames, file_tags},
    trigger_keys::CountTriggerNames,
};

mod effect_names;

#[derive(Clone, Debug, Default)]
pub struct KeyNameProducts {
    pub names: KeyNames,
    pub buildings: Option<Arc<buildings::Buildings>>,
    equipment_database: Option<Arc<Database>>,
}

impl Deref for KeyNameProducts {
    type Target = KeyNames;

    fn deref(&self) -> &KeyNames {
        &self.names
    }
}

impl KeyNameProducts {
    pub fn equipment_database(&self) -> Option<&Arc<Database>> {
        self.equipment_database.as_ref()
    }

    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        use content_reader_layers::parallel::{join, spawn};
        let (
            ideologies,
            resources,
            scripted,
            scripted_effects,
            countries,
            aliases,
            characters,
            sub_units,
            template_columns,
        ) = std::thread::scope(|scope| {
            let ideologies = spawn(scope, || crate::ideologies::read(layers));
            let resources = spawn(scope, || resources::read(layers));
            let scripted = spawn(scope, || {
                crate::scripted_triggers::names(layers)
            });
            let effects = spawn(scope, || crate::scripted_effects::names(layers));
            let countries = spawn(scope, || country_tags(layers));
            let aliases = spawn(scope, || crate::tag_aliases::names(layers));
            let characters = spawn(scope, || crate::characters::tokens(layers));
            let sub_units = spawn(scope, || {
                crate::unit_leader::sub_units(layers)
            });
            let columns = spawn(scope, || template_columns(layers));
            (
                join(ideologies),
                join(resources),
                join(scripted),
                join(effects),
                join(countries),
                join(aliases),
                join(characters),
                join(sub_units),
                join(columns),
            )
        });
        let (ideologies, _) = ideologies?;
        let (resources, _) = resources?;
        let mut names = KeyNames {
            scripted: scripted?,
            scripted_effects: scripted_effects?,
            counts: count_trigger_names(&buildings::Buildings::default(), &ideologies, &resources),
            countries: countries?,
            aliases: aliases?,
            characters: characters?,
            province_buildings: BTreeMap::new(),
            sub_units: Some(sub_units?),
            template_columns: Some(template_columns),
            ..KeyNames::default()
        };
        let buildings = if layers.paths()?.any(buildings::is_building_file) {
            let buildings = layers.product("building types", 1, &[], |layers| {
                buildings::read_with(layers, &names)
            })?;
            names.counts.buildings =
                count_trigger_names(&buildings, &ideologies, &Default::default()).buildings;
            for (name, building) in &buildings.types {
                let provincial = building.provincial();
                let flag = buildings
                    .repeated_types
                    .get(name)
                    .map_or(Some(provincial), |repeats| {
                        repeats
                            .iter()
                            .all(|other| other.provincial() == provincial)
                            .then_some(provincial)
                    });
                names
                    .province_buildings
                    .insert(name.as_bytes().to_ascii_lowercase(), flag);
            }
            Some(buildings)
        } else {
            None
        };
        let (effects, equipment_database) = effect_names::read(layers, &names)?;
        names.effects = effects;
        Ok(Self {
            names,
            buildings,
            equipment_database: Some(equipment_database),
        })
    }
}

pub trait KeyNamesExt {
    fn key_names(&self) -> io::Result<Arc<KeyNameProducts>>;
    fn key_name_products(&self) -> io::Result<Arc<KeyNameProducts>>;
}

impl KeyNamesExt for ResolvedLayers {
    fn key_names(&self) -> io::Result<Arc<KeyNameProducts>> {
        self.key_name_products()
    }

    fn key_name_products(&self) -> io::Result<Arc<KeyNameProducts>> {
        self.retained_recorded_roster_product("key names", 1, KeyNameProducts::read, |product| {
            !product
                .names
                .template_columns
                .as_ref()
                .is_some_and(Result::is_err)
        })
    }
}

fn template_columns(layers: &ResolvedLayers) -> Result<[i32; 3], String> {
    let read = || -> io::Result<[i32; 3]> {
        let defines = content_reader_defines::defines::read(layers)?;
        Ok([
            defines.int32("NMilitary", "MAX_DIVISION_BRIGADE_WIDTH")?,
            defines.int32("NMilitary", "MAX_DIVISION_SUPPORT_WIDTH")?,
            defines.int32("NMilitary", "MAX_REGIMENTAL_SUPPORT_WIDTH")?,
        ])
    };
    read().map_err(|why| format!("division-template column defines: {why}"))
}

pub fn country_tags(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut tags = BTreeSet::new();
    for path in
        content_reader_layers::script_files::paths(layers, "common/country_tags/", Some(".txt"))?
    {
        tags.extend(file_tags(&path.read(layers)?));
    }
    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_base() -> synthetic_game::Built {
        synthetic_game::Installation::new("key-names")
            .file(
                "common/defines/00_defines.lua",
                "NDefines = { NMilitary = { MAX_DIVISION_BRIGADE_WIDTH = 5,                  MAX_DIVISION_SUPPORT_WIDTH = 1, MAX_REGIMENTAL_SUPPORT_WIDTH = 2 } }",
            )
            .file(
                "common/scripted_triggers/00_base.txt",
                "base_trigger = { always = yes }",
            )
            .file("common/scripted_effects/00_base.txt", "base_effect = { }")
            .file("common/country_tags/00_base.txt", "QQB = \"countries/Base.txt\"")
            .file(
                "common/ideologies/00_base.txt",
                "ideologies = { centre = { types = { moderate = { } } color = { 1 1 1 } } }",
            )
            .file("common/resources/00_base.txt", "resources = { ore = { icon_frame = 1 } }")
            .file("common/country_tag_aliases/00_base.txt", "")
            .file("common/characters/00_base.txt", "")
            .file("common/units/00_base.txt", "")
            .build()
    }

    #[test]
    fn names_read_at_once_equal_names_read_in_turn() {
        content_reader_script::test_game::enter();
        let installed = synthetic_base();
        let game = installed.game_dir.clone();
        let overlay =
            std::env::temp_dir().join(format!("key_names_at_once_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&overlay);
        for (path, text) in [
            (
                "common/scripted_triggers/zz_synthetic.txt",
                "synthetic_trigger = { always = yes }",
            ),
            (
                "common/scripted_effects/zz_synthetic.txt",
                "synthetic_effect = { }",
            ),
            (
                "common/country_tags/zz_synthetic.txt",
                "QQX = \"countries/Synthetic.txt\"",
            ),
        ] {
            let path = overlay.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[
            (game.as_path(), &[][..]),
            (overlay.as_path(), &[][..]),
        ])
        .unwrap();
        let names = KeyNameProducts::read(&layers).unwrap().names;
        let (ideologies, _) = crate::ideologies::read(&layers).unwrap();
        let (resources, _) = resources::read(&layers).unwrap();
        assert_eq!(
            names.scripted,
            crate::scripted_triggers::names(&layers).unwrap()
        );
        assert!(names.scripted.contains(b"synthetic_trigger".as_slice()));
        assert_eq!(
            names.scripted_effects,
            crate::scripted_effects::names(&layers).unwrap()
        );
        assert_eq!(names.countries, country_tags(&layers).unwrap());
        assert!(names.countries.contains(b"QQX".as_slice()));
        assert_eq!(names.aliases, crate::tag_aliases::names(&layers).unwrap());
        assert_eq!(
            names.characters,
            crate::characters::tokens(&layers).unwrap()
        );
        assert_eq!(
            names.sub_units,
            Some(crate::unit_leader::sub_units(&layers).unwrap())
        );
        assert_eq!(names.template_columns, Some(template_columns(&layers)));
        let counts = count_trigger_names(&buildings::Buildings::default(), &ideologies, &resources);
        assert_eq!(names.counts.resources, counts.resources);
        assert_eq!(names.counts.ideologies, counts.ideologies);
        assert_eq!(names.counts.ideology_types, counts.ideology_types);
        let _ = std::fs::remove_dir_all(&overlay);
    }

    #[test]
    fn traced_readers_of_a_stack_share_one_read_and_trace_what_it_read() {
        use std::sync::{Arc, Mutex};
        content_reader_script::test_game::enter();
        let installed = synthetic_base();
        let game = installed.game_dir.clone();
        let open = || ResolvedLayers::from_folders(&[(game.as_path(), &[][..])]).unwrap();
        type Log = Arc<Mutex<Vec<content_reader_layers::content::ContentRead>>>;
        let traced = |layers: &ResolvedLayers, purpose: &str| {
            let log = Log::default();
            let kept = Arc::clone(&log);
            let layers = layers
                .with_read_trace(Arc::new(move |read| kept.lock().unwrap().push(read)))
                .with_read_purpose(purpose);
            (layers, log)
        };
        let reads = |log: &Log, purpose: &str| {
            let log = log.lock().unwrap();
            assert!(log.iter().all(|read| read.purpose == purpose));
            log.iter()
                .map(|read| (read.path.clone(), read.layer, format!("{:?}", read.outcome)))
                .collect::<BTreeSet<_>>()
        };
        let stack = open();
        let (a, a_log) = traced(&stack, "a");
        let (b, b_log) = traced(&stack, "b");
        let names = a.key_names().unwrap();
        let traced_once = a_log.lock().unwrap().len();
        assert!(Arc::ptr_eq(&names, &b.key_names().unwrap()));
        assert!(Arc::ptr_eq(&names, &a.key_names().unwrap()));
        let holder = a_log.lock().unwrap().len();
        assert_eq!(holder, traced_once, "a holder traces nothing again");
        let (alone, alone_log) = traced(&open(), "alone");
        assert!(!Arc::ptr_eq(&names, &alone.key_names().unwrap()));
        let expected = reads(&alone_log, "alone");
        assert!(!expected.is_empty());
        assert_eq!(reads(&a_log, "a"), expected);
        assert_eq!(reads(&b_log, "b"), expected);

        let (stack, names) = {
            let _generation = content_reader_layers::content::Generation::enter();
            let stack = open();
            let (first, _) = traced(&stack, "first");
            let names = first.key_names().unwrap();
            (stack, names)
        };
        let tags = game.join("common/country_tags/00_base.txt");
        let hidden = tags.with_extension("hidden");
        std::fs::rename(&tags, &hidden).unwrap();
        let (fresh, log) = traced(&stack, "fresh");
        let kept = fresh.key_names();
        std::fs::rename(&hidden, &tags).unwrap();
        assert!(Arc::ptr_eq(&names, &kept.unwrap()));
        assert_eq!(reads(&log, "fresh"), expected);
    }

    #[test]
    fn a_traced_read_that_met_a_failure_is_not_shared() {
        use std::sync::{Arc, Mutex};
        content_reader_script::test_game::enter();
        let installed = synthetic_base();
        let game = installed.game_dir.clone();
        let overlay =
            std::env::temp_dir().join(format!("key_names_failure_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&overlay);
        let defines = overlay.join("common/defines/zz_synthetic.lua");
        std::fs::create_dir_all(defines.parent().unwrap()).unwrap();
        std::fs::write(&defines, "NDefines.NMilitary.SYNTHETIC = 1").unwrap();
        let stack = ResolvedLayers::from_folders(&[
            (game.as_path(), &[][..]),
            (overlay.as_path(), &[][..]),
        ])
        .unwrap();
        let traced = |purpose: &str| {
            let log = Arc::new(Mutex::new(Vec::new()));
            let kept = Arc::clone(&log);
            let layers = stack
                .with_read_trace(Arc::new(move |read| kept.lock().unwrap().push(read)))
                .with_read_purpose(purpose);
            (layers, log)
        };
        let hidden = defines.with_extension("hidden");
        std::fs::rename(&defines, &hidden).unwrap();
        let failed = traced("a").0.key_names().unwrap();
        assert!(matches!(failed.template_columns, Some(Err(_))));
        std::fs::rename(&hidden, &defines).unwrap();
        let (b, log) = traced("b");
        let whole = b.key_names().unwrap();
        assert!(!Arc::ptr_eq(&failed, &whole));
        assert!(matches!(whole.template_columns, Some(Ok(_))));
        assert!(log.lock().unwrap().iter().any(|read| {
            read.path == b"common/defines/zz_synthetic.lua"
                && matches!(
                    read.outcome,
                    content_reader_layers::content::ReadOutcome::Bytes { .. }
                )
        }));
        assert!(Arc::ptr_eq(&whole, &traced("c").0.key_names().unwrap()));
        let _ = std::fs::remove_dir_all(&overlay);
    }
}

pub fn count_trigger_names(
    buildings: &buildings::Buildings,
    ideologies: &Ideologies,
    resources: &Resources,
) -> CountTriggerNames {
    let mut provincial = BTreeMap::new();
    for (name, building) in &buildings.types {
        let class = building.provincial();
        let agreed = buildings
            .repeated_types
            .get(name)
            .is_none_or(|repeats| repeats.iter().all(|other| other.provincial() == class));
        let class = agreed.then_some(class);
        provincial
            .entry(name.as_bytes().to_ascii_lowercase())
            .and_modify(|known: &mut Option<bool>| {
                if *known != class {
                    *known = None;
                }
            })
            .or_insert(class);
    }
    CountTriggerNames::new(
        buildings
            .types
            .keys()
            .map(|name| name.as_bytes().to_ascii_lowercase())
            .collect(),
        provincial,
        ideologies
            .groups()
            .0
            .iter()
            .map(|group| group.token.as_bytes().to_ascii_lowercase())
            .collect(),
        ideologies
            .groups()
            .0
            .iter()
            .flat_map(|group| &group.types)
            .map(|token| token.as_bytes().to_ascii_lowercase())
            .collect(),
        ideologies.groups().1.map(ToString::to_string),
        resources
            .declarations
            .iter()
            .map(|resource| resource.token.as_bytes().to_ascii_lowercase())
            .collect(),
        resources.gap().map(ToString::to_string),
    )
}

#[cfg(test)]
mod count_tests {
    use super::*;
    use crate::buildings::Buildings;
    use content_reader_script::trigger_keys::{CountKind, Reader};
    #[test]
    fn count_trigger_names_fold_the_loaded_databases() {
        content_reader_script::test_game::with(|game| game.trigger("armor", 0, Reader::Assign, 0));
        let mut buildings = Buildings::default();
        buildings.types.insert(
            "Arms_Factory".into(),
            crate::buildings::BuildingType::default(),
        );
        let groups = crate::ideologies::parse(b"ideologies = { fascism = { } }");
        let ideologies = Ideologies::from_groups(groups.clone(), None);
        let steel = crate::resources::Resource {
            token: "Steel".into(),
            icon_frame: None,
        };
        let resources = Resources::new(vec![steel.clone()]);
        let names = count_trigger_names(&buildings, &ideologies, &resources);
        assert!(names.is_building(b"ARMS_FACTORY"));
        assert_eq!(names.is_ideology(b"Fascism"), Ok(true));
        assert!(!names.is_building(b"fascism"));
        assert_eq!(
            names.count_kind(b"arms_factory"),
            Ok(Some(CountKind::Building))
        );
        assert_eq!(names.count_kind(b"STEEL"), Ok(Some(CountKind::Resource)));
        assert_eq!(names.count_kind(b"fascism"), Ok(Some(CountKind::Ideology)));
        assert_eq!(
            names.count_kind(b"Infantry"),
            Ok(Some(CountKind::Equipment))
        );
        assert_eq!(names.count_kind(b"armor"), Ok(None));
        assert_eq!(names.count_kind(b"unknown"), Ok(None));
        let stopped = Ideologies::from_groups(groups, Some(("common/ideologies/b.txt", "line 3")));
        let names = count_trigger_names(&buildings, &stopped, &resources);
        assert_eq!(names.is_ideology(b"FASCISM"), Ok(true));
        assert_eq!(
            names.count_kind(b"arms_factory"),
            Ok(Some(CountKind::Building))
        );
        let unknown = names.count_kind(b"technocracy").unwrap_err();
        assert!(
            unknown.starts_with("whether technocracy names an ideology group is unknown: common/ideologies/b.txt is unknown from line 3"),
            "{unknown}"
        );
        let stopped = Resources::stopped(vec![steel], "common/resources/b.txt", "line 2");
        let names = count_trigger_names(&buildings, &ideologies, &stopped);
        assert_eq!(names.count_kind(b"steel"), Ok(Some(CountKind::Resource)));
        assert_eq!(
            names.count_kind(b"arms_factory"),
            Ok(Some(CountKind::Building))
        );
        let unknown = names.count_kind(b"fascism").unwrap_err();
        assert!(
            unknown.starts_with("whether fascism names a resource is unknown: common/resources/b.txt is unknown from line 2"),
            "{unknown}"
        );
    }
    #[test]
    fn building_classes_are_known_only_when_declarations_agree() {
        use crate::buildings::BuildingType;
        let provincial = BuildingType {
            province_max: Some(1),
            ..BuildingType::default()
        };
        let mut buildings = Buildings::default();
        buildings.types.insert("Bunker".into(), provincial.clone());
        buildings
            .types
            .insert("mill".into(), BuildingType::default());
        buildings.types.insert("site".into(), provincial.clone());
        buildings
            .repeated_types
            .insert("site".into(), vec![BuildingType::default(), provincial]);
        let names = count_trigger_names(&buildings, &Ideologies::default(), &Resources::default());
        assert_eq!(names.is_provincial(b"BUNKER"), Some(true));
        assert_eq!(names.is_provincial(b"Mill"), Some(false));
        assert_eq!(names.is_provincial(b"site"), None);
        assert_eq!(names.is_provincial(b"absent"), None);
    }
}
