
use std::io;

use crate::{
    ResolvedLayers, crt,
    fields::{self, block, boolean, invalid},
    parser::{self, Item, Value},
    terrain::DefaultMap,
    triggers::{self, Triggers},
};

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconPlacement {
    pub province: u32,
    pub offset: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdjacencyRule {
    pub name: String,
    pub(crate) identity: Vec<u8>,
    pub permissions: [u8; 4],
    pub required_provinces: Vec<u32>,
    pub is_disabled: Triggers,
    pub disabled_reason: Result<Vec<u8>, String>,
    pub is_enemy: Triggers,
    pub is_friend: Triggers,
    pub is_neutral: Triggers,
    pub unsupported: Option<String>,
    pub icon: Result<Option<IconPlacement>, String>,
}

impl AdjacencyRule {
    pub fn permits(&self, verdict: Option<Verdict>, bit: u8) -> bool {
        verdict.is_some_and(|verdict| self.permissions[verdict as usize] & bit != 0)
    }
}

fn last<'a>(items: &'a [Item], key: &str) -> Option<&'a Value> {
    items
        .iter()
        .rev()
        .find(|item| item.keyword_is(key))
        .map(|item| &item.value)
}

fn permissions(value: &Value, what: &str) -> io::Result<u8> {
    let mut bits = 0x0f;
    for item in block(value, what)? {
        let bit = match item.keyword().map(|key| key.folded()).as_deref() {
            Some(b"army") => ARMY,
            Some(b"navy") => NAVY,
            Some(b"submarine") => SUBMARINE,
            Some(b"trade") => TRADE,
            _ => {
                return Err(invalid(format!(
                    "{what} has unknown permission {}",
                    item.key.as_deref().unwrap_or_default().escape_ascii()
                )));
            }
        };
        if boolean(&item.value, what)? {
            bits |= bit;
        } else {
            bits &= !bit;
        }
    }
    Ok(bits)
}

fn trigger(value: &Value, what: &str) -> io::Result<Triggers> {
    triggers::body(value).ok_or_else(|| invalid(format!("{what} is not a block")))
}

fn rule(items: &[Item]) -> AdjacencyRule {
    let identity = last(items, "name")
        .and_then(Value::read_string)
        .unwrap_or_default()
        .to_vec();
    let (name, unheld) = match std::str::from_utf8(&identity) {
        Ok(name) => (name.to_owned(), None),
        Err(_) => (
            identity.escape_ascii().to_string(),
            Some(format!(
                "its name {} is not UTF-8, which the port's text cannot hold",
                identity.escape_ascii()
            )),
        ),
    };
    let icon = icon(items, &name).map_err(|error| error.to_string());
    let read = match unheld {
        Some(why) => Err(why),
        None => read_rule(items, &name).map_err(|error| error.to_string()),
    };
    let mut rule = read.unwrap_or_else(|why| AdjacencyRule {
        name: String::new(),
        identity: Vec::new(),
        permissions: [0; 4],
        required_provinces: Vec::new(),
        is_disabled: Triggers::default(),
        disabled_reason: Ok(Vec::new()),
        is_enemy: Triggers::default(),
        is_friend: Triggers::default(),
        is_neutral: Triggers::default(),
        unsupported: Some(why),
        icon: Ok(None),
    });
    rule.name = name;
    rule.identity = identity;
    rule.icon = icon;
    rule
}

fn icon(items: &[Item], name: &str) -> io::Result<Option<IconPlacement>> {
    let what = |key: &str| format!("adjacency_rule {name:?} {key}");
    let Some(province) = last(items, "icon") else {
        return Ok(None);
    };
    let scalar = province
        .scalar()
        .ok_or_else(|| invalid(format!("{} is not a scalar", what("icon"))))?;
    let province = crt::scan_i32(scalar)
        .stored()
        .and_then(|province| u32::try_from(province).ok())
        .ok_or_else(|| invalid(format!("{} is not a province id", what("icon"))))?;
    let offset = match last(items, "offset") {
        None => [0.0; 3],
        Some(value) => {
            let what = what("offset");
            let numbers = block(value, &what)?
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid(format!("{what} holds a keyed entry")));
                    }
                    fields::number(&item.value, &what)
                })
                .collect::<io::Result<Vec<f32>>>()?;
            <[f32; 3]>::try_from(numbers.as_slice())
                .map_err(|_| invalid(format!("{what} holds {} numbers, not 3", numbers.len())))?
        }
    };
    Ok(Some(IconPlacement { province, offset }))
}

