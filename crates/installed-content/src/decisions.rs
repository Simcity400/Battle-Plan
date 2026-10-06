//! Decision names and highlight scopes (140725B40, 140727980, 140720D40).

use crate::{
    ResolvedLayers,
    key_names::KeyNames,
    modifiers,
    parser::{
        self, Item,
        native::{self, Field, Shape},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decision {
    pub id: Vec<u8>,
    pub name: Vec<u8>,
    pub source: String,
    /// CDecision+0x140: authored identifier's folded FNV-1a (140716650).
    pub seed_component: i32,
    pub mission: bool,
    pub fixed_random_seed: bool,
    /// CDecision+0x9E0 can exist with an empty target body (140727980).
    pub has_state_targets: bool,
    pub highlight_states: Vec<Item>,
    pub highlight_conditions: Vec<Item>,
    pub target_root_trigger: Vec<Item>,
    pub target_trigger: Vec<Item>,
    pub target_arrays: Vec<Vec<u8>>,
    pub target_states: Vec<Vec<u8>>,
    pub refused: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Database {
    pub entries: BTreeMap<Vec<u8>, Decision>,
    pub notes: Vec<String>,
    unread: Option<String>,
}

impl Database {
    pub fn decision(&self, token: &[u8]) -> Result<Option<&Decision>, String> {
        if let Some(why) = &self.unread {
            return Err(why.clone());
        }
        // 14038BF60 builds title metadata only after a matching active instance.
        Ok(self
            .entries
            .get(&parser::c_string(token).to_ascii_lowercase()))
    }
}

/// Hashed string constructor 140BBA1B0; 140716650 copies its +0x20 to +0x140.
pub fn seed_component(id: &[u8]) -> i32 {
    parser::c_string(id)
        .iter()
        .fold(0x811C9DC5_u32, |hash, byte| {
            (hash ^ u32::from(byte.to_ascii_lowercase())).wrapping_mul(0x01000193)
        }) as i32
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Category,
    Decision,
    Highlight,
    Targets,
    Mtth,
    Modifier(modifiers::Node),
}

/// ReadScriptTargetsMember 140C1D730; these are scoped operands, not effects.
fn targets(field: &Field<'_>) -> Option<Shape<Node>> {
    Some(match field.keyword().as_deref()? {
        b"targets" => Shape::List,
        b"target_array"
        | b"state"
        | b"country"
        | b"targets_dynamic"
        | b"target_non_existing"
        | b"state_target" => Shape::One,
        b"target_trigger" | b"target_root_trigger" => Shape::Trigger,
        _ => return None,
    })
}

fn append(
    database: &mut Database,
    bytes: &[u8],
    source: &str,
    categories: &BTreeSet<Vec<u8>>,
    names: &KeyNames,
) {
    let mut positions = Vec::new();
    let mut undecided = Vec::new();
    let registry = &|name: &[u8]| modifiers::registered(name, Some(names));
    let read = native::read_in_effect_scope(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        crate::scope::ScopeKind::Country.bit(),
        &mut |node, field| match node {
            // 1407198A0 reads each category and decision, even for a missing category.
            Node::File => Shape::Object(Node::Category),
            Node::Category => {
                positions.push(field.key.line);
                Shape::Object(Node::Decision)
            }
            Node::Decision => {
                if let Some(shape) = targets(field) {
                    return shape;
                }
                match field.keyword().as_deref() {
                    Some(
                        b"allowed"
                        | b"available"
                        | b"visible"
                        | b"activation"
                        | b"remove_trigger"
                        | b"cancel_trigger"
                        | b"custom_cost_trigger",
                    ) => Shape::Trigger,
                    Some(
                        b"remove_effect" | b"complete_effect" | b"timeout_effect"
                        | b"cancel_effect",
                    ) => Shape::Effect,
                    Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
                    Some(b"targeted_modifier") => {
                        Shape::Object(Node::Modifier(modifiers::Node::Targeted))
                    }
                    Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                    Some(b"highlight_states") => Shape::Object(Node::Highlight),
                    // 140301220 appends one CScopedVariable, whose vf3 takes the held token.
                    Some(
                        b"war_with_on_remove" | b"war_with_on_complete" | b"war_with_on_timeout",
                    ) => Shape::One,
                    Some(
                        b"name"
                        | b"priority"
                        | b"icon"
                        | b"cost"
                        | b"alert_days"
                        | b"fire_only_once"
                        | b"desc"
                        | b"cosmetic_tag"
                        | b"days_remove"
                        | b"days_re_enable"
                        | b"days_mission_timeout"
                        | b"days_decision_timeout"
                        | b"is_good"
                        | b"fixed_random_seed"
                        | b"selectable_mission"
                        | b"custom_cost_text"
                        | b"war_with_target_on_remove"
                        | b"war_with_target_on_complete"
                        | b"war_with_target_on_timeout"
                        | b"cancel_if_not_visible"
                        | b"on_map_mode"
                        | b"power_balance"
                        | b"cosmetic_ideology",
                    ) => Shape::One,
                    _ => Shape::Skip,
                }
            }
            // CHighlightStates::vf4 140727980; its ordinary trigger is state-scoped.
            Node::Highlight => match field.keyword().as_deref() {
                Some(b"highlight_state_targets") => Shape::Object(Node::Targets),
                Some(b"highlight_states_trigger") => Shape::Trigger,
                Some(b"highlight_provinces") => Shape::RawList,
                Some(
                    b"highlight_only_provinces"
                    | b"highlight_color_before_active"
                    | b"highlight_color_while_active",
                ) => Shape::One,
                // This reader ignores unknown tokens without RecordError or skipping.
                _ => Shape::One,
            },
            Node::Targets => targets(field).unwrap_or(Shape::Skip),
            Node::Mtth => native::mean_time_to_happen(field),
            Node::Modifier(node) => {
                modifiers::shape(node, field, registry, &mut undecided).map(Node::Modifier)
            }
        },
    );
    database
        .notes
        .extend(read.misreads.iter().map(|note| format!("{source}: {note}")));
    database
        .notes
        .extend(undecided.iter().map(|note| format!("{source}: {note}")));
    if let Some(why) = undecided.first() {
        database.unread = Some(format!("{source}: unread decisions: {why}"));
    }
    if let Some(why) = &read.unsettled {
        database.unread = Some(format!("{source}: unread decisions: {why}"));
    }
    let mut ordinal = 0;
    for category in &read.items {
        let category_key =
            parser::c_string(category.key.as_deref().unwrap_or_default()).to_ascii_lowercase();
        for item in category.value.items().unwrap_or_default() {
            let id = parser::c_string(item.key.as_deref().unwrap_or_default()).to_vec();
            let source = format!("{source}:{}", positions.get(ordinal).copied().unwrap_or(1));
            ordinal += 1;
            if !categories.contains(&category_key) {
                database.notes.push(format!(
                    "{source}: decision {} ignored: category {} is not registered (1407198A0)",
                    String::from_utf8_lossy(&id),
                    String::from_utf8_lossy(&category_key)
                ));
                continue;
            }
            let mut decision = Decision {
                name: id.clone(),
                seed_component: seed_component(&id),
                id: id.clone(),
                source,
                ..Default::default()
            };
            let fields = item.value.items().unwrap_or_default();
            decision.fixed_random_seed =
                parser::read_bool_field(fields, "fixed_random_seed", false);
            for field in fields {
                if field.keyword_is("name") {
                    let name = field.value.read_string().unwrap_or_default();
                    // Empty alternate key keeps the authored identifier (140720B40).
                    decision.name = if name.is_empty() {
                        id.clone()
                    } else {
                        name.to_vec()
                    };
                }
                if field.keyword_is("days_mission_timeout")
                    || field.keyword_is("days_decision_timeout")
                {
                    decision.mission = true;
                }
                if field.keyword_is("highlight_states") {
                    decision
                        .highlight_states
                        .extend_from_slice(field.value.items().unwrap_or_default());
                }
            }
            for field in &decision.highlight_states {
                if field.keyword_is("highlight_states_trigger") {
                    decision
                        .highlight_conditions
                        .extend_from_slice(field.value.items().unwrap_or_default());
                }
                if field.keyword_is("highlight_state_targets") {
                    decision.has_state_targets = true;
                    for target in field.value.items().unwrap_or_default() {
                        match target.keyword().map(|key| key.folded()).as_deref() {
                            Some(b"state") => decision
                                .target_states
                                .push(target.value.scalar().unwrap_or_default().to_vec()),
                            Some(b"targets") => {
                                decision.target_states.extend(
                                    target.value.items().unwrap_or_default().iter().map(|item| {
                                        item.value.scalar().unwrap_or_default().to_vec()
                                    }),
                                )
                            }
                            Some(b"target_array") => decision
                                .target_arrays
                                .push(target.value.scalar().unwrap_or_default().to_vec()),
                            Some(b"target_trigger") => decision
                                .target_trigger
                                .extend_from_slice(target.value.items().unwrap_or_default()),
                            Some(b"target_root_trigger") => decision
                                .target_root_trigger
                                .extend_from_slice(target.value.items().unwrap_or_default()),
                            Some(
                                b"country"
                                | b"targets_dynamic"
                                | b"target_non_existing"
                                | b"state_target",
                            ) => {
                                let why = decision.refused.get_or_insert_with(String::new);
                                if !why.is_empty() {
                                    why.push_str("; ");
                                }
                                why.push_str(&format!(
                                    "{}: highlight script-target {} is not ported (140C1D730)",
                                    decision.source,
                                    String::from_utf8_lossy(
                                        target.key.as_deref().unwrap_or_default()
                                    )
                                ));
                            }
                            _ => {}
                        }
                    }
                }
            }
            // Native 140719320 retains the first matching decision definition.
            let key = id.to_ascii_lowercase();
            if let std::collections::btree_map::Entry::Vacant(entry) = database.entries.entry(key) {
                entry.insert(decision);
            } else {
                database.notes.push(format!(
                    "{}: duplicate decision {} ignored (140719320)",
                    decision.source,
                    String::from_utf8_lossy(&id)
                ));
            }
        }
    }
}

/// 140723270 loads category names before decision bodies; both directories use mode 0.
pub fn read(layers: &ResolvedLayers) -> io::Result<Database> {
    let names = layers.key_names()?;
    let mut categories = BTreeSet::new();
    let mut database = Database::default();
    for path in layers.directory_files("common/decisions/categories", ".txt")? {
        let read = native::read(
            &layers.read(&path)?,
            native::Mode::KeepMark,
            native::Top::File(()),
            None,
            &mut |(), _| Shape::SkipIfOpen,
        );
        database
            .notes
            .extend(read.misreads.iter().map(|note| format!("{path}: {note}")));
        if let Some(why) = read.unsettled {
            database.unread = Some(format!("{path}: unread decision categories: {why}"));
        }
        categories.extend(read.items.iter().map(|item| {
            parser::c_string(item.key.as_deref().unwrap_or_default()).to_ascii_lowercase()
        }));
    }
    for path in layers.directory_files("common/decisions", ".txt")? {
        append(
            &mut database,
            &layers.read(&path)?,
            &path,
            &categories,
            &names,
        );
    }
    Ok(database)
}

#[cfg(test)]
mod tests;
