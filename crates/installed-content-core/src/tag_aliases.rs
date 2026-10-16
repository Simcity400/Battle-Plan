//! `common/country_tag_aliases`: the tag alias database, read in two passes.
//!
//! 1. Names: each top-level key interns its name and creates an entry.
//!    A name another file already holds, or one repeated in the same file,
//!    is reported and its block skipped, so the first definition in file
//!    order stays. Bodies are skipped during this pass.
//! 2. Bodies: every file again, each entry read by its key reader
//!    ([`read_file`]).
//!
//! A tag alias names a country chosen at run time from its targets and
//! triggers; `dashboard::triggers` resolves it over the recorded hour.
//! A name the port cannot hold, or a file whose reading it cannot follow,
//! is kept as a named gap.

use crate::{
    ResolvedLayers,
    combat_tactics::{self, Node as TriggerNode},
    key_names::KeyNames,
    parser::{
        Item,
        native::{self, Field, Shape},
    },
    script_files,
    triggers::Triggers,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// One alias as its key reader leaves it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagAlias {
    /// `original_tag`: the last country-specifier text read.
    pub original_tag: Option<Vec<u8>>,
    /// How many `original_tag` assignments the body makes: each updates
    /// one specifier, whose state across assignments (an alias pointer
    /// kept past a country) the port does not replay.
    pub original_tag_assignments: usize,
    /// `targets` (a string list) and `country`/`state` target elements, in order.
    pub targets: Vec<Vec<u8>>,
    /// `variable`, `event_target` or `global_event_target`: the scoped
    /// variable that decides the alias alone, with the key that set it.
    pub variable: Option<(VariableKey, Vec<u8>)>,
    /// `fallback`: the fallback scoped variable.
    pub fallback: Option<Vec<u8>>,
    /// Every other key: a conjunction child, read in country scope.
    pub limit: Triggers,
    /// `country_score`: its mean-time fields as the reader keeps them.
    /// It decides only when it contains at least one `modifier`.
    pub score: Option<Triggers>,
    /// Target members the port does not follow (`target_array`,
    /// `target_trigger`, `target_root_trigger`, `targets_dynamic`,
    /// `target_non_existing`, `state_target`), by name.
    pub unported: Vec<String>,
    /// Whether any target member was read, which with no original tag
    /// and no variable decides the check of every country.
    pub targets_read: bool,
    /// Why the body cannot be taken as the game reads it: its file's
    /// reading could not be followed through it.
    pub refused: Option<String>,
}

impl TagAlias {
    /// No original tag, no target member and no variable, so every
    /// country slot is a candidate.
    pub fn checks_all(&self) -> bool {
        self.original_tag.is_none() && !self.targets_read && self.variable.is_none()
    }

    /// The score decides (a `country_score` with a `modifier`).
    pub fn scored(&self) -> Option<&Triggers> {
        self.score
            .as_ref()
            .filter(|fields| fields.iter().any(|field| field.keyword_is("modifier")))
    }
}

/// The key that set an alias's scoped variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariableKey {
    /// `variable`: the value read as an operand.
    Variable,
    /// `event_target`: a saved event target by name.
    EventTarget,
    /// `global_event_target`: a global saved event target by name.
    GlobalEventTarget,
}

/// Every alias, and the files whose names cannot all be held, each with its
/// cause.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagAliases {
    /// The first definition of each name by file order, keyed by the name
    /// folded as the lexer folds it: the database is indexed by the name's
    /// token, with the name as first read.
    pub entries: BTreeMap<Vec<u8>, (String, TagAlias)>,
    pub unknown: Vec<String>,
}

impl TagAliases {
    /// The alias a text names, found by its token (without ASCII case).
    pub fn get(&self, text: &str) -> Option<&TagAlias> {
        self.entries
            .get(&text.as_bytes().to_ascii_lowercase())
            .map(|(_, alias)| alias)
    }
}

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    /// An alias entry.
    Alias,
    /// A trigger body's field, as the combat-tactic reader dispatches it.
    Trigger(TriggerNode),
}

/// The key reader of an alias block, trying target members first.
fn alias_field(shaper: &mut combat_tactics::Shaper<'_>, field: &Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        // Read a string list.
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
        // Read mean-time fields.
        Some(b"country_score") => Shape::Object(Node::Trigger(TriggerNode::Mtth)),
        _ => shaper.shape(TriggerNode::Trigger, field).map(Node::Trigger),
    }
}

