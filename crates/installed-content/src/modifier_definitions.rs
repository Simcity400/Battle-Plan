//! Scripted modifier definitions (`common/modifier_definitions`): how a
//! content-defined modifier is shown (good or bad colour, number or percentage,
//! precision, postfix). Built-in modifiers are defined in the executable, not in
//! these files; their display comes from the recording, never from a list here.
//!
//! LoadScriptModifierDefinitions (1406025E0) gives each file its own CReader
//! and reads it through the reader replay ([`native`]): keys until the end of
//! the text, each a definition read by CPersistent::Read with
//! SModifierDefinitonReader::vf4 (140604040) whatever its value, into a
//! reader built fresh for the key (14060290B), whose stored values
//! FindOrRegisterDynamicModifierDefinition (1405FF750) registers.

use crate::{
    ResolvedLayers,
    crt::{Scanned, scan_i32},
    parser::{
        Item, NotText,
        native::{self, Shape},
    },
    script_files,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// A definition as 1405FF750 registers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModifierDefinition {
    /// The name as first registered.
    pub token: String,
    /// The reader's flags word (+0xC), each value ORed in by 140604040:
    /// `value_type` percentage 0x1, percentage_in_hundred 0x80, yes_no 0x20
    /// (number adds nothing); `color_type` good 0x2, neutral 0x4 (bad adds
    /// nothing).
    pub flags: u32,
    /// `precision` (+8), ReadScriptSignedInt32 (1424AB970); 2 as the reader
    /// is built. Signed: a negative precision writes a value's decimals with
    /// trailing zeros dropped.
    pub precision: i32,
    /// `postfix` (+0x10): days 1, hours 2, daily 4, each replacing the last;
    /// `none` stores nothing; 0 as the reader is built.
    pub postfix: u32,
}

impl ModifierDefinition {
    /// The reader 1406025E0 builds for each key (14060290B).
    fn new(token: String) -> Self {
        Self {
            token,
            flags: 0,
            precision: 2,
            postfix: 0,
        }
    }

    /// The postfix's localisation key, as 1405FF750 maps the stored value.
    pub fn postfix_key(&self) -> Option<&'static str> {
        match self.postfix {
            1 => Some("MOD_DAYS"),
            2 => Some("MOD_HOUR"),
            4 => Some("MOD_DAILY"),
            _ => None,
        }
    }
}

/// Every scripted definition, by the name the lexer's token registry keys
/// it with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModifierDefinitions {
    /// By name folded: add_dynamic_token (1424A65A0 -> 1424A5EE0) folds A-Z
    /// and compares without case (StringView_CompareNoCase 1424B6750).
    by_name: BTreeMap<Vec<u8>, ModifierDefinition>,
    /// Native token names, including byte names the display projection
    /// cannot hold as text (1406025E0 -> 1405FF750).
    registered: BTreeSet<Vec<u8>>,
    /// An unread suffix may register further modifier names.
    gap: Option<String>,
}

impl ModifierDefinitions {
    /// Every effective scripted definition.
    pub fn iter(&self) -> impl Iterator<Item = &ModifierDefinition> {
        self.by_name.values()
    }

    pub(crate) fn registered_names(&self) -> impl Iterator<Item = &[u8]> {
        self.registered.iter().map(Vec::as_slice)
    }

    pub(crate) fn gap(&self) -> Option<&str> {
        self.gap.as_deref()
    }

    /// The definition a modifier key names, whatever its case.
    pub fn get(&self, key: &str) -> Option<&ModifierDefinition> {
        self.by_name.get(&key.as_bytes().to_ascii_lowercase())
    }

    /// 1405FF750: a name already registered keeps its first definition,
    /// the later block read only into its own discarded reader.
    fn register(&mut self, definition: ModifierDefinition) -> Registered {
        // add_dynamic_token (1424A65A0) refuses a name starting with a digit
        // or `-`, logged, and 1405FF750 registers nothing.
        if matches!(
            definition.token.as_bytes().first(),
            Some(b'0'..=b'9' | b'-')
        ) {
            return Registered::Rejected;
        }
        let name = definition.token.as_bytes().to_ascii_lowercase();
        if self.by_name.contains_key(&name) {
            return Registered::Taken;
        }
        self.by_name.insert(name, definition);
        Registered::New
    }
}

/// What [`ModifierDefinitions::register`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Registered {
    New,
    Taken,
    Rejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    Definition,
}

