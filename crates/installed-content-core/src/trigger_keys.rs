//! Trigger dispatch checks registered classes before scopes, content counts and
//! scripted triggers. Bare names use ASCII case identity; quoted and numeric
//! keys do not enter keyword lookup. Immutable registries come from the verified
//! executable, while authored names come from the resolved content stack.

use engine_executable::executable_tables::RuntimeTable;
use std::collections::BTreeSet;

use crate::{
    buildings::Buildings,
    ideologies::Ideologies,
    parser::Item,
    resources::Resource,
    scope::Scope,
    scripted_triggers::{Lookup, ScriptedTriggers, Template},
};
pub static REGISTERED_TRIGGERS: RuntimeTable<&'static [(u32, &'static str)]> =
    RuntimeTable::new(|| {
        &engine_executable::executable_tables::get()
            .registries
            .triggers
    });
pub use engine_executable::executable_tables::TriggerReader as Reader;
pub static TRIGGER_CLASSES: RuntimeTable<&'static [(u32, u32, Reader)]> = RuntimeTable::new(|| {
    &engine_executable::executable_tables::get()
        .registries
        .trigger_classes
});
pub fn trigger_class(token: u32) -> Option<(u32, Reader)> {
    TRIGGER_CLASSES
        .binary_search_by_key(&token, |(id, _, _)| *id)
        .ok()
        .map(|index| (TRIGGER_CLASSES[index].1, TRIGGER_CLASSES[index].2))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnKey {
    Value(&'static str),
    BoundLocalization(&'static str),
}

impl OwnKey {
    pub fn name(self) -> &'static str {
        match self {
            Self::Value(name) | Self::BoundLocalization(name) => name,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyReader {
    ThenLoadKey(&'static [OwnKey]),
    Only(&'static [OwnKey]),
    Native(u64),
    Refused(u64),
}

const ITERATOR: KeyReader =
    KeyReader::ThenLoadKey(&[OwnKey::BoundLocalization("tooltip"), OwnKey::Value("count")]);
pub fn key_reader(token: u32) -> KeyReader {
    let Some((_, reader)) = trigger_class(token) else {
        return KeyReader::ThenLoadKey(&[]);
    };
    let registry = &engine_executable::executable_tables::get()
        .registries
        .trigger_key_readers;
    let key = registry
        .binary_search_by_key(&token, |&(id, _)| id)
        .ok()
        .map(|index| registry[index].1)
        .expect("registered trigger key reader");
    match (reader, key) {
        (Reader::Body, 0x1404A3B70)
        | (Reader::Body, 0x1404379E0)
        | (Reader::Body, 0x14045CE30)
        | (Reader::Body, 0x140473280)
        | (Reader::Body, 0x14045B0A0) => ITERATOR,
        (Reader::Body, 0x1404A3CC0) => {
            KeyReader::Only(&[OwnKey::Value("specialization"), OwnKey::Value("level")])
        }
        (Reader::Own(0x1413AA150), 0x140550030) => KeyReader::Only(&[
            OwnKey::Value("value"),
            OwnKey::Value("date"),
            OwnKey::Value("days"),
            OwnKey::Value("flag"),
        ]),
        (Reader::Body, 0x1404A3BB0) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("specialization")])
        }
        (Reader::Own(0x1404362A0), 0x140550030) => KeyReader::Only(&[
            OwnKey::Value("state"),
            OwnKey::Value("preparation"),
            OwnKey::Value("activated"),
        ]),
        (Reader::Body, 0x1404738E0) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("ai_area"),
            OwnKey::Value("strategic_region"),
            OwnKey::Value("continent"),
            OwnKey::Value("array"),
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("count"),
        ]),
        (Reader::Body, 0x14043AB40) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("buildings"),
            OwnKey::Value("resource"),
            OwnKey::Value("extracted"),
            OwnKey::Value("collection"),
        ]),
        (Reader::Body, 0x1404386F0) => KeyReader::Only(&[
            OwnKey::Value("path"),
            OwnKey::Value("controller_priority"),
            OwnKey::Value("level"),
            OwnKey::Value("fallback"),
            OwnKey::Value("target_state"),
            OwnKey::Value("start_state"),
            OwnKey::Value("start_province"),
            OwnKey::Value("target_province"),
            OwnKey::Value("build_only_on_allied"),
        ]),
        (Reader::Body, 0x14043A8F0) => {
            KeyReader::Only(&[OwnKey::Value("state"), OwnKey::Value("level")])
        }
        (Reader::Body, 0x140438DC0) => KeyReader::Native(0x140438DC0),
        (Reader::Body, 0x14043B580) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip_evaluation")])
        }
        (Reader::Body, 0x140437A20)
        | (Reader::Body, 0x140473AA0)
        | (Reader::Body, 0x14043A260)
        | (Reader::Own(0x1404360D0), 0x1413AAB60) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x14043AE10) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("only_imported"),
            OwnKey::Value("buildings"),
            OwnKey::Value("resource"),
            OwnKey::Value("extracted"),
        ]),
        (Reader::Body, 0x14043B0C0) => KeyReader::Only(&[
            OwnKey::Value("receiver"),
            OwnKey::Value("state"),
            OwnKey::Value("resources"),
        ]),
        (Reader::Body, 0x140439320) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("state"),
            OwnKey::Value("value"),
            OwnKey::Value("unit"),
            OwnKey::Value("tag"),
        ]),
        (Reader::Body, 0x14043C9F0) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("area"),
        ]),
        (Reader::Body, 0x140439840) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("unit_category"),
        ]),
        (Reader::Body, 0x14043A7B0) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("unit"),
            OwnKey::Value("archetype"),
        ]),
        (Reader::Body, 0x140521470) => KeyReader::Only(&[
            OwnKey::Value("collection"),
            OwnKey::Value("buildings"),
            OwnKey::Value("unit_category"),
            OwnKey::Value("equipment_ratio"),
            OwnKey::Value("stockpile"),
            OwnKey::Value("size"),
            OwnKey::Value("manpower"),
            OwnKey::Value("unit"),
        ]),
        (Reader::Body, 0x14043B810) | (Reader::Body, 0x140439740) => {
            KeyReader::Only(&[OwnKey::Value("ratio"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140439780) => {
            KeyReader::Only(&[OwnKey::Value("size"), OwnKey::Value("type")])
        }
        (Reader::Body, 0x140439980) => {
            KeyReader::Only(&[OwnKey::Value("type"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x140437120) => KeyReader::Only(&[
            OwnKey::Value("idea"),
            OwnKey::Value("limit"),
            OwnKey::Value("ignore"),
            OwnKey::Value("characters"),
        ]),
        (Reader::Body, 0x140473B30) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("province"),
            OwnKey::Value("building"),
            OwnKey::Value("include_locked"),
        ]),
        (Reader::Own(0x1413A9FD0), 0x1413AAB90) => KeyReader::ThenLoadKey(&[
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::BoundLocalization("not_tooltip"),
        ]),
        (Reader::Body, 0x140439BD0) => KeyReader::Native(0x140439BD0),
        (Reader::Body, 0x14043BC10) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("progress"), OwnKey::Value("focus")])
        }
        (Reader::Body, 0x14043CA60) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("state"),
        ]),
        (Reader::Body, 0x14043B540) => {
            KeyReader::Only(&[OwnKey::Value("count"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140439BA0) => {
            KeyReader::Only(&[OwnKey::Value("group"), OwnKey::Value("value")])
        }
        (Reader::Own(0x140436880), 0x140550030) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("type")])
        }
        (Reader::Body, 0x140473DA0) => KeyReader::Native(0x140473DA0),
        (Reader::Body, 0x140438440) | (Reader::Body, 0x140456270) | (Reader::Body, 0x140456240) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip")])
        }
        (Reader::Body, 0x14043B280) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("state"),
            OwnKey::Value("resource"),
            OwnKey::Value("delivered"),
        ]),
        (Reader::Body, 0x14043B500) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("category"), OwnKey::Value("technology")])
        }
        (Reader::Body, 0x140439AF0) => KeyReader::Only(&[
            OwnKey::Value("id"),
            OwnKey::Value("name"),
            OwnKey::Value("ruling_only"),
            OwnKey::Value("character"),
        ]),
        (Reader::Body, 0x140439240) => KeyReader::ThenLoadKey(&[OwnKey::Value("amount")]),
        (Reader::Body, 0x14043A120) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("equipment"),
            OwnKey::Value("from"),
            OwnKey::Value("archetype"),
        ]),
        (Reader::Body, 0x14043BAD0) => KeyReader::Refused(0x14043BAD0),
        (Reader::Body, 0x14043A930) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("modifier")])
        }
        (Reader::Body, 0x1404732C0) => KeyReader::Only(&[
            OwnKey::Value("province"),
            OwnKey::Value("building"),
            OwnKey::Value("level"),
        ]),
        (Reader::Body, 0x140439530) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140437B40) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("categories"),
            OwnKey::Value("slots"),
        ]),
        (Reader::Body, 0x140456200) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip"), OwnKey::Value("include_invisible")])
        }
        (Reader::Body, 0x140439A80) => {
            KeyReader::Only(&[OwnKey::Value("achievement"), OwnKey::Value("mod")])
        }
        (Reader::Own(0x14044CB90), 0x14044CCC0) => KeyReader::Only(&[OwnKey::Value("character")]),
        (Reader::Body, 0x140459B70) => KeyReader::ThenLoadKey(&[
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("include_invisible"),
            OwnKey::Value("count"),
        ]),
        (Reader::Body, 0x14043CC80) => KeyReader::Native(0x14043CC80),
        (Reader::Body, 0x140439000) => KeyReader::Native(0x140439000),
        (Reader::Body, 0x140439950) => KeyReader::Native(0x140439950),
        (Reader::Body, 0x1403B4000) => KeyReader::Native(0x1403B4000),
        (Reader::Body, 0x1404391E0) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("regions")])
        }
        (Reader::Body, 0x140438470) => KeyReader::Only(&[
            OwnKey::Value("type"),
            OwnKey::Value("amount"),
            OwnKey::Value("state"),
        ]),
        (Reader::Body, 0x140439280) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("state"),
            OwnKey::Value("border_state"),
        ]),
        (Reader::Body, 0x14043CC30) => {
            KeyReader::Only(&[OwnKey::Value("ratio"), OwnKey::Value("archetype")])
        }
        (Reader::Body, 0x14043A060) => KeyReader::Native(0x14043A060),
        (Reader::Body, 0x1403B0BE0) => KeyReader::Native(0x1403B0BE0),
        (Reader::Body, 0x14043C800) => KeyReader::Native(0x14043C800),
        (Reader::Body, 0x140436BE0) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("index"),
            OwnKey::Value("value"),
            OwnKey::Value("array"),
        ]),
        (Reader::Body, 0x140436E80) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip"), OwnKey::Value("array")])
        }
        (Reader::Own(0x1404360D0), 0x140473BD0) => {
            KeyReader::Only(&[OwnKey::Value("building"), OwnKey::Value("level")])
        }
        (Reader::Body, 0x1403B3C00) => KeyReader::Native(0x1403B3C00),
        (Reader::Own(0x140436150), 0x140439B60) => {
            KeyReader::Only(&[OwnKey::Value("modifier"), OwnKey::Value("scope")])
        }
        (Reader::Body, 0x1403AF530) => {
            KeyReader::Only(&[OwnKey::Value("amount"), OwnKey::Value("region")])
        }
        (Reader::Body, 0x14043BC90) => KeyReader::Only(&[
            OwnKey::Value("tooltip"),
            OwnKey::Value("ratio"),
            OwnKey::Value("other"),
            OwnKey::Value("sub_unit_def_weights"),
        ]),
        (Reader::Body, 0x14048B790) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("range")])
        }
        (Reader::Body, 0x140488180) => KeyReader::Only(&[OwnKey::Value("id")]),
        (Reader::Own(0x1404360D0), 0x14048B850) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x14048B800) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("side")])
        }
        (Reader::Body, 0x1404881B0) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("modifier")])
        }
        (Reader::Body, 0x140438870) => {
            KeyReader::Only(&[OwnKey::Value("opponent"), OwnKey::Value("thousands")])
        }
        (Reader::Body, 0x14043C2B0) => {
            KeyReader::Only(&[OwnKey::Value("sender"), OwnKey::Value("value")])
        }
        (Reader::Own(0x1404360D0), 0x140439AC0) => KeyReader::Native(0x140439AC0),
        (Reader::Own(0x1404360D0), 0x1404391B0) => KeyReader::Native(0x1404391B0),
        (Reader::Body, 0x140441BA0) => KeyReader::Native(0x140441BA0),
        (Reader::Body, 0x140441B10) => KeyReader::Only(&[
            OwnKey::Value("ratio"),
            OwnKey::Value("first"),
            OwnKey::Value("second"),
            OwnKey::Value("compare"),
        ]),
        (Reader::Body, 0x140441E20) => KeyReader::Native(0x140441E20),
        (Reader::Body, 0x140441D50) => KeyReader::Refused(0x140441D50),
        (Reader::Body, 0x1404419A0) => KeyReader::Refused(0x1404419A0),
        (Reader::Body, 0x140441DE0) => {
            KeyReader::Only(&[OwnKey::Value("tooltip"), OwnKey::Value("ribbon")])
        }
        (Reader::Own(0x140481E10), 0x140482D00) => KeyReader::Only(&[
            OwnKey::Value("type"),
            OwnKey::Value("need"),
            OwnKey::Value("var"),
        ]),
        (Reader::Own(0x140459B30), 0x140459C00) => KeyReader::Native(0x140459C00),
        (Reader::Own(0x140459B50), 0x140301870) => {
            KeyReader::Only(&[OwnKey::Value("trait"), OwnKey::Value("show_modifiers")])
        }
        (Reader::Own(0x140459B50), 0x140459C20) => KeyReader::Only(&[
            OwnKey::Value("check_mio_parent_completed"),
            OwnKey::Value("check_mio_mutually_exclusive"),
            OwnKey::Value("trait"),
            OwnKey::Value("show_modifiers"),
        ]),
        (Reader::Body, 0x140521200) => KeyReader::Only(&[
            OwnKey::Value("country"),
            OwnKey::Value("countries"),
            OwnKey::Value("collection"),
        ]),
        (Reader::Body, 0x14052D900) => {
            KeyReader::Only(&[OwnKey::Value("track"), OwnKey::Value("amount")])
        }
        (Reader::Body, 0x14052D6D0) => {
            KeyReader::Only(&[OwnKey::Value("amount"), OwnKey::Value("sub_doctrine")])
        }
        (Reader::Body, 0x140473100) | (Reader::Body, 0x1404A9690) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("target"),
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("count"),
        ]),
        (Reader::Body, 0x14043CE50) => {
            KeyReader::Only(&[OwnKey::Value("months"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140521170) => KeyReader::ThenLoadKey(&[
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("count"),
            OwnKey::Value("collection"),
        ]),
        (Reader::Body, 0x1404B8B80) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("leader")])
        }
        (Reader::Body, 0x14043C000) => KeyReader::Only(&[
            OwnKey::Value("target"),
            OwnKey::Value("state"),
            OwnKey::Value("value"),
        ]),
        (Reader::Body, 0x1403AF5B0) => {
            KeyReader::Only(&[OwnKey::Value("tag"), OwnKey::Value("token")])
        }
        (Reader::Own(0x140481570), 0x140482C70) => {
            KeyReader::Only(&[OwnKey::Value("var"), OwnKey::Value("scorer")])
        }
        (Reader::Own(0x1404819C0), 0x140482CB0) => KeyReader::Only(&[
            OwnKey::Value("array"),
            OwnKey::Value("scores"),
            OwnKey::Value("scorer"),
        ]),
        (Reader::Body, 0x140436AB0) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("original_tag"),
            OwnKey::Value("original_tag_to_check"),
            OwnKey::Value("count"),
            OwnKey::BoundLocalization("tooltip"),
        ]),
        (Reader::Body, 0x140439140) | (Reader::Body, 0x14043BA20) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("target")])
        }
        (Reader::Own(0x140482260), 0x140482D50) => KeyReader::Native(0x140482B30),
        (Reader::Own(0x1404360D0), 0x14043C260) => KeyReader::Native(0x14043C260),
        (Reader::Own(0x1404826B0), 0x140482D60) => KeyReader::Only(&[
            OwnKey::Value("min"),
            OwnKey::Value("max"),
            OwnKey::Value("var"),
            OwnKey::Value("integer"),
        ]),
        (Reader::Own(0x1404360D0), 0x14043BA90) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("operation")])
        }
        (Reader::Body, 0x140521450) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("input"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x1404B8C30) => KeyReader::Refused(0x1404B8C30),
        (Reader::Own(0x14044CC90), 0x14044CCF0) => KeyReader::Only(&[
            OwnKey::Value("province"),
            OwnKey::Value("include_field_marshal"),
        ]),
        (Reader::Body, 0x140473CF0) => KeyReader::Only(&[
            OwnKey::Value("count"),
            OwnKey::Value("states"),
            OwnKey::Value("types"),
            OwnKey::Value("exclude"),
        ]),
        (Reader::Own(address), _) => KeyReader::Refused(address),
        _ => KeyReader::ThenLoadKey(&[]),
    }
}
pub fn registered_trigger(text: &[u8]) -> Option<u32> {
    REGISTERED_TRIGGERS
        .iter()
        .find(|(_, name)| text.eq_ignore_ascii_case(name.as_bytes()))
        .map(|(token, _)| *token)
}
pub static EQUIPMENT_CATEGORIES: RuntimeTable<&'static [(u32, u64, &'static str)]> =
    RuntimeTable::new(|| {
        &engine_executable::executable_tables::get()
            .auxiliary
            .equipment_categories
    });
