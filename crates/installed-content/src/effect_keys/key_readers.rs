//! ReadScript's slot-4 readers. Static own-key lists and their fallback
//! come from captures/astra/effect-key-readers-2/extract_tables.py and
//! its cited per-range extraction scripts, using hoi4re dumps and the
//! factory/vtable inventory in
//! captures/astra/key-validity/classes.py. Addresses accompany the rows.

use super::{READ_EFFECT, READ_SCRIPT};

/// The `else` and `else_if` keyword tokens ReadEffect (1405345D0) tests
/// before the registry; both read with CIfEffect's key reader.
pub(super) const ELSE_TOKEN: u32 = 0x36D3;
pub(super) const ELSE_IF_TOKEN: u32 = 0x38ED;
/// CPoliticalPartyEffect child constructed at 14032CA40 and dispatched by
/// ReadScript (140534280) with this token.
const POLITICAL_PARTY_TOKEN: u32 = 0x3009;
#[cfg(test)]
mod tests;
use crate::{
    combat_tactics,
    key_names::{self, KeyNames, Load},
    parser::{
        Operator, c_string,
        native::{Field, Shape},
    },
};

#[derive(Clone, Copy)]
enum ValueReader {
    One,
    Current,
    SkipIfOpen,
    ValueList,
    ValueListOrSkip,
    ValueListIfOpen,
    ValueListIfOpenElseCurrent,
    ValueListExceptAll,
    RawList,
    RawText,
    Localization,
    Limit,
    Province,
    Trigger,
    Effect,
    Resource,
    Comparison,
    Building,
    Collection,
    ScopeSelectorChain,
    Gender,
    Ledger,
    ProvinceId,
    ProvinceVictoryPoint,
    MtthModifier,
    OptionalAssetsList,
    Specialization,
    Object(Helper),
    ObjectIfOpen(Helper),
    ObjectOrSkip(Helper),
}

#[derive(Clone, Copy)]
enum Fallback {
    Effect,
    Only,
    One,
    Reader(Helper),
    Object(Helper),
    AIStrategy,
    Popularities,
    RandomListMember,
    UnitBonus,
    PoliticalPartyScript,
    EquipmentStatRegistry,
    ModifierRegistry,
    EquipmentUpgrade,
    MeanTimeToHappen,
}

#[derive(Clone, Copy)]
enum KeyReader {
    Keys(&'static [(&'static str, ValueReader)], Fallback),
    Variable,
    DivideVariable,
    Conditional,
    GenerateCharacter,
}

mod ai_strategy;
mod data;
mod registries;
mod unit_bonus;
use data::{Helper, KEY_READERS};

fn reader(token: u32) -> Option<KeyReader> {
    KEY_READERS
        .binary_search_by_key(&token, |(id, _)| *id)
        .ok()
        .map(|index| KEY_READERS[index].1)
}

// Slot 4 separates stored operands, child effects and random-list members.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DescriptionField {
    Own,
    Effect,
    RandomListMember,
}

fn description_field(reader: KeyReader, item: &crate::parser::Item) -> DescriptionField {
    use DescriptionField::*;
    match reader {
        KeyReader::Conditional if !item.keyword_is("limit") => Effect,
        KeyReader::Conditional
        | KeyReader::Variable
        | KeyReader::DivideVariable
        | KeyReader::GenerateCharacter => Own,
        KeyReader::Keys(keys, fallback) => {
            if keys.iter().any(|(name, _)| item.keyword_is(name)) {
                return Own;
            }
            match fallback {
                Fallback::Effect => Effect,
                Fallback::Reader(helper) => description_field(data::helper(helper), item),
                Fallback::RandomListMember
                    if !c_string(item.key.as_deref().unwrap_or_default()).is_empty() =>
                {
                    RandomListMember
                }
                _ => Own,
            }
        }
    }
}

fn description_projection(
    token: u32,
    items: &[crate::parser::Item],
    field: DescriptionField,
) -> Result<Vec<crate::parser::Item>, String> {
    let reader = reader(token).ok_or_else(|| {
        format!("effect own-key projection is not ported for token {token:X} (140534280)")
    })?;
    Ok(items
        .iter()
        .filter(|item| description_field(reader, item) == field)
        .cloned()
        .collect())
}

