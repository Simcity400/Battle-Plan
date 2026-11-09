//! Trigger-key classification against the content names available at each load point.
//!
//! Every trigger body is read at load. Each key uses the current value slot, including
//! the previous value when the key has no operator. Classification checks, in order:
//! 1. `else` and `else_if`: read and attach to the last `if`;
//! 2. the trigger registry ([`trigger_keys`]);
//! 3. scope text when the value slot holds `{`;
//! 4. a building, resource or ideology by token, or an equipment category;
//! 5. a scripted trigger by text;
//! 6. otherwise report an invalid trigger and skip the current block, adding no child.
//!
//! Country tags load first, followed by scripted-trigger names, tag-alias names,
//! ideologies, character tokens with their bodies skipped, resources and buildings.
//! Most trigger bodies use [`Point::Late`]; idea tags, static modifiers, equipment and
//! buildings use their own load points ([`KeyNames::at`]). Character bodies load after
//! ideas.

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

/// Content names available at one point in database loading.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyNames {
    pub buildings: Option<std::sync::Arc<buildings::Buildings>>,
    /// Folded scripted-trigger names.
    pub scripted: BTreeSet<Vec<u8>>,
    /// Scripted-effect names whose definition flag is set, loaded after idea tags.
    pub scripted_effects: BTreeSet<Vec<u8>>,
    pub counts: CountTriggerNames,
    /// Country tags with their exact declared bytes.
    pub countries: BTreeSet<Vec<u8>>,
    /// Tag-alias names with ASCII case folded for token lookup.
    pub aliases: BTreeSet<Vec<u8>>,
    /// Character tokens with ASCII case folded.
    pub characters: BTreeSet<Vec<u8>>,
    /// Provincial building flags for province-building-level triggers; `None` when
    /// repeated declarations disagree.
    pub province_buildings: BTreeMap<Vec<u8>, Option<bool>>,
    pub effects: effect_names::EffectNames,
    /// The sub-unit database after `common/units`, for effect division-template
    /// placement. `None` at earlier load points whose final roster is unavailable.
    pub sub_units: Option<crate::unit_leader::SubUnits>,
    /// Regiment, support and regimental-support column widths from the recorded
    /// NMilitary defines.
    pub template_columns: Option<Result<[i32; 3], String>>,
}

/// The load point at which a trigger body is read, identifying the names available
/// then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    /// `common/idea_tags`: after country tags and scripted-trigger names only.
    IdeaTags,
    /// `common/modifiers`: before resources and buildings, after country tags, aliases,
    /// character tokens, ideologies and scripted-trigger names.
    StaticModifiers,
    /// `common/units/equipment` and `common/buildings`: after resources, while building
    /// names are unavailable because their own triggers are read during loading.
    BeforeBuildings,
    /// All required database names are available.
    Late,
}

/// Key classification and the corresponding value reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    /// `else` or `else_if`: a conditional body that adds no child of its own.
    Conditional,
    Coded(u32, Reader),
    /// A scope whose value is read as a trigger body.
    Scope,
    /// A content count trigger or scripted trigger takes one value token and skips no
    /// block.
    OneValue,
    /// An invalid trigger adds no child and uses a reporting skip.
    Invalid,
    /// Classification cannot be determined here, with its cause.
    Unknown(String),
}

impl Load {
    /// Whether classification adds a child for the key.
    pub fn adds_child(&self) -> Option<bool> {
        match self {
            Self::Coded(..) | Self::Scope | Self::OneValue => Some(true),
            Self::Conditional | Self::Invalid => Some(false),
            Self::Unknown(_) => None,
        }
    }
}

/// Scope lookups over the load's names, recording questions those names cannot answer.
struct ScopeNames<'a> {
    names: &'a KeyNames,
    unknown: RefCell<Option<String>>,
}

impl Names for ScopeNames<'_> {
    /// Find a country tag by exact bytes, then a tag alias by token.
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

    /// Prefix lookup includes every name interned so far. The fixed prefixes recognized
    /// here are `mio`, `sp` and `scope`.
    fn is_keyword(&self, text: &str) -> bool {
        // Other prefixes depend on dynamic tokens that this load-point roster does not hold.
        if ["mio", "sp", "scope"]
            .iter()
            .any(|prefix| text.eq_ignore_ascii_case(prefix))
        {
            return true;
        }
        self.unknown.replace(Some(format!(
            "whether its prefix {text} is a lexer token is not known to the content read"
        )));
        false
    }
}

