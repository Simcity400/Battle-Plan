//! Ordered combat-tactic identities, fields and effects from matching game
//! content: the one reader of `common/combat_tactics.txt`.
//!
//! Native 1406B3690 reserves tactic ID 0 for the null entry, then constructs
//! one tactic for every top-level entry except `phases`. It matches that key
//! by its text, not its keyword ID: 1406B3875 compares the six bytes
//! `phases`, so `Phases` is a tactic. IDs follow source order, including
//! duplicate names. The file has one reader, replayed token by token
//! ([`native`]): a definition `@name = value` (1424AC830) is consumed
//! wherever the reader reads by key, so it takes no ID and is no field, and
//! a value read after it takes its value (1424AB540). Each field goes to its
//! own native reader ([`Shaper`]); where the text leaves the tree's shape the
//! reading follows the game's and is noted ([`Database::logged`]). Only a
//! block the port cannot settle (`native::Read::unsettled`) leaves the
//! tactic being read and every later one unproven
//! ([`Tactic::unproven`]). The lexer-assigned unlock IDs are campaign state
//! and are deliberately not inferred here.

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
    /// The fields the game reads for this tactic, as the reader replay
    /// returns them: definitions consumed, every value token resolved. A
    /// scalar `trigger`, which CTrigger::LoadBody logs and reads nothing
    /// of, is left out.
    pub fields: Vec<Item>,
    /// `phase` and `display_phase` as 1406B3CB0 stores them (+0x168,
    /// +0x16C): the phase's index among the phases read before this tactic
    /// (1406B35C0, an exact match), [`NO_PHASE`] for the literal `no`, and
    /// [`UNKNOWN_PHASE`] when absent (1406B0A70's default) or unmatched.
    pub phase: i32,
    pub display_phase: i32,
    /// The [`EFFECTS`] in that order, as their readers leave them (0 when
    /// absent, 1406B0A70).
    pub effects: [i64; 6],
    /// Why this tactic's ID, phase IDs or fields are unproven: this or an
    /// earlier entry holds a block the port cannot settle.
    pub unproven: Option<String>,
}

/// A phase field that is absent or names no phase read before it.
pub const UNKNOWN_PHASE: i32 = -1;
pub const NO_PHASE: i32 = -2;

/// The six effects 1406B3CB0 stores at +0x138..+0x160, which the tactic
/// tooltip (1406B12D0) and the possible-tactics list (141C5C1F0) show.
/// `attacker_movement_speed` is read through ReadFixed5 (1424ABB10 ->
/// 1424B0480, `%lli`: `010` is eight), the others through
/// ParseTokenFixedPoint64 (1424AFF10 -> 1424B8600).
pub const EFFECTS: [&str; 6] = [
    "combat_width",
    "attacker_movement_speed",
    "attacker",
    "attacker_org_damage_modifier",
    "defender",
    "defender_org_damage_modifier",
];

/// The other keys 1406B3CB0 reads as one value. Any other key is reported
/// (1424A9CE0 -> 1424AD100 -> 1424ACF90), which skips its block through
/// 1424AD270 and defines nothing.
const SINGLE_VALUES: [&str; 8] = [
    "is_attacker",
    "active",
    "picture",
    "phase",
    "display_phase",
    "countered_by",
    "only_show_for",
    "aggressiveness",
];