pub(super) fn description_fields(
    token: u32,
    items: &[crate::parser::Item],
) -> Result<Vec<crate::parser::Item>, String> {
    description_projection(token, items, DescriptionField::Effect)
}

pub(super) fn random_list_members(
    token: u32,
    items: &[crate::parser::Item],
) -> Result<Vec<crate::parser::Item>, String> {
    description_projection(token, items, DescriptionField::RandomListMember)
}

pub(super) fn random_member_fields(items: &[crate::parser::Item]) -> Vec<crate::parser::Item> {
    let reader = data::helper(Helper::RandomListMember);
    items
        .iter()
        .filter(|item| description_field(reader, item) == DescriptionField::Effect)
        .cloned()
        .collect()
}

/// Whether the token's static key reader handles this bare key before fallback.
pub fn is_own_field(token: u32, key: &[u8]) -> bool {
    match reader(token) {
        Some(KeyReader::Keys(keys, _)) => keys
            .iter()
            .any(|(name, _)| name.as_bytes().eq_ignore_ascii_case(key)),
        _ => false,
    }
}

/// The inline loop at 14051BBB0 reads on after any value; ReadScript
/// (140534280) instead requires the value slot to hold `{`.
pub(crate) fn requires_block(token: u32) -> bool {
    super::class(token).is_none_or(|(value, _)| value == READ_SCRIPT)
}

pub(super) fn value_shape(address: u64, field: &Field<'_>) -> Option<Shape<()>> {
    if address != 0x14051BBB0 {
        return None;
    }
    field
        .keyword()
        .as_deref()
        .and_then(super::registered)
        .map(Shape::EffectKeys)
}

