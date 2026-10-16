//! Rules from the `adjacency_rules` file named by `map/default.map`.
//!
//! Each verdict (contested, enemy, friend, neutral) has four permission bits,
//! all initially set and cleared or set one key at a time: army 1, navy 2,
//! submarine 4, trade 8. Adjacencies name a rule; the first of that name answers.

use std::io;

use crate::{
    ResolvedLayers,
    fields::{self, block, boolean, invalid, key_str},
    parser::{self, Item, Value},
    terrain::DefaultMap,
    triggers::{self, Triggers},
};

/// A verdict's index into [`AdjacencyRule::permissions`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Contested = 0,
    Enemy = 1,
    Friend = 2,
    Neutral = 3,
}

pub const ARMY: u8 = 1;
pub const NAVY: u8 = 2;
pub const SUBMARINE: u8 = 4;
pub const TRADE: u8 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdjacencyRule {
    pub name: String,
    /// Permission bits by [`Verdict`].
    pub permissions: [u8; 4],
    pub required_provinces: Vec<u32>,
    /// A custom-tooltip AND block: its `tooltip` is not a trigger.
    pub is_disabled: Triggers,
    pub is_enemy: Triggers,
    pub is_friend: Triggers,
    pub is_neutral: Triggers,
    /// Why this rule is refused: a field the port cannot read. A crossing
    /// that names it is then unknown, never guessed.
    pub unsupported: Option<String>,
}

impl AdjacencyRule {
    /// A disabled rule answers no permission at all.
    pub fn permits(&self, verdict: Option<Verdict>, bit: u8) -> bool {
        verdict.is_some_and(|verdict| self.permissions[verdict as usize] & bit != 0)
    }
}

fn permissions(value: &Value, what: &str) -> io::Result<u8> {
    let mut bits = 0x0f;
    for item in block(value, what)? {
        let bit = match key_str(item, what)? {
            "army" => ARMY,
            "navy" => NAVY,
            "submarine" => SUBMARINE,
            "trade" => TRADE,
            // The game reports an unknown key and goes on; this reader
            // refuses the rule rather than guess that permission.
            other => return Err(invalid(format!("{what} has unknown permission {other:?}"))),
        };
        if boolean(&item.value, what)? {
            bits |= bit;
        } else {
            bits &= !bit;
        }
    }
    Ok(bits)
}

/// A trigger field's body. The tree read cannot follow the game's reading of
/// a value that opens no block, so the rule is refused by name.
fn trigger(value: &Value, what: &str) -> io::Result<Triggers> {
    triggers::body(value).ok_or_else(|| invalid(format!("{what} is not a block")))
}

/// One rule; a field it cannot read refuses the rule, not the file.
fn rule(items: &[Item]) -> AdjacencyRule {
    let name = fields::last(items, b"name")
        .and_then(Value::scalar_str)
        .unwrap_or_default()
        .to_owned();
    read_rule(items).unwrap_or_else(|error| AdjacencyRule {
        name,
        permissions: [0; 4],
        required_provinces: Vec::new(),
        is_disabled: Triggers::default(),
        is_enemy: Triggers::default(),
        is_friend: Triggers::default(),
        is_neutral: Triggers::default(),
        unsupported: Some(error.to_string()),
    })
}

fn read_rule(items: &[Item]) -> io::Result<AdjacencyRule> {
    let mut rule = AdjacencyRule {
        name: String::new(),
        permissions: [0x0f; 4],
        required_provinces: Vec::new(),
        is_disabled: Triggers::default(),
        is_enemy: Triggers::default(),
        is_friend: Triggers::default(),
        is_neutral: Triggers::default(),
        unsupported: None,
    };
    for item in items {
        let key = key_str(item, "adjacency_rule")?;
        let what = format!("adjacency_rule {:?} {key}", rule.name);
        match key {
            "name" => rule.name = fields::text(&item.value, &what)?.to_owned(),
            "contested" => rule.permissions[0] = permissions(&item.value, &what)?,
            "enemy" => rule.permissions[1] = permissions(&item.value, &what)?,
            "friend" => rule.permissions[2] = permissions(&item.value, &what)?,
            "neutral" => rule.permissions[3] = permissions(&item.value, &what)?,
            "required_provinces" => {
                rule.required_provinces = fields::scalar_list(&item.value, &what)?;
            }
            "is_disabled" => {
                let mut triggers = trigger(&item.value, &what)?;
                triggers.retain(|trigger| trigger.key.as_deref() != Some(b"tooltip"));
                rule.is_disabled = triggers;
            }
            "is_enemy" => rule.is_enemy = trigger(&item.value, &what)?,
            "is_friend" => rule.is_friend = trigger(&item.value, &what)?,
            "is_neutral" => rule.is_neutral = trigger(&item.value, &what)?,
            // `icon` and `offset` place the map icon; the native reader logs
            // any other key and goes on.
            _ => {}
        }
    }
    Ok(rule)
}