fn read_rule(items: &[Item], name: &str) -> io::Result<AdjacencyRule> {
    let mut rule = AdjacencyRule {
        name: name.to_owned(),
        identity: Vec::new(),
        permissions: [0x0f; 4],
        required_provinces: Vec::new(),
        is_disabled: Triggers::default(),
        disabled_reason: Ok(Vec::new()),
        is_enemy: Triggers::default(),
        is_friend: Triggers::default(),
        is_neutral: Triggers::default(),
        unsupported: None,
        icon: Ok(None),
    };
    for item in items {
        let Some(key) = item.keyword().map(|key| key.folded()) else {
            continue;
        };
        let what = format!("adjacency_rule {name:?} {}", key.escape_ascii());
        match key.as_slice() {
            b"contested" => rule.permissions[0] = permissions(&item.value, &what)?,
            b"enemy" => rule.permissions[1] = permissions(&item.value, &what)?,
            b"friend" => rule.permissions[2] = permissions(&item.value, &what)?,
            b"neutral" => rule.permissions[3] = permissions(&item.value, &what)?,
            b"required_provinces" => {
                rule.required_provinces = fields::scalar_list(&item.value, &what)?;
            }
            b"is_disabled" => {
                let mut triggers = trigger(&item.value, &what)?;
                rule.disabled_reason = disabled_reason(&triggers, &what);
                triggers.retain(|trigger| !trigger.keyword_is("tooltip") && !trigger.keyword_is("not_tooltip"));
                rule.is_disabled = triggers;
            }
            b"is_enemy" => rule.is_enemy = trigger(&item.value, &what)?,
            b"is_friend" => rule.is_friend = trigger(&item.value, &what)?,
            b"is_neutral" => rule.is_neutral = trigger(&item.value, &what)?,
            _ => {}
        }
    }
    Ok(rule)
}

fn disabled_reason(items: &[Item], what: &str) -> Result<Vec<u8>, String> {
    let key = |name: &str| -> Result<Option<Vec<u8>>, String> {
        let Some(value) = last(items, name) else {
            return Ok(None);
        };
        value
            .read_string()
            .map(|key| Some(key.to_vec()))
            .ok_or_else(|| format!("{what} {name} is not a plain localisation key, which the port does not read"))
    };
    if let Some(key) = key("not_tooltip")? {
        return Ok(key);
    }
    Ok(match key("tooltip")? {
        Some(mut key) if !key.is_empty() => {
            key.extend_from_slice(b"_NOT");
            key
        }
        _ => Vec::new(),
    })
}

fn variable(items: &[Item]) -> Option<&[u8]> {
    items.iter().find_map(|item| {
        if let Some(key) = item.key.as_deref()
            && !item.key_quoted
            && key.starts_with(b"@")
        {
            return Some(key);
        }
        match &item.value {
            Value::Scalar(text) if !item.quoted() && text.starts_with(b"@") => {
                Some(text.as_slice())
            }
            value => variable(value.items().unwrap_or_default()),
        }
    })
}

fn out_of_step(items: &[Item]) -> Option<String> {
    items.iter().find_map(|item| {
        let Some(key) = item.key.as_deref() else {
            return Some("an entry without a key".to_owned());
        };
        let entries = match &item.value {
            Value::Block(entries) => Some(entries.as_slice()),
            _ => None,
        };
        let fits = match item.keyword().map(|key| key.folded()).as_deref() {
            Some(b"name" | b"icon") => item.value.scalar().is_some(),
            Some(b"contested" | b"enemy" | b"friend" | b"neutral") => {
                entries.is_some_and(|entries| {
                    entries
                        .iter()
                        .all(|entry| entry.key.is_some() && entry.value.scalar().is_some())
                })
            }
            Some(b"required_provinces" | b"offset") => entries
                .is_some_and(|entries| entries.iter().all(|entry| entry.value.scalar().is_some())),
            Some(b"is_disabled" | b"is_enemy" | b"is_friend" | b"is_neutral") => {
                entries.is_some()
            }
            _ => true,
        };
        (!fits).then(|| {
            format!(
                "field {} has another shape than its reader takes",
                key.escape_ascii()
            )
        })
    })
}

fn read_loop(items: &[Item], rules: &mut Vec<AdjacencyRule>) -> io::Result<()> {
    for item in items {
        if item.key.is_none() || matches!(item.value, Value::Tagged { .. }) {
            return Err(invalid(
                "the adjacency rules hold an entry without a key and value, which the game's file loop reads out of step with this reader",
            ));
        }
        if item.keyword_is("adjacency_rule") {
            let Some(fields) = item.value.items() else {
                return Err(invalid(
                    "an adjacency_rule is not a block: the game reads the fields after it as the rule's, which this reader cannot follow",
                ));
            };
            if let Some(why) = out_of_step(fields) {
                return Err(invalid(format!(
                    "adjacency_rule {}: {why}, so the game reads the rest of the file out of step with this reader",
                    last(fields, "name")
                        .and_then(Value::scalar)
                        .unwrap_or_default()
                        .escape_ascii()
                )));
            }
            rules.push(rule(fields));
        } else if let Some(fields) = item.value.items() {
            return read_loop(fields, rules);
        }
    }
    Ok(())
}

pub fn parse(input: &[u8]) -> io::Result<Vec<AdjacencyRule>> {
    let document = parser::parse(input).map_err(|error| invalid(error.to_string()))?;
    if let Some(variable) = variable(&document.items) {
        return Err(invalid(format!(
            "the adjacency rules use the script variable {}, which this reader does not replay",
            variable.escape_ascii()
        )));
    }
    let mut rules = Vec::new();
    read_loop(&document.items, &mut rules)?;
    Ok(rules)
}

