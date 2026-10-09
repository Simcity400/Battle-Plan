//! Wargoal type identities from the recorded content stack.
//!
//! 140A2C690 reads direct `.txt` files in filename order with lexer mode 0;
//! 140A2B480 appends every `wargoal_types` declaration. 140A2AEA0 interns its
//! name, and 140A2C5D0 returns the first matching token. Descriptions only need
//! that first declared spelling (+0x18), localised by 140616BC0.

use crate::{
    ResolvedLayers,
    fields::invalid,
    key_names::KeyNames,
    parser::{
        c_string,
        native::{self, Shape},
    },
    script_files,
    tokens::Token,
};
use std::{collections::BTreeMap, io};

#[derive(Default)]
pub struct Database {
    pub names: Vec<Vec<u8>>,
    pub notes: Vec<String>,
    gap: Option<String>,
    prefixes: Vec<(Vec<u8>, Result<String, String>)>,
}

#[derive(Default)]
pub struct Names {
    names: BTreeMap<Token, Result<String, String>>,
}

impl Names {
    pub fn prefix(&self, token: &str) -> Result<&str, String> {
        self.names
            .get(&Token::c_str(token.as_bytes()))
            .ok_or_else(|| format!("war goal type {token:?} is absent from the recorded content"))?
            .as_deref()
            .map_err(Clone::clone)
    }

    fn append(&mut self, name: &[u8], prefix: Result<String, String>) {
        let token = Token::c_str(name);
        match self.names.get(&token) {
            Some(Ok(old)) if prefix.as_ref().is_ok_and(|new| old == new) => (),
            Some(_) => {
                self.names.insert(
                    token,
                    Err(format!(
                        "war goal type {:?} has differing naming overrides in repeated definitions",
                        String::from_utf8_lossy(name)
                    )),
                );
            }
            None => {
                self.names.insert(token, prefix);
            }
        }
    }
}

impl Database {
    pub fn find(&self, name: &[u8]) -> Result<Option<&[u8]>, String> {
        if let Some(why) = &self.gap {
            return Err(format!("wargoal definitions uncertain: {why}"));
        }
        let token = Token::c_str(name);
        Ok(self
            .names
            .iter()
            .find(|entry| Token::c_str(entry) == token)
            .map(Vec::as_slice))
    }
}

/// CWarGoalType::ReadToken 140A2C950. Names and war-name prefixes are projected; its
/// trigger/effect/one-value readers still decide the surrounding consumption.
pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Database> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Types,
        Type,
    }
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File if field.keyword_is("wargoal_types") => Shape::Object(Node::Types),
            Node::File => Shape::Skip,
            Node::Types => Shape::Object(Node::Type),
            Node::Type => match field.keyword().as_deref() {
                Some(
                    b"allowed" | b"available" | b"take_states" | b"liberate" | b"puppet"
                    | b"force_government",
                ) => Shape::Trigger,
                Some(b"instant_reward") => Shape::EffectIn(4),
                Some(
                    b"annex"
                    | b"war_name"
                    | b"expire"
                    | b"threat"
                    | b"take_states_limit"
                    | b"liberate_limit"
                    | b"puppet_limit"
                    | b"force_government_limit"
                    | b"take_states_cost"
                    | b"liberate_cost"
                    | b"puppet_cost"
                    | b"force_government_cost"
                    | b"take_states_threat_factor"
                    | b"liberate_threat_factor"
                    | b"puppet_threat_factor"
                    | b"force_government_threat_factor"
                    | b"generate_base_cost"
                    | b"generate_per_state_cost",
                ) => Shape::One,
                _ => Shape::Skip,
            },
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("wargoals: {cause}")));
    }
    Ok(Database {
        prefixes: read
            .items
            .iter()
            .filter(|item| item.keyword_is("wargoal_types"))
            .flat_map(|item| item.value.items().unwrap_or_default())
            .map(|item| {
                let prefix = item
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .filter(|field| field.keyword_is("war_name"))
                    .map(|field| {
                        field
                            .value
                            .read_str()
                            .map(str::to_owned)
                            .map_err(|error| error.to_string())
                    })
                    .next_back()
                    .unwrap_or_else(|| Ok(String::new()));
                (
                    c_string(item.key.as_deref().unwrap_or_default()).to_vec(),
                    prefix,
                )
            })
            .collect(),
        names: read
            .items
            .iter()
            .filter(|item| item.keyword_is("wargoal_types"))
            .flat_map(|item| item.value.items().unwrap_or_default())
            .map(|item| c_string(item.key.as_deref().unwrap_or_default()).to_vec())
            .collect(),
        notes: read.misreads.iter().map(ToString::to_string).collect(),
        gap: None,
    })
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Database> {
    let names = layers.key_names()?;
    let mut database = Database::default();
    for path in script_files::paths(layers, "common/wargoals/", Some(".txt")) {
        match parse(&layers.read(&path)?, &names) {
            Ok(parsed) => {
                database.prefixes.extend(parsed.prefixes);
                database.names.extend(parsed.names);
                database.notes.extend(parsed.notes);
            }
            Err(why) => {
                let why = format!("Wargoals unavailable in {path}: {why}");
                database.gap.get_or_insert_with(|| why.clone());
                database.notes.push(why);
            }
        }
    }
    Ok(database)
}

