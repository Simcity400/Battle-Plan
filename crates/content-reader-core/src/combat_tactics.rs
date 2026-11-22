
use std::io;

mod body_keys;
pub(crate) mod own;

use crate::{
    ResolvedLayers,
    key_names::{self, KeyNames, Load},
    parser::{
        Item, Value, c_string,
        native::{self, Field, Shape},
    },
    trigger_keys::{self, KeyReader, Reader},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    pub phases: Vec<Vec<u8>>,
    pub tactics: Vec<Tactic>,
    pub unsupported: Vec<String>,
    pub logged: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tactic {
    pub id: i32,
    pub token: Vec<u8>,
    pub fields: Vec<Item>,
    pub phase: i32,
    pub display_phase: i32,
    pub effects: [i64; 6],
    pub unproven: Option<String>,
}

pub const UNKNOWN_PHASE: i32 = -1;
pub const NO_PHASE: i32 = -2;

pub fn effect_slot(keyword: &[u8]) -> Option<usize> {
    match keyword {
        b"combat_width" => Some(0),
        b"attacker_movement_speed" => Some(1),
        b"attacker" => Some(2),
        b"attacker_org_damage_modifier" => Some(3),
        b"defender" => Some(4),
        b"defender_org_damage_modifier" => Some(5),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    Top,
    Tactic,
    Trigger,
    Modifier,
    BoundLocalization,
    Conditional(usize),
    Limit(usize),
    Class(u32),
    KeyReader(u64),
    Mtth,
    CollectionOperator,
    ScalarFields,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Limit {
    Empty,
    Children,
    Unknown(String),
}

pub struct Shaper<'n> {
    limits: Vec<Limit>,
    names: Option<&'n KeyNames>,
}

impl<'n> Shaper<'n> {
    pub fn new(names: Option<&'n KeyNames>) -> Self {
        Self {
            limits: Vec::new(),
            names,
        }
    }

    pub fn set_names(&mut self, names: &'n KeyNames) {
        self.names = Some(names);
    }

    pub fn needs_names(&self, node: Node, field: &Field<'_>) -> bool {
        if self.names.is_some() {
            return false;
        }
        let keyword = field.keyword();
        let keyword = keyword.as_deref();
        match node {
            Node::Limit(_) | Node::Trigger | Node::Modifier => {
                matches!(load_trigger_key(None, field), Load::Unknown(_))
            }
            Node::Conditional(_) if keyword == Some(b"limit") => false,
            Node::Conditional(_) => {
                matches!(load_trigger_key(None, field), Load::Unknown(_))
            }
            Node::Class(token) => {
                let reader = trigger_keys::key_reader(token);
                if let KeyReader::ThenLoadKey(keys) | KeyReader::Only(keys) = reader
                    && let Some(key) = keys
                        .iter()
                        .find(|key| keyword == Some(key.name().as_bytes()))
                {
                    return matches!(key, trigger_keys::OwnKey::Value(_))
                        && Self::body_value_needs_names(token, field);
                }
                matches!(reader, KeyReader::ThenLoadKey(_))
                    && matches!(load_trigger_key(None, field), Load::Unknown(_))
            }
            Node::KeyReader(_)
            | Node::Mtth
            | Node::CollectionOperator
            | Node::ScalarFields
            | Node::Top
            | Node::Tactic
            | Node::BoundLocalization => false,
        }
    }

    pub fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        let keyword = field.keyword();
        let keyword = keyword.as_deref();
        match node {
            Node::KeyReader(address) => self.native_key(address, field),
            Node::Mtth if keyword == Some(b"modifier") => Shape::Body(Node::Modifier),
            Node::Mtth => Shape::One,
            Node::CollectionOperator => self.collection_operator(field),
            Node::ScalarFields => Shape::One,
            Node::Top if c_string(&field.key.text) == b"phases" => Shape::List,
            Node::Top => Shape::Object(Node::Tactic),
            Node::Tactic => match keyword {
                Some(b"base") => Shape::Object(Node::Mtth),
                Some(b"trigger") => Shape::Body(Node::Trigger),
                Some(keyword)
                    if effect_slot(keyword).is_some()
                        || matches!(
                            keyword,
                            b"is_attacker"
                                | b"active"
                                | b"picture"
                                | b"phase"
                                | b"display_phase"
                                | b"countered_by"
                                | b"only_show_for"
                                | b"aggressiveness"
                        ) =>
                {
                    Shape::One
                }
                _ => Shape::Skip,
            },
            Node::Modifier if matches!(keyword, Some(b"factor" | b"add")) => Shape::One,
            Node::BoundLocalization if keyword == Some(b"localization_key") => Shape::One,
            Node::BoundLocalization => Shape::ObjectIfOpen(Node::BoundLocalization),
            Node::Conditional(index) if keyword == Some(b"limit") => match &self.limits[index] {
                Limit::Empty => Shape::Body(Node::Limit(index)),
                Limit::Children => Shape::Skip,
                Limit::Unknown(why) => Shape::Unsettled(format!(
                    "a conditional trigger reads a later limit only into an empty one, and whether the first holds a child is not known here: {why}"
                )),
            },
            Node::Limit(index) => {
                let load = load_trigger_key(self.names, field);
                self.note_child(index, &load);
                self.trigger_field(field, load)
            }
            Node::Class(token) => {
                let reader = trigger_keys::key_reader(token);
                if let KeyReader::ThenLoadKey(keys) | KeyReader::Only(keys) = reader
                    && let Some(key) = keys
                        .iter()
                        .find(|key| keyword == Some(key.name().as_bytes()))
                {
                    return match key {
                        trigger_keys::OwnKey::Value(_) => self.body_value(token, field),
                        trigger_keys::OwnKey::BoundLocalization(_) => {
                            Shape::ObjectIfOpen(Node::BoundLocalization)
                        }
                    };
                }
                match reader {
                    KeyReader::Only(_) => Shape::Skip,
                    KeyReader::Native(address) => self.shape(Node::KeyReader(address), field),
                    KeyReader::Refused(address) => Self::refuse_reader(
                        address,
                        trigger_keys::REGISTERED_TRIGGERS
                            .iter()
                            .find(|(id, _)| *id == token)
                            .expect("a class token names a registered trigger")
                            .1
                            .as_bytes(),
                    ),
                    KeyReader::ThenLoadKey(_) => self.shape(Node::Trigger, field),
                }
            }
            Node::Trigger | Node::Modifier | Node::Conditional(_) => {
                let load = load_trigger_key(self.names, field);
                self.trigger_field(field, load)
            }
        }
    }

    fn conditional(&mut self) -> Shape<Node> {
        self.limits.push(Limit::Empty);
        Shape::Body(Node::Conditional(self.limits.len() - 1))
    }

    fn note_child(&mut self, index: usize, load: &Load) {
        if self.limits[index] == Limit::Children {
            return;
        }
        match load {
            _ if load.adds_child() == Some(true) => self.limits[index] = Limit::Children,
            Load::Unknown(why) if self.limits[index] == Limit::Empty => {
                self.limits[index] = Limit::Unknown(why.clone());
            }
            _ => {}
        }
    }

    fn refuse_reader(_reader_identity: u64, name: &[u8]) -> Shape<Node> {
        Shape::Unsettled(format!(
            "unported trigger reader for {}",
            String::from_utf8_lossy(name)
        ))
    }

    fn trigger_field(&mut self, field: &Field<'_>, load: Load) -> Shape<Node> {
        let keyword = field.keyword();
        let keyword = keyword.as_deref();
        match load {
            Load::Conditional => self.conditional(),
            Load::Coded(_, _) if keyword == Some(b"if") => self.conditional(),
            Load::Coded(token, reader) => match reader {
                Reader::Assign | Reader::Comparison => Shape::One,
                Reader::Body => Shape::Body(Node::Class(token)),
                Reader::Own(address) => self.own_value(token, address, field),
            },
            Load::Scope => Shape::Body(Node::Trigger),
            Load::OneValue => Shape::One,
            Load::Invalid => Shape::Skip,
            Load::Unknown(why) if field.opens() || field.skips_block() => {
                Shape::Unsettled(format!(
                    "trigger dispatch reads it as a scope's body, one value or an invalid key's skip, and which is not known here: {why}"
                ))
            }
            Load::Unknown(_) => Shape::One,
        }
    }
}

fn token_only_scope(field: &Field<'_>) -> bool {
    struct TokenOnly;

    impl crate::scope::Names for TokenOnly {
        fn is_country(&self, _tag: &str) -> bool {
            false
        }

        fn is_character(&self, _token: &str) -> bool {
            false
        }

        fn is_keyword(&self, text: &str) -> bool {
            ["mio", "sp", "scope"]
                .iter()
                .any(|prefix| text.eq_ignore_ascii_case(prefix))
        }
    }

    if !field.opens() {
        return false;
    }
    let Ok(text) = std::str::from_utf8(c_string(&field.key.text)) else {
        return false;
    };
    !matches!(
        crate::scope::parse(text, field.key.kind == native::Kind::Quoted, &TokenOnly),
        crate::scope::Scope::Unknown(_)
    )
}

fn load_trigger_key(names: Option<&KeyNames>, field: &Field<'_>) -> Load {
    let load = key_names::load_key(names, field);
    if names.is_none() && matches!(load, Load::Unknown(_)) && token_only_scope(field) {
        Load::Scope
    } else {
        load
    }
}

fn phase_id(phases: &[Vec<u8>], value: &Value) -> i32 {
    let Some(name) = value.scalar().map(c_string) else {
        return UNKNOWN_PHASE;
    };
    if name == b"no" {
        return NO_PHASE;
    }
    phases
        .iter()
        .position(|phase| phase.as_slice() == name)
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(UNKNOWN_PHASE)
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Database> {
    Ok(parse(
        &layers.read("common/combat_tactics.txt")?,
        &*layers.key_names()?,
    ))
}

pub fn parse(bytes: &[u8], names: &KeyNames) -> Database {
    let mut shaper = Shaper::new(Some(names));
    let read = native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        Some(names),
        &mut |node, field| shaper.shape(node, field),
    );
    let mut database = Database {
        logged: read
            .misreads
            .iter()
            .map(|misread| format!("combat_tactics.txt {misread}; read as the game reads it"))
            .collect(),
        ..Database::default()
    };
    for item in read.items {
        let token = c_string(item.key.as_deref().unwrap_or_default()).to_vec();
        if token == b"phases" {
            for entry in item.value.items().unwrap_or_default() {
                database
                    .phases
                    .extend(entry.value.read_string().map(<[u8]>::to_vec));
            }
            continue;
        }
        let id = i32::try_from(database.tactics.len() + 1)
            .expect("a file's tactics are fewer than i32::MAX");
        let name = String::from_utf8_lossy(&token).into_owned();
        let mut fields = match item.value {
            Value::Block(fields) => fields,
            Value::Scalar(_) | Value::Tagged { .. } => Vec::new(),
        };
        fields.retain_mut(|field| keep(&name, field, &mut database.logged));
        let (mut phase, mut display_phase) = (UNKNOWN_PHASE, UNKNOWN_PHASE);
        for field in &mut fields {
            let keyword = field.keyword().map(|keyword| keyword.folded());
            match (keyword.as_deref(), &mut field.value) {
                (Some(b"phase"), value) => phase = phase_id(&database.phases, value),
                (Some(b"display_phase"), value) => {
                    display_phase = phase_id(&database.phases, value)
                }
                (Some(b"trigger"), Value::Block(items)) => {
                    let place = format!("tactic {name}'s trigger");
                    empty_scalar_groups(&place, items, &mut database.logged);
                    resolve_phase_triggers(&database.phases, items)
                }
                (Some(b"base"), Value::Block(items)) => {
                    for modifier in items.iter_mut().filter(|item| item.keyword_is("modifier")) {
                        if let Value::Block(conditions) = &mut modifier.value {
                            let place = format!("tactic {name}'s base's modifier");
                            empty_scalar_groups(&place, conditions, &mut database.logged);
                        }
                    }
                    resolve_phase_triggers(&database.phases, items)
                }
                _ => {}
            }
        }
        let effects = effects(&name, &fields, &mut database.logged);
        database.tactics.push(Tactic {
            id,
            token,
            fields,
            phase,
            display_phase,
            effects,
            unproven: None,
        });
    }
    if let Some(unsettled) = read.unsettled {
        let reason = format!("combat_tactics.txt {unsettled}");
        database.unsupported.push(reason.clone());
        if let Some(last) = database.tactics.last_mut() {
            last.unproven = Some(reason);
        }
    }
    database
}

fn keep(tactic: &str, field: &mut Item, logged: &mut Vec<String>) -> bool {
    let key = String::from_utf8_lossy(field.key.as_deref().unwrap_or_default()).into_owned();
    match (
        field.keyword().map(|keyword| keyword.folded()).as_deref(),
        &mut field.value,
    ) {
        (Some(b"trigger"), Value::Scalar(text)) => {
            logged.push(format!(
                "tactic {tactic}'s {key} = {} is not a block: the trigger reader reports an error and reads no condition from it",
                String::from_utf8_lossy(text)
            ));
            false
        }
        (Some(b"base"), Value::Block(entries)) => {
            for entry in entries.iter_mut() {
                let name =
                    String::from_utf8_lossy(entry.key.as_deref().unwrap_or_default()).into_owned();
                let keyword = entry.keyword().map(|keyword| keyword.folded());
                match (keyword.as_deref(), &entry.value) {
                    (Some(b"modifier"), Value::Scalar(text)) => {
                        logged.push(format!(
                            "tactic {tactic}'s {key}'s {name} = {} is not a block: the modifier reader reports an error and keeps a modifier with no condition, factor 1 and add 0",
                            String::from_utf8_lossy(text)
                        ));
                        entry.value = Value::Block(Vec::new());
                    }
                    (Some(b"modifier"), _) => {}
                    (Some(b"factor" | b"base" | b"days" | b"months" | b"years"), _) => {}
                    (_, value) => logged.push(format!(
                        "tactic {tactic}'s {key}'s {name} = {}: the weight reader reports an unknown command and reads on",
                        shown(value)
                    )),
                }
            }
            true
        }
        _ => true,
    }
}

fn shown(value: &Value) -> String {
    match value {
        Value::Scalar(text) => String::from_utf8_lossy(text).into_owned(),
        Value::Block(_) | Value::Tagged { .. } => "{ ... }".into(),
    }
}

fn effects(tactic: &str, fields: &[Item], logged: &mut Vec<String>) -> [i64; 6] {
    let mut values = [0_i64; 6];
    for field in fields {
        let Some(slot) = field.keyword().and_then(|keyword| {
            let keyword = keyword.folded();
            effect_slot(&keyword)
        }) else {
            continue;
        };
        let key = String::from_utf8_lossy(field.key.as_deref().unwrap_or_default());
        let value = field.value.scalar().unwrap_or_default();
        let text = String::from_utf8_lossy(value);
        if slot == 1 {
            match crate::fixed5::script_fixed5(value) {
                Some(read) => values[slot] = read,
                None => logged.push(format!(
                    "tactic {tactic}'s {key} = {text} does not scan: the game reports it and the field keeps its previous value"
                )),
            }
        } else {
            values[slot] = crate::fixed5::decimal_fixed5(value);
        }
        if !field.quoted() && value.starts_with(b"@") {
            logged.push(format!(
                "tactic {tactic}'s {key} names the variable {text}, which is not defined before it, so the game reads that text as the value"
            ));
        }
    }
    values
}

pub fn empty_scalar_groups(place: &str, items: &mut [Item], logged: &mut Vec<String>) {
    normalise_trigger_body(place, items, logged, false);
}

pub(crate) fn normalise_trigger_body(
    place: &str,
    items: &mut [Item],
    logged: &mut Vec<String>,
    tooltip_bindings: bool,
) {
    for item in items {
        let custom_tooltip = item.keyword_is("custom_trigger_tooltip");
        let bindings = custom_tooltip || item.keyword_is("custom_override_tooltip");
        let group = item.keyword().is_some_and(|keyword| {
            matches!(
                keyword.folded().as_slice(),
                b"and" | b"or" | b"not" | b"custom_trigger_tooltip"
            )
        });
        match &mut item.value {
            Value::Scalar(text) if group => {
                logged.push(format!(
                    "{place}: {} = {} is not a block: the group reader reports an error and reads no child, and the game keeps the empty group",
                    String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()),
                    String::from_utf8_lossy(text)
                ));
                item.value = Value::Block(Vec::new());
            }
            Value::Block(children) => {
                if custom_tooltip && !tooltip_bindings {
                    children.retain(|child| {
                        !child.keyword_is("tooltip") && !child.keyword_is("not_tooltip")
                    });
                }
                for child in children {
                    if tooltip_bindings
                        && bindings
                        && (child.keyword_is("tooltip") || child.keyword_is("not_tooltip"))
                    {
                        continue;
                    }
                    normalise_trigger_body(
                        place,
                        std::slice::from_mut(child),
                        logged,
                        tooltip_bindings,
                    );
                }
            }
            Value::Scalar(_) | Value::Tagged { .. } => {}
        }
    }
}

pub fn resolve_phase_triggers(phases: &[Vec<u8>], items: &mut [Item]) {
    for item in items {
        let is_phase = item.keyword_is("phase");
        match &mut item.value {
            Value::Scalar(operand) if is_phase => {
                let id = if c_string(operand) == b"no" {
                    UNKNOWN_PHASE
                } else {
                    phase_id(phases, &Value::Scalar(operand.clone())).max(UNKNOWN_PHASE)
                };
                *operand = id.to_string().into_bytes();
            }
            Value::Block(items) => resolve_phase_triggers(phases, items),
            Value::Scalar(_) | Value::Tagged { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests;
