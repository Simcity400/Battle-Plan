
use super::{READ_EFFECT, READ_SCRIPT};

const STRATEGIC_LOCATION_VALUE: u64 = 0x14056F280;
const TARGETED_RESISTANCE_VALUE: u64 = 0x140333C10;
const COLLECTION_KEYS: u64 = 0x140A6A4C0;

pub(super) const ELSE_TOKEN: u32 = 0x36D3;
pub(super) const ELSE_IF_TOKEN: u32 = 0x38ED;

const POLITICAL_PARTY_TOKEN: u32 = 0x3009;
#[cfg(test)]
mod tests;
use crate::{key_names::{self, KeyNames, Load}, native::{Field, Shape}};
use content_reader_parser::parser::{Operator, c_string};

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
    Keys(fn(Option<&[u8]>) -> Option<ValueReader>, Fallback),
    Variable,
    DivideVariable,
    Conditional,
    GenerateCharacter,
}

mod ai_strategy;
mod grammar;
#[cfg(any(test, feature = "test-support"))]
pub(super) use grammar::SYNTHETIC_EFFECTS;
#[cfg(any(test, feature = "test-support"))]
pub(super) const SYNTHETIC_VALUES: &[(&str, u64)] = &[
    ("strategic_state_location", STRATEGIC_LOCATION_VALUE),
    ("strategic_province_location", STRATEGIC_LOCATION_VALUE),
];
#[cfg(any(test, feature = "test-support"))]
pub(super) const SYNTHETIC_TOKENS: &[(&str, u32)] = &[("set_political_party", POLITICAL_PARTY_TOKEN)];
mod registries;
mod unit_bonus;
use grammar::Helper;