/// The weight's one-value keys (CMeanTimeToHappen::vf4 140545850):
/// `factor` (ReadFixed5 1424ABB10), `base` (1424AFF10), `days`, `months`
/// and `years` (1424B0080).
const WEIGHT_VALUES: [&str; 5] = ["factor", "base", "days", "months", "years"];

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Node {
    /// 1406B3690's file loop.
    Top,
    /// 1406B3CB0.
    Tactic,
    /// A trigger body (CTrigger::LoadBody 1405431C0): a tactic's `trigger`,
    /// every block under it or under a weight modifier, a scripted
    /// trigger's body.
    Trigger,
    /// A weight's `modifier`, a CMTTHModifier: LoadBody with its own key
    /// reader (vtable slot 6, 140545820), which takes `factor` (0x296B) and
    /// `add` (0x35EA) before the trigger registry as scoped variables
    /// (CScopedVariable::vf3 140539440 -> 140539B50), each one value token
    /// with no block skipped; any other key is a trigger (LoadKey).
    Modifier,
    /// CBoundLocalization's block reader (1423912A0): `localization_key`
    /// reads one CFormattedLocalization value (142393E90), and every other
    /// key binds an argument, recursively when its value opens a block.
    BoundLocalization,
    /// A conditional's body (`if`, `else`, `else_if`: CIfTrigger::load_key
    /// 141395B90), by its index in [`Shaper`].
    Conditional(usize),
    /// A conditional's limit (the CAndTrigger at +0x58), LoadKey's children.
    Limit(usize),
    /// The body of a coded trigger, by its token, whose class reads its keys
    /// through a key reader of its own ([`trigger_keys::key_reader`]).
    Class(u32),
    /// A ported own reader's key loop, by its native address.
    KeyReader(u64),
    /// CMeanTimeToHappen@140545850: a tactic's base or railway priority.
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

/// The trigger readers' field dispatch over one read, with the state
/// CIfTrigger::load_key (141395B90) keeps: a `limit` is read (LoadBody
/// into +0x58) only while the limit holds no child (+0x6C); a later one is
/// RecordError (1424ACF90), whose skip (1424AD270) reads and defines
/// nothing. Every trigger body the port reads goes through it: the tactics,
/// the scripted triggers, the tag aliases, the scripted localisation and
/// each content reader's conditions ([`Shape::Trigger`]).
pub(crate) struct Shaper<'n> {
    limits: Vec<Limit>,
    /// The content names LoadKey reads at this point of the load; `None`
    /// where the reader was given none, so that a key naming content is
    /// unknown ([`key_names::load_key`]).
    names: Option<&'n KeyNames>,
}

fn is_one_of(keyword: Option<&[u8]>, names: &[&str]) -> bool {
    keyword.is_some_and(|keyword| names.iter().any(|name| name.as_bytes() == keyword))
}

impl<'n> Shaper<'n> {
    pub(crate) fn new(names: Option<&'n KeyNames>) -> Self {
        Self {
            limits: Vec::new(),
            names,
        }
    }

