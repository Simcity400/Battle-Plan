//! What CTrigger::LoadKey (1405434F0) makes of a trigger key while the
//! content loads, and the content names it reads to decide.
//!
//! Every trigger body is read at load (CTrigger::LoadBody 1405431C0): each
//! key 1424AC5E0 reads goes to LoadKey with the reader's value slot (+0xC0)
//! as it stands, the previous value when the key was read without an
//! operator. LoadKey looks, in order:
//! 1. `else` (0x36D3) and `else_if` (0x38ED): a CIfTrigger, read by its
//!    reader and attached to the last `if`;
//! 2. the trigger registry, by the key's token ([`trigger_keys`]);
//! 3. with a `{` in the value slot, the key's text as a scope (14146B860 ->
//!    14146DAA0): a CScopeTrigger (14138C3A0);
//! 4. a building (140676C60), resource (140AB2EC0) or ideology
//! 5. a scripted trigger by its text (140AA78C0, the template's +0x58 flag);
//! 6. otherwise "Invalid trigger" and RecordError (1424ACF90), which skips
//!    a block the lexer stands on (1424AD270): the key adds no child.
//!
//! The names these read are the databases CBuildingDatabase::load
//! (14018B610) has loaded when the body is read: the country tags
//! (14070F430) first, the scripted-trigger names (their name pass), the tag
//! alias names (their name pass), the ideologies, the character tokens
//! (CCharacterTemplateDatabase::InitFromDirectory 1406B0120, bodies
//! skipped), the resources and then the buildings. Every trigger body the
//! port reads is read after the buildings ([`Point::Late`]) but the idea
//! tags', the equipment's and the buildings' own ([`KeyNames::at`]); the
//! character bodies are read by PostFinalize (1406B02E0), after the ideas.

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

/// The content names a trigger key is read against, at one point of the
/// load.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyNames {
    /// The scripted-trigger names, folded ([`crate::scripted_triggers`]'s
    /// name pass).
    pub scripted: BTreeSet<Vec<u8>>,
    /// Scripted-effect names with +0x58 set, loaded fifth in 14018B610,
    /// after idea tags (14018CF67 -> 140193190).
    pub scripted_effects: BTreeSet<Vec<u8>>,
    /// The building, resource and ideology names loaded by then.
    pub counts: CountTriggerNames,
    /// The country tags, exactly as declared (FindCountryIndexByTag
    /// 14070F050 hashes the bytes).
    pub countries: BTreeSet<Vec<u8>>,
    /// The tag alias names, folded (found by token, 140BA53F0 ->
    /// 1424A7060).
    pub aliases: BTreeSet<Vec<u8>>,
    /// The character tokens, folded (1406B0030 finds a character by token).
    pub characters: BTreeSet<Vec<u8>>,
    /// Provincial building flags for CAnyProvinceBuildingLevelTrigger
    /// (140471900) and non_damaged_building_level (140472210);
    /// None when repeated declarations disagree.
    pub province_buildings: BTreeMap<Vec<u8>, Option<bool>>,
    /// The sub-unit database after `common/units` (14018F865), for effect
    /// division-template placement (140B937A0 / 1403AF6B0). None at an
    /// earlier load point whose final roster is unavailable.
    pub sub_units: Option<crate::unit_leader::SubUnits>,
    /// Regiments, support and regimental-support column widths from the
    /// recorded NMilitary defines (140906D10 / 140907130 / 140909230).
    pub template_columns: Option<Result<[i32; 3], String>>,
}

/// Where in 14018B610 a trigger body is read, for the names loaded by then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    /// `common/idea_tags` (140A31EB0), fourth: after the country tags and
    /// the scripted-trigger names only.
    IdeaTags,
    /// `common/units/equipment` and `common/buildings` (14067AE90), after
    /// the resources: no building is found (the building database's own
    /// triggers are read while it loads, and no vanilla or mod building
    /// trigger names a building).
    BeforeBuildings,
    /// After the buildings: every other trigger body the port reads.
    Late,
}