/// One file's aliases in order, and where its names cannot be held.
/// The name pass skips each body; the body pass reads each entry. In
/// each pass a name an earlier file gave, or this pass already met in
/// this file, is reported as a duplicate and its block skipped, reading
/// and defining nothing. A body the port cannot follow the reading
/// through, and every later one, is refused by name.
fn read_file(
    input: &[u8],
    earlier: &BTreeMap<Vec<u8>, (String, TagAlias)>,
    key_names: &KeyNames,
) -> (Vec<(String, TagAlias)>, Option<String>) {
    let names = name_pass(input, &|name| earlier.contains_key(name));
    let mut seen = BTreeSet::new();
    let mut shaper = combat_tactics::Shaper::new(Some(key_names));
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
    // The body being read where the reading stopped is partial.
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
            // An alias name the port cannot hold: no tag the port reads can
            // equal it, but the lookup through it cannot be followed.
            Err(refused) => {
                stop.get_or_insert_with(|| format!("an alias name is {refused}"));
                continue;
            }
        };
        let folded = name.as_bytes().to_ascii_lowercase();
        // A name the body pass did not reach keeps the default body, unless
        // the reading stopped before it.
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

/// Whether a name another file already gave (`earlier`) or this pass
/// already met in this file (`seen`) is a duplicate: the name's token,
/// without case.
fn duplicate(
    seen: &mut BTreeSet<Vec<u8>>,
    field: &Field<'_>,
    earlier: &dyn Fn(&[u8]) -> bool,
) -> bool {
    let name = field.key.text.to_ascii_lowercase();
    earlier(&name) || !seen.insert(name)
}

/// The name pass over one file.
fn name_pass(input: &[u8], earlier: &dyn Fn(&[u8]) -> bool) -> native::Read {
    // The name pass constructs the lexer in mode 0.
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

/// An alias's fields as its reader keeps them.
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
            // Every block reads into the one mean-time object, appending
            // its modifiers.
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

/// Every alias name of the content stack's name pass, folded: the names
/// the scope step finds by token once the pass has run, before ideologies.
pub fn names(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut names = BTreeSet::new();
    for path in script_files::paths(layers, FOLDER, Some(".txt")) {
        let read = name_pass(&layers.read(&path)?, &|name| names.contains(name));
        names.extend(
            read.items
                .iter()
                .map(|item| item.key.as_deref().unwrap_or_default().to_ascii_lowercase()),
        );
    }
    Ok(names)
}

/// The aliases of the content stack, listing the directory's own `.txt`
/// files through [`script_files::paths`].
pub fn read(layers: &ResolvedLayers) -> io::Result<TagAliases> {
    let mut files = Vec::new();
    for path in script_files::paths(layers, FOLDER, Some(".txt")) {
        files.push((path.clone(), layers.read(&path)?));
    }
    Ok(parse(
        &files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>(),
        &*layers.key_names()?,
    ))
}

/// Both passes over `files` (path and text), in file order, the bodies
/// read against `names`.
pub fn parse(files: &[(&str, &[u8])], names: &KeyNames) -> TagAliases {
    let mut aliases = TagAliases::default();
    for (path, bytes) in files {
        let (read, stop) = read_file(bytes, &aliases.entries, names);
        for (name, alias) in read {
            // "Duplicate database id": a name another file or this one
            // already gave, by token, is skipped.
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
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// The reader's keys: targets and target elements, the variable, the
    /// fallback, a score that decides only with a modifier, and every other
    /// key a trigger. A first definition stays, found by its token, without case.
    #[test]
    fn the_key_reader_keeps_each_member() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// Every `country_score` block reads into the one mean-time object:
    /// a later block without a modifier keeps the earlier modifiers,
    /// and the score still decides.
    #[test]
    fn repeated_scores_join_one_mean_time() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let aliases = parse(&[(
            "a.txt",
            b"SOU = { country_score = { base = 1 modifier = { add = 2 exists = yes } } country_score = { base = 3 } }",
        )], &KeyNames::default());
        let score = aliases.get("SOU").unwrap().scored().unwrap();
        assert_eq!(score.len(), 3);
        assert!(score.last().unwrap().keyword_is("base"));
    }

    /// A body the reading cannot be followed through is refused by name, as
    /// is every later one, whose names the name pass still gives; earlier
    /// bodies stand.
    #[test]
    fn an_unsettled_body_refuses_it_and_the_later_ones() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// A duplicate's block is skipped before it is read, so its `@`
    /// definitions never reach later aliases; a name an earlier file
    /// gave is skipped too.
    #[test]
    fn a_duplicate_body_is_skipped_unread() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// An alias name that is not UTF-8 is a named gap, never dropped.
    #[test]
    fn an_alias_name_that_is_not_text_is_a_gap() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
