//! Strategic resources (`common/resources`): each resource token and the frame
//! its icon takes in the interface's resource strip. The recording names
//! resources by token (`ResourceDefinitions` through LexicalTokens) and holds
//! their order; this only joins a token to its icon frame, so a mod's resources
//! show with their own icons.

use crate::{
    ResolvedLayers,
    parser::{self, Item},
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

fn key(item: &Item) -> Option<&str> {
    std::str::from_utf8(item.key.as_deref()?).ok()
}

pub fn parse(input: &[u8]) -> io::Result<Vec<Resource>> {
    Ok(from_document(&parser::parse_unclosed(input)?.0))
}

fn from_document(document: &parser::Document) -> Vec<Resource> {
    let mut resources = Vec::new();
    for root in document.get_all(b"resources") {
        for item in root.items().unwrap_or_default() {
            let (Some(token), Some(items)) = (key(item), item.value.items()) else {
                continue;
            };
            let icon_frame = items
                .iter()
                .rev()
                .find(|i| i.key.as_deref() == Some(b"icon_frame".as_slice()))
                .and_then(|i| i.value.scalar_str())
                .and_then(|text| text.parse::<u32>().ok())
                .filter(|frame| *frame > 0);
            resources.push(Resource {
                token: token.to_owned(),
                icon_frame,
            });
        }
    }
    resources
}

/// Every resource declaration in load order. A token declared again is kept as
/// another resource, as the game keeps it: its loader
/// (`CStrategicResourceDatabase::load_file@140AB2ED0`) interns each declared key
/// and appends a new item and key without looking for an earlier one.
pub fn read(layers: &ResolvedLayers) -> io::Result<(Vec<Resource>, Vec<String>)> {
    let mut warnings = Vec::new();
    let mut result: Vec<Resource> = Vec::new();
    for (_, document) in
        script_files::documents(layers, "common/resources/", "Resources", &mut warnings)?
    {
        result.extend(from_document(&document));
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

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