/// What LoadKey makes of a key, for the reader of its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    /// `else` or `else_if`: a CIfTrigger whose reader (slot 5) reads a
    /// body; it adds no child of its own.
    Conditional,
    /// A coded trigger by its token, with its class's value reader where
    /// [`trigger_keys::TRIGGER_CLASSES`] records it.
    Coded(u32, Reader),
    /// A scope: a CScopeTrigger, whose reader is LoadBody.
    Scope,
    /// A content count trigger or a scripted trigger: its reader takes one
    /// value token and skips no block (ReadTriggerComparison 141394DF0,
    /// CSimpleAssignTrigger::vf5 141395A40 -> 1424ACD40).
    OneValue,
    /// "Invalid trigger": no child, and RecordError's skip.
    Invalid,
    /// What LoadKey finds is not known here, and why.
    Unknown(String),
}

impl Load {
    /// Whether LoadKey adds a child for the key (140120090).
    pub fn adds_child(&self) -> Option<bool> {
        match self {
            Self::Coded(..) | Self::Scope | Self::OneValue => Some(true),
            Self::Conditional | Self::Invalid => Some(false),
            Self::Unknown(_) => None,
        }
    }
}

/// The scope step's lookups (14146DB00) over the load's names, recording a
/// question the names cannot answer.
struct ScopeNames<'a> {
    names: &'a KeyNames,
    unknown: RefCell<Option<String>>,
}

impl Names for ScopeNames<'_> {
    /// FindCountryIndexByTag (14070F050) by the exact bytes, then the
    /// alias database by token (140BA53F0).
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

    /// 1424A92B0 finds the prefix among the lexer's tokens, which hold every
    /// name the content interned so far; only the prefixes 14146E4E0
    /// compares (`mio` 0x4D37, `sp` 0x4D38, `scope` 0x2996) are static
    /// keywords the port knows.
    fn is_keyword(&self, text: &str) -> bool {
        if ["mio", "sp", "scope"]
            .iter()
            .any(|prefix| text.eq_ignore_ascii_case(prefix))
        {
            return true;
        }
        self.unknown.replace(Some(format!(
            "whether its prefix {text} is a lexer token (1424A92B0) is not known to the content read"
        )));
        false
    }
}

