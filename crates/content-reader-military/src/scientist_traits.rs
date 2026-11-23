use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{coverage::Coverage, parser::Unheld, tokens::Token};
use content_reader_script::{key_names::KeyNames, modifiers::{self, Modifiers}, native::{self, Shape}};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScientistTrait {
    pub token: String,
    pub name: Option<String>,
    pub modifiers: Modifiers,
    pub name_refusal: Option<String>,
    pub refusal: Option<String>,
}

impl ScientistTrait {
    pub fn display_key(&self) -> Result<&str, String> {
        if let Some(why) = &self.name_refusal {
            return Err(why.clone());
        }
        Ok(self
            .name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.token))
    }
}

#[derive(Default)]
pub struct ScientistTraits {
    definitions: BTreeMap<Token, ScientistTrait>,
    coverage: Coverage,
}

fn key(token: &Token) -> String {
    match token {
        Token::Rejected => "\0".to_owned(),
        Token::Name(name) => String::from_utf8_lossy(name).into_owned(),
    }
}

impl ScientistTraits {
    pub fn get(&self, token: &str) -> Result<Option<&ScientistTrait>, String> {
        let token = Token::c_str(token.as_bytes());
        if let Some(gap) = self.coverage.before_first(&key(&token)) {
            return Err(gap.to_string());
        }
        Ok(self.definitions.get(&token))
    }
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Trait,
    Modifier(modifiers::Node),
}

struct TraitFile {
    traits: Vec<ScientistTrait>,
    notes: Vec<content_reader_parser::Diagnostic>,
    gap: Option<String>,
}

fn parse(input: &[u8], names: &KeyNames, earlier: &BTreeSet<Token>) -> TraitFile {
    let mut seen = earlier.clone();
    let mut entries = 0usize;
    let mut undecided = Vec::new();
    let mut stop = None;
    let registry = |key: &[u8]| modifiers::registered(key, Some(names));
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(Node::File),
        Some(names),
        &mut |node, field| {
            let shape = match node {
                Node::File => {
                    if seen.insert(Token::c_str(field.key.c_str())) {
                        entries += 1;
                        Shape::Object(Node::Trait)
                    } else {
                        Shape::Skip
                    }
                }
                Node::Trait => match field.keyword().as_deref() {
                    Some(b"name" | b"icon") => Shape::One,
                    Some(b"specialization") => Shape::List,
                    Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
                    Some(b"available") => Shape::Trigger,
                    _ => Shape::Skip,
                },
                Node::Modifier(node) => {
                    modifiers::shape(node, field, &registry, &mut undecided).map(Node::Modifier)
                }
            };
            if stop.is_none()
                && let Some(why) = undecided.first()
            {
                stop = Some((entries.saturating_sub(1), why.to_string()));
            }
            shape
        },
    );
    if stop.is_none()
        && let Some(why) = read.unsettled
    {
        stop = Some((entries.saturating_sub(1), why.to_string()));
    }
    let mut result = TraitFile {
        traits: Vec::new(),
        notes: read
            .misreads
            .iter()
            .map(|note| (note.kind, note.to_string()))
            .collect(),
        gap: None,
    };
    let mut projected = earlier.clone();
    for item in read.items {
        let Some(fields) = item.value.items() else {
            continue;
        };
        let token = match item.key_str() {
            Ok(token) => token.to_owned(),
            Err(why) => {
                result.gap = Some(format!("scientist trait token: {why}"));
                break;
            }
        };
        if !projected.insert(Token::c_str(token.as_bytes())) {
            continue;
        }
        let mut found = ScientistTrait {
            token,
            ..ScientistTrait::default()
        };
        let mut unheld = Unheld::default();
        for field in fields {
            if field.keyword_is("name") {
                found.name = unheld
                    .keep("scientist trait name", field.value.read_str())
                    .map(str::to_owned);
            } else if field.keyword_is("modifier") {
                found
                    .modifiers
                    .read_all(field.value.items().unwrap_or_default());
            }
        }
        let stream_refusal = || {
            stop.as_ref()
                .filter(|(at, _)| *at == result.traits.len())
                .map(|(_, why)| why.clone())
        };
        found.name_refusal = unheld.cause().map(str::to_owned).or_else(stream_refusal);
        unheld.absorb(&found.modifiers.unheld);
        found.refusal = unheld.cause().map(str::to_owned).or_else(stream_refusal);
        let refused = found.refusal.clone();
        result.traits.push(found);
        if let Some(why) = refused {
            result.gap = Some(why);
            break;
        }
    }
    result
}

