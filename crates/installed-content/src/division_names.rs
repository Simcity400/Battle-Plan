//! Historical division and ship names from the authored name groups.
//!
//! The simulation supplies the selected group, ordinal and ordered flag; this
//! module only renders that selection. It never chooses or reserves a name.
//!
//! Native evidence (a729): `CNameGroup::vf4` at `1409b9a20` appends unordered
//! entries and replaces matching ordered IDs without discarding other entries.
//! `1409b5640` selects an exact ordered ID or a zero-based unordered entry;
//! `1409b4ce0` uses the authored fallback. `1409b57c0` replaces all `%s` with
//! Roman numerals, otherwise all `%d` with decimal, through `1424b7f40`.
//! `1422b8dd0` uses the ordinary subtractive Roman table, repeating M above 3999.
//! Ship `unique` lists enter the same unordered array. `CNameGroup::vf8` at
//! `1409b86a0` prefixes both entry arrays, never the fallback. `CPersistent::
//! Read` at `1424a9730` invokes that prefix pass after every group
//! declaration, including overrides.
//!
//! The database loader `1409b4370` keeps four tables by the group's `type`
//! (+0x150): army +0x28, ship +0x38, code name +0x48 and railway gun +0x58. A
//! declaration re-reads a group of its name found in the first three, which
//! stays in its table whatever `type` the new declaration gives; otherwise it
//! creates a group and inserts it into the table of the `type` it read. The
//! railway-gun table is not searched, so each railway-gun declaration is a
//! new group. A division's group is an army group: templates look theirs up
//! in the army table (1409B71F0) or take it from the country's division
//! tracker (1409B6820), which holds army groups (1409B88F0, kind 0).
//!
//! One reader reads each file (1409B8510), and the file is read through the
//! reader replay ([`native`]): its `@name` table holds from a definition to
//! the end of that file, and text the readers take out of step with its
//! braces is read as they read it, each such place noted. A block read by
//! key that the port cannot settle leaves the rest of the file unknown: the
//! group being read and every group that rest could declare are refused.

use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    io,
};

use crate::{
    ResolvedLayers,
    fields::invalid,
    fixed5::decimal_i32,
    key_names::KeyNames,
    parser::{
        self, Item,
        native::{self, Shape},
    },
};

/// A group's names as stored bytes (ReadStringInto 1424ABB50 keeps the
/// text's bytes); a lookup that selects one that is not UTF-8 is refused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Group {
    ordered: BTreeMap<i32, Vec<u8>>,
    unordered: Vec<Vec<u8>>,
    fallback: Option<Vec<u8>>,
    prefix: Option<Vec<u8>>,
}

/// A name group's `type` (+0x150), which picks the table 1409B4370 inserts
/// a new group into. CNameGroup's constructor (1409B2AD0) sets army.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Table {
    #[default]
    Army,
    Ship,
    Codename,
    RailwayGun,
}

impl Table {
    /// The `type` keyword 1409B9A20 compares the value token with.
    fn keyword(self) -> &'static str {
        match self {
            Self::Army => "army",
            Self::Ship => "ship",
            Self::Codename => "codename",
            Self::RailwayGun => "railway_gun",
        }
    }
}

/// Authored name groups, independent of any recording client's language/UI.
/// Keyed by the ASCII-folded group name: the loader 1409B4370 finds a group by
/// the folded hash 140BBA1B0 and `_stricmp` (14253D9F8), so `GROUP` and a
/// later layer's `group` are one group, and a recorded key matches either.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    /// The groups of the three tables 1409B4370 searches (army, ship, code
    /// name), each with the table it was inserted into.
    groups: BTreeMap<Vec<u8>, (Table, Group)>,
    /// Folded names a refused file declares or could declare. The game read
    /// that file, so from then on those groups are unknown here.
    refused: BTreeSet<Vec<u8>>,
}

/// A group name as the loader compares it.
fn fold(key: &[u8]) -> Vec<u8> {
    key.to_ascii_lowercase()
}

