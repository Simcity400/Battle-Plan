//! Strategic resources (`common/resources`): each resource token and the frame
//! its icon takes in the interface's resource strip. The recording names
//! resources by token (`ResourceDefinitions` through LexicalTokens) and holds
//! their order; this only joins a token to its icon frame, so a mod's resources
//! show with their own icons.

use crate::{
    ResolvedLayers,
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

pub fn parse(input: &[u8]) -> io::Result<Vec<Resource>> {
    parse_file(input)
        .map(|(resources, _)| resources)
        .map_err(|(_, error)| error)
}

fn parse_file(
    input: &[u8],
) -> Result<(Vec<Resource>, Vec<crate::Diagnostic>), (crate::DiagnosticKind, io::Error)> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Resources,
        Resource,
        Modifier(modifiers::Node),
    }
    let mut undecided = Vec::new();
    // LoadFile 140AB2ED0 reads both loops without an opening-brace check.
    // CStrategicResource::vf4 140BBD150 reads icon_frame (0x2F58), cic
    // (0x305F), convoys (0x3062), and reports/skips every unknown field.
    // Its modifier branch (0x2965) forwards this same field to CModifier's
    // vf4, NOT Read: verified by the tail jump at 140BBD1B0.
    // 140AB3660 constructs each file's lexer in mode 0 (1424A6370).
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        None,
        &mut |node, field| match node {
            Node::File if field.keyword_is("resources") => Shape::Object(Node::Resources),
            Node::File => Shape::Skip,
            Node::Resources => Shape::Object(Node::Resource),
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
        },
    );
    if let Some(cause) = read.unsettled {
        return Err((cause.kind, invalid(format!("resources: {cause}"))));
    }
    if let Some(cause) = undecided.first() {
        return Err((
            crate::DiagnosticKind::Unsettled,
            invalid(format!("resources: {cause}")),
        ));
    }
    let mut resources = Vec::new();
    let mut scans = modifiers::ScanFailures::default();
    for root in read
        .items
        .iter()
        .filter(|item| item.keyword_is("resources"))
    {
        for item in root.value.items().unwrap_or_default() {
            let token = item.key_str().map_err(|error| {
                (
                    crate::DiagnosticKind::Refused,
                    invalid(format!("resource name: {error}")),
                )
            })?;
            // Constructor 140BBB090 initializes +0xDC to zero. Each %i
            // scan (1424AB970 -> 1424B02C0) retains it on a failed scan.
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
    }
    let mut warnings: Vec<_> = read
        .misreads
        .iter()
        .map(|note| (note.kind, note.to_string()))
        .collect();
    warnings.extend(
        scans
            .warning()
            .map(|message| (crate::DiagnosticKind::Misread, message)),
    );
    Ok((resources, warnings))
}

/// Every resource declaration in load order. A token declared again is kept as
/// another resource, as the game keeps it: its loader
/// (`CStrategicResourceDatabase::load_file@140AB2ED0`) interns each declared key
/// and appends a new item and key without looking for an earlier one.
pub fn read(layers: &ResolvedLayers) -> io::Result<(Vec<Resource>, Vec<crate::Diagnostic>)> {
    let mut warnings = Vec::new();
    let mut result: Vec<Resource> = Vec::new();
    for path in script_files::paths(layers, "common/resources/", Some(".txt")) {
        match parse_file(&layers.read(&path)?) {
            Ok((resources, notes)) => {
                result.extend(resources);
                warnings.extend(
                    notes
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("Resources in {path}: {note}"))),
                );
            }
            Err((kind, error)) => {
                warnings.push((kind, format!("Resources unavailable in {path}: {error}")))
            }
        }
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_dispatch_variables_recovery_and_integer_scans() {
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
}
