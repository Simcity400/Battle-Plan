//! Scripted modifier definitions (`common/modifier_definitions`): how a
//! content-defined modifier is shown (good or bad colour, number or percentage,
//! precision, postfix). Built-in modifiers are defined in the executable, not in
//! these files; their display comes from the recording, never from a list here.
//!
//! Each file has its own reader replay ([`native`]): keys until the end
//! of text each create a fresh definition reader, whatever their value.
//! The values that reader stores are then registered under the key.

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

/// One registered scripted modifier definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModifierDefinition {
    /// The name as first registered.
    pub token: String,
    /// The flags word, each value ORed in:
    /// `value_type` percentage 0x1, percentage_in_hundred 0x80, yes_no 0x20
    /// (number adds nothing); `color_type` good 0x2, neutral 0x4 (bad adds
    /// nothing).
    pub flags: u32,
    /// `precision`, read as a signed integer; initially 2.
    /// A negative precision writes a value's decimals with trailing zeros
    /// dropped.
    pub precision: i32,
    /// `postfix`: days 1, hours 2, daily 4, each replacing the last;
    /// `none` stores nothing; initially 0.
    pub postfix: u32,
}

impl ModifierDefinition {
    /// A fresh definition reader for each key.
    fn new(token: String) -> Self {
        Self {
            token,
            flags: 0,
            precision: 2,
            postfix: 0,
        }
    }

    /// The postfix's localisation key for the stored value.
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
    /// By name folded: A-Z folds before hashing and case-insensitive
    /// comparison.
    by_name: BTreeMap<Vec<u8>, ModifierDefinition>,
    /// Registered token names, including byte names the display projection
    /// cannot hold as text.
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

    /// A name already registered keeps its first definition;
    /// the later block reads only into its own discarded reader.
    fn register(&mut self, definition: ModifierDefinition) -> Registered {
        // A name starting with a digit or `-` is logged and refused,
        // so nothing is registered.
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
    // Each file constructs its own mode-0 lexer.
    native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        None,
        &mut |node, field| match node {
            Node::Top => Shape::Object(Node::Definition),
            // `color_type`, `value_type` and `postfix` compare the value
            // keyword; any other value reports an error without skipping.
            // `precision` reads a signed integer. `category` compares the value
            // keyword and skips an unexpected block. Any other key returns
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
                    // `good` and `neutral` add colour flags; `bad` adds nothing.
                    (Some(b"color_type"), Some("good")) => definition.flags |= 0x2,
                    (Some(b"color_type"), Some("neutral")) => definition.flags |= 0x4,
                    // `percentage`, `percentage_in_hundred` and `yes_no` add value
                    // flags; `number` adds nothing.
                    (Some(b"value_type"), Some("percentage")) => definition.flags |= 0x1,
                    (Some(b"value_type"), Some("percentage_in_hundred")) => {
                        definition.flags |= 0x80;
                    }
                    (Some(b"value_type"), Some("yes_no")) => definition.flags |= 0x20,
                    // `days`, `hours` and `daily` replace the postfix; `none`
                    // stores nothing.
                    (Some(b"postfix"), Some("days")) => definition.postfix = 1,
                    (Some(b"postfix"), Some("hours")) => definition.postfix = 2,
                    (Some(b"postfix"), Some("daily")) => definition.postfix = 4,
                    // Integer scan: EOF stores nothing, no match is reported;
                    // both leave it as it was.
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

/// Every scripted modifier definition, keeping the first registration
/// of a name.
pub fn read(layers: &ResolvedLayers) -> io::Result<(ModifierDefinitions, Vec<crate::Diagnostic>)> {
    let mut warnings = Vec::new();
    let mut scans = crate::modifiers::ScanFailures::default();
    let mut result = ModifierDefinitions::default();
    for path in script_files::paths(layers, "common/modifier_definitions/", Some(".txt"))? {
        let read = read_file(&path.read(layers)?);
        // Registration takes the raw key's C string after its body returns.
        // Keep that identity even when the display fields cannot represent
        // its byte spelling as UTF-8.
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
                            "Modifier definition {token} in {path} not registered: a name starting with a digit or `-` is refused."
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
                "Modifier definitions in {path} already registered keep their first definition: {}.",
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

    /// Each colour and value spelling is ORed into one flags word, so
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

    /// A name keeps its first registration, whatever the later spelling's
    /// case, and a lookup finds it without case.
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
    /// and a quoted value no keyword, a bare one folds; integer scanning
    /// reads `010` as 8; a variable holds within its file; a block where one
    /// value is read becomes the definition's fields; the end of text is no
    /// keyword, whatever its text; a name that is not UTF-8 is refused by name.
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
