
use content_reader_parser::{DiagnosticKind, DiagnosticMessage, parser::Document};
use crate::{ResolvedLayers, content::LayerPath};
use std::{borrow::Cow, fmt, io};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct ScriptPath(std::sync::Arc<LayerPath>);

impl ScriptPath {
    pub fn new(path: LayerPath) -> Self {
        Self(std::sync::Arc::new(path))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }

    pub fn to_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    pub fn folder_text(&self) -> Option<&str> {
        let end = self.0.iter().rposition(|&byte| byte == b'/').unwrap_or(0);
        std::str::from_utf8(&self.0[..end]).ok()
    }

    pub fn read(&self, layers: &ResolvedLayers) -> io::Result<crate::content::Bytes> {
        layers.read_path(&self.0)
    }

    pub fn read_uncovered(&self, layers: &ResolvedLayers) -> io::Result<crate::content::Bytes> {
        layers.read_uncovered_path(&self.0)
    }
}

impl fmt::Display for ScriptPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

impl fmt::Debug for ScriptPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_str() {
            Some(text) => fmt::Debug::fmt(text, f),
            None => write!(f, "b\"{}\"", self.0.escape_ascii()),
        }
    }
}

impl PartialEq<&str> for ScriptPath {
    fn eq(&self, other: &&str) -> bool {
        self.0.as_slice() == other.as_bytes()
    }
}

impl From<&str> for ScriptPath {
    fn from(path: &str) -> Self {
        Self::new(path.as_bytes().to_vec())
    }
}

impl AsRef<[u8]> for ScriptPath {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

pub fn paths(
    layers: &ResolvedLayers,
    folder: &str,
    filter: Option<&str>,
) -> io::Result<Vec<ScriptPath>> {
    let mut files: Vec<(LayerPath, LayerPath)> = layers
        .listed_layer_paths(folder.as_bytes())?
        .into_iter()
        .filter_map(|path| {
            let name = path.get(folder.len()..)?.to_vec();
            (!name.is_empty()
                && !name.contains(&b'/')
                && filter.is_none_or(|filter| name.ends_with(filter.as_bytes())))
            .then_some((name, path))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files.into_iter().map(|(_, path)| ScriptPath::new(path)).collect())
}

pub fn is_database_file(path: &[u8], folder: &str) -> bool {
    path.to_ascii_lowercase()
        .strip_prefix(folder.as_bytes())
        .is_some_and(|name| !name.contains(&b'/') && name.ends_with(b".txt"))
}

pub fn database_paths(layers: &ResolvedLayers, folder: &str) -> io::Result<Vec<ScriptPath>> {
    let mut paths: Vec<ScriptPath> = layers
        .paths()?
        .filter(|path| is_database_file(path, folder))
        .map(|path| ScriptPath::new(path.to_vec()))
        .collect();
    paths.sort_by(|a, b| a.0[folder.len()..].cmp(&b.0[folder.len()..]));
    Ok(paths)
}

pub fn documents<D: DiagnosticMessage>(
    layers: &ResolvedLayers,
    folder: &str,
    what: &str,
    warnings: &mut Vec<D>,
) -> io::Result<Vec<(String, Document)>> {
    let mut result = Vec::new();
    for path in paths(layers, folder, Some(".txt"))? {
        match content_reader_parser::parser::parse_unclosed(&path.read(layers)?) {
            Ok((document, repaired)) => {
                if repaired {
                    warnings.push(D::from_diagnostic((
                        DiagnosticKind::Warning,
                        format!(
                            "{path} ends inside an open block; it was closed as the game closes it."
                        ),
                    )));
                }
                result.push((path.to_string(), document));
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
    use super::*;

    #[cfg(feature = "test-support")]
    #[test]
    fn a_folder_lists_as_the_database_loaders_list_it() {
        let root =
            std::env::temp_dir().join(format!("content-reader-script-paths-{}", std::process::id()));
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
            paths(&layers, "common/ideas/", Some(".txt")).unwrap(),
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
        assert_eq!(paths(&layers, "common/ideas/", None).unwrap().len(), 5);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_database_folder_orders_its_files_by_name_whatever_the_folder_case() {
        let root = std::env::temp_dir().join(format!(
            "content-reader-database-paths-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (base, over) = (root.join("base"), root.join("over"));
        for (dir, file) in [
            (&base, "common/abilities/a.txt"),
            (&base, "common/abilities/m.TXT"),
            (&over, "COMMON/ABILITIES/z.txt"),
            (&over, "COMMON/ABILITIES/nested/b.txt"),
            (&over, "COMMON/ABILITIES/notes.yml"),
        ] {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, file).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])]).unwrap();
        let listed: Vec<_> = database_paths(&layers, "common/abilities/").unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            listed,
            [
                "common/abilities/a.txt",
                "common/abilities/m.TXT",
                "COMMON/ABILITIES/z.txt"
            ]
        );
        assert!(is_database_file(
            b"COMMON/BUILDINGS/UPPER.TXT",
            "common/buildings/"
        ));
        assert!(!is_database_file(
            b"common/buildings/nested/air.txt",
            "common/buildings/"
        ));
    }

    #[test]
    fn an_archive_file_whose_name_is_not_utf8_is_listed_and_read() {
        use crate::test_support::{Record, zip_records};
        let root = std::env::temp_dir().join(format!(
            "content-reader-script-raw-name-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let archive = root.join("raw.zip");
        std::fs::write(
            &archive,
            zip_records(
                &[],
                &[Record {
                    name: b"common/ideas/\x80.txt",
                    host: 0,
                    flags: 0,
                    deflate: false,
                    data: b"raw",
                }],
            ),
        )
        .unwrap();
        let layers = ResolvedLayers::from_folders(&[(&archive, &[][..])]).unwrap();
        let listed = paths(&layers, "common/ideas/", Some(".txt")).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].as_bytes(), b"common/ideas/\x80.txt");
        assert_eq!(listed[0].read(&layers).unwrap(), b"raw");
        assert_eq!(listed[0].to_string(), "common/ideas/\u{FFFD}.txt");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_is_text_apart_from_its_file_name() {
        let path = |bytes: &[u8]| ScriptPath::new(bytes.to_vec());
        assert_eq!(path(b"sound/\x80.asset").folder_text(), Some("sound"));
        assert_eq!(path(b"sound/x/a.asset").folder_text(), Some("sound/x"));
        assert_eq!(path(b"a.asset").folder_text(), Some(""));
        assert_eq!(path(b"sound/\x80/a.asset").folder_text(), None);
        assert_eq!(format!("{:?}", path(b"sound/a.asset")), "\"sound/a.asset\"");
        assert_eq!(format!("{:?}", path(b"sound/\x80.asset")), "b\"sound/\\x80.asset\"");
    }
}

pub fn trigger_files(
    layers: &ResolvedLayers,
    folder: &str,
) -> io::Result<Vec<ScriptPath>> {
    layers.directory_files(folder, ".txt")
}
