//! NScript::CConstantDatabase) a `constant:` operand reads
//! ([`crate::scoped_variable`]).
//!
//! CBuildingDatabase::load (14018B610) loads it first among the script
//! databases, so every trigger, effect and text reads the whole database:
//! InitFromDirectory (14013F7D0) lists the folder's `.txt` files without
//! recursion in name-byte order ([`script_files::paths`]), and ReadFile
//! (14012C750) reads each top-level key as a category, found by its token
//! (AddOrFindDynamicToken 1424A6500, without ASCII case) or created.
//! CConstant::Read (140A98140) wants `schema` first (static token 0x468D;
//! otherwise "A constant must define a schema as first entry" and the block
//! is skipped), reads the schema (ReadSchemaNode 140A99110), then each other
//! entry into the category's root (ReadEntry 140A98600): a repeated
//! category merges into the first, a repeated key is replaced whole.
//!
//! The bodies are read by key ([`native::Shape::ByKey`]); a keyless block (a
//! schema's `data` children, an array's elements) is kept as the list reader
//! reads it. What the port cannot settle is kept as a named refusal.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    crt::{Scanned, scan_i32},
    fixed5::script_fixed5,
    parser::{
        Item,
        native::{self, Shape},
    },
    scoped_variable::Constant,
    script_files,
};

/// A token as the lexer finds it (1424A65A0): the text without ASCII case.
/// A text that is empty or starts with a digit or `-` gets token 0 ("Trying
/// to add empty dynamic token", "keywords may not start with a digit or
/// '-'"), here the empty key, so all such keys collide as they do natively.
fn token(text: &[u8]) -> Vec<u8> {
    match text.first() {
        None | Some(b'0'..=b'9' | b'-') => Vec::new(),
        Some(_) => text.to_ascii_lowercase(),
    }
}

/// A leaf's type token (table 0x143098420).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Int,
    FixedPoint,
    Token,
    Country,
    State,
}

/// A schema node (ReadSchemaNode 140A99110, 0x28 bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Schema {
    /// +0x24, `key = X`.
    key: Option<Vec<u8>>,
    /// +0x20, `any_key` with any value.
    any_key: bool,
    /// +0x18 variant 0: a leaf of this type; +0x21 `array`. `None` is an
    /// object (variant 1) with `children`.
    leaf: Option<(Kind, bool)>,
    children: Vec<Schema>,
}

/// A stored value node: its type byte (+0x18) and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    /// Type 0: `int`, and every scalar that failed or is unsupported (i32).
    Int(i32),
    /// Type 1: `fixed_point`, fixed5.
    Fixed(i64),
    /// Types 3, 4 and the arrays (5, 6, 8, 9), which a scoped variable does
    /// not take.
    Other,
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
    /// FindValue (140A96B20) of a `constant:` path's parts, then the scoped
    /// variable's type test in 140539B50: type 1 is the literal as stored,
    /// type 0 the integer times 100000, any other logged ("Scoped variable
    /// constant only supports Fixed Points") and unset.
    pub fn value(&self, parts: &[&str]) -> Constant {
        let shown = parts.join(".");
        let Some((category, path)) = parts.split_first() else {
            return Constant::Unset(format!("constant:{shown} names no category"));
        };
        let unknown = |why: &str| {
            Constant::Unset(format!(
                "constant:{shown} cannot be read as the game reads it: {why}"
            ))
        };
        let Some(found) = self.categories.get(&token(category.as_bytes())) else {
            if let Some(why) = &self.unsettled {
                return unknown(why);
            }
            return Constant::Unset(format!("No constant category: {category}"));
        };
        if let Some(why) = &found.refused {
            return unknown(why);
        }
        let mut map = &found.root;
        let mut node = None;
        for (at, part) in path.iter().enumerate() {
            match map.get(&token(part.as_bytes())) {
                Some(Node::Map(next)) => {
                    map = next;
                    node = None;
                }
                Some(Node::Refused(why)) => return unknown(why),
                Some(value @ Node::Value(_)) if at + 1 == path.len() => node = Some(value),
                // A value before the last part, or a missing key.
                _ => return Constant::Unset(format!("No constant value: {shown}")),
            }
        }
        match node {
            Some(Node::Value(Value::Fixed(value))) => Constant::Value(*value),
            Some(Node::Value(Value::Int(value))) => Constant::Value(i64::from(*value) * 100_000),
            Some(Node::Value(Value::Other)) => Constant::Unset(format!(
                "constant:{shown}: Scoped variable constant only supports Fixed Points"
            )),
            // The walk ended on a map: too few parts.
            _ => Constant::Unset(format!("No constant value: {shown}")),
        }
    }
}