/// Why [`Database::resolve`] cannot render the selected name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unresolved {
    /// The recorded key is empty and no group is declared with an empty
    /// name, so the member's group is null (1409B7290 returned 0) and it has
    /// no stored name: 1409B5640 builds the legacy name (1409B4CE0).
    NoGroup,
    /// The recorded key is empty and a group is declared (or refused) with
    /// an empty name: the key is that group's or a null group's.
    EmptyKey,
    /// No name file that was read declares the recorded group.
    GroupAbsent,
    /// The recorded group is first declared with another `type`, so it is
    /// not in the army table a division's group comes from.
    NotArmy(Table),
    /// A name file that declares or could declare the group was refused.
    GroupRefused,
    /// An ordered member at order 0 has not been given its number yet:
    /// 1409B5640 calls the simulation's allocator, which is not recorded.
    UnassignedOrder,
    /// Neither the selected entry nor an authored non-empty fallback exists;
    /// 1409B4CE0 then builds the legacy name from the template.
    NoFallback,
    /// The selected name, with its prefix, is not UTF-8.
    NotText,
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoGroup => f.write_str(
                "it has neither a name group (ResolveNameGroup 1409B7290 found none) nor a stored name, so the game builds a legacy name from the template (1409B4CE0), which is not reproduced",
            ),
            Self::EmptyKey => f.write_str(
                "its recorded name group key is empty, which a null name group and the group a name file declares with an empty name both give",
            ),
            Self::GroupAbsent => {
                f.write_str("no name file that was read declares its recorded name group")
            }
            Self::NotArmy(table) => write!(
                f,
                "its name group is first declared with type {}, so it is not in the army table a division's group comes from (1409B4370, 1409B71F0)",
                table.keyword()
            ),
            Self::GroupRefused => {
                f.write_str("a name file that declares or could declare its name group was refused")
            }
            Self::UnassignedOrder => f.write_str(
                "its ordered name is not yet assigned (order 0 asks the simulation's allocator 1409B3A00)",
            ),
            Self::NoFallback => f.write_str(
                "its group has neither the selected entry nor a non-empty fallback_name, so the game builds a legacy name from the template (1409B4CE0), which is not reproduced",
            ),
            Self::NotText => f.write_str("its selected name is not UTF-8"),
        }
    }
}

impl Database {
    /// Render the simulation's selected historical name or authored fallback
    /// from the army group of the recorded key; an empty key is what the
    /// capture records for a null group.
    ///
    /// Ordered IDs are positive and may have gaps; unordered IDs are zero-based.
    /// The error says why the name cannot be rendered. An authored empty name
    /// remains empty.
    pub fn resolve(
        &self,
        group_key: impl AsRef<[u8]>,
        order: i32,
        ordered: bool,
    ) -> Result<String, Unresolved> {
        self.resolve_in(group_key.as_ref(), order, ordered, Some(Table::Army))
    }

    /// [`Self::resolve`] for a member whose group the recording names
    /// whatever table holds it (a raid's division or source ship): the
    /// group of the key in the army, ship or code-name table.
    pub fn resolve_any(
        &self,
        group_key: impl AsRef<[u8]>,
        order: i32,
        ordered: bool,
    ) -> Result<String, Unresolved> {
        self.resolve_in(group_key.as_ref(), order, ordered, None)
    }

    /// The database of these files, read in order; for other modules'
    /// fixtures.
    #[cfg(test)]
    pub(crate) fn from_files(files: &[&[u8]]) -> Self {
        let mut database = Self::default();
        for file in files {
            database.apply(file, &KeyNames::default());
        }
        database
    }

    /// Each folded group name a refused file declares or could declare.
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
        // 1409B5640 calls the simulation's allocator for an unassigned ordered
        // member. Its zero does not mean the authored fallback's first number.
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
            // Native unordered entries are copied, including any percent tokens.
            name.clone()
        } else {
            let fallback = group
                .fallback
                .as_deref()
                .filter(|name| !name.is_empty())
                .ok_or(Unresolved::NoFallback)?;
            // The native signed addition happens before clamping. Its unformatted
            // branch would require both distinct tokens to occur at position zero,
            // so every possible string follows this formatting path.
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

    /// One declaration as 1409B4370 applies it: it re-reads the group of
    /// its name in the army, ship or code-name table, which stays there;
    /// otherwise the declaration is a new group in the table of its `type`,
    /// and a railway-gun group, never searched again, is not kept.
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

    /// Reads one name file onto the database: what the game only logs about
    /// it, and why part of it is refused. Every group the replay read is
    /// applied. When the port cannot follow the reading to the end of the
    /// file, the group being read and every name a token of the unknown
    /// rest could declare are refused: 1409B4370 takes each group's name
    /// from a key token, and keys are not variables (1424AC5E0).
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
        // The group being read when the reading is unsettled is refused
        // below; the others are complete.
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
        // The line is the unsettled block's `{`: every token after it is at
        // or below it.
        // 1409B8510 constructs the lexer in mode 1 (CTextLexer 1424A6370).
        let mut names: BTreeSet<Vec<u8>> = native::lex(bytes, native::Mode::SkipMark)
            .tokens
            .iter()
            // A read that returned 0 names nothing unless it ended a bare
            // token: the `?` or comment marker it may keep is no name.
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

/// A declaration's group name ([`group_name`] of its key).
fn group_key(item: &Item) -> &[u8] {
    group_name(item.key.as_deref().unwrap_or_default())
}

/// A key token's text as a group name: 1409B4370 takes it to its first NUL
/// (the `strlen` before 14011D460), quoted or not.
fn group_name(key: &[u8]) -> &[u8] {
    crate::parser::c_string(key)
}

/// Parse one complete content file, preserving repeated group/field
/// declarations, its triggers against `names`. Anything [`read`] would warn
/// about is an error here.
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

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    /// 1409B4370's file loop.
    Top,
    /// CNameGroup::vf4 (1409B9A20) under CPersistent::Read (1424A9730).
    Group,
    /// 1409B9A20's `ordered` loop.
    Ordered,
    /// 1409B9A20's `unordered` loop.
    Unordered,
}