    /// The tactic file's field dispatch, by keyword id (register_authored_keys
    /// 140082CB0): a bare key folds, a quoted one is token 0xF (1424A7A90).
    pub(crate) fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        let keyword = field.keyword();
        let keyword = keyword.as_deref();
        match node {
            Node::KeyReader(address) => self.native_key(address, field),
            // CMeanTimeToHappen::vf4@140545850 uses this Shaper's
            // CMTTHModifier so nested triggers retain its names and limits.
            Node::Mtth if keyword == Some(b"modifier") => Shape::Body(Node::Modifier),
            Node::Mtth => Shape::One,
            Node::CollectionOperator => self.collection_operator(field),
            Node::ScalarFields => Shape::One,
            // A text compare, not a keyword (1406B3875): a string list
            // (ReadStringList 1401426E0), which resolves references and defines
            // nothing. Every other key is a tactic, read through
            // ReadObjectFromParser whatever its value (CPersistent::Read
            // 1424A9730 does not check for `{`).
            Node::Top if c_string(&field.key.text) == b"phases" => Shape::List,
            Node::Top => Shape::Object(Node::Tactic),
            Node::Tactic => match keyword {
                // `base` 0x2C86: a CMeanTimeToHappen through ReadObjectFromParser.
                Some(b"base") => Shape::Object(Node::Mtth),
                // `trigger` 0x2963: a CAndTrigger read by CTrigger::LoadBody,
                // which reads nothing of a value that opens no block.
                Some(b"trigger") => Shape::Body(Node::Trigger),
                _ if is_one_of(keyword, &EFFECTS) || is_one_of(keyword, &SINGLE_VALUES) => {
                    Shape::One
                }
                // Serializable_ReportUnexpectedToken (1424A9CE0).
                _ => Shape::Skip,
            },
            // 140545820 dispatches the key slot whatever 1424AC5E0 read.
            Node::Modifier if matches!(keyword, Some(b"factor" | b"add")) => Shape::One,
            Node::BoundLocalization if keyword == Some(b"localization_key") => Shape::One,
            Node::BoundLocalization => Shape::ObjectIfOpen(Node::BoundLocalization),
            Node::Conditional(index) if keyword == Some(b"limit") => match &self.limits[index] {
                Limit::Empty => Shape::Body(Node::Limit(index)),
                Limit::Children => Shape::Skip,
                Limit::Unknown(why) => Shape::Unsettled(format!(
                    "CIfTrigger::load_key (141395B90) reads a later limit only into an empty one, and whether the first holds a child is not known here: {why}"
                )),
            },
            Node::Limit(index) => {
                let load = key_names::load_key(self.names, field);
                self.note_child(index, &load);
                self.trigger_field(field, load)
            }
            // Each class's own keys use its native value reader; its
            // fallback is LoadKey or ReportUnexpectedToken's error skip.
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

    /// Whether a field of a limit adds a child to it (LoadKey 1405434F0,
    /// [`Load::adds_child`]).
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

    fn refuse_reader(address: u64, name: &[u8]) -> Shape<Node> {
        Shape::Unsettled(format!(
            "unported trigger reader {address:X} for {}",
            String::from_utf8_lossy(name)
        ))
    }

    /// One child of a trigger body (LoadBody 1405431C0 -> CTrigger::LoadKey
    /// 1405434F0). LoadBody calls LoadKey after every ReadKeyOperatorValue
    /// (1424AC5E0), whatever it read: a key read without an operator goes to
    /// its reader with the value slot as the last read left it, and a skip
    /// after RecordError tests the token 1424AC5E0 consumed in the
    /// operator's place ([`Field::skips_block`]). What LoadKey makes of the
    /// key ([`key_names::load_key`]) decides its reader:
    /// - a coded trigger's class reader ([`crate::trigger_keys::trigger_class`],
    ///   vtable slot 5): one that reads one value token
    ///   (CSimpleAssignTrigger::vf5 141395A40, ReadTriggerComparison
    ///   141394DF0) records a failed read (TryHandleOrRecordError 1424ACD40)
    ///   and skips nothing, so a block there is read as the enclosing
    ///   trigger's children and its `}` ends that trigger; one whose class
    ///   reads a body (LoadBody) reads its children as a trigger body, as
    ///   `if`, `else` and `else_if` do (a CIfTrigger); a class with a reader
    ///   of its own follows its native value reader ([`own`]);
    /// - a scope's body is the context trigger's LoadBody (14138C3A0);
    /// - a content count or scripted trigger reads one value token and skips
    ///   no block, as a coded one-value trigger does;
    /// - an invalid key is RecordError (1424ACF90), whose skip passes a block
    ///   the lexer stands on;
    /// - where the names cannot tell, a block those readers would read apart
    ///   leaves the rest of the text unsettled, refused by name.
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
                    "LoadKey (1405434F0) reads it as a scope's body, one value or an invalid key's skip, and which is not known here: {why}"
                ))
            }
            Load::Unknown(_) => Shape::One,
        }
    }
}

