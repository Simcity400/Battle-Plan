
use content_reader_parser::{Diagnostic, DiagnosticKind, coverage::Gap, crt::{Scanned, scan_i32}, fields::invalid};
use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_script::{modifiers, native::{self, Shape}};
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resource {
    pub token: String,
    pub icon_frame: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resources {
    pub declarations: Vec<Resource>,
    numbered: usize,
    gaps: Vec<Gap>,
}

impl Resources {
    pub fn new(declarations: Vec<Resource>) -> Self {
        Self {
            numbered: declarations.len(),
            declarations,
            gaps: Vec::new(),
        }
    }

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

    pub fn numbered(&self) -> &[Resource] {
        &self.declarations[..self.numbered]
    }

    pub fn gap(&self) -> Option<&Gap> {
        self.gaps.first()
    }

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
    let mut stop: Option<(usize, DiagnosticKind, String)> = None;
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

pub fn read(layers: &ResolvedLayers) -> io::Result<(Resources, Vec<Diagnostic>)> {
    layers.recorded_product("strategic resources", 1, &[], read_resources).map(|resources| resources.as_ref().clone())
}

fn read_resources(layers: &ResolvedLayers) -> io::Result<(Resources, Vec<Diagnostic>)> {
    let mut warnings = Vec::new();
    let mut result = Resources::default();
    for path in script_files::paths(layers, "common/resources/", Some(".txt"))? {
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
        let resources = parse(
            b"resources = { oil = { icon_frame = 1 cic = 0.125 } mod_ore = { icon_frame = 9 } \
              broken = { icon_frame = 0 } scripted = { icon_frame = @frame } bare = { } }",
        )
        .unwrap();
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

    #[test]
    fn a_file_that_stops_short_leaves_every_later_declaration_unnumbered() {
        content_reader_script::test_game::enter();
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