pub fn equipment_category_mask(token: u32) -> Option<u64> {
    EQUIPMENT_CATEGORIES
        .binary_search_by_key(&token, |(id, _, _)| *id)
        .ok()
        .map(|index| EQUIPMENT_CATEGORIES[index].1)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountKind {
    Building,
    Resource,
    Ideology,
    Equipment,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountTriggerNames {
    pub buildings: BTreeSet<Vec<u8>>,
    pub ideologies: BTreeSet<Vec<u8>>,
    pub ideology_types: BTreeSet<Vec<u8>>,
    pub ideologies_gap: Option<String>,
    pub resources: BTreeSet<Vec<u8>>,
}

impl CountTriggerNames {
    pub fn new(buildings: &Buildings, ideologies: &Ideologies, resources: &[Resource]) -> Self {
        Self {
            buildings: buildings
                .types
                .keys()
                .map(|name| name.as_bytes().to_ascii_lowercase())
                .collect(),
            ideologies: ideologies
                .groups()
                .0
                .iter()
                .map(|group| group.token.as_bytes().to_ascii_lowercase())
                .collect(),
            ideology_types: ideologies
                .groups()
                .0
                .iter()
                .flat_map(|group| &group.types)
                .map(|token| token.as_bytes().to_ascii_lowercase())
                .collect(),
            ideologies_gap: ideologies.groups().1.map(ToString::to_string),
            resources: resources
                .iter()
                .map(|resource| resource.token.as_bytes().to_ascii_lowercase())
                .collect(),
        }
    }
    pub fn count_kind(&self, text: &[u8]) -> Result<Option<CountKind>, String> {
        let folded = text.to_ascii_lowercase();
        Ok(if self.buildings.contains(&folded) {
            Some(CountKind::Building)
        } else if self.resources.contains(&folded) {
            Some(CountKind::Resource)
        } else if self.is_ideology(text)? {
            Some(CountKind::Ideology)
        } else {
            EQUIPMENT_CATEGORIES
                .iter()
                .find(|(token, _, name)| {
                    name.as_bytes() == folded
                        && !REGISTERED_TRIGGERS.iter().any(|(id, _)| id == token)
                })
                .map(|_| CountKind::Equipment)
        })
    }
    pub fn is_building(&self, text: &[u8]) -> bool {
        self.buildings.contains(&text.to_ascii_lowercase())
    }
    pub fn is_ideology_type(&self, text: &[u8]) -> Result<bool, String> {
        if self.ideology_types.contains(&text.to_ascii_lowercase()) {
            return Ok(true);
        }
        match &self.ideologies_gap {
            Some(gap) => Err(format!(
                "whether {} names an ideology type is unknown: {gap}",
                String::from_utf8_lossy(text)
            )),
            None => Ok(false),
        }
    }
    pub fn is_ideology(&self, text: &[u8]) -> Result<bool, String> {
        if self.ideologies.contains(&text.to_ascii_lowercase()) {
            return Ok(true);
        }
        match &self.ideologies_gap {
            Some(gap) => Err(format!(
                "whether {} names an ideology group is unknown: {gap}",
                String::from_utf8_lossy(text)
            )),
            None => Ok(false),
        }
    }
}
pub fn describe(kind: CountKind) -> &'static str {
    match kind {
        CountKind::Building => "a building count trigger",
        CountKind::Resource => "a resource count trigger",
        CountKind::Ideology => "an ideology support trigger",
        CountKind::Equipment => "an equipment category trigger",
    }
}
#[cfg(test)]
pub const SCRIPTED: &str = "a scripted trigger";
#[derive(Clone, Debug, Default)]
pub struct TriggerNames {
    pub scripted: ScriptedTriggers,
    pub counts: Option<CountTriggerNames>,
}
pub enum ScopeStep<E = String> {
    Scope(Scope),
    NotScope,
    Unknown(E),
}
pub enum Dispatch<C, T> {
    Coded,
    Scope(Scope),
    Count(C),
    Scripted(T),
    Absent,
}
pub struct Dispatched<C, T, E = String> {
    pub dispatch: Dispatch<C, T>,
    pub unless_scope: Option<E>,
}
pub struct Key<'a> {
    pub keyword: Option<&'a [u8]>,
    pub text: &'a [u8],
    pub quoted: bool,
    pub opens: bool,
}
pub fn dispatch<C, T>(
    key: Key<'_>,
    scope: impl FnOnce(&[u8], bool) -> ScopeStep,
    count: impl FnOnce(&[u8]) -> Result<Option<C>, String>,
    scripted: impl FnOnce(&[u8]) -> Option<T>,
) -> Result<Dispatched<C, T>, String> {
    dispatch_with_error(key, scope, count, scripted)
}

