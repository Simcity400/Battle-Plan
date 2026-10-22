//! Script constants loaded from `common/script_constants`, used by `constant:` operands
//! ([`crate::scoped_variable`]).
//!
//! This database loads before other script databases, so triggers, effects and text see
//! the complete set. Direct `.txt` files load in filename-byte order
//! ([`script_files::paths`]). Each top-level key identifies a category by its
//! ASCII-folded token.
//!
//! A category requires `schema` first; other first entries report and skip the block.
//! After reading the schema, remaining entries populate the category root. Repeated
//! categories merge into the first; repeated keys replace the whole entry.
//!
//! Bodies read by key ([`native::Shape::ByKey`]); keyless schema children and array
//! elements use list consumption. Unsettled forms remain named refusals.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    crt::{Scanned, scan_i32},
    fixed5::{decimal_i32, script_fixed5},
    parser::{
        Item,
        native::{self, Shape},
    },
    scoped_variable::Constant,
    script_files,
};

/// Token identity uses an ASCII case fold. Empty names and names starting with a digit
/// or `-` share the rejected token, represented here as the empty key, so those keys
/// collide.
fn token(text: &[u8]) -> Vec<u8> {
    match text.first() {
        None | Some(b'0'..=b'9' | b'-') => Vec::new(),
        Some(_) => text.to_ascii_lowercase(),
    }
}

/// A leaf's declared type token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Int,
    FixedPoint,
    Token,
    Country,
    State,
}

/// One schema node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Schema {
    /// `key = X`.
    key: Option<Vec<u8>>,
    /// `any_key` with any value.
    any_key: bool,
    /// A leaf of this type, optionally an `array`. `None` is an
    /// object (variant 1) with `children`.
    leaf: Option<(Kind, bool)>,
    children: Vec<Schema>,
}

/// A stored value node: its type and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    /// Type 0: `int`, and every scalar that failed or is unsupported (i32).
    Int(i32),
    /// Type 1: `fixed_point`, fixed5.
    Fixed(i64),
    /// Non-scope arrays (5, 6), which a scoped variable does not take.
    Other,
    Collection(CollectionConstant),
}

/// Stored value types accepted by collection inputs.
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
    /// A node the port cannot read as the game does, and why.
    Refused(String),
}

/// One category: its root map, or why the port cannot read it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Category {
    root: BTreeMap<Vec<u8>, Node>,
    refused: Option<String>,
}

/// The database: every category by its token.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Constants {
    categories: BTreeMap<Vec<u8>, Category>,
    /// A file whose reading the port cannot follow: every later category is
    /// unknown.
    unsettled: Option<String>,
}

impl Constants {
    fn find_value(&self, parts: &[&str]) -> Result<Option<&Value>, &str> {
        let Some((category, path)) = parts.split_first() else {
            return Ok(None);
        };
        let Some(found) = self.categories.get(&token(category.as_bytes())) else {
            return self.unsettled.as_deref().map_or(Ok(None), Err);
        };
        if let Some(why) = &found.refused {
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

    /// Read an integer constant, rejecting fixed-point values even when numerically
    /// integral. The caller retains its field on refusal.
    pub fn integer(&self, parts: &[&str]) -> Result<Option<i32>, String> {
        match self.find_value(parts).map_err(str::to_owned)? {
            Some(Value::Int(value)) => Ok(Some(*value)),
            _ => Ok(None),
        }
    }

    /// Find a `constant:` path, then enforce scoped-variable types. Fixed5 literals
    /// remain stored values; integer literals are multiplied by 100000. Other types
    /// report and remain unset.
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
            // The walk ended on a map: too few parts.
            _ => Constant::Unset(format!("missing constant value: {shown}")),
        }
    }

    /// Keep a constant path's country/state scalar or array variant. Scoped-variable
    /// operands deliberately reject these variants.
    pub fn collection(&self, parts: &[&str]) -> Result<Option<&CollectionConstant>, String> {
        match self.find_value(parts).map_err(str::to_owned)? {
            Some(Value::Collection(value)) => Ok(Some(value)),
            _ => Ok(None),
        }
    }
}

/// The database of the content stack.
pub fn read(layers: &ResolvedLayers) -> io::Result<Constants> {
    let mut files = Vec::new();
    for path in script_files::paths(layers, "common/script_constants/", Some(".txt")) {
        files.push((path.to_string(), path.read(layers)?));
    }
    Ok(parse(
        &files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>(),
    ))
}