/// One name file as its reader reads it (1409B8510 builds one CReader per
/// file, so its `@name` table holds to the end of that file), through
/// [`native`]: 1409B4370 reads keys until the end of the text only, each a
/// group through CPersistent::Read (vtable +0x18) whatever its value.
/// Keyword ids from register_authored_keys (140082CB0).
fn read_file(bytes: &[u8], names: &KeyNames) -> native::Read {
    // 1409B8510 constructs the lexer in mode 1 (CTextLexer 1424A6370).
    native::read(
        bytes,
        native::Mode::SkipMark,
        native::Top::File(Node::Top),
        Some(names),
        &mut |node, field| match node {
            Node::Top => Shape::Object(Node::Group),
            Node::Group => match field.keyword().as_deref() {
                // ReadStringInto (1424ABB50): `name` 0x1B, `prefix` 0x30F8,
                // `fallback_name` 0x38E0.
                Some(b"name" | b"prefix" | b"fallback_name") => Shape::One,
                // ReadStringList (1401426E0): `unique` 0x30FA, `division_types`
                // 0x38DD, `link_numbering_with` 0x38DE, `for_countries` 0x38E4,
                // `ship_types` 0x3C84.
                Some(
                    b"unique"
                    | b"division_types"
                    | b"link_numbering_with"
                    | b"for_countries"
                    | b"ship_types",
                ) => Shape::List,
                // `ordered` 0x3907 and `unordered` 0x3908: 1409B9A20's own loops,
                // which stop at `}` or the end as CPersistent::Read does.
                Some(b"ordered") => Shape::Object(Node::Ordered),
                Some(b"unordered") => Shape::Object(Node::Unordered),
                // `can_use` 0x37AE: a CAndTrigger (vtable +0x28, CTrigger::
                // LoadBody 1405431C0) on this reader.
                Some(b"can_use") => Shape::Trigger,
                // `type` 0xE1 compares the value token (army 0x289D, ship
                // 0x28A0, codename 0x3D17, railway_gun 0x4D14); any other, a `{`
                // included, is RecordError "unexpected name type" (1424ACF90),
                // which skips a block (1424AD270). Any other key, a quoted one
                // included, reaches the default (1409B9FF0): RecordError
                // "Unexpected token while reading division names database", the
                // same skip.
                _ => Shape::Skip,
            },
            // Each entry: 1424AC5E0, then ParseTokenInt32 (1424B0080 -> atol)
            // on the key's text, quoted or not. Below 1 the game records "The
            // entry index of ... is out of range" and leaves the loop, the
            // entry's list unread; otherwise ReadStringList (1401426E0) reads
            // to `}` whatever the value was.
            Node::Ordered if decimal_i32(&field.key.text) < 1 => Shape::Break,
            Node::Ordered | Node::Unordered => Shape::List,
        },
    )
}

