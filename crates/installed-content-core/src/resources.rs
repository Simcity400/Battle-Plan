//! Strategic resources (`common/resources`): each resource token and the frame
//! its icon takes in the interface's resource strip. The recording names
//! resources by token (`ResourceDefinitions` through LexicalTokens) and holds
//! their order; this only joins a token to its icon frame, so a mod's resources
//! show with their own icons. A file whose reading stops short keeps the
//! declarations before the stop; what it declares after it is unknown, so no
//! later declaration has a known place in the database's order ([`Resources`]).

use crate::{
    Diagnostic, DiagnosticKind, ResolvedLayers,
    coverage::Gap,
    crt::{Scanned, scan_i32},
    fields::invalid,
    modifiers,
    parser::native::{self, Shape},
    script_files,
};
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resource {
    pub token: String,
    /// The one-based frame of the resource strip; None when missing or not a
    /// positive whole number.
    pub icon_frame: Option<u32>,
}

/// Every resource declaration of the content stack in load order. A repeated
/// token is another resource: each declared key is interned and appended with a
/// new item, without looking for an earlier declaration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resources {
    /// In load order; a file whose reading stops short gives those before the stop.
    pub declarations: Vec<Resource>,
    /// How many declarations come before the first file that stops short.
    numbered: usize,
    /// Every file that stops short, in load order.
    gaps: Vec<Gap>,
}

impl Resources {
    /// These declarations, read from files that all read to their end.
    pub fn new(declarations: Vec<Resource>) -> Self {
        Self {
            numbered: declarations.len(),
            declarations,
            gaps: Vec::new(),
        }
    }

    /// These declarations, from files the last of which stops short at `path` from `cause`,
    /// for other modules' tests.
    #[cfg(test)]
    pub(crate) fn stopped(declarations: Vec<Resource>, path: &str, cause: &str) -> Self {
        Self {
            numbered: declarations.len(),
            declarations,
            gaps: vec![Gap {
                path: path.to_owned(),
                cause: cause.to_owned(),
            }],
        }
    }

    /// The declarations whose place in the database's order is known: every one
    /// before the first file that stops short. Declaration `i` is database index `i + 1`.
    pub fn numbered(&self) -> &[Resource] {
        &self.declarations[..self.numbered]
    }

    /// The first file that stops short: whatever it and every later file declare
    /// has no known place in the database's order.
    pub fn gap(&self) -> Option<&Gap> {
        self.gaps.first()
    }

    /// The `ordinal`th (one-based) declaration of `token`: None when the content
    /// has no such declaration, the gap when a file that stops short could hold it
    /// or one before it.
    pub fn declaration(&self, token: &str, ordinal: u32) -> Result<Option<&Resource>, &Gap> {
        let found = ordinal.checked_sub(1).and_then(|index| {
            self.numbered()
                .iter()
                .filter(|resource| resource.token == token)
                .nth(index as usize)
        });
        match (found, self.gap()) {
            (Some(resource), _) => Ok(Some(resource)),
            (None, Some(gap)) if ordinal > 0 => Err(gap),
            (None, _) => Ok(None),
        }
    }
}

pub fn parse(input: &[u8]) -> io::Result<Vec<Resource>> {
    match parse_file(input) {
        (resources, _, None) => Ok(resources),
        (_, _, Some((_, why))) => Err(invalid(why)),
    }
}

/// One file's declarations up to where its reading stops, its notes, and the
/// stop's kind and cause.
type FileRead = (
    Vec<Resource>,
    Vec<Diagnostic>,
    Option<(DiagnosticKind, String)>,
);

