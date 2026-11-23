
use std::{collections::BTreeMap, io};

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{crt::{Scanned, scan_i32}, fixed5::{decimal_i32, script_fixed5}, parser::Item};
use content_reader_script::{native::{self, Shape}, scoped_variable::Constant};

fn token(text: &[u8]) -> Vec<u8> {
    match text.first() {
        None | Some(b'0'..=b'9' | b'-') => Vec::new(),
        Some(_) => text.to_ascii_lowercase(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Int,
    FixedPoint,
    Token,
    Country,
    State,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Schema {
    key: Option<Vec<u8>>,
    any_key: bool,
    leaf: Option<(Kind, bool)>,
    children: Vec<Schema>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Int(i32),
    Fixed(i64),
    Other,
    Collection(CollectionConstant),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectionConstant {
    Country(Vec<u8>),
    State(i32),
    Countries(Vec<Vec<u8>>),
    States(Vec<i32>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Value(Value),
    Map(BTreeMap<Vec<u8>, Node>),
    Refused(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Category {
    root: BTreeMap<Vec<u8>, Node>,
    refused: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Constants {
    categories: BTreeMap<Vec<u8>, Category>,
    defined: BTreeMap<(Vec<u8>, Vec<u8>), usize>,
    gaps: Vec<(usize, String)>,
}

impl Constants {
    fn gap(&self, from: Option<usize>) -> Option<&str> {
        self.gaps
            .iter()
            .rev()
            .find(|(file, _)| from.is_none_or(|from| *file >= from))
            .map(|(_, why)| why.as_str())
    }

    fn find_value(&self, parts: &[&str]) -> Result<Option<&Value>, &str> {
        let Some((category, path)) = parts.split_first() else {
            return Ok(None);
        };
        let category = token(category.as_bytes());
        let Some(found) = self.categories.get(&category) else {
            return self.gap(None).map_or(Ok(None), Err);
        };
        if let Some(why) = &found.refused {
            return Err(why);
        }
        if let Some(entry) = path.first()
            && let Some(why) = self.gap(
                self.defined
                    .get(&(category, token(entry.as_bytes())))
                    .copied(),
            )
        {
            return Err(why);
        }
        let mut map = &found.root;
        for (at, part) in path.iter().enumerate() {
            match map.get(&token(part.as_bytes())) {
                Some(Node::Map(next)) => map = next,
                Some(Node::Refused(why)) => return Err(why),
                Some(Node::Value(value)) if at + 1 == path.len() => return Ok(Some(value)),
                _ => return Ok(None),
            }
        }
        Ok(None)
    }

    pub fn integer(&self, parts: &[&str]) -> Result<Option<i32>, String> {
        match self.find_value(parts).map_err(str::to_owned)? {
            Some(Value::Int(value)) => Ok(Some(*value)),
            _ => Ok(None),
        }
    }

    pub fn value(&self, parts: &[&str]) -> Constant {
        let shown = parts.join(".");
        let Some((category, _)) = parts.split_first() else {
            return Constant::Unset(format!("constant:{shown} names no category"));
        };
        let unknown = |why: &str| {
            Constant::Unset(format!(
                "constant:{shown} cannot be read as the game reads it: {why}"
            ))
        };
        match self.find_value(parts) {
            Err(why) => unknown(why),
            Ok(Some(Value::Fixed(value))) => Constant::Value(*value),
            Ok(Some(Value::Int(value))) => Constant::Value(i64::from(*value) * 100_000),
            Ok(Some(Value::Other | Value::Collection(_))) => Constant::Unset(format!(
                "constant:{shown} is not an integer or fixed5 value"
            )),
            Ok(None) if !self.categories.contains_key(&token(category.as_bytes())) => {
                Constant::Unset(format!("missing constant category: {category}"))
            }
            _ => Constant::Unset(format!("missing constant value: {shown}")),
        }
    }

    pub fn collection(&self, parts: &[&str]) -> Result<Option<&CollectionConstant>, String> {
        match self.find_value(parts).map_err(str::to_owned)? {
            Some(Value::Collection(value)) => Ok(Some(value)),
            _ => Ok(None),
        }
    }
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Constants> {
    let mut files = Vec::new();
    for path in script_files::paths(layers, "common/script_constants/", Some(".txt"))? {
        files.push((path.to_string(), path.read(layers)?));
    }
    Ok(parse(
        &files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>(),
    ))
}

pub fn parse(files: &[(&str, &[u8])]) -> Constants {
    let mut out = Constants::default();
    for (file, (path, input)) in files.iter().enumerate() {
        let read = native::read(
            input,
            native::Mode::KeepMark,
            native::Top::File(()),
            None,
            &mut |(), _| Shape::ByKey,
        );
        for item in &read.items {
            let Some(key) = item.key.as_deref() else {
                continue;
            };
            let name = token(key);
            let category = out
                .categories
                .entry(name.clone())
                .or_insert_with(|| Category {
                    root: BTreeMap::new(),
                    refused: None,
                });
            if category.refused.is_none() {
                match read_category(item, &mut category.root) {
                    Ok(stored) => {
                        for entry in stored {
                            out.defined.insert((name.clone(), entry), file);
                        }
                    }
                    Err(why) => category.refused = Some(format!("{path}: {why}")),
                }
            }
        }
        if let Some(unsettled) = read.unsettled {
            out.gaps.push((
                file,
                format!(
                    "{path} is unknown from {unsettled}, so a constant it could define or replace there is unknown"
                ),
            ));
        }
    }
    out
}

fn read_category(item: &Item, root: &mut BTreeMap<Vec<u8>, Node>) -> Result<Vec<Vec<u8>>, String> {
    let name = String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()).into_owned();
    let Some(fields) = item.value.items() else {
        return Err(format!("category {name} is not a block"));
    };
    let Some((first, entries)) = fields.split_first() else {
        return Ok(Vec::new());
    };
    if !first.keyword_is("schema") {
        return Err(format!(
            "category {name} does not start with its schema, and the port does not read the category the game leaves"
        ));
    }
    let Some(schema) = first.value.items() else {
        return Err(format!("category {name}'s schema is not an object"));
    };
    let schema = read_schema(schema).map_err(|why| format!("category {name}'s schema: {why}"))?;
    Ok(entries
        .iter()
        .filter_map(|entry| read_entry(entry, &schema, root))
        .collect())
}

fn read_schema(fields: &[Item]) -> Result<Schema, String> {
    let mut schema = Schema::default();
    for field in fields {
        let Some(keyword) = field.keyword() else {
            return Err("a schema field has no keyword".into());
        };
        let value = field.value.scalar();
        match keyword.folded().as_slice() {
            b"key" | b"any_key" if schema.key.is_some() || schema.any_key => {
                return Err("key or key_type already set".into());
            }
            b"key" => schema.key = Some(token(value.unwrap_or_default())),
            b"any_key" => schema.any_key = true,
            b"data" => match field.value.items() {
                Some(children) => {
                    for child in children {
                        let Some(fields) = child.value.items().filter(|_| child.key.is_none())
                        else {
                            return Err("a data child is not a block without a key".into());
                        };
                        schema.children.push(read_schema(fields)?);
                    }
                }
                None => schema.leaf = Some((kind(field.value_keyword())?, false)),
            },
            b"array" => schema.leaf = Some((kind(field.value_keyword())?, true)),
            other => {
                return Err(format!(
                    "the schema key {} is reported and skipped",
                    String::from_utf8_lossy(other)
                ));
            }
        }
    }
    Ok(schema)
}

fn kind(value: Option<String>) -> Result<Kind, String> {
    Ok(match value.as_deref() {
        Some("int") => Kind::Int,
        Some("fixed_point") => Kind::FixedPoint,
        Some("token") => Kind::Token,
        Some("country") => Kind::Country,
        Some("state") => Kind::State,
        _ => return Err("unsupported constant data type".into()),
    })
}

fn read_entry(entry: &Item, schema: &Schema, map: &mut BTreeMap<Vec<u8>, Node>) -> Option<Vec<u8>> {
    let key = token(entry.key.as_deref()?);
    if !schema.any_key && schema.key.as_deref() != Some(key.as_slice()) {
        return None;
    }
    let node = match schema.leaf {
        Some((kind, array)) => Node::Value(value(entry, kind, array)),
        None => {
            let fields = entry.value.items()?;
            let mut inner = BTreeMap::new();
            for field in fields {
                if field.key.is_none() {
                    map.insert(
                        key.clone(),
                        Node::Refused("an object entry holds a value without a key".into()),
                    );
                    return Some(key);
                }
                let field_key = token(field.key.as_deref().unwrap_or_default());
                if let Some(child) = schema.children.iter().find(|child| {
                    child.any_key || child.key.as_deref() == Some(field_key.as_slice())
                }) {
                    read_entry(field, child, &mut inner);
                }
            }
            Node::Map(inner)
        }
    };
    map.insert(key.clone(), node);
    Some(key)
}

fn value(entry: &Item, kind: Kind, array: bool) -> Value {
    if array {
        return match (kind, entry.value.items()) {
            (Kind::Token, _) | (_, None) => Value::Int(0),
            (Kind::Country, Some(items)) => Value::Collection(CollectionConstant::Countries(
                items
                    .iter()
                    .map(|item| item.value.scalar().unwrap_or_default().to_vec())
                    .collect(),
            )),
            (Kind::State, Some(items)) => Value::Collection(CollectionConstant::States(
                items
                    .iter()
                    .map(|item| decimal_i32(item.value.scalar().unwrap_or_default()))
                    .collect(),
            )),
            _ => Value::Other,
        };
    }
    let Some(text) = entry.value.scalar() else {
        return Value::Int(0);
    };
    match kind {
        Kind::Int => Value::Int(match scan_i32(text) {
            Scanned::Stored(value) => value,
            Scanned::End | Scanned::NoMatch => 0,
        }),
        Kind::FixedPoint => script_fixed5(text).map_or(Value::Int(0), Value::Fixed),
        Kind::Country => Value::Collection(CollectionConstant::Country(text.to_vec())),
        Kind::State => match decimal_i32(text) {
            id if id > 0 => Value::Collection(CollectionConstant::State(id)),
            _ => Value::Int(0),
        },
        Kind::Token => Value::Int(0),
    }
}

#[cfg(test)]
#[path = "script_constants/collections/tests.rs"]
mod collection_tests;

#[cfg(test)]
mod tests {
    use super::*;

    const PROPAGANDA: &str = "propaganda = {
        schema = { any_key = yes data = { { any_key = yes data = int } } }
        SIA = { empty_slot = 9 cost_increase = 1.35 }
    }
    sp = {
        schema = { any_key = yes data = { { key = min data = int } { key = gain data = fixed_point } } }
        small = { min = 20 gain = 0.125 }
        wrong = { max = 5 }
    }
    groups = {
        schema = { any_key = yes array = country }
        nordics = { SWE NOR }
    }";

    fn database() -> Constants {
        parse(&[("common/script_constants/a.txt", PROPAGANDA.as_bytes())])
    }

    #[test]
    fn constants_resolve_as_the_scoped_variable_reads_them() {
        let constants = database();
        assert_eq!(
            constants.value(&["propaganda", "SIA", "empty_slot"]),
            Constant::Value(900_000)
        );
        assert_eq!(
            constants.value(&["propaganda", "sia", "COST_INCREASE"]),
            Constant::Value(100_000)
        );
        assert_eq!(
            constants.value(&["sp", "small", "gain"]),
            Constant::Value(12_500)
        );
        assert_eq!(
            constants.value(&["sp", "small"]),
            Constant::Unset("missing constant value: sp.small".into())
        );
        assert_eq!(
            constants.value(&["sp", "wrong", "max"]),
            Constant::Unset("missing constant value: sp.wrong.max".into())
        );
        assert_eq!(
            constants.value(&["groups", "nordics"]),
            Constant::Unset("constant:groups.nordics is not an integer or fixed5 value".into())
        );
        assert_eq!(
            constants.value(&["missing", "x"]),
            Constant::Unset("missing constant category: missing".into())
        );
    }

    #[test]
    fn integer_lookup_preserves_native_leaf_type() {
        let constants = database();
        assert_eq!(
            constants.integer(&["sp", "small", "min"]).unwrap(),
            Some(20)
        );
        assert_eq!(constants.integer(&["sp", "small", "gain"]).unwrap(), None);
        assert_eq!(constants.integer(&["missing", "value"]).unwrap(), None);
        assert_eq!(constants.integer(&["groups", "nordics"]).unwrap(), None);
    }

    #[test]
    fn a_file_that_stops_short_leaves_what_it_could_replace_unknown() {
        let stops = "propaganda = {
            schema = { any_key = yes data = { { any_key = yes data = int } } }
            GER = { @level = 2 loose }
        }";
        let later = "sp = {
            schema = { any_key = yes data = { { key = min data = int } } }
            small = { min = 7 }
        }";
        let constants = parse(&[
            ("a.txt", PROPAGANDA.as_bytes()),
            ("b.txt", stops.as_bytes()),
            ("c.txt", later.as_bytes()),
        ]);
        assert_eq!(constants.gaps.len(), 1);
        assert_eq!(constants.gaps[0].0, 1);
        for parts in [
            &["propaganda", "SIA", "empty_slot"][..],
            &["propaganda", "GER", "empty_slot"],
            &["propaganda", "absent", "x"],
            &["sp", "wrong", "max"],
            &["groups", "nordics"],
            &["missing", "x"],
        ] {
            let Constant::Unset(why) = constants.value(parts) else {
                panic!("{parts:?} is known");
            };
            assert!(why.contains("b.txt is unknown from"), "{parts:?}: {why}");
            assert!(constants.collection(parts).is_err(), "{parts:?}");
        }
        assert_eq!(
            constants.value(&["sp", "small", "min"]),
            Constant::Value(700_000)
        );
        assert_eq!(constants.integer(&["sp", "small", "min"]).unwrap(), Some(7));
    }

    #[test]
    fn schema_types_dispatch_on_the_value_token() {
        let constants = parse(&[(
            "a.txt",
            &b"sample = { schema = { any_key = yes data = int\0suffix } entry = 7 }\n\
              quoted = { schema = { any_key = yes data = \"int\" } entry = 7 }"[..],
        )]);
        assert_eq!(
            constants.value(&["sample", "entry"]),
            Constant::Value(700_000)
        );
        let Constant::Unset(why) = constants.value(&["quoted", "entry"]) else {
            panic!("a quoted type is read");
        };
        assert!(why.contains("unsupported constant data type"), "{why}");
    }

    #[test]
    fn repeated_categories_merge_and_keys_are_replaced() {
        let later = "propaganda = {
            schema = { any_key = yes data = { { any_key = yes data = fixed_point } } }
            SIA = { cost_increase = 1.35 }
            GER = { empty_slot = 2 }
        }";
        let constants = parse(&[
            ("a.txt", PROPAGANDA.as_bytes()),
            ("b.txt", later.as_bytes()),
        ]);
        assert_eq!(
            constants.value(&["propaganda", "SIA", "cost_increase"]),
            Constant::Value(135_000)
        );
        assert_eq!(
            constants.value(&["propaganda", "SIA", "empty_slot"]),
            Constant::Unset("missing constant value: propaganda.SIA.empty_slot".into())
        );
        assert_eq!(
            constants.value(&["propaganda", "GER", "empty_slot"]),
            Constant::Value(200_000)
        );
    }
}
