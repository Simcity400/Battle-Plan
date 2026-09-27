//! Idea categories (`common/idea_tags`) and ideas (`common/ideas`): which slot
//! or category each idea belongs to, and what it shows (name, picture, effects).
//!
//! The recording names active ideas by token only; this is what joins a token to
//! its category, slot and effects. Nothing here assumes vanilla categories:
//! a mod's categories and slots come through the same reader.
//!
//! Conditions (`allowed`, `visible`, `available`) and effects (`on_add`, …) need
//! the game to evaluate and are not read. Modifier values the reader cannot
//! evaluate stay [`Amount::Unsupported`](crate::modifiers::Amount).

use crate::{
    ResolvedLayers,
    modifiers::{self, EquipmentBonus, Modifier, Modifiers, TargetedModifier, Variables},
    parser::{self, Document, Item},
    script_files,
};
use std::{collections::BTreeMap, io};

/// One idea category from `idea_categories`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdeaCategory {
    pub name: String,
    /// Idea slots, in source order (laws, designers).
    pub slots: Vec<String>,
    /// Character seats, one entry per seat (three political advisors are three entries).
    pub character_slots: Vec<String>,
    /// `type`, such as `national_spirit`.
    pub kind: Option<String>,
    pub hidden: bool,
    /// False when the category sets `politics_tab = no`.
    pub politics_tab: bool,
    /// Every ledger it shows on; `ledger` may be assigned more than once.
    pub ledgers: Vec<String>,
    pub cost: Option<f64>,
    pub removal_cost: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdeaTags {
    /// In load order: files by layer then path, categories in source order.
    pub categories: Vec<IdeaCategory>,
}

impl IdeaTags {
    pub fn category(&self, name: &str) -> Option<&IdeaCategory> {
        self.categories
            .iter()
            .find(|category| category.name == name)
    }

    /// The category holding an idea slot or character seat.
    pub fn category_of_slot(&self, slot: &str) -> Option<&IdeaCategory> {
        self.categories.iter().find(|category| {
            category.slots.iter().any(|s| s == slot)
                || category.character_slots.iter().any(|s| s == slot)
        })
    }
}

/// One idea from `ideas = { <group> = { <token> = { … } } }`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Idea {
    pub token: String,
    /// The outer key: an idea slot (`economy`, `tank_manufacturer`) or a category
    /// (`country`, `hidden_ideas`).
    pub group: String,
    pub law: bool,
    pub designer: bool,
    /// The slot's default law.
    pub default: bool,
    /// A localisation key other than the token, from `name`.
    pub name: Option<String>,
    /// The picture's name, from `picture`; the sprite is `GFX_idea_<picture>`.
    pub picture: Option<String>,
    pub modifiers: Modifiers,
    pub targeted: Vec<TargetedModifier>,
    pub equipment_bonuses: Vec<EquipmentBonus>,
    pub research_bonuses: Vec<Modifier>,
    pub traits: Vec<String>,
    pub cost: Option<f64>,
    pub removal_cost: Option<f64>,
    /// Every ledger it shows on; `ledger` may be assigned more than once.
    pub ledgers: Vec<String>,
}

impl Idea {
    /// The localisation key of its name.
    pub fn name_key(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.token)
    }

    /// The localisation key of its description, which follows a `name` override
    /// (`CHL_the_mapuche_conflict_rebellion` is described by `CHL_the_mapuche_conflict_desc`).
    pub fn description_key(&self) -> String {
        format!("{}_desc", self.name_key())
    }

    /// The sprite the game draws it with: its picture's, else its token's.
    pub fn sprite(&self) -> String {
        format!(
            "GFX_idea_{}",
            self.picture.as_deref().unwrap_or(&self.token)
        )
    }
}

fn scalar<'a>(items: &'a [Item], key: &[u8]) -> Option<&'a str> {
    items
        .iter()
        .rev()
        .find(|item| item.key.as_deref() == Some(key))?
        .value
        .scalar_str()
}

fn flag(items: &[Item], key: &[u8]) -> bool {
    scalar(items, key) == Some("yes")
}

fn scalars(items: &[Item], key: &[u8]) -> Vec<String> {
    items
        .iter()
        .filter(|item| item.key.as_deref() == Some(key))
        .filter_map(|item| item.value.scalar_str().map(str::to_owned))
        .collect()
}

