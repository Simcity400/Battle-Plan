//! The country tags content declares (`common/country_tags`).

use crate::{ResolvedLayers, parser, script_files};
use std::{collections::BTreeMap, io};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryTag {
    pub file: String,
    pub dynamic: bool,
}

/// The tags content declares, and the keys the engine's reader skips.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagIndex {
    /// Every declared tag and its country file, `common/` plus the value.
    pub files: BTreeMap<String, String>,
    /// Keys that are not three bytes, as `<path>: <key>`: the engine declares no tag
    /// for them, so neither does this reader, but they are counted, not dropped silently.
    pub skipped: Vec<String>,
}

impl TagIndex {
    /// Every skipped key as one counted warning.
    pub fn warning(&self) -> Option<String> {
        if self.skipped.is_empty() {
            return None;
        }
        Some(format!(
            "{} common/country_tags keys are not three bytes, so the game declares no tag for them: {}",
            self.skipped.len(),
            self.skipped.join("; "),
        ))
    }
}

/// `common/country_tags/*.txt`: declared tags and country files, with
/// `common/` prepended to each value. `dynamic_tags` marks where dynamic-country
/// slots begin without declaring a tag; following tags still register.
/// Keys must be exactly three bytes; others are counted in [`TagIndex::skipped`].
/// The first declaration wins. Block values are refused because their
/// consumption is unsupported.
pub fn country_tag_index(layers: &ResolvedLayers) -> io::Result<TagIndex> {
    let mut index = TagIndex::default();
    for path in index_paths(layers) {
        add_tag_index(&path, &layers.read(&path)?, &mut index)?;
    }
    Ok(index)
}

fn index_paths(layers: &ResolvedLayers) -> Vec<String> {
    layers
        .paths()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.starts_with("common/country_tags/") && lower.ends_with(".txt")
        })
        .map(str::to_owned)
        .collect()
}

/// Corpus inventory of declarations, including opaque longer identities the
/// engine can carry elsewhere. This does not widen the train/unit-model reader.
/// Includes the authored dynamic boundary, never inferred from a tag's spelling.
pub fn read_definitions(layers: &ResolvedLayers) -> io::Result<BTreeMap<String, CountryTag>> {
    let mut tags = BTreeMap::new();
    let mut dynamic = false;
    for path in script_files::paths(layers, "common/country_tags/", Some(".txt")) {
        add_definitions(&path, &layers.read(&path)?, &mut tags, &mut dynamic)?;
    }
    Ok(tags)
}

