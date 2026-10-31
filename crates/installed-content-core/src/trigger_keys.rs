//! Trigger dispatch checks registered classes before scopes, content counts and
//! scripted triggers. Bare names use ASCII case identity; quoted and numeric
//! keys do not enter keyword lookup. Immutable registries come from the verified
//! executable, while authored names come from the resolved content stack.

use engine_executable::executable_tables::RuntimeTable;
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    buildings::Buildings,
    ideologies::Ideologies,
    parser::Item,
    resources::Resources,
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
        (Reader::Body, 0x1404C1800)
        | (Reader::Body, 0x140455960)
        | (Reader::Body, 0x14047AF30)
        | (Reader::Body, 0x140491050)
        | (Reader::Body, 0x1404791A0) => ITERATOR,
        (Reader::Body, 0x1404C1950) => {
            KeyReader::Only(&[OwnKey::Value("specialization"), OwnKey::Value("level")])
        }
        (Reader::Own(0x141446170), 0x140597260) => KeyReader::Only(&[
            OwnKey::Value("value"),
            OwnKey::Value("date"),
            OwnKey::Value("days"),
            OwnKey::Value("flag"),
        ]),
        (Reader::Body, 0x1404C1840) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("specialization")])
        }
        (Reader::Own(0x140454220), 0x140597260) => KeyReader::Only(&[
            OwnKey::Value("state"),
            OwnKey::Value("preparation"),
            OwnKey::Value("activated"),
        ]),
        (Reader::Body, 0x1404916B0) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("ai_area"),
            OwnKey::Value("strategic_region"),
            OwnKey::Value("continent"),
            OwnKey::Value("array"),
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("count"),
        ]),
        (Reader::Body, 0x140458AB0) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("buildings"),
            OwnKey::Value("resource"),
            OwnKey::Value("extracted"),
            OwnKey::Value("collection"),
        ]),
        (Reader::Body, 0x140456670) => KeyReader::Only(&[
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
        (Reader::Body, 0x140458860) => {
            KeyReader::Only(&[OwnKey::Value("state"), OwnKey::Value("level")])
        }
        (Reader::Body, 0x140456D40) => KeyReader::Native(0x140456D40),
        (Reader::Body, 0x140459520) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip_evaluation")])
        }
        (Reader::Body, 0x1404559A0)
        | (Reader::Body, 0x140491870)
        | (Reader::Body, 0x1404581D0)
        | (Reader::Own(0x140454050), 0x141446B80) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x140458D90) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("only_imported"),
            OwnKey::Value("buildings"),
            OwnKey::Value("resource"),
            OwnKey::Value("extracted"),
        ]),
        (Reader::Body, 0x140459050) => KeyReader::Only(&[
            OwnKey::Value("receiver"),
            OwnKey::Value("state"),
            OwnKey::Value("resources"),
        ]),
        (Reader::Body, 0x1404572a0) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("state"),
            OwnKey::Value("value"),
            OwnKey::Value("unit"),
            OwnKey::Value("tag"),
        ]),
        (Reader::Body, 0x14045A970) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("area"),
        ]),
        (Reader::Body, 0x1404577c0) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("unit_category"),
        ]),
        (Reader::Body, 0x140458720) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("unit"),
            OwnKey::Value("archetype"),
        ]),
        (Reader::Body, 0x140567820) => KeyReader::Only(&[
            OwnKey::Value("collection"),
            OwnKey::Value("buildings"),
            OwnKey::Value("unit_category"),
            OwnKey::Value("equipment_ratio"),
            OwnKey::Value("stockpile"),
            OwnKey::Value("size"),
            OwnKey::Value("manpower"),
            OwnKey::Value("unit"),
        ]),
        (Reader::Body, 0x140459790) | (Reader::Body, 0x1404576c0) => {
            KeyReader::Only(&[OwnKey::Value("ratio"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140457700) => {
            KeyReader::Only(&[OwnKey::Value("size"), OwnKey::Value("type")])
        }
        (Reader::Body, 0x140457900) => {
            KeyReader::Only(&[OwnKey::Value("type"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x1404550a0) => KeyReader::Only(&[
            OwnKey::Value("idea"),
            OwnKey::Value("limit"),
            OwnKey::Value("ignore"),
            OwnKey::Value("characters"),
        ]),
        (Reader::Body, 0x140491900) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("province"),
            OwnKey::Value("building"),
            OwnKey::Value("include_locked"),
        ]),
        (Reader::Own(0x141445FF0), 0x141446BB0) => KeyReader::ThenLoadKey(&[
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::BoundLocalization("not_tooltip"),
        ]),
        (Reader::Body, 0x140457B50) => KeyReader::Native(0x140457B50),
        (Reader::Body, 0x140459B90) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("progress"), OwnKey::Value("focus")])
        }
        (Reader::Body, 0x14045A9E0) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("state"),
        ]),
        (Reader::Body, 0x1404594E0) => {
            KeyReader::Only(&[OwnKey::Value("count"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140457B20) => {
            KeyReader::Only(&[OwnKey::Value("group"), OwnKey::Value("value")])
        }
        (Reader::Own(0x140454800), 0x140597260) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("type")])
        }
        (Reader::Body, 0x140491BA0) => KeyReader::Native(0x140491BA0),
        (Reader::Body, 0x1404563c0) | (Reader::Body, 0x140474360) | (Reader::Body, 0x140474330) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip")])
        }
        (Reader::Body, 0x140459210) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("state"),
            OwnKey::Value("resource"),
            OwnKey::Value("delivered"),
        ]),
        (Reader::Body, 0x1404594A0) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("category"), OwnKey::Value("technology")])
        }
        (Reader::Body, 0x140457A70) => KeyReader::Only(&[
            OwnKey::Value("id"),
            OwnKey::Value("name"),
            OwnKey::Value("ruling_only"),
            OwnKey::Value("character"),
        ]),
        (Reader::Body, 0x1404571c0) => KeyReader::ThenLoadKey(&[OwnKey::Value("amount")]),
        (Reader::Body, 0x140458090) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("equipment"),
            OwnKey::Value("from"),
            OwnKey::Value("archetype"),
        ]),
        (Reader::Body, 0x140459A50) => KeyReader::Refused(0x140459A50),
        (Reader::Body, 0x1404588A0) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("modifier")])
        }
        (Reader::Body, 0x140491090) => KeyReader::Only(&[
            OwnKey::Value("province"),
            OwnKey::Value("building"),
            OwnKey::Value("level"),
        ]),
        (Reader::Body, 0x1404574b0) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140455AC0) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("categories"),
            OwnKey::Value("slots"),
        ]),
        (Reader::Body, 0x1404742f0) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip"), OwnKey::Value("include_invisible")])
        }
        (Reader::Body, 0x140457A00) => {
            KeyReader::Only(&[OwnKey::Value("achievement"), OwnKey::Value("mod")])
        }
        (Reader::Own(0x14046AC80), 0x14046ADB0) => KeyReader::Only(&[OwnKey::Value("character")]),
        (Reader::Body, 0x140477C70) => KeyReader::ThenLoadKey(&[
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("include_invisible"),
            OwnKey::Value("count"),
        ]),
        (Reader::Body, 0x14045AC00) => KeyReader::Native(0x14045AC00),
        (Reader::Body, 0x140456f80) => KeyReader::Native(0x140456f80),
        (Reader::Body, 0x1404578d0) => KeyReader::Native(0x1404578d0),
        (Reader::Body, 0x1403CC4E0) => KeyReader::Native(0x1403CC4E0),
        (Reader::Body, 0x140457160) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("regions")])
        }
        (Reader::Body, 0x1404563f0) => KeyReader::Only(&[
            OwnKey::Value("type"),
            OwnKey::Value("amount"),
            OwnKey::Value("state"),
        ]),
        (Reader::Body, 0x140457200) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("state"),
            OwnKey::Value("border_state"),
        ]),
        (Reader::Body, 0x14045ABB0) => {
            KeyReader::Only(&[OwnKey::Value("ratio"), OwnKey::Value("archetype")])
        }
        (Reader::Body, 0x140457FD0) => KeyReader::Native(0x140457FD0),
        (Reader::Body, 0x1403C90A0) => KeyReader::Native(0x1403C90A0),
        (Reader::Body, 0x14045A780) => KeyReader::Native(0x14045A780),
        (Reader::Body, 0x140454B60) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("index"),
            OwnKey::Value("value"),
            OwnKey::Value("array"),
        ]),
        (Reader::Body, 0x140454E00) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip"), OwnKey::Value("array")])
        }
        (Reader::Own(0x140454050), 0x1404919D0) => {
            KeyReader::Only(&[OwnKey::Value("building"), OwnKey::Value("level")])
        }
        (Reader::Body, 0x1403CC0E0) => KeyReader::Native(0x1403CC0E0),
        (Reader::Own(0x1404540D0), 0x140457AE0) => {
            KeyReader::Only(&[OwnKey::Value("modifier"), OwnKey::Value("scope")])
        }
        (Reader::Body, 0x1403C79E0) => {
            KeyReader::Only(&[OwnKey::Value("amount"), OwnKey::Value("region")])
        }
        (Reader::Body, 0x140459C10) => KeyReader::Only(&[
            OwnKey::Value("tooltip"),
            OwnKey::Value("ratio"),
            OwnKey::Value("other"),
            OwnKey::Value("sub_unit_def_weights"),
        ]),
        (Reader::Body, 0x1404A9400) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("range")])
        }
        (Reader::Body, 0x1404a5df0) => KeyReader::Only(&[OwnKey::Value("id")]),
        (Reader::Own(0x140454050), 0x1404A94C0) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x1404A9470) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("side")])
        }
        (Reader::Body, 0x1404A5E20) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("modifier")])
        }
        (Reader::Body, 0x1404567f0) => {
            KeyReader::Only(&[OwnKey::Value("opponent"), OwnKey::Value("thousands")])
        }
        (Reader::Body, 0x14045A230) => {
            KeyReader::Only(&[OwnKey::Value("sender"), OwnKey::Value("value")])
        }
        (Reader::Own(0x140454050), 0x140457A40) => KeyReader::Native(0x140457A40),
        (Reader::Own(0x140454050), 0x140457130) => KeyReader::Native(0x140457130),
        (Reader::Body, 0x14045FC90) => KeyReader::Native(0x14045FC90),
        (Reader::Body, 0x14045FC00) => KeyReader::Only(&[
            OwnKey::Value("ratio"),
            OwnKey::Value("first"),
            OwnKey::Value("second"),
            OwnKey::Value("compare"),
        ]),
        (Reader::Body, 0x14045FF10) => KeyReader::Native(0x14045FF10),
        (Reader::Body, 0x14045FE40) => KeyReader::Refused(0x14045FE40),
        (Reader::Body, 0x14045FA90) => KeyReader::Refused(0x14045FA90),
        (Reader::Body, 0x14045FED0) => {
            KeyReader::Only(&[OwnKey::Value("tooltip"), OwnKey::Value("ribbon")])
        }
        (Reader::Own(0x14049FA80), 0x1404A0970) => KeyReader::Only(&[
            OwnKey::Value("type"),
            OwnKey::Value("need"),
            OwnKey::Value("var"),
        ]),
        (Reader::Own(0x140477C30), 0x140477D00) => KeyReader::Native(0x140477D00),
        (Reader::Own(0x140477C50), 0x140319EB0) => {
            KeyReader::Only(&[OwnKey::Value("trait"), OwnKey::Value("show_modifiers")])
        }
        (Reader::Own(0x140477C50), 0x140477D20) => KeyReader::Only(&[
            OwnKey::Value("check_mio_parent_completed"),
            OwnKey::Value("check_mio_mutually_exclusive"),
            OwnKey::Value("trait"),
            OwnKey::Value("show_modifiers"),
        ]),
        (Reader::Body, 0x1405675b0) => KeyReader::Only(&[
            OwnKey::Value("country"),
            OwnKey::Value("countries"),
            OwnKey::Value("collection"),
        ]),
        (Reader::Body, 0x140574660) => {
            KeyReader::Only(&[OwnKey::Value("track"), OwnKey::Value("amount")])
        }
        (Reader::Body, 0x140574430) => {
            KeyReader::Only(&[OwnKey::Value("amount"), OwnKey::Value("sub_doctrine")])
        }
        (Reader::Body, 0x140490ed0) | (Reader::Body, 0x1404C7310) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("target"),
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("count"),
        ]),
        (Reader::Body, 0x14045ADD0) => {
            KeyReader::Only(&[OwnKey::Value("months"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140567520) => KeyReader::ThenLoadKey(&[
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("count"),
            OwnKey::Value("collection"),
        ]),
        (Reader::Body, 0x1404D6800) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("leader")])
        }
        (Reader::Body, 0x140459F80) => KeyReader::Only(&[
            OwnKey::Value("target"),
            OwnKey::Value("state"),
            OwnKey::Value("value"),
        ]),
        (Reader::Body, 0x1403C7A60) => {
            KeyReader::Only(&[OwnKey::Value("tag"), OwnKey::Value("token")])
        }
        (Reader::Own(0x14049F1E0), 0x1404A08E0) => {
            KeyReader::Only(&[OwnKey::Value("var"), OwnKey::Value("scorer")])
        }
        (Reader::Own(0x14049F630), 0x1404A0920) => KeyReader::Only(&[
            OwnKey::Value("array"),
            OwnKey::Value("scores"),
            OwnKey::Value("scorer"),
        ]),
        (Reader::Body, 0x140454A30) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("original_tag"),
            OwnKey::Value("original_tag_to_check"),
            OwnKey::Value("count"),
            OwnKey::BoundLocalization("tooltip"),
        ]),
        (Reader::Body, 0x1404570c0) | (Reader::Body, 0x1404599A0) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("target")])
        }
        (Reader::Own(0x14049FED0), 0x1404A09C0) => KeyReader::Native(0x1404A07A0),
        (Reader::Own(0x140454050), 0x14045A1E0) => KeyReader::Native(0x14045A1E0),
        (Reader::Own(0x1404A0320), 0x1404A09D0) => KeyReader::Only(&[
            OwnKey::Value("min"),
            OwnKey::Value("max"),
            OwnKey::Value("var"),
            OwnKey::Value("integer"),
        ]),
        (Reader::Own(0x140454050), 0x140459A10) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("operation")])
        }
        (Reader::Body, 0x140567800) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("input"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x1404D68B0) => KeyReader::Refused(0x1404D68B0),
        (Reader::Own(0x14046AD80), 0x14046ADE0) => KeyReader::Only(&[
            OwnKey::Value("province"),
            OwnKey::Value("include_field_marshal"),
        ]),
        (Reader::Body, 0x140491AF0) => KeyReader::Only(&[
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
    /// Each building name's `provincial` class; `None` when declarations disagree.
    pub provincial: BTreeMap<Vec<u8>, Option<bool>>,
    pub ideologies: BTreeSet<Vec<u8>>,
    pub ideology_types: BTreeSet<Vec<u8>>,
    pub ideologies_gap: Option<String>,
    pub resources: BTreeSet<Vec<u8>>,
    /// A resource file whose reading stops short, which could declare any other name.
    pub resources_gap: Option<String>,
}

impl CountTriggerNames {
    pub fn new(buildings: &Buildings, ideologies: &Ideologies, resources: &Resources) -> Self {
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
        Self {
            buildings: buildings
                .types
                .keys()
                .map(|name| name.as_bytes().to_ascii_lowercase())
                .collect(),
            provincial,
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
                .declarations
                .iter()
                .map(|resource| resource.token.as_bytes().to_ascii_lowercase())
                .collect(),
            resources_gap: resources.gap().map(ToString::to_string),
        }
    }
    pub fn count_kind(&self, text: &[u8]) -> Result<Option<CountKind>, String> {
        let folded = text.to_ascii_lowercase();
        Ok(if self.buildings.contains(&folded) {
            Some(CountKind::Building)
        } else if self.resources.contains(&folded) {
            Some(CountKind::Resource)
        } else if let Some(gap) = &self.resources_gap {
            return Err(format!(
                "whether {} names a resource is unknown: {gap}",
                String::from_utf8_lossy(text)
            ));
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
    /// Whether a building type is provincial; `None` when unknown or ambiguous.
    pub fn is_provincial(&self, text: &[u8]) -> Option<bool> {
        self.provincial
            .get(&text.to_ascii_lowercase())
            .copied()
            .flatten()
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
                    0x140454050
                        | 0x141445FF0
                        | 0x14049F1E0
                        | 0x14049F630
                        | 0x14049FA80
                        | 0x14049FED0
                        | 0x1404A0320
                        | 0x140477C30
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
        let steel = crate::resources::Resource {
            token: "Steel".into(),
            icon_frame: None,
        };
        let resources = Resources::new(vec![steel.clone()]);
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
        // A resource file that stops short could declare any name a building does not take.
        let stopped = Resources::stopped(vec![steel], "common/resources/b.txt", "line 2");
        let names = CountTriggerNames::new(&buildings, &ideologies, &stopped);
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
        let names =
            CountTriggerNames::new(&buildings, &Ideologies::default(), &Resources::default());
        assert_eq!(names.is_provincial(b"BUNKER"), Some(true));
        assert_eq!(names.is_provincial(b"Mill"), Some(false));
        assert_eq!(names.is_provincial(b"site"), None);
        assert_eq!(names.is_provincial(b"absent"), None);
    }
}
