//! Country leader and advisor traits (`common/country_leader`): each trait's
//! modifiers, equipment bonuses and targeted modifiers. Advisors in
//! `common/characters` and designers in `common/ideas` name these traits.
//!
//! Trait files define `@name = value` variables at the top; a value naming one
//! resolves through the same file's definitions only.

use crate::{
    ResolvedLayers,
    modifiers::{self, EquipmentBonus, Modifiers, TargetedModifier},
    parser, script_files,
};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LeaderTrait {
    pub token: String,
    pub modifiers: Modifiers,
    pub equipment_bonuses: Vec<EquipmentBonus>,
    pub targeted: Vec<TargetedModifier>,
}

/// Keys inside a trait that are not modifiers.
const STRUCTURE: &[&str] = &[
    "random",
    "sprite",
    "ai_will_do",
    "equipment_bonus",
    "targeted_modifier",
    // AI preferences, not modifiers.
    "ai_strategy",
];

/// Parses one trait file.
pub fn parse(input: &[u8]) -> io::Result<Vec<LeaderTrait>> {
    Ok(from_document(&parser::parse_unclosed(input)?.0))
}

fn from_document(document: &parser::Document) -> Vec<LeaderTrait> {
    let variables = modifiers::variables(&document.items);
    let mut traits = Vec::new();
    for root in document.get_all(b"leader_traits") {
        for item in root.items().unwrap_or_default() {
            let (Some(token), Some(items)) = (
                item.key
                    .as_deref()
                    .and_then(|key| std::str::from_utf8(key).ok()),
                item.value.items(),
            ) else {
                continue;
            };
            let mut found = LeaderTrait {
                token: token.to_owned(),
                modifiers: modifiers::values(items, STRUCTURE, &variables),
                ..LeaderTrait::default()
            };
            for child in items {
                let Some(children) = child.value.items() else {
                    continue;
                };
                match child.key.as_deref() {
                    Some(b"equipment_bonus") => found
                        .equipment_bonuses
                        .extend(modifiers::equipment_bonuses(children, &variables)),
                    Some(b"targeted_modifier") => found
                        .targeted
                        .push(modifiers::targeted(children, &variables)),
                    _ => {}
                }
            }
            traits.push(found);
        }
    }
    traits
}

/// Every trait in load order; a trait defined again replaces the earlier one.
pub fn read(layers: &ResolvedLayers) -> io::Result<(BTreeMap<String, LeaderTrait>, Vec<String>)> {
    let mut warnings = Vec::new();
    let mut result = BTreeMap::new();
    for (_, document) in script_files::documents(
        layers,
        "common/country_leader/",
        "Leader traits",
        &mut warnings,
    )? {
        result.extend(
            from_document(&document)
                .into_iter()
                .map(|found| (found.token.clone(), found)),
        );
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifiers::Amount;

    #[test]
    fn traits_resolve_file_variables_and_skip_structure() {
        let traits = parse(
            b"@tier2 = 20 leader_traits = { army_regrouping_2 = { random = no sprite = 5 army_morale_factor = 0.08 \
              command_cap_increase = @tier2 ai_will_do = { factor = 1 } } \
              heavy_tank_manufacturer = { equipment_bonus = { armor = { armor_value = 0.05 } } } }",
        )
        .unwrap();
        let values: Vec<_> = traits[0]
            .modifiers
            .values
            .iter()
            .map(|m| (m.key.as_str(), m.amount.clone()))
            .collect();
        assert_eq!(
            values,
            [
                ("army_morale_factor", Amount::Number(0.08)),
                ("command_cap_increase", Amount::Number(20.0))
            ]
        );
        assert!(traits[1].modifiers.values.is_empty());
        assert_eq!(traits[1].equipment_bonuses[0].equipment, "armor");
    }
}
