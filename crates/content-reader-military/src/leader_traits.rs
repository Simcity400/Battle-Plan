use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{coverage::{Coverage, Gap}, parser::{Item, Unheld}, tokens::{self, Token}};
use content_reader_script::{key_names::KeyNames, modifiers::{self, EquipmentBonus, Modifier, Modifiers, TargetedModifier}, native::{self, Shape}};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LeaderTrait {
    pub token: String,
    pub modifiers: Modifiers,
    pub command: Vec<Modifier>,
    pub equipment_bonuses: Vec<EquipmentBonus>,
    pub targeted: Vec<TargetedModifier>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraitFile {
    pub traits: Vec<LeaderTrait>,
    pub refused: Option<String>,
    pub refused_tokens: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    Table,
    Trait,
    Strategy,
    Mtth,
    Modifier(modifiers::Node),
}

pub fn parse(input: &[u8], names: &KeyNames) -> TraitFile {
    let mut undecided = Vec::new();
    let mut entries = 0_usize;
    let mut stop: Option<(usize, String)> = None;
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        Some(names),
        &mut |node, field| {
            let shape = match node {
                Node::Top if field.keyword_is("leader_traits") => Shape::Object(Node::Table),
                Node::Top => Shape::Skip,
                Node::Table => {
                    entries += 1;
                    Shape::Object(Node::Trait)
                }
                Node::Trait => match field.keyword().as_deref() {
                    Some(b"equipment_bonus") => {
                        Shape::Object(Node::Modifier(modifiers::Node::EquipmentBonuses))
                    }
                    Some(
                        b"sprite"
                        | b"random"
                        | b"command_power"
                        | b"command_cap_increase"
                        | b"command_cap_cost",
                    ) => Shape::One,
                    Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                    Some(b"available") => Shape::Trigger,
                    Some(b"ai_strategy") => Shape::Object(Node::Strategy),
                    Some(b"targeted_modifier") => {
                        Shape::Object(Node::Modifier(modifiers::Node::Targeted))
                    }
                    _ => modifiers::shape(
                        modifiers::Node::Modifier,
                        field,
                        modifiers::UNKNOWN,
                        &mut undecided,
                    )
                    .map(Node::Modifier),
                },
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
            _ => file.traits.extend(found.ok()),
        }
    }
    file.notes.extend(scans.warning());
    file.refused = stop.map(|(_, reason)| reason);
    file
}

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
            Some(keyword @ (b"command_power" | b"command_cap_increase" | b"command_cap_cost")) => {
                let Some(value) = modifiers::read_fixed5(field) else {
                    continue;
                };
                let name = String::from_utf8_lossy(keyword).into_owned();
                let (Some(key), Some(text)) = (
                    unheld.keep("a command field's key", field.key_str()),
                    unheld.keep(
                        "a command field's value",
                        content_reader_parser::parser::text(field.value.scalar().unwrap_or_default()),
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
            _ => found.modifiers.read(field),
        }
    }
    unheld.absorb(&found.modifiers.unheld);
    match unheld.cause() {
        Some(cause) => Err(format!("trait {}: {cause}", found.token)),
        None => Ok(found),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LeaderTraits {
    definitions: Vec<(Token, LeaderTrait)>,
    files: Vec<usize>,
    traits: BTreeMap<String, usize>,
    coverage: Coverage,
    tokens: Coverage,
}

impl LeaderTraits {
    pub fn get(&self, token: &str) -> Result<Option<&LeaderTrait>, &Gap> {
        let name = token.to_ascii_lowercase();
        match self.coverage.before_first(&name) {
            Some(gap) => Err(gap),
            None => Ok(self.traits.get(&name).map(|at| &self.definitions[*at].1)),
        }
    }

    pub fn first_ordinal(&self, token: &str) -> Result<Option<u32>, &Gap> {
        let name = token.to_ascii_lowercase();
        match self.coverage.before_first(&name) {
            Some(gap) => Err(gap),
            None => Ok(self.traits.get(&name).map(|at| *at as u32 + 1)),
        }
    }

    pub fn by_ordinal(&self, ordinal: u32) -> Option<&LeaderTrait> {
        let index = usize::try_from(ordinal).ok()?.checked_sub(1)?;
        let (_, found) = self.definitions.get(index)?;
        self.coverage
            .gap_before(self.files[index])
            .is_none()
            .then_some(found)
    }

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

fn token_key(token: &Token) -> String {
    match token {
        Token::Rejected => "\0".to_owned(),
        Token::Name(name) => String::from_utf8_lossy(name).into_owned(),
    }
}

pub fn read(layers: &ResolvedLayers) -> io::Result<(LeaderTraits, Vec<content_reader_parser::Diagnostic>)> {
    let mut warnings = Vec::new();
    let mut result = LeaderTraits::default();
    let names = layers.key_names()?;
    for path in script_files::paths(layers, "common/country_leader/", Some(".txt"))? {
        result.coverage.next_file();
        result.tokens.next_file();
        let file = parse(&path.read(layers)?, &names);
        warnings.extend(file.notes.iter().map(|note| {
            (
                content_reader_parser::DiagnosticKind::Misread,
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
            result.coverage.gap(&path.text(), reason.clone());
            result.tokens.gap(&path.text(), reason.clone());
            warnings.push((content_reader_parser::DiagnosticKind::Unsettled, format!(
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
        content_reader_script::test_game::enter();
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

    #[test]
    fn traits_follow_the_file_reader_table_in_document_order() {
        content_reader_script::test_game::enter();
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
                ("a", "@late", 0),
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
        assert_eq!(amounts(by("last")), [("h", "@skipped", 0)]);
        let file = parse(
            b"leader_traits = { t = { available = { OR = { @av = 4 always = yes } } \
              targeted_modifier = { tag = GER valid_relation_trigger = { @vr = 5 always = yes } a = @av } \
              added_modifier = { @am = 6 b = 1 } k = @av l = @vr m = @am } }", &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        let found = &file.traits[0];
        assert_eq!(found.targeted[0].modifiers.values.len(), 1);
        assert_eq!(found.targeted[0].modifiers.values[0].value, 400_000);
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

    #[test]
    fn out_of_step_forms_are_read_and_unsettled_ones_refuse() {
        content_reader_script::test_game::enter();
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

    #[test]
    fn trait_keywords_fold_and_quoted_keys_are_modifier_names() {
        content_reader_script::test_game::enter();
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

    #[test]
    fn the_replay_classes_hold_for_traits() {
        content_reader_script::test_game::enter();
        let file = parse(
            b"leader_traits = { t = { command_power = { army_morale_factor = 0.1 } after = 0.2 } }",
            &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        let by = |token: &str| file.traits.iter().find(|t| t.token == token).unwrap();
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

    #[test]
    fn a_trait_is_found_by_token_as_the_database_scans_it() {
        content_reader_script::test_game::enter();
        let root =
            std::env::temp_dir().join(format!("content-reader-traits-token-{}", std::process::id()));
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
        assert_eq!(traits.get("bRiLLiant").unwrap(), Some(first));
        assert_eq!(traits.first_ordinal("bRiLLiant"), Ok(Some(1)));
        assert_eq!(traits.first_ordinal("OTHER"), Ok(Some(4)));
        assert_eq!(
            traits.by_ordinal(3).map(|found| found.token.as_str()),
            Some("BRILLIANT")
        );
        assert!(traits.by_ordinal(0).is_none());
        assert!(traits.first_ordinal("late").is_err());
        assert!(traits.by_ordinal(5).is_none());
        assert!(traits.first_ordinal("never").is_err());
        let gap = traits.by_token(&Token::register(b"never")).unwrap_err();
        assert_eq!(gap.path, "common/country_leader/b.txt");
        assert!(traits.by_token(&Token::Rejected).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod dispatch_tests {
#[test]
fn content_dispatch_distinguishes_keywords_from_authored_names() {
    let traits = crate::leader_traits::parse(
        b"LEADER_TRAITS={synthetic_trait={COMMAND_POWER=0.35 \"command_power\"=0.75}}",
        &content_reader_script::key_names::KeyNames::default(),
    );
    assert_eq!(traits.refused, None);
    assert_eq!(traits.traits.len(), 1);
    let found = &traits.traits[0];
    assert_eq!(found.command.len(), 1);
    assert_eq!(found.command[0].name, "command_power");
    assert_eq!(found.command[0].value, 35_000);
    assert_eq!(found.modifiers.values.len(), 1);
    assert_eq!(found.modifiers.values[0].name, "command_power");
    assert_eq!(found.modifiers.values[0].value, 75_000);

    let bare = content_reader_key_names::ideologies::parse(b"IDEOLOGIES={synthetic={TYPES={branch={}}}}");
    assert_eq!(bare.len(), 1);
    assert_eq!(bare[0].types, ["branch"]);
    let quoted_field = content_reader_key_names::ideologies::parse(b"ideologies={synthetic={\"types\"={branch={}}}}");
    assert_eq!(quoted_field.len(), 1);
    assert!(quoted_field[0].types.is_empty());
    assert!(content_reader_key_names::ideologies::parse(b"\"ideologies\"={synthetic={}}").is_empty());
}

}
