//! Country leader and advisor traits (`common/country_leader`): each trait's
//! modifiers, equipment bonuses and targeted modifiers. Advisors in
//! `common/characters` and designers in `common/ideas` name these traits.
//!
//! Every file has its own reader replay ([`native`]): `leader_traits`
//! entries read into trait objects on that reader; any other top-level
//! key is reported and its block skipped.

use crate::{
    ResolvedLayers,
    coverage::{Coverage, Gap},
    key_names::KeyNames,
    modifiers::{self, EquipmentBonus, Modifier, Modifiers, TargetedModifier},
    parser::{
        Item, Unheld,
        native::{self, Shape},
    },
    script_files,
    tokens::{self, Token},
};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LeaderTrait {
    pub token: String,
    pub modifiers: Modifiers,
    /// The trait's own command fields: `command_power`,
    /// `command_cap_increase` and `command_cap_cost`, each read as fixed5.
    /// One entry per field that converted, its last value; a failed
    /// conversion keeps the earlier value.
    pub command: Vec<Modifier>,
    pub equipment_bonuses: Vec<EquipmentBonus>,
    pub targeted: Vec<TargetedModifier>,
}

/// One file's traits in read order. `refused` is set when the reading
/// cannot be followed: it names where and why, and `refused_tokens` the
/// traits from there on, whose contents are not known. `notes` are the
/// places the game's reading leaves the text's braces, each read as the game
/// reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraitFile {
    pub traits: Vec<LeaderTrait>,
    pub refused: Option<String>,
    pub refused_tokens: Vec<String>,
    pub notes: Vec<String>,
}

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    /// The file loop.
    Top,
    /// The `leader_traits` loop.
    Table,
    /// One trait and its fields.
    Trait,
    /// AI strategy fields.
    Strategy,
    /// Mean-time-to-happen fields.
    Mtth,
    Modifier(modifiers::Node),
}

