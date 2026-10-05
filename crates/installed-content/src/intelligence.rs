//! Operation presentation through the recorded content stack.
//! Native readers consume every field; only names and authored sprites are projected.

use crate::{
    ResolvedLayers,
    fields::invalid,
    key_names::KeyNames,
    parser::{
        Item,
        native::{self, Field, Shape},
    },
    script_files,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Operation {
    pub token: String,
    pub name: String,
    pub sprite: String,
}

#[derive(Default)]
pub struct Intelligence {
    pub operations: BTreeMap<String, Operation>,
    /// An uncertain stream can register an earlier token, so no lookup guesses.
    pub operation_refusals: Vec<String>,
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

/// CPersistentScriptTargets::vf4 140727A20 delegates to 140C1D730.
fn targets(field: &Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        Some(b"targets") => Shape::List,
        Some(b"target_array") => Shape::Unsettled(format!(
            "script targets target_array at line {}: the variable-array reader 140C1D730 is not ported here",
            field.key.line
        )),
        Some(
            b"state" | b"country" | b"targets_dynamic" | b"target_non_existing" | b"state_target",
        ) => Shape::One,
        Some(b"target_trigger" | b"target_root_trigger") => Shape::Trigger,
        _ => Shape::Skip,
    }
}

/// COperation::vf4 140A70140 and nested readers 140A6C360/140A6C630,
/// 140A6BA80/140A6BF30. Unknown keywords take ReportUnexpectedToken's skip.
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

/// 1401674C0 refuses a token already read in this file or an earlier file,
/// with RecordError's skip. The first registered definition therefore wins.
pub fn parse_operations(
    input: &[u8],
    names: &KeyNames,
    earlier: &BTreeSet<String>,
) -> io::Result<(Vec<Operation>, Vec<String>)> {
    let mut seen = earlier.clone();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        // The reloadable operation loop 1401674C0 stops at token 4 or 0x13.
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
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("operation definitions: {cause}")));
    }
    let mut operations = Vec::new();
    let mut projected = earlier.clone();
    for item in read.items {
        let Some(fields) = item.value.items() else {
            continue;
        };
        let token = token(&item)?;
        // RecordError's skipped duplicate retains a tree placeholder, not a
        // registered operation (1401674C0).
        if !projected.insert(token.to_ascii_lowercase()) {
            continue;
        }
        let mut operation = Operation {
            token,
            ..Operation::default()
        };
        for field in fields {
            if field.keyword_is("name") {
                operation.name = string(field)?;
            } else if field.keyword_is("icon") {
                operation.sprite = string(field)?;
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

pub fn read(layers: &ResolvedLayers) -> io::Result<Intelligence> {
    let names = layers.key_names()?;
    let mut result = Intelligence::default();
    let mut earlier = BTreeSet::new();
    for path in script_files::paths(layers, "common/operations/", Some(".txt")) {
        match parse_operations(&layers.read(&path)?, &names, &earlier) {
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
    }
    result
        .warnings
        .extend(result.operation_refusals.iter().cloned());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intelligence_operation_names_art_variables_and_native_duplicate_skip() {
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
    fn intelligence_reused_layers_keep_replace_paths_and_fresh_names() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_intelligence_layers_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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

    /// Read-only common/ syntax audit; every KeyNames dependency is below common/.
    /// Artwork, localisation and full recording-stack verification are not audited.
    #[test]
    #[ignore = "requires the installed game and Workshop corpus"]
    fn intelligence_installed_content_readers() {
        let game =
            std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("BATTLE_PLAN_GAME_DIR"));
        let workshop = std::path::PathBuf::from(
            std::env::var_os("BATTLE_PLAN_WORKSHOP_DIR").or_else(|| std::env::var_os("HOI4_WORKSHOP_DIR")).expect("BATTLE_PLAN_WORKSHOP_DIR"),
        );
        let mut roots = vec![game.clone()];
        let mut mods: Vec<_> = std::fs::read_dir(workshop)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
            .collect();
        mods.sort();
        roots.extend(mods);
        let mut files = 0;
        let mut operations = 0;
        let mut refusals = Vec::new();
        println!("Inventorying base {}", game.display());
        let vanilla = ResolvedLayers::from_test_common_folders(&[(&game, &[][..])]).unwrap();
        for root in &roots {
            let replacements = std::fs::read(root.join("descriptor.mod"))
                .ok()
                .map(|bytes| {
                    crate::parser::parse(&bytes)
                        .unwrap()
                        .get_all(b"replace_path")
                        .map(|value| value.read_str().unwrap().to_owned())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let replacement_refs: Vec<_> = replacements.iter().map(String::as_str).collect();
            let layers = if root == &game {
                vanilla.clone()
            } else {
                println!("Inventorying mod {}", root.display());
                let mut roster = vanilla.layers.clone();
                let additional =
                    ResolvedLayers::from_test_common_folders(&[(root, &replacement_refs)]).unwrap();
                for mut layer in additional.layers {
                    layer.id = format!("test{}", roster.len());
                    roster.push(layer);
                }
                ResolvedLayers::from_test_layers(roster).unwrap()
            };
            let names = layers.key_names().unwrap();
            let Ok(entries) = std::fs::read_dir(root.join("common/operations")) else {
                continue;
            };
            let mut paths: Vec<_> = entries
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "txt"))
                .collect();
            paths.sort();
            for path in paths {
                files += 1;
                let bytes = std::fs::read(&path).unwrap();
                match parse_operations(&bytes, &names, &BTreeSet::new()) {
                    Ok((definitions, warnings)) => {
                        for warning in warnings {
                            println!("NOTE {}: {warning}", path.display());
                        }
                        operations += definitions.len();
                        println!("{}: {} definitions", path.display(), definitions.len());
                    }
                    Err(why) => refusals.push(format!("{}: {why}", path.display())),
                }
            }
        }
        println!(
            "{} roots, {files} files, {operations} operations, {} refused files",
            roots.len(),
            refusals.len()
        );
        for refusal in &refusals {
            println!("REFUSED {refusal}");
        }
        assert!(
            refusals.is_empty(),
            "reached content forms must be settled: {} refused files",
            refusals.len()
        );
    }
}