/// Naming uses the same 140A2C950 consumption as identity descriptions, including
/// reached instant_reward effects. Differing same-token definitions stay unknown
/// because recorded goal operands do not identify their definition pointer.
pub fn read_names(layers: &ResolvedLayers) -> io::Result<Names> {
    let database = read(layers)?;
    if let Some(why) = database.gap {
        return Err(invalid(why));
    }
    let mut names = Names::default();
    for (name, prefix) in database.prefixes {
        names.append(&name, prefix);
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn war_naming_keeps_main_reader_forms_and_native_token_identity() {
        let database = parse(
            b"wargoal_types={First={instant_reward={set_country_flag=synthetic} take_states_limit=2 war_name=EARLY war_name=LATE} FIRST={war_name=LATE}}",
            &KeyNames::default(),
        )
        .unwrap();
        let mut names = Names::default();
        for (name, prefix) in database.prefixes {
            names.append(&name, prefix);
        }
        assert_eq!(names.prefix("first").unwrap(), "LATE");
        names.append(b"fIrSt", Ok("DIFFERENT".into()));
        assert!(
            names
                .prefix("FIRST")
                .unwrap_err()
                .contains("differing naming overrides")
        );
        assert!(
            names
                .prefix("undefined")
                .unwrap_err()
                .contains("absent from the recorded content")
        );
    }

    #[test]
    fn native_wargoal_names_and_field_consumption() {
        let database = parse(
            b"@cost=8 \"wargoal_types\"={hidden={}} WARGOAL_TYPES={First={allowed={always=no} instant_reward={set_country_flag=synthetic} generate_base_cost=@cost} FIRST={annex=yes} other={take_states={always=yes} unknown={@lost=7} expire=@lost}}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            database.names,
            [b"First".to_vec(), b"FIRST".to_vec(), b"other".to_vec()]
        );
        assert_eq!(database.find(b"first").unwrap(), Some(b"First".as_slice()));
        assert!(database.find(b"undefined").unwrap().is_none());
        // ReadFixed5/ReadString/ReadInt consume one token, retaining a later
        // definition rather than skipping the block after the operand.
        let database = parse(
            b"wargoal_types={one={expire={}} two={}}",
            &KeyNames::default(),
        )
        .unwrap();
        assert!(database.names.contains(&b"one".to_vec()));
    }

    #[test]
    fn wargoal_stack_order_and_replace_path() {
        let root = std::env::temp_dir().join(format!("wargoal-types-{}", std::process::id()));
        let base = root.join("base");
        let over = root.join("over");
        for (dir, path, text) in [
            (&base, "common/wargoals/a.txt", "wargoal_types={hidden={}}"),
            (&over, "common/wargoals/a.txt", "wargoal_types={First={}}"),
            (
                &base,
                "common/wargoals/b.txt",
                "wargoal_types={FIRST={} second={}}",
            ),
            (
                &base,
                "common/wargoals/sub/c.txt",
                "wargoal_types={nested={}}",
            ),
        ] {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])]).unwrap();
        let database = read(&layers).unwrap();
        assert_eq!(
            database.names,
            [b"First".to_vec(), b"FIRST".to_vec(), b"second".to_vec()]
        );
        assert_eq!(database.find(b"FIRST").unwrap(), Some(b"First".as_slice()));
        let layers =
            ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &["common/wargoals"][..])])
                .unwrap();
        assert_eq!(read(&layers).unwrap().names, [b"First".to_vec()]);
        let _ = std::fs::remove_dir_all(root);
    }
}
