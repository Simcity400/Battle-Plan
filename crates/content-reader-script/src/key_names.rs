
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

use content_reader_parser::parser::c_string;
use crate::{native::{self, Field, Kind, Shape}, scope::{self, Names, Scope}, trigger_keys::{self, CountTriggerNames, Dispatch, Dispatched, Key, Reader, ScopeStep}};

use crate::effect_names::EffectNames;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyNames {
    pub scripted: BTreeSet<Vec<u8>>,
    pub scripted_effects: BTreeSet<Vec<u8>>,
    pub counts: CountTriggerNames,
    pub countries: BTreeSet<Vec<u8>>,
    pub aliases: BTreeSet<Vec<u8>>,
    pub characters: BTreeSet<Vec<u8>>,
    pub province_buildings: BTreeMap<Vec<u8>, Option<bool>>,
    pub effects: EffectNames,
    pub sub_units: Option<crate::sub_units::SubUnits>,
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
    use crate::native::Token;

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
                operator: content_reader_parser::parser::Operator::Equals,
                current: &value,
            },
        )
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