impl KeyNames {
    pub fn equipment_database(&self) -> Option<&std::sync::Arc<crate::equipment::Database>> {
        self.effects.equipment_database.as_ref()
    }

    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        // The readers before buildings read nothing of each other: they run at once, each as
        // this thread runs, and the first in this order that fails fails the read, as in turn.
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

    /// The result of parsing a key's scope text against these names.
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

/// Read the recorded integer defines used for unsigned column comparisons. Missing or
/// nonnumeric leaves give zero; a missing table or failed script is refused.
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

/// Country-tag files are read in file order with lexer mode 0. The loop peeks and stops
/// at `}` or the end. Each key whose C string is three bytes registers a tag; repeats
/// are reported. `dynamic_tags` only marks the beginning of dynamic tags. Values are
/// neither read nor skipped, so block contents become keys.
pub fn country_tags(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut tags = BTreeSet::new();
    // The files the database loader lists, each by its bytes.
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
        // Stop at an unquoted closing brace; values never hide nested three-byte keys.
        .take_while(|item| item.key.as_deref() != Some(b"}".as_slice()) || item.key_quoted)
        .filter_map(|item| {
            let key = c_string(item.key.as_deref().unwrap_or_default());
            (key.len() == 3).then(|| key.to_vec())
        })
        .collect()
}

const UNNAMED: &str = "the reader was given no content names, so whether it names a scope, a content count or a scripted trigger is not known";

/// Classify `field` in trigger dispatch order against the names loaded so far: `else`
/// and `else_if`, then [`trigger_keys::dispatch`]. Without names, a key that could name
/// content stays unknown.
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
            None => ScopeStep::Unknown(UNNAMED.into()),
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

    /// The readers run at once give exactly what they give in turn, over the installation with
    /// a mod layer that adds to several of them.
    #[test]
    fn names_read_at_once_equal_names_read_in_turn() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let game = std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).unwrap());
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

    /// Traced readers of one stack share one read of the names, each tracing, under its own
    /// purpose, exactly what reading them itself traces.
    #[test]
    fn traced_readers_of_a_stack_share_one_read_and_trace_what_it_read() {
        use std::sync::{Arc, Mutex};
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let game = std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).unwrap());
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
        // Another stack reads the names itself.
        let (alone, alone_log) = traced(&open(), "alone");
        assert!(!Arc::ptr_eq(&names, &alone.key_names().unwrap()));
        let expected = reads(&alone_log, "alone");
        assert!(!expected.is_empty());
        assert_eq!(reads(&a_log, "a"), expected);
        assert_eq!(reads(&b_log, "b"), expected);
    }

    /// A traced read that met a failure is not shared: a later traced reader reads the names
    /// itself, and gets them whole once the failure is gone.
    #[test]
    fn a_traced_read_that_met_a_failure_is_not_shared() {
        use std::sync::{Arc, Mutex};
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let game = std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).unwrap());
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
        // The verified script is away while the first reader reads the names.
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
        // The whole read is shared.
        assert!(Arc::ptr_eq(&whole, &traced("c").0.key_names().unwrap()));
        let _ = std::fs::remove_dir_all(&overlay);
    }

    /// Dispatch checks `else`, the registry, scope text only with `{`, counts by token,
    /// and scripted triggers by text before refusing. Tags compare exactly; aliases and
    /// characters fold case.
    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn keys_load_in_dispatch_order() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("set BATTLE_PLAN_GAME_DIR to the supported installation");
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
        assert_eq!(bare(b"OR", Kind::Open), Load::Coded(0x2969, Reader::Body));
        assert_eq!(
            bare(b"tag", Kind::Bare),
            Load::Coded(0x2A02, Reader::Assign)
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
        assert_eq!(
            load(None, b"tag", Kind::Bare, Kind::Bare),
            Load::Coded(0x2A02, Reader::Assign)
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

    /// Register every three-byte key, including block contents read as keys, until the
    /// loop stops at `}`.
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
