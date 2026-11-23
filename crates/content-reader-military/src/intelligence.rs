use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_parser::{Diagnostic, DiagnosticKind, fields::invalid, parser::Item};
use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_script::{key_names::KeyNames, modifiers, native::{self, Field, Shape}};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Operation {
    pub token: String,
    pub name: String,
    pub description: String,
    pub sprite: String,
    pub map_icon: String,
}

#[derive(Default)]
pub struct Intelligence {
    pub operations: BTreeMap<String, Operation>,
    pub operation_refusals: Vec<String>,
    pub notes: Vec<Diagnostic>,
    pub warnings: Vec<String>,
}

impl Intelligence {
    pub fn operation(&self, token: &str) -> Result<Option<&Operation>, String> {
        if self.operation_refusals.is_empty() {
            Ok(self.operations.get(&token.to_ascii_lowercase()))
        } else {
            Err(format!(
                "operation {token}: {}",
                self.operation_refusals.join("; ")
            ))
        }
    }
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Operation,
    Phases,
    Mtth,
    Targets,
    Resources,
    Civilian,
}

fn string(item: &Item) -> io::Result<String> {
    item.value.read_str().map(str::to_owned).map_err(|why| {
        invalid(format!(
            "{}: {why} (count: 1)",
            String::from_utf8_lossy(item.key.as_deref().unwrap_or_default())
        ))
    })
}

fn token(item: &Item) -> io::Result<String> {
    item.key_str()
        .map(str::to_owned)
        .map_err(|why| invalid(format!("intelligence definition name: {why} (count: 1)")))
}

fn targets(field: &Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        Some(b"targets") => Shape::List,
        Some(b"target_array") => Shape::Unsettled(format!(
            "script targets target_array at line {}: the variable-array reader is not ported here",
            field.key.line
        )),
        Some(
            b"state" | b"country" | b"targets_dynamic" | b"target_non_existing" | b"state_target",
        ) => Shape::One,
        Some(b"target_trigger" | b"target_root_trigger") => Shape::Trigger,
        _ => Shape::Skip,
    }
}

fn operation_field(field: &Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        Some(
            b"name"
            | b"desc"
            | b"icon"
            | b"map_icon"
            | b"difficulty"
            | b"priority"
            | b"days"
            | b"experience"
            | b"operatives"
            | b"network_strength"
            | b"cost_multiplier"
            | b"time_multiplier"
            | b"danger_level"
            | b"will_lead_to_war_with"
            | b"is_staged_coup"
            | b"is_captured_cipher"
            | b"prevent_captured_operative_to_die"
            | b"return_on_complete"
            | b"risk_chance"
            | b"outcome_chance"
            | b"outcome_extra_chance"
            | b"target_type"
            | b"scale_cost_independent_of_target",
        ) => Shape::One,
        Some(b"allowed" | b"available" | b"visible" | b"requirements" | b"optional") => {
            Shape::Trigger
        }
        Some(b"ai_will_do" | b"target_weight") => Shape::Object(Node::Mtth),
        Some(b"phases") => Shape::Object(Node::Phases),
        Some(b"equipment") => Shape::Object(Node::Resources),
        Some(
            b"required_tokens" | b"awarded_tokens" | b"cost_modifiers" | b"risk_modifiers"
            | b"outcome_modifiers",
        ) => Shape::List,
        Some(
            b"on_start" | b"outcome_potential" | b"outcome_execute" | b"outcome_extra_execute",
        ) => Shape::EffectIn(0x80),
        Some(b"operation_target" | b"selection_target") => Shape::Object(Node::Targets),
        Some(b"selection_target_state") => Shape::Trigger,
        _ => Shape::Skip,
    }
}

pub fn parse_operations(
    input: &[u8],
    names: &KeyNames,
    earlier: &BTreeSet<String>,
) -> io::Result<(Vec<Operation>, Vec<String>)> {
    parse_operations_with_notes(input, names, earlier, &mut Vec::new())
}