pub fn read(layers: &ResolvedLayers) -> io::Result<(ScientistTraits, Vec<content_reader_parser::Diagnostic>)> {
    let names = layers.key_names()?;
    let mut result = ScientistTraits::default();
    let mut notes = Vec::new();
    let mut earlier = BTreeSet::new();
    for path in script_files::paths(layers, "common/scientist_traits/", Some(".txt"))? {
        result.coverage.next_file();
        let file = parse(&path.read(layers)?, &names, &earlier);
        notes.extend(
            file.notes
                .into_iter()
                .map(|(kind, why)| (kind, format!("Scientist traits in {path}: {why}"))),
        );
        for found in file.traits {
            let token = Token::c_str(found.token.as_bytes());
            result.coverage.add(&key(&token));
            earlier.insert(token.clone());
            result.definitions.entry(token).or_insert(found);
        }
        if let Some(why) = file.gap {
            result.coverage.gap(&path.text(), why.clone());
            notes.push((
                content_reader_parser::DiagnosticKind::Unsettled,
                format!("Scientist traits in {path}: {why}; unread definitions are unknown"),
            ));
        }
    }
    Ok((result, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scientist_traits_read_names_modifiers_variables_and_skips() {
        content_reader_script::test_game::enter();
        let file = parse(
            br#"@v = 0.2 trait = { name = First NAME = final icon = art
          specialization = { land air } available = { always = yes }
          modifier = { research_speed_factor = @v } modifier = { research_speed_factor = 0.3 }
          ignored = { name = wrong } } empty = { name = "" }"#,
            &KeyNames::default(),
            &BTreeSet::new(),
        );
        assert!(file.gap.is_none(), "{:?}", file.gap);
        assert_eq!(file.traits[0].name.as_deref(), Some("final"));
        assert_eq!(file.traits[0].modifiers.values[0].value, 30_000);
        assert_eq!(file.traits[1].name.as_deref(), Some(""));
    }

    #[test]
    fn scientist_trait_repeated_tokens_keep_first_and_skip_their_variables() {
        content_reader_script::test_game::enter();
        let file = parse(
            b"trait = { name = first } TRAIT = { @n = wrong name = second } other = { name = @n }",
            &KeyNames::default(),
            &BTreeSet::new(),
        );
        assert_eq!(file.traits.len(), 2);
        assert_eq!(file.traits[0].name.as_deref(), Some("first"));
        assert_eq!(file.traits[1].name.as_deref(), Some("@n"));
        let earlier = BTreeSet::from([Token::c_str(b"trait")]);
        assert!(
            parse(
                b"TRAIT = { name = changed }",
                &KeyNames::default(),
                &earlier
            )
            .traits
            .is_empty()
        );
    }

    #[test]
    fn scientist_trait_uncertain_modifier_consumption_is_named() {
        content_reader_script::test_game::enter();
        let mut names = KeyNames::default();
        names.effects.modifiers_gap = Some("synthetic registry suffix".into());
        let file = parse(b"known = { name = known } uncertain = { modifier = { unknown = { name = x } } } tail = { name = tail }",
            &names, &BTreeSet::new());
        assert_eq!(file.traits.len(), 2);
        assert!(file.traits[1].refusal.as_ref().unwrap().contains("unknown"));
        assert!(file.gap.unwrap().contains("modifier value reader"));
    }

    #[test]
    fn scientist_trait_name_does_not_depend_on_unheld_modifier_text() {
        content_reader_script::test_game::enter();
        let file = parse(
            b"trait = { name = valid modifier = { research_speed_factor = \xfe } }",
            &KeyNames::default(),
            &BTreeSet::new(),
        );
        assert_eq!(file.traits[0].display_key(), Ok("valid"));
        assert!(file.traits[0].refusal.is_some());
        let unnamed = ScientistTrait {
            token: "trait".into(),
            name: Some(String::new()),
            ..ScientistTrait::default()
        };
        assert_eq!(unnamed.display_key(), Ok("trait"));
    }

    #[test]
    fn scientist_traits_load_the_winning_files_and_first_registered_tokens() {
        content_reader_script::test_game::enter();
        let root =
            std::env::temp_dir().join(format!("content-reader-scientist-traits-{}", std::process::id()));
        let base = root.join("base");
        let over = root.join("over");
        for (dir, file, text) in [
            (&base, "a.txt", "trait = { name = base }"),
            (&over, "a.txt", "trait = { name = winning }"),
            (
                &base,
                "b.txt",
                "TRAIT = { name = later } other = { name = other }",
            ),
        ] {
            let path = dir.join("common/scientist_traits").join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])]).unwrap();
        let (database, notes) = read(&layers).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(
            database.get("TrAiT").unwrap().unwrap().name.as_deref(),
            Some("winning")
        );
        assert_eq!(
            database.get("other").unwrap().unwrap().name.as_deref(),
            Some("other")
        );
        assert_eq!(database.get("missing").unwrap(), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
