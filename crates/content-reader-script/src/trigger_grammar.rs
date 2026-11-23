mod body_keys;
pub mod own;

use crate::{
    key_names::{self, KeyNames, Load},
    native::{Field, Shape},
    trigger_keys::{self, KeyReader, Reader},
};
use content_reader_parser::parser::{Item, Value, c_string};

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
                matches!(key_names::load_key(None, field), Load::Unknown(_))
            }
            Node::Conditional(_) if keyword == Some(b"limit") => false,
            Node::Conditional(_) => {
                matches!(key_names::load_key(None, field), Load::Unknown(_))
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
                    && matches!(key_names::load_key(None, field), Load::Unknown(_))
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
                let load = key_names::load_key(self.names, field);
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
                let load = key_names::load_key(self.names, field);
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

pub fn empty_scalar_groups(place: &str, items: &mut [Item], logged: &mut Vec<String>) {
    normalise_trigger_body(place, items, logged, false);
}

pub fn normalise_trigger_body(
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
