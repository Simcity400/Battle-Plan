
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{
    ResolvedLayers, buildings,
    parser::{
        c_string,
        native::{self, Field, Kind, Shape},
    },
    resources,
    scope::{self, Names, Scope},
    trigger_keys::{self, CountTriggerNames, Dispatch, Dispatched, Key, Reader, ScopeStep},
};

mod effect_names;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyNames {
    pub buildings: Option<std::sync::Arc<buildings::Buildings>>,
    pub scripted: BTreeSet<Vec<u8>>,
    pub scripted_effects: BTreeSet<Vec<u8>>,
    pub counts: CountTriggerNames,
    pub countries: BTreeSet<Vec<u8>>,
    pub aliases: BTreeSet<Vec<u8>>,
    pub characters: BTreeSet<Vec<u8>>,
    pub province_buildings: BTreeMap<Vec<u8>, Option<bool>>,
    pub effects: effect_names::EffectNames,
    pub sub_units: Option<crate::unit_leader::SubUnits>,
    pub template_columns: Option<Result<[i32; 3], String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    IdeaTags,
    StaticModifiers,
    BeforeBuildings,
    Late,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    Conditional,
    Coded(u32, Reader),
    Scope,
    OneValue,
    Invalid,
    Unknown(String),
}

impl Load {
    pub fn adds_child(&self) -> Option<bool> {
        match self {
            Self::Coded(..) | Self::Scope | Self::OneValue => Some(true),
            Self::Conditional | Self::Invalid => Some(false),
            Self::Unknown(_) => None,
        }
    }
}

fn is_scope_prefix(text: &str) -> bool {
    ["mio", "sp", "scope"]
        .iter()
        .any(|prefix| text.eq_ignore_ascii_case(prefix))
}

struct ScopeNames<'a> {
    names: &'a KeyNames,
    unknown: RefCell<Option<String>>,
}

impl Names for ScopeNames<'_> {
    fn is_country(&self, tag: &str) -> bool {
        self.names.countries.contains(tag.as_bytes())
            || self
                .names
                .aliases
                .contains(&tag.as_bytes().to_ascii_lowercase())
    }

    fn is_character(&self, token: &str) -> bool {
        self.names
            .characters
            .contains(&token.as_bytes().to_ascii_lowercase())
    }

    fn is_keyword(&self, text: &str) -> bool {
        if is_scope_prefix(text) {
            return true;
        }
        self.unknown.replace(Some(format!(
            "whether its prefix {text} is a lexer token is not known to the content read"
        )));
        false
    }
}

fn token_only_scope(text: &[u8], quoted: bool) -> ScopeStep {
    struct TokenOnly;

    impl Names for TokenOnly {
        fn is_country(&self, _tag: &str) -> bool {
            false
        }

        fn is_character(&self, _token: &str) -> bool {
            false
        }

        fn is_keyword(&self, text: &str) -> bool {
            is_scope_prefix(text)
        }
    }

    let Ok(text) = std::str::from_utf8(text) else {
        return ScopeStep::Unknown(UNNAMED.into());
    };
    match scope::parse(text, quoted, &TokenOnly) {
        Scope::Unknown(_) => ScopeStep::Unknown(UNNAMED.into()),
        scope => ScopeStep::Scope(scope),
    }
}

