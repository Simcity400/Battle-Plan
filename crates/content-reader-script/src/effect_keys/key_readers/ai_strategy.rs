
use content_reader_parser::{crt, fixed5};
use crate::{key_names::KeyNames, native::{Field, Shape, Token}};

#[derive(Clone, Copy)]
pub(crate) enum Node {
    Strategy,
    Trigger(crate::trigger_grammar::Node),
}

#[derive(Clone, Copy)]
enum Helper {
    Market,
    Scorch,
    Buffer,
    Targeted,
    Front,
    Mission,
    Operation,
    StockpileFuel,
}

#[derive(Clone, Copy)]
enum Context {
    Allocated(Helper),

    Null,
    Unknown(&'static str),
}

#[derive(Clone, Copy)]
enum ValueReader {
    One,
    Values,
    Tag,
    State,
    Region,
    Area,
    Trigger,
    OrderType,
    ExecutionType,
}

const EQUIPMENT_MARKET_STRATEGY: i32 = 0x23;
const SCORCH_STRATEGY: i32 = 0x42;
const BUFFER_STRATEGY: i32 = 0x46;
const INVASION_REQUEST_STRATEGY: i32 = 0x47;
const FRONT_REQUEST_STRATEGY: i32 = 0x48;
const MISSION_STRATEGY: i32 = 0x4B;
const FRONT_CONTROL_STRATEGY: i32 = 0x4F;
const OPERATION_STRATEGY: i32 = 0x51;
const CONCENTRATION_FRONT_STRATEGY: i32 = 0x62;
const CONCENTRATION_TARGET_STRATEGY: i32 = 0x64;
const NAVAL_DOMINANCE_STRATEGY: i32 = 0x6B;
const STOCKPILE_FUEL_STRATEGY: i32 = 0x70;

fn targeted_field(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"country" | b"tag") => ValueReader::Tag,
        Some(b"state") => ValueReader::State,
        Some(b"ratio") => ValueReader::One,
        Some(b"area") => ValueReader::Area,
        Some(b"strategic_region") => ValueReader::Region,
        Some(b"state_trigger" | b"country_trigger") => ValueReader::Trigger,
        _ => return None,
    })
}

fn helper_field(helper: Helper, key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match (helper, key) {
        (Helper::Market, Some(b"equipment_type" | b"seller")) => ValueReader::One,
        (Helper::Scorch | Helper::Buffer, Some(b"states")) => ValueReader::Values,
        (
            Helper::Buffer,
            Some(
                b"ratio"
                | b"order_id"
                | b"subtract_invasions_from_need"
                | b"subtract_fronts_from_need",
            ),
        ) => ValueReader::One,
        (Helper::Buffer, Some(b"area")) => ValueReader::Area,
        (Helper::Targeted, _) => return targeted_field(key),
        (Helper::Front, Some(b"priority" | b"execute_order" | b"manual_attack")) => {
            ValueReader::One
        }
        (Helper::Front, Some(b"ordertype")) => ValueReader::OrderType,
        (Helper::Front, Some(b"execution_type")) => ValueReader::ExecutionType,
        (Helper::Front, _) => return targeted_field(key),
        (
            Helper::Mission,
            Some(b"priority" | b"state" | b"mission" | b"mission_target" | b"num_operatives"),
        ) => ValueReader::One,
        (
            Helper::Operation,
            Some(b"priority" | b"state" | b"region" | b"operation" | b"operation_target"),
        ) => ValueReader::One,
        (Helper::StockpileFuel, Some(b"stop_above" | b"start_below")) => ValueReader::One,
        _ => return None,
    })
}

fn helper(kind: i32) -> Option<Helper> {
    match kind {
        EQUIPMENT_MARKET_STRATEGY => Some(Helper::Market),
        SCORCH_STRATEGY => Some(Helper::Scorch),
        BUFFER_STRATEGY => Some(Helper::Buffer),
        INVASION_REQUEST_STRATEGY
        | FRONT_REQUEST_STRATEGY
        | CONCENTRATION_FRONT_STRATEGY
        | CONCENTRATION_TARGET_STRATEGY => Some(Helper::Targeted),
        MISSION_STRATEGY => Some(Helper::Mission),
        FRONT_CONTROL_STRATEGY => Some(Helper::Front),
        OPERATION_STRATEGY => Some(Helper::Operation),
        STOCKPILE_FUEL_STRATEGY => Some(Helper::StockpileFuel),
        _ => None,
    }
}

fn token_is_one_of(token: &Token, words: &[(&str, u32)]) -> bool {
    words.iter().any(|(word, _)| token.is_keyword(word))
}

fn accepted(yes: bool) -> Shape<Node> {
    if yes { Shape::One } else { Shape::Skip }
}

pub(super) struct Shaper<'n> {
    names: Option<&'n KeyNames>,
    triggers: crate::trigger_grammar::Shaper<'n>,
    kind: i32,
    context: Context,
}

