//! The script files of one content folder in load order, as the database
//! loaders list them ([`paths`]). Each reader moved to the reader replay
//! ([`crate::parser::native`]) reads its files itself; [`documents`] is the
//! tree read kept for main's readers that have not moved yet (a follow-up
//! after the political layers, recorded in the plan).

use crate::{DiagnosticKind, DiagnosticMessage, ResolvedLayers, parser::Document};
use std::io;

/// The files a database loader reads from `folder` (lower case, ending in
/// `/`), in its order: `enumerate_directory_wrapper` (1424C73B0) or
/// `Vfs_EnumerateRecursive` (1424C7630), both with depth 0, so
/// `enumerate_directory` (1424C5170) does not enter subdirectories
/// (`param_7` 0 at 140A34150, 140A34320, 140712E90, 1401886F0, 1406025E0,
/// 140A9D0E0, 1406B0120, 1409B8510, 140AB3660 and 140AAE5A0). It keys the merged tree's
/// regular files (CollectVfsEnumerationEntry 1424C5EF0, types 0, 2, 3) by
/// name in a `std::map`, bytewise, so the files of every layer sort together
/// by name and a name listed by several layers is one entry, read from the
/// winning layer; `filter` is `ends_with` (1424B6AB0, exact bytes), and
/// `None` lists every file (1401886F0 passes no filter).
pub fn paths(layers: &ResolvedLayers, folder: &str, filter: Option<&str>) -> Vec<String> {
    let mut files: Vec<(String, String)> = layers
        .listed_paths(folder)
        .into_iter()
        .filter_map(|path| {
            let name = path.get(folder.len()..)?.to_owned();
            (!name.is_empty()
                && !name.contains('/')
                && filter.is_none_or(|filter| name.ends_with(filter)))
            .then_some((name, path))
        })
        .collect();
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    files.into_iter().map(|(_, path)| path).collect()
}

/// Each parsed file under `folder` in load order, as trees, for the content
/// readers not yet on the reader replay. `what` names the content in warnings.
pub fn documents<D: DiagnosticMessage>(
    layers: &ResolvedLayers,
    folder: &str,
    what: &str,
    warnings: &mut Vec<D>,
) -> io::Result<Vec<(String, Document)>> {
    let mut result = Vec::new();
    for path in paths(layers, folder, Some(".txt")) {
        match crate::parser::parse_unclosed(&layers.read(&path)?) {
            Ok((document, repaired)) => {
                if repaired {
                    warnings.push(D::from_diagnostic((
                        DiagnosticKind::Warning,
                        format!(
                            "{path} ends inside an open block; it was closed as the game closes it."
                        ),
                    )));
                }
                result.push((path, document));
            }
            Err(error) => warnings.push(D::from_diagnostic((
                DiagnosticKind::ParseFailure,
                format!("{what} unavailable in {path}: {error}"),
            ))),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "test-support")]
    use super::*;

    /// enumerate_directory (1424C5170) with depth 0: the regular files
    /// directly in the folder, every layer's names in one bytewise order, a
    /// name several layers list once, `ends_with` the filter exactly.
    #[cfg(feature = "test-support")]
    #[test]
    fn a_folder_lists_as_the_database_loaders_list_it() {
        let root =
            std::env::temp_dir().join(format!("installed-content-script-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (base, over) = (root.join("base"), root.join("over"));
        for (dir, file) in [
            (&base, "common/ideas/b.txt"),
            (&base, "common/ideas/a.txt"),
            (&base, "common/ideas/sub/c.txt"),
            (&over, "common/ideas/a.txt"),
            (&over, "common/ideas/0.txt"),
            (&over, "common/ideas/x.TXT"),
            (&over, "common/ideas/notes.yml"),
        ] {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, file).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])]).unwrap();
        assert_eq!(
            paths(&layers, "common/ideas/", Some(".txt")),
            [
                "common/ideas/0.txt",
                "common/ideas/a.txt",
                "common/ideas/b.txt"
            ]
        );
        assert_eq!(
            layers.read("common/ideas/a.txt").unwrap(),
            b"common/ideas/a.txt"
        );
        assert_eq!(paths(&layers, "common/ideas/", None).len(), 5);
        let _ = std::fs::remove_dir_all(&root);
    }
}