impl KeyNames {
    pub fn equipment_database(&self) -> Option<&std::sync::Arc<crate::equipment::Database>> {
        self.effects.equipment_database.as_ref()
    }

    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        use crate::parallel::{join, spawn};
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
            let scripted = spawn(scope, || crate::scripted_triggers::names(layers));
            let effects = spawn(scope, || crate::scripted_effects::names(layers));
            let countries = spawn(scope, || country_tags(layers));
            let aliases = spawn(scope, || crate::tag_aliases::names(layers));
            let characters = spawn(scope, || crate::characters::tokens(layers));
            let sub_units = spawn(scope, || crate::unit_leader::sub_units(layers));
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
        let mut names = Self {
            scripted: scripted?,
            scripted_effects: scripted_effects?,
            counts: CountTriggerNames::new(
                &buildings::Buildings::default(),
                &ideologies,
                &resources,
            ),
            countries: countries?,
            aliases: aliases?,
            characters: characters?,
            province_buildings: BTreeMap::new(),
            sub_units: Some(sub_units?),
            template_columns: Some(template_columns),
            ..Self::default()
        };
        if layers.paths()?.any(buildings::is_building_file) {
            let buildings = layers.product("building types", 1, &[], |layers| buildings::read_with(layers, &names))?;
            names.counts.buildings =
                CountTriggerNames::new(&buildings, &ideologies, &Default::default()).buildings;
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
            names.buildings = Some(buildings);
        }
        names.effects = effect_names::EffectNames::read(layers, &names)?;
        Ok(names)
    }

    pub fn at(&self, point: Point) -> Self {
        let mut names = self.clone();
        match point {
            Point::IdeaTags => {
                names.scripted_effects.clear();
                names.aliases.clear();
                names.characters.clear();
                names.sub_units = None;
                names.counts = CountTriggerNames::default();
                names.province_buildings.clear();
                names.effects.terrains.clear();
                names.effects.terrains_gap = None;
                names.effects.equipment.clear();
                names.effects.equipment_database = None;
                names.effects.equipment_gap = None;
                names.effects.equipment_upgrades.clear();
                names.effects.equipment_upgrades_gap = None;
                names.effects.modifiers = names.effects.early_modifiers.clone();
                names.effects.modifiers_gap = names.effects.early_modifiers_gap.clone();
            }
            Point::StaticModifiers | Point::BeforeBuildings => {
                names.sub_units = None;
                names.counts.buildings.clear();
                names.province_buildings.clear();
                if point == Point::StaticModifiers {
                    names.counts.resources.clear();
                    names.counts.resources_gap = None;
                }
            }
            Point::Late => {}
        }
        names
    }

    pub(crate) fn scope(&self, text: &[u8], quoted: bool) -> ScopeStep {
        let Ok(text) = std::str::from_utf8(text) else {
            return ScopeStep::Unknown(format!(
                "key {} is not UTF-8, so whether it is a scope is not read",
                String::from_utf8_lossy(text)
            ));
        };
        let names = ScopeNames {
            names: self,
            unknown: RefCell::new(None),
        };
        match scope::parse(text, quoted, &names) {
            Scope::Unknown(_) => match names.unknown.into_inner() {
                Some(why) => {
                    ScopeStep::Unknown(format!("whether key {text} is a scope is unknown: {why}"))
                }
                None => ScopeStep::NotScope,
            },
            scope => ScopeStep::Scope(scope),
        }
    }
}

fn template_columns(layers: &ResolvedLayers) -> Result<[i32; 3], String> {
    let read = || -> io::Result<[i32; 3]> {
        let defines = crate::defines::read(layers)?;
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
    for path in crate::script_files::paths(layers, "common/country_tags/", Some(".txt"))? {
        tags.extend(file_tags(&path.read(layers)?));
    }
    Ok(tags)
}

pub fn file_tags(input: &[u8]) -> Vec<Vec<u8>> {
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(()),
        None,
        &mut |(), _| Shape::One,
    );
    read.items
        .iter()
        .take_while(|item| item.key.as_deref() != Some(b"}".as_slice()) || item.key_quoted)
        .filter_map(|item| {
            let key = c_string(item.key.as_deref().unwrap_or_default());
            (key.len() == 3).then(|| key.to_vec())
        })
        .collect()
}

const UNNAMED: &str = "the reader was given no content names, so whether it names a scope, a content count or a scripted trigger is not known";

