//! Presentation fields of the native special-project databases. Definitions
//! are keyed by LexerToken, not by their position in the database.

use crate::{
    ResolvedLayers,
    fields::invalid,
    parser::{
        Value,
        native::{self, Mode, Shape},
    },
    script_files,
    tokens::Token,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

mod output;
mod token_set;
pub use output::Output;
pub use token_set::order as specialization_order;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    pub specialization: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub refusal: Option<String>,
    prototype_time: Vec<crate::parser::Item>,
    complexity: Vec<crate::parser::Item>,
    pub output: Output,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub projects: BTreeMap<String, Project>,
    pub specializations: BTreeSet<String>,
    /// First registered token spelling, before any repeated definition is
    /// skipped (1401698B0). The public raw-name set remains for its existing
    /// consumers; effect display names use this token-identity projection.
    specialization_tokens: BTreeMap<Token, String>,
    /// 140AABAD0 copies the manager's registration array, not token order.
    specialization_order: Vec<String>,
    pub warnings: Vec<String>,
    /// An unreadable stream may override prior tokens; none are assumed valid.
    pub unsettled: Option<String>,
}

#[derive(Clone, Copy)]
enum Node {
    Projects,
    Project,
    Narrative,
    Specializations,
    Specialization,
    Body,
    Complexity,
    Mtth,
    Costs,
    Uniform,
    Reward,
    Option,
    Output,
    DlcList,
    EquipmentBonus,
    EquipmentStats,
}

fn read(
    bytes: &[u8],
    specialization: bool,
    names: Option<&crate::key_names::KeyNames>,
    earlier: Option<&BTreeSet<Token>>,
) -> io::Result<Vec<crate::parser::Item>> {
    let mut registered = earlier.cloned().unwrap_or_default();
    // Projects 140141670/140130160: mode 0, reuse repeated templates.
    // Specializations 14018ABD0 -> 14016A5B0/1401698B0: mode 0, first
    // token only, repeated bodies skipped. Dispatch 140AAC100/140A85D70.
    let read = native::read(
        bytes,
        Mode::KeepMark,
        if specialization {
            native::Top::Persistent(Node::Specializations)
        } else {
            native::Top::File(Node::Projects)
        },
        names,
        &mut |node, field| match node {
            Node::Projects => Shape::Object(Node::Project),
            Node::Specializations => {
                if registered.insert(Token::c_str(field.key.c_str())) {
                    Shape::Object(Node::Specialization)
                } else {
                    Shape::Skip
                }
            }
            Node::Project => match field.keyword().as_deref() {
                Some(b"specialization" | b"icon" | b"blueprint_image" | b"prototype_time") => {
                    Shape::One
                }
                Some(b"narrative") => Shape::Object(Node::Narrative),
                Some(b"special_project_parent") => Shape::ListIfOpen,
                Some(b"project_tags") => Shape::ListIfOpen,
                Some(b"visible" | b"allowed" | b"available") => Shape::Trigger,
                Some(b"complexity") => Shape::ObjectIfOpen(Node::Complexity), // 141453030
                Some(b"project_output") => Shape::Object(Node::Output),
                Some(b"ai_will_do" | b"empty_reward_weight") => Shape::Object(Node::Mtth),
                Some(b"resource_cost") => Shape::Object(Node::Body),
                Some(b"breakthrough_cost") => Shape::Object(Node::Costs),
                // 140A85D70 -> 1409BFE50: the shared custom statement loop
                // supports both references and inline reward definitions.
                Some(b"unique_prototype_rewards" | b"generic_prototype_rewards") => {
                    Shape::Statements(Node::Uniform)
                }
                _ => Shape::Skip,
            },
            Node::Narrative => match field.keyword().as_deref() {
                Some(b"name" | b"desc") => Shape::One,
                _ => Shape::Skip,
            },
            Node::Specialization => match field.keyword().as_deref() {
                Some(b"icon" | b"blueprint_image" | b"program_background") => Shape::One,
                Some(b"color") => Shape::Color,
                _ => Shape::One, // 140AAC100 returns without ReportUnexpectedToken.
            },
            Node::Complexity => match field.keyword().as_deref() {
                Some(b"min" | b"max") => Shape::One,
                _ => Shape::Skip,
            },
            Node::Mtth => native::mean_time_to_happen(field),
            Node::Costs => Shape::ObjectIfOpen(Node::Mtth),
            Node::Uniform => {
                if field.opens() {
                    Shape::Object(Node::Reward)
                } else {
                    Shape::One
                }
            }
            // CPrototypeReward::ReadToken 140A82ED0; scalar presentation is
            // projected away, but every reached reader still consumes its form.
            Node::Reward => match field.keyword().as_deref() {
                Some(b"option") => Shape::Object(Node::Option),
                Some(b"threshold") => Shape::Object(Node::Complexity),
                Some(b"weight") => Shape::Object(Node::Mtth),
                Some(b"narrative") => Shape::Object(Node::Narrative),
                Some(b"allowed") => Shape::Trigger,
                Some(b"icon" | b"fire_only_once" | b"force_reward_if_available") => Shape::One,
                _ => Shape::Skip,
            },
            Node::Option => match field.keyword().as_deref() {
                Some(b"narrative") => Shape::Object(Node::Narrative),
                Some(b"iteration_output") => Shape::Object(Node::Output),
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                Some(b"default" | b"token") => Shape::One,
                _ => Shape::Skip, // CPrototypeRewardOption::ReadToken 140A83310.
            },
            // SOutput::ReadToken 140A83400 -> 1409BFE50/14048FD40.
            Node::Output => match field.keyword().as_deref() {
                Some(b"country_effects") => Shape::EffectIn(4),
                Some(b"scientist_effects") => Shape::EffectIn(8),
                Some(b"facility_state_effects") => Shape::EffectIn(2),
                Some(b"enable_equipments" | b"enable_equipment_modules" | b"enable_subunits") => {
                    Shape::Statements(Node::DlcList)
                }
                Some(b"equipment_bonus") => Shape::Statements(Node::EquipmentBonus),
                Some(b"sub_unit_bonus") => Shape::Object(Node::Body),
                _ => Shape::Skip,
            },
            Node::DlcList => {
                if field.keyword_is("limit") {
                    Shape::Trigger
                } else {
                    Shape::One
                }
            }
            Node::EquipmentBonus => Shape::Object(Node::EquipmentStats),
            Node::EquipmentStats => {
                if field.keyword_is("instant")
                    || field
                        .keyword()
                        .as_deref()
                        .is_some_and(|key| crate::equipment::stat_index(key).is_some())
                {
                    Shape::One // ReadAndCombineStatToken 140625D00.
                } else {
                    Shape::Skip
                }
            }
            // These fields do not supply project presentation. The shared
            // by-key reader walks their ordinary nested trigger/effect forms;
            // ambiguous consumption is a named refusal, never a tree guess.
            Node::Body => Shape::ByKey,
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("special projects: {cause}")));
    }
    Ok(read.items)
}

