
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    io,
};

use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_script::native::{self, Shape};

use crate::{
    ResolvedLayers,
    fields::invalid,
    fixed5::decimal_i32,
    key_names::KeyNames,
    parser::{self, Item},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Group {
    ordered: BTreeMap<i32, Vec<u8>>,
    unordered: Vec<Vec<u8>>,
    fallback: Option<Vec<u8>>,
    prefix: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Table {
    #[default]
    Army,
    Ship,
    Codename,
    RailwayGun,
}

impl Table {
    fn keyword(self) -> &'static str {
        match self {
            Self::Army => "army",
            Self::Ship => "ship",
            Self::Codename => "codename",
            Self::RailwayGun => "railway_gun",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    groups: BTreeMap<Vec<u8>, (Table, Group)>,
    refused: BTreeSet<Vec<u8>>,
}

fn fold(key: &[u8]) -> Vec<u8> {
    key.to_ascii_lowercase()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unresolved {
    NoGroup,
    EmptyKey,
    GroupAbsent,
    NotArmy(Table),
    GroupRefused,
    UnassignedOrder,
    NoFallback,
    NotText,
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoGroup => f.write_str(
                "it has neither a name group nor a stored name, so the game builds a legacy name from the template, which is not reproduced",
            ),
            Self::EmptyKey => f.write_str(
                "its recorded name group key is empty, which a null name group and the group a name file declares with an empty name both give",
            ),
            Self::GroupAbsent => {
                f.write_str("no name file that was read declares its recorded name group")
            }
            Self::NotArmy(table) => write!(
                f,
                "its name group is first declared with type {}, so it is not in the army table a division's group comes from",
                table.keyword()
            ),
            Self::GroupRefused => {
                f.write_str("a name file that declares or could declare its name group was refused")
            }
            Self::UnassignedOrder => f.write_str(
                "its ordered name is not yet assigned (order 0 asks the simulation's allocator)",
            ),
            Self::NoFallback => f.write_str(
                "its group has neither the selected entry nor a non-empty fallback_name, so the game builds a legacy name from the template, which is not reproduced",
            ),
            Self::NotText => f.write_str("its selected name is not UTF-8"),
        }
    }
}

impl Database {
    pub fn resolve(
        &self,
        group_key: impl AsRef<[u8]>,
        order: i32,
        ordered: bool,
    ) -> Result<String, Unresolved> {
        self.resolve_in(group_key.as_ref(), order, ordered, Some(Table::Army))
    }

    pub fn resolve_any(
        &self,
        group_key: impl AsRef<[u8]>,
        order: i32,
        ordered: bool,
    ) -> Result<String, Unresolved> {
        self.resolve_in(group_key.as_ref(), order, ordered, None)
    }

    #[cfg(any(test, feature = "layer-fixtures"))]
    pub fn from_files(files: &[&[u8]]) -> Self {
        let mut database = Self::default();
        for file in files {
            database.apply(file, &KeyNames::default());
        }
        database
    }

    pub fn refused(&self) -> impl Iterator<Item = &[u8]> {
        self.refused.iter().map(Vec::as_slice)
    }

    fn resolve_in(
        &self,
        group_key: &[u8],
        order: i32,
        ordered: bool,
        table: Option<Table>,
    ) -> Result<String, Unresolved> {
        let key = fold(group_key);
        if key.is_empty() {
            return Err(
                if self.refused.contains(&key) || self.groups.contains_key(&key) {
                    Unresolved::EmptyKey
                } else {
                    Unresolved::NoGroup
                },
            );
        }
        if self.refused.contains(&key) {
            return Err(Unresolved::GroupRefused);
        }
        let group = match (self.groups.get(&key), table) {
            (None, _) => return Err(Unresolved::GroupAbsent),
            (Some((held, _)), Some(wanted)) if *held != wanted => {
                return Err(Unresolved::NotArmy(*held));
            }
            (Some((_, group)), _) => group,
        };
        if ordered && order == 0 {
            return Err(Unresolved::UnassignedOrder);
        }
        let name = if let Some(name) = ordered.then(|| group.ordered.get(&order)).flatten() {
            format_number(name, order as u32)
        } else if let Some(name) = (!ordered)
            .then(|| {
                usize::try_from(order)
                    .ok()
                    .and_then(|at| group.unordered.get(at))
            })
            .flatten()
        {
            name.clone()
        } else {
            let fallback = group
                .fallback
                .as_deref()
                .filter(|name| !name.is_empty())
                .ok_or(Unresolved::NoFallback)?;
            let number = if ordered {
                order
            } else {
                order.wrapping_add(1)
            }
            .max(1);
            format_number(fallback, number as u32)
        };
        String::from_utf8(name).map_err(|_| Unresolved::NotText)
    }

    fn declare(&mut self, key: &[u8], table: Table, definition: Group) {
        let group = match self.groups.entry(fold(key)) {
            Entry::Occupied(entry) => &mut entry.into_mut().1,
            Entry::Vacant(_) if table == Table::RailwayGun => return,
            Entry::Vacant(entry) => &mut entry.insert((table, Group::default())).1,
        };
        group.ordered.extend(definition.ordered);
        group.unordered.extend(definition.unordered);
        if definition.fallback.is_some() {
            group.fallback = definition.fallback;
        }
        if definition.prefix.is_some() {
            group.prefix = definition.prefix;
        }
        if let Some(prefix) = group.prefix.as_deref().filter(|prefix| !prefix.is_empty()) {
            for name in group.ordered.values_mut().chain(group.unordered.iter_mut()) {
                name.splice(0..0, prefix.iter().copied());
            }
        }
    }

    fn apply(
        &mut self,
        bytes: &[u8],
        names: &KeyNames,
    ) -> (Vec<crate::Diagnostic>, Option<String>) {
        let read = read_file(bytes, names);
        let mut notes: Vec<_> = read
            .misreads
            .iter()
            .map(|note| (note.kind, note.to_string()))
            .collect();
        let complete = read
            .items
            .len()
            .saturating_sub(usize::from(read.unsettled.is_some()));
        for item in &read.items[..complete] {
            let key = group_key(item);
            let (table, definition) = group(
                &String::from_utf8_lossy(key),
                item.value.items().unwrap_or_default(),
                &mut notes,
            );
            self.declare(key, table, definition);
        }
        let Some(unsettled) = &read.unsettled else {
            return (notes, None);
        };
        let mut names: BTreeSet<Vec<u8>> = native::lex(bytes, native::Mode::SkipMark)
            .tokens
            .iter()
            .filter(|token| {
                token.line >= unsettled.line
                    && !(token.kind == parser::Kind::End
                        && matches!(token.text.as_slice(), b"" | b"?" | b"!" | b"#" | b";"))
            })
            .map(|token| fold(group_name(&token.text)))
            .collect();
        names.extend(read.items.last().map(|item| fold(group_key(item))));
        let shown: Vec<String> = names
            .iter()
            .map(|name| name.escape_ascii().to_string())
            .collect();
        self.refused.extend(names);
        let refusal = format!(
            "{unsettled}; the rest of the file is unknown, so the group being read and every group a token of that rest could name are refused ({}): {}",
            shown.len(),
            shown.join(", ")
        );
        (notes, Some(refusal))
    }
}

fn group_key(item: &Item) -> &[u8] {
    group_name(item.key.as_deref().unwrap_or_default())
}

fn group_name(key: &[u8]) -> &[u8] {
    crate::parser::c_string(key)
}

pub fn parse(bytes: &[u8], names: &KeyNames) -> io::Result<Database> {
    let mut database = Database::default();
    let (mut notes, refusal) = database.apply(bytes, names);
    notes.extend(refusal.map(|message| (crate::DiagnosticKind::Unsettled, message)));
    if !notes.is_empty() {
        return Err(invalid(
            notes
                .into_iter()
                .map(|(_, message)| message)
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    Ok(database)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    Group,
    Ordered,
    Unordered,
}

fn read_file(bytes: &[u8], names: &KeyNames) -> native::Read {
    native::read(
        bytes,
        native::Mode::SkipMark,
        native::Top::File(Node::Top),
        Some(names),
        &mut |node, field| match node {
            Node::Top => Shape::Object(Node::Group),
            Node::Group => match field.keyword().as_deref() {
                Some(b"name" | b"prefix" | b"fallback_name") => Shape::One,
                Some(
                    b"unique"
                    | b"division_types"
                    | b"link_numbering_with"
                    | b"for_countries"
                    | b"ship_types",
                ) => Shape::List,
                Some(b"ordered") => Shape::Object(Node::Ordered),
                Some(b"unordered") => Shape::Object(Node::Unordered),
                Some(b"can_use") => Shape::Trigger,
                _ => Shape::Skip,
            },
            Node::Ordered if decimal_i32(&field.key.text) < 1 => Shape::Break,
            Node::Ordered | Node::Unordered => Shape::List,
        },
    )
}

fn group(key: &str, fields: &[Item], notes: &mut Vec<crate::Diagnostic>) -> (Table, Group) {
    let place = format!("group {key}");
    let mut table = Table::default();
    let mut group = Group::default();
    for field in fields {
        match field.keyword().map(parser::Keyword::folded).as_deref() {
            Some(b"fallback_name") => {
                if let Some(text) = stored(field, &place, notes) {
                    group.fallback = Some(text);
                }
            }
            Some(b"prefix") => {
                if let Some(text) = stored(field, &place, notes) {
                    group.prefix = Some(text);
                }
            }
            Some(b"ordered") => {
                for entry in field.value.items().unwrap_or_default() {
                    let number = decimal_i32(entry.key.as_deref().unwrap_or_default());
                    let Some(list) = entry.value.items().filter(|_| number >= 1) else {
                        continue;
                    };
                    if let Some(name) = list_name(list, &place, notes) {
                        group.ordered.insert(number, name);
                    }
                }
            }
            Some(b"unordered") => {
                for entry in field.value.items().unwrap_or_default() {
                    let list = entry.value.items().unwrap_or_default();
                    if let Some(name) = list_name(list, &place, notes) {
                        group.unordered.push(name);
                    }
                }
            }
            Some(b"unique") => {
                for entry in field.value.items().unwrap_or_default() {
                    group.unordered.extend(stored(entry, &place, notes));
                }
            }
            Some(b"type") => {
                let value = field.value_keyword();
                match [
                    Table::Army,
                    Table::Ship,
                    Table::Codename,
                    Table::RailwayGun,
                ]
                .into_iter()
                .find(|table| value.as_deref() == Some(table.keyword()))
                {
                    Some(found) => table = found,
                    None => notes.push((crate::DiagnosticKind::Logged, format!(
                        "{place}: type {} is not army, ship, codename or railway_gun; the game logs the invalid value and keeps the type it had",
                        shown(field)
                    ))),
                }
            }
            Some(
                b"name"
                | b"can_use"
                | b"division_types"
                | b"link_numbering_with"
                | b"for_countries"
                | b"ship_types",
            ) => {}
            _ => {
                let name = String::from_utf8_lossy(field.key.as_deref().unwrap_or_default());
                let quoted = if field.key_quoted { "quoted " } else { "" };
                notes.push((crate::DiagnosticKind::Logged, format!(
                    "{place}: {quoted}field {name} is not a division-name field; the game rejects it and skips its block"
                )));
            }
        }
    }
    (table, group)
}

fn shown(field: &Item) -> String {
    match field.value.scalar() {
        Some(text) if field.kind == parser::Kind::End => {
            format!("the end of the file (text `{}`)", text.escape_ascii())
        }
        Some(text) if field.quoted() => format!("\"{}\"", text.escape_ascii()),
        Some(text) => format!("`{}`", text.escape_ascii()),
        None => "a block".to_owned(),
    }
}

fn stored(item: &Item, place: &str, notes: &mut Vec<crate::Diagnostic>) -> Option<Vec<u8>> {
    let text = item.value.read_string().map(<[u8]>::to_vec);
    if text.is_none() {
        let name = String::from_utf8_lossy(item.key.as_deref().unwrap_or(b"entry"));
        notes.push((
            crate::DiagnosticKind::Unsupported,
            format!(
                "{place}: {name} is a block where the reader consumes one value, so it is not read"
            ),
        ));
    }
    text
}

fn list_name(items: &[Item], place: &str, notes: &mut Vec<crate::Diagnostic>) -> Option<Vec<u8>> {
    items.first().and_then(|first| stored(first, place, notes))
}

fn format_number(pattern: &[u8], number: u32) -> Vec<u8> {
    if pattern.windows(2).any(|pair| pair == b"%s") {
        replace(pattern, b"%s", roman(number).as_bytes())
    } else {
        replace(pattern, b"%d", number.to_string().as_bytes())
    }
}

fn replace(text: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        if text[at..].starts_with(from) {
            output.extend_from_slice(to);
            at += from.len();
        } else {
            output.push(text[at]);
            at += 1;
        }
    }
    output
}

fn roman(mut number: u32) -> String {
    let mut output = String::new();
    for (value, digits) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        for _ in 0..number / value {
            output.push_str(digits);
        }
        number %= value;
    }
    output
}

pub fn read<N: crate::DiagnosticMessage>(
    layers: &ResolvedLayers,
) -> io::Result<(Database, Vec<N>)> {
    read_directories(layers, &DIRECTORIES)
}

pub const DIRECTORIES: [&str; 4] = [
    "common/units/names_divisions/",
    "common/units/names_ships/",
    "common/units/codenames_operatives/",
    "common/units/names_railway_guns/",
];

pub fn read_directories<N: crate::DiagnosticMessage>(
    layers: &ResolvedLayers,
    directories: &[&str],
) -> io::Result<(Database, Vec<N>)> {
    let mut parameters = Vec::new();
    for directory in directories {
        parameters.extend_from_slice(&(directory.len() as u64).to_le_bytes());
        parameters.extend_from_slice(directory.as_bytes());
    }
    let product = layers.recorded_product("division name groups", 1, &parameters, |layers| {
        read_name_directories(layers, directories)
    })?;
    Ok((product.0.clone(), product.1.iter().cloned().map(N::from_diagnostic).collect()))
}

fn read_name_directories(
    layers: &ResolvedLayers,
    directories: &[&str],
) -> io::Result<(Database, Vec<crate::Diagnostic>)> {
    let mut paths = Vec::new();
    for directory in directories {
        paths.extend(crate::script_files::paths(layers, directory, Some(".txt"))?);
    }
    let mut database = Database::default();
    let mut warnings = Vec::new();
    let names = layers.key_names()?;
    for path in paths {
        let (notes, refusal) = database.apply(&path.read(layers)?, &names);
        warnings.extend(
            notes
                .into_iter()
                .map(|(kind, note)| (kind, format!("Name groups in {path}: {note}."))),
        );
        warnings.extend(refusal.map(|refusal| {
            (
                crate::DiagnosticKind::Unsettled,
                format!("Name groups refused in {path}: {refusal}"),
            )
        }));
    }
    Ok((
        database,
        warnings,
    ))
}

#[cfg(test)]
mod tests;