/// Parses one trait file, its triggers against `names`.
pub fn parse(input: &[u8], names: &KeyNames) -> TraitFile {
    let mut undecided = Vec::new();
    // Each trait entry's position among the entries read, when it began.
    let mut entries = 0_usize;
    let mut stop: Option<(usize, String)> = None;
    // Construct a mode-0 lexer for each file.
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        Some(names),
        &mut |node, field| {
            let shape = match node {
                // `leader_traits` opens its own loop; any other key,
                // a quoted one included, is reported and skipped.
                Node::Top if field.keyword_is("leader_traits") => Shape::Object(Node::Table),
                Node::Top => Shape::Skip,
                // Each key names a trait by the hash of its text, quoted or not,
                // and reads a trait object whatever its value.
                Node::Table => {
                    entries += 1;
                    Shape::Object(Node::Trait)
                }
                Node::Trait => match field.keyword().as_deref() {
                    // Its own loop of equipment-bonus entries.
                    Some(b"equipment_bonus") => {
                        Shape::Object(Node::Modifier(modifiers::Node::EquipmentBonuses))
                    }
                    // `sprite` reads a signed integer, `random` a boolean,
                    // and the command fields fixed5 values.
                    Some(
                        b"sprite"
                        | b"random"
                        | b"command_power"
                        | b"command_cap_increase"
                        | b"command_cap_cost",
                    ) => Shape::One,
                    // A mean-time-to-happen object.
                    Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                    // A conjunction trigger.
                    Some(b"available") => Shape::Trigger,
                    // An AI strategy object.
                    Some(b"ai_strategy") => Shape::Object(Node::Strategy),
                    // A targeted modifier object.
                    Some(b"targeted_modifier") => {
                        Shape::Object(Node::Modifier(modifiers::Node::Targeted))
                    }
                    // The trait's modifier field dispatch.
                    _ => modifiers::shape(
                        modifiers::Node::Modifier,
                        field,
                        modifiers::UNKNOWN,
                        &mut undecided,
                    )
                    .map(Node::Modifier),
                },
                // `id`, `target`, `type`, `value` and `target_country` take one
                // value each; any other key goes to a reader per strategy type that
                // the port does not trace. What it consumes and defines is unknown,
                // so reading stops here.
                Node::Strategy => match field.keyword().as_deref() {
                    Some(b"id" | b"target" | b"type" | b"value" | b"target_country") => Shape::One,
                    _ => {
                        undecided.push(modifiers::Undecided::untraced(
                            field,
                            "the strategy type's own reader",
                        ));
                        Shape::Skip
                    }
                },
                Node::Mtth => native::mean_time_to_happen(field),
                Node::Modifier(modifier) => {
                    modifiers::shape(modifier, field, modifiers::UNKNOWN, &mut undecided)
                        .map(Node::Modifier)
                }
            };
            if stop.is_none()
                && let Some(first) = undecided.first()
            {
                stop = Some((entries.saturating_sub(1), first.to_string()));
            }
            shape
        },
    );
    let mut file = TraitFile {
        notes: read.misreads.iter().map(ToString::to_string).collect(),
        ..TraitFile::default()
    };
    if stop.is_none()
        && let Some(unsettled) = &read.unsettled
    {
        stop = Some((
            entries.saturating_sub(1),
            format!("{unsettled}; any trait the rest of the file defines is unknown"),
        ));
    }
    let entries = read
        .items
        .iter()
        .filter(|item| item.keyword_is("leader_traits"))
        .flat_map(|item| item.value.items().unwrap_or_default());
    // A trait whose stored text the port cannot hold stops the file there.
    let mut read_traits = Vec::new();
    let mut scans = modifiers::ScanFailures::default();
    for (index, entry) in entries.enumerate() {
        let token = match entry.key_str() {
            Ok(token) => token.to_owned(),
            Err(_) => entry
                .key
                .as_deref()
                .unwrap_or_default()
                .escape_ascii()
                .to_string(),
        };
        let found = read_trait(entry, entry.value.items().unwrap_or_default());
        if let Err(cause) = &found
            && stop.as_ref().is_none_or(|(at, _)| index < *at)
        {
            stop = Some((index, cause.clone()));
        }
        if let Ok(trait_) = &found {
            scans.modifiers(&token, &trait_.modifiers);
            for group in &trait_.targeted {
                scans.modifiers(&token, &group.modifiers);
            }
            for bonus in &trait_.equipment_bonuses {
                scans.scan(&token, &bonus.values);
            }
        }
        read_traits.push((token, found));
    }
    for (index, (token, found)) in read_traits.into_iter().enumerate() {
        match &stop {
            Some((at, _)) if index >= *at => file.refused_tokens.push(token),
            // A trait that cannot be held stopped the file at itself.
            _ => file.traits.extend(found.ok()),
        }
    }
    file.notes.extend(scans.warning());
    file.refused = stop.map(|(_, reason)| reason);
    file
}

/// One trait body as read, or what it stores that the port cannot hold.
fn read_trait(entry: &Item, fields: &[Item]) -> Result<LeaderTrait, String> {
    let mut unheld = Unheld::default();
    let mut found = LeaderTrait {
        token: unheld
            .keep("a trait's token", entry.key_str())
            .unwrap_or_default()
            .to_owned(),
        ..LeaderTrait::default()
    };
    for field in fields {
        match field.keyword().map(|keyword| keyword.folded()).as_deref() {
            Some(b"equipment_bonus") => found.equipment_bonuses.extend(
                modifiers::equipment_bonuses(field.value.items().unwrap_or_default(), &mut unheld),
            ),
            Some(b"targeted_modifier") => {
                let targeted = modifiers::targeted(field.value.items().unwrap_or_default());
                unheld.absorb(&targeted.modifiers.unheld);
                found.targeted.push(targeted);
            }
            Some(b"available" | b"ai_will_do" | b"ai_strategy" | b"random" | b"sprite") => {}
            // Read fixed5 into the trait's own command fields.
            Some(keyword @ (b"command_power" | b"command_cap_increase" | b"command_cap_cost")) => {
                let Some(value) = modifiers::read_fixed5(field) else {
                    continue;
                };
                let name = String::from_utf8_lossy(keyword).into_owned();
                let (Some(key), Some(text)) = (
                    unheld.keep("a command field's key", field.key_str()),
                    unheld.keep(
                        "a command field's value",
                        crate::parser::text(field.value.scalar().unwrap_or_default()),
                    ),
                ) else {
                    continue;
                };
                found.command.retain(|entry| entry.name != name);
                found.command.push(Modifier {
                    key: key.to_owned(),
                    name,
                    text: text.to_owned(),
                    value,
                });
            }
            // Anything else belongs to the trait's modifier object.
            _ => found.modifiers.read(field),
        }
    }
    unheld.absorb(&found.modifiers.unheld);
    match unheld.cause() {
        Some(cause) => Err(format!("trait {}: {cause}", found.token)),
        None => Ok(found),
    }
}

