use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{fields::invalid, parser::Value, tokens::Token};
use content_reader_script::native::{self, Mode, Shape};
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
    prototype_time: Vec<content_reader_parser::parser::Item>,
    complexity: Vec<content_reader_parser::parser::Item>,
    pub output: Output,
    pub reward_order: Vec<RewardOccurrence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RewardOccurrence {
    Inline(PrototypeReward),
    Reference(Token),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrototypeReward {
    pub token: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub options: Vec<PrototypeOption>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrototypeOption {
    pub token: String,
    pub name: Option<String>,
    pub output: Output,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub projects: BTreeMap<String, Project>,
    pub rewards: BTreeMap<Token, PrototypeReward>,
    pub specializations: BTreeSet<String>,
    specialization_tokens: BTreeMap<Token, String>,
    specialization_order: Vec<String>,
    pub warnings: Vec<String>,
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
    names: Option<&content_reader_script::key_names::KeyNames>,
    earlier: Option<&BTreeSet<Token>>,
) -> io::Result<Vec<content_reader_parser::parser::Item>> {
    read_mode(bytes, specialization, false, names, earlier)
}

fn read_mode(
    bytes: &[u8],
    specialization: bool,
    rewards: bool,
    names: Option<&content_reader_script::key_names::KeyNames>,
    earlier: Option<&BTreeSet<Token>>,
) -> io::Result<Vec<content_reader_parser::parser::Item>> {
    let mut registered = earlier.cloned().unwrap_or_default();
    let read = native::read(
        bytes,
        Mode::KeepMark,
        if rewards {
            native::Top::File(Node::Uniform)
        } else if specialization {
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
                Some(b"complexity") => Shape::ObjectIfOpen(Node::Complexity),
                Some(b"project_output") => Shape::Object(Node::Output),
                Some(b"ai_will_do" | b"empty_reward_weight") => Shape::Object(Node::Mtth),
                Some(b"resource_cost") => Shape::Object(Node::Body),
                Some(b"breakthrough_cost") => Shape::Object(Node::Costs),
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
                _ => Shape::One,
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
                _ => Shape::Skip,
            },
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
                        .is_some_and(|key| content_reader_script::equipment_stats::stat_index(key).is_some())
                {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
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
    pub fn reward_named(&self, project: &str, token: &str) -> Result<&PrototypeReward, String> {
        if let Some(reason) = &self.unsettled {
            return Err(reason.clone());
        }
        let project = self
            .projects
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(project))
            .map(|(_, value)| value)
            .ok_or("Special project definition is unavailable")?;
        let mut found = project.reward_order.iter().filter_map(|occurrence| match occurrence {
            RewardOccurrence::Inline(value) => Some(value),
            RewardOccurrence::Reference(identity) => self.rewards.get(identity),
        }).filter(|reward| Token::c_str(reward.token.as_bytes()) == Token::c_str(token.as_bytes()));
        let reward = found.next().ok_or("Chosen reward is absent from recorded content")?;
        if found.any(|other| !std::ptr::eq(other, reward)) {
            return Err("Several reward occurrences share this recorded token".into());
        }
        Ok(reward)
    }

    pub fn reward_at(
        &self,
        project: &str,
        ordinal: u32,
        token: &str,
    ) -> Result<&PrototypeReward, String> {
        if let Some(reason) = &self.unsettled {
            return Err(reason.clone());
        }
        let project = self
            .projects
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(project))
            .map(|(_, value)| value)
            .ok_or("Special project definition is unavailable")?;
        let occurrence = project
            .reward_order
            .get(ordinal as usize)
            .ok_or("Pending reward occurrence is absent from recorded content")?;
        let reward = match occurrence {
            RewardOccurrence::Inline(value) => value,
            RewardOccurrence::Reference(identity) => self
                .rewards
                .get(identity)
                .ok_or("Pending global reward is absent from recorded content")?,
        };
        if Token::c_str(reward.token.as_bytes()) != Token::c_str(token.as_bytes()) {
            return Err("Pending reward occurrence disagrees with its recorded token".into());
        }
        Ok(reward)
    }
    pub fn specializations_in_order(&self) -> Result<&[String], String> {
        if let Some(why) = &self.unsettled {
            return Err(why.clone());
        }
        Ok(&self.specialization_order)
    }

    pub fn specialization(&self, token: &Token) -> Result<Option<&str>, String> {
        if let Some(why) = &self.unsettled {
            return Err(why.clone());
        }
        Ok(self.specialization_tokens.get(token).map(String::as_str))
    }

    pub fn cached(layers: &ResolvedLayers) -> Result<std::sync::Arc<Self>, String> {
        #[derive(Default)]
        struct Cached(content_reader_layers::content::diagnostics::Once<Result<std::sync::Arc<Catalog>, String>>);
        layers
            .derived::<Cached>()
            .0
            .get_or_init(|| {
                Self::read(layers)
                    .map(std::sync::Arc::new)
                    .map_err(|why| why.to_string())
            })
            .clone()
    }

    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut result = Self::default();
        let mut project_tokens = BTreeMap::<Token, String>::new();
        let names = layers.key_names()?;
        for path in script_files::paths(
            layers,
            "common/special_projects/prototype_rewards/",
            Some(".txt"),
        )? {
            match read_mode(&path.read(layers)?, false, true, Some(&names), None) {
                Ok(items) => {
                    for item in items {
                        let token = item.key_str().map_err(|error| invalid(error.to_string()))?;
                        let identity = Token::c_str(token.as_bytes());
                        let reward =
                            result
                                .rewards
                                .entry(identity)
                                .or_insert_with(|| PrototypeReward {
                                    token: token.into(),
                                    ..Default::default()
                                });
                        read_reward(reward, item.value.items().unwrap_or_default())?;
                    }
                }
                Err(error) => {
                    let reason = format!("Prototype rewards: refused 1 file {path}: {error}");
                    result.unsettled = Some(reason.clone());
                    result.warnings.push(reason);
                }
            }
        }
        for (folder, specialization) in [
            ("common/special_projects/specialization/", true),
            ("common/special_projects/projects/", false),
        ] {
            let mut registered = BTreeSet::new();
            for path in script_files::paths(layers, folder, Some(".txt"))? {
                let items = match read(
                    &path.read(layers)?,
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
                    let registered = project_tokens
                        .entry(Token::c_str(token.as_bytes()))
                        .or_insert_with(|| token.clone());
                    let project = result.projects.entry(registered.clone()).or_default();
                    for field in item.value.items().unwrap_or_default() {
                        if field.keyword_is("prototype_time") {
                            project.prototype_time.push(field.clone());
                        } else if field.keyword_is("complexity") {
                            project.complexity.push(field.clone());
                        } else if field.keyword_is("project_output") {
                            project.output.read(field.value.items().unwrap_or_default());
                        } else if field.keyword_is("unique_prototype_rewards")
                            || field.keyword_is("generic_prototype_rewards")
                        {
                            for reward in field.value.items().unwrap_or_default() {
                                let token = reward
                                    .key
                                    .as_deref()
                                    .or_else(|| reward.value.read_string())
                                    .ok_or_else(|| {
                                        invalid("Prototype reward reference has no token")
                                    })?;
                                if let Some(fields) = reward.value.items() {
                                    let mut occurrence = PrototypeReward {
                                        token: String::from_utf8(
                                            content_reader_parser::parser::c_string(token).to_vec(),
                                        )
                                        .map_err(|error| invalid(error.to_string()))?,
                                        ..Default::default()
                                    };
                                    read_reward(&mut occurrence, fields)?;
                                    project
                                        .reward_order
                                        .push(RewardOccurrence::Inline(occurrence));
                                } else {
                                    let identity = Token::c_str(token);
                                    if result.rewards.contains_key(&identity) {
                                        project
                                            .reward_order
                                            .push(RewardOccurrence::Reference(identity));
                                    }
                                }
                            }
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

fn read_reward(reward: &mut PrototypeReward, fields: &[content_reader_parser::parser::Item]) -> io::Result<()> {
    for field in fields {
        if field.keyword_is("narrative") {
            for entry in field.value.items().unwrap_or_default() {
                if entry.keyword_is("name") {
                    reward.name = Some(string(&entry.value)?);
                }
                if entry.keyword_is("desc") {
                    reward.description = Some(string(&entry.value)?);
                }
            }
        } else if field.keyword_is("option") {
            let mut option = PrototypeOption::default();
            for entry in field.value.items().unwrap_or_default() {
                if entry.keyword_is("token") {
                    option.token = string(&entry.value)?;
                }
                if entry.keyword_is("iteration_output") {
                    option.output.read(entry.value.items().unwrap_or_default());
                }
                if entry.keyword_is("narrative") {
                    for value in entry.value.items().unwrap_or_default() {
                        if value.keyword_is("name") {
                            option.name = Some(string(&value.value)?);
                        }
                    }
                }
            }
            reward.options.push(option);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gap_research_grants_prototype_rewards_resolve_inline_and_generic_mod_options_in_authored_order()
     {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!("project-rewards-{}", std::process::id()));
        let projects = root.join("common/special_projects/projects");
        let rewards = root.join("common/special_projects/prototype_rewards");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::create_dir_all(&rewards).unwrap();
        std::fs::write(rewards.join("mod.txt"), "mod_reward={narrative={name=mod_name desc=mod_description} option={token=first narrative={name=first_name} iteration_output={country_effects={add_stability=0.1}}} option={token=first iteration_output={scientist_effects={add_scientist_level=1}}}}").unwrap();
        std::fs::write(projects.join("mod.txt"), "mod_project={generic_prototype_rewards={mod_reward} unique_prototype_rewards={mod_inline={option={token=inline iteration_output={facility_state_effects={add_manpower=5}}}}}}").unwrap();
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let catalog = Catalog::read(&layers).unwrap();
        let generic = catalog.reward_at("mod_project", 0, "MOD_REWARD").unwrap();
        assert_eq!(generic.name.as_deref(), Some("mod_name"));
        assert_eq!(
            generic
                .options
                .iter()
                .map(|option| option.token.as_str())
                .collect::<Vec<_>>(),
            ["first", "first"]
        );
        assert_eq!(generic.options[0].output.country_effects.len(), 1);
        assert_eq!(generic.options[1].output.scientist_effects.len(), 1);
        assert_eq!(
            catalog
                .reward_at("mod_project", 1, "mod_inline")
                .unwrap()
                .options[0]
                .output
                .facility_state_effects
                .len(),
            1
        );
        assert!(catalog.reward_at("mod_project", 0, "missing").is_err());
        std::fs::write(rewards.join("z.txt"), "MOD_REWARD={option={token=last}}").unwrap();
        std::fs::write(projects.join("z.txt"), "MOD_PROJECT={narrative={name=later_name} unique_prototype_rewards={mod_inline={option={token=other}}}}").unwrap();
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let catalog = Catalog::read(&layers).unwrap();
        assert_eq!(catalog.projects.len(), 1);
        assert_eq!(catalog.projects["mod_project"].reward_order.len(), 3);
        assert_eq!(catalog.rewards.len(), 1);
        assert_eq!(
            catalog.projects["mod_project"].name.as_deref(),
            Some("later_name")
        );
        assert_eq!(
            catalog
                .reward_at("mod_project", 0, "mod_reward")
                .unwrap()
                .options
                .len(),
            3
        );
        assert_eq!(
            catalog
                .reward_at("mod_project", 1, "mod_inline")
                .unwrap()
                .options[0]
                .token,
            "inline"
        );
        assert_eq!(
            catalog
                .reward_at("mod_project", 2, "MOD_INLINE")
                .unwrap()
                .options[0]
                .token,
            "other"
        );
        assert!(catalog.reward_at("mod_project", 1, "mod_reward").is_err());
        assert_eq!(catalog.reward_named("mod_project", "MOD_REWARD").unwrap().options.len(), 3);
        assert!(catalog.reward_named("mod_project", "mod_inline").unwrap_err().contains("Several"));
        assert!(catalog.reward_named("mod_project", "missing").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scalar_consumption_variables_and_narrative_dispatch() {
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
            "content-reader-specialization-tokens-{}",
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

}