fn string(value: &Value) -> io::Result<String> {
    let bytes = value
        .read_string()
        .ok_or_else(|| invalid("special project string is not a scalar"))?;
    String::from_utf8(bytes.to_vec()).map_err(|e| invalid(format!("special project string: {e}")))
}

impl Catalog {
    pub fn specializations_in_order(&self) -> Result<&[String], String> {
        if let Some(why) = &self.unsettled {
            return Err(why.clone());
        }
        Ok(&self.specialization_order)
    }

    /// Native token match (14022F320), with the first registered spelling
    /// used by get_token_string (1424A7300) in specialization names
    /// (140AABD40). An unsettled stream cannot prove a lookup's answer.
    pub fn specialization(&self, token: &Token) -> Result<Option<&str>, String> {
        if let Some(why) = &self.unsettled {
            return Err(why.clone());
        }
        Ok(self.specialization_tokens.get(token).map(String::as_str))
    }

    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut result = Self::default();
        // CBuildingDatabase::load 14018B610 loads project bodies after buildings.
        let names = layers.key_names()?;
        for (folder, specialization) in [
            ("common/special_projects/specialization/", true),
            ("common/special_projects/projects/", false),
        ] {
            let mut registered = BTreeSet::new();
            for path in script_files::paths(layers, folder, Some(".txt")) {
                let items = match read(
                    &layers.read(&path)?,
                    specialization,
                    Some(&names),
                    Some(&registered),
                ) {
                    Ok(items) => items,
                    Err(error) => {
                        let reason = format!("Special projects: refused 1 file {path}: {error}");
                        result.unsettled = Some(reason.clone());
                        result.warnings.push(reason);
                        continue;
                    }
                };
                for item in items {
                    let token = match item.key_str() {
                        Ok(token) => token.to_owned(),
                        Err(error) => {
                            result.warnings.push(format!(
                                "Special projects: refused 1 definition in {path}: {error}"
                            ));
                            continue;
                        }
                    };
                    if specialization {
                        let identity = Token::c_str(token.as_bytes());
                        registered.insert(identity.clone());
                        if !result.specialization_tokens.contains_key(&identity) {
                            result.specialization_order.push(token.clone());
                        }
                        result
                            .specialization_tokens
                            .entry(identity)
                            .or_insert_with(|| token.clone());
                        result.specializations.insert(token);
                        continue;
                    }
                    let project = result.projects.entry(token.clone()).or_default();
                    // 140130160 reuses the earlier template: scalars overwrite,
                    // omitted fields retain their previous value.
                    for field in item.value.items().unwrap_or_default() {
                        if field.keyword_is("prototype_time") {
                            project.prototype_time.push(field.clone());
                        } else if field.keyword_is("complexity") {
                            project.complexity.push(field.clone());
                        } else if field.keyword_is("project_output") {
                            project.output.read(field.value.items().unwrap_or_default());
                        }
                        let destination = if field.keyword_is("specialization") {
                            Some(&mut project.specialization)
                        } else if field.keyword_is("icon") {
                            Some(&mut project.icon)
                        } else {
                            None
                        };
                        if let Some(destination) = destination {
                            match string(&field.value) {
                                Ok(value) => *destination = Some(value),
                                Err(error) => {
                                    let reason = format!(
                                        "{path}: {token}: refused 1 {}: {error}",
                                        field.key_str().unwrap_or("field")
                                    );
                                    project.refusal = Some(reason.clone());
                                    result.warnings.push(reason);
                                }
                            }
                        }
                        if field.keyword_is("narrative") {
                            for entry in field.value.items().unwrap_or_default() {
                                if entry.keyword_is("name") {
                                    match string(&entry.value) {
                                        Ok(value) => project.name = Some(value),
                                        Err(error) => {
                                            let reason = format!(
                                                "{path}: {token}: refused 1 narrative.name: {error}"
                                            );
                                            project.refusal = Some(reason.clone());
                                            result.warnings.push(reason);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_scalar_consumption_variables_and_narrative_dispatch() {
        let items = read(br#"@kind = mod_science p = { SPECIALIZATION = @kind icon = GFX_mod narrative = { name = mod_name desc = x unknown = {name=no} } }"#, false, None, None).unwrap();
        let fields = items[0].value.items().unwrap();
        assert_eq!(string(&fields[0].value).unwrap(), "mod_science");
        assert_eq!(
            fields[2].value.items().unwrap()[0].value.scalar_str(),
            Some("mod_name")
        );
        let items = read(b"p={icon={specialization=mod_kind}}", false, None, None).unwrap();
        assert_eq!(
            items[0].value.items().unwrap()[1].value.scalar_str(),
            Some("mod_kind")
        );
    }

    #[test]
    fn specialization_tokens_keep_first_spelling_and_skip_repeated_bodies() {
        let root = std::env::temp_dir().join(format!(
            "installed-content-specialization-tokens-{}",
            std::process::id()
        ));
        let folder = root.join("common/special_projects/specialization");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("a.txt"),
            "mod_kind = { icon = first } MOD_KIND = { @skip = changed } after = { icon = @skip }",
        )
        .unwrap();
        std::fs::write(folder.join("b.txt"), "MOD_KIND = { icon = later }").unwrap();
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let catalog = Catalog::read(&layers).unwrap();
        assert_eq!(
            catalog.specialization(&Token::c_str(b"MOD_KIND")).unwrap(),
            Some("mod_kind")
        );
        // Preserve the public raw-name set used by existing consumers.
        assert!(catalog.specializations.contains("MOD_KIND"));
        assert_eq!(
            catalog.specialization(&Token::c_str(b"missing")).unwrap(),
            None
        );
        let items = read(
            b"kind = { } KIND = { @skip = wrong } after = { icon = @skip }",
            true,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            items[2].value.items().unwrap()[0].value.scalar_str(),
            Some("@skip")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Read-only audit of the installed vanilla and Workshop corpus. No game.
    #[test]
    #[ignore = "requires BATTLE_PLAN_SP_GAME and BATTLE_PLAN_SP_WORKSHOP read-only content folders"]
    fn installed_content_corpus() {
        use std::path::Path;
        fn walk(
            root: &Path,
            specialization: bool,
            files: &mut usize,
            definitions: &mut usize,
            refusals: &mut Vec<String>,
            names: &crate::key_names::KeyNames,
        ) {
            if !root.is_dir() {
                return;
            }
            for entry in std::fs::read_dir(root).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, specialization, files, definitions, refusals, names);
                } else if path.extension().is_some_and(|ext| ext == "txt") {
                    *files += 1;
                    match read(
                        &std::fs::read(&path).unwrap(),
                        specialization,
                        Some(names),
                        None,
                    ) {
                        Ok(items) => *definitions += items.len(),
                        Err(reason) => refusals.push(format!("{}: {reason}", path.display())),
                    }
                }
            }
        }
        let game = std::env::var_os("BATTLE_PLAN_SP_GAME").or_else(|| std::env::var_os("HOI4_SP_GAME")).expect("BATTLE_PLAN_SP_GAME");
        let workshop = std::env::var_os("BATTLE_PLAN_SP_WORKSHOP").or_else(|| std::env::var_os("HOI4_SP_WORKSHOP")).expect("BATTLE_PLAN_SP_WORKSHOP");
        let mut roots = vec![std::path::PathBuf::from(game)];
        roots.extend(
            std::fs::read_dir(workshop)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.is_dir()),
        );
        roots.sort();
        let folders: Vec<_> = roots.iter().map(|root| (root.as_path(), &[][..])).collect();
        let layers = ResolvedLayers::from_folders(&folders).unwrap();
        let names = layers.key_names().unwrap();
        let mut refusals = Vec::new();
        for root in roots {
            let (mut files, mut definitions) = (0, 0);
            for (folder, specialization) in [("projects", false), ("specialization", true)] {
                walk(
                    &root.join("common/special_projects").join(folder),
                    specialization,
                    &mut files,
                    &mut definitions,
                    &mut refusals,
                    &names,
                );
            }
            println!(
                "{}: {files} files, {definitions} definitions",
                root.display()
            );
        }
        for reason in &refusals {
            println!("REFUSED {reason}");
        }
        assert!(refusals.is_empty(), "{} refused files", refusals.len());
    }
}