/// One file as its reader reads it.
fn read_file(input: &[u8]) -> native::Read {
    // LoadScriptModifierDefinitions (1406025E0) constructs the lexer in mode 0
    // (CTextLexer::constructor 1424A63A0).
    native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        None,
        &mut |node, field| match node {
            Node::Top => Shape::Object(Node::Definition),
            // SModifierDefinitonReader::vf4 (140604040), keyword ids from
            // register_authored_keys (140082CB0): `color_type` 0x4A62,
            // `value_type` 0x4A63 and `postfix` 0x4A69 compare the value token's
            // id (report_field_error 1424ACEE0, which skips nothing, on any
            // other), `precision` 0x4A6C ReadScriptSignedInt32; `category` 0x2BE
            // compares the value token's id, any other ReportUnexpectedToken
            // (1424AD100), whose skip passes over a block; any other key returns
            // with only its value token read.
            Node::Definition if field.keyword_is("category") => Shape::Skip,
            Node::Definition => Shape::One,
        },
    )
}

/// One file's definitions in order, each as its reader stored it, or the
/// name the port cannot hold.
fn definitions(items: &[Item]) -> Vec<Result<ModifierDefinition, NotText>> {
    items
        .iter()
        .map(|item| {
            let mut definition = ModifierDefinition::new(item.key_str()?.to_owned());
            for field in item.value.items().unwrap_or_default() {
                let keyword = field.keyword().map(|keyword| keyword.folded());
                let value = field.value_keyword();
                match (keyword.as_deref(), value.as_deref()) {
                    // good 0x4A64, neutral 0x3670; bad 0x4A65 adds nothing.
                    (Some(b"color_type"), Some("good")) => definition.flags |= 0x2,
                    (Some(b"color_type"), Some("neutral")) => definition.flags |= 0x4,
                    // percentage 0x4A66, percentage_in_hundred 0x4A67,
                    // yes_no 0x4A68; number 0x4A6D adds nothing.
                    (Some(b"value_type"), Some("percentage")) => definition.flags |= 0x1,
                    (Some(b"value_type"), Some("percentage_in_hundred")) => {
                        definition.flags |= 0x80;
                    }
                    (Some(b"value_type"), Some("yes_no")) => definition.flags |= 0x20,
                    // days 0x296D, hours 0x3170, daily 0x4A6A; none 0x165
                    // stores nothing.
                    (Some(b"postfix"), Some("days")) => definition.postfix = 1,
                    (Some(b"postfix"), Some("hours")) => definition.postfix = 2,
                    (Some(b"postfix"), Some("daily")) => definition.postfix = 4,
                    // `%i` (1424B02C0): EOF stores nothing, no match is
                    // reported; both leave it as it was.
                    (Some(b"precision"), _) => {
                        if let Scanned::Stored(value) =
                            scan_i32(field.value.scalar().unwrap_or_default())
                        {
                            definition.precision = value;
                        }
                    }
                    _ => {}
                }
            }
            Ok(definition)
        })
        .collect()
}

