use crate::key_name_products::KeyNamesExt as _;

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_script::{trigger_grammar::Node as TriggerNode, key_names::KeyNames, native::{self, Field, Shape}, triggers::Triggers};
use content_reader_parser::parser::Item;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TagAlias {
    pub original_tag: Option<Vec<u8>>,
    pub original_tag_assignments: usize,
    pub targets: Vec<Vec<u8>>,
    pub variable: Option<(VariableKey, Vec<u8>)>,
    pub fallback: Option<Vec<u8>>,
    pub limit: Triggers,
    pub score: Option<Triggers>,
    pub unported: Vec<String>,
    pub targets_read: bool,
    pub refused: Option<String>,
}

impl TagAlias {
    pub fn checks_all(&self) -> bool {
        self.original_tag.is_none() && !self.targets_read && self.variable.is_none()
    }

    pub fn scored(&self) -> Option<&Triggers> {
        self.score
            .as_ref()
            .filter(|fields| fields.iter().any(|field| field.keyword_is("modifier")))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VariableKey {
    Variable,
    EventTarget,
    GlobalEventTarget,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagAliases {
    pub entries: BTreeMap<Vec<u8>, (String, TagAlias)>,
    pub unknown: Vec<String>,
}

impl TagAliases {
    pub fn get(&self, text: &str) -> Option<&TagAlias> {
        self.entries
            .get(&text.as_bytes().to_ascii_lowercase())
            .map(|(_, alias)| alias)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    Alias,
    Trigger(TriggerNode),
}

fn alias_field(shaper: &mut content_reader_script::trigger_grammar::Shaper<'_>, field: &Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        Some(b"targets") => Shape::List,
        Some(
            b"target_array"
            | b"state"
            | b"country"
            | b"targets_dynamic"
            | b"target_non_existing"
            | b"state_target"
            | b"variable"
            | b"fallback"
            | b"original_tag"
            | b"event_target"
            | b"global_event_target",
        ) => Shape::One,
        Some(b"target_trigger" | b"target_root_trigger") => {
            Shape::Body(Node::Trigger(TriggerNode::Trigger))
        }
        Some(b"country_score") => Shape::Object(Node::Trigger(TriggerNode::Mtth)),
        _ => shaper.shape(TriggerNode::Trigger, field).map(Node::Trigger),
    }
}

fn read_file(
    input: &[u8],
    earlier: &BTreeMap<Vec<u8>, (String, TagAlias)>,
    key_names: &KeyNames,
) -> (Vec<(String, TagAlias)>, Option<String>) {
    let names = name_pass(input, &|name| earlier.contains_key(name));
    let mut seen = BTreeSet::new();
    let mut shaper = content_reader_script::trigger_grammar::Shaper::new(Some(key_names));
    let bodies = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        Some(key_names),
        &mut |node, field| match node {
            Node::Top if duplicate(&mut seen, field, &|name| earlier.contains_key(name)) => {
                Shape::Skip
            }
            Node::Top => Shape::Object(Node::Alias),
            Node::Alias => alias_field(&mut shaper, field),
            Node::Trigger(node) => shaper.shape(node, field).map(Node::Trigger),
        },
    );
    let unsettled = bodies.unsettled.map(|unsettled| unsettled.to_string());
    let complete = match unsettled {
        Some(_) => bodies.items.len().saturating_sub(1),
        None => bodies.items.len(),
    };
    let mut read: BTreeMap<Vec<u8>, TagAlias> = BTreeMap::new();
    for (index, body) in bodies.items.iter().enumerate() {
        let alias = match &unsettled {
            Some(why) if index >= complete => TagAlias {
                refused: Some(format!(
                    "the body pass cannot be followed through its body: {why}"
                )),
                ..TagAlias::default()
            },
            _ => alias(body),
        };
        let key = body.key.as_deref().unwrap_or_default().to_ascii_lowercase();
        read.entry(key).or_insert(alias);
    }
    let mut aliases = Vec::new();
    let mut stop = None;
    for item in &names.items {
        let name = match item.key_str() {
            Ok(name) => name.to_owned(),
            Err(refused) => {
                stop.get_or_insert_with(|| format!("an alias name is {refused}"));
                continue;
            }
        };
        let folded = name.as_bytes().to_ascii_lowercase();
        let alias = match (read.get(&folded), &unsettled) {
            (Some(alias), _) => alias.clone(),
            (None, Some(why)) => TagAlias {
                refused: Some(format!("the body pass stops before its body: {why}")),
                ..TagAlias::default()
            },
            (None, None) => TagAlias::default(),
        };
        aliases.push((name, alias));
    }
    (aliases, stop)
}

fn duplicate(
    seen: &mut BTreeSet<Vec<u8>>,
    field: &Field<'_>,
    earlier: &dyn Fn(&[u8]) -> bool,
) -> bool {
    let name = field.key.text.to_ascii_lowercase();
    earlier(&name) || !seen.insert(name)
}

fn name_pass(input: &[u8], earlier: &dyn Fn(&[u8]) -> bool) -> native::Read {
    let mut seen = BTreeSet::new();
    native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(()),
        None,
        &mut |(), field| {
            if duplicate(&mut seen, field, earlier) {
                Shape::Skip
            } else {
                Shape::SkipIfOpen
            }
        },
    )
}

fn alias(item: &Item) -> TagAlias {
    let mut out = TagAlias::default();
    let text = |item: &Item| item.value.scalar().unwrap_or_default().to_vec();
    for field in item.value.items().unwrap_or_default() {
        let Some(keyword) = field.keyword() else {
            out.limit.push(field.clone());
            continue;
        };
        match keyword.folded().as_slice() {
            b"targets" => {
                out.targets_read = true;
                out.targets
                    .extend(field.value.items().unwrap_or_default().iter().map(text));
            }
            b"state" | b"country" => {
                out.targets_read = true;
                out.targets.push(text(field));
            }
            name @ (b"target_array"
            | b"target_trigger"
            | b"target_root_trigger"
            | b"targets_dynamic"
            | b"target_non_existing"
            | b"state_target") => {
                out.targets_read = true;
                out.unported
                    .push(String::from_utf8_lossy(name).into_owned());
            }
            b"variable" => out.variable = Some((VariableKey::Variable, text(field))),
            b"event_target" => out.variable = Some((VariableKey::EventTarget, text(field))),
            b"global_event_target" => {
                out.variable = Some((VariableKey::GlobalEventTarget, text(field)));
            }
            b"fallback" => out.fallback = Some(text(field)),
            b"original_tag" => {
                out.original_tag = Some(text(field));
                out.original_tag_assignments += 1;
            }
            b"country_score" => out
                .score
                .get_or_insert_with(Vec::new)
                .extend(field.value.items().unwrap_or_default().iter().cloned()),
            _ => out.limit.push(field.clone()),
        }
    }
    out
}

const FOLDER: &str = "common/country_tag_aliases/";

pub fn names(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut names = BTreeSet::new();
    for path in script_files::paths(layers, FOLDER, Some(".txt"))? {
        let read = name_pass(&path.read(layers)?, &|name| names.contains(name));
        names.extend(
            read.items
                .iter()
                .map(|item| item.key.as_deref().unwrap_or_default().to_ascii_lowercase()),
        );
    }
    Ok(names)
}

pub fn read(layers: &ResolvedLayers) -> io::Result<TagAliases> {
    layers.recorded_product("tag aliases", 1, &[], read_aliases).map(|aliases| aliases.as_ref().clone())
}

fn read_aliases(layers: &ResolvedLayers) -> io::Result<TagAliases> {
    let mut files = Vec::new();
    for path in script_files::paths(layers, FOLDER, Some(".txt"))? {
        files.push((path.to_string(), path.read(layers)?));
    }
    Ok(parse(
        &files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>(),
        &*layers.key_names()?,
    ))
}

pub fn parse(files: &[(&str, &[u8])], names: &KeyNames) -> TagAliases {
    let mut aliases = TagAliases::default();
    for (path, bytes) in files {
        let (read, stop) = read_file(bytes, &aliases.entries, names);
        for (name, alias) in read {
            aliases
                .entries
                .entry(name.as_bytes().to_ascii_lowercase())
                .or_insert((name, alias));
        }
        if let Some(cause) = stop {
            aliases.unknown.push(format!("{path}: {cause}"));
        }
    }
    aliases
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(aliases: &TagAliases) -> Vec<&str> {
        aliases
            .entries
            .values()
            .map(|(name, _)| name.as_str())
            .collect()
    }

    #[test]
    fn every_top_level_key_is_an_alias() {
        content_reader_script::test_game::enter();
        let aliases = parse(
            &[(
                "a.txt",
                b"SPA = { original_tag = SPR OR = { has_country_flag = x } } \
              VIC = { original_tag = FRA has_focus_tree = vichy_french_focus }",
            )],
            &KeyNames::default(),
        );
        assert_eq!(names(&aliases), ["SPA", "VIC"]);
        let spa = aliases.get("SPA").unwrap();
        assert_eq!(spa.original_tag.as_deref(), Some(&b"SPR"[..]));
        assert_eq!(spa.limit.len(), 1);
        assert!(spa.limit[0].keyword_is("or"));
        assert!(!spa.checks_all());
        assert!(aliases.unknown.is_empty());
    }

    #[test]
    fn the_key_reader_keeps_each_member() {
        content_reader_script::test_game::enter();
        let aliases = parse(&[
            (
                "a.txt",
                b"SOU = { original_tag = SOV country_score = { base = 1 modifier = { add = 2 exists = yes } } } \
                  SB1 = { variable = SWI.SWI_country_to_appease_1 } \
                  TGT = { targets = { GER FRA } country = ITA fallback = ENG exists = yes } \
                  ALL = { has_country_flag = x } \
                  BAS = { original_tag = GER country_score = { base = 5 } } \
                  ARR = { target_array = global.x }",
            ),
            ("b.txt", b"sou = { original_tag = GER }"),
        ], &KeyNames::default());
        let sou = aliases.get("sou").unwrap();
        assert_eq!(sou.original_tag.as_deref(), Some(&b"SOV"[..]));
        assert!(sou.scored().is_some());
        assert!(aliases.get("BAS").unwrap().scored().is_none());
        assert_eq!(
            aliases.get("SB1").unwrap().variable,
            Some((
                VariableKey::Variable,
                b"SWI.SWI_country_to_appease_1".to_vec()
            ))
        );
        let tgt = aliases.get("TGT").unwrap();
        assert_eq!(
            tgt.targets,
            [b"GER".to_vec(), b"FRA".to_vec(), b"ITA".to_vec()]
        );
        assert_eq!(tgt.fallback.as_deref(), Some(&b"ENG"[..]));
        assert_eq!(tgt.limit.len(), 1);
        assert!(aliases.get("ALL").unwrap().checks_all());
        assert!(!tgt.checks_all());
        assert_eq!(aliases.get("ARR").unwrap().unported, ["target_array"]);
    }

    #[test]
    fn repeated_scores_join_one_mean_time() {
        content_reader_script::test_game::enter();
        let aliases = parse(&[(
            "a.txt",
            b"SOU = { country_score = { base = 1 modifier = { add = 2 exists = yes } } country_score = { base = 3 } }",
        )], &KeyNames::default());
        let score = aliases.get("SOU").unwrap().scored().unwrap();
        assert_eq!(score.len(), 3);
        assert!(score.last().unwrap().keyword_is("base"));
    }

    #[test]
    fn an_unsettled_body_refuses_it_and_the_later_ones() {
        content_reader_script::test_game::enter();
        let aliases = parse(&[(
            "a.txt",
            b"EAR = { exists = yes } MID = { OR = { x = { } } always = no } LAT = { exists = yes }",
        )], &KeyNames::unknown());
        assert_eq!(names(&aliases), ["EAR", "LAT", "MID"]);
        assert_eq!(aliases.get("EAR").unwrap().refused, None);
        for name in ["MID", "LAT"] {
            let why = aliases.get(name).unwrap().refused.clone().unwrap();
            assert!(why.contains("the body pass"), "{name}: {why}");
        }
    }

    #[test]
    fn a_duplicate_body_is_skipped_unread() {
        content_reader_script::test_game::enter();
        let aliases = parse(
            &[
                (
                    "a.txt",
                    b"ALI = { } ali = { @v = yes always = yes } LAT = { always = @v }",
                ),
                ("b.txt", b"lat = { @w = yes } NEW = { always = @w }"),
            ],
            &KeyNames::default(),
        );
        assert!(aliases.get("ALI").unwrap().limit.is_empty());
        let lat = aliases.get("LAT").unwrap();
        assert_eq!(lat.limit.len(), 1);
        assert_ne!(lat.limit[0].value.scalar(), Some(&b"yes"[..]));
        let new = aliases.get("NEW").unwrap();
        assert_ne!(new.limit[0].value.scalar(), Some(&b"yes"[..]));
        assert!(aliases.unknown.is_empty(), "{:?}", aliases.unknown);
    }

    #[test]
    fn an_alias_name_that_is_not_text_is_a_gap() {
        content_reader_script::test_game::enter();
        let aliases = parse(
            &[("a.txt", b"SPA = { } \"\xfeA\" = { }")],
            &KeyNames::default(),
        );
        assert_eq!(names(&aliases), ["SPA"]);
        assert_eq!(
            aliases.unknown,
            [
                "a.txt: an alias name is the stored text `\\xfeA` is not UTF-8, which the port's text cannot hold"
            ]
        );
    }
}