fn parse_operations_with_notes(
    input: &[u8],
    names: &KeyNames,
    earlier: &BTreeSet<String>,
    notes: &mut Vec<Diagnostic>,
) -> io::Result<(Vec<Operation>, Vec<String>)> {
    let mut seen = earlier.clone();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File => {
                let token = String::from_utf8_lossy(field.key.c_str()).to_ascii_lowercase();
                if seen.insert(token) {
                    Shape::Object(Node::Operation)
                } else {
                    Shape::Skip
                }
            }
            Node::Operation => operation_field(field),
            Node::Phases => Shape::Object(Node::Mtth),
            Node::Mtth => native::mean_time_to_happen(field),
            Node::Targets => targets(field),
            Node::Resources if field.keyword_is("civilian_factories") => {
                Shape::Object(Node::Civilian)
            }
            Node::Resources => Shape::One,
            Node::Civilian => match field.keyword().as_deref() {
                Some(b"amount" | b"days") => Shape::One,
                _ => Shape::Skip,
            },
        },
    );
    notes.extend(
        read.misreads
            .iter()
            .map(|note| (note.kind, note.to_string())),
    );
    if let Some(cause) = read.unsettled {
        let reason = format!("operation definitions: {cause}");
        notes.push((DiagnosticKind::Unsettled, reason.clone()));
        return Err(invalid(reason));
    }
    let mut operations = Vec::new();
    let mut projected = earlier.clone();
    for item in read.items {
        let Some(fields) = item.value.items() else {
            continue;
        };
        let token = token(&item).map_err(|why| note_refusal(notes, why))?;
        if !projected.insert(token.to_ascii_lowercase()) {
            continue;
        }
        let mut operation = Operation {
            token,
            ..Operation::default()
        };
        for field in fields {
            if field.keyword_is("name") {
                operation.name = string(field).map_err(|why| note_refusal(notes, why))?;
            } else if field.keyword_is("desc") {
                operation.description = string(field).map_err(|why| note_refusal(notes, why))?;
            } else if field.keyword_is("icon") {
                operation.sprite = string(field).map_err(|why| note_refusal(notes, why))?;
            } else if field.keyword_is("map_icon") {
                operation.map_icon = string(field).map_err(|why| note_refusal(notes, why))?;
            }
        }
        operations.push(operation);
    }
    Ok((
        operations,
        read.misreads
            .into_iter()
            .map(|why| why.to_string())
            .collect(),
    ))
}