/// The database of the content stack.
pub fn read(layers: &ResolvedLayers) -> io::Result<Constants> {
    let mut files = Vec::new();
    for path in script_files::paths(layers, "common/script_constants/", Some(".txt")) {
        files.push((path.clone(), layers.read(&path)?));
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
        // 14013F7D0's reader is constructed in mode 0.
        let read = native::read(input, native::Mode::KeepMark, (), &mut |(), _| Shape::ByKey);
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

/// CConstant::Read (140A98140) of one category block into `root`.
fn read_category(item: &Item, root: &mut BTreeMap<Vec<u8>, Node>) -> Result<(), String> {
    let name = String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()).into_owned();
    let Some(fields) = item.value.items() else {
        return Err(format!("category {name} is not a block"));
    };
    let Some((first, entries)) = fields.split_first() else {
        return Ok(());
    };
    if !first.keyword_is("schema") {
        // "A constant must define a schema as first entry": the block is
        // skipped; a new category keeps no node, which the port does not
        // read through.
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

/// ReadSchemaNode (140A99110). A form it reports is refused by name: what
/// the node holds after it is not ported.
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
        _ => return Err("Invalid data type".into()),
    })
}

/// ReadEntry (140A98600) of one entry under `schema` into `map`.
fn read_entry(entry: &Item, schema: &Schema, map: &mut BTreeMap<Vec<u8>, Node>) {
    let Some(key) = entry.key.as_deref() else {
        return;
    };
    let key = token(key);
    // "doesn't match expected key": not stored.
    if !schema.any_key && schema.key.as_deref() != Some(key.as_slice()) {
        return;
    }
    let node = match schema.leaf {
        Some((kind, array)) => Node::Value(value(entry, kind, array)),
        // "Expected an object": nothing is stored.
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
                // none: "Unknown key for schema", skipped.
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

/// ParseScalar (140A97750) and ParseArray (140A96E10): a failed or
/// unsupported scalar is an int 0.
fn value(entry: &Item, kind: Kind, array: bool) -> Value {
    if array {
        return match (kind, entry.value.items()) {
            (Kind::Token, _) | (_, None) => Value::Int(0),
            _ => Value::Other,
        };
    }
    let Some(text) = entry.value.scalar() else {
        return Value::Int(0);
    };
    match kind {
        // sscanf `%i` (1424B02C0): any nonzero return is taken.
        Kind::Int => Value::Int(match scan_i32(text) {
            Scanned::Stored(value) => value,
            Scanned::End | Scanned::NoMatch => 0,
        }),
        // read_fixed5 (1424B0480).
        Kind::FixedPoint => script_fixed5(text).map_or(Value::Int(0), Value::Fixed),
        Kind::Country | Kind::State => Value::Other,
        Kind::Token => Value::Int(0),
    }
}

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
            Constant::Unset("No constant value: sp.small".into())
        );
        assert_eq!(
            constants.value(&["sp", "wrong", "max"]),
            Constant::Unset("No constant value: sp.wrong.max".into())
        );
        assert_eq!(
            constants.value(&["groups", "nordics"]),
            Constant::Unset(
                "constant:groups.nordics: Scoped variable constant only supports Fixed Points"
                    .into()
            )
        );
        assert_eq!(
            constants.value(&["missing", "x"]),
            Constant::Unset("No constant category: missing".into())
        );
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
            Constant::Unset("No constant value: propaganda.SIA.empty_slot".into())
        );
        assert_eq!(
            constants.value(&["propaganda", "GER", "empty_slot"]),
            Constant::Value(200_000)
        );
    }
}