/// Every leader trait in the content stack.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LeaderTraits {
    /// Every trait in load order, appended after the null object,
    /// a repeated key included.
    definitions: Vec<(Token, LeaderTrait)>,
    /// The file each definition was read from, in load order.
    files: Vec<usize>,
    /// By folded name, the first definition's index.
    traits: BTreeMap<String, usize>,
    coverage: Coverage,
    /// By token ([`token_key`]): every definition counts, the first decides.
    tokens: Coverage,
}

impl LeaderTraits {
    /// The first trait whose hashed name matches without ASCII case,
    /// else the null object. Unlike a lexer-token lookup, names beginning
    /// with a digit or `-` stay distinct. Only a gap before the first
    /// definition can change the answer.
    pub fn get(&self, token: &str) -> Result<Option<&LeaderTrait>, &Gap> {
        let name = token.to_ascii_lowercase();
        match self.coverage.before_first(&name) {
            Some(gap) => Err(gap),
            None => Ok(self.traits.get(&name).map(|at| &self.definitions[*at].1)),
        }
    }

    /// The database ordinal of [`Self::get`]'s trait: its position after
    /// the null object, assigned when constructed. Known only when no
    /// file before its own stops short, as [`Self::get`].
    pub fn first_ordinal(&self, token: &str) -> Result<Option<u32>, &Gap> {
        let name = token.to_ascii_lowercase();
        match self.coverage.before_first(&name) {
            Some(gap) => Err(gap),
            None => Ok(self.traits.get(&name).map(|at| *at as u32 + 1)),
        }
    }

    /// The definition at a database ordinal (0 is the null object),
    /// known only when no file before its own stops short.
    pub fn by_ordinal(&self, ordinal: u32) -> Option<&LeaderTrait> {
        let index = usize::try_from(ordinal).ok()?.checked_sub(1)?;
        let (_, found) = self.definitions.get(index)?;
        self.coverage
            .gap_before(self.files[index])
            .is_none()
            .then_some(found)
    }

    /// The first trait after the null object whose key token is `token`;
    /// None is the null object. The gap when a file before that trait
    /// (any file when no trait has the token) stops short: its unread
    /// rest could hold an earlier trait of the token.
    pub fn by_token(&self, token: &Token) -> Result<Option<&LeaderTrait>, &Gap> {
        if let Some(gap) = self.tokens.before_first(&token_key(token)) {
            return Err(gap);
        }
        Ok(tokens::first(
            self.definitions.iter().map(|(token, found)| (token, found)),
            token,
        ))
    }
}

/// A token as [`LeaderTraits::tokens`] keys it: a registered name's folded
/// text (trait keys are text), NUL for token 0, which no name holds.
fn token_key(token: &Token) -> String {
    match token {
        Token::Rejected => "\0".to_owned(),
        Token::Name(name) => String::from_utf8_lossy(name).into_owned(),
    }
}