/// Every `adjacency_rule` in file order. A rule the port cannot read is kept
/// refused ([`AdjacencyRule::unsupported`]); only an unparsable file fails.
pub fn parse(input: &[u8]) -> io::Result<Vec<AdjacencyRule>> {
    let document = parser::parse(input).map_err(|error| invalid(error.to_string()))?;
    let mut rules = Vec::new();
    for item in &document.items {
        if item.key.as_deref() != Some(b"adjacency_rule") {
            continue;
        }
        rules.push(match item.value.items() {
            Some(items) => rule(items),
            None => AdjacencyRule {
                name: String::new(),
                permissions: [0; 4],
                required_provinces: Vec::new(),
                is_disabled: Triggers::default(),
                is_enemy: Triggers::default(),
                is_friend: Triggers::default(),
                is_neutral: Triggers::default(),
                unsupported: Some("adjacency_rule is not a block".into()),
            },
        });
    }
    Ok(rules)
}

/// The `map/` file `default.map` names under `key` (`adjacencies` or
/// `adjacency_rules`). Both paths start empty and are set to `map/` plus
/// the field's value. An empty path opens nothing, so the game loads no
/// straits or rules. That changes every crossing, so a map without the key
/// is refused by name rather than read as empty.
pub fn map_file(map: &DefaultMap, key: &str) -> io::Result<String> {
    map.file(key).map_err(|_| {
        invalid(format!(
            "map/default.map names no {key}; the game would load none, which is not supported"
        ))
    })
}

/// The rules in file order.
pub fn read(layers: &ResolvedLayers) -> io::Result<Vec<AdjacencyRule>> {
    read_map(layers, &crate::terrain::read_default_map(layers)?)
}

/// The rules of an already read `default.map`.
pub fn read_map(layers: &ResolvedLayers, map: &DefaultMap) -> io::Result<Vec<AdjacencyRule>> {
    let path = map_file(map, "adjacency_rules")?;
    parse(
        &layers
            .read(&path)
            .map_err(|error| invalid(format!("{path}: {error}")))?,
    )
}

/// The rule an adjacency names: the first of that name.
pub fn find<'a>(rules: &'a [AdjacencyRule], name: &str) -> Option<&'a AdjacencyRule> {
    rules.iter().find(|rule| rule.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_default_to_allowed_and_keys_change_one_bit() {
        let rules = parse(
            br#"adjacency_rule = {
                name = "STRAIT"
                contested = { army = no navy = no }
                enemy = { army = no navy = no submarine = no trade = no }
                friend = { trade = no }
                required_provinces = { 3 4 }
                is_disabled = { has_country_flag = BLOCKED tooltip = blocked_tt }
                is_friend = { always = yes }
                icon = 3
                offset = { 1 0 -6 }
            }
            adjacency_rule = { name = "STRAIT" enemy = { army = yes } }"#,
        )
        .unwrap();
        let rule = find(&rules, "STRAIT").unwrap();
        assert_eq!(rule.permissions, [0x0c, 0x00, 0x07, 0x0f]);
        assert_eq!(rule.required_provinces, [3, 4]);
        assert_eq!(rule.is_disabled.len(), 1);
        assert_eq!(rule.is_friend.len(), 1);
        assert!(rule.permits(Some(Verdict::Neutral), ARMY));
        assert!(!rule.permits(Some(Verdict::Enemy), ARMY));
        assert!(!rule.permits(None, ARMY));
    }

    #[test]
    fn required_provinces_keep_signed_scan_and_zeroed_array_slots() {
        let rules =
            parse(b"adjacency_rule = { name = TEST required_provinces = { 017 0x1F invalid -1 } }")
                .unwrap();
        assert_eq!(rules[0].unsupported, None);
        assert_eq!(rules[0].required_provinces, [15, 31, 0, u32::MAX]);
    }

    #[test]
    fn an_unreadable_rule_is_refused_by_name_and_the_others_still_load() {
        let rules = parse(
            b"adjacency_rule = { name = BAD enemy = { tanks = no } }
              adjacency_rule = { name = GOOD friend = { army = no } }",
        )
        .unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "BAD");
        assert!(rules[0].unsupported.as_deref().unwrap().contains("tanks"));
        assert_eq!(rules[1].unsupported, None);
        assert_eq!(rules[1].permissions[Verdict::Friend as usize], 0x0e);
    }

    #[test]
    fn a_map_without_the_file_key_is_refused_by_name() {
        let map = crate::terrain::parse_default_map(
            b"definitions = a provinces = b terrain = c rivers = d heightmap = e tree_definition = f adjacencies = \"adjacencies.csv\"",
        )
        .unwrap();
        assert_eq!(
            map_file(&map, "adjacencies").unwrap(),
            "map/adjacencies.csv"
        );
        let error = map_file(&map, "adjacency_rules").unwrap_err().to_string();
        assert!(error.contains("names no adjacency_rules"), "{error}");
    }
}