fn note_refusal(notes: &mut Vec<Diagnostic>, why: io::Error) -> io::Error {
    notes.push((DiagnosticKind::Refused, why.to_string()));
    why
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Intelligence> {
    let names = layers.key_names()?;
    let mut result = Intelligence::default();
    let mut earlier = BTreeSet::new();
    for path in script_files::paths(layers, "common/operations/", Some(".txt"))? {
        let mut notes = Vec::new();
        match parse_operations_with_notes(&path.read(layers)?, &names, &earlier, &mut notes) {
            Ok((operations, warnings)) => {
                result
                    .warnings
                    .extend(warnings.into_iter().map(|why| format!("{path}: {why}")));
                for operation in operations {
                    let key = operation.token.to_ascii_lowercase();
                    earlier.insert(key.clone());
                    result.operations.entry(key).or_insert(operation);
                }
            }
            Err(why) => result.operation_refusals.push(format!("{path}: {why}")),
        }
        result.notes.extend(
            notes
                .into_iter()
                .map(|(kind, reason)| (kind, format!("{path}: {reason}"))),
        );
    }
    result
        .warnings
        .extend(result.operation_refusals.iter().cloned());
    Ok(result)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationToken {
    pub token: String,
    pub name: String,
    pub icon: String,
}

#[derive(Default)]
pub struct OperationTokens {
    pub definitions: BTreeMap<String, OperationToken>,
    pub refusals: Vec<String>,
    pub warnings: Vec<String>,
}

impl OperationTokens {
    pub fn token(&self, token: &str) -> Result<Option<&OperationToken>, String> {
        if self.refusals.is_empty() {
            Ok(self.definitions.get(&token.to_ascii_lowercase()))
        } else {
            Err(format!(
                "operation token {token}: {}",
                self.refusals.join("; ")
            ))
        }
    }
}

#[derive(Clone, Copy)]
enum TokenNode {
    File,
    Token,
}

pub fn parse_operation_tokens(
    input: &[u8],
    names: &KeyNames,
    earlier: &BTreeSet<String>,
) -> io::Result<(Vec<OperationToken>, Vec<String>)> {
    let mut seen = earlier.clone();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(TokenNode::File),
        Some(names),
        &mut |node, field| match node {
            TokenNode::File => {
                if seen.insert(String::from_utf8_lossy(field.key.c_str()).to_ascii_lowercase()) {
                    Shape::Object(TokenNode::Token)
                } else {
                    Shape::Skip
                }
            }
            TokenNode::Token => match field.keyword().as_deref() {
                Some(
                    b"name" | b"desc" | b"icon" | b"text_icon" | b"intel_source" | b"intel_gain",
                ) => Shape::One,
                Some(b"targeted_modifier") if field.skips_block() => Shape::Unsettled(format!(
                    "operation token targeted_modifier at line {}: block consumption depends on the static-modifier lookup (count: 1)",
                    field.key.line
                )),
                Some(b"targeted_modifier") => Shape::One,
                _ => Shape::Skip,
            },
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("operation token definitions: {cause}")));
    }
    let mut projected = earlier.clone();
    let mut result = Vec::new();
    for item in read.items {
        let Some(fields) = item.value.items() else {
            continue;
        };
        let token = token(&item)?;
        if !projected.insert(token.to_ascii_lowercase()) {
            continue;
        }
        let mut definition = OperationToken {
            token,
            ..OperationToken::default()
        };
        for field in fields {
            if field.keyword_is("name") {
                definition.name = string(field)?;
            } else if field.keyword_is("text_icon") {
                definition.icon = string(field)?;
            }
        }
        result.push(definition);
    }
    Ok((
        result,
        read.misreads
            .into_iter()
            .map(|why| why.to_string())
            .collect(),
    ))
}

pub fn read_operation_tokens(layers: &ResolvedLayers) -> io::Result<OperationTokens> {
    let names = layers.key_names()?;
    let mut result = OperationTokens::default();
    let mut earlier = BTreeSet::new();
    for path in script_files::paths(layers, "common/operation_tokens/", Some(".txt"))? {
        match parse_operation_tokens(&path.read(layers)?, &names, &earlier) {
            Ok((definitions, warnings)) => {
                result
                    .warnings
                    .extend(warnings.into_iter().map(|why| format!("{path}: {why}")));
                for definition in definitions {
                    let key = definition.token.to_ascii_lowercase();
                    earlier.insert(key.clone());
                    result.definitions.entry(key).or_insert(definition);
                }
            }
            Err(why) => result.refusals.push(format!("{path}: {why}")),
        }
    }
    result.warnings.extend(result.refusals.iter().cloned());
    Ok(result)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationPhase {
    pub token: String,
    pub icon: String,
    pub map_icon: String,
}

impl OperationPhase {
    pub fn map_sprite(&self) -> &str {
        if self.map_icon.is_empty() {
            &self.icon
        } else {
            &self.map_icon
        }
    }
}

#[derive(Default)]
pub struct OperationPhases {
    pub definitions: BTreeMap<String, OperationPhase>,
    pub refusals: Vec<String>,
    pub warnings: Vec<String>,
}

impl OperationPhases {
    pub fn phase(&self, token: &str) -> Result<Option<&OperationPhase>, String> {
        if self.refusals.is_empty() {
            Ok(self.definitions.get(&token.to_ascii_lowercase()))
        } else {
            Err(format!(
                "operation phase {token}: {}",
                self.refusals.join("; ")
            ))
        }
    }
}

#[derive(Clone, Copy)]
enum PhaseNode {
    File,
    Phase,
}

pub fn parse_operation_phases(
    input: &[u8],
    names: &KeyNames,
    earlier: &BTreeSet<String>,
) -> io::Result<(Vec<OperationPhase>, Vec<String>)> {
    let mut seen = earlier.clone();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(PhaseNode::File),
        Some(names),
        &mut |node, field| match node {
            PhaseNode::File => {
                if seen.insert(String::from_utf8_lossy(field.key.c_str()).to_ascii_lowercase()) {
                    Shape::Object(PhaseNode::Phase)
                } else {
                    Shape::Skip
                }
            }
            PhaseNode::Phase => match field.keyword().as_deref() {
                Some(b"icon" | b"map_icon") => Shape::One,
                _ => Shape::SkipIfOpen,
            },
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("operation phase definitions: {cause}")));
    }
    let mut projected = earlier.clone();
    let mut result = Vec::new();
    for item in read.items {
        let Some(fields) = item.value.items() else {
            continue;
        };
        let token = token(&item)?;
        if !projected.insert(token.to_ascii_lowercase()) {
            continue;
        }
        let mut definition = OperationPhase {
            token,
            ..OperationPhase::default()
        };
        for field in fields {
            if field.keyword_is("icon") {
                definition.icon = string(field)?;
            } else if field.keyword_is("map_icon") {
                definition.map_icon = string(field)?;
            }
        }
        result.push(definition);
    }
    Ok((
        result,
        read.misreads
            .into_iter()
            .map(|why| why.to_string())
            .collect(),
    ))
}