/// Every trait in load order, including repeated names. Both lookups
/// select the first match. A file that stops short leaves lookups unknown
/// only when its unread suffix could precede their first match.
pub fn read(layers: &ResolvedLayers) -> io::Result<(LeaderTraits, Vec<crate::Diagnostic>)> {
    let mut warnings = Vec::new();
    let mut result = LeaderTraits::default();
    let names = layers.key_names()?;
    for path in script_files::paths(layers, "common/country_leader/", Some(".txt")) {
        result.coverage.next_file();
        result.tokens.next_file();
        let file = parse(&layers.read(&path)?, &names);
        warnings.extend(file.notes.iter().map(|note| {
            (
                crate::DiagnosticKind::Misread,
                format!("Leader traits in {path}: {note}."),
            )
        }));
        for found in file.traits {
            let token = Token::c_str(found.token.as_bytes());
            let name = found.token.to_ascii_lowercase();
            result.coverage.add(&name);
            result.tokens.add(&token_key(&token));
            result
                .traits
                .entry(name)
                .or_insert(result.definitions.len());
            result.definitions.push((token, found));
            result.files.push(result.coverage.file());
        }
        if let Some(reason) = file.refused {
            result.coverage.gap(&path, reason.clone());
            result.tokens.gap(&path, reason.clone());
            warnings.push((crate::DiagnosticKind::Unsettled, format!(
                "Leader traits unavailable in {path}: {reason}, so the rest of the file cannot be determined; refused traits: {}",
                if file.refused_tokens.is_empty() {
                    "none".to_owned()
                } else {
                    file.refused_tokens.join(", ")
                }
            )));
        }
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each stored modifier: its key as authored, its text and its fixed5.
    fn amounts(found: &LeaderTrait) -> Vec<(&str, &str, i64)> {
        found
            .modifiers
            .values
            .iter()
            .map(|m| (m.key.as_str(), m.text.as_str(), m.value))
            .collect()
    }

    #[test]
    fn traits_resolve_file_variables_and_skip_structure() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let file = parse(
            b"@tier2 = 20 leader_traits = { army_regrouping_2 = { random = no sprite = 5 army_morale_factor = 0.08 \
              command_cap_increase = @tier2 ai_will_do = { factor = 1 } Command_Power = 1 command_power = bad } \
              heavy_tank_manufacturer = { equipment_bonus = { armor = { armor_value = 0.05 } } } }", &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        assert!(file.notes.is_empty(), "{:?}", file.notes);
        assert_eq!(
            amounts(&file.traits[0]),
            [("army_morale_factor", "0.08", 8_000)]
        );
        // The trait's own fixed5 fields: a failed conversion keeps
        // the earlier value.
        let command: Vec<_> = file.traits[0]
            .command
            .iter()
            .map(|m| (m.name.as_str(), m.value))
            .collect();
        assert_eq!(
            command,
            [
                ("command_cap_increase", 2_000_000),
                ("command_power", 100_000)
            ]
        );
        assert!(file.traits[1].modifiers.values.is_empty());
        assert_eq!(file.traits[1].equipment_bonuses[0].equipment, "armor");
    }

    /// The file's variable table in document order: quoting, first
    /// definition, definitions inside blocks read by key (ai_will_do
    /// included) and a reference before its definition.
    #[test]
    fn traits_follow_the_file_reader_table_in_document_order() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let file = parse(
            b"LEADER_TRAITS = { early = { a = @late b = \"@top\" } \
              nested = { @inner = 0.5 c = @inner ai_will_do = { @weight = 2 factor = 1 } d = @weight } \
              later = { e = @inner f = @weight g = @top } } \
              @late = 1 @top = 3 other = { @skipped = 4 x = 1 } leader_traits = { last = { h = @skipped } }", &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        let by = |token: &str| file.traits.iter().find(|t| t.token == token).unwrap();
        assert_eq!(
            amounts(by("early")),
            [
                // Read before its definition, the reference keeps its text,
                // which does not convert: the value stays 0.
                ("a", "@late", 0),
                // A quoted token is never a reference.
                ("b", "@top", 0),
            ]
        );
        assert_eq!(
            amounts(by("nested")),
            [("c", "0.5", 50_000), ("d", "2", 200_000)]
        );
        assert_eq!(
            amounts(by("later")),
            [("e", "0.5", 50_000), ("f", "2", 200_000), ("g", "@top", 0)]
        );
        // An unknown top-level key is skipped, defining nothing.
        assert_eq!(amounts(by("last")), [("h", "@skipped", 0)]);
        // A trigger, a targeted modifier's trigger and an added modifier
        // define variables; `available` is no modifier line.
        let file = parse(
            b"leader_traits = { t = { available = { OR = { @av = 4 always = yes } } \
              targeted_modifier = { tag = GER valid_relation_trigger = { @vr = 5 always = yes } a = @av } \
              added_modifier = { @am = 6 b = 1 } k = @av l = @vr m = @am } }", &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        let found = &file.traits[0];
        assert_eq!(found.targeted[0].modifiers.values.len(), 1);
        assert_eq!(found.targeted[0].modifiers.values[0].value, 400_000);
        // `added_modifier` is a child modifier object with no tooltip line.
        assert_eq!(
            amounts(found),
            [
                ("k", "4", 400_000),
                ("l", "5", 500_000),
                ("m", "6", 600_000)
            ]
        );
        assert_eq!(found.modifiers.children[0].values[0].name, "b");
    }

    /// A taken name reads nothing more: its `=` is the next key, which
    /// consumes the value as its operator, and the trait reads on; a value
    /// without a key takes the next token as its operator. Only a block whose
    /// reading the port cannot settle refuses the trait being read and every
    /// later one in the file, by name.
    #[test]
    fn out_of_step_forms_are_read_and_unsettled_ones_refuse() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let file = parse(
            b"@x = 1 leader_traits = { kept = { a = @x } taken = { @x = 2 b = @x } after = { c = 1 } }", &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        assert_eq!(file.traits.len(), 3);
        assert_eq!(amounts(&file.traits[1]), [("b", "1", 100_000)]);
        assert!(file.notes[0].contains("variable `@x` is already taken"));
        let file = parse(b"leader_traits = { t = { a = 2 x } }", &KeyNames::default());
        assert_eq!(
            amounts(&file.traits[0]),
            [("a", "2", 200_000), ("x", "2", 200_000)]
        );
        assert!(file.notes[0].contains("key `x` has no operator"));
        for (source, expected, refused) in [
            (
                b"leader_traits = { s = { } t = { some_modifier = { @m = 1 a = 2 } } u = { } }"
                    .as_slice(),
                "line 1: some_modifier has a block value, which modifier value reader reads",
                ["t", "u"].as_slice(),
            ),
            (
                b"leader_traits = { s = { } t = { available = { OR = { tag = GER x = { @v = 1 } } } } u = { } }",
                "line 1: field `x`: trigger dispatch reads it",
                ["t"].as_slice(),
            ),
            // A strategy field the type's own reader takes.
            (
                b"leader_traits = { s = { } t = { ai_strategy = { type = alliance Weight = { x = 1 } } } u = { } }",
                "line 1: Weight is read by the strategy type's own reader, which the port does not trace",
                ["t", "u"].as_slice(),
            ),
        ] {
            let file = parse(source, &KeyNames::unknown());
            let reason = file.refused.unwrap_or_default();
            assert!(reason.starts_with(expected), "{reason}");
            assert_eq!(file.traits.len(), 1);
            assert_eq!(file.refused_tokens, refused);
        }
    }

    /// Trait fields dispatch on their keyword: a bare key folds; a quoted
    /// one has no keyword and reaches the default, the trait's modifier,
    /// by its name.
    #[test]
    fn trait_keywords_fold_and_quoted_keys_are_modifier_names() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let file = parse(
            b"LEADER_TRAITS = { t = { Equipment_Bonus = { armor = { armor_value = 0.1 } } \
              \"random\" = 2 RANDOM = no \"leader_traits\" = 1 } } \"leader_traits\" = { u = { } }",
            &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        assert_eq!(file.traits.len(), 1);
        let found = &file.traits[0];
        assert_eq!(found.equipment_bonuses[0].equipment, "armor");
        assert_eq!(
            amounts(found),
            [("random", "2", 200_000), ("leader_traits", "1", 100_000)]
        );
    }

    /// The replay's classes through this reader: a variable holds within
    /// its file only; a block where fixed5 takes one value is read as the
    /// trait's own fields; the end of the text closes every loop.
    #[test]
    fn the_replay_classes_hold_for_traits() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let file = parse(
            b"leader_traits = { t = { command_power = { army_morale_factor = 0.1 } after = 0.2 } }",
            &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        let by = |token: &str| file.traits.iter().find(|t| t.token == token).unwrap();
        // `command_power` read `{`, which does not convert; the block's
        // entry is the trait's, and its `}` ended the trait, so `after` is
        // read as a trait of its own.
        assert!(by("t").command.is_empty());
        assert_eq!(amounts(by("t")), [("army_morale_factor", "0.1", 10_000)]);
        assert!(
            file.notes[0].contains("field `command_power` is a block"),
            "{:?}",
            file.notes
        );
        assert!(file.traits.iter().any(|t| t.token == "after"));
        let file = parse(
            b"leader_traits = { open = { Army_Morale_Factor = @missing",
            &KeyNames::default(),
        );
        assert_eq!(file.notes.len(), 1, "{:?}", file.notes);
        assert!(
            file.notes[0].contains("1 content values failed native numeric conversion")
                && file.notes[0].contains("open: Army_Morale_Factor = @missing"),
            "{:?}",
            file.notes
        );
        assert_eq!(
            amounts(&file.traits[0]),
            [("Army_Morale_Factor", "@missing", 0)]
        );
    }

    /// Every trait entry is appended, a repeated key included, and the first
    /// after the null object whose token matches is returned: the lexer
    /// fold joins spellings differing in `A`-`Z` only. A file before that trait
    /// stopping short leaves the lookup unknown; one after it does not.
    #[cfg(feature = "test-support")]
    #[test]
    fn a_trait_is_found_by_token_as_the_database_scans_it() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root =
            std::env::temp_dir().join(format!("installed-content-traits-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "common/country_leader/a.txt",
            "leader_traits = { Brilliant = { army_morale_factor = 0.1 } brilliant = { army_morale_factor = 0.2 } }",
        );
        write(
            "common/country_leader/b.txt",
            "leader_traits = { BRILLIANT = { army_morale_factor = 0.3 } other = { } lost = { odd = { } } }",
        );
        write(
            "common/country_leader/c.txt",
            "leader_traits = { late = { } }",
        );
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let (traits, _) = read(&layers).unwrap();
        let first = traits
            .by_token(&Token::register(b"bRiLLiant"))
            .unwrap()
            .unwrap();
        assert_eq!(first.token, "Brilliant");
        assert_eq!(amounts(first), [("army_morale_factor", "0.1", 10_000)]);
        // The advisor/idea name lookup also selects the first spelling;
        // a later unread suffix cannot replace it.
        assert_eq!(traits.get("bRiLLiant").unwrap(), Some(first));
        // Its database ordinal is its append position after the null
        // object: the repeats are 2 and 3.
        assert_eq!(traits.first_ordinal("bRiLLiant"), Ok(Some(1)));
        // Ordinals run on across files and repeats: a.txt holds 1 and 2,
        // b.txt's BRILLIANT is 3 and `other` 4.
        assert_eq!(traits.first_ordinal("OTHER"), Ok(Some(4)));
        assert_eq!(
            traits.by_ordinal(3).map(|found| found.token.as_str()),
            Some("BRILLIANT")
        );
        assert!(traits.by_ordinal(0).is_none());
        // Past b.txt, which stops short, no ordinal is known.
        assert!(traits.first_ordinal("late").is_err());
        assert!(traits.by_ordinal(5).is_none());
        assert!(traits.first_ordinal("never").is_err());
        // No trait of the token: b.txt's unread rest could hold one.
        let gap = traits.by_token(&Token::register(b"never")).unwrap_err();
        assert_eq!(gap.path, "common/country_leader/b.txt");
        assert!(traits.by_token(&Token::Rejected).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
