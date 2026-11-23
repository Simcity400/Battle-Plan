
use content_reader_layers::ResolvedLayers;
use content_reader_parser::{crt::{self, Scanned}, parser, tokens::Token};
use content_reader_script::native::{self, Shape};
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Opinion {
    pub token: String,
    pub name: String,
    pub opinion: i16,
    pub trade: bool,
}

#[derive(Default)]
pub struct Database {
    pub definitions: Vec<Opinion>,
    pub notes: Vec<String>,
    gap: Option<String>,
}

impl Database {
    pub fn find(&self, name: &str) -> Result<Option<&Opinion>, String> {
        let token = Token::c_str(name.as_bytes());
        if let Some(found) = self
            .definitions
            .iter()
            .find(|item| Token::c_str(item.token.as_bytes()) == token)
        {
            return Ok(Some(found));
        }
        match &self.gap {
            Some(reason) => Err(format!("opinion modifier {name}: {reason}")),
            None => Ok(None),
        }
    }
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Group,
    Opinion,
}

pub fn parse(input: &[u8], bounds: [i16; 2]) -> io::Result<Vec<Opinion>> {
    parse_with_notes(input, bounds).map(|(definitions, _)| definitions)
}

fn parse_with_notes(input: &[u8], bounds: [i16; 2]) -> io::Result<(Vec<Opinion>, Vec<String>)> {
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        None,
        &mut |node, field| match node {
            Node::File if field.keyword_is("opinion_modifiers") => Shape::Object(Node::Group),
            Node::File => Shape::Skip,
            Node::Group => Shape::Object(Node::Opinion),
            Node::Opinion => match field.keyword().as_deref() {
                Some(
                    b"value" | b"target" | b"trade" | b"days" | b"months" | b"years" | b"decay"
                    | b"min_trust" | b"max_trust",
                ) => Shape::One,
                _ => Shape::Skip,
            },
        },
    );
    if let Some(error) = read.unsettled {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            error.to_string(),
        ));
    }
    let notes = read.misreads.iter().map(ToString::to_string).collect();
    let mut definitions = Vec::new();
    for group in &read.items {
        if !group.keyword_is("opinion_modifiers") {
            continue;
        }
        for item in group.value.items().unwrap_or_default() {
            let invalid = |reason: String| io::Error::new(io::ErrorKind::InvalidData, reason);
            let token = parser::text(parser::c_string(item.key.as_deref().unwrap_or_default()))
                .map_err(|error| invalid(error.to_string()))?
                .to_owned();
            let mut definition = Opinion {
                name: token.clone(),
                token,
                ..Opinion::default()
            };
            let (mut min, mut max) = (bounds[0], bounds[1]);
            let mut target = false;
            for field in item.value.items().unwrap_or_default() {
                match field.keyword().map(|key| key.folded()).as_deref() {
                    Some(b"value" | b"min_trust" | b"max_trust") => {
                        let value = match crt::scan_d32(field.value.scalar().unwrap_or(b"{")) {
                            Scanned::Stored(value) => value as i16,
                            Scanned::NoMatch => 0,
                            Scanned::End => {
                                return Err(invalid(format!(
                                    "opinion modifier {}: empty short operand has no stored value",
                                    definition.token
                                )));
                            }
                        };
                        if field.keyword_is("value") {
                            definition.opinion = value;
                        } else if field.keyword_is("min_trust") {
                            min = value;
                        } else {
                            max = value;
                        }
                    }
                    Some(b"trade") => {
                        if let Some(value) =
                            parser::read_bool(field.value.scalar().unwrap_or_default())
                        {
                            definition.trade = value;
                        }
                    }
                    Some(b"target") => {
                        if let Some(value) =
                            parser::read_bool(field.value.scalar().unwrap_or_default())
                        {
                            target = value;
                        }
                        if target {
                            if definition.token.len() + 7 >= 128 {
                                return Err(invalid(format!(
                                    "opinion modifier {}: target name exceeds the 128-byte formatting buffer",
                                    definition.token
                                )));
                            }
                            definition.name = format!("{}_target", definition.token);
                        }
                    }
                    _ => {}
                }
            }
            if definition.opinion < min {
                definition.opinion = min;
            } else if definition.opinion > max {
                definition.opinion = max;
            }
            if !target {
                definitions.push(definition);
            }
        }
    }
    Ok((definitions, notes))
}

pub fn read(layers: &ResolvedLayers, defines: &content_reader_defines::defines::Defines) -> io::Result<Database> {
    let paths = layers.directory_files("common/opinion_modifiers", ".txt")?;
    if paths.is_empty() {
        return Ok(Database::default());
    }
    let bounds = [
        defines.int32("NDiplomacy", "MIN_TRUST_VALUE")? as i16,
        defines.int32("NDiplomacy", "MAX_TRUST_VALUE")? as i16,
    ];
    let mut database = Database::default();
    for path in paths {
        match parse_with_notes(&path.read(layers)?, bounds) {
            Ok((definitions, notes)) => {
                database.definitions.extend(definitions);
                database
                    .notes
                    .extend(notes.into_iter().map(|note| format!("{path}: {note}")));
            }
            Err(error) => {
                let reason =
                    format!("{path}: {error}; unread first-token definitions cannot be determined");
                database.notes.push(reason.clone());
                database.gap = Some(reason);
                break;
            }
        }
    }
    Ok(database)
}

#[cfg(test)]
mod tests;