pub fn read_operation_phases(layers: &ResolvedLayers) -> io::Result<OperationPhases> {
    let names = layers.key_names()?;
    let mut result = OperationPhases::default();
    let mut earlier = BTreeSet::new();
    for path in script_files::paths(layers, "common/operation_phases/", Some(".txt"))? {
        match parse_operation_phases(&path.read(layers)?, &names, &earlier) {
            Ok((definitions, warnings)) => {
                result
                    .warnings
                    .extend(warnings.into_iter().map(|why| format!("{path}: {why}")));
                for definition in definitions {
                    let key = definition.token.to_ascii_lowercase();
                    earlier.insert(key.clone());
                    result.definitions.entry(key).or_insert(definition);
                }
            }
            Err(why) => result.refusals.push(format!("{path}: {why}")),
        }
    }
    result.warnings.extend(result.refusals.iter().cloned());
    Ok(result)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgencyUpgrade {
    pub token: String,
    pub name: String,
    pub branch: String,
    pub sprite: String,
}

#[derive(Default)]
pub struct AgencyUpgrades {
    pub definitions: BTreeMap<String, AgencyUpgrade>,
    pub branch_definitions: BTreeMap<(String, String), AgencyUpgrade>,
    pub refusals: Vec<String>,
    pub warnings: Vec<String>,
}

impl AgencyUpgrades {
    pub fn upgrade(&self, token: &str) -> Result<Option<&AgencyUpgrade>, String> {
        if self.refusals.is_empty() {
            Ok(self.definitions.get(&token.to_ascii_lowercase()))
        } else {
            Err(format!(
                "agency upgrade {token}: {}",
                self.refusals.join("; ")
            ))
        }
    }
    pub fn branch_upgrade(
        &self,
        branch: &str,
        token: &str,
    ) -> Result<Option<&AgencyUpgrade>, String> {
        self.upgrade(token)?;
        Ok(self
            .branch_definitions
            .get(&(branch.to_ascii_lowercase(), token.to_ascii_lowercase())))
    }

    pub fn global_upgrade(&self, token: &str) -> Result<Option<&AgencyUpgrade>, String> {
        self.upgrade(token)
    }

    pub fn upgrade_branches(&self, token: &str) -> Result<Vec<&AgencyUpgrade>, String> {
        self.upgrade(token)?;
        let token = token.to_ascii_lowercase();
        Ok(self
            .branch_definitions
            .iter()
            .filter(|((_, key), _)| key == &token)
            .map(|(_, definition)| definition)
            .collect())
    }
}

#[derive(Clone, Copy)]
enum UpgradeNode {
    File,
    Branch,
    Upgrade,
    Level,
    Mtth,
    Modifier(modifiers::Node),
}

type UpgradeBranches = Vec<(String, Vec<AgencyUpgrade>)>;

fn parse_agency_upgrades(
    input: &[u8],
    names: &KeyNames,
) -> io::Result<(UpgradeBranches, Vec<String>)> {
    let mut undecided = Vec::new();
    let registry = &|key: &[u8]| modifiers::registered(key, Some(names));
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(UpgradeNode::File),
        Some(names),
        &mut |node, field| match node {
            UpgradeNode::File => Shape::Object(UpgradeNode::Branch),
            UpgradeNode::Branch => Shape::Object(UpgradeNode::Upgrade),
            UpgradeNode::Upgrade => match field.keyword().as_deref() {
                Some(b"frame" | b"picture" | b"sound") => Shape::One,
                Some(b"available" | b"visible") => Shape::Trigger,
                Some(b"ai_will_do") => Shape::Object(UpgradeNode::Mtth),
                Some(b"modifiers_during_progress") => {
                    Shape::Object(UpgradeNode::Modifier(modifiers::Node::Modifier))
                }
                Some(b"level") => Shape::Object(UpgradeNode::Level),
                _ => Shape::Skip,
            },
            UpgradeNode::Level => match field.keyword().as_deref() {
                Some(b"modifier") => {
                    Shape::Object(UpgradeNode::Modifier(modifiers::Node::Modifier))
                }
                Some(b"complete_effect") => Shape::EffectIn(4),
                _ => Shape::Skip,
            },
            UpgradeNode::Mtth => native::mean_time_to_happen(field),
            UpgradeNode::Modifier(node) => {
                modifiers::shape(node, field, registry, &mut undecided).map(UpgradeNode::Modifier)
            }
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!(
            "agency upgrade definitions: {cause} (count: 1)"
        )));
    }
    if !undecided.is_empty() {
        return Err(invalid(format!(
            "agency upgrade definitions: {} (count: {})",
            undecided
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
            undecided.len()
        )));
    }
    let mut result = Vec::new();
    for item in read.items {
        let Some(fields) = item.value.items() else {
            continue;
        };
        let branch = token(&item)?;
        let mut upgrades = Vec::new();
        for field in fields {
            let token = token(field)?;
            let mut upgrade = AgencyUpgrade {
                name: token.clone(),
                token,
                branch: branch.clone(),
                ..Default::default()
            };
            for member in field.value.items().unwrap_or_default() {
                if member.keyword_is("picture") {
                    upgrade.sprite = string(member)?;
                }
            }
            upgrades.push(upgrade);
        }
        result.push((branch, upgrades));
    }
    Ok((
        result,
        read.misreads
            .into_iter()
            .map(|why| why.to_string())
            .collect(),
    ))
}