pub fn load_key(names: Option<&KeyNames>, field: &Field<'_>) -> Load {
    let keyword = field.keyword();
    if matches!(keyword.as_deref(), Some(b"else" | b"else_if")) {
        return Load::Conditional;
    }
    let dispatched = trigger_keys::dispatch(
        Key {
            keyword: keyword.as_deref(),
            text: c_string(&field.key.text),
            quoted: field.key.kind == Kind::Quoted,
            opens: field.opens(),
        },
        |text, quoted| match names {
            Some(names) => names.scope(text, quoted),
            None => token_only_scope(text, quoted),
        },
        |keyword| match names {
            Some(names) => names.counts.count_kind(keyword),
            None => Err(UNNAMED.into()),
        },
        |text| {
            names
                .is_some_and(|names| {
                    names
                        .scripted
                        .contains(&c_string(text).to_ascii_lowercase())
                })
                .then_some(())
        },
    );
    let Dispatched {
        dispatch,
        unless_scope,
    } = match dispatched {
        Ok(dispatched) => dispatched,
        Err(why) => return Load::Unknown(why),
    };
    match (dispatch, unless_scope) {
        (Dispatch::Coded, _) => {
            let token = keyword
                .as_deref()
                .and_then(trigger_keys::registered_trigger)
                .unwrap_or_default();
            Load::Coded(
                token,
                trigger_keys::trigger_class(token)
                    .expect("every registered trigger has a class reader")
                    .1,
            )
        }
        (Dispatch::Scope(_), _) => Load::Scope,
        (_, Some(why)) => Load::Unknown(why),
        (Dispatch::Count(_), None) | (Dispatch::Scripted(()), None) => Load::OneValue,
        (Dispatch::Absent, None) if names.is_none() => Load::Unknown(UNNAMED.into()),
        (Dispatch::Absent, None) => Load::Invalid,
    }
}

#[cfg(any(test, feature = "layer-fixtures"))]
impl KeyNames {
    pub fn unknown() -> Self {
        let mut names = Self::default();
        names.counts.ideologies_gap = Some("common/ideologies/a.txt is unknown from line 1".into());
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::native::Token;

    fn load(names: Option<&KeyNames>, key: &[u8], kind: Kind, value: Kind) -> Load {
        let token = |kind, text: &[u8]| Token {
            kind,
            text: text.to_vec(),
            line: 1,
            quoted: kind == Kind::Quoted,
            byte_end: 0,
        };
        let (key, value) = (token(kind, key), token(value, b"x"));
        load_key(
            names,
            &Field {
                key: &key,
                value: &value,
                fresh: true,
                effect_scope: None,
                operator: crate::parser::Operator::Equals,
                current: &value,
            },
        )
    }

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
        crate::test_game::enter();
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
        let names = KeyNames::read(&layers).unwrap();
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
        let counts =
            CountTriggerNames::new(&buildings::Buildings::default(), &ideologies, &resources);
        assert_eq!(names.counts.resources, counts.resources);
        assert_eq!(names.counts.ideologies, counts.ideologies);
        assert_eq!(names.counts.ideology_types, counts.ideology_types);
        let _ = std::fs::remove_dir_all(&overlay);
    }

    // quarantine: #154 a synthetic base does not retain the traced key-name product
    #[test]
    #[ignore = "#154"]
    fn traced_readers_of_a_stack_share_one_read_and_trace_what_it_read() {
        use std::sync::{Arc, Mutex};
        crate::test_game::enter();
        let installed = synthetic_base();
        let game = installed.game_dir.clone();
        let open = || ResolvedLayers::from_folders(&[(game.as_path(), &[][..])]).unwrap();
        type Log = Arc<Mutex<Vec<crate::content::ContentRead>>>;
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
    }

