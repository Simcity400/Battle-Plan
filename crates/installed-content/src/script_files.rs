//! Reading every script file of one content folder in load order, as the
//! content readers do: files by layer then path, blocks a file leaves open closed
//! as the game closes them, and a file that cannot be parsed left out with a
//! warning so the others still load.

use crate::{ResolvedLayers, parser::Document};
use std::io;

/// The `.txt` files under `folder` (lower case), in load order. Folder and
/// extension match without regard to ASCII case, as the content layers resolve
/// paths, so a mod's `common/Ideas/` loads too.
pub fn paths(layers: &ResolvedLayers, folder: &str) -> Vec<String> {
    let mut paths = layers
        .paths()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.starts_with(folder) && lower.ends_with(".txt")
        })
        .map(|path| (layers.layer_index(path).unwrap_or(0), path.to_owned()))
        .collect::<Vec<_>>();
    paths.sort();
    paths.into_iter().map(|(_, path)| path).collect()
}

/// Each parsed file under `folder` in load order. `what` names the content in warnings.
pub fn documents(
    layers: &ResolvedLayers,
    folder: &str,
    what: &str,
    warnings: &mut Vec<String>,
) -> io::Result<Vec<(String, Document)>> {
    let mut result = Vec::new();
    for path in paths(layers, folder) {
        match crate::parser::parse_unclosed(&layers.read(&path)?) {
            Ok((document, repaired)) => {
                if repaired {
                    warnings.push(format!(
                        "{path} ends inside an open block; it was closed as the game closes it."
                    ));
                }
                result.push((path, document));
            }
            Err(error) => warnings.push(format!("{what} unavailable in {path}: {error}")),
        }
    }
    Ok(result)
}
