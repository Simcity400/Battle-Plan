use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_layers::ResolvedLayers;
use content_reader_parser::parser::{Item, Value, c_string};
use content_reader_script::{
    key_names::KeyNames,
    native,
    trigger_grammar::{Node, Shaper, effect_slot, empty_scalar_groups},
};
use std::io;

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
            match content_reader_parser::fixed5::script_fixed5(value) {
                Some(read) => values[slot] = read,
                None => logged.push(format!(
                    "tactic {tactic}'s {key} = {text} does not scan: the game reports it and the field keeps its previous value"
                )),
            }
        } else {
            values[slot] = content_reader_parser::fixed5::decimal_fixed5(value);
        }
        if !field.quoted() && value.starts_with(b"@") {
            logged.push(format!(
                "tactic {tactic}'s {key} names the variable {text}, which is not defined before it, so the game reads that text as the value"
            ));
        }
    }
    values
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