    // quarantine: #154 a synthetic base does not retain the traced key-name product
    #[test]
    #[ignore = "#154"]
    fn a_traced_read_that_met_a_failure_is_not_shared() {
        use std::sync::{Arc, Mutex};
        crate::test_game::enter();
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
                && matches!(read.outcome, crate::content::ReadOutcome::Bytes { .. })
        }));
        assert!(Arc::ptr_eq(&whole, &traced("c").0.key_names().unwrap()));
        let _ = std::fs::remove_dir_all(&overlay);
    }

    #[test]
    fn keys_load_in_dispatch_order() {
        crate::test_game::with(|game| game.trigger("tag", 0, Reader::Assign, 0));
        let or = trigger_keys::registered_trigger(b"or").unwrap();
        let tag = trigger_keys::registered_trigger(b"tag").unwrap();
        let mut names = KeyNames {
            scripted: [b"my_check".to_vec(), b"ger".to_vec()].into(),
            countries: [b"GER".to_vec()].into(),
            aliases: [b"ali".to_vec()].into(),
            characters: [b"ger_hitler".to_vec()].into(),
            ..KeyNames::default()
        };
        names.counts.buildings.insert(b"arms_factory".to_vec());
        names.counts.resources.insert(b"steel".to_vec());
        let bare = |key: &[u8], value| load(Some(&names), key, Kind::Bare, value);
        assert_eq!(bare(b"Else_If", Kind::Open), Load::Conditional);
        assert_eq!(bare(b"OR", Kind::Open), Load::Coded(or, Reader::Body));
        assert_eq!(
            bare(b"tag", Kind::Bare),
            Load::Coded(tag, Reader::Assign)
        );
        assert_eq!(bare(b"GER", Kind::Open), Load::Scope);
        assert!(matches!(bare(b"GER", Kind::Bare), Load::OneValue));
        assert!(matches!(bare(b"ger", Kind::Open), Load::OneValue));
        assert_eq!(bare(b"ALI", Kind::Open), Load::Scope);
        assert_eq!(bare(b"GER_Hitler", Kind::Open), Load::Scope);
        assert_eq!(bare(b"ROOT.owner", Kind::Open), Load::Scope);
        assert!(matches!(bare(b"Arms_Factory", Kind::Open), Load::OneValue));
        assert!(matches!(bare(b"My_Check", Kind::Open), Load::OneValue));
        assert_eq!(bare(b"nothing", Kind::Open), Load::Invalid);
        let quoted = |key: &[u8]| load(Some(&names), key, Kind::Quoted, Kind::Open);
        assert_eq!(quoted(b"arms_factory"), Load::Invalid);
        assert!(matches!(quoted(b"my_check"), Load::OneValue));
        assert_eq!(quoted(b"GER"), Load::Scope);
        let prefixed = bare(b"foo:bar", Kind::Open);
        assert!(
            matches!(&prefixed, Load::Unknown(why) if why.contains("lexer token")),
            "{prefixed:?}"
        );
        assert!(matches!(
            load(None, b"GER", Kind::Bare, Kind::Open),
            Load::Unknown(_)
        ));
        assert_eq!(load(None, b"ROOT", Kind::Bare, Kind::Open), Load::Scope);
        assert_eq!(load(None, b"mio:test", Kind::Bare, Kind::Open), Load::Scope);
        assert!(matches!(
            load(None, b"ROOT", Kind::Bare, Kind::Bare),
            Load::Unknown(_)
        ));
        assert_eq!(
            load(None, b"tag", Kind::Bare, Kind::Bare),
            Load::Coded(tag, Reader::Assign)
        );
        let gap = KeyNames::unknown();
        assert!(matches!(
            load(Some(&gap), b"fascism", Kind::Bare, Kind::Bare),
            Load::Unknown(_)
        ));
        let early = names.at(Point::BeforeBuildings);
        assert_eq!(
            load(Some(&early), b"arms_factory", Kind::Bare, Kind::Open),
            Load::Invalid
        );
        let static_modifiers = names.at(Point::StaticModifiers);
        assert_eq!(
            load(Some(&static_modifiers), b"FROM", Kind::Bare, Kind::Open),
            Load::Scope
        );
        assert_eq!(
            load(Some(&static_modifiers), b"GER", Kind::Bare, Kind::Open),
            Load::Scope
        );
        for key in [b"arms_factory".as_slice(), b"steel"] {
            assert_eq!(
                load(Some(&static_modifiers), key, Kind::Bare, Kind::Open),
                Load::Invalid
            );
        }
        assert_eq!(
            load(Some(&early), b"steel", Kind::Bare, Kind::Open),
            Load::OneValue
        );
        let tags = names.at(Point::IdeaTags);
        assert_eq!(
            load(Some(&tags), b"ALI", Kind::Bare, Kind::Open),
            Load::Invalid
        );
        assert_eq!(
            load(Some(&tags), b"GER", Kind::Bare, Kind::Open),
            Load::Scope
        );
    }

    #[test]
    fn country_tag_files_register_three_byte_keys() {
        assert_eq!(
            file_tags(
                b"GER = \"countries/Germany.txt\" dynamic_tags = yes D01 = x LONG = y \"SOV\" = z"
            ),
            [b"GER".to_vec(), b"D01".to_vec(), b"SOV".to_vec()]
        );
        assert_eq!(file_tags(b"A = { ENG = x } FRA = y"), [b"ENG".to_vec()]);
    }
}
