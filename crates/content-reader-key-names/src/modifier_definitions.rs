
use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{crt::{Scanned, scan_i32}, parser::{Item, NotText}};
use content_reader_script::native::{self, Shape};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModifierDefinition {
    pub token: String,
    pub flags: u32,
    pub precision: i32,
    pub postfix: u32,
}

impl ModifierDefinition {
    fn new(token: String) -> Self {
        Self {
            token,
            flags: 0,
            precision: 2,
            postfix: 0,
        }
    }

    pub fn postfix_key(&self) -> Option<&'static str> {
        match self.postfix {
            1 => Some("MOD_DAYS"),
            2 => Some("MOD_HOUR"),
            4 => Some("MOD_DAILY"),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModifierDefinitions {
    by_name: BTreeMap<Vec<u8>, ModifierDefinition>,
    registered: BTreeSet<Vec<u8>>,
    gap: Option<String>,
}

impl ModifierDefinitions {
    pub fn iter(&self) -> impl Iterator<Item = &ModifierDefinition> {
        self.by_name.values()
    }

    pub(crate) fn registered_names(&self) -> impl Iterator<Item = &[u8]> {
        self.registered.iter().map(Vec::as_slice)
    }

    pub(crate) fn gap(&self) -> Option<&str> {
        self.gap.as_deref()
    }

    pub fn get(&self, key: &str) -> Option<&ModifierDefinition> {
        self.by_name.get(&key.as_bytes().to_ascii_lowercase())
    }

    fn register(&mut self, definition: ModifierDefinition) -> Registered {
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

fn read_file(input: &[u8]) -> native::Read {
    native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        None,
        &mut |node, field| match node {
            Node::Top => Shape::Object(Node::Definition),
            Node::Definition if field.keyword_is("category") => Shape::Skip,
            Node::Definition => Shape::One,
        },
    )
}

fn definitions(items: &[Item]) -> Vec<Result<ModifierDefinition, NotText>> {
    items
        .iter()
        .map(|item| {
            let mut definition = ModifierDefinition::new(item.key_str()?.to_owned());
            for field in item.value.items().unwrap_or_default() {
                let keyword = field.keyword().map(|keyword| keyword.folded());
                let value = field.value_keyword();
                match (keyword.as_deref(), value.as_deref()) {
                    (Some(b"color_type"), Some("good")) => definition.flags |= 0x2,
                    (Some(b"color_type"), Some("neutral")) => definition.flags |= 0x4,
                    (Some(b"value_type"), Some("percentage")) => definition.flags |= 0x1,
                    (Some(b"value_type"), Some("percentage_in_hundred")) => {
                        definition.flags |= 0x80;
                    }
                    (Some(b"value_type"), Some("yes_no")) => definition.flags |= 0x20,
                    (Some(b"postfix"), Some("days")) => definition.postfix = 1,
                    (Some(b"postfix"), Some("hours")) => definition.postfix = 2,
                    (Some(b"postfix"), Some("daily")) => definition.postfix = 4,
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

pub fn read(layers: &ResolvedLayers) -> io::Result<(ModifierDefinitions, Vec<content_reader_parser::Diagnostic>)> {
    let mut warnings = Vec::new();
    let mut scans = content_reader_script::modifiers::ScanFailures::default();
    let mut result = ModifierDefinitions::default();
    for path in script_files::paths(layers, "common/modifier_definitions/", Some(".txt"))? {
        let read = read_file(&path.read(layers)?);
        let complete = if read.unsettled.is_some() {
            read.items.len().saturating_sub(1)
        } else {
            read.items.len()
        };
        for entry in &read.items[..complete] {
            if let content_reader_parser::tokens::Token::Name(name) =
                content_reader_parser::tokens::Token::c_str(entry.key.as_deref().unwrap_or_default())
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
                        Registered::Rejected => warnings.push((content_reader_parser::DiagnosticKind::Refused, format!(
                            "Modifier definition {token} in {path} not registered: a name starting with a digit or `-` is refused."
                        ))),
                    }
                }
                Err(refused) => warnings.push((
                    content_reader_parser::DiagnosticKind::Refused,
                    format!("Modifier definition in {path} not read: its name is {refused}."),
                )),
            }
        }
        if !repeated.is_empty() {
            warnings.push((content_reader_parser::DiagnosticKind::Warning, format!(
                "Modifier definitions in {path} already registered keep their first definition: {}.",
                repeated.join(", ")
            )));
        }
    }
    warnings.extend(
        scans
            .warning()
            .map(|message| (content_reader_parser::DiagnosticKind::Misread, message)),
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
        let negative = parse(b"fine = { precision = -1 } plain = { }");
        assert_eq!(negative[0].precision, -1);
        assert_eq!(negative[1].precision, 2);
    }

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
        assert_eq!(
            (found[1].flags, found[1].postfix_key(), found[1].precision),
            (0x24, Some("MOD_DAILY"), 4)
        );
    }

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

    #[test]
    fn the_replay_classes_hold_for_modifier_definitions() {
        let found = parse(
            b"@p = 010 a = { \"precision\" = 3 PRECISION = @p Value_Type = \"percentage\" COLOR_TYPE = good } \
              b = { postfix = { precision = 1 } category = army } c = { postfix = days",
        );
        assert_eq!((found[0].precision, found[0].flags), (8, 0x2));
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