pub fn map_file(map: &DefaultMap, key: &str) -> io::Result<String> {
    map.file(key).map_err(|_| {
        invalid(format!(
            "map/default.map names no {key}; the game would load none, which is not supported"
        ))
    })
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Vec<AdjacencyRule>> {
    read_map(layers, &crate::terrain::read_default_map(layers)?)
}

pub fn read_map(layers: &ResolvedLayers, map: &DefaultMap) -> io::Result<Vec<AdjacencyRule>> {
    let path = map_file(map, "adjacency_rules")?;
    parse(
        &layers
            .read(&path)
            .map_err(|error| invalid(format!("{path}: {error}")))?,
    )
}

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
        assert_eq!(rule.disabled_reason.as_deref(), Ok(&b"blocked_tt_NOT"[..]));
        assert_eq!(rule.is_friend.len(), 1);
        let reason = |body: &str| {
            parse(format!("adjacency_rule = {{ name = R is_disabled = {{ {body} }} }}").as_bytes()).unwrap()[0].clone()
        };
        let named = reason("tooltip = a not_tooltip = b always = yes");
        assert_eq!(named.disabled_reason.as_deref(), Ok(&b"b"[..]));
        assert_eq!(named.is_disabled.len(), 1);
        assert_eq!(reason("always = yes").disabled_reason.as_deref(), Ok(&b""[..]));
        assert!(reason("tooltip = { localization_key = a } always = yes").disabled_reason.is_err());
        assert!(rule.permits(Some(Verdict::Neutral), ARMY));
        assert!(!rule.permits(Some(Verdict::Enemy), ARMY));
        assert!(!rule.permits(None, ARMY));
        assert_eq!(
            rule.icon,
            Ok(Some(IconPlacement {
                province: 3,
                offset: [1.0, 0.0, -6.0]
            }))
        );
        assert_eq!(rules[1].icon, Ok(None));
    }

    #[test]
    fn the_icon_keeps_its_last_assignment_and_survives_a_refused_rule() {
        let rules = parse(
            b"adjacency_rule = { name = A icon = 7 icon = 9 offset = { -31.0 0.0 -2.5 } }
              adjacency_rule = { name = B enemy = { tanks = no } icon = 4 }
              adjacency_rule = { name = C icon = 5 offset = { 1 2 } }
              adjacency_rule = { name = D icon = -3 }",
        )
        .unwrap();
        assert_eq!(
            rules[0].icon,
            Ok(Some(IconPlacement {
                province: 9,
                offset: [-31.0, 0.0, -2.5]
            }))
        );
        assert!(rules[1].unsupported.is_some());
        assert_eq!(
            rules[1].icon,
            Ok(Some(IconPlacement {
                province: 4,
                offset: [0.0; 3]
            }))
        );
        let short = rules[2].icon.as_ref().unwrap_err();
        assert!(short.contains("\"C\" offset holds 2 numbers"), "{short}");
        let negative = rules[3].icon.as_ref().unwrap_err();
        assert!(negative.contains("icon is not a province id"), "{negative}");
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
    fn rule_keys_dispatch_as_case_folded_keywords() {
        let rules = parse(
            br#"ADJACENCY_RULE = { NAME = canal ENEMY = { ARMY = no } Contested = { Navy = no } "friend" = { army = no } "name" = other }
                "adjacency_rule" = { name = quoted }"#,
        )
        .unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "canal");
        assert_eq!(rules[0].unsupported, None);
        assert_eq!(rules[0].permissions, [0x0d, 0x0e, 0x0f, 0x0f]);
        let quoted = parse(br#"adjacency_rule = { name = q enemy = { "army" = no } }"#).unwrap();
        let refused = quoted[0].unsupported.as_deref().unwrap();
        assert!(refused.contains("unknown permission"), "{refused}");
    }

    #[test]
    fn a_block_under_another_key_is_read_as_the_file_loop() {
        let rules =
            parse(b"wrapper = { adjacency_rule = { name = inner } } adjacency_rule = { name = after }")
                .unwrap();
        let names: Vec<_> = rules.iter().map(|rule| rule.name.as_str()).collect();
        assert_eq!(names, ["inner"]);
    }

    #[test]
    fn readings_the_tree_cannot_follow_refuse_the_file() {
        for (text, why) in [
            (
                &b"@army = no adjacency_rule = { name = A enemy = { army = @army } }"[..],
                "script variable @army",
            ),
            (
                b"adjacency_rule = { name = A enemy = no } adjacency_rule = { name = B }",
                "field enemy has another shape",
            ),
            (
                b"adjacency_rule = { name = A icon = { 1 } }",
                "field icon has another shape",
            ),
            (
                b"adjacency_rule = { name = { A } }",
                "field name has another shape",
            ),
            (b"adjacency_rule = A", "not a block"),
        ] {
            let error = parse(text).unwrap_err().to_string();
            assert!(error.contains(why), "{error}");
        }
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
