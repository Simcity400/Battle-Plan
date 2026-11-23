use content_reader_key_names::key_name_products::KeyNamesExt as _;

use std::io;

use content_reader_layers::ResolvedLayers;
use content_reader_parser::{crt, fields::{self, block, boolean, invalid}, parser::{Item, Kind, Value}};
use content_reader_script::{key_names::KeyNames, native::{self, Shape}, triggers::{self, Triggers}};
use content_reader_key_names::terrain::DefaultMap;

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

fn shape_mismatch(key: &[u8]) -> String {
    format!(
        "field {} has another shape than its reader takes, so the game reads the rest of the file out of step with this reader",
        key.escape_ascii()
    )
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Rule,
    Permissions,
    DisabledTrigger,
    TriggerBody,
    BoundLocalization,
    Trigger(content_reader_script::trigger_grammar::Node),
}

enum TriggerNames<'a> {
    None,
    Provided(&'a KeyNames),
    Lazy(&'a ResolvedLayers),
}

fn trigger_registry_status() -> Option<Result<(), String>> {
    let tables = engine_executable::executable_tables::current()?;
    Some(
        tables
            .registries
            .as_ref()
            .map(|_| ())
            .map_err(|withheld| withheld.to_string()),
    )
}

fn key_names_tables_status() -> Result<(), String> {
    let Some(tables) = engine_executable::executable_tables::current() else {
        return Ok(());
    };
    if let Err(withheld) = &tables.keywords {
        return Err(withheld.to_string());
    }
    if let Err(withheld) = &tables.registries {
        return Err(withheld.to_string());
    }
    let auxiliary = tables.auxiliary.as_ref().map_err(ToString::to_string)?;
    if let Some(withheld) = [
        auxiliary.category_names.as_ref().err(),
        auxiliary.stat_metadata.as_ref().err(),
        auxiliary.equipment_missions.as_ref().err(),
        auxiliary.equipment_categories.as_ref().err(),
    ]
    .into_iter()
    .flatten()
    .next()
    {
        return Err(withheld.to_string());
    }
    Ok(())
}

fn parse_inner(input: &[u8], names: TriggerNames<'_>) -> io::Result<Vec<AdjacencyRule>> {
    let lazy_names = std::cell::OnceCell::new();
    let mut interrupted: Option<io::Error> = None;
    let mut shaper: Option<content_reader_script::trigger_grammar::Shaper<'_>> = None;
    let provided = match names {
        TriggerNames::Provided(names) => Some(names),
        TriggerNames::None | TriggerNames::Lazy(_) => None,
    };
    let document = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(Node::File),
        provided,
        &mut |node, field| {
            let mut trigger_shape = |target: content_reader_script::trigger_grammar::Node| {
                let Some(registry) = trigger_registry_status() else {
                    return native::untraced(field);
                };
                if let Err(withheld) = registry {
                    return Shape::Unsettled(withheld);
                }
                match names {
                    TriggerNames::Provided(names) => shaper
                        .get_or_insert_with(|| content_reader_script::trigger_grammar::Shaper::new(Some(names)))
                        .shape(target, field)
                        .map(Node::Trigger),
                    TriggerNames::Lazy(layers) => {
                        let shaper = shaper
                            .get_or_insert_with(|| content_reader_script::trigger_grammar::Shaper::new(None));
                        if shaper.needs_names(target, field) {
                            let loaded = lazy_names.get_or_init(|| {
                                key_names_tables_status()?;
                                match layers.key_names() {
                                    Ok(names) => Ok(names),
                                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                        let reason = error.to_string();
                                        interrupted = Some(error);
                                        Err(reason)
                                    }
                                    Err(error) => Err(error.to_string()),
                                }
                            });
                            match loaded {
                                Ok(names) => shaper.set_names(names),
                                Err(why) if interrupted.is_some() => {
                                    return Shape::Unsettled(why.clone());
                                }
                                Err(_) => {}
                            }
                        }
                        match shaper.shape(target, field).map(Node::Trigger) {
                            Shape::Unsettled(reason) => match lazy_names.get() {
                                Some(Err(why)) => Shape::Unsettled(format!("{reason} ({why})")),
                                _ => Shape::Unsettled(reason),
                            },
                            other => other,
                        }
                    }
                    TriggerNames::None => shaper
                        .get_or_insert_with(|| content_reader_script::trigger_grammar::Shaper::new(None))
                        .shape(target, field)
                        .map(Node::Trigger),
                }
            };
            match node {
                Node::File => {
                    if !field.fresh {
                        return Shape::Unsettled(
                            "the adjacency rules hold an entry without a key and value, which the game's file loop reads out of step with this reader".into(),
                        );
                    }
                    if field.keyword_is("adjacency_rule") {
                        if field.opens() {
                            Shape::Object(Node::Rule)
                        } else {
                            Shape::Unsettled(
                                "an adjacency_rule is not a block: the game reads the fields after it as the rule's, which this reader cannot follow".into(),
                            )
                        }
                    } else {
                        Shape::One
                    }
                }
                Node::Rule => {
                    if !field.fresh {
                        return Shape::Unsettled(
                            "an entry without a key, so the game reads the rest of the file out of step with this reader".into(),
                        );
                    }
                    match field.keyword().as_deref() {
                        Some(b"name" | b"icon") => {
                            if field.opens() {
                                Shape::Unsettled(shape_mismatch(&field.key.text))
                            } else {
                                Shape::One
                            }
                        }
                        Some(b"contested" | b"enemy" | b"friend" | b"neutral") => {
                            if field.opens() {
                                Shape::Object(Node::Permissions)
                            } else {
                                Shape::Unsettled(shape_mismatch(&field.key.text))
                            }
                        }
                        Some(b"required_provinces" | b"offset") => {
                            if field.opens() {
                                Shape::List
                            } else {
                                Shape::Unsettled(shape_mismatch(&field.key.text))
                            }
                        }
                        Some(b"is_disabled") => {
                            if field.opens() {
                                Shape::Object(Node::DisabledTrigger)
                            } else {
                                Shape::Unsettled(shape_mismatch(&field.key.text))
                            }
                        }
                        Some(b"is_enemy" | b"is_friend" | b"is_neutral") => {
                            if field.opens() {
                                Shape::Object(Node::TriggerBody)
                            } else {
                                Shape::Unsettled(shape_mismatch(&field.key.text))
                            }
                        }
                        _ => Shape::Skip,
                    }
                }
                Node::Permissions => {
                    if !field.fresh || field.opens() {
                        Shape::Unsettled(shape_mismatch(&field.key.text))
                    } else {
                        Shape::One
                    }
                }
                Node::DisabledTrigger => {
                    if matches!(
                        field.keyword().as_deref(),
                        Some(b"tooltip" | b"not_tooltip")
                    ) {
                        Shape::ObjectIfOpen(Node::BoundLocalization)
                    } else {
                        trigger_shape(content_reader_script::trigger_grammar::Node::Trigger)
                    }
                }
                Node::TriggerBody => trigger_shape(content_reader_script::trigger_grammar::Node::Trigger),
                Node::BoundLocalization => {
                    if field.keyword_is("localization_key") {
                        Shape::One
                    } else {
                        Shape::ObjectIfOpen(Node::BoundLocalization)
                    }
                }
                Node::Trigger(node) => trigger_shape(node),
            }
        },
    );
    if let Some(error) = interrupted {
        return Err(error);
    }
    if let Some(cause) = document.unsettled {
        return Err(invalid(cause.to_string()));
    }
    let mut rules = Vec::new();
    for item in &document.items {
        if !item.keyword_is("adjacency_rule") {
            continue;
        }
        let Some(fields) = item.value.items() else {
            return Err(invalid(
                "an adjacency_rule is not a block: the game reads the fields after it as the rule's, which this reader cannot follow",
            ));
        };
        for field in fields {
            if (field.keyword_is("required_provinces") || field.keyword_is("offset"))
                && field
                    .value
                    .items()
                    .is_some_and(|entries| entries.iter().any(|entry| entry.kind == Kind::Open))
            {
                return Err(invalid(format!(
                    "adjacency_rule {}: {}",
                    last(fields, "name")
                        .and_then(Value::scalar)
                        .unwrap_or_default()
                        .escape_ascii(),
                    shape_mismatch(field.key.as_deref().unwrap_or_default())
                )));
            }
        }
        rules.push(rule(fields));
    }
    Ok(rules)
}

