//! Ordered combat-tactic identities, fields and effects from the recorded
//! content stack: the reader of `common/combat_tactics.txt`.
//!
//! ID 0 is the null entry; each top-level entry except exact text `phases`
//! constructs a tactic, so `Phases` is a tactic. IDs follow source order,
//! including duplicate names. One reader processes the file token by token
//! ([`native`]). Definitions `@name = value` are consumed wherever it reads
//! by key, taking no ID and contributing no field; later references resolve
//! their values. Each field uses its own reader ([`Shaper`]); deviations
//! from balanced-tree reading follow the game and join [`Database::logged`].
//! An unsettled block (`native::Read::unsettled`) leaves the current and all
//! later tactics unproven ([`Tactic::unproven`]). Lexer-assigned unlock IDs
//! are campaign state and are not inferred here.

use std::io;

mod body_keys;
mod own;

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
    /// Phase names in declaration order across all `phases` blocks.
    pub phases: Vec<Vec<u8>>,
    /// Tactic ID 1 is the first entry; ID 0 is the native null entry.
    pub tactics: Vec<Tactic>,
    /// Where the file's reading cannot be followed, naming the place and
    /// why: the tactic being read then and every later one are unproven.
    pub unsupported: Vec<String>,
    /// Errors the game's readers log and read past, and the places the
    /// game's reading leaves the text, each read as the game reads it.
    pub logged: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tactic {
    pub id: i32,
    /// Raw authored name, preserving case, bytes, and duplicate identities.
    pub token: Vec<u8>,
    /// Reader-projected fields: definitions consumed and every value token
    /// resolved. Scalar `trigger` fields report an error, contribute no
    /// condition, and are omitted.
    pub fields: Vec<Item>,
    /// `phase` and `display_phase`: an exact phase-name index among phases
    /// read before this tactic, [`NO_PHASE`] for literal `no`, and
    /// [`UNKNOWN_PHASE`] when absent or unmatched.
    pub phase: i32,
    pub display_phase: i32,
    /// Effects in [`effect_slot`] order, retaining zero for absent fields.
    pub effects: [i64; 6],
    /// Why this tactic's ID, phase IDs or fields are unproven: this or an
    /// earlier entry holds a block the port cannot settle.
    pub unproven: Option<String>,
}

/// A phase field that is absent or names no phase read before it.
pub const UNKNOWN_PHASE: i32 = -1;
/// A phase field spelled exactly `no`.
pub const NO_PHASE: i32 = -2;

/// A folded effect keyword's slot in [`Tactic::effects`]. The fixed slot order
/// is shared with tactic tooltip and popup consumers.
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

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    /// The tactic-file loop.
    Top,
    Tactic,
    /// A trigger body: a tactic's `trigger`,
    /// every block under it or under a weight modifier, a scripted
    /// trigger's body.
    Trigger,
    /// A weight's `modifier` reads `factor` and `add` as one scoped-variable
    /// value each, before trigger dispatch. These fields skip no block;
    /// other keys are triggers.
    Modifier,
    /// A bound-localisation block: `localization_key` reads one formatted
    /// value; other keys bind arguments, recursively for opening values.
    BoundLocalization,
    /// A conditional body (`if`, `else`, `else_if`), indexed in [`Shaper`].
    Conditional(usize),
    /// A conditional's limit, retaining children added by trigger dispatch.
    Limit(usize),
    /// The body of a coded trigger, by its token, whose class reads its keys
    /// through a key reader of its own ([`trigger_keys::key_reader`]).
    Class(u32),
    /// A specialized key loop selected by its reader identity.
    KeyReader(u64),
    /// A weight: a tactic's base or railway priority.
    Mtth,
    CollectionOperator,
    ScalarFields,
}

/// What a conditional's limit holds as its loader fills it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Limit {
    Empty,
    Children,
    /// Whether it holds a child is not known where it is read, and why.
    Unknown(String),
}

/// Trigger field dispatch and per-read conditional state. `limit` reads
/// only while its current body has no child; later limits use error recovery,
/// reading and defining nothing. Shared by tactic and scripted triggers,
/// tag aliases, scripted localisation, and content-reader conditions
/// ([`Shape::Trigger`]).
pub struct Shaper<'n> {
    limits: Vec<Limit>,
    /// The content names trigger dispatch reads at this point of the load; `None`
    /// where the reader was given none, so that a key naming content is
    /// unknown ([`key_names::load_key`]).
    names: Option<&'n KeyNames>,
}

impl<'n> Shaper<'n> {
    pub fn new(names: Option<&'n KeyNames>) -> Self {
        Self {
            limits: Vec::new(),
            names,
        }
    }