/// Runtime consumers carry their source errors through dispatch unchanged.
pub fn dispatch_with_error<C, T, E>(
    key: Key<'_>,
    scope: impl FnOnce(&[u8], bool) -> ScopeStep<E>,
    count: impl FnOnce(&[u8]) -> Result<Option<C>, E>,
    scripted: impl FnOnce(&[u8]) -> Option<T>,
) -> Result<Dispatched<C, T, E>, E> {
    let dispatched = |dispatch, unless_scope| Dispatched {
        dispatch,
        unless_scope,
    };
    if key.keyword.and_then(registered_trigger).is_some() {
        return Ok(dispatched(Dispatch::Coded, None));
    }
    let mut unless_scope = None;
    if key.opens {
        match scope(key.text, key.quoted) {
            ScopeStep::Scope(scope) => return Ok(dispatched(Dispatch::Scope(scope), None)),
            ScopeStep::NotScope => {}
            ScopeStep::Unknown(why) => unless_scope = Some(why),
        }
    }
    if let Some(keyword) = key.keyword
        && let Some(count) = count(keyword)?
    {
        return Ok(dispatched(Dispatch::Count(count), unless_scope));
    }
    Ok(dispatched(
        match scripted(key.text) {
            Some(template) => Dispatch::Scripted(template),
            None => Dispatch::Absent,
        },
        unless_scope,
    ))
}