fn number(items: &[Item], key: &[u8]) -> Option<f64> {
    scalar(items, key)?
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

fn key(item: &Item) -> Option<&str> {
    std::str::from_utf8(item.key.as_deref()?).ok()
}

/// Parses one `common/idea_tags` file.
pub fn parse_tags(input: &[u8]) -> io::Result<Vec<IdeaCategory>> {
    Ok(tags_from(&parser::parse_unclosed(input)?.0))
}

fn tags_from(document: &Document) -> Vec<IdeaCategory> {
    let mut categories = Vec::new();
    for root in document.get_all(b"idea_categories") {
        for item in root.items().unwrap_or_default() {
            let (Some(name), Some(items)) = (key(item), item.value.items()) else {
                continue;
            };
            let all = |wanted: &[u8]| {
                items
                    .iter()
                    .filter(|i| i.key.as_deref() == Some(wanted))
                    .filter_map(|i| i.value.scalar_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            };
            categories.push(IdeaCategory {
                name: name.to_owned(),
                slots: all(b"slot"),
                character_slots: all(b"character_slot"),
                kind: scalar(items, b"type").map(str::to_owned),
                hidden: flag(items, b"hidden"),
                politics_tab: scalar(items, b"politics_tab") != Some("no"),
                ledgers: scalars(items, b"ledger"),
                cost: number(items, b"cost"),
                removal_cost: number(items, b"removal_cost"),
            });
        }
    }
    categories
}

/// Keys inside an idea that are not its effects.
const IDEA_STRUCTURE: &[&str] = &[];

/// Parses one `common/ideas` file.
pub fn parse_ideas(input: &[u8]) -> io::Result<Vec<Idea>> {
    Ok(ideas_from(&parser::parse_unclosed(input)?.0))
}

fn ideas_from(document: &Document) -> Vec<Idea> {
    let variables: Variables = modifiers::variables(&document.items);
    let mut ideas = Vec::new();
    for root in document.get_all(b"ideas") {
        for group in root.items().unwrap_or_default() {
            let (Some(group_name), Some(group_items)) = (key(group), group.value.items()) else {
                continue;
            };
            let law = flag(group_items, b"law");
            let designer = flag(group_items, b"designer");
            for item in group_items {
                let (Some(token), Some(items)) = (key(item), item.value.items()) else {
                    continue;
                };
                let mut idea = Idea {
                    token: token.to_owned(),
                    group: group_name.to_owned(),
                    law,
                    designer: designer || flag(items, b"designer"),
                    default: flag(items, b"default"),
                    name: scalar(items, b"name").map(str::to_owned),
                    picture: scalar(items, b"picture").map(str::to_owned),
                    cost: number(items, b"cost"),
                    removal_cost: number(items, b"removal_cost"),
                    ledgers: scalars(items, b"ledger"),
                    ..Idea::default()
                };
                for child in items {
                    let Some(children) = child.value.items() else {
                        continue;
                    };
                    match child.key.as_deref() {
                        // Several modifier blocks add up, in source order.
                        Some(b"modifier") => {
                            let block = modifiers::values(children, IDEA_STRUCTURE, &variables);
                            idea.modifiers.values.extend(block.values);
                            idea.modifiers.hidden.extend(block.hidden);
                            idea.modifiers.tooltips.extend(block.tooltips);
                        }
                        Some(b"targeted_modifier") => idea
                            .targeted
                            .push(modifiers::targeted(children, &variables)),
                        Some(b"equipment_bonus") => idea
                            .equipment_bonuses
                            .extend(modifiers::equipment_bonuses(children, &variables)),
                        Some(b"research_bonus") => idea
                            .research_bonuses
                            .extend(modifiers::values(children, IDEA_STRUCTURE, &variables).values),
                        Some(b"traits") => idea.traits.extend(
                            children
                                .iter()
                                .filter(|trait_item| trait_item.key.is_none())
                                .filter_map(|trait_item| {
                                    trait_item.value.scalar_str().map(str::to_owned)
                                }),
                        ),
                        _ => {}
                    }
                }
                ideas.push(idea);
            }
        }
    }
    ideas
}

/// Every idea category and idea in the content stack.
#[derive(Clone, Debug, Default)]
pub struct IdeaContent {
    pub tags: IdeaTags,
    /// By token. A token defined again later in load order replaces the earlier one.
    pub ideas: BTreeMap<String, Idea>,
}

/// Reads `common/idea_tags` and `common/ideas`. A file that cannot be parsed is
/// left out with a warning; other files still load.
pub fn read(layers: &ResolvedLayers) -> io::Result<(IdeaContent, Vec<String>)> {
    let mut content = IdeaContent::default();
    let mut warnings = Vec::new();
    for (_, document) in script_files::documents(
        layers,
        "common/idea_tags/",
        "Idea categories",
        &mut warnings,
    )? {
        for category in tags_from(&document) {
            // A category defined again replaces the earlier definition in place.
            match content
                .tags
                .categories
                .iter_mut()
                .find(|c| c.name == category.name)
            {
                Some(existing) => *existing = category,
                None => content.tags.categories.push(category),
            }
        }
    }
    let mut repeated = 0;
    for (_, document) in script_files::documents(layers, "common/ideas/", "Ideas", &mut warnings)? {
        for idea in ideas_from(&document) {
            repeated += usize::from(content.ideas.insert(idea.token.clone(), idea).is_some());
        }
    }
    if repeated > 0 {
        warnings.push(format!(
            "{repeated} ideas are defined more than once; the last definition in load order is used."
        ));
    }
    Ok((content, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifiers::Amount;

    #[test]
    fn categories_keep_slots_seats_and_the_politics_tab_flag() {
        let categories = parse_tags(
            b"idea_categories = { government = { slot = economy slot = trade_laws character_slot = political_advisor \
              character_slot = political_advisor ledger = civilian cost = 150 removal_cost = 0 } \
              country = { type = national_spirit cost = 0 removal_cost = -1 } \
              army_spirit = { politics_tab = no type = army_spirit } hidden_ideas = { hidden = yes } } \
              slot_ledgers = { army_chief = army }",
        )
        .unwrap();
        let tags = IdeaTags { categories };
        let government = tags.category("government").unwrap();
        assert_eq!(government.slots, ["economy", "trade_laws"]);
        assert_eq!(government.character_slots.len(), 2);
        assert_eq!(
            (government.cost, government.removal_cost),
            (Some(150.0), Some(0.0))
        );
        assert_eq!(
            tags.category_of_slot("political_advisor").unwrap().name,
            "government"
        );
        assert_eq!(
            tags.category("country").unwrap().kind.as_deref(),
            Some("national_spirit")
        );
        assert!(!tags.category("army_spirit").unwrap().politics_tab);
        assert!(tags.category("hidden_ideas").unwrap().hidden);
    }

    #[test]
    fn ideas_carry_their_group_law_flag_picture_and_effects() {
        let ideas = parse_ideas(
            b"@bonus = 0.15 ideas = { economy = { law = yes use_list_view = yes \
                civilian_economy = { default = yes allowed = { always = yes } modifier = { consumer_goods_factor = 0.35 } } } \
              tank_manufacturer = { designer = yes porsche = { picture = porsche_logo research_bonus = { armor = @bonus } \
                traits = { heavy_tank_manufacturer } equipment_bonus = { armor = { armor_value = 0.05 } } } } \
              country = { GER_spirit = { name = GER_SPIRIT_NAME modifier = { stability_factor = 0.1 } \
                modifier = { war_support_factor = [?x] custom_modifier_tooltip = GER_TIP } \
                targeted_modifier = { tag = SOV attack_bonus_against = 0.1 } } } }",
        )
        .unwrap();
        let by = |token: &str| ideas.iter().find(|idea| idea.token == token).unwrap();
        let law = by("civilian_economy");
        assert!(law.law && law.default && !law.designer);
        assert_eq!(law.group, "economy");
        assert_eq!(law.sprite(), "GFX_idea_civilian_economy");
        let designer = by("porsche");
        assert!(designer.designer);
        assert_eq!(designer.sprite(), "GFX_idea_porsche_logo");
        assert_eq!(designer.research_bonuses[0].amount, Amount::Number(0.15));
        assert_eq!(designer.traits, ["heavy_tank_manufacturer"]);
        assert_eq!(designer.equipment_bonuses[0].equipment, "armor");
        let spirit = by("GER_spirit");
        assert_eq!(spirit.name_key(), "GER_SPIRIT_NAME");
        // The name override names the description too.
        assert_eq!(spirit.description_key(), "GER_SPIRIT_NAME_desc");
        assert_eq!(
            spirit
                .modifiers
                .values
                .iter()
                .map(|m| (m.key.as_str(), m.amount.clone()))
                .collect::<Vec<_>>(),
            [
                ("stability_factor", Amount::Number(0.1)),
                ("war_support_factor", Amount::Unsupported("[?x]".into()))
            ]
        );
        assert_eq!(spirit.modifiers.tooltips, ["GER_TIP"]);
        assert_eq!(spirit.targeted[0].tag.as_deref(), Some("SOV"));
    }
}