impl KeyNames {
    /// The names every trigger body after the buildings is read against.
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let (ideologies, _) = crate::ideologies::read(layers)?;
        let (resources, _) = resources::read(layers)?;
        let mut names = Self {
            scripted: crate::scripted_triggers::names(layers)?,
            scripted_effects: crate::scripted_effects::names(layers)?,
            counts: CountTriggerNames::new(
                &buildings::Buildings::default(),
                &ideologies,
                &resources,
            ),
            countries: country_tags(layers)?,
            aliases: crate::tag_aliases::names(layers)?,
            characters: crate::characters::tokens(layers)?,
            province_buildings: BTreeMap::new(),
            sub_units: Some(crate::unit_leader::sub_units(layers)?),
            template_columns: Some(template_columns(layers)),
        };
        // A stack without a building file declares no building.
        if layers.paths().any(buildings::is_building_file) {
            let buildings = buildings::read_with(layers, &names)?;
            names.counts.buildings = CountTriggerNames::new(&buildings, &ideologies, &[]).buildings;
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
        }
        Ok(names)
    }

    /// These names as they stand at `point` of the load: a database not
    /// loaded by then holds no name, so a key naming one is invalid there.
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
            }
            Point::BeforeBuildings => {
                names.sub_units = None;
                names.counts.buildings.clear();
                names.province_buildings.clear();
            }
            Point::Late => {}
        }
        names
    }

    /// Where the scope step (14146DB00 -> 14146DAA0) ends for a key's text
    /// over these names.
    pub(crate) fn scope(&self, text: &[u8], quoted: bool) -> ScopeStep {
        let Ok(text) = std::str::from_utf8(text) else {
            return ScopeStep::Unknown(format!(
                "key {} is not UTF-8, so whether it is a scope (14146DB00) is not read",
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

/// LuaTable_ReadInt32 (14205B690), registered at 140906D10, 140907130 and
/// 140909230 for 1403AF6B0's unsigned column comparisons. Missing/non-number
/// leaves take the getter's zero; a missing table or failed script is refused.
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

/// The country tags CCountryDatabase (14070F430) registers: every
/// `common/country_tags` file (Vfs_EnumerateRecursive 1424C7630, `.txt`),
/// each read by load_tags (14070E900) in lexer mode 0 (1424A63A0). Its loop
/// peeks and stops at `}` or the end; each ReadKeyOperatorValue (1424AC5E0)
/// key whose C string is three bytes is a tag (a repeated one is logged);
/// `dynamic_tags` (0x309F) only marks where the dynamic tags start. No
/// value is read or skipped, so a block's contents are read as keys.
pub fn country_tags(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut tags = BTreeSet::new();
    let paths: Vec<String> = layers
        .paths()
        .filter(|path| path.starts_with("common/country_tags/") && path.ends_with(".txt"))
        .map(str::to_owned)
        .collect();
    for path in paths {
        tags.extend(file_tags(&layers.read(&path)?));
    }
    Ok(tags)
}

/// One country tag file's tags ([`country_tags`]).
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
        // The loop stops at a `}` it peeks, where the replay's file loop
        // reads it as a key.
        .take_while(|item| item.key.as_deref() != Some(b"}".as_slice()) || item.key_quoted)
        .filter_map(|item| {
            let key = c_string(item.key.as_deref().unwrap_or_default());
            (key.len() == 3).then(|| key.to_vec())
        })
        .collect()
}

/// Why a key that could name content is unknown to a reader given no
/// names.
const UNNAMED: &str = "the reader was given no content names, so whether it names a scope, a content count or a scripted trigger is not known";

/// CTrigger::LoadKey's dispatch of `field`, in its order, against the
/// content names loaded by then: `else` and `else_if`, then the one
/// classifier ([`trigger_keys::dispatch`]). `None` where the reader was
/// given no names, so that a key that could name content is unknown.
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

#[cfg(test)]
impl KeyNames {
    /// Names under which whether a bare key names content is unknown: an
    /// ideology file stops short, so any name not read could be a group.
    pub(crate) fn unknown() -> Self {
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

    /// LoadKey's order (1405434F0): `else`, the registry, a scope only with
    /// a `{`, the counts by token, the scripted triggers by text, else
    /// invalid; tags exact, aliases and characters folded.
    #[test]
    fn keys_load_in_loadkeys_order() {
        let mut names = KeyNames {
            scripted: [b"my_check".to_vec(), b"ger".to_vec()].into(),
            countries: [b"GER".to_vec()].into(),
            aliases: [b"ali".to_vec()].into(),
            characters: [b"ger_hitler".to_vec()].into(),
            ..KeyNames::default()
        };
        names.counts.buildings.insert(b"arms_factory".to_vec());
        let bare = |key: &[u8], value| load(Some(&names), key, Kind::Bare, value);
        assert_eq!(bare(b"Else_If", Kind::Open), Load::Conditional);
        assert_eq!(bare(b"OR", Kind::Open), Load::Coded(0x2969, Reader::Body));
        assert_eq!(
            bare(b"tag", Kind::Bare),
            Load::Coded(0x2A02, Reader::Assign)
        );
        assert_eq!(bare(b"GER", Kind::Open), Load::Scope);
        // Without a block no scope is asked: the scripted trigger by text.
        assert!(matches!(bare(b"GER", Kind::Bare), Load::OneValue));
        // A tag is exact; an alias and a character are found by token.
        assert!(matches!(bare(b"ger", Kind::Open), Load::OneValue));
        assert_eq!(bare(b"ALI", Kind::Open), Load::Scope);
        assert_eq!(bare(b"GER_Hitler", Kind::Open), Load::Scope);
        assert_eq!(bare(b"ROOT.owner", Kind::Open), Load::Scope);
        assert!(matches!(bare(b"Arms_Factory", Kind::Open), Load::OneValue));
        assert!(matches!(bare(b"My_Check", Kind::Open), Load::OneValue));
        assert_eq!(bare(b"nothing", Kind::Open), Load::Invalid);
        // A quoted key finds no token: only the text lookups.
        let quoted = |key: &[u8]| load(Some(&names), key, Kind::Quoted, Kind::Open);
        assert_eq!(quoted(b"arms_factory"), Load::Invalid);
        assert!(matches!(quoted(b"my_check"), Load::OneValue));
        assert_eq!(quoted(b"GER"), Load::Scope);
        // What the names cannot tell.
        let prefixed = bare(b"foo:bar", Kind::Open);
        assert!(
            matches!(&prefixed, Load::Unknown(why) if why.contains("1424A92B0")),
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
        // Before the buildings are loaded no key names one.
        let early = names.at(Point::BeforeBuildings);
        assert_eq!(
            load(Some(&early), b"arms_factory", Kind::Bare, Kind::Open),
            Load::Invalid
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

    /// load_tags (14070E900): every three-byte key, a block's contents read
    /// as keys, up to a `}` the loop stops at.
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