    /// Tactic field dispatch by keyword identity: bare keys fold; quoted keys
    /// are not keywords.
    pub fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        let keyword = field.keyword();
        let keyword = keyword.as_deref();
        match node {
            Node::KeyReader(address) => self.native_key(address, field),
            // The weight uses this Shaper's modifier reader so nested triggers
            // retain its names and limits.
            Node::Mtth if keyword == Some(b"modifier") => Shape::Body(Node::Modifier),
            Node::Mtth => Shape::One,
            Node::CollectionOperator => self.collection_operator(field),
            Node::ScalarFields => Shape::One,
            // `phases` compares text exactly and reads a string list, resolving
            // references without registering definitions. Every other key reads
            // a tactic object regardless of whether its value opens a block.
            Node::Top if c_string(&field.key.text) == b"phases" => Shape::List,
            Node::Top => Shape::Object(Node::Tactic),
            Node::Tactic => match keyword {
                // `base` reads a weight object.
                Some(b"base") => Shape::Object(Node::Mtth),
                // An AND trigger reads no condition from a value that opens no block.
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
                // Report and skip an unexpected field.
                _ => Shape::Skip,
            },
            // Dispatch the key regardless of the field-read result.
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
            // Each class reads its own keys with its value reader; fallback
            // uses trigger dispatch or unexpected-field recovery.
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

    /// A new conditional's body, its limit empty.
    fn conditional(&mut self) -> Shape<Node> {
        self.limits.push(Limit::Empty);
        Shape::Body(Node::Conditional(self.limits.len() - 1))
    }

    /// Whether a limit field adds a child ([`Load::adds_child`]).
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

    /// Read one trigger child. Every field reaches dispatch, including keys
    /// without operators. Such keys retain the last value slot; error skips
    /// test the token consumed in place of the operator ([`Field::skips_block`]).
    /// Classification ([`key_names::load_key`]) selects consumption:
    /// - coded single-value triggers report failed reads without skipping, so
    ///   a block becomes the enclosing trigger body and its `}` ends it; body
    ///   readers consume nested children, and specialized readers follow [`own`];
    /// - scopes read a context trigger body;
    /// - content counts and scripted triggers read one value and skip no block;
    /// - invalid keys skip the block at the current lexer token;
    /// - ambiguous names leave divergent block consumption unsettled and refused.
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

/// Resolve a phase field as it is read: exact text `no` first, then an
/// exact match against phases registered so far. `NO` is a phase name.
fn phase_id(phases: &[Vec<u8>], value: &Value) -> i32 {
    // Phase comparison uses first-NUL value text.
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

/// Read the effective, hash-verified file from the recorded content layers.
pub fn read(layers: &ResolvedLayers) -> io::Result<Database> {
    Ok(parse(
        &layers.read("common/combat_tactics.txt")?,
        &*layers.key_names()?,
    ))
}

/// Parse one combat-tactics file using initialized runtime metadata, with
/// authored triggers resolved against `names`.
pub fn parse(bytes: &[u8], names: &KeyNames) -> Database {
    // The file lexer uses mode 0.
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
        // The key comparison and tactic name use the same first-NUL text.
        let token = c_string(item.key.as_deref().unwrap_or_default()).to_vec();
        if token == b"phases" {
            // Every list entry, including `=`, is a phase name. Text ends at its
            // first NUL; line-feed text uses the shared unreadable-string value.
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
            // Only a block the tactic reader reads is a tactic's value.
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
                // The AND trigger and the weight's modifiers read triggers.
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
        // The tactic being read then, and every later one, which the text
        // after it holds.
        let reason = format!("combat_tactics.txt {unsettled}");
        database.unsupported.push(reason.clone());
        if let Some(last) = database.tactics.last_mut() {
            last.unproven = Some(reason);
        }
    }
    database
}

/// Project tactic fields retained by trigger and weight readers; their
/// reported errors join `logged`.
fn keep(tactic: &str, field: &mut Item, logged: &mut Vec<String>) -> bool {
    let key = String::from_utf8_lossy(field.key.as_deref().unwrap_or_default()).into_owned();
    match (
        field.keyword().map(|keyword| keyword.folded()).as_deref(),
        &mut field.value,
    ) {
        // A value without an opening brace reports an error and consumes nothing.
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

/// How a value reads in a message: its text, or that it is a block.
fn shown(value: &Value) -> String {
    match value {
        Value::Scalar(text) => String::from_utf8_lossy(text).into_owned(),
        Value::Block(_) | Value::Tagged { .. } => "{ ... }".into(),
    }
}

/// Reads effect occurrences in source order. Movement speed keeps its previous
/// value after a failed scan; the other fields use decimal fixed-point conversion.
/// Undefined bare references retain their own text as the value.
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

/// Normalises scalar AND, OR, NOT and custom-tooltip groups to empty bodies,
/// retaining the logged error. Empty AND and NOT evaluate true, OR false.
/// Custom-tooltip localisation bindings have already been consumed and add no
/// condition, even when authored content defines a trigger with the same name.
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
                    // Localisation argument keys are not triggers.
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

/// A `phase` trigger in tactic conditions or weight modifiers resolves
/// while being read. Exact `no` and unregistered names store -1; evaluation
/// compares the stored ID with the combat phase. Replace the operand with
/// that ID in decimal for viewer evaluation. The trigger name is a folded
/// keyword; the operand's `no` comparison is exact.
pub fn resolve_phase_triggers(phases: &[Vec<u8>], items: &mut [Item]) {
    for item in items {
        let is_phase = item.keyword_is("phase");
        match &mut item.value {
            Value::Scalar(operand) if is_phase => {
                // Compare `no` after truncating the operand at its first NUL.
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
