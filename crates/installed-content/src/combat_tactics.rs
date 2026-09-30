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
//! own native reader ([`shape`]); where the text leaves the tree's shape the
//! reading follows the game's and is noted ([`Database::logged`]). Only a
//! block the port cannot settle (the key-validity gap, `native::Read::
//! unsettled`) leaves the tactic being read and every later one unproven
//! ([`Tactic::unproven`]). The lexer-assigned unlock IDs are campaign state
//! and are deliberately not inferred here.

use std::io;

use crate::{
    ResolvedLayers,
    parser::{
        Item, Value, c_string,
        native::{self, Field, Shape},
    },
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

/// The triggers the viewer evaluates whose class reads one value token
/// through CSimpleAssignTrigger::vf5 (141395A40): CIsAttackerTrigger,
/// CHasReservesTrigger and CFrontageFullTrigger (value reader 14138D3D0),
/// CCombatPhaseTrigger (140473850), CHasCombatModifierTrigger (140473990),
/// CHasUnitTypeTrigger (1404739B0), CHasTraitTrigger (14030A700),
/// CTagTrigger (1402F90D0), CIsFightingInTerrainTrigger (140473CB0) and
/// NDoctrines::CHasCombatTacticTrigger (1404A8120).
pub const ASSIGN_TRIGGERS: [&str; 10] = [
    "is_attacker",
    "has_reserves",
    "frontage_full",
    "phase",
    "has_combat_modifier",
    "has_unit_type",
    "has_trait",
    "tag",
    "is_fighting_in_terrain",
    "has_tactic",
];

/// The triggers the viewer evaluates whose class reads its operator and one
/// value token through ReadTriggerComparison (141394DF0): CSkillTrigger and
/// CSkillAdvantageTrigger (CIntTrigger::vf4 14138D4E0), CHardnessTrigger,
/// CProvinceVpTrigger and CHasArtilleryRatioTrigger (14138D630).
pub const COMPARISON_TRIGGERS: [&str; 5] = [
    "skill",
    "skill_advantage",
    "hardness",
    "province_vp",
    "has_artillery_ratio",
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
    /// A tactic's `base`, a CMeanTimeToHappen (140545850).
    Weight,
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
    /// CCustomOverrideTooltipTrigger::load_key (141395B30): `tooltip`
    /// and `not_tooltip` are bound localisations, every other key LoadKey.
    CustomTooltip,
    /// CBoundLocalization's block reader (1423912A0): `localization_key`
    /// reads one CFormattedLocalization value (142393E90), and every other
    /// key binds an argument, recursively when its value opens a block.
    BoundLocalization,
    /// `meta_trigger` / `meta_effect` (1403B34F0): `text` is copied raw
    /// (141408C80) and parsed later by a reader of its own.
    Meta,
}

fn is_one_of(keyword: Option<&[u8]>, names: &[&str]) -> bool {
    keyword.is_some_and(|keyword| names.iter().any(|name| name.as_bytes() == keyword))
}

/// The tactic file's field dispatch, by keyword id (register_authored_keys
/// 140082CB0): a bare key folds, a quoted one is token 0xF (1424A7A90).
pub(crate) fn shape(node: Node, field: &Field<'_>) -> Shape<Node> {
    let keyword = field.keyword();
    let keyword = keyword.as_deref();
    match node {
        // A text compare, not a keyword (1406B3875): a string list
        // (ReadStringList 1401426E0), which resolves references and defines
        // nothing. Every other key is a tactic, read through
        // ReadObjectFromParser whatever its value (CPersistent::Read
        // 1424A9730 does not check for `{`).
        Node::Top if c_string(&field.key.text) == b"phases" => Shape::List,
        Node::Top => Shape::Object(Node::Tactic),
        Node::Tactic => match keyword {
            // `base` 0x2C86: a CMeanTimeToHappen through ReadObjectFromParser.
            Some(b"base") => Shape::Object(Node::Weight),
            // `trigger` 0x2963: a CAndTrigger read by CTrigger::LoadBody,
            // which reads nothing of a value that opens no block.
            Some(b"trigger") => Shape::Body(Node::Trigger),
            _ if is_one_of(keyword, &EFFECTS) || is_one_of(keyword, &SINGLE_VALUES) => Shape::One,
            // Serializable_ReportUnexpectedToken (1424A9CE0).
            _ => Shape::Skip,
        },
        // 140545850: `modifier` 0x2965 a CMTTHModifier (LoadBody); any other
        // key only logged, nothing skipped.
        Node::Weight if keyword == Some(b"modifier") => Shape::Body(Node::Modifier),
        Node::Weight => Shape::One,
        // 140545820 dispatches the key slot whatever 1424AC5E0 read.
        Node::Modifier if matches!(keyword, Some(b"factor" | b"add")) => Shape::One,
        Node::CustomTooltip if matches!(keyword, Some(b"tooltip" | b"not_tooltip")) => {
            Shape::ObjectIfOpen(Node::BoundLocalization)
        }
        Node::BoundLocalization if keyword == Some(b"localization_key") => Shape::One,
        Node::BoundLocalization => Shape::ObjectIfOpen(Node::BoundLocalization),
        Node::Trigger | Node::Modifier | Node::Meta | Node::CustomTooltip => {
            trigger_field(node == Node::Meta, field)
        }
    }
}

/// One child of a trigger body (LoadBody 1405431C0 -> CTrigger::LoadKey
/// 1405434F0). LoadBody calls LoadKey after every ReadKeyOperatorValue
/// (1424AC5E0), whatever it read: a key read without an operator goes to
/// its reader with the value slot as the last read left it. The triggers
/// the viewer evaluates hand only the current value token to their class's
/// value reader, which records a failed read (TryHandleOrRecordError
/// 1424ACD40) and skips nothing: a block there is read as the enclosing
/// trigger's children and its `}` ends that trigger. A block LoadKey hands
/// to a reader the port knows is a LoadBody: a group of [`BODY_GROUPS`], or
/// a key [`names_a_scope`] (the context trigger's LoadBody, 14138C3A0). Any
/// other block is read by key (the key-validity gap, see [`Shape::ByKey`]).
fn trigger_field(meta: bool, field: &Field<'_>) -> Shape<Node> {
    let keyword = field.keyword();
    let keyword = keyword.as_deref();
    match keyword {
        Some(b"text") if meta => Shape::Skip,
        Some(b"meta_trigger" | b"meta_effect") if field.opens() => Shape::Object(Node::Meta),
        _ if is_one_of(keyword, &ASSIGN_TRIGGERS) || is_one_of(keyword, &COMPARISON_TRIGGERS) => {
            Shape::One
        }
        Some(b"custom_trigger_tooltip") if field.opens() => Shape::Body(Node::CustomTooltip),
        Some(name) if field.opens() && is_one_of(Some(name), &BODY_GROUPS) => {
            Shape::Body(Node::Trigger)
        }
        _ if field.opens() && names_a_scope(field) => Shape::Body(Node::Trigger),
        _ if field.opens() => Shape::ByKey,
        _ => Shape::One,
    }
}

/// Whether CTrigger::LoadKey (1405434F0) reads a block under this key as a
/// scope: no coded trigger holds its token, and the event target parse
/// (14146B860 -> 14146DB00, [`crate::scope`]) finds one by a step that needs
/// no content: a state id (step 1), `var:` (2), a scope keyword or a zero
/// number token (4), `event_target:` (5) or a keyword head (7). Steps 3 and
/// 6 (a tag, a character, a keyword `prefix:`) need content, so a key that
/// only they could name is left to the key-validity gap.
fn names_a_scope(field: &Field<'_>) -> bool {
    if field
        .keyword()
        .is_some_and(|keyword| crate::trigger_keys::registered_trigger(&keyword).is_some())
    {
        return false;
    }
    let quoted = field.key.kind == native::Kind::Quoted;
    let Ok(text) = std::str::from_utf8(c_string(&field.key.text)) else {
        return false;
    };
    let keyword = |text: &str| !quoted && crate::scope::Keyword::named(text.as_bytes()).is_some();
    crate::fixed5::decimal_i32(text.as_bytes()) != 0
        || text.starts_with("var:")
        || crate::parser::number_token(text.as_bytes(), quoted)
        || keyword(text)
        || text.starts_with("event_target:")
        || text
            .split_once('.')
            .is_some_and(|(head, _)| crate::scope::Keyword::named(head.as_bytes()).is_some())
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
    Ok(parse(&layers.read("common/combat_tactics.txt")?))
}

/// Parse one combat-tactics file without requiring a game installation.
pub fn parse(bytes: &[u8]) -> Database {
    // 1406B3690 constructs the lexer in mode 0 (CTextLexer::constructor 1424A63A0).
    let read = native::read(bytes, native::Mode::KeepMark, Node::Top, &mut shape);
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
    for item in items {
        let custom_tooltip = item.keyword_is("custom_trigger_tooltip");
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
                if custom_tooltip {
                    children.retain(|child| {
                        !child.keyword_is("tooltip") && !child.keyword_is("not_tooltip")
                    });
                }
                empty_scalar_groups(place, children, logged);
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
mod tests {
    use super::*;
    use crate::parser::Operator;

    /// 141395B30 dispatches both bindings to 142390E40. Its block reader
    /// 1423912A0 reads argument keys recursively, consuming definitions;
    /// argument names that also name triggers are never conditions.
    #[test]
    fn custom_tooltip_bindings_read_localisation_before_conditions() {
        let database = parse(
            b"t = { trigger = { custom_trigger_tooltip = { \
                tooltip = { localization_key = POSITIVE \
                    @threshold = 2 OR = { @amount = 0.5 localization_key = ARG } } \
                NOT_TOOLTIP = NEGATIVE \
                skill > @threshold \"not_tooltip\" = yes } } attacker = @amount } \
              later = { }",
        );
        assert!(database.unsupported.is_empty(), "{database:?}");
        assert_eq!(database.tactics.len(), 2, "{database:#?}");
        assert_eq!(database.tactics[1].token, b"later");
        assert_eq!(database.tactics[0].effects[2], 50_000);
        let trigger = database.tactics[0].fields[0].value.items().unwrap();
        let conditions = trigger[0].value.items().unwrap();
        assert_eq!(conditions.len(), 2, "{conditions:?}");
        assert!(conditions[0].keyword_is("skill"));
        assert_eq!(conditions[0].value.scalar(), Some(b"2".as_slice()));
        // A quoted key is not one of the binding keywords (1424A7A90).
        assert!(conditions[1].key_quoted);
        assert_eq!(
            conditions[1].key.as_deref(),
            Some(b"not_tooltip".as_slice())
        );
    }

    /// CFormattedLocalization::vf3 (142393E90) reads only the current
    /// value. A block on localization_key is not skipped: the next key is
    /// still the enclosing binding's, including a definition there.
    #[test]
    fn a_localisation_key_block_keeps_its_native_parse_boundary() {
        let database = parse(
            b"t = { trigger = { custom_trigger_tooltip = { \
                tooltip = { @threshold = 2 localization_key = { argument = KEY } \
                skill > @threshold } } attacker = 0.5 } later = { }",
        );
        assert!(database.unsupported.is_empty(), "{database:?}");
        assert_eq!(database.tactics.len(), 2, "{database:#?}");
        assert_eq!(database.tactics[0].effects[2], 50_000);
        let trigger = database.tactics[0].fields[0].value.items().unwrap();
        let conditions = trigger[0].value.items().unwrap();
        assert_eq!(conditions.len(), 1);
        assert!(conditions[0].keyword_is("skill"));
        assert_eq!(conditions[0].value.scalar(), Some(b"2".as_slice()));
        assert!(
            database
                .logged
                .iter()
                .any(|reason| reason.contains("localization_key"))
        );
    }

    /// LoadBody (1405431C0) hands LoadKey every key 1424AC5E0 reads, one
    /// read without an operator included, with the value slot as the last
    /// read left it: `OR {` takes `{` as its operator and keeps the slot's
    /// `{` (from `trigger = {`), so the group reads the tokens after it as
    /// its body. Likewise a modifier's `factor` goes to 140545820.
    #[test]
    fn a_key_without_an_operator_goes_to_its_trigger_reader() {
        let database = parse(b"t = { trigger = { Or { is_attacker = yes } } attacker = 0.1 }");
        let trigger = database.tactics[0].fields[0].value.items().unwrap();
        assert_eq!(trigger.len(), 1, "{trigger:?}");
        assert_eq!(trigger[0].key.as_deref(), Some(b"Or".as_slice()));
        let body = trigger[0].value.items().unwrap();
        assert_eq!(body[0].key.as_deref(), Some(b"is_attacker".as_slice()));
        assert_eq!(database.tactics[0].effects[2], 10_000);
        assert!(
            database
                .logged
                .iter()
                .any(|line| line.contains("key `Or` has no operator")),
            "{:?}",
            database.logged
        );
    }

    #[test]
    fn ids_follow_every_named_non_phase_entry_in_source_order() {
        let database = parse(
            b"phases={ opening breakthrough }\n\
              tactic_alpha={ active=yes }\n\
              tactic_alpha={ }\n\
              phases={ counterattack }\n\
              tactic_beta={ active=maybe }\n\
              Phases={}\n",
        );
        assert_eq!(
            database.phases,
            [
                b"opening".to_vec(),
                b"breakthrough".to_vec(),
                b"counterattack".to_vec()
            ]
        );
        assert_eq!(
            database
                .tactics
                .iter()
                .map(|tactic| (tactic.id, tactic.token.as_slice(), tactic.fields.len()))
                .collect::<Vec<_>>(),
            [
                (1, b"tactic_alpha".as_slice(), 1),
                (2, b"tactic_alpha".as_slice(), 0),
                (3, b"tactic_beta".as_slice(), 1),
                (4, b"Phases".as_slice(), 0),
            ]
        );
        assert!(database.unsupported.is_empty());
        assert!(database.logged.is_empty(), "{:?}", database.logged);
    }

    /// CPersistent::Read (1424A9730) reads a `name = value` tactic's fields
    /// from the tokens after it, up to the next `}`; a block on a field
    /// 1406B3CB0 reads as one value is read as `{`, its contents as the
    /// tactic's fields and its `}` ends the tactic. An unknown key's block
    /// is skipped (1424AD270).
    #[test]
    fn a_single_value_tactic_reads_the_rest_of_the_file() {
        let database = parse(
            b"phases = { opening }\n\
              first = { active = { is_attacker = yes } unknown = { x = 1 } }\n\
              second = value\n\
              phases = { later }\n\
              phase = opening\n\
              third = { active = no }\n",
        );
        assert_eq!(database.phases, [b"opening".to_vec()]);
        let keys = |tactic: &Tactic| -> Vec<Vec<u8>> {
            tactic
                .fields
                .iter()
                .map(|field| field.key.clone().unwrap())
                .collect()
        };
        // `first`'s `}` after `is_attacker = yes` ends it; `unknown` is a
        // tactic whose block is read as its fields, and the stray `}` a key.
        assert_eq!(
            keys(&database.tactics[0]),
            [b"active".to_vec(), b"is_attacker".to_vec()]
        );
        assert_eq!(database.tactics[1].token, b"unknown");
        assert!(
            database.logged[0]
                .contains("field `active` is a block where its reader takes one value"),
            "{:?}",
            database.logged
        );
        assert!(
            database
                .tactics
                .iter()
                .all(|tactic| tactic.unproven.is_none())
        );
    }

    /// `phases` and both `no` sentinels are compared by their text, exactly
    /// (1406B3875, 1406B3D9C, 14047389A); field and trigger keys are
    /// keywords and fold.
    #[test]
    fn phases_and_no_match_their_exact_text_while_keys_fold() {
        let database = parse(
            b"phases = { NO opening }\n\
              PHASES = { }\n\
              a = { Phase = NO Display_Phase = no \
                    TRIGGER = { PHASE = NO Phase = no } }\n",
        );
        assert_eq!(database.phases, [b"NO".to_vec(), b"opening".to_vec()]);
        assert_eq!(
            database
                .tactics
                .iter()
                .map(|tactic| tactic.token.as_slice())
                .collect::<Vec<_>>(),
            [b"PHASES".as_slice(), b"a".as_slice()]
        );
        let a = &database.tactics[1];
        assert_eq!((a.phase, a.display_phase), (0, NO_PHASE));
        let trigger = a.fields[2].value.items().unwrap();
        let operands: Vec<_> = trigger
            .iter()
            .map(|item| item.value.scalar().unwrap())
            .collect();
        assert_eq!(operands, [b"0".as_slice(), b"-1".as_slice()]);
        assert!(database.unsupported.is_empty());
    }

    /// ReadStringList (1401426E0) reads every token of `phases` to `}`,
    /// `=` included; a variable name that is not UTF-8 is a name like any
    /// other (a 0xFF byte would end the text: the lexer's `char` -1). A value
    /// without a key at the top level is a key to 1424AC5E0, which consumes
    /// the next token as its operator.
    #[test]
    fn unreadable_entries_are_named_and_mark_later_ids_unproven() {
        let database = parse(
            b"first = { }\n\
              @\xe9 = 2\n\
              phases = { opening key = value }\n\
              second = { attacker = @\xe9 }\n\
              loose\n\
              third = { }\n",
        );
        assert_eq!(
            database.phases,
            [
                b"opening".to_vec(),
                b"key".to_vec(),
                b"=".to_vec(),
                b"value".to_vec()
            ]
        );
        assert_eq!(database.tactics[1].effects[2], 200_000);
        // `loose` takes `third` as its operator and the stale value.
        assert_eq!(database.tactics[2].token, b"loose");
        assert!(
            database
                .logged
                .iter()
                .any(|line| line.contains("key `loose` has no operator")),
            "{:?}",
            database.logged
        );
        assert!(
            database
                .tactics
                .iter()
                .all(|tactic| tactic.unproven.is_none())
        );
    }

    /// A file variable takes no tactic ID and is no field; references read
    /// its value. A second definition of a taken name reads nothing more
    /// (1424AC830): its `=` goes to the next key.
    #[test]
    fn file_variables_take_no_id_and_resolve_in_fields() {
        let database = parse(
            b"@weight = 4\n\
              tactic_example = { base = { factor = @weight } @inner = 2 limit = @inner }\n\
              tactic_other = { base = { factor = @weight } }\n",
        );
        let ids: Vec<_> = database
            .tactics
            .iter()
            .map(|tactic| (tactic.id, tactic.token.as_slice()))
            .collect();
        assert_eq!(
            ids,
            [
                (1, b"tactic_example".as_slice()),
                (2, b"tactic_other".as_slice())
            ]
        );
        let example = &database.tactics[0].fields;
        assert_eq!(example.len(), 2);
        let factor = |fields: &[Item]| {
            fields[0].value.items().unwrap()[0]
                .value
                .scalar()
                .unwrap()
                .to_vec()
        };
        assert_eq!(factor(example), b"4");
        assert_eq!(example[1].value.scalar(), Some(b"2".as_slice()));
        assert_eq!(factor(&database.tactics[1].fields), b"4");
        let taken = parse(b"@x = 0.1 @x = 0.2 t = { attacker = @x }");
        assert!(
            taken.logged[0].contains("variable `@x` is already taken"),
            "{:?}",
            taken.logged
        );
        // A picture named by a variable reads its value.
        let pictures = parse(b"@pic = attack\ntactic_a = { picture = @pic }");
        let fields = &pictures.tactics[0].fields;
        assert_eq!(fields[0].value.scalar(), Some(b"attack".as_slice()));
    }

    /// A phase list entry naming a variable reads its value, so a tactic's
    /// `phase = @p` and the list agree on the phase's name.
    #[test]
    fn phase_lists_resolve_file_variables() {
        let database = parse(
            b"@p = breakthrough
              phases = { opening @p }
              tactic_example = { phase = @p }
",
        );
        assert_eq!(
            database.phases,
            [b"opening".to_vec(), b"breakthrough".to_vec()]
        );
        let fields = &database.tactics[0].fields;
        assert_eq!(fields[0].value.scalar(), Some(b"breakthrough".as_slice()));
    }

    /// Phase fields resolve against the phases read before the tactic, `no`
    /// is its own sentinel even when a phase is named `no`, and an absent or
    /// unmatched field is unknown.
    #[test]
    fn phase_fields_resolve_in_read_order() {
        let database = parse(
            b"phases = { opening no }\n\
              early = { phase = opening display_phase = later }\n\
              phases = { later }\n\
              late = { phase = no display_phase = later }\n\
              none = { }\n",
        );
        let ids: Vec<_> = database
            .tactics
            .iter()
            .map(|tactic| (tactic.phase, tactic.display_phase))
            .collect();
        assert_eq!(
            ids,
            [
                (0, UNKNOWN_PHASE),
                (NO_PHASE, 2),
                (UNKNOWN_PHASE, UNKNOWN_PHASE)
            ]
        );
    }

    /// Field keys match without case; a phase trigger stores the ID of a
    /// phase registered before it, and -1 for `no` or a later phase.
    #[test]
    fn phase_keys_fold_and_triggers_resolve_in_read_order() {
        let database = parse(
            b"phases = { opening }\n\
              early = { PHASE = opening DISPLAY_PHASE = opening \
                        trigger = { phase = later OR = { Phase = opening } } \
                        base = { modifier = { phase = no } } }\n\
              phases = { later }\n\
              late = { trigger = { phase = later } }\n",
        );
        assert_eq!(
            (database.tactics[0].phase, database.tactics[0].display_phase),
            (0, 0)
        );
        let operand = |tactic: usize, path: &[&str]| {
            let mut items = database.tactics[tactic].fields.as_slice();
            let (last, blocks) = path.split_last().unwrap();
            for key in blocks {
                items = items
                    .iter()
                    .find(|item| item.keyword_is(key))
                    .unwrap()
                    .value
                    .items()
                    .unwrap();
            }
            items
                .iter()
                .find(|item| item.keyword_is(last))
                .unwrap()
                .value
                .scalar()
                .unwrap()
                .to_vec()
        };
        assert_eq!(operand(0, &["trigger", "phase"]), b"-1");
        assert_eq!(operand(0, &["trigger", "OR", "Phase"]), b"0");
        assert_eq!(operand(0, &["base", "modifier", "phase"]), b"-1");
        assert_eq!(operand(1, &["trigger", "phase"]), b"1");
    }

    #[test]
    fn raw_name_bytes_and_nested_fields_survive_parsing() {
        let database = parse(b"tactic_\xe9={ allowed={ scripted=value } active=no }");
        assert_eq!(database.tactics[0].id, 1);
        assert_eq!(database.tactics[0].token, b"tactic_\xe9");
        let fields = &database.tactics[0].fields;
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].key.as_deref(), Some(b"allowed".as_slice()));
    }

    /// The six effects as 1406B3CB0 reads them: keys fold, the last one
    /// wins, `attacker_movement_speed` scans `%lli` (`010` is eight) and
    /// the others read any text as 1424B8600 does ('e' is a digit to it, six
    /// places are taken unscaled). A repeated name is a second tactic with
    /// its own values; a quoted key is no keyword.
    #[test]
    fn effects_follow_each_fields_native_reader() {
        let database = parse(
            br#"@bonus = 0.25
                tactic_a = { ATTACKER = @bonus Combat_Width = 0.1 attacker_movement_speed = 010 }
                tactic_b = { defender = @missing }
                tactic_a = { attacker = 0.5 attacker = 0.75 }
                tactic_c = { defender_org_damage_modifier = 1.5e3 active = yes defender = 0.000001 }
                quoted = { defender = "@bonus" "attacker" = 0.5 }
                PHASES = { assault seize }"#,
        );
        let effects = |id: usize| database.tactics[id - 1].effects;
        assert_eq!(effects(1), [10_000, 800_000, 25_000, 0, 0, 0]);
        assert_eq!(effects(3)[2], 75_000);
        // An undefined reference is its own text, which reads zero.
        assert_eq!(effects(2)[4], 0);
        assert_eq!((effects(4)[5], effects(4)[4]), (203_300, 1));
        // A quoted "@bonus" is text, never a reference (1424AB540).
        assert_eq!(effects(5), [0; 6]);
        assert!(
            database.logged.iter().any(|line| line
                == "tactic tactic_b's defender names the variable @missing, which is not defined before it, so the game reads that text as the value"),
            "{:?}",
            database.logged
        );
        // `PHASES` is tactic 6; its bare names are keys without operators.
        assert_eq!(database.tactics[5].token, b"PHASES");
        assert!(database.tactics.iter().all(|t| t.unproven.is_none()));
    }

    /// ReadFixed5 (1424ABB10) reports a value that does not scan and leaves
    /// the field as it was, so an earlier assignment stands.
    #[test]
    fn an_unscannable_movement_value_keeps_the_previous_one() {
        let database =
            parse(b"t = { attacker_movement_speed = 0.5 ATTACKER_MOVEMENT_SPEED = fast }");
        assert_eq!(database.tactics[0].effects[1], 50_000);
        assert_eq!(
            database.logged,
            [
                "tactic t's ATTACKER_MOVEMENT_SPEED = fast does not scan (1424B0480): the game reports it (1424ACEE0) and the field keeps its previous value"
            ]
        );
    }

    /// The file's one reader and its variables: a chain resolves when
    /// defined and names keep their case (1424AC830); a definition in a
    /// tactic body holds for the rest of the file; `phases` is a string list
    /// (1401426E0) that defines nothing; `@(1+1)` is the reference `@` and
    /// then `(`, `1+1` and `)` read as keys, the last consuming the tactic's
    /// `}`.
    #[test]
    fn tactic_variables_follow_the_game_reader() {
        let database = parse(
            br#"@w = 0.1
                @W = 0.2
                @chain = @w
                phases = { @unregistered = 1 }
                a = { combat_width = @chain attacker = @W }
                b = { @inner = 0.3 attacker = @inner }
                c = { attacker = @inner }
                unregistered = { defender = @unregistered }
                expression = { attacker = @(1+1) }
                after = { attacker = 0.1 }"#,
        );
        let effect = |id: usize, slot: usize| database.tactics[id - 1].effects[slot];
        assert_eq!((effect(1, 0), effect(1, 2)), (10_000, 20_000));
        assert_eq!(effect(2, 2), 30_000);
        assert_eq!(effect(3, 2), 30_000);
        assert_eq!(effect(4, 4), 0);
        assert_eq!(
            database.phases,
            [b"@unregistered".to_vec(), b"=".to_vec(), b"1".to_vec()]
        );
        // `after` is read as a field of `expression`.
        assert_eq!(database.tactics.len(), 5);
        assert!(database.tactics.iter().all(|t| t.unproven.is_none()));
    }

    /// The tactic's `trigger` (a CAndTrigger, 1405431C0) and `base` (a
    /// CMeanTimeToHappen, 140545850) are read by key on the file's reader,
    /// so their definitions register in document order and their values
    /// resolve; an unknown field is skipped (1424ACF90 -> 1424AD270) and
    /// defines nothing. A comparison keeps its operator (+0x78).
    #[test]
    fn definitions_inside_trigger_and_base_register() {
        let database = parse(
            b"t = { attacker = @late trigger = { OR = { @late = 0.1 always = yes } phase = no } \
              base = { factor = 1 modifier = { @weight = 0.2 factor = 2 } } unknown = { @skipped = 1 x = 1 } } \
              u = { attacker = @late defender = @weight combat_width = @skipped \
                    trigger = { skill > @late skill_advantage<@weight } }",
        );
        assert!(
            database.unsupported.is_empty(),
            "{:?}",
            database.unsupported
        );
        assert_eq!(database.tactics[0].effects[2], 0);
        let u = &database.tactics[1];
        assert_eq!(
            (u.effects[2], u.effects[4], u.effects[0]),
            (10_000, 20_000, 0)
        );
        let trigger = u.fields[3].value.items().unwrap();
        let compared: Vec<_> = trigger
            .iter()
            .map(|item| {
                (
                    item.key.clone().unwrap(),
                    item.operator,
                    item.value.scalar().unwrap().to_vec(),
                )
            })
            .collect();
        assert_eq!(
            compared,
            [
                (b"skill".to_vec(), Operator::Greater, b"0.1".to_vec()),
                (b"skill_advantage".to_vec(), Operator::Less, b"0.2".to_vec()),
            ]
        );
    }

    /// LoadBody (1405431C0) hands `x`, read without an operator (it
    /// consumes `@y`), the stale `{`: which reader `x` is decides what it
    /// reads, and the port does not know it (the key-validity gap), so the
    /// tactic being read is unproven by name and no later tactic is read.
    #[test]
    fn unsettled_tactic_variables_mark_the_rest_of_the_file_unproven() {
        let database =
            parse(b"s = { attacker = 0.1 } t = { trigger = { x @y = 1 } } u = { attacker = @y }");
        assert_eq!(database.tactics.len(), 2);
        assert_eq!(database.tactics[0].unproven, None);
        let reason = database.tactics[1].unproven.clone().unwrap();
        assert!(
            reason.contains(
                "key `x` has no operator and its reader is handed the previous value `{`"
            ),
            "{reason}"
        );
        assert_eq!(database.unsupported, [reason]);
    }

    /// Each nested reader consumes as its native reader does: the weight's
    /// one-value fields and unknown keys never skip a block (140545850), so
    /// a block there ends the weight early; a block under a trigger the
    /// viewer evaluates is read as the enclosing trigger's children
    /// (141395A40 / 141394DF0 -> 1424ACD40, no skip) and its `}` ends that
    /// trigger. The reading goes on as the game's does and is noted; later
    /// tactics are read.
    #[test]
    fn nested_readers_follow_their_own_consumption() {
        for text in [
            &b"s = { } a = { base = { factor = { factor = 2 } } } b = { }"[..],
            b"s = { } a = { BASE = { Mystery = { x = 1 } } } b = { }",
            b"s = { } a = { base = { Days = { 1 } } } b = { }",
            b"s = { } a = { trigger = { IS_ATTACKER = { skill > 2 } } } b = { }",
            b"s = { } a = { base = { modifier = { OR = { skill > { 1 } } } } } b = { }",
        ] {
            let database = parse(text);
            let shown = String::from_utf8_lossy(text);
            assert!(
                database
                    .logged
                    .iter()
                    .any(|line| line.contains("is a block where its reader takes one value")),
                "{shown}: {:?}",
                database.logged
            );
            assert!(
                database.tactics.iter().all(|t| t.unproven.is_none()),
                "{shown}"
            );
            assert_eq!(database.tactics[0].token, b"s", "{shown}");
        }
        // Under IS_ATTACKER the block's `skill > 2` is the trigger's own.
        let database = parse(b"a = { trigger = { IS_ATTACKER = { skill > 2 } } attacker = 0.1 }");
        let trigger = database.tactics[0].fields[0].value.items().unwrap();
        assert_eq!(
            trigger
                .iter()
                .map(|item| item.key.clone().unwrap())
                .collect::<Vec<_>>(),
            [b"IS_ATTACKER".to_vec(), b"skill".to_vec()]
        );
    }

    /// A weight modifier's `factor` and `add` go to the CMTTHModifier's own
    /// key reader (140545820), a scoped variable taking one value token
    /// (140539440 -> 140539B50) with no block skipped: a block there is read
    /// as `{`, its contents as the modifier's conditions, and its `}` ends
    /// the modifier, so what follows is the weight's again.
    #[test]
    fn a_modifier_factor_block_ends_the_modifier() {
        let database = parse(
            b"a = { base = { modifier = { FACTOR = { is_attacker = yes } skill > 2 } attacker = 0.1 }\n\
              b = { base = { modifier = { add = { has_reserves = yes } } }\n",
        );
        assert!(
            database
                .logged
                .iter()
                .any(|line| line
                    .contains("field `FACTOR` is a block where its reader takes one value")),
            "{:?}",
            database.logged
        );
        let base = database.tactics[0].fields[0].value.items().unwrap();
        let keys = |items: &[Item]| -> Vec<Vec<u8>> {
            items.iter().map(|item| item.key.clone().unwrap()).collect()
        };
        assert_eq!(keys(base), [b"modifier".to_vec(), b"skill".to_vec()]);
        assert_eq!(
            keys(&database.tactics[0].fields),
            [b"base".to_vec(), b"attacker".to_vec()]
        );
        let modifier = base[0].value.items().unwrap();
        assert_eq!(
            keys(modifier),
            [b"FACTOR".to_vec(), b"is_attacker".to_vec()]
        );
        assert_eq!(modifier[0].value.scalar(), Some(b"{".as_slice()));
        // Later tactics are read as the game reads them.
        assert_eq!(database.tactics.len(), 2, "{:?}", database.tactics);
        assert!(database.tactics.iter().all(|t| t.unproven.is_none()));
    }

    /// A scalar `trigger` is logged by CTrigger::LoadBody (1405431C0 ->
    /// 1424ACF90), which consumes nothing: its tactic keeps its other
    /// fields. A scalar `modifier` is a modifier without a condition, and
    /// the weight logs an unknown command. A quoted key is no keyword
    /// (token 0xF): its block is skipped and defines nothing.
    #[test]
    fn a_scalar_trigger_is_a_local_error() {
        let database = parse(
            b"a = { trigger = { always = yes } Trigger = yes attacker = 0.1 \
                    base = { modifier = yes factor = 2 note = 1 } } \
              b = { attacker = 0.2 \"trigger\" = { @hidden = 0.3 } defender = @hidden }",
        );
        assert!(
            database.unsupported.is_empty(),
            "{:?}",
            database.unsupported
        );
        assert_eq!(
            database.logged,
            [
                "combat_tactics.txt line 1: field `Trigger` is `yes` where its trigger reads a block: \"Non assign trigger is not enclosed in {}\" is reported (1405431C0) and nothing is read; read as the game reads it",
                "combat_tactics.txt line 1: field `modifier` is `yes` where its trigger reads a block: \"Non assign trigger is not enclosed in {}\" is reported (1405431C0) and nothing is read; read as the game reads it",
                "tactic a's Trigger = yes is not a block: the game's trigger reader (1405431C0) logs an error (1424ACF90) and reads no condition from it",
                "tactic a's base's modifier = yes is not a block: the game's modifier reader (1405431C0) logs an error (1424ACF90) and keeps a modifier with no condition, factor 1 and add 0",
                "tactic a's base's note = 1: the weight's reader (140545850) logs an unknown command and reads on",
                "tactic b's defender names the variable @hidden, which is not defined before it, so the game reads that text as the value",
            ]
        );
        let a = &database.tactics[0];
        let keys: Vec<_> = a
            .fields
            .iter()
            .map(|field| field.key.clone().unwrap())
            .collect();
        assert_eq!(
            keys,
            [b"trigger".to_vec(), b"attacker".to_vec(), b"base".to_vec()]
        );
        let base = a.fields[2].value.items().unwrap();
        assert_eq!(base[0].value, Value::Block(Vec::new()));
        assert_eq!(a.effects[2], 10_000);
        let b = &database.tactics[1];
        assert_eq!((b.effects[2], b.effects[4]), (20_000, 0));
    }

    /// 1424AC5E0 consumes a definition before handing the next key to the
    /// trigger's reader (1405431C0), at any depth: none is a condition. A
    /// comparison's operand, spaced or compact, is resolved as a phase
    /// (140473850 -> 1406B35C0) and through the file's variables.
    #[test]
    fn consumed_definitions_are_no_conditions_and_comparisons_resolve() {
        let database = parse(
            b"phases = { 99 opening }\n\
              t = { trigger = { @x = yes is_attacker = @x OR = { @y = 1 phase>99 } } \
                    base = { modifier = { @z = 2 factor = @z PHASE < opening } } }",
        );
        assert!(
            database.unsupported.is_empty(),
            "{:?}",
            database.unsupported
        );
        let fields = &database.tactics[0].fields;
        let shown = |items: &[Item]| -> Vec<String> {
            items
                .iter()
                .map(|item| {
                    format!(
                        "{}{}{}",
                        String::from_utf8_lossy(item.key.as_deref().unwrap()),
                        String::from_utf8_lossy(item.operator.text()),
                        match &item.value {
                            Value::Scalar(text) => String::from_utf8_lossy(text).into_owned(),
                            _ => "{}".into(),
                        }
                    )
                })
                .collect()
        };
        let trigger = fields[0].value.items().unwrap();
        assert_eq!(shown(trigger), ["is_attacker=yes", "OR={}"]);
        assert_eq!(shown(trigger[1].value.items().unwrap()), ["phase>0"]);
        let modifier = fields[1].value.items().unwrap()[0].value.items().unwrap();
        assert_eq!(shown(modifier), ["factor=2", "PHASE<1"]);
    }

    /// `meta_trigger`'s `text` is copied raw (141408C80): nothing in it
    /// defines; a block holding values without keys is read as a list.
    #[test]
    fn meta_text_lists_and_keys_without_operators_follow_the_trigger_reader() {
        let database = parse(
            b"t = { trigger = { meta_trigger = { text = { @hidden = 1 } x = y } \
                    any_unit = { infantry } } attacker = @hidden }",
        );
        let trigger = database.tactics[0].fields[0].value.items().unwrap();
        assert_eq!(
            trigger[0].value.items().unwrap()[0].value,
            Value::Block(Vec::new())
        );
        assert_eq!(
            trigger[1].value.items().unwrap()[0].value.scalar(),
            Some(b"infantry".as_slice())
        );
        assert_eq!(database.tactics[0].effects[2], 0);
    }

    /// A group given a scalar is the empty group LoadKey keeps (1405434F0,
    /// its LoadBody reading no child), at any depth of a tactic trigger or a
    /// weight modifier; keys fold.
    #[test]
    fn scalar_groups_are_the_empty_groups_the_game_keeps() {
        let database = parse(
            b"t = { trigger = { AND = yes OR = { or = no } Not = yes CUSTOM_TRIGGER_TOOLTIP = x \"and\" = yes } \
                    base = { modifier = { or = no } } }",
        );
        let trigger = database.tactics[0].fields[0].value.items().unwrap();
        assert_eq!(trigger[0].value, Value::Block(Vec::new()));
        assert_eq!(
            trigger[1].value.items().unwrap()[0].value,
            Value::Block(Vec::new())
        );
        assert_eq!(trigger[2].value, Value::Block(Vec::new()));
        assert_eq!(trigger[3].value, Value::Block(Vec::new()));
        // A quoted key is token 0xF, no group.
        assert_eq!(trigger[4].value.scalar(), Some(b"yes".as_slice()));
        let modifier = database.tactics[0].fields[1].value.items().unwrap()[0]
            .value
            .items()
            .unwrap();
        assert_eq!(modifier[0].value, Value::Block(Vec::new()));
        assert_eq!(
            database
                .logged
                .iter()
                .filter(|line| line.contains("the game keeps the empty group"))
                .count(),
            5,
            "{:?}",
            database.logged
        );
    }

    /// ReadStringList stores each phase through ReadStringInto (1401426E0
    /// -> 1424ABB50): the text to its first NUL, and a text holding a line
    /// feed as `Unreadable String` (reported, 1424ACEE0).
    #[test]
    fn phase_names_are_stored_as_read_string_into_stores_them() {
        let database = parse(b"phases = { open\x00ing \"line\nfeed\" }");
        assert_eq!(
            database.phases,
            [b"open".to_vec(), crate::parser::UNREADABLE_STRING.to_vec()]
        );
    }

    /// The tactic file's reader dispatches through `Field::keyword`
    /// (1424A7A90: a quoted key is token 0xF, a bare one folds); its one
    /// text compare, `phases` (1406B3875), is an exact byte match.
    #[test]
    fn the_tactic_reader_asks_the_keyword_helper() {
        const RAW: [&str; 6] = [
            "key.as_deref() ==",
            "key.as_deref() !=",
            "eq_ignore_ascii_case(",
            "to_ascii_lowercase(",
            ".get(b\"",
            "get_all(",
        ];
        let code = include_str!("combat_tactics.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        let raw: Vec<_> = code
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.starts_with("//") && RAW.iter().any(|pattern| line.contains(pattern))
            })
            .collect();
        assert!(raw.is_empty(), "raw key comparisons: {raw:#?}");
    }
}