impl<'n> Shaper<'n> {
    pub(super) fn new(names: Option<&'n KeyNames>) -> Self {
        Self {
            names,
            triggers: crate::trigger_grammar::Shaper::new(names),
            kind: 0,
            context: Context::Unknown("initial global strategy-specific index 0"),
        }
    }

    pub(super) fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        if let Node::Trigger(node) = node {
            return self.triggers.shape(node, field).map(Node::Trigger);
        }
        match field.keyword().as_deref() {
            Some(b"type") => {
                if let Some((_, kind)) = engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.ai_strategy_types))
                    .iter()
                    .find(|(word, _)| field.value.is_keyword(word))
                {
                    self.kind = *kind as i32;
                    if let Some(helper) = helper(*kind as i32) {
                        self.context = Context::Allocated(helper);
                    }
                } else if let Some(kind) = crt::scan_i32(field.value.c_str()).stored() {
                    self.kind = kind;
                }
                Shape::One
            }
            Some(b"value") => Shape::One,
            Some(b"id") => {
                if token_is_one_of(
                    field.value,
                    &engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.selectors)),
                ) || field.value.c_str().starts_with(b"event_target:")
                {
                    return Shape::One;
                }
                if self.kind == MISSION_STRATEGY {
                    return accepted(token_is_one_of(
                        field.value,
                        &engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.missions)),
                    ));
                }
                if self.kind == NAVAL_DOMINANCE_STRATEGY && field.value.is_number() {
                    self.set_index(fixed5::decimal_i32(field.value.c_str()));
                }

                Shape::One
            }
            Some(b"target") => {
                let text = field.value.c_str();
                if !text.is_empty() && text.iter().all(u8::is_ascii_digit) {
                    if let Some(index) = crt::scan_u32(text).stored() {
                        self.set_index(index as i32);
                    }
                } else {
                    self.context = Context::Unknown("target area or global lexer token lookup");
                }
                Shape::One
            }
            Some(b"target_country") => {
                let value = fixed5::decimal_i32(field.value.c_str());
                if self
                    .names
                    .is_some_and(|names| names.countries.contains(field.value.c_str()))
                {
                    self.context = Context::Unknown("target_country global country index");
                } else if value < 0 && self.names.is_some() {
                    self.context = Context::Null;
                } else {
                    self.context = Context::Unknown("target_country global index lookup");
                }
                Shape::One
            }
            _ => self.specific(field),
        }
    }

    fn set_index(&mut self, index: i32) {
        self.context = if index < 0 {
            Context::Null
        } else {
            Context::Unknown("explicit global strategy-specific index")
        };
    }

    fn specific(&self, field: &Field<'_>) -> Shape<Node> {
        if helper(self.kind).is_none() {
            return Shape::Skip;
        }
        let context = match self.context {
            Context::Allocated(helper) => helper,
            Context::Null => return Shape::Skip,
            Context::Unknown(cause) => {
                return Shape::Unsettled(format!(
                    "AI strategy {cause} needs the global strategy-specific data table"
                ));
            }
        };
        let keyword = field.keyword();
        let Some(reader) = helper_field(context, keyword.as_deref()) else {
            return Shape::Skip;
        };
        match reader {
            ValueReader::One => Shape::One,
            ValueReader::Values => Shape::ValueList,

            ValueReader::Trigger => Shape::Body(Node::Trigger(crate::trigger_grammar::Node::Trigger)),
            ValueReader::OrderType => accepted(token_is_one_of(
                field.value,
                &engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.order_types)),
            )),
            ValueReader::ExecutionType => accepted(token_is_one_of(
                field.value,
                &engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.execution_types)),
            )),
            ValueReader::Tag => self.tag(field),
            ValueReader::State => self.index(field, false),
            ValueReader::Region => self.index(field, true),
            ValueReader::Area => self.area(field),
        }
    }

    fn tag(&self, field: &Field<'_>) -> Shape<Node> {
        if !field.skips_block() {
            return Shape::Skip;
        }
        let Some(names) = self.names else {
            return Shape::Unsettled("AI strategy country tags were not supplied".into());
        };
        accepted(names.countries.contains(field.value.c_str()))
    }

    fn area(&self, field: &Field<'_>) -> Shape<Node> {
        if !field.skips_block() {
            return Shape::Skip;
        }
        Shape::Reported {
            kind: content_reader_parser::DiagnosticKind::Unsupported,
            report:
                "Deferred effect AI strategy AI area: block consumption needs the AI-area registry"
                    .into(),
            read: Box::new(Shape::Unsettled(
                "AI area block needs the unavailable registry".into(),
            )),
        }
    }

    fn index(&self, field: &Field<'_>, region: bool) -> Shape<Node> {
        if !field.skips_block() {
            return Shape::Skip;
        }
        let what = if region { "strategic region" } else { "state" };
        Shape::Reported {
            kind: content_reader_parser::DiagnosticKind::Unsupported,
            report: format!(
                "Deferred effect AI strategy {what}: block consumption needs the indexed registry"
            ),
            read: Box::new(Shape::Unsettled(format!(
                "AI strategy {what} block needs the unavailable registry"
            ))),
        }
    }
}