pub fn read_agency_upgrades(layers: &ResolvedLayers) -> io::Result<AgencyUpgrades> {
    let names = layers.key_names()?;
    let mut result = AgencyUpgrades::default();
    let mut owners = BTreeMap::new();
    for path in script_files::paths(layers, "common/intelligence_agency_upgrades/", Some(".txt"))? {
        match parse_agency_upgrades(&path.read(layers)?, &names) {
            Ok((branches, warnings)) => {
                result
                    .warnings
                    .extend(warnings.into_iter().map(|why| format!("{path}: {why}")));
                for (branch, upgrades) in branches {
                    let branch = branch.to_ascii_lowercase();
                    for upgrade in upgrades {
                        let key = upgrade.token.to_ascii_lowercase();
                        let owner = owners.entry(key.clone()).or_insert_with(|| branch.clone());
                        result
                            .branch_definitions
                            .insert((branch.clone(), key.clone()), upgrade.clone());
                        if owner == &branch {
                            result.definitions.insert(key, upgrade);
                        }
                    }
                }
            }
            Err(why) => result.refusals.push(format!("{path}: {why}")),
        }
    }
    result.warnings.extend(result.refusals.iter().cloned());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn operation_map_icon_is_separate_and_keeps_first_definition() {
        content_reader_script::test_game::enter();
        let (definitions, _) = parse_operations(
            b"op = { name = NAME icon = CARD map_icon = MAP } OP = { name = WRONG map_icon = WRONG } other = { name = OTHER }",
            &KeyNames::default(),
            &BTreeSet::new(),
        ).unwrap();
        assert_eq!(definitions.len(), 2);
        assert_eq!(definitions[0].name, "NAME");
        assert_eq!(definitions[0].sprite, "CARD");
        assert_eq!(definitions[0].map_icon, "MAP");
        assert_eq!(definitions[1].name, "OTHER");
        assert!(definitions[1].map_icon.is_empty());
    }

    #[test]
    fn operation_phase_map_sprite_falls_back_to_its_icon_and_keeps_the_first_definition() {
        content_reader_script::test_game::enter();
        let (definitions, _) = parse_operation_phases(
            b"steal = { icon = GFX_steal map_icon = GFX_steal_map outcome = { a = b } }
              STEAL = { icon = GFX_wrong }
              infiltrate = { name = NAME icon = GFX_infiltrate }",
            &KeyNames::default(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(definitions.len(), 2);
        assert_eq!(definitions[0].map_sprite(), "GFX_steal_map");
        assert_eq!(definitions[1].token, "infiltrate");
        assert_eq!(definitions[1].map_sprite(), "GFX_infiltrate");
        let phases = OperationPhases {
            definitions: definitions
                .into_iter()
                .map(|phase| (phase.token.to_ascii_lowercase(), phase))
                .collect(),
            ..OperationPhases::default()
        };
        assert_eq!(
            phases.phase("INFILTRATE").unwrap().unwrap().icon,
            "GFX_infiltrate"
        );
        let refused = OperationPhases {
            refusals: vec!["common/operation_phases/a.txt: unsettled".into()],
            ..OperationPhases::default()
        };
        assert!(refused.phase("steal").is_err());
    }

    #[test]
    fn intelligence_operation_token_name_text_icon_and_duplicate_skip() {
        content_reader_script::test_game::enter();
        let (definitions, warnings) = parse_operation_tokens(
            br#"
            @name=MOD_NAME
            custom={name=@name icon=wrong text_icon=MOD_ICON desc=MOD_DESC intel_source=army intel_gain=10}
            CUSTOM={name=LATER @leaked=wrong}
            second={name=@leaked text_icon=SECOND}
            "#,
            &KeyNames::default(),
            &BTreeSet::new(),
        ).unwrap();
        assert_eq!(
            definitions,
            [
                OperationToken {
                    token: "custom".into(),
                    name: "MOD_NAME".into(),
                    icon: "MOD_ICON".into()
                },
                OperationToken {
                    token: "second".into(),
                    name: "@leaked".into(),
                    icon: "SECOND".into()
                }
            ]
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        let (definitions, _) = parse_operation_tokens(
            b"CUSTOM={name=ignored} empty={} } later={name=ignored}",
            &KeyNames::default(),
            &BTreeSet::from(["custom".into()]),
        )
        .unwrap();
        assert_eq!(
            definitions,
            [OperationToken {
                token: "empty".into(),
                ..OperationToken::default()
            }]
        );
        let why = parse_operation_tokens(
            b"custom={targeted_modifier={name=unknown}}",
            &KeyNames::default(),
            &BTreeSet::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(why.contains("targeted_modifier") && why.contains("count: 1"));
    }

    #[test]
    fn intelligence_agency_upgrade_readers_and_identifier_name() {
        content_reader_script::test_game::enter();
        let (branches, warnings) = parse_agency_upgrades(
            br#"
            branch={
                custom={name=ignored picture=custom_art frame=custom_frame sound=custom_sound
                    available={always=yes} visible={always=yes}
                    ai_will_do={factor=1 modifier={factor=2 always=yes}}
                    modifiers_during_progress={political_power_factor=0.1}
                    level={modifier={political_power_factor=0.2} complete_effect={log=debug}}
                }
                CUSTOM={picture=other}
            }
        "#,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            branches,
            [(
                "branch".into(),
                vec![
                    AgencyUpgrade {
                        token: "custom".into(),
                        name: "custom".into(),
                        branch: "branch".into(),
                        sprite: "custom_art".into(),
                    },
                    AgencyUpgrade {
                        token: "CUSTOM".into(),
                        name: "CUSTOM".into(),
                        branch: "branch".into(),
                        sprite: "other".into(),
                    }
                ]
            )]
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn intelligence_token_upgrade_layers_first_global_pointer_and_replace_path() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "hoi4_intelligence_domains_{}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let base = root.join("base");
        let over = root.join("over");
        for folder in [&base, &over] {
            std::fs::create_dir_all(folder.join("common/operation_tokens")).unwrap();
            std::fs::create_dir_all(folder.join("common/intelligence_agency_upgrades")).unwrap();
        }
        std::fs::write(
            base.join("common/operation_tokens/a.txt"),
            b"custom={name=BASE text_icon=BASE_ICON}",
        )
        .unwrap();
        std::fs::write(
            over.join("common/operation_tokens/b.txt"),
            b"custom={name=ignored text_icon=ignored} second={name=MOD text_icon=MOD_ICON}",
        )
        .unwrap();
        std::fs::write(
            base.join("common/intelligence_agency_upgrades/a.txt"),
            b"branch={custom={}} discarded={removed={}}",
        )
        .unwrap();
        std::fs::write(
            over.join("common/intelligence_agency_upgrades/b.txt"),
            b"other={CUSTOM={}} BRANCH={Custom={}} ",
        )
        .unwrap();
        let stack =
            ResolvedLayers::from_test_common_folders(&[(&base, &[]), (&over, &[])]).unwrap();
        assert_eq!(
            read_operation_tokens(&stack)
                .unwrap()
                .token("CUSTOM")
                .unwrap()
                .unwrap()
                .name,
            "BASE"
        );
        assert_eq!(
            read_agency_upgrades(&stack)
                .unwrap()
                .upgrade("CUSTOM")
                .unwrap()
                .unwrap()
                .name,
            "Custom"
        );
        let replaced = ResolvedLayers::from_test_common_folders(&[
            (&base, &[]),
            (
                &over,
                &[
                    "common/operation_tokens",
                    "common/intelligence_agency_upgrades",
                ],
            ),
        ])
        .unwrap();
        assert_eq!(
            read_operation_tokens(&replaced)
                .unwrap()
                .token("custom")
                .unwrap()
                .unwrap()
                .name,
            "ignored"
        );
        let upgrades = read_agency_upgrades(&replaced).unwrap();
        assert_eq!(upgrades.upgrade("custom").unwrap().unwrap().name, "CUSTOM");
        assert!(upgrades.upgrade("removed").unwrap().is_none());
        std::fs::write(
            over.join("common/operation_tokens/c.txt"),
            b"unknown={targeted_modifier={unknown=yes}}",
        )
        .unwrap();
        let refused =
            ResolvedLayers::from_test_common_folders(&[(&base, &[]), (&over, &[])]).unwrap();
        let why = read_operation_tokens(&refused)
            .unwrap()
            .token("custom")
            .unwrap_err();
        assert!(why.contains("c.txt") && why.contains("count: 1"));
        let cleanup_root = std::fs::canonicalize(root).unwrap();
        let cleanup_parent = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert_eq!(cleanup_root.parent(), Some(cleanup_parent.as_path()));
        std::fs::remove_dir_all(cleanup_root).unwrap();
    }

    #[test]
    fn intelligence_operation_names_art_variables_and_duplicate_skip() {
        content_reader_script::test_game::enter();
        let (operations, warnings) = parse_operations(
            br#"
            @sprite = mod_art
            mod_operation = { name = mod_name icon = @sprite desc = mod_desc
                equipment = { civilian_factories = { amount = 3 days = 9 } mod_equipment = 20 }
                phases = { mod_phase = { base = 1 modifier = { factor = 2 always = yes } } }
                operation_target = { targets = { mod_country } target_trigger = { always = yes } }
            }
            mod_operation = { icon = wrong @leaked = wrong }
            another = { icon = @leaked }
        "#,
            &KeyNames::default(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0].name, "mod_name");
        assert_eq!(operations[0].sprite, "mod_art");
        assert_eq!(operations[1].sprite, "@leaked");
        let (skipped, _) = parse_operations(
            b"MOD_OPERATION = { icon = later }",
            &KeyNames::default(),
            &BTreeSet::from(["mod_operation".into()]),
        )
        .unwrap();
        assert!(skipped.is_empty());
        let (empty, _) = parse_operations(
            b"empty = {} EMPTY = { icon = later }",
            &KeyNames::default(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(empty.len(), 1);
        assert!(empty[0].sprite.is_empty());
        let (closed, _) = parse_operations(
            b"one={icon=one} } later={icon=later}",
            &KeyNames::default(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(closed.len(), 1);
    }

    #[test]
    fn intelligence_operation_named_unreached_array_refusal() {
        content_reader_script::test_game::enter();
        let why = parse_operations(
            b"custom={operation_target={target_array=a}}",
            &KeyNames::default(),
            &BTreeSet::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(why.contains("target_array") && why.contains("line 1"));
    }

    #[test]
    fn intelligence_operation_notes_keep_deferred_kind_and_public_parser_output() {
        content_reader_script::test_game::enter();
        let input = b"custom = { on_start = { free_operative = { operative = custom_name } } }";
        let mut notes = Vec::new();
        let parsed =
            parse_operations_with_notes(input, &KeyNames::default(), &BTreeSet::new(), &mut notes)
                .unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].0, DiagnosticKind::Unsupported);
        assert!(notes[0].1.contains("Deferred effect `free_operative`"));
        assert_eq!(parsed.1, vec![notes[0].1.clone()]);
        assert_eq!(
            parse_operations(input, &KeyNames::default(), &BTreeSet::new()).unwrap(),
            parsed
        );
    }

    #[test]
    fn intelligence_reused_layers_keep_replace_paths_and_fresh_names() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "hoi4_intelligence_layers_{}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let base = root.join("base");
        let over = root.join("over");
        for folder in [&base, &over] {
            std::fs::create_dir_all(folder.join("common/scripted_triggers")).unwrap();
            std::fs::create_dir_all(folder.join("common/operations")).unwrap();
        }
        std::fs::write(
            base.join("common/scripted_triggers/base.txt"),
            b"base_only = { always = yes }",
        )
        .unwrap();
        std::fs::write(
            over.join("common/scripted_triggers/mod.txt"),
            b"mod_only = { always = yes }",
        )
        .unwrap();
        std::fs::write(
            base.join("common/operations/main.txt"),
            b"custom = { name = BASE_NAME icon = base_art }",
        )
        .unwrap();
        std::fs::write(
            over.join("common/operations/main.txt"),
            b"custom = { name = MOD_NAME icon = mod_art available = { mod_only = yes } }",
        )
        .unwrap();
        let vanilla = ResolvedLayers::from_test_common_folders(&[(&base, &[])]).unwrap();
        assert!(
            vanilla
                .key_names()
                .unwrap()
                .scripted
                .contains(b"base_only".as_slice())
        );
        let mut roster = vanilla.layers.clone();
        let additional =
            ResolvedLayers::from_test_common_folders(&[(&over, &["common/scripted_triggers"])])
                .unwrap();
        for mut layer in additional.layers {
            layer.id = format!("test{}", roster.len());
            roster.push(layer);
        }
        let stack = ResolvedLayers::from_test_layers(roster).unwrap();
        let names = stack.key_names().unwrap();
        assert!(names.scripted.contains(b"mod_only".as_slice()));
        assert!(!names.scripted.contains(b"base_only".as_slice()));
        assert_eq!(read(&stack).unwrap().operations["custom"].name, "MOD_NAME");
        let cleanup_root = std::fs::canonicalize(root).unwrap();
        let cleanup_parent = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert_eq!(cleanup_root.parent(), Some(cleanup_parent.as_path()));
        std::fs::remove_dir_all(cleanup_root).unwrap();
    }

}