impl TriggerNames {
    pub fn new(scripted: ScriptedTriggers, counts: CountTriggerNames) -> Self {
        Self {
            scripted,
            counts: Some(counts),
        }
    }
    pub fn dispatch<'a, C>(
        &'a self,
        item: &Item,
        scope: impl FnOnce(&[u8], bool) -> ScopeStep,
        count: impl FnOnce(&[u8]) -> Result<Option<C>, String>,
    ) -> Result<Dispatched<C, &'a Template>, String> {
        let keyword = item.keyword();
        dispatch(
            Key {
                keyword: keyword.as_ref().map(|keyword| keyword.text()),
                text: item.key.as_deref().unwrap_or_default(),
                quoted: item.key_quoted,
                opens: item.value.items().is_some(),
            },
            scope,
            count,
            |text| match self.scripted.lookup(text) {
                Lookup::Template(template) => Some(template),
                Lookup::Missing => None,
            },
        )
    }
}
pub fn cycle(stack: &[Vec<u8>], template: &Template) -> Option<String> {
    let folded = frame(template);
    let start = stack.iter().position(|name| *name == folded)?;
    let path: Vec<_> = stack[start..]
        .iter()
        .chain(std::iter::once(&folded))
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect();
    Some(format!(
        "scripted trigger {} calls itself ({}): evaluation has no depth guard, so the game recurses without end",
        String::from_utf8_lossy(&template.name),
        path.join(" -> ")
    ))
}
pub fn frame(template: &Template) -> Vec<u8> {
    template.name.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_and_categories_are_sorted_and_unique() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation");
        assert!(
            REGISTERED_TRIGGERS
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
        );
        let names: std::collections::BTreeSet<_> =
            REGISTERED_TRIGGERS.iter().map(|(_, name)| *name).collect();
        assert_eq!(names.len(), REGISTERED_TRIGGERS.len());
        assert!(
            names
                .iter()
                .all(|name| name.bytes().all(|b| !b.is_ascii_uppercase()))
        );
        assert_eq!(registered_trigger(b"AND"), Some(0x2968));
        assert_eq!(registered_trigger(b"Has_Tactic"), Some(0x41BC));
        assert_eq!(registered_trigger(b"fascism"), None);
        assert!(
            EQUIPMENT_CATEGORIES
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
        );
        let mut seen = 0_u64;
        for &(_, mask, _) in EQUIPMENT_CATEGORIES.iter() {
            assert_eq!(mask.count_ones(), 1);
            assert_eq!(seen & mask, 0);
            seen |= mask;
        }
        assert_eq!(equipment_category_mask(0x2781), Some(0x20));
        assert_eq!(equipment_category_mask(0x4C4A), None);
        for scope in [0x2C94, 0x298F, 0x283E] {
            assert!(REGISTERED_TRIGGERS.iter().all(|(id, _)| *id != scope));
        }
        assert_eq!(registered_trigger(b"if"), Some(0x27C3));
        assert!(TRIGGER_CLASSES.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(
            TRIGGER_CLASSES
                .iter()
                .all(|(id, _, _)| REGISTERED_TRIGGERS.iter().any(|(token, _)| token == id))
        );
        assert!(
            REGISTERED_TRIGGERS
                .iter()
                .all(|(id, _)| trigger_class(*id).is_some())
        );
        assert_eq!(key_reader(0x27C3), KeyReader::ThenLoadKey(&[]));
        assert_eq!(trigger_class(0x2A02), Some((0x1C, Reader::Assign)));
        assert_eq!(trigger_class(0x2968), Some((0, Reader::Body)));
        assert_eq!(trigger_class(0x2828), Some((0, Reader::Body)));
    }
    #[test]
    fn every_load_body_thunk_has_a_key_reader() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation");
        for &(token, _, reader) in TRIGGER_CLASSES.iter() {
            if matches!(
                reader,
                Reader::Own(
                    0x1404360D0
                        | 0x1413A9FD0
                        | 0x140481570
                        | 0x1404819C0
                        | 0x140481E10
                        | 0x140482260
                        | 0x1404826B0
                        | 0x140459B30
                )
            ) {
                assert!(
                    !matches!(key_reader(token), KeyReader::Refused(_)),
                    "trigger {token} enters a body without a key grammar"
                );
            }
        }
    }

    #[test]
    fn count_trigger_names_fold_the_loaded_databases() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation");
        let mut buildings = Buildings::default();
        buildings.types.insert(
            "Arms_Factory".into(),
            crate::buildings::BuildingType::default(),
        );
        let groups = crate::ideologies::parse(b"ideologies = { fascism = { } }");
        let ideologies = Ideologies::from_groups(groups.clone(), None);
        let resources = [Resource {
            token: "Steel".into(),
            icon_frame: None,
        }];
        let names = CountTriggerNames::new(&buildings, &ideologies, &resources);
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
        let names = CountTriggerNames::new(&buildings, &stopped, &resources);
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
    }
}