/// One index file's declarations, added in file order; the first wins.
fn add_tag_index(path: &str, bytes: &[u8], index: &mut TagIndex) -> io::Result<()> {
    let document = parser::parse(bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
    for item in &document.items {
        let Some(key) = item.key.as_deref() else {
            continue;
        };
        // The marker requires a keyword; quoted text does not match.
        if item.keyword_is("dynamic_tags") {
            continue;
        }
        if key.len() != 3 {
            index
                .skipped
                .push(format!("{path}: {}", String::from_utf8_lossy(key)));
            continue;
        }
        let file = item.value.scalar_str().ok_or_else(|| {
            invalid(format!(
                "{path}: country tag {} names its file with an unsupported block",
                String::from_utf8_lossy(key)
            ))
        })?;
        index
            .files
            .entry(String::from_utf8_lossy(key).into_owned())
            .or_insert_with(|| format!("common/{}", file.trim_matches('"')));
    }
    Ok(())
}

fn add_definitions(
    path: &str,
    bytes: &[u8],
    tags: &mut BTreeMap<String, CountryTag>,
    dynamic: &mut bool,
) -> io::Result<()> {
    let document = parser::parse(bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
    // The last marker snapshots the registration count as the first
    // dynamic slot. Its value is ignored, and the boundary survives file changes.
    for item in &document.items {
        if item.keyword_is("dynamic_tags") {
            for definition in tags.values_mut() {
                definition.dynamic = false;
            }
            *dynamic = true;
            continue;
        }
        let tag = item
            .key_str()
            .map_err(|error| invalid(format!("{path}: country tag: {error}")))?;
        if tag.is_empty() {
            return Err(invalid(format!("{path}: empty country tag")));
        }
        let file = item.value.read_str().map_err(|error| {
            invalid(format!(
                "{path}: country tag {tag:?} has an unsupported country file: {error}"
            ))
        })?;
        tags.entry(tag.to_owned()).or_insert_with(|| CountryTag {
            file: format!("common/{file}"),
            dynamic: *dynamic,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn definitions_use_database_enumeration_and_first_registration() {
        struct Folder(std::path::PathBuf);
        impl Drop for Folder {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "installed-content-country-tags-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _folder = Folder(root.clone());
        let (base, over) = (root.join("base"), root.join("over"));
        for (folder, path, text) in [
            (&base, "common/country_tags/a.TXT", "ABC=countries/base.txt"),
            (
                &base,
                "common/country_tags/nested/z.txt",
                "ABC=countries/ignored.txt DEF=countries/nested.txt",
            ),
            (&over, "common/country_tags/a.TXT", "ABC=countries/over.txt"),
            (&base, "common/country_tags/0.txt", "JKL=countries/base.txt"),
            (&over, "common/country_tags/0.txt", "JKL=countries/over.txt"),
            (
                &over,
                "common/country_tags/b.txt",
                "GHI=countries/later.txt JKL=countries/ignored.txt",
            ),
            (
                &over,
                "common/country_tags/notes.yml",
                "XYZ=countries/ignored.txt",
            ),
        ] {
            let file = folder.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])]).unwrap();
        let files = country_tag_index(&layers).unwrap().files;
        assert_eq!(
            files,
            BTreeMap::from([
                ("ABC".into(), "common/countries/over.txt".into()),
                ("DEF".into(), "common/countries/nested.txt".into()),
                ("GHI".into(), "common/countries/later.txt".into()),
                ("JKL".into(), "common/countries/over.txt".into()),
            ])
        );
        assert_eq!(
            read_definitions(&layers)
                .unwrap()
                .into_iter()
                .map(|(tag, definition)| (tag, definition.file))
                .collect::<BTreeMap<_, _>>(),
            BTreeMap::from([
                ("GHI".into(), "common/countries/later.txt".into()),
                ("JKL".into(), "common/countries/over.txt".into()),
            ])
        );
    }

    #[test]
    fn dynamic_slots_follow_content_and_first_registration() {
        let mut tags = BTreeMap::new();
        let mut dynamic = false;
        add_definitions("first.txt", br#"D01="countries/static.txt" dynamic_tags=yes "runtime slot"="countries/dynamic.txt""#, &mut tags, &mut dynamic).unwrap();
        add_definitions(
            "second.txt",
            br#"D01="countries/dynamic.txt" after="countries/dynamic.txt""#,
            &mut tags,
            &mut dynamic,
        )
        .unwrap();
        assert!(!tags["D01"].dynamic);
        assert!(tags["runtime slot"].dynamic);
        assert!(tags["after"].dynamic);
        add_definitions(
            "third.txt",
            br#"dynamic_tags=no fresh="countries/dynamic.txt""#,
            &mut tags,
            &mut dynamic,
        )
        .unwrap();
        assert!(!tags["runtime slot"].dynamic);
        assert!(!tags["after"].dynamic);
        assert!(tags["fresh"].dynamic);
    }

    #[test]
    fn tags_are_opaque_and_first_registration_wins() {
        let mut tags = BTreeMap::new();
        add_definitions("mod.txt", br#"dynamic_tags=yes "mod tag"="countries/mod.txt" long_token="countries/long.txt" "mod tag"="countries/ignored.txt" "dynamic_tags"="countries/literal.txt""#, &mut tags, &mut false).unwrap();
        assert_eq!(
            tags.into_iter()
                .map(|(tag, definition)| (tag, definition.file))
                .collect::<BTreeMap<_, _>>(),
            BTreeMap::from([
                ("mod tag".into(), "common/countries/mod.txt".into()),
                ("long_token".into(), "common/countries/long.txt".into()),
                ("dynamic_tags".into(), "common/countries/literal.txt".into()),
            ])
        );
        assert!(
            add_definitions(
                "refused.txt",
                b"tag={file=x}",
                &mut BTreeMap::new(),
                &mut false
            )
            .unwrap_err()
            .to_string()
            .contains("country tag \"tag\"")
        );
    }

    #[test]
    fn the_tag_index_skips_the_dynamic_marker_and_keeps_the_first_declaration() {
        let mut index = TagIndex::default();
        add_tag_index(
            "00_countries.txt",
            b"GER = \"countries/Germany.txt\"\nLONGKEY = \"x.txt\"\nGB = \"y.txt\"\n",
            &mut index,
        )
        .unwrap();
        add_tag_index(
            "zz_dynamic_countries.txt",
            b"DYNAMIC_TAGS = yes\nD01 = \"countries/D01.txt\"\nGER = \"countries/Other.txt\"\n",
            &mut index,
        )
        .unwrap();
        // A key that is not three bytes declares no tag, and is counted.
        assert_eq!(
            index.skipped,
            ["00_countries.txt: LONGKEY", "00_countries.txt: GB"]
        );
        assert!(
            index
                .warning()
                .unwrap()
                .starts_with("2 common/country_tags keys")
        );
        assert_eq!(
            index.files,
            BTreeMap::from([
                ("D01".to_owned(), "common/countries/D01.txt".to_owned()),
                ("GER".to_owned(), "common/countries/Germany.txt".to_owned()),
            ])
        );
        assert!(add_tag_index("bad.txt", b"SOV = { a = b }", &mut index).is_err());
        let extra = (0..8)
            .map(|id| format!("skipped_{id}=unused\n"))
            .collect::<String>();
        add_tag_index("extra.txt", extra.as_bytes(), &mut index).unwrap();
        assert_eq!(index.skipped.len(), 10);
        assert!(index.warning().unwrap().ends_with("extra.txt: skipped_7"));
        // Quoted text is not the dynamic boundary.
        add_tag_index("quoted.txt", b"\"dynamic_tags\" = unused", &mut index).unwrap();
        assert_eq!(index.skipped.len(), 11);
        assert_eq!(index.skipped.last().unwrap(), "quoted.txt: dynamic_tags");
    }
}