fn reader(token: u32) -> Option<KeyReader> {
    let token = if matches!(token, ELSE_TOKEN | ELSE_IF_TOKEN) {
        super::IF_TOKEN
    } else {
        token
    };
    let (value, key) = super::class(token)?;
    if !matches!(
        value,
        READ_SCRIPT | STRATEGIC_LOCATION_VALUE | TARGETED_RESISTANCE_VALUE
    ) {
        return None;
    }
    let address = if value == STRATEGIC_LOCATION_VALUE {
        value
    } else {
        key
    };
    grammar::reader(address, super::class_vtable(token)?)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DescriptionField {
    Own,
    Effect,
    RandomListMember,
}

fn description_field(reader: KeyReader, item: &content_reader_parser::parser::Item) -> DescriptionField {
    use DescriptionField::*;
    match reader {
        KeyReader::Conditional if !item.keyword_is("limit") => Effect,
        KeyReader::Conditional
        | KeyReader::Variable
        | KeyReader::DivideVariable
        | KeyReader::GenerateCharacter => Own,
        KeyReader::Keys(keys, fallback) => {
            let keyword = item.keyword().map(|keyword| keyword.folded());
            if keys(keyword.as_deref()).is_some() {
                return Own;
            }
            match fallback {
                Fallback::Effect => Effect,
                Fallback::Reader(helper) => description_field(grammar::helper(helper), item),
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
    items: &[content_reader_parser::parser::Item],
    field: DescriptionField,
) -> Result<Vec<content_reader_parser::parser::Item>, String> {
    let reader = reader(token)
        .ok_or_else(|| format!("effect own-key projection is not ported for token {token:X}"))?;
    Ok(items
        .iter()
        .filter(|item| description_field(reader, item) == field)
        .cloned()
        .collect())
}

pub(super) fn description_fields(
    token: u32,
    items: &[content_reader_parser::parser::Item],
) -> Result<Vec<content_reader_parser::parser::Item>, String> {
    description_projection(token, items, DescriptionField::Effect)
}

pub(super) fn random_list_members(
    token: u32,
    items: &[content_reader_parser::parser::Item],
) -> Result<Vec<content_reader_parser::parser::Item>, String> {
    description_projection(token, items, DescriptionField::RandomListMember)
}

pub(super) fn random_member_fields(items: &[content_reader_parser::parser::Item]) -> Vec<content_reader_parser::parser::Item> {
    let reader = grammar::helper(Helper::RandomListMember);
    items
        .iter()
        .filter(|item| description_field(reader, item) == DescriptionField::Effect)
        .cloned()
        .collect()
}

pub fn is_own_field(effect: &[u8], key: &[u8]) -> Result<bool, String> {
    let unknown = || format!(
        "no verified reader for effect {} field {}",
        String::from_utf8_lossy(effect), String::from_utf8_lossy(key)
    );
    let token = super::registered(effect).ok_or_else(unknown)?;
    let field = key.to_ascii_lowercase();
    if let Some(reader) = reader(token) {
        return Ok(matches!(reader, KeyReader::Keys(keys, _) if keys(Some(&field)).is_some()));
    }
    let (value, fields) = super::class(token).ok_or_else(unknown)?;
    match (value, fields) {
        (super::READ_VALUE, _) | (READ_SCRIPT, READ_EFFECT) => Ok(false),
        _ => super::value_readers::is_own_field(value, fields, effect, &field).ok_or_else(unknown),
    }
}

pub fn reads_scope_chain(effect: &[u8], key: &[u8]) -> bool {
    matches!(super::registered(effect).and_then(reader), Some(KeyReader::Keys(keys, _))
        if matches!(keys(Some(key)), Some(ValueReader::ScopeSelectorChain)))
}

pub(crate) fn requires_block(token: u32) -> bool {
    super::class(token).is_none_or(|(value, _)| value == READ_SCRIPT)
}

pub(super) fn value_shape(address: u64, field: &Field<'_>) -> Option<Shape<()>> {
    if address != STRATEGIC_LOCATION_VALUE {
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
        kind: content_reader_parser::DiagnosticKind::Unsupported,
        report: format!(
            "Deferred effect `{}`: unsupported key grammar; by-key reading retained",
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
    Trigger(crate::trigger_grammar::Node),
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
    triggers: crate::trigger_grammar::Shaper<'n>,
    ai: ai_strategy::Shaper<'n>,
    limit: Limit,
    expression: bool,
    generated: Generated,
    local_generated: std::collections::BTreeSet<content_reader_parser::tokens::Token>,
}

impl<'n> Shaper<'n> {
    pub(crate) fn new(names: Option<&'n KeyNames>, token: u32) -> Self {
        Self {
            token,
            names,
            triggers: crate::trigger_grammar::Shaper::new(names),
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
                    .shape(crate::trigger_grammar::Node::Trigger, field)
                    .map(Node::Trigger)
            }

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
            Node::Helper(helper) => self.keys(grammar::helper(helper), field),
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
                    Shape::Unsettled("variable effect: scalar value after an expression cannot use the stored expression".into())
                } else {
                    Shape::One
                }
            }
            KeyReader::GenerateCharacter => self.generate(field),
            KeyReader::Keys(keys, fallback) => {
                if let Some(value) = keys(key) {
                    return self.value(value, field);
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
                    Fallback::MeanTimeToHappen => crate::native::mean_time_to_happen(field),
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
                Shape::ObjectIfOpen(Node::Trigger(crate::trigger_grammar::Node::BoundLocalization))
            }
            Limit => self.limit(),
            Province => Shape::ObjectIfOpen(Node::Province),
            Trigger => Shape::Body(Node::Trigger(crate::trigger_grammar::Node::Trigger)),
            Effect => Shape::Effect,
            Object(helper) => Shape::Object(Node::Helper(helper)),
            ObjectIfOpen(Helper::ProvinceTarget) if !field.opens() => self.value(ProvinceId, field),
            ObjectIfOpen(helper) => Shape::ObjectIfOpen(Node::Helper(helper)),
            ObjectOrSkip(helper) => Shape::ObjectOrSkip(Node::Helper(helper)),
            MtthModifier => Shape::MtthModifier,
            Collection => Shape::ObjectIfOpen(Node::Trigger(crate::trigger_grammar::Node::KeyReader(
                COLLECTION_KEYS,
            ))),
            Comparison => {
                use crate::native::Kind;
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
                None => named_refusal("add_resource", "no resource database names"),
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
                None => named_refusal("construct_building", "no building database names"),
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
            Ledger => crate::idea_fields::ledger_shape(field),
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
            return named_refusal("equipment upgrades", "no equipment database names");
        };
        if names
            .effects
            .equipment_upgrades
            .contains(&name.to_ascii_lowercase())
        {
            Shape::One
        } else if let Some(gap) = &names.effects.equipment_upgrades_gap {
            named_refusal("equipment upgrades", &gap.to_string())
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
            kind: content_reader_parser::DiagnosticKind::Unsupported,
            report: format!(
                "Deferred effect specialization `{}`: block consumption needs the specialization registry",
                String::from_utf8_lossy(name)
            ),
            read: Box::new(Shape::Unsettled(
                "specialization block needs the unavailable specialization registry".into(),
            )),
        }
    }

    fn generate(&mut self, field: &Field<'_>) -> Shape<Node> {
        if matches!(self.generated, Generated::Collision) {
            return Shape::SkipIfOpen;
        }
        if field.keyword_is("token_base") {
            let text = c_string(&field.value.text).to_ascii_lowercase();
            let claim = content_reader_parser::tokens::Token::c_str(&text);
            if claim.is_rejected() {
                return Shape::Reported {
                    kind: content_reader_parser::DiagnosticKind::Unsupported,
                    report: "Deferred effect generate_character token_base: dynamic token 0 may already be claimed outside this effect read".into(),
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
                "effect limit: a later limit reads only into an empty trigger conjunction; whether the first holds a child is unknown: {why}"
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
        kind: content_reader_parser::DiagnosticKind::Unsupported,
        report: format!("Deferred effect {what}: {why}"),
        read: Box::new(Shape::Unsettled(format!("{what}: {why}"))),
    }
}