fn parse_file(input: &[u8]) -> FileRead {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Resources,
        Resource,
        Modifier(modifiers::Node),
    }
    let mut undecided = Vec::new();
    let mut started = 0_usize;
    // The declaration whose reading the port cannot follow: it and every later one are unknown.
    let mut stop: Option<(usize, DiagnosticKind, String)> = None;
    // Both database loops read without an opening-brace check. Resource
    // fields consume icon_frame, cic and convoys, and report/skip unknown
    // fields. The modifier branch forwards the same field to modifier
    // dispatch rather than opening a separate persistent-reader block.
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        None,
        &mut |node, field| {
            let shape = match node {
                Node::File if field.keyword_is("resources") => Shape::Object(Node::Resources),
                Node::File => Shape::Skip,
                Node::Resources => {
                    started += 1;
                    Shape::Object(Node::Resource)
                }
                Node::Resource => match field.keyword().as_deref() {
                    Some(b"icon_frame" | b"cic" | b"convoys") => Shape::One,
                    Some(b"modifier") => modifiers::shape(
                        modifiers::Node::Modifier,
                        field,
                        modifiers::UNKNOWN,
                        &mut undecided,
                    )
                    .map(Node::Modifier),
                    _ => Shape::Skip,
                },
                Node::Modifier(node) => {
                    modifiers::shape(node, field, modifiers::UNKNOWN, &mut undecided)
                        .map(Node::Modifier)
                }
            };
            if stop.is_none()
                && let Some(first) = undecided.first()
            {
                stop = Some((
                    started.saturating_sub(1),
                    DiagnosticKind::Unsettled,
                    first.to_string(),
                ));
            }
            shape
        },
    );
    if stop.is_none()
        && let Some(cause) = &read.unsettled
    {
        stop = Some((started.saturating_sub(1), cause.kind, cause.to_string()));
    }
    let mut resources = Vec::new();
    let mut scans = modifiers::ScanFailures::default();
    let entries = read
        .items
        .iter()
        .filter(|item| item.keyword_is("resources"))
        .flat_map(|root| root.value.items().unwrap_or_default());
    for (index, item) in entries.enumerate() {
        if stop.as_ref().is_some_and(|(at, _, _)| index >= *at) {
            break;
        }
        let token = match item.key_str() {
            Ok(token) => token,
            Err(error) => {
                stop = Some((
                    index,
                    DiagnosticKind::Refused,
                    format!("resource name: {error}"),
                ));
                break;
            }
        };
        // The frame starts at zero. Each signed integer scan retains
        // its previous value on failure.
        let mut icon_frame = 0;
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("icon_frame") {
                scans.int32(token, field);
                if let Some(text) = field.value.scalar()
                    && let Scanned::Stored(value) = scan_i32(text)
                {
                    icon_frame = value;
                }
            }
        }
        resources.push(Resource {
            token: token.to_owned(),
            icon_frame: (icon_frame > 0).then_some(icon_frame as u32),
        });
    }
    let mut warnings: Vec<_> = read
        .misreads
        .iter()
        .map(|note| (note.kind, note.to_string()))
        .collect();
    warnings.extend(
        scans
            .warning()
            .map(|message| (DiagnosticKind::Misread, message)),
    );
    (resources, warnings, stop.map(|(_, kind, why)| (kind, why)))
}

