use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_script::native::{self, Shape};
use crate::{
    ResolvedLayers,
    fields::invalid,
    key_names::KeyNames,
    parser::{
        self, c_string,
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
    terms: Vec<Terms>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terms {
    pub annex: bool,
    pub allowed: Vec<parser::Item>,
    pub available: Vec<parser::Item>,
    pub limits: [i32; 4],
    pub cost_factors: [i64; 4],
    pub threat_factors: [i64; 4],
    pub conditions: [Vec<parser::Item>; 4],
    pub generate_base_cost: i32,
    pub generate_per_state_cost: i32,
    pub expiry_days: i32,
    pub threat: i64,
}
impl Default for Terms {
    fn default() -> Self {
        Self {
            annex: false,
            allowed: Vec::new(),
            available: Vec::new(),
            limits: [0; 4],
            cost_factors: [100_000; 4],
            threat_factors: [100_000; 4],
            conditions: Default::default(),
            generate_base_cost: 0,
            generate_per_state_cost: 0,
            expiry_days: 0,
            threat: 0,
        }
    }
}
impl Terms {
    fn read(items: &[parser::Item]) -> Self {
        let mut out = Self::default();
        for item in items {
            let Some(key) = item.keyword() else { continue };
            let key = key.folded();
            let key = key.as_slice();
            if key == b"allowed" {
                out.allowed
                    .extend_from_slice(item.value.items().unwrap_or_default());
            } else if key == b"available" {
                out.available
                    .extend_from_slice(item.value.items().unwrap_or_default());
            } else if key == b"annex" {
                out.annex = item
                    .value
                    .scalar()
                    .and_then(parser::read_bool)
                    .unwrap_or(out.annex);
            } else if key == b"threat" {
                out.threat = content_reader_script::modifiers::read_fixed5(item).unwrap_or(out.threat);
            } else if let Some(index) = [
                b"take_states".as_slice(),
                b"liberate",
                b"puppet",
                b"force_government",
            ]
            .iter()
            .position(|k| *k == key)
            {
                out.conditions[index].extend_from_slice(item.value.items().unwrap_or_default());
            } else if let Some(index) = [
                b"take_states_limit".as_slice(),
                b"liberate_limit",
                b"puppet_limit",
                b"force_government_limit",
            ]
            .iter()
            .position(|k| *k == key)
            {
                if let Some(value) = item
                    .value
                    .scalar()
                    .and_then(|v| content_reader_parser::crt::scan_i32(v).stored())
                {
                    out.limits[index] = value;
                }
            } else if let Some(index) = [
                b"take_states_cost".as_slice(),
                b"liberate_cost",
                b"puppet_cost",
                b"force_government_cost",
                b"take_states_threat_factor",
                b"liberate_threat_factor",
                b"puppet_threat_factor",
                b"force_government_threat_factor",
            ]
            .iter()
            .position(|k| *k == key)
            {
                let factor = if index < 4 {
                    &mut out.cost_factors[index]
                } else {
                    &mut out.threat_factors[index - 4]
                };
                let value = content_reader_script::modifiers::read_fixed5(item).unwrap_or(*factor);
                *factor = value
                    .wrapping_mul(1000)
                    .wrapping_div(100_000)
                    .wrapping_add(100_000);
            } else {
                let field = match key {
                    b"generate_base_cost" => &mut out.generate_base_cost,
                    b"generate_per_state_cost" => &mut out.generate_per_state_cost,
                    b"expire" => &mut out.expiry_days,
                    _ => continue,
                };
                if let Some(value) = item
                    .value
                    .scalar()
                    .and_then(|v| content_reader_parser::crt::scan_i32(v).stored())
                {
                    *field = value;
                }
            }
        }
        out
    }
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
    pub fn terms(&self, name: &[u8]) -> Result<&Terms, String> {
        self.find(name)?
            .ok_or_else(|| "war-goal definition is absent from recorded content".to_string())?;
        let mut matches = self
            .names
            .iter()
            .zip(&self.terms)
            .filter(|(n, _)| Token::c_str(n) == Token::c_str(name))
            .map(|(_, t)| t);
        let first = matches
            .next()
            .ok_or_else(|| "war-goal terms are unavailable".to_string())?;
        if matches.any(|other| other != first) {
            return Err("war-goal definition occurrence has differing terms".into());
        }
        Ok(first)
    }
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
        terms: read
            .items
            .iter()
            .filter(|item| item.keyword_is("wargoal_types"))
            .flat_map(|item| item.value.items().unwrap_or_default())
            .map(|item| Terms::read(item.value.items().unwrap_or_default()))
            .collect(),
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
    for path in script_files::paths(layers, "common/wargoals/", Some(".txt"))? {
        match parse(&path.read(layers)?, &names) {
            Ok(parsed) => {
                database.prefixes.extend(parsed.prefixes);
                database.terms.extend(parsed.terms);
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
    fn wargoal_terms_keep_native_defaults_repeated_assignments_and_mod_definition_ambiguity() {
        use content_reader_script::{test_game, trigger_keys::Reader};
        test_game::with(|game| game.trigger("tag", 0, Reader::Assign, 0));
        let database = parse(b"@cost=25 wargoal_types={custom={annex=yes annex=invalid take_states_limit=3 take_states_cost=@cost take_states_cost=-12.5 liberate_threat_factor=50 expire=90 generate_base_cost=12 generate_per_state_cost=4 take_states={always=yes} allowed={always=yes} allowed={always=no} available={tag=MOD}}}", &KeyNames::default()).unwrap();
        let term = database.terms(b"custom").unwrap();
        assert!(term.annex);
        assert_eq!(term.limits, [3, 0, 0, 0]);
        assert_eq!(term.cost_factors, [87500, 100000, 100000, 100000]);
        assert_eq!(term.threat_factors, [100000, 150000, 100000, 100000]);
        assert_eq!(
            (
                term.expiry_days,
                term.generate_base_cost,
                term.generate_per_state_cost
            ),
            (90, 12, 4)
        );
        assert_eq!(term.conditions[0].len(), 1);
        assert_eq!(term.allowed.len(), 2);
        assert_eq!(term.available.len(), 1);
        assert!(term.allowed[0].keyword_is("always"));
        assert_eq!(term.allowed[0].value.scalar(), Some(b"yes".as_slice()));
        assert_eq!(term.allowed[1].value.scalar(), Some(b"no".as_slice()));
        assert!(term.available[0].keyword_is("tag"));
        assert_eq!(term.available[0].value.scalar(), Some(b"MOD".as_slice()));
        for field in ["allowed", "available"] {
            let source = format!(
                "wargoal_types={{custom={{{field}={{always=yes}}}} CUSTOM={{{field}={{always=no}}}}}}"
            );
            assert!(
                parse(source.as_bytes(), &KeyNames::default())
                    .unwrap()
                    .terms(b"custom")
                    .unwrap_err()
                    .contains("differing terms")
            );
        }
        let duplicate = parse(
            b"wargoal_types={custom={annex=yes} CUSTOM={annex=no}}",
            &KeyNames::default(),
        )
        .unwrap();
        assert!(
            duplicate
                .terms(b"custom")
                .unwrap_err()
                .contains("differing terms")
        );
        let identical =
            parse(b"wargoal_types={custom={} CUSTOM={}}", &KeyNames::default()).unwrap();
        assert_eq!(identical.terms(b"custom").unwrap(), &Terms::default());
    }

    #[test]
    fn war_naming_keeps_main_reader_forms_and_token_identity() {
        content_reader_script::test_game::enter();
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
    fn wargoal_names_and_field_consumption() {
        content_reader_script::test_game::enter();
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
