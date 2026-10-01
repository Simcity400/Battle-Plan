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
            database.apply(file);
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
    fn apply(&mut self, bytes: &[u8]) -> (Vec<String>, Option<String>) {
        let read = read_file(bytes);
        let mut notes: Vec<String> = read.misreads.iter().map(ToString::to_string).collect();
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
/// declarations. Anything [`read`] would warn about is an error here.
pub fn parse(bytes: &[u8]) -> io::Result<Database> {
    let mut database = Database::default();
    let (mut notes, refusal) = database.apply(bytes);
    notes.extend(refusal);
    if !notes.is_empty() {
        return Err(invalid(notes.join("; ")));
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
fn read_file(bytes: &[u8]) -> native::Read {
    // 1409B8510 constructs the lexer in mode 1 (CTextLexer 1424A6370).
    native::read(
        bytes,
        native::Mode::SkipMark,
        Node::Top,
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
                // LoadBody 1405431C0) on this reader, its fields read by key
                // (the key-validity gap).
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
fn group(key: &str, fields: &[Item], notes: &mut Vec<String>) -> (Table, Group) {
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
                    None => notes.push(format!(
                        "{place}: type {} is not army, ship, codename or railway_gun; the game records \"unexpected name type\" and keeps the type it had (1409B9A20, 1424ACF90)",
                        shown(field)
                    )),
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
                notes.push(format!(
                    "{place}: {quoted}field {name} is not a division-name field; the game rejects it and skips its block (1409B9FF0, 1424ACF90)"
                ));
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
fn stored(item: &Item, place: &str, notes: &mut Vec<String>) -> Option<Vec<u8>> {
    let text = item.value.read_string().map(<[u8]>::to_vec);
    if text.is_none() {
        let name = String::from_utf8_lossy(item.key.as_deref().unwrap_or(b"entry"));
        notes.push(format!(
            "{place}: {name} is a block where ReadStringInto (1424ABB50) reads one value, so it is not read"
        ));
    }
    text
}

/// A name entry's list as CReader::ReadStringList (1401426E0) read it: an
/// empty list adds no entry (1409B9A20 tests the count) and the name is its
/// first string. Descriptions/URLs affect the name picker, outside map scope.
fn list_name(items: &[Item], place: &str, notes: &mut Vec<String>) -> Option<Vec<u8>> {
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
pub fn read(layers: &ResolvedLayers) -> io::Result<(Database, Vec<String>)> {
    read_directories(layers, &DIRECTORIES[..1])
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
pub fn read_directories(
    layers: &ResolvedLayers,
    directories: &[&str],
) -> io::Result<(Database, Vec<String>)> {
    // 1409B8510: enumerate_directory_wrapper with `.txt` and depth 0.
    let paths = directories
        .iter()
        .flat_map(|directory| crate::script_files::paths(layers, directory, Some(".txt")));
    let mut database = Database::default();
    let mut warnings = Vec::new();
    for path in paths {
        let (notes, refusal) = database.apply(&layers.read(&path)?);
        warnings.extend(
            notes
                .into_iter()
                .map(|note| format!("Name groups in {path}: {note}.")),
        );
        warnings.extend(refusal.map(|refusal| format!("Name groups refused in {path}: {refusal}")));
    }
    Ok((database, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Database {
        /// [`Database::apply`] as a result: its notes, or its refusal.
        fn applied(&mut self, bytes: &[u8]) -> io::Result<Vec<String>> {
            match self.apply(bytes) {
                (notes, None) => Ok(notes),
                (_, Some(refusal)) => Err(invalid(refusal)),
            }
        }
    }

    /// The group, ordered-list and string-list readers (1424A9730,
    /// 1409B9A20, 1401426E0) and the lexer's quoted string (1424A7A90) end
    /// at the end of the text, however deep. The game logs nothing for it:
    /// 1409B4370 reads keys until token 0x13 and each loop stops there.
    #[test]
    fn native_name_readers_close_every_block_open_at_eof() {
        let source = br#"FIRST = { ordered = { 1 = { "First %d" } } }
            LAST = { ordered = { 1 = { "Last %d" } }
            # No final group brace in the shipped Brazilian name file.
        "#;
        let names = parse(source).unwrap();
        assert_eq!(
            names.resolve("FIRST", 1, true).ok().as_deref(),
            Some("First 1")
        );
        assert_eq!(
            names.resolve("LAST", 1, true).ok().as_deref(),
            Some("Last 1")
        );
        for (source, expected) in [
            (br#"G = { ordered = { 1 = { "First" }"#.as_slice(), "First"),
            (br#"G = { ordered = { 1 = { "First""#, "First"),
            (br#"G = { unique = { "First""#, "First"),
        ] {
            let names = parse(source).unwrap();
            let (order, ordered) = if source.starts_with(b"G = { unique") {
                (0, false)
            } else {
                (1, true)
            };
            assert_eq!(
                names.resolve("G", order, ordered).ok().as_deref(),
                Some(expected)
            );
        }
        // An empty value before `}`: the ordered loop reads nothing.
        let mut names = Database::default();
        names.applied(b"SECOND = { ordered = }").unwrap();
        assert_eq!(
            names.resolve("SECOND", 1, true),
            Err(Unresolved::NoFallback)
        );
    }

    #[test]
    fn selection_uses_exact_ids_and_literal_unordered_names() {
        let names = parse(
            br#"
            INF = {
                ordered = { 1 = { "%d. Infantry" "DESCRIPTION" } 12 = { "%s Army %s / %d" } }
                unordered = { a = { "Guard %d" } b = { "Reserve %s" "DESC" "URL" } }
                fallback_name = "%d. Reserve"
            }
        "#,
        )
        .unwrap();
        assert_eq!(
            names.resolve("INF", 1, true).ok().as_deref(),
            Some("1. Infantry")
        );
        assert_eq!(
            names.resolve("INF", 12, true).ok().as_deref(),
            Some("XII Army XII / %d")
        );
        assert_eq!(
            names.resolve("INF", 2, true).ok().as_deref(),
            Some("2. Reserve")
        );
        assert_eq!(
            names.resolve("INF", 0, false).ok().as_deref(),
            Some("Guard %d")
        );
        assert_eq!(
            names.resolve("INF", 1, false).ok().as_deref(),
            Some("Reserve %s")
        );
        assert_eq!(
            names.resolve("INF", 2, false).ok().as_deref(),
            Some("3. Reserve")
        );
    }

    #[test]
    fn repeated_groups_merge_entries_and_keep_omitted_fallback() {
        let mut names = parse(
            br#"
            INF = { ordered = { 1 = { "Old %d" } 2 = { "Second %d" } }
                    unordered = { a = { "First" } } fallback_name = "Fallback %d" }
            INF = { ordered = { 1 = { "Updated %d" } } unordered = { a = { "Next" } } }
        "#,
        )
        .unwrap();
        names
            .applied(
                br#"INF = { ordered = { 1 = { "Mod %d" } 3 = { "Third %d" } }
                                      unordered = { a = { "Mod" } } }"#,
            )
            .unwrap();
        assert_eq!(names.resolve("INF", 1, true).ok().as_deref(), Some("Mod 1"));
        assert_eq!(
            names.resolve("INF", 2, true).ok().as_deref(),
            Some("Second 2")
        );
        assert_eq!(
            names.resolve("INF", 3, true).ok().as_deref(),
            Some("Third 3")
        );
        assert_eq!(
            names.resolve("INF", 4, true).ok().as_deref(),
            Some("Fallback 4")
        );
        for (order, expected) in ["First", "Next", "Mod"].into_iter().enumerate() {
            assert_eq!(
                names.resolve("INF", order as i32, false).ok().as_deref(),
                Some(expected)
            );
        }
        names.applied(br#"INF = { fallback_name = "" }"#).unwrap();
        assert_eq!(names.resolve("INF", 4, true), Err(Unresolved::NoFallback));
    }

    #[test]
    fn missing_operands_do_not_invent_names_but_authored_empty_names_survive() {
        let names = parse(
            br#"INF = { ordered = { 1 = { "" } 2 = {} }
                                       unordered = { a = {} b = { "" } } }"#,
        )
        .unwrap();
        assert_eq!(
            names.resolve("UNKNOWN", 1, true),
            Err(Unresolved::GroupAbsent)
        );
        assert_eq!(names.resolve("INF", 2, true), Err(Unresolved::NoFallback));
        assert_eq!(names.resolve("INF", 1, true).ok().as_deref(), Some(""));
        assert_eq!(names.resolve("INF", 0, false).ok().as_deref(), Some(""));
    }

    #[test]
    fn fallback_clamps_native_signed_ordinals_before_formatting() {
        let names = parse(br#"INF = { fallback_name = "%d / %d" }"#).unwrap();
        for (order, ordered) in [(-1, true), (-2, false), (i32::MAX, false)] {
            assert_eq!(
                names.resolve("INF", order, ordered).ok().as_deref(),
                Some("1 / 1")
            );
        }
        assert_eq!(
            names.resolve("INF", 0, true),
            Err(Unresolved::UnassignedOrder)
        );
        assert_eq!(roman(3999), "MMMCMXCIX");
        assert_eq!(roman(4004), "MMMMIV");
    }

    #[test]
    fn malformed_name_operands_are_reported() {
        for input in [
            br#"INF = { ordered = { 0 = { "bad" } } }"#.as_slice(),
            br#"INF = { ordered = { -1 = { "bad" } } }"#,
            br#"INF = { ordered = { 1 = "bad" } }"#,
            br#"INF = { unordered = { "bad" } }"#,
            br#"INF = { unordered = { 1 = "bad" } }"#,
            br#"INF = { unordered = { {} } }"#,
            br#"INF = { unknown_field = { fallback_name = "x" } }"#,
            br#"INF = { "anonymous" }"#,
            br#"INF = { fallback_name = {} }"#,
            br#"SHIP = { unique = { { "nested" } } }"#,
            br#"SHIP = { prefix = {} }"#,
        ] {
            assert!(parse(input).is_err());
        }
    }

    #[test]
    fn ship_unique_names_share_unordered_indices_and_prefix_only_authored_entries() {
        let names = parse(
            br#"SHIP = {
            prefix = "HMS "
            unique = { "Gallant" "Garland" }
            unordered = { x = { "Literal %s" } }
            unique = { "Gipsy" }
            ordered = { 7 = { "%s Flotilla" } }
            fallback_name = "Destroyer %s"
        }"#,
        )
        .unwrap();
        for (order, expected) in ["HMS Gallant", "HMS Garland", "HMS Literal %s", "HMS Gipsy"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                names.resolve("SHIP", order as i32, false).ok().as_deref(),
                Some(expected)
            );
        }
        assert_eq!(
            names.resolve("SHIP", 7, true).ok().as_deref(),
            Some("HMS VII Flotilla")
        );
        assert_eq!(
            names.resolve("SHIP", 4, false).ok().as_deref(),
            Some("Destroyer V")
        );
    }

    #[test]
    fn prefix_finalization_runs_on_each_declaration_and_preserves_absent_prefix() {
        let mut names = parse(
            br#"
            SHIP = { prefix = "HMS " unique = { "Gallant" } }
            SHIP = { unique = { "Garland" } }
        "#,
        )
        .unwrap();
        assert_eq!(
            names.resolve("SHIP", 0, false).ok().as_deref(),
            Some("HMS HMS Gallant")
        );
        assert_eq!(
            names.resolve("SHIP", 1, false).ok().as_deref(),
            Some("HMS Garland")
        );
        names
            .applied(br#"SHIP = { prefix = "USS " unique = { "Gipsy" } }"#)
            .unwrap();
        assert_eq!(
            names.resolve("SHIP", 0, false).ok().as_deref(),
            Some("USS HMS HMS Gallant")
        );
        assert_eq!(
            names.resolve("SHIP", 1, false).ok().as_deref(),
            Some("USS HMS Garland")
        );
        assert_eq!(
            names.resolve("SHIP", 2, false).ok().as_deref(),
            Some("USS Gipsy")
        );
        names
            .applied(br#"SHIP = { prefix = "" unique = { "Bare" } }"#)
            .unwrap();
        assert_eq!(
            names.resolve("SHIP", 2, false).ok().as_deref(),
            Some("USS Gipsy")
        );
        assert_eq!(
            names.resolve("SHIP", 3, false).ok().as_deref(),
            Some("Bare")
        );
    }

    /// 1409B4370 folds group names (140BBA1B0, `_stricmp` 14253D9F8): a later
    /// declaration spelled differently overrides the same group, and a
    /// recorded key in any case finds it.
    #[test]
    fn group_names_compare_without_case() {
        let mut database = parse(b"GER_INF = { ordered = { 1 = { \"First %d\" } } }").unwrap();
        database
            .applied(b"ger_inf = { ordered = { 1 = { \"Erste\" } } }")
            .unwrap();
        for key in ["GER_INF", "ger_inf", "Ger_Inf"] {
            assert_eq!(
                database.resolve(key, 1, true).ok().as_deref(),
                Some("Erste")
            );
        }
    }

    /// Field keys are folded as the lexer folds them, so a later layer's
    /// upper-case field overrides a lower-case group's.
    #[test]
    fn field_keys_compare_without_case() {
        let mut database = parse(b"G = { fallback_name = \"Old %d\" }").unwrap();
        database
            .applied(b"g = { FALLBACK_NAME = \"New %d\" ORDERED = { 2 = { \"Two\" } } }")
            .unwrap();
        assert_eq!(
            database.resolve("G", 1, true).ok().as_deref(),
            Some("New 1")
        );
        assert_eq!(database.resolve("G", 2, true).ok().as_deref(), Some("Two"));
    }

    /// Every field keyword folds (1424A7A90), whatever its case.
    #[test]
    fn every_field_keyword_compares_without_case() {
        let names = parse(
            br#"G = { UNORDERED = { u = { "U" } } Unique = { "Q" } PREFIX = "P "
                      Ordered = { 1 = { "O" } } Fallback_Name = "F %d" }"#,
        )
        .unwrap();
        for (order, ordered, expected) in [
            (0, false, "P U"),
            (1, false, "P Q"),
            (1, true, "P O"),
            (2, true, "F 2"),
        ] {
            assert_eq!(
                names.resolve("g", order, ordered).ok().as_deref(),
                Some(expected)
            );
        }
    }

    /// 1424A7A90 gives a quoted key token 0xF, so 1409B9A20 dispatches it to
    /// its default branch (1409B9FF0) like any name that is no keyword:
    /// RecordError (1424ACF90) skips its block.
    #[test]
    fn a_quoted_field_key_is_no_keyword() {
        let mut names = Database::default();
        let notes = names
            .applied(br#"G = { "fallback_name" = "Quoted %d" FALLBACK_name = "Bare %d" "PREFIX" = "X" }"#)
            .unwrap();
        assert_eq!(names.resolve("G", 3, true).ok().as_deref(), Some("Bare 3"));
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].starts_with(
            "group G: quoted field fallback_name is not a division-name field; the game rejects it and skips its block (1409B9FF0, 1424ACF90)"
        ));
        let notes = names
            .applied(br#"H = { "ordered" = { 1 = { "One" } } fallback_name = "H %d" }"#)
            .unwrap();
        assert!(
            notes[0].contains("quoted field ordered is not a division-name field"),
            "{notes:?}"
        );
        assert_eq!(names.resolve("H", 1, true).ok().as_deref(), Some("H 1"));
    }

    /// 1409B9A20 reads an ordered index with ParseTokenInt32 (1424B0080 ->
    /// 14254AF28 -> 14253C770): a signed decimal prefix of the key's text.
    /// Below 1 it leaves the ordered loop with the entry's list unread; the
    /// loop's closing read consumes the list's first token, and the group
    /// reads what follows as its own fields.
    #[test]
    fn ordered_indices_take_the_native_decimal_prefix() {
        let names = parse(
            br#"G = { ordered = { "1tail" = { "First" } 02x = { "Second" } 99999999999 = { "Max" } } }"#,
        )
        .unwrap();
        for (order, expected) in [(1, "First"), (2, "Second"), (i32::MAX, "Max")] {
            assert_eq!(
                names.resolve("G", order, true).ok().as_deref(),
                Some(expected)
            );
        }
        let mut names = Database::default();
        let notes = names
            .applied(
                br#"G = { ordered = { 2 = { "Two" } -3 = { "Negative" } 4 = { "Four" } } fallback_name = "F %d" }"#,
            )
            .unwrap();
        assert!(
            notes[0].contains("field `-3` ends the loop reading it"),
            "{notes:?}"
        );
        assert!(notes[1].ends_with("consumes \"Negative\""), "{notes:?}");
        assert_eq!(names.resolve("G", 2, true).ok().as_deref(), Some("Two"));
        // The entry's `}` ended the group: entry 4 and the fallback were
        // read as groups of their own.
        assert_eq!(names.resolve("G", 4, true), Err(Unresolved::NoFallback));
        assert!(names.groups.contains_key(b"4".as_slice()));
        assert!(parse(br#"G = { ordered = { tail = { "Zero" } } }"#).is_err());
    }

    /// The readers stop at the end of the file as at `}` (1424A9730,
    /// 1409B9A20, 1401426E0), also when it falls inside a comment.
    #[test]
    fn a_file_ending_inside_a_comment_keeps_its_entries() {
        for source in [
            br#"G = { ordered = { 1 = { "First" } # tail"#.as_slice(),
            br#"G = { ordered = { 1 = { "First"#,
        ] {
            let mut names = Database::default();
            let notes = names.applied(source).unwrap();
            assert!(notes.is_empty(), "{notes:?}");
            assert_eq!(names.resolve("G", 1, true).ok().as_deref(), Some("First"));
        }
    }

    /// One reader reads a file (1409B8510): a definition holds from where it
    /// is read to the end of that file (1424AC830), a bare reference takes
    /// its value (1424AB540), and a quoted one, a name not yet defined, or
    /// another spelling of a name stays its own text.
    #[test]
    fn variables_hold_to_the_end_of_their_file_and_quoted_text_stays_literal() {
        let mut names = Database::default();
        let notes = names
            .applied(
                br#"@top = "Top %d"
                EARLY = { fallback_name = @late }
                @late = "Late %d"
                G = {
                    @n = "Division %d"
                    fallback_name = @n
                    ordered = { @o = "Ordered %d" 1 = { @o "DESC" } 2 = { "@o" } }
                    unordered = { @u = "Loose" x = { @u } }
                    unique = { @top @missing "@n" }
                }
                H = { fallback_name = @N ordered = { 1 = { @o } } }
                LATE = { fallback_name = @late }
                TOP = { FALLBACK_NAME = @top }"#,
            )
            .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        for (group, order, ordered, expected) in [
            ("EARLY", 1, true, "@late"),
            ("G", 5, true, "Division 5"),
            ("G", 1, true, "Ordered 1"),
            ("G", 2, true, "@o"),
            ("G", 0, false, "Loose"),
            ("G", 1, false, "Top %d"),
            ("G", 2, false, "@missing"),
            ("G", 3, false, "@n"),
            ("H", 3, true, "@N"),
            ("H", 1, true, "Ordered 1"),
            ("LATE", 2, true, "Late 2"),
            ("TOP", 4, true, "Top 4"),
        ] {
            assert_eq!(
                names.resolve(group, order, ordered).ok().as_deref(),
                Some(expected),
                "{group} {order}"
            );
        }
        // The next file's reader starts with no variables.
        names.applied(br#"NEXT = { fallback_name = @n }"#).unwrap();
        assert_eq!(names.resolve("NEXT", 1, true).ok().as_deref(), Some("@n"));
    }

    /// The fields that only choose a group still share the file's reader:
    /// `can_use` is a trigger read by key (1405431C0), so a definition there
    /// registers in document order; a string list (1401426E0) and a skipped
    /// `type` block (1424ACF90 -> 1424AD270) define nothing.
    #[test]
    fn unread_group_fields_define_as_their_readers_do() {
        let mut names = Database::default();
        names
            .applied(
                br#"G = { fallback_name = @n can_use = { OR = { @n = "Guard %d" always = yes } } }
                H = { fallback_name = @n division_types = { @l = "x" } type = { @t = "y" } }
                I = { fallback_name = @l } J = { fallback_name = @t }"#,
            )
            .unwrap();
        for (group, expected) in [
            // Read before its definition, the reference keeps its text.
            ("G", "@n"),
            ("H", "Guard 1"),
            ("I", "@l"),
            ("J", "@t"),
        ] {
            assert_eq!(
                names.resolve(group, 1, true).ok().as_deref(),
                Some(expected),
                "{group}"
            );
        }
    }

    /// `can_use` is a CAndTrigger body (1405431C0), which hands LoadKey a
    /// key read without an operator with the stale value slot: `OR {`
    /// under `can_use = {` reads its block by key; `always yes` hands
    /// `always` the stale `{`, which its reader decides, so the rest of the
    /// file is unknown.
    #[test]
    fn a_can_use_key_without_an_operator_follows_the_stale_value() {
        let mut names = Database::default();
        let (_, refusal) =
            names.apply(br#"G = { can_use = { OR { always = yes } } unique = { "A" } }"#);
        assert!(refusal.is_none(), "{refusal:?}");
        assert_eq!(names.resolve("G", 0, false).ok().as_deref(), Some("A"));
        let (_, refusal) = names.apply(br#"H = { can_use = { always yes } unique = { "B" } }"#);
        let refusal = refusal.unwrap();
        assert!(
            refusal.contains(
                "key `always` has no operator and its reader is handed the previous value `{`"
            ),
            "{refusal}"
        );
        assert_eq!(names.resolve("H", 0, false), Err(Unresolved::GroupRefused));
    }

    /// A repeated definition reads nothing more (1424AC830): the reader
    /// takes its `=` as the next key, which consumes the value as its
    /// operator, and 1409B9A20 rejects that key; the first value holds. A
    /// definition naming a variable takes its value without consuming it,
    /// so that token is read next as a repeated definition.
    #[test]
    fn repeated_and_chained_definitions_follow_the_reader() {
        let mut names = Database::default();
        let notes = names
            .applied(
                br#"@b = "B %d"
                G = { @n = "First %d" @n = "Second %d" fallback_name = @n }
                H = { @a = @b fallback_name = @a }
                Q = { @q = "@b" fallback_name = @q }"#,
            )
            .unwrap();
        assert!(
            notes[0].starts_with("line 2: variable `@n` is already taken"),
            "{notes:?}"
        );
        assert!(
            notes[1].starts_with("line 2: key `=` has no operator"),
            "{notes:?}"
        );
        assert!(
            notes[2].starts_with("line 3: definition `@a` takes `@b`"),
            "{notes:?}"
        );
        assert!(
            notes
                .last()
                .unwrap()
                .starts_with("group G: field = is not a division-name field"),
            "{notes:?}"
        );
        assert!(parse(br#"G = { @n = "A" @n = "B" fallback_name = @n }"#).is_err());
        for (group, expected) in [("G", "First 2"), ("H", "B 2"), ("Q", "@b")] {
            assert_eq!(
                names.resolve(group, 2, true).ok().as_deref(),
                Some(expected),
                "{group}"
            );
        }
    }

    /// 1409B9A20 reads each `unordered` entry through 1424AC5E0 and then
    /// 1401426E0: a keyed entry `x = { "A" "B" }` is named "A", while the
    /// anonymous `{ "A" "B" }` spends "A" as the key's operator (reported by
    /// 1424AB2A0) and is named "B"; `{ "A" }` then adds no entry.
    #[test]
    fn unordered_entries_are_read_as_the_game_reads_them() {
        let source =
            br#"G = { unordered = { x = { "A" "B" } { "A" "B" "URL" } { "Alone" } y = { "C" } } }"#;
        let mut names = Database::default();
        let notes = names.applied(source).unwrap();
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(
            notes[0].starts_with("line 1: key `{` has no operator: 1424AC5E0 consumes \"A\""),
            "{notes:?}"
        );
        assert!(notes[2].contains("consumes \"Alone\""), "{notes:?}");
        for (order, expected) in [(0, "A"), (1, "B"), (2, "C")] {
            assert_eq!(
                names.resolve("G", order, false).ok().as_deref(),
                Some(expected),
                "{order}"
            );
        }
        assert_eq!(names.resolve("G", 3, false), Err(Unresolved::NoFallback));
        // The strict reader treats the reported token as an error.
        assert!(parse(source).is_err());
    }

    /// 1409B9A20's default (1409B9FF0) rejects a field it does not read
    /// through RecordError (1424ACF90), which skips a block (1424AD270): the
    /// group reads on either way.
    #[test]
    fn unknown_group_fields_are_rejected_as_the_game_rejects_them() {
        let mut names = Database::default();
        let notes = names
            .applied(br#"G = { colour = "red" fallback_name = "F %d" }"#)
            .unwrap();
        assert_eq!(
            notes,
            [
                "group G: field colour is not a division-name field; the game rejects it and skips its block (1409B9FF0, 1424ACF90)"
            ]
        );
        assert_eq!(names.resolve("G", 2, true).ok().as_deref(), Some("F 2"));
        let notes = names
            .applied(br#"H = { extra = { fallback_name = "X %d" } fallback_name = "F %d" }"#)
            .unwrap();
        assert!(notes[0].starts_with("group H: field extra is not a division-name field"));
        assert_eq!(names.resolve("H", 1, true).ok().as_deref(), Some("F 1"));
        // Fields 1409B9A20 does read are no refusal, in any case.
        assert!(
            parse(br#"G = { NAME = "x" type = army can_use = { always = yes } division_types = { "a" }
                      link_numbering_with = { "b" } for_countries = { GER } ship_types = { "c" } }"#)
            .is_ok()
        );
    }

    /// CReader::ReadStringInto (1424ABB50) stores a string holding a line
    /// feed as "Unreadable String".
    #[test]
    fn a_name_holding_a_line_feed_is_unreadable() {
        let names = parse(b"G = { ordered = { 1 = { \"Line\nBreak\" } } }").unwrap();
        assert_eq!(
            names.resolve("G", 1, true).ok().as_deref(),
            Some("Unreadable String")
        );
    }

    /// A block read by key that the port cannot settle (a trigger holding a
    /// value without a key and a definition) leaves the rest of the file
    /// unknown. Every group the replay read is applied; the group being read
    /// and every name a token from the unsettled block on could declare are
    /// refused, earlier layers' groups of those names included, and stay
    /// refused through later files. Other groups keep their values.
    #[test]
    fn an_unsettled_file_refuses_every_group_its_rest_could_declare() {
        let mut names = parse(
            br#"SHIP = { prefix = "HMS " unique = { "Gallant" } }
                OTHER = { unique = { "Kept" } }
                KEPT = { unique = { "Earlier" } }"#,
        )
        .unwrap();
        let (notes, refusal) = names.apply(
            br#"SHIP = { unique = { "Garland" } } FIRST = { unique = { "First" } }
                ship = {
                    can_use = { OR = { always @v = yes } } unique = { "Lost" } }
                OTHER = { unique = { "Unknown" } }"#,
        );
        assert!(notes.is_empty(), "{notes:?}");
        let refusal = refusal.unwrap();
        assert!(
            refusal.starts_with("line 3: variable @v is in a block read by a trigger"),
            "{refusal}"
        );
        assert!(
            refusal.ends_with(
                "are refused (13): =, @v, always, can_use, lost, or, other, ship, unique, unknown, yes, {, }"
            ),
            "{refusal}"
        );
        // The first declaration applied (prefix pass included) before the
        // group that cannot be followed.
        assert_eq!(
            names.groups[b"ship".as_slice()].1.unordered,
            [b"HMS HMS Gallant".to_vec(), b"HMS Garland".to_vec()]
        );
        assert_eq!(
            names.resolve("Ship", 0, false),
            Err(Unresolved::GroupRefused)
        );
        // The rest names OTHER, so its earlier values may be stale.
        assert_eq!(
            names.resolve("OTHER", 0, false),
            Err(Unresolved::GroupRefused)
        );
        for (group, expected) in [("FIRST", "First"), ("KEPT", "Earlier")] {
            assert_eq!(
                names.resolve(group, 0, false).ok().as_deref(),
                Some(expected)
            );
        }
        names
            .applied(br#"SHIP = { unique = { "Later" } } OTHER = { unique = { "Later" } }"#)
            .unwrap();
        for group in ["SHIP", "other"] {
            assert_eq!(
                names.resolve(group, 1, false),
                Err(Unresolved::GroupRefused)
            );
        }
    }

    /// Text the readers take out of step with its braces is read as they
    /// read it (the replay), each place noted: a definition that ends its
    /// block, a taken name, a definition without `=` or with a block value,
    /// `@(` (the lexer ends a token at `(`), a block where ReadStringInto
    /// takes one value, and a value where ReadStringList reads a list.
    #[test]
    fn out_of_step_forms_are_read_as_the_game_reads_them() {
        for (source, fallback, note) in [
            (
                br#"G = { fallback_name = "x" @n = "y" }"#.as_slice(),
                Some("x"),
                "key `}` has no operator",
            ),
            (
                br#"@n = "a" @n = "b" G = { fallback_name = @n }"#,
                None,
                "variable `@n` is already taken",
            ),
            (
                br#"G = { unordered = { @n { "x" } } }"#,
                None,
                "definition `@n` has no `=`",
            ),
            (
                br#"G = { @n = { "a" } fallback_name = @n }"#,
                Some("{"),
                "definition `@n` keeps only the `{` of its block value",
            ),
            (
                br#"G = { fallback_name = @(1+1) }"#,
                Some("@"),
                "key `(` has no operator",
            ),
            (
                br#"G = { NAME = { "a" } fallback_name = "x" }"#,
                Some("x"),
                "field `NAME` is a block where its reader takes one value",
            ),
            (
                br#"G = { for_countries = GER fallback_name = "x" }"#,
                None,
                "field `for_countries` is `GER` where its reader reads a list",
            ),
        ] {
            let mut names = Database::default();
            let notes = names.applied(source).unwrap();
            assert!(
                notes.iter().any(|found| found.contains(note)),
                "{}: {notes:?}",
                String::from_utf8_lossy(source)
            );
            let expected = match fallback {
                Some(text) => Ok(text.to_owned()),
                None => Err(Unresolved::NoFallback),
            };
            if names.groups.contains_key(b"g".as_slice()) {
                assert_eq!(names.resolve("G", 1, true), expected);
            } else {
                assert!(fallback.is_none());
            }
        }
    }

    /// The replay's classes through this reader: a quoted key is no keyword
    /// (1424A7A90), a bare one folds; a variable holds from its definition
    /// to the end of its file only (1409B8510); a block where ReadStringInto
    /// takes one value is read as the group's fields; the end of the text
    /// closes every loop.
    #[test]
    fn the_replay_classes_hold_for_name_groups() {
        let mut names = Database::default();
        let notes = names
            .applied(
                br#"@late = "Late %d"
                MIXED = { "unique" = { "Quoted" } UNIQUE = { "Bare" } Fallback_Name = @late }
                BLOCK = { prefix = { fallback_name = "Inner %d" } unique = { "After" } }"#,
            )
            .unwrap();
        assert_eq!(
            names.resolve("mixed", 0, false).ok().as_deref(),
            Some("Bare")
        );
        assert_eq!(
            names.resolve("MIXED", 4, true).ok().as_deref(),
            Some("Late 4")
        );
        assert!(
            notes
                .iter()
                .any(|note| note.contains("quoted field unique"))
        );
        // `prefix` reads `{`; the block's fallback is the group's own, and
        // its `}` ends the group, so `unique` is a group at the top level.
        assert_eq!(
            names.groups[b"block".as_slice()].1.prefix.as_deref(),
            Some(b"{".as_slice())
        );
        assert_eq!(
            names.resolve("BLOCK", 2, true).ok().as_deref(),
            Some("Inner 2")
        );
        assert!(names.groups.contains_key(b"unique".as_slice()));
        assert!(
            notes
                .iter()
                .any(|note| note.contains("field `prefix` is a block")),
            "{notes:?}"
        );
        names
            .applied(br#"NEXT = { fallback_name = @late } OPEN = { ordered = { 1 = { "Open"#)
            .unwrap();
        assert_eq!(
            names.resolve("NEXT", 1, true).ok().as_deref(),
            Some("@late")
        );
        assert_eq!(names.resolve("OPEN", 1, true).ok().as_deref(), Some("Open"));
    }

    /// 1409B4370 searches the army, ship and code-name tables, not the
    /// railway-gun one: a railway-gun group and a later army group of the
    /// same name are two groups, so the army ordinals do not shift. A
    /// redeclaration re-reads the group in its first table, whatever its
    /// new `type`; a group first declared as a ship or code name is not in
    /// the army table divisions take their group from (1409B71F0).
    #[test]
    fn group_identity_follows_the_typed_tables() {
        let mut names = Database::default();
        let notes = names
            .applied(
                br#"G = { type = railway_gun unique = { "Rail" } }
                G = { unique = { "Army" } }
                R = { TYPE = Railway_Gun unique = { "One" } } R = { type = railway_gun unique = { "Two" } }
                A = { unique = { "First" } } A = { type = ship unique = { "Second" } }
                S = { type = ship unique = { "Ship" } } S = { type = army unique = { "Army" } }
                C = { type = codename unique = { "Code" } }
                L = { type = ship type = army unique = { "Last" } }"#,
            )
            .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        for (group, order, expected) in [
            ("g", 0, Ok("Army".to_owned())),
            ("G", 1, Err(Unresolved::NoFallback)),
            ("R", 0, Err(Unresolved::GroupAbsent)),
            ("A", 1, Ok("Second".to_owned())),
            ("S", 0, Err(Unresolved::NotArmy(Table::Ship))),
            ("C", 0, Err(Unresolved::NotArmy(Table::Codename))),
            ("L", 0, Ok("Last".to_owned())),
        ] {
            assert_eq!(names.resolve(group, order, false), expected, "{group}");
        }
        // Across layers too: a later layer's army group is its own.
        names.applied(br#"R = { unique = { "Army R" } }"#).unwrap();
        assert_eq!(names.resolve("R", 0, false).as_deref(), Ok("Army R"));
        assert!(
            Unresolved::NotArmy(Table::Ship)
                .to_string()
                .contains("first declared with type ship")
        );
    }

    /// 1409B9A20 compares the value token's id: a quoted `type`, a number and
    /// a block are "unexpected name type" (1424ACF90), and the group keeps
    /// the type it had.
    #[test]
    fn an_unexpected_type_keeps_the_type_the_group_had() {
        let mut names = Database::default();
        let notes = names
            .applied(
                br#"Q = { type = "railway_gun" unique = { "Quoted" } }
                N = { type = 3 unique = { "Number" } }
                B = { type = { railway_gun } unique = { "Block" } }"#,
            )
            .unwrap();
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(
            notes[0].starts_with(
                "group Q: type \"railway_gun\" is not army, ship, codename or railway_gun; the game records \"unexpected name type\""
            ),
            "{notes:?}"
        );
        assert!(notes[1].starts_with("group N: type `3`"), "{notes:?}");
        assert!(notes[2].starts_with("group B: type a block"), "{notes:?}");
        for (group, expected) in [("Q", "Quoted"), ("N", "Number"), ("B", "Block")] {
            assert_eq!(names.resolve(group, 0, false).as_deref(), Ok(expected));
        }
        // A text ending in `type = ship`: the last bare token's advance
        // fails (1424A7A90), so the value slot holds the end token 0x13
        // with the text `ship`, which is no keyword; the group stays army.
        let mut names = Database::default();
        let notes = names
            .applied(b"E = { unique = { \"End\" } type = ship")
            .unwrap();
        assert!(
            notes[0].starts_with("group E: type the end of the file (text `ship`)"),
            "{notes:?}"
        );
        assert_eq!(names.resolve("E", 0, false).as_deref(), Ok("End"));
        let mut names = Database::default();
        names
            .applied(b"S = { unique = { \"Ship\" } TYPE = Ship\n")
            .unwrap();
        assert_eq!(
            names.resolve("S", 0, false),
            Err(Unresolved::NotArmy(Table::Ship))
        );
    }

    /// 1409B4370 takes a group's name to its first NUL, and ReadStringInto
    /// (1424ABB50, read_string_token 1424B05E0) a name to its first NUL.
    #[test]
    fn names_and_group_keys_end_at_their_first_nul() {
        let names = parse(b"\"G\0tail\" = { unique = { \"Ab\0cd\" } }").unwrap();
        assert_eq!(names.resolve("g", 0, false).as_deref(), Ok("Ab"));
    }

    /// Names are stored bytes: only a lookup that selects one that is not
    /// UTF-8 is refused, the group's other names still render. (A byte 0xFF
    /// ends the text for the lexer, 1424A7A90.)
    #[test]
    fn a_name_that_is_not_utf8_refuses_only_the_lookups_that_select_it() {
        let names = parse(
            b"G = { unique = { \"\xe9\" \"Fine\" } ordered = { 2 = { \"\xe8 %d\" } } fallback_name = \"F %d\" }\n\
              P = { prefix = \"\xe9\" unique = { \"Any\" } fallback_name = \"Plain %d\" }",
        )
        .unwrap();
        for (group, order, ordered, expected) in [
            ("G", 0, false, Err(Unresolved::NotText)),
            ("G", 1, false, Ok("Fine".to_owned())),
            ("G", 2, true, Err(Unresolved::NotText)),
            ("G", 3, true, Ok("F 3".to_owned())),
            ("P", 0, false, Err(Unresolved::NotText)),
            // The prefix never reaches the fallback (1409B86A0).
            ("P", 1, false, Ok("Plain 2".to_owned())),
        ] {
            assert_eq!(
                names.resolve(group, order, ordered),
                expected,
                "{group} {order}"
            );
        }
    }

    /// The capture records an empty key for a null group; a group declared
    /// with an empty name makes that key ambiguous.
    #[test]
    fn an_empty_key_is_a_null_group_unless_one_is_declared_empty() {
        let names = parse(br#"G = { unique = { "N" } }"#).unwrap();
        assert_eq!(names.resolve("", 0, false), Err(Unresolved::NoGroup));
        let names = parse(br#""" = { unique = { "Empty" } }"#).unwrap();
        assert_eq!(names.resolve(b"", 0, false), Err(Unresolved::EmptyKey));
    }
}