pub fn parse(input: &[u8]) -> io::Result<Vec<AdjacencyRule>> {
    parse_inner(input, TriggerNames::None)
}

pub fn parse_with_names(input: &[u8], names: &KeyNames) -> io::Result<Vec<AdjacencyRule>> {
    parse_inner(input, TriggerNames::Provided(names))
}

pub fn map_file(map: &DefaultMap, key: &str) -> io::Result<String> {
    map.file(key).map_err(|_| {
        invalid(format!(
            "map/default.map names no {key}; the game would load none, which is not supported"
        ))
    })
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Vec<AdjacencyRule>> {
    read_map(layers, &content_reader_key_names::terrain::read_default_map(layers)?)
}

pub fn read_map(layers: &ResolvedLayers, map: &DefaultMap) -> io::Result<Vec<AdjacencyRule>> {
    let path = map_file(map, "adjacency_rules")?;
    let bytes = layers
        .read(&path)
        .map_err(|error| invalid(format!("{path}: {error}")))?;
    parse_inner(&bytes, TriggerNames::Lazy(layers)).map_err(|error| {
        if error.kind() == io::ErrorKind::Interrupted {
            error
        } else {
            invalid(format!("{path}: {error}"))
        }
    })
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
        let replayed =
            parse(b"@army = no adjacency_rule = { name = A enemy = { army = @army } }").unwrap();
        assert_eq!(replayed[0].unsupported, None);
        assert_eq!(replayed[0].permissions[Verdict::Enemy as usize], 0x0e);
        for (text, why) in [
            (
                &b"@open = { adjacency_rule = { name = A icon = @open } }"[..],
                "field icon has another shape",
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
            (
                b"adjacency_rule = { name = A required_provinces = { { 1 } } }",
                "field required_provinces has another shape",
            ),
            (
                b"adjacency_rule = { name = A is_enemy = { unknown_scope = { @x = 1 always = yes } } icon = @x }",
                "unknown_scope",
            ),
            (b"adjacency_rule = A", "not a block"),
        ] {
            let error = parse(text).unwrap_err().to_string();
            assert!(error.contains(why), "{error}");
        }
    }

    #[test]
    fn a_map_without_the_file_key_is_refused_by_name() {
        let map = content_reader_key_names::terrain::parse_default_map(
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

    #[cfg(feature = "test-support")]
    #[test]
    fn variable_backed_modded_strait_reads_through_native_stream() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("adjacency_rules_variables");
        stack.write(
            &stack.base(),
            "map/default.map",
            "definitions = d.csv provinces = p.bmp terrain = t.bmp rivers = r.bmp heightmap = h.bmp tree_definition = tr.bmp adjacencies = adjacencies.csv adjacency_rules = adjacency_rules.txt",
        );
        stack.write(
            &stack.addon(),
            "map/adjacency_rules.txt",
            r#"@strait_name = "MOD_STRAIT"
@blocked = no
@allowed = yes
@prov_a = 101
@prov_b = 102
@icon_prov = 101
@ox = -12.5
@oy = 0.0
@oz = 4.25
@flag = MOD_STRAIT_CLOSED
@tt = MOD_STRAIT_CLOSED_TT
adjacency_rule = {
    name = @strait_name
    contested = { army = @blocked navy = @blocked }
    enemy = { army = @blocked navy = @blocked submarine = @blocked trade = @blocked }
    friend = { army = @allowed navy = @allowed submarine = @allowed trade = @allowed }
    neutral = { army = @allowed navy = @blocked }
    required_provinces = { @prov_a @prov_b }
    is_disabled = { has_country_flag = @flag tooltip = @tt }
    is_friend = { always = @allowed }
    icon = @icon_prov
    offset = { @ox @oy @oz }
}
adjacency_rule = {
    name = "BAD_MOD_STRAIT"
    enemy = { tanks = @blocked }
    icon = @icon_prov
    offset = { @ox @oy @oz }
}
adjacency_rule = {
    name = "MISSING_VAR_STRAIT"
    enemy = { army = @missing_perm }
    icon = @icon_prov
}"#,
        );
        let rules = read(&stack.layers(&[])).unwrap();
        let strait = find(&rules, "MOD_STRAIT").unwrap();
        assert_eq!(strait.unsupported, None);
        assert_eq!(strait.permissions, [0x0c, 0x00, 0x0f, 0x0d]);
        assert_eq!(strait.required_provinces, [101, 102]);
        assert_eq!(strait.is_disabled.len(), 1);
        assert_eq!(
            strait.is_disabled[0].value.scalar(),
            Some(&b"MOD_STRAIT_CLOSED"[..])
        );
        assert_eq!(
            strait.disabled_reason.as_deref(),
            Ok(&b"MOD_STRAIT_CLOSED_TT_NOT"[..])
        );
        assert_eq!(strait.is_friend.len(), 1);
        assert_eq!(strait.is_friend[0].value.scalar(), Some(&b"yes"[..]));
        assert_eq!(
            strait.icon,
            Ok(Some(IconPlacement {
                province: 101,
                offset: [-12.5, 0.0, 4.25]
            }))
        );
        let bad = find(&rules, "BAD_MOD_STRAIT").unwrap();
        assert!(bad.unsupported.as_deref().unwrap().contains("tanks"));
        assert_eq!(
            bad.icon,
            Ok(Some(IconPlacement {
                province: 101,
                offset: [-12.5, 0.0, 4.25]
            }))
        );
        let missing = find(&rules, "MISSING_VAR_STRAIT").unwrap();
        assert!(
            missing
                .unsupported
                .as_deref()
                .unwrap()
                .contains("@missing_perm")
        );
        assert_eq!(
            missing.icon,
            Ok(Some(IconPlacement {
                province: 101,
                offset: [0.0; 3]
            }))
        );
    }

    #[test]
    fn known_trigger_body_replays_definitions_in_plain_parse_while_unknown_children_refuse() {
        let rules =
            parse(b"adjacency_rule = { name = A is_enemy = { @x = 1 always = yes } icon = @x }")
                .unwrap();
        assert_eq!(rules[0].unsupported, None);
        assert_eq!(
            rules[0].icon,
            Ok(Some(IconPlacement {
                province: 1,
                offset: [0.0; 3]
            }))
        );
        let refused = parse(
            b"adjacency_rule = { name = A is_enemy = { unknown_scope = { @x = 1 always = yes } } icon = @x }",
        )
        .unwrap_err()
        .to_string();
        assert!(
            refused.contains("unknown_scope") || refused.contains("@x"),
            "{refused}"
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn read_map_succeeds_without_key_names_when_no_trigger_needs_them() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("adjacency_rules_lazy_key_names");
        stack.write(
            &stack.base(),
            "map/default.map",
            "definitions = d.csv provinces = p.bmp terrain = t.bmp rivers = r.bmp heightmap = h.bmp tree_definition = tr.bmp adjacencies = adjacencies.csv adjacency_rules = adjacency_rules.txt",
        );
        stack.write(
            &stack.base(),
            "map/adjacency_rules.txt",
            "adjacency_rule = { name = A icon = 7 }",
        );
        stack.write(&stack.base(), "common/country_tags/broken.txt", "AAA = x\n");
        let layers = stack.layers(&[]);
        std::fs::remove_file(stack.base().join("common/country_tags/broken.txt")).unwrap();
        let rules = read(&layers).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "A");
        assert_eq!(
            rules[0].icon,
            Ok(Some(IconPlacement {
                province: 7,
                offset: [0.0; 3]
            }))
        );

        let needs = content_reader_layers::test_support::Stack::new("adjacency_rules_needs_key_names");
        needs.write(
            &needs.base(),
            "map/default.map",
            "definitions = d.csv provinces = p.bmp terrain = t.bmp rivers = r.bmp heightmap = h.bmp tree_definition = tr.bmp adjacencies = adjacencies.csv adjacency_rules = adjacency_rules.txt",
        );
        needs.write(
            &needs.base(),
            "map/adjacency_rules.txt",
            "adjacency_rule = { name = B is_enemy = { custom_scope = { always = yes } } }",
        );
        needs.write(&needs.base(), "common/country_tags/broken.txt", "AAA = x\n");
        let layers = needs.layers(&[]);
        std::fs::remove_file(needs.base().join("common/country_tags/broken.txt")).unwrap();
        let error = read(&layers).unwrap_err().to_string();
        assert!(error.contains("custom_scope"), "{error}");
    }

    #[test]
    fn withheld_executable_registries_refuse_trigger_dispatch_without_panicking() {
        content_reader_script::test_game::with(|game| {
            game.withhold(
                synthetic_game::Part::Registries,
                "fixture withheld registries",
            )
        });
        let error = parse(b"adjacency_rule = { name = A is_enemy = { always = yes } }")
            .unwrap_err()
            .to_string();
        assert!(error.contains("fixture withheld registries"), "{error}");
    }

    #[test]
    fn parse_with_names_without_executable_tables_does_not_panic() {
        engine_executable::executable_tables::enter(None);
        let rules = parse_with_names(
            b"adjacency_rule = { name = A is_enemy = { always = yes } }",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "A");
        assert_eq!(rules[0].is_enemy.len(), 1);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn withheld_auxiliary_tables_read_registered_triggers_and_refuse_dynamic_without_panicking() {
        content_reader_script::test_game::with(|game| {
            game.withhold(
                synthetic_game::Part::Auxiliary,
                "fixture withheld auxiliary",
            )
        });
        let stack = content_reader_layers::test_support::Stack::new("adjacency_rules_withheld_auxiliary_ok");
        stack.write(
            &stack.base(),
            "common/units/fixture.txt",
            "sub_units = { infantry = { type = infantry } }",
        );
        stack.write(
            &stack.base(),
            "map/default.map",
            "definitions = d.csv provinces = p.bmp terrain = t.bmp rivers = r.bmp heightmap = h.bmp tree_definition = tr.bmp adjacencies = adjacencies.csv adjacency_rules = adjacency_rules.txt",
        );
        stack.write(
            &stack.base(),
            "map/adjacency_rules.txt",
            "adjacency_rule = { name = A is_enemy = { always = yes } }",
        );
        let layers = stack.layers(&[]);
        let rules = read(&layers).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "A");
        assert_eq!(rules[0].is_enemy.len(), 1);

        let dynamic = content_reader_layers::test_support::Stack::new("adjacency_rules_withheld_auxiliary_dynamic");
        dynamic.write(
            &dynamic.base(),
            "common/units/fixture.txt",
            "sub_units = { infantry = { type = infantry } }",
        );
        dynamic.write(
            &dynamic.base(),
            "map/default.map",
            "definitions = d.csv provinces = p.bmp terrain = t.bmp rivers = r.bmp heightmap = h.bmp tree_definition = tr.bmp adjacencies = adjacencies.csv adjacency_rules = adjacency_rules.txt",
        );
        dynamic.write(
            &dynamic.base(),
            "map/adjacency_rules.txt",
            "adjacency_rule = { name = B is_enemy = { custom_scope = { always = yes } } }",
        );
        let layers = dynamic.layers(&[]);
        let error = read(&layers).unwrap_err().to_string();
        assert!(error.contains("fixture withheld auxiliary"), "{error}");

        let error = parse_with_names(
            b"adjacency_rule = { name = C is_enemy = { custom_scope = { always = yes } } }",
            &KeyNames::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("fixture withheld auxiliary"), "{error}");
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn read_map_loads_key_names_for_own_keys_that_need_buildings() {
        content_reader_script::trigger_grammar::own::register_non_damaged_building_level();
        let stack = content_reader_layers::test_support::Stack::new("adjacency_rules_own_key_building");
        stack.write(
            &stack.base(),
            "common/buildings/00_buildings.txt",
            "buildings = { factory = { level_cap = { state_max = 10 } } }",
        );
        stack.write(
            &stack.base(),
            "map/default.map",
            "definitions = d.csv provinces = p.bmp terrain = t.bmp rivers = r.bmp heightmap = h.bmp tree_definition = tr.bmp adjacencies = adjacencies.csv adjacency_rules = adjacency_rules.txt",
        );
        stack.write(
            &stack.base(),
            "map/adjacency_rules.txt",
            "adjacency_rule = { name = A is_enemy = { non_damaged_building_level = { building = factory level > 2 } } }",
        );
        let layers = stack.layers(&[]);
        let rules = read(&layers).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "A");
        assert_eq!(rules[0].is_enemy.len(), 1);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn withheld_auxiliary_tables_read_numeric_state_scope_without_catalogue() {
        content_reader_script::test_game::with(|game| {
            game.withhold(
                synthetic_game::Part::Auxiliary,
                "fixture withheld auxiliary",
            )
        });
        let stack = content_reader_layers::test_support::Stack::new("adjacency_rules_withheld_auxiliary_state");
        stack.write(
            &stack.base(),
            "common/units/fixture.txt",
            "sub_units = { infantry = { type = infantry } }",
        );
        stack.write(
            &stack.base(),
            "map/default.map",
            "definitions = d.csv provinces = p.bmp terrain = t.bmp rivers = r.bmp heightmap = h.bmp tree_definition = tr.bmp adjacencies = adjacencies.csv adjacency_rules = adjacency_rules.txt",
        );
        stack.write(
            &stack.base(),
            "map/adjacency_rules.txt",
            "adjacency_rule = { name = A is_enemy = { 12 = { always = yes } } }",
        );
        let layers = stack.layers(&[]);
        let rules = read(&layers).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "A");
        assert_eq!(rules[0].is_enemy.len(), 1);
    }

    #[test]
    fn embedded_end_marker_in_keyed_unknown_block_refuses_before_accepting_icon() {
        engine_executable::executable_tables::enter(None);
        let error = parse(
            b"adjacency_rule = { name = A is_enemy = { opaque = { k = 1\xff } } @x = 7 icon = @x }",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("opaque"), "{error}");
    }

    #[test]
    fn embedded_end_marker_in_nested_unknown_object_refuses_before_accepting_icon() {
        const INPUT: &[u8] = b"adjacency_rule = { name = A is_enemy = { owns_any_state_of = { nested = { \xff } } @x = 7 icon = @x } }";
        content_reader_script::trigger_grammar::own::register_owns_any_state_of();
        let rules = parse(INPUT).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "A");
        assert_eq!(rules[0].icon, Ok(None));

        engine_executable::executable_tables::enter(None);
        let error = parse(INPUT).unwrap_err().to_string();
        assert!(error.contains("nested"), "{error}");
    }
}