pub(super) fn shape(address: u64, field: &Field<'_>) -> Shape<()> {
    if address == READ_EFFECT {
        return Shape::Effect;
    }
    let keyword = field.keyword().unwrap_or_default();
    let token = match keyword.as_slice() {
        b"else" => Some(ELSE_TOKEN),
        b"else_if" => Some(ELSE_IF_TOKEN),
        _ => super::registered(&keyword),
    };
    if let Some(token) = token
        && reader(token).is_some()
    {
        return Shape::EffectKeys(token);
    }
    Shape::Reported {
        kind: crate::DiagnosticKind::Unsupported,
        report: format!(
            "Deferred effect `{}`: value reader {READ_SCRIPT:#X}, key reader {address:#X}; by-key reading retained",
            String::from_utf8_lossy(&keyword)
        ),
        read: Box::new(Shape::EffectByKey {
            trigger_limits: false,
        }),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Node {
    Keys,
    Limit,
    Province,
    Helper(Helper),
    Trigger(combat_tactics::Node),
    UnitBonus(unit_bonus::Node),
    Registry(crate::modifiers::Node),
    AI(ai_strategy::Node),
}

enum Limit {
    Empty,
    Children,
    Unknown(String),
}

enum Generated {
    Before,
    Active,
    Collision,
}

pub(crate) struct Shaper<'n> {
    token: u32,
    names: Option<&'n KeyNames>,
    triggers: combat_tactics::Shaper<'n>,
    ai: ai_strategy::Shaper<'n>,
    limit: Limit,
    expression: bool,
    generated: Generated,
    local_generated: std::collections::BTreeSet<crate::tokens::Token>,
}

impl<'n> Shaper<'n> {
    pub(crate) fn new(names: Option<&'n KeyNames>, token: u32) -> Self {
        Self {
            token,
            names,
            triggers: combat_tactics::Shaper::new(names),
            ai: ai_strategy::Shaper::new(names),
            limit: Limit::Empty,
            expression: false,
            generated: Generated::Before,
            local_generated: Default::default(),
        }
    }

    pub(crate) fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        let keyword = field.keyword();
        let key = keyword.as_deref();
        match node {
            Node::Trigger(node) => self.triggers.shape(node, field).map(Node::Trigger),
            Node::UnitBonus(node) => {
                unit_bonus::shape(self.names, node, field).map(Node::UnitBonus)
            }
            Node::Registry(node) => registries::shape(self.names, node, field).map(Node::Registry),
            Node::AI(node) => self.ai.shape(node, field).map(Node::AI),
            Node::Limit => {
                // CAndTrigger +0x14: LoadKey adds a child as dispatch decides.
                let load = key_names::load_key(self.names, field);
                if !matches!(self.limit, Limit::Children) {
                    match load {
                        _ if load.adds_child() == Some(true) => self.limit = Limit::Children,
                        Load::Unknown(why) if matches!(self.limit, Limit::Empty) => {
                            self.limit = Limit::Unknown(why)
                        }
                        _ => {}
                    }
                }
                self.triggers
                    .shape(combat_tactics::Node::Trigger, field)
                    .map(Node::Trigger)
            }
            // CBuildingConstructionEffect@14031D4D0's own-only province loop.
            Node::Province => match key {
                Some(b"level") if !matches!(field.operator, Operator::Less | Operator::Greater) => {
                    Shape::Skip
                }
                Some(b"limit_to_victory_point") if !victory_point(field) => Shape::Skip,
                Some(
                    b"id"
                    | b"level"
                    | b"first"
                    | b"all_provinces"
                    | b"limit_to_coastal"
                    | b"limit_to_border"
                    | b"limit_to_naval_base"
                    | b"limit_to_victory_point"
                    | b"limit_to_border_country"
                    | b"limit_to_supply_node"
                    | b"limit_to_coastal_bunker",
                ) => Shape::One,
                _ => Shape::Skip,
            },
            Node::Helper(helper) => self.keys(data::helper(helper), field),
            Node::Keys => self.keys(reader(self.token).expect("ported effect key reader"), field),
        }
    }

    fn keys(&mut self, reader: KeyReader, field: &Field<'_>) -> Shape<Node> {
        let keyword = field.keyword();
        let key = keyword.as_deref();
        match reader {
            KeyReader::Conditional => match key {
                Some(b"limit") if self.token == ELSE_TOKEN => Shape::Skip,
                Some(b"limit") => self.limit(),
                _ => self.effect(field),
            },
            // CVariableEffect@1403B6640; divide-temp@1403B2580.
            KeyReader::Variable | KeyReader::DivideVariable => {
                if matches!(key, Some(b"var" | b"tooltip"))
                    || (matches!(reader, KeyReader::DivideVariable) && key == Some(b"if_zero"))
                {
                    return Shape::One;
                }
                if field.opens() {
                    self.expression = true;
                    Shape::Expression
                } else if self.expression {
                    Shape::Unsettled("CVariableEffect 1403B6640: scalar value after an expression calls std::_Throw_bad_variant_access (1403BA3C0)".into())
                } else {
                    Shape::One
                }
            }
            KeyReader::GenerateCharacter => self.generate(field),
            KeyReader::Keys(keys, fallback) => {
                if let Some((_, value)) = keys.iter().find(|(name, _)| key == Some(name.as_bytes()))
                {
                    return self.value(*value, field);
                }
                match fallback {
                    Fallback::Effect => self.effect(field),
                    Fallback::Only => Shape::Skip,
                    Fallback::One => Shape::One,
                    Fallback::Reader(helper) => self.shape(Node::Helper(helper), field),
                    Fallback::Object(helper) => Shape::Object(Node::Helper(helper)),
                    Fallback::AIStrategy => self
                        .ai
                        .shape(ai_strategy::Node::Strategy, field)
                        .map(Node::AI),
                    Fallback::Popularities => {
                        registries::popularities(self.names, field).map(|()| Node::Keys)
                    }
                    Fallback::RandomListMember if c_string(&field.key.text).is_empty() => {
                        Shape::Skip
                    }
                    Fallback::RandomListMember => {
                        Shape::Object(Node::Helper(Helper::RandomListMember))
                    }
                    Fallback::UnitBonus => {
                        unit_bonus::shape(self.names, unit_bonus::Node::Bonus, field)
                            .map(Node::UnitBonus)
                    }
                    Fallback::PoliticalPartyScript => Shape::EffectKeys(POLITICAL_PARTY_TOKEN),
                    Fallback::EquipmentStatRegistry => {
                        registries::shape(self.names, crate::modifiers::Node::EquipmentBonus, field)
                            .map(Node::Registry)
                    }
                    Fallback::ModifierRegistry => {
                        registries::shape(self.names, crate::modifiers::Node::Modifier, field)
                            .map(Node::Registry)
                    }
                    Fallback::EquipmentUpgrade => self.upgrade(field),
                    Fallback::MeanTimeToHappen => crate::parser::native::mean_time_to_happen(field),
                }
            }
        }
    }

    fn effect(&self, field: &Field<'_>) -> Shape<Node> {
        super::shape(self.names, field).map(|()| Node::Keys)
    }

    fn value(&mut self, value: ValueReader, field: &Field<'_>) -> Shape<Node> {
        use ValueReader::*;
        match value {
            One => Shape::One,
            Current => Shape::Current,
            SkipIfOpen => Shape::SkipIfOpen,
            ValueList => Shape::ValueList,
            ValueListOrSkip => {
                if field.opens() {
                    Shape::ValueList
                } else {
                    Shape::Skip
                }
            }
            ValueListIfOpen => {
                if field.opens() {
                    Shape::ValueList
                } else {
                    Shape::One
                }
            }
            ValueListIfOpenElseCurrent => {
                if field.opens() {
                    Shape::ValueList
                } else {
                    Shape::Current
                }
            }
            ValueListExceptAll => {
                if field.value.is_keyword("all") {
                    Shape::One
                } else {
                    Shape::ValueList
                }
            }
            RawList => Shape::RawList,
            RawText => Shape::RawText,
            Localization => {
                Shape::ObjectIfOpen(Node::Trigger(combat_tactics::Node::BoundLocalization))
            }
            Limit => self.limit(),
            Province => Shape::ObjectIfOpen(Node::Province),
            Trigger => Shape::Body(Node::Trigger(combat_tactics::Node::Trigger)),
            Effect => Shape::Effect,
            Object(helper) => Shape::Object(Node::Helper(helper)),
            ObjectIfOpen(Helper::ProvinceTarget) if !field.opens() => self.value(ProvinceId, field),
            ObjectIfOpen(helper) => Shape::ObjectIfOpen(Node::Helper(helper)),
            ObjectOrSkip(helper) => Shape::ObjectOrSkip(Node::Helper(helper)),
            MtthModifier => Shape::MtthModifier,
            Collection => {
                Shape::ObjectIfOpen(Node::Trigger(combat_tactics::Node::KeyReader(0x140A0FC90)))
            }
            Comparison => {
                use crate::parser::native::Kind;
                if matches!(field.value.kind, Kind::Less | Kind::Greater)
                    || field.value.lookup().is_some_and(|word| {
                        [
                            "less_than",
                            "greater_than",
                            "less_than_or_equals",
                            "greater_than_or_equals",
                            "equals",
                            "not_equals",
                        ]
                        .iter()
                        .any(|choice| word.eq_ignore_ascii_case(choice.as_bytes()))
                    })
                {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
            Resource if !field.skips_block() => Shape::Skip,
            Resource => match self.names {
                None => named_refusal("add_resource", "1403AF090: no resource database names"),
                Some(names) => {
                    if names
                        .counts
                        .resources
                        .contains(&c_string(&field.value.text).to_ascii_lowercase())
                    {
                        Shape::One
                    } else {
                        Shape::Skip
                    }
                }
            },
            Building if !field.skips_block() => Shape::Skip,
            Building => match self.names {
                None => named_refusal(
                    "construct_building",
                    "1403B3640: no building database names",
                ),
                Some(names) => {
                    if field
                        .value
                        .lookup()
                        .is_some_and(|name| names.counts.is_building(name))
                    {
                        Shape::One
                    } else {
                        Shape::Skip
                    }
                }
            },
            Specialization => self.specialization(field),
            // ParseScopeSelector (1403AD690) accepts only these segments when dotted.
            ScopeSelectorChain => {
                let text = c_string(&field.value.text).to_ascii_lowercase();
                if !text.contains(&b'.')
                    || text
                        .split(|c| *c == b'.')
                        .all(|part| matches!(part, b"from" | b"root" | b"prev"))
                {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
            // Gender (1413DAFA0) reads CURRENT IDs: male/female/undefined.
            Gender => {
                if ["male", "female", "undefined"]
                    .iter()
                    .any(|word| field.current.is_keyword(word))
                {
                    Shape::Current
                } else {
                    Shape::Skip
                }
            }
            Ledger => crate::ideas::ledger_shape(field),
            ProvinceVictoryPoint => {
                if victory_point(field) {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
            ProvinceId => {
                if field.value.is_number() {
                    Shape::One
                } else if c_string(&field.value.text).is_empty() {
                    Shape::Skip
                } else {
                    Shape::One
                }
            }
            OptionalAssetsList => Shape::ObjectList(Node::Helper(Helper::OptionalEquipmentAsset)),
        }
    }

    fn upgrade(&self, field: &Field<'_>) -> Shape<Node> {
        if !field.skips_block() {
            return Shape::Skip;
        }
        let Some(name) = field.key.lookup() else {
            return Shape::Skip;
        };
        let Some(names) = self.names else {
            return named_refusal(
                "equipment upgrades",
                "140F79C80: no equipment database names",
            );
        };
        if names
            .effects
            .equipment_upgrades
            .contains(&name.to_ascii_lowercase())
        {
            Shape::One
        } else if let Some(gap) = &names.effects.equipment_upgrades_gap {
            named_refusal("equipment upgrades", &format!("140F79C80: {gap}"))
        } else {
            Shape::Skip
        }
    }

    fn specialization(&self, field: &Field<'_>) -> Shape<Node> {
        if !field.skips_block() {
            return Shape::Skip;
        }
        let Some(name) = field.value.lookup() else {
            return Shape::Skip;
        };
        if name.eq_ignore_ascii_case(b"all") {
            return Shape::One;
        }
        Shape::Reported {
            kind: crate::DiagnosticKind::Unsupported,
            report: format!(
                "Deferred effect specialization `{}` (140496280/140496160): block consumption needs the specialization registry",
                String::from_utf8_lossy(name)
            ),
            read: Box::new(Shape::Unsettled(
                "specialization block needs the unavailable specialization registry".into(),
            )),
        }
    }

    // CGenerateCharacterEffect@1403B2F70 registers token_base immediately
    // in the generated map (+0x60), distinct from authored characters (+0x40).
    fn generate(&mut self, field: &Field<'_>) -> Shape<Node> {
        if matches!(self.generated, Generated::Collision) {
            return Shape::SkipIfOpen;
        }
        if field.keyword_is("token_base") {
            let text = c_string(&field.value.text).to_ascii_lowercase();
            let claim = crate::tokens::Token::c_str(&text);
            if claim.is_rejected() {
                return Shape::Reported {
                    kind: crate::DiagnosticKind::Unsupported,
                    report: "Deferred effect generate_character token_base (1403B2F70): dynamic token 0 may already be claimed outside this effect read".into(),
                    read: Box::new(Shape::Unsettled("generate_character token_base 0 needs claims from other reads to choose template reading or skipping".into())),
                };
            }
            self.generated = if self.local_generated.insert(claim) {
                Generated::Active
            } else {
                Generated::Collision
            };
            Shape::One
        } else if matches!(self.generated, Generated::Before) {
            Shape::One
        } else {
            self.shape(Node::Helper(Helper::CharacterTemplate), field)
        }
    }

    fn limit(&self) -> Shape<Node> {
        match &self.limit {
            Limit::Empty => Shape::Body(Node::Limit),
            Limit::Children => Shape::Skip,
            Limit::Unknown(why) => Shape::Unsettled(format!(
                "effect limit: native key reader reads a later limit only into an empty CAndTrigger; whether the first holds a child is unknown: {why}"
            )),
        }
    }
}

fn victory_point(field: &Field<'_>) -> bool {
    c_string(&field.value.text) == b"yes"
        || matches!(field.operator, Operator::Less | Operator::Greater)
}

fn named_refusal(what: &str, why: &str) -> Shape<Node> {
    Shape::Reported {
        kind: crate::DiagnosticKind::Unsupported,
        report: format!("Deferred effect {what}: {why}"),
        read: Box::new(Shape::Unsettled(format!("{what}: {why}"))),
    }
}