/// One group as CNameGroup::vf4 (1409B9A20) read it, in source order, with
/// the table its last valid `type` picks.
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
                    // The entry that ended the loop added nothing.
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
            // `unique` reads into the same array as the `unordered` entries.
            Some(b"unique") => {
                for entry in field.value.items().unwrap_or_default() {
                    group.unordered.extend(stored(entry, &place, notes));
                }
            }
            // 1409B9A20 compares the value token's id: a bare token folds to
            // its keyword (1424A7A90); a quoted one (0xF), a number, the end
            // of the text (0x13, its text kept) and a block are "unexpected
            // name type" (1424ACF90).
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
                        "{place}: type {} is not army, ship, codename or railway_gun; the game records \"unexpected name type\" and keeps the type it had (1409B9A20, 1424ACF90)",
                        shown(field)
                    ))),
                }
            }
            // `name`, `can_use`, `division_types`, `link_numbering_with`,
            // `for_countries` and `ship_types` decide the selected group in
            // simulation, not how it renders.
            Some(
                b"name"
                | b"can_use"
                | b"division_types"
                | b"link_numbering_with"
                | b"for_countries"
                | b"ship_types",
            ) => {}
            // 1409B9FF0 rejects any other key, a quoted one included, and
            // skips its block; the group reads on.
            _ => {
                let name = String::from_utf8_lossy(field.key.as_deref().unwrap_or_default());
                let quoted = if field.key_quoted { "quoted " } else { "" };
                notes.push((crate::DiagnosticKind::Logged, format!(
                    "{place}: {quoted}field {name} is not a division-name field; the game rejects it and skips its block (1409B9FF0, 1424ACF90)"
                )));
            }
        }
    }
    (table, group)
}

/// A field's value as a note shows it.
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

/// A string as CReader::ReadStringInto (1424ABB50) stores it
/// ([`parser::Value::read_string`]); the replay has resolved a variable
/// (1424AB540). [`read_file`] gives every such field one token, so a block
/// here is noted, not read.
fn stored(item: &Item, place: &str, notes: &mut Vec<crate::Diagnostic>) -> Option<Vec<u8>> {
    let text = item.value.read_string().map(<[u8]>::to_vec);
    if text.is_none() {
        let name = String::from_utf8_lossy(item.key.as_deref().unwrap_or(b"entry"));
        notes.push((crate::DiagnosticKind::Unsupported, format!(
            "{place}: {name} is a block where ReadStringInto (1424ABB50) reads one value, so it is not read"
        )));
    }
    text
}

/// A name entry's list as CReader::ReadStringList (1401426E0) read it: an
/// empty list adds no entry (1409B9A20 tests the count) and the name is its
/// first string. Descriptions/URLs affect the name picker, outside map scope.
fn list_name(items: &[Item], place: &str, notes: &mut Vec<crate::Diagnostic>) -> Option<Vec<u8>> {
    items.first().and_then(|first| stored(first, place, notes))
}

/// 1409B57C0 through 1424B7F40: every `%s` becomes the Roman numeral, or
/// else every `%d` the decimal number.
fn format_number(pattern: &[u8], number: u32) -> Vec<u8> {
    if pattern.windows(2).any(|pair| pair == b"%s") {
        replace(pattern, b"%s", roman(number).as_bytes())
    } else {
        replace(pattern, b"%d", number.to_string().as_bytes())
    }
}

/// Every non-overlapping `from`, left to right, replaced by `to`.
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

/// Reads the content stack's name files in the engine's directory order.
/// Read failures are fatal. What the game only logs about a file, and the
/// groups of a file the port cannot follow to its end, are warnings.
pub fn read<N: crate::DiagnosticMessage>(
    layers: &ResolvedLayers,
) -> io::Result<(Database, Vec<N>)> {
    // Later directories can re-read a group already retained in the army
    // table, even when the declaration changes its type (1409B4370).
    read_directories(layers, &DIRECTORIES)
}

/// The directories CBuildingDatabase::load (140191044..140191083) hands
/// 1409B8510, in order, into the one name database.
pub const DIRECTORIES: [&str; 4] = [
    "common/units/names_divisions/",
    "common/units/names_ships/",
    "common/units/codenames_operatives/",
    "common/units/names_railway_guns/",
];

/// [`read`] over several directories, each in the engine's order, into one
/// database.
pub fn read_directories<N: crate::DiagnosticMessage>(
    layers: &ResolvedLayers,
    directories: &[&str],
) -> io::Result<(Database, Vec<N>)> {
    // 1409B8510: enumerate_directory_wrapper with `.txt` and depth 0.
    let paths = directories
        .iter()
        .flat_map(|directory| crate::script_files::paths(layers, directory, Some(".txt")));
    let mut database = Database::default();
    let mut warnings = Vec::new();
    let names = layers.key_names()?;
    for path in paths {
        let (notes, refusal) = database.apply(&layers.read(&path)?, &names);
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
        warnings.into_iter().map(N::from_diagnostic).collect(),
    ))
}

#[cfg(test)]
mod tests;
