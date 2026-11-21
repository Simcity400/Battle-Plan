
use engine_executable::executable_tables::{RuntimeTable, view};
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
    RuntimeTable::new(|| view(|tables| &tables.registries, |table| &table.triggers[..]));
pub use engine_executable::executable_tables::TriggerReader as Reader;
pub static TRIGGER_CLASSES: RuntimeTable<&'static [(u32, u32, Reader)]> = RuntimeTable::new(|| {
    view(
        |tables| &tables.registries,
        |table| &table.trigger_classes[..],
    )
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
    let registry = &engine_executable::executable_tables::required(
        engine_executable::executable_tables::get()
            .registries
            .as_ref(),
    )
    .trigger_key_readers;
    let key = registry
        .binary_search_by_key(&token, |&(id, _)| id)
        .ok()
        .map(|index| registry[index].1)
        .expect("registered trigger key reader");
    match (reader, key) {
        (Reader::Body, 0x1404C1870)
        | (Reader::Body, 0x1404559d0)
        | (Reader::Body, 0x14047AFA0)
        | (Reader::Body, 0x1404910c0)
        | (Reader::Body, 0x140479210) => ITERATOR,
        (Reader::Body, 0x1404C19C0) => {
            KeyReader::Only(&[OwnKey::Value("specialization"), OwnKey::Value("level")])
        }
        (Reader::Own(0x1414461e0), 0x1405972d0) => KeyReader::Only(&[
            OwnKey::Value("value"),
            OwnKey::Value("date"),
            OwnKey::Value("days"),
            OwnKey::Value("flag"),
        ]),
        (Reader::Body, 0x1404C18B0) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("specialization")])
        }
        (Reader::Own(0x140454290), 0x1405972d0) => KeyReader::Only(&[
            OwnKey::Value("state"),
            OwnKey::Value("preparation"),
            OwnKey::Value("activated"),
        ]),
        (Reader::Body, 0x140491720) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("ai_area"),
            OwnKey::Value("strategic_region"),
            OwnKey::Value("continent"),
            OwnKey::Value("array"),
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("count"),
        ]),
        (Reader::Body, 0x140458B20) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("buildings"),
            OwnKey::Value("resource"),
            OwnKey::Value("extracted"),
            OwnKey::Value("collection"),
        ]),
        (Reader::Body, 0x1404566e0) => KeyReader::Only(&[
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
        (Reader::Body, 0x1404588d0) => {
            KeyReader::Only(&[OwnKey::Value("state"), OwnKey::Value("level")])
        }
        (Reader::Body, 0x140456DB0) => KeyReader::Native(0x140456DB0),
        (Reader::Body, 0x140459590) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip_evaluation")])
        }
        (Reader::Body, 0x140455A10)
        | (Reader::Body, 0x1404918e0)
        | (Reader::Body, 0x140458240)
        | (Reader::Own(0x1404540c0), 0x141446BF0) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x140458E00) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("only_imported"),
            OwnKey::Value("buildings"),
            OwnKey::Value("resource"),
            OwnKey::Value("extracted"),
        ]),
        (Reader::Body, 0x1404590c0) => KeyReader::Only(&[
            OwnKey::Value("receiver"),
            OwnKey::Value("state"),
            OwnKey::Value("resources"),
        ]),
        (Reader::Body, 0x140457310) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("state"),
            OwnKey::Value("value"),
            OwnKey::Value("unit"),
            OwnKey::Value("tag"),
        ]),
        (Reader::Body, 0x14045A9E0) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("area"),
        ]),
        (Reader::Body, 0x140457830) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("unit_category"),
        ]),
        (Reader::Body, 0x140458790) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("unit"),
            OwnKey::Value("archetype"),
        ]),
        (Reader::Body, 0x140567890) => KeyReader::Only(&[
            OwnKey::Value("collection"),
            OwnKey::Value("buildings"),
            OwnKey::Value("unit_category"),
            OwnKey::Value("equipment_ratio"),
            OwnKey::Value("stockpile"),
            OwnKey::Value("size"),
            OwnKey::Value("manpower"),
            OwnKey::Value("unit"),
        ]),
        (Reader::Body, 0x140459800) | (Reader::Body, 0x140457730) => {
            KeyReader::Only(&[OwnKey::Value("ratio"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140457770) => {
            KeyReader::Only(&[OwnKey::Value("size"), OwnKey::Value("type")])
        }
        (Reader::Body, 0x140457970) => {
            KeyReader::Only(&[OwnKey::Value("type"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x140455110) => KeyReader::Only(&[
            OwnKey::Value("idea"),
            OwnKey::Value("limit"),
            OwnKey::Value("ignore"),
            OwnKey::Value("characters"),
        ]),
        (Reader::Body, 0x140491970) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("province"),
            OwnKey::Value("building"),
            OwnKey::Value("include_locked"),
        ]),
        (Reader::Own(0x141446060), 0x141446C20) => KeyReader::ThenLoadKey(&[
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::BoundLocalization("not_tooltip"),
        ]),
        (Reader::Body, 0x140457BC0) => KeyReader::Native(0x140457BC0),
        (Reader::Body, 0x140459C00) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("progress"), OwnKey::Value("focus")])
        }
        (Reader::Body, 0x14045AA50) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("state"),
        ]),
        (Reader::Body, 0x140459550) => {
            KeyReader::Only(&[OwnKey::Value("count"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140457B90) => {
            KeyReader::Only(&[OwnKey::Value("group"), OwnKey::Value("value")])
        }
        (Reader::Own(0x140454870), 0x1405972d0) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("type")])
        }
        (Reader::Body, 0x140491C10) => KeyReader::Native(0x140491C10),
        (Reader::Body, 0x140456430) | (Reader::Body, 0x1404743d0) | (Reader::Body, 0x1404743a0) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip")])
        }
        (Reader::Body, 0x140459280) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("state"),
            OwnKey::Value("resource"),
            OwnKey::Value("delivered"),
        ]),
        (Reader::Body, 0x140459510) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("category"), OwnKey::Value("technology")])
        }
        (Reader::Body, 0x140457AE0) => KeyReader::Only(&[
            OwnKey::Value("id"),
            OwnKey::Value("name"),
            OwnKey::Value("ruling_only"),
            OwnKey::Value("character"),
        ]),
        (Reader::Body, 0x140457230) => KeyReader::ThenLoadKey(&[OwnKey::Value("amount")]),
        (Reader::Body, 0x140458100) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("equipment"),
            OwnKey::Value("from"),
            OwnKey::Value("archetype"),
        ]),
        (Reader::Body, 0x140459AC0) => KeyReader::Refused(0x140459AC0),
        (Reader::Body, 0x140458910) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("modifier")])
        }
        (Reader::Body, 0x140491100) => KeyReader::Only(&[
            OwnKey::Value("province"),
            OwnKey::Value("building"),
            OwnKey::Value("level"),
        ]),
        (Reader::Body, 0x140457520) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140455B30) => KeyReader::Only(&[
            OwnKey::Value("amount"),
            OwnKey::Value("categories"),
            OwnKey::Value("slots"),
        ]),
        (Reader::Body, 0x140474360) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip"), OwnKey::Value("include_invisible")])
        }
        (Reader::Body, 0x140457A70) => {
            KeyReader::Only(&[OwnKey::Value("achievement"), OwnKey::Value("mod")])
        }
        (Reader::Own(0x14046ACF0), 0x14046AE20) => KeyReader::Only(&[OwnKey::Value("character")]),
        (Reader::Body, 0x140477CE0) => KeyReader::ThenLoadKey(&[
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("include_invisible"),
            OwnKey::Value("count"),
        ]),
        (Reader::Body, 0x14045AC70) => KeyReader::Native(0x14045AC70),
        (Reader::Body, 0x140456ff0) => KeyReader::Native(0x140456ff0),
        (Reader::Body, 0x140457940) => KeyReader::Native(0x140457940),
        (Reader::Body, 0x1403CC550) => KeyReader::Native(0x1403CC550),
        (Reader::Body, 0x1404571d0) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("regions")])
        }
        (Reader::Body, 0x140456460) => KeyReader::Only(&[
            OwnKey::Value("type"),
            OwnKey::Value("amount"),
            OwnKey::Value("state"),
        ]),
        (Reader::Body, 0x140457270) => KeyReader::Only(&[
            OwnKey::Value("size"),
            OwnKey::Value("type"),
            OwnKey::Value("state"),
            OwnKey::Value("border_state"),
        ]),
        (Reader::Body, 0x14045AC20) => {
            KeyReader::Only(&[OwnKey::Value("ratio"), OwnKey::Value("archetype")])
        }
        (Reader::Body, 0x140458040) => KeyReader::Native(0x140458040),
        (Reader::Body, 0x1403C9110) => KeyReader::Native(0x1403C9110),
        (Reader::Body, 0x14045A7F0) => KeyReader::Native(0x14045A7F0),
        (Reader::Body, 0x140454BD0) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("index"),
            OwnKey::Value("value"),
            OwnKey::Value("array"),
        ]),
        (Reader::Body, 0x140454E70) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("tooltip"), OwnKey::Value("array")])
        }
        (Reader::Own(0x1404540c0), 0x140491A40) => {
            KeyReader::Only(&[OwnKey::Value("building"), OwnKey::Value("level")])
        }
        (Reader::Body, 0x1403CC150) => KeyReader::Native(0x1403CC150),
        (Reader::Own(0x140454140), 0x140457B50) => {
            KeyReader::Only(&[OwnKey::Value("modifier"), OwnKey::Value("scope")])
        }
        (Reader::Body, 0x1403C7A50) => {
            KeyReader::Only(&[OwnKey::Value("amount"), OwnKey::Value("region")])
        }
        (Reader::Body, 0x140459C80) => KeyReader::Only(&[
            OwnKey::Value("tooltip"),
            OwnKey::Value("ratio"),
            OwnKey::Value("other"),
            OwnKey::Value("sub_unit_def_weights"),
        ]),
        (Reader::Body, 0x1404A9470) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("range")])
        }
        (Reader::Body, 0x1404a5e60) => KeyReader::Only(&[OwnKey::Value("id")]),
        (Reader::Own(0x1404540c0), 0x1404A9530) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x1404A94E0) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("side")])
        }
        (Reader::Body, 0x1404A5E90) => {
            KeyReader::Only(&[OwnKey::Value("id"), OwnKey::Value("modifier")])
        }
        (Reader::Body, 0x140456860) => {
            KeyReader::Only(&[OwnKey::Value("opponent"), OwnKey::Value("thousands")])
        }
        (Reader::Body, 0x14045A2A0) => {
            KeyReader::Only(&[OwnKey::Value("sender"), OwnKey::Value("value")])
        }
        (Reader::Own(0x1404540c0), 0x140457AB0) => KeyReader::Native(0x140457AB0),
        (Reader::Own(0x1404540c0), 0x1404571a0) => KeyReader::Native(0x1404571a0),
        (Reader::Body, 0x14045FD00) => KeyReader::Native(0x14045FD00),
        (Reader::Body, 0x14045FC70) => KeyReader::Only(&[
            OwnKey::Value("ratio"),
            OwnKey::Value("first"),
            OwnKey::Value("second"),
            OwnKey::Value("compare"),
        ]),
        (Reader::Body, 0x14045FF80) => KeyReader::Native(0x14045FF80),
        (Reader::Body, 0x14045FEB0) => KeyReader::Refused(0x14045FEB0),
        (Reader::Body, 0x14045FB00) => KeyReader::Refused(0x14045FB00),
        (Reader::Body, 0x14045FF40) => {
            KeyReader::Only(&[OwnKey::Value("tooltip"), OwnKey::Value("ribbon")])
        }
        (Reader::Own(0x14049FAF0), 0x1404A09E0) => KeyReader::Only(&[
            OwnKey::Value("type"),
            OwnKey::Value("need"),
            OwnKey::Value("var"),
        ]),
        (Reader::Own(0x140477CA0), 0x140477D70) => KeyReader::Native(0x140477D70),
        (Reader::Own(0x140477CC0), 0x140319F20) => {
            KeyReader::Only(&[OwnKey::Value("trait"), OwnKey::Value("show_modifiers")])
        }
        (Reader::Own(0x140477CC0), 0x140477D90) => KeyReader::Only(&[
            OwnKey::Value("check_mio_parent_completed"),
            OwnKey::Value("check_mio_mutually_exclusive"),
            OwnKey::Value("trait"),
            OwnKey::Value("show_modifiers"),
        ]),
        (Reader::Body, 0x140567620) => KeyReader::Only(&[
            OwnKey::Value("country"),
            OwnKey::Value("countries"),
            OwnKey::Value("collection"),
        ]),
        (Reader::Body, 0x1405746d0) => {
            KeyReader::Only(&[OwnKey::Value("track"), OwnKey::Value("amount")])
        }
        (Reader::Body, 0x1405744a0) => {
            KeyReader::Only(&[OwnKey::Value("amount"), OwnKey::Value("sub_doctrine")])
        }
        (Reader::Body, 0x140490f40) | (Reader::Body, 0x1404C7380) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("target"),
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("count"),
        ]),
        (Reader::Body, 0x14045AE40) => {
            KeyReader::Only(&[OwnKey::Value("months"), OwnKey::Value("tag")])
        }
        (Reader::Body, 0x140567590) => KeyReader::ThenLoadKey(&[
            OwnKey::BoundLocalization("tooltip"),
            OwnKey::Value("count"),
            OwnKey::Value("collection"),
        ]),
        (Reader::Body, 0x1404D6870) => {
            KeyReader::Only(&[OwnKey::Value("value"), OwnKey::Value("leader")])
        }
        (Reader::Body, 0x140459FF0) => KeyReader::Only(&[
            OwnKey::Value("target"),
            OwnKey::Value("state"),
            OwnKey::Value("value"),
        ]),
        (Reader::Body, 0x1403C7AD0) => {
            KeyReader::Only(&[OwnKey::Value("tag"), OwnKey::Value("token")])
        }
        (Reader::Own(0x14049F250), 0x1404A0950) => {
            KeyReader::Only(&[OwnKey::Value("var"), OwnKey::Value("scorer")])
        }
        (Reader::Own(0x14049F6A0), 0x1404A0990) => KeyReader::Only(&[
            OwnKey::Value("array"),
            OwnKey::Value("scores"),
            OwnKey::Value("scorer"),
        ]),
        (Reader::Body, 0x140454AA0) => KeyReader::ThenLoadKey(&[
            OwnKey::Value("original_tag"),
            OwnKey::Value("original_tag_to_check"),
            OwnKey::Value("count"),
            OwnKey::BoundLocalization("tooltip"),
        ]),
        (Reader::Body, 0x140457130) | (Reader::Body, 0x140459A10) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("target")])
        }
        (Reader::Own(0x14049FF40), 0x1404A0A30) => KeyReader::Native(0x1404A0810),
        (Reader::Own(0x1404540c0), 0x14045A250) => KeyReader::Native(0x14045A250),
        (Reader::Own(0x1404A0390), 0x1404A0A40) => KeyReader::Only(&[
            OwnKey::Value("min"),
            OwnKey::Value("max"),
            OwnKey::Value("var"),
            OwnKey::Value("integer"),
        ]),
        (Reader::Own(0x1404540c0), 0x140459A80) => {
            KeyReader::Only(&[OwnKey::Value("target"), OwnKey::Value("operation")])
        }
        (Reader::Body, 0x140567870) => {
            KeyReader::ThenLoadKey(&[OwnKey::Value("input"), OwnKey::Value("value")])
        }
        (Reader::Body, 0x1404D6920) => KeyReader::Refused(0x1404D6920),
        (Reader::Own(0x14046ADF0), 0x14046AE50) => KeyReader::Only(&[
            OwnKey::Value("province"),
            OwnKey::Value("include_field_marshal"),
        ]),
        (Reader::Body, 0x140491B60) => KeyReader::Only(&[
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
        engine_executable::executable_tables::auxiliary_part(|tables| &tables.equipment_categories)
            .map(|categories| &categories[..])
            .map_err(Clone::clone)
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
    pub provincial: BTreeMap<Vec<u8>, Option<bool>>,
    pub ideologies: BTreeSet<Vec<u8>>,
    pub ideology_types: BTreeSet<Vec<u8>>,
    pub ideologies_gap: Option<String>,
    pub resources: BTreeSet<Vec<u8>>,
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
                    0x1404540c0
                        | 0x141446060
                        | 0x14049F250
                        | 0x14049F6A0
                        | 0x14049FAF0
                        | 0x14049FF40
                        | 0x1404A0390
                        | 0x140477CA0
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