/// The database from files in load order.
pub fn parse(files: &[(&str, &[u8])]) -> Constants {
    let mut out = Constants::default();
    for (path, input) in files {
        if out.unsettled.is_some() {
            break;
        }
        // Use lexer mode 0 for script constants.
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
            let category = out
                .categories
                .entry(token(key))
                .or_insert_with(|| Category {
                    root: BTreeMap::new(),
                    refused: None,
                });
            if category.refused.is_none()
                && let Err(why) = read_category(item, &mut category.root)
            {
                category.refused = Some(format!("{path}: {why}"));
            }
        }
        if let Some(unsettled) = read.unsettled {
            out.unsettled = Some(format!("{path}: {unsettled}"));
        }
    }
    out
}

/// Read one category block into `root`.
fn read_category(item: &Item, root: &mut BTreeMap<Vec<u8>, Node>) -> Result<(), String> {
    let name = String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()).into_owned();
    let Some(fields) = item.value.items() else {
        return Err(format!("category {name} is not a block"));
    };
    let Some((first, entries)) = fields.split_first() else {
        return Ok(());
    };
    if !first.keyword_is("schema") {
        // A missing initial schema skips the block. A new category keeps
        // no node, which cannot be read through.
        return Err(format!(
            "category {name} does not start with its schema, and the port does not read the category the game leaves"
        ));
    }
    let Some(schema) = first.value.items() else {
        return Err(format!("category {name}'s schema is not an object"));
    };
    let schema = read_schema(schema).map_err(|why| format!("category {name}'s schema: {why}"))?;
    for entry in entries {
        read_entry(entry, &schema, root);
    }
    Ok(())
}

/// Read a schema node, refusing reported forms by name when their resulting state
/// cannot be followed.
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
                None => schema.leaf = Some((kind(value)?, false)),
            },
            b"array" => schema.leaf = Some((kind(value)?, true)),
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

fn kind(value: Option<&[u8]>) -> Result<Kind, String> {
    Ok(match value.map(<[u8]>::to_ascii_lowercase).as_deref() {
        Some(b"int") => Kind::Int,
        Some(b"fixed_point") => Kind::FixedPoint,
        Some(b"token") => Kind::Token,
        Some(b"country") => Kind::Country,
        Some(b"state") => Kind::State,
        _ => return Err("unsupported constant data type".into()),
    })
}

/// Read an entry under `schema` into `map`.
fn read_entry(entry: &Item, schema: &Schema, map: &mut BTreeMap<Vec<u8>, Node>) {
    let Some(key) = entry.key.as_deref() else {
        return;
    };
    let key = token(key);
    // A key that fails the schema match is not stored.
    if !schema.any_key && schema.key.as_deref() != Some(key.as_slice()) {
        return;
    }
    let node = match schema.leaf {
        Some((kind, array)) => Node::Value(value(entry, kind, array)),
        // A nonobject value stores nothing.
        None => {
            let Some(fields) = entry.value.items() else {
                return;
            };
            let mut inner = BTreeMap::new();
            for field in fields {
                if field.key.is_none() {
                    map.insert(
                        key,
                        Node::Refused("an object entry holds a value without a key".into()),
                    );
                    return;
                }
                let field_key = token(field.key.as_deref().unwrap_or_default());
                // The first child whose key matches or that takes any key;
                // If none matches, skip the unknown key.
                if let Some(child) = schema.children.iter().find(|child| {
                    child.any_key || child.key.as_deref() == Some(field_key.as_slice())
                }) {
                    read_entry(field, child, &mut inner);
                }
            }
            Node::Map(inner)
        }
    };
    map.insert(key, node);
}

/// Failed or unsupported scalar conversion stores integer 0, including array elements.
fn value(entry: &Item, kind: Kind, array: bool) -> Value {
    if array {
        return match (kind, entry.value.items()) {
            (Kind::Token, _) | (_, None) => Value::Int(0),
            (Kind::Country, Some(items)) => Value::Collection(CollectionConstant::Countries(
                items
                    .iter()
                    .map(|item| item.value.scalar().unwrap_or_default().to_vec())
                    .collect(),
            )), // Retain null tags.
            (Kind::State, Some(items)) => Value::Collection(CollectionConstant::States(
                items
                    .iter()
                    .map(|item| decimal_i32(item.value.scalar().unwrap_or_default()))
                    .collect(),
            )), // Retain zero IDs.
            _ => Value::Other,
        };
    }
    let Some(text) = entry.value.scalar() else {
        return Value::Int(0);
    };
    match kind {
        // Signed `%i` scanning treats every nonzero return as success.
        Kind::Int => Value::Int(match scan_i32(text) {
            Scanned::Stored(value) => value,
            Scanned::End | Scanned::NoMatch => 0,
        }),
        // Use script fixed5 conversion.
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
        // `%i` stops at the dot: an int schema keeps 1.
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

    /// A repeated category merges into the first; a repeated key is
    /// replaced whole.
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