/// Every scripted modifier definition, the first registration of a name
/// kept (1405FF750).
pub fn read(layers: &ResolvedLayers) -> io::Result<(ModifierDefinitions, Vec<crate::Diagnostic>)> {
    let mut warnings = Vec::new();
    let mut scans = crate::modifiers::ScanFailures::default();
    let mut result = ModifierDefinitions::default();
    for path in script_files::paths(layers, "common/modifier_definitions/", Some(".txt")) {
        let read = read_file(&layers.read(&path)?);
        // The native registrar takes the raw key's C string after its
        // body returns. Keep that identity even when the display fields
        // cannot represent its byte spelling as UTF-8.
        let complete = if read.unsettled.is_some() {
            read.items.len().saturating_sub(1)
        } else {
            read.items.len()
        };
        for entry in &read.items[..complete] {
            if let crate::tokens::Token::Name(name) =
                crate::tokens::Token::c_str(entry.key.as_deref().unwrap_or_default())
            {
                result.registered.insert(name);
            }
        }
        if let Some(gap) = &read.unsettled {
            result.gap.get_or_insert_with(|| format!("{path} {gap}"));
        }
        warnings.extend(read.misreads.iter().map(|misread| {
            (
                misread.kind,
                format!("Modifier definitions in {path}: {misread}."),
            )
        }));
        for entry in &read.items {
            for field in entry.value.items().unwrap_or_default() {
                if field.keyword_is("precision") {
                    scans.int32(entry.key_str().unwrap_or("<non-text key>"), field);
                }
            }
        }
        let mut repeated = Vec::new();
        for definition in definitions(&read.items) {
            match definition {
                Ok(definition) => {
                    let token = definition.token.clone();
                    match result.register(definition) {
                        Registered::New => {}
                        Registered::Taken => repeated.push(token),
                        Registered::Rejected => warnings.push((crate::DiagnosticKind::Refused, format!(
                            "Modifier definition {token} in {path} not registered: a name starting with a digit or `-` is refused (1424A65A0)."
                        ))),
                    }
                }
                // A name the port cannot hold is no key a modifier the port
                // reads can name ([`crate::modifiers`] refuses the same).
                Err(refused) => warnings.push((
                    crate::DiagnosticKind::Refused,
                    format!("Modifier definition in {path} not read: its name is {refused}."),
                )),
            }
        }
        if !repeated.is_empty() {
            warnings.push((crate::DiagnosticKind::Warning, format!(
                "Modifier definitions in {path} already registered keep their first definition (1405FF750): {}.",
                repeated.join(", ")
            )));
        }
    }
    warnings.extend(
        scans
            .warning()
            .map(|message| (crate::DiagnosticKind::Misread, message)),
    );
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> Vec<ModifierDefinition> {
        definitions(&read_file(input).items)
            .into_iter()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn definitions_keep_their_display_fields() {
        let definitions = parse(b"operation_outcome = { color_type = good value_type = percentage precision = 0 category = intelligence_agency }");
        assert_eq!(definitions[0].token, "operation_outcome");
        assert_eq!(definitions[0].flags, 0x3);
        assert_eq!(definitions[0].precision, 0);
        assert_eq!(definitions[0].postfix_key(), None);
        // A negative precision is kept, not dropped; the reader starts at 2.
        let negative = parse(b"fine = { precision = -1 } plain = { }");
        assert_eq!(negative[0].precision, -1);
        assert_eq!(negative[1].precision, 2);
    }

    /// 140604040 ORs each colour and value spelling into one flags word, so
    /// a later `bad` or `number` keeps an earlier bit, and `postfix = none`
    /// stores nothing; any other key reads its value token only.
    #[test]
    fn spellings_accumulate_as_the_reader_stores_them() {
        let found = parse(
            b"a = { color_type = good color_type = bad value_type = percentage value_type = number \
              postfix = days postfix = none } b = { Color_Type = NEUTRAL value_type = yes_no \
              postfix = hours postfix = DAILY unknown = { precision = 4 } }",
        );
        assert_eq!(
            (found[0].flags, found[0].postfix_key()),
            (0x3, Some("MOD_DAYS"))
        );
        // `unknown` reads `{` only: the block's `precision` is b's own.
        assert_eq!(
            (found[1].flags, found[1].postfix_key(), found[1].precision),
            (0x24, Some("MOD_DAILY"), 4)
        );
    }

    /// 1405FF750 keeps a name's first registration, whatever the later
    /// spelling's case, and a lookup finds it without case.
    #[test]
    fn the_first_registration_of_a_name_is_kept() {
        let mut definitions = ModifierDefinitions::default();
        let registered: Vec<_> =
            parse(b"Speed = { postfix = days } speed = { postfix = hours } 1x = { }")
                .into_iter()
                .map(|definition| definitions.register(definition))
                .collect();
        assert_eq!(
            registered,
            [Registered::New, Registered::Taken, Registered::Rejected]
        );
        let speed = definitions.get("SPEED").unwrap();
        assert_eq!((speed.token.as_str(), speed.postfix), ("Speed", 1));
    }

    /// The replay's classes through this reader: a quoted key is no keyword
    /// and a quoted value no token (1424A7A90), a bare one folds; `%i`
    /// reads `010` as 8; a variable holds within its file; a block where one
    /// value is read is read as the definition's fields; a value that is the
    /// end of the text (0x13) is no keyword, whatever its text; a name that
    /// is not UTF-8 is refused by name.
    #[test]
    fn the_replay_classes_hold_for_modifier_definitions() {
        let found = parse(
            b"@p = 010 a = { \"precision\" = 3 PRECISION = @p Value_Type = \"percentage\" COLOR_TYPE = good } \
              b = { postfix = { precision = 1 } category = army } c = { postfix = days",
        );
        assert_eq!((found[0].precision, found[0].flags), (8, 0x2));
        // `postfix` read `{`; the block's precision is b's own and its `}`
        // ended b, so `category` is a definition of its own.
        assert_eq!((found[1].precision, found[1].postfix), (1, 0));
        assert!(
            found
                .iter()
                .any(|definition| definition.token == "category")
        );
        let last = found.last().unwrap();
        assert_eq!((last.token.as_str(), last.postfix), ("c", 0));
        let refused = definitions(&read_file(b"\"\xfe\" = { }").items);
        assert_eq!(refused, [Err(NotText::NotUtf8(vec![0xfe]))]);
    }
}