/// Every resource declaration in load order, and the files whose reading stops short.
pub fn read(layers: &ResolvedLayers) -> io::Result<(Resources, Vec<Diagnostic>)> {
    let mut warnings = Vec::new();
    let mut result = Resources::default();
    for path in script_files::paths(layers, "common/resources/", Some(".txt")) {
        let path_text = path.to_string();
        let (resources, notes, stop) = parse_file(&path.read(layers)?);
        result.declarations.extend(resources);
        if result.gaps.is_empty() {
            result.numbered = result.declarations.len();
        }
        warnings.extend(
            notes
                .into_iter()
                .map(|(kind, note)| (kind, format!("Resources in {path_text}: {note}"))),
        );
        if let Some((kind, why)) = stop {
            warnings.push((
                kind,
                format!(
                    "Resources in {path_text} are unknown from {why}; no later declaration has a known place in the resource order."
                ),
            ));
            result.gaps.push(Gap {
                path: path_text,
                cause: why,
            });
        }
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_dispatch_variables_recovery_and_integer_scans() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = parse(
            br#"
            @frame = "0x10"
            "resources" = { ignored = { icon_frame = 2 } }
            RESOURCES = { ore = {
                unknown = { icon_frame = 99 @frame = 77 }
                ICON_FRAME = @frame icon_frame = invalid
                "icon_frame" = 3
            } }
        "#,
        )
        .unwrap();
        assert_eq!(
            resources,
            [Resource {
                token: "ore".into(),
                icon_frame: Some(16)
            }]
        );
        // A scalar reader consumes only `{`; the enclosing reader then
        // reads icon_frame=7 and the close ends that resource.
        let resources = parse(b"resources={ore={cic={icon_frame=7}}}").unwrap();
        assert_eq!(resources[0].icon_frame, Some(7));
        assert!(
            parse(b"resources={ore={modifier={x=1}}}")
                .unwrap_err()
                .to_string()
                .contains("modifier")
        );
        assert!(parse(b"resources={\xff").unwrap().is_empty());
    }

    #[test]
    fn resources_keep_their_icon_frames_and_refuse_unusable_ones() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = parse(
            b"resources = { oil = { icon_frame = 1 cic = 0.125 } mod_ore = { icon_frame = 9 } \
              broken = { icon_frame = 0 } scripted = { icon_frame = @frame } bare = { } }",
        )
        .unwrap();
        // A repeated token is another declaration, kept in order as the game keeps it.
        let repeated =
            parse(b"resources = { oil = { icon_frame = 1 } oil = { icon_frame = 8 } }").unwrap();
        assert_eq!(
            repeated.iter().map(|r| r.icon_frame).collect::<Vec<_>>(),
            [Some(1), Some(8)]
        );
        let frames: Vec<_> = resources
            .iter()
            .map(|r| (r.token.as_str(), r.icon_frame))
            .collect();
        assert_eq!(
            frames,
            [
                ("oil", Some(1)),
                ("mod_ore", Some(9)),
                ("broken", None),
                ("scripted", None),
                ("bare", None)
            ]
        );
    }

    /// A file that stops short keeps the declarations before the declaration it
    /// stops in; neither that one nor any later one, in it or a later file, is numbered.
    #[test]
    fn a_file_that_stops_short_leaves_every_later_declaration_unnumbered() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        // The resource reader hands `modifier` itself to the modifier dispatch.
        let (kept, _, stop) = parse_file(
            b"resources = { oil = { icon_frame = 2 } oil = { icon_frame = 5 modifier = { x = 1 } } steel = { icon_frame = 3 } }",
        );
        assert_eq!(
            kept,
            [Resource {
                token: "oil".into(),
                icon_frame: Some(2)
            }]
        );
        let (kind, why) = stop.unwrap();
        assert_eq!(kind, DiagnosticKind::Unsettled);
        assert!(why.contains("modifier has a block value"), "{why}");
        let declared = |token: &str, icon_frame| Resource {
            token: token.into(),
            icon_frame: Some(icon_frame),
        };
        // Three files: the middle one repeats oil, then stops.
        let mut resources = Resources::new(vec![declared("oil", 1), declared("steel", 4)]);
        resources.declarations.push(declared("oil", 2));
        resources.numbered = resources.declarations.len();
        resources.gaps.push(Gap {
            path: "common/resources/b.txt".into(),
            cause: why,
        });
        resources.declarations.push(declared("oil", 3));
        assert_eq!(resources.numbered().len(), 3);
        assert_eq!(
            resources.declaration("oil", 1).unwrap().unwrap().icon_frame,
            Some(1)
        );
        assert_eq!(
            resources.declaration("oil", 2).unwrap().unwrap().icon_frame,
            Some(2)
        );
        // The third file's oil is read, but the stopped file could declare more oil before it.
        for (token, ordinal) in [("oil", 3), ("steel", 2), ("absent", 1)] {
            assert_eq!(
                resources.declaration(token, ordinal).unwrap_err().path,
                "common/resources/b.txt"
            );
        }
        assert_eq!(resources.declaration("oil", 0), Ok(None));
        let complete = Resources::new(vec![declared("oil", 1)]);
        assert_eq!(complete.declaration("oil", 2), Ok(None));
        assert!(complete.gap().is_none());
    }
}