/// 1406B3CB0 resolves a phase field as it reads it: `no` first, then an exact
/// match against the phases registered so far (1406B35C0). The `no` test is
/// on the value's text, not its keyword ID: 1406B3D9C compares its two bytes
/// (and 1406B40DC for `display_phase`), so `NO` is looked up as a phase name.
fn phase_id(phases: &[Vec<u8>], value: &Value) -> i32 {
    // strlen of the value slot (+0xC8) first: the C string.
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

/// Parse one combat-tactics file without requiring a game installation, its
/// triggers against `names`.
pub fn parse(bytes: &[u8], names: &KeyNames) -> Database {
    // 1406B3690 constructs the lexer in mode 0 (CTextLexer::constructor 1424A63A0).
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
        // 1406B3690 compares the key's C string (strlen of the key slot);
        // the tactic (1406B0A70) is named by the same C string.
        let token = c_string(item.key.as_deref().unwrap_or_default()).to_vec();
        if token == b"phases" {
            // Each list entry, `=` included, is a phase name, stored by
            // ReadStringInto (1401426E0 -> 1424ABB50): to its first NUL, and
            // `Unreadable String` for a text holding a line feed.
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
                // The CAndTrigger and the weight's modifiers read triggers.
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

/// What CTrigger::LoadBody (1405431C0) and the weight's reader keep of a
/// tactic field; the errors they log go to `logged`.
fn keep(tactic: &str, field: &mut Item, logged: &mut Vec<String>) -> bool {
    let key = String::from_utf8_lossy(field.key.as_deref().unwrap_or_default()).into_owned();
    match (
        field.keyword().map(|keyword| keyword.folded()).as_deref(),
        &mut field.value,
    ) {
        // LoadBody checks for an opening brace: on any other value it
        // records an error (RecordError 1424ACF90) and consumes nothing.
        (Some(b"trigger"), Value::Scalar(text)) => {
            logged.push(format!(
                "tactic {tactic}'s {key} = {} is not a block: the game's trigger reader (1405431C0) logs an error (1424ACF90) and reads no condition from it",
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
                            "tactic {tactic}'s {key}'s {name} = {} is not a block: the game's modifier reader (1405431C0) logs an error (1424ACF90) and keeps a modifier with no condition, factor 1 and add 0",
                            String::from_utf8_lossy(text)
                        ));
                        entry.value = Value::Block(Vec::new());
                    }
                    (Some(b"modifier"), _) => {}
                    _ if is_one_of(keyword.as_deref(), &WEIGHT_VALUES) => {}
                    (_, value) => logged.push(format!(
                        "tactic {tactic}'s {key}'s {name} = {}: the weight's reader (140545850) logs an unknown command and reads on",
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

/// The [`EFFECTS`] as 1406B3CB0 reads each occurrence in order.
/// `attacker_movement_speed` goes through ReadFixed5 (1424ABB10 ->
/// 1424B0480): a value that does not scan is reported (1424ACEE0) and the
/// field keeps what it held. The other five go through
/// ParseTokenFixedPoint64 (1424AFF10 -> 1424B8600,
/// [`crate::fixed5::decimal_fixed5`]), which reads any text; an undefined
/// bare reference keeps its own text (1424AB540), read so too.
fn effects(tactic: &str, fields: &[Item], logged: &mut Vec<String>) -> [i64; 6] {
    let mut values = [0_i64; 6];
    for field in fields {
        let Some(slot) = field.keyword().and_then(|keyword| {
            let keyword = keyword.folded();
            EFFECTS.iter().position(|name| name.as_bytes() == keyword)
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
                    "tactic {tactic}'s {key} = {text} does not scan (1424B0480): the game reports it (1424ACEE0) and the field keeps its previous value"
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

/// The coded groups the viewer evaluates whose class reads its body through
/// CTrigger::LoadBody (vtable slot 5, 1405431C0): CAndTrigger (vtable
/// 1427024C0), COrTrigger (1427BF1E0), CNotTrigger (1427BF2A0), and
/// CCustomOverrideTooltipTrigger, whose slot 5 (141394F70) ends in LoadBody.
pub const BODY_GROUPS: [&str; 4] = ["and", "or", "not", "custom_trigger_tooltip"];

/// A group of [`BODY_GROUPS`], at any depth of a trigger body, whose value
/// opens no block: CTrigger::LoadKey (1405434F0) constructs the group,
/// appends it (140120090) and calls its LoadBody, which records an error
/// (1424ACF90) and reads no child, so the body holds the empty group. It
/// becomes that group here, logged; the group evaluators then read it
/// (empty AND 14053DFF0 and NOT 14053E070 true, OR 14053E0F0 false).
/// A custom tooltip's two localisation bindings are removed after their
/// readers have consumed them (and any file variables): 141395B30 handles
/// `tooltip` (0x92) and `not_tooltip` (0x43D0) before LoadKey, so neither
/// adds a condition, even when content has a trigger with the same name.
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
        let group = item
            .keyword()
            .is_some_and(|keyword| BODY_GROUPS.iter().any(|name| keyword.is(name)));
        match &mut item.value {
            Value::Scalar(text) if group => {
                logged.push(format!(
                    "{place}: {} = {} is not a block: the group's reader (1405431C0) logs an error (1424ACF90) and reads no child, and the game keeps the empty group (1405434F0)",
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
                    // 141395B30: localisation argument keys are not triggers.
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

/// A `phase` trigger anywhere inside a tactic's blocks (its trigger and its
/// weight's modifiers) is resolved as it is read (CCombatPhaseTrigger's
/// reader 140473850 -> 1406B35C0): `no` and a name not registered yet both
/// store -1, and CCombatPhaseTrigger::Evaluate (140472EB0) compares that ID
/// with the combat's phase. Its operand is replaced by the stored ID in
/// decimal, which is what the viewer evaluates. The trigger's name is a
/// keyword (found without case); its `no` is the operand's text, compared
/// exactly (14047389A: the two bytes `no`).
pub fn resolve_phase_triggers(phases: &[Vec<u8>], items: &mut [Item]) {
    for item in items {
        let is_phase = item.keyword_is("phase");
        match &mut item.value {
            Value::Scalar(operand) if is_phase => {
                // 140473850 takes the operand's strlen, then compares `no`.
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
