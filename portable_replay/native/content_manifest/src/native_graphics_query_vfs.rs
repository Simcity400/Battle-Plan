//! Exact-build law and bounds: extractor/native/runs/native_graphics_query_a729/EVIDENCE.json.
//! Native query order is separate from asynchronous parser registration order.
use super::*;
use crate::native_graphics_query::{
    NativeGraphicsQueryEntry, NativeGraphicsQueryLayer,
    NativeGraphicsQueryUnavailable as Unavailable, NativeGraphicsQueryWitness,
    ObservedNativeGraphicsQuery, MAX_QUERY_BYTES, MAX_QUERY_DEPTH, MAX_QUERY_ENTRIES,
    MAX_QUERY_PATH_BYTES, MAX_QUERY_PAYLOAD_BYTES, NATIVE_GRAPHICS_EXECUTABLE_SHA256,
};

type QueryResult<T> = Result<T, Unavailable>;
type DirectoryRows = BTreeMap<String, bool>; // true is directory; names retain their bytes
type ArchiveTree = BTreeMap<String, DirectoryRows>;

struct QueryBudget {
    entries: usize,
    metadata_bytes: u64,
    payload_bytes: u64,
    max_entries: usize,
    max_metadata_bytes: u64,
    max_payload_bytes: u64,
    max_depth: usize,
}
impl Default for QueryBudget {
    fn default() -> Self {
        Self {
            entries: 0,
            metadata_bytes: 0,
            payload_bytes: 0,
            max_entries: MAX_QUERY_ENTRIES,
            max_metadata_bytes: MAX_QUERY_BYTES,
            max_payload_bytes: MAX_QUERY_PAYLOAD_BYTES,
            max_depth: MAX_QUERY_DEPTH,
        }
    }
}
impl QueryBudget {
    fn path(&mut self, value: &str) -> QueryResult<()> {
        if value.len() > MAX_QUERY_PATH_BYTES {
            return Err(Unavailable::BoundsExceeded);
        }
        self.entries = self
            .entries
            .checked_add(1)
            .filter(|n| *n <= self.max_entries)
            .ok_or(Unavailable::BoundsExceeded)?;
        // Charge retained names and their worst-case JSON escaping before allocation.
        let bytes = (value.len() as u64)
            .checked_mul(6)
            .and_then(|n| n.checked_add(64))
            .ok_or(Unavailable::BoundsExceeded)?;
        self.metadata_bytes = self
            .metadata_bytes
            .checked_add(bytes)
            .filter(|n| *n <= self.max_metadata_bytes)
            .ok_or(Unavailable::BoundsExceeded)?;
        Ok(())
    }
}

impl EffectiveVfs {
    /// Observe the exact native query through selected mounts. Only this producer
    /// enumerates roots; playback consumes the sealed sequence and authenticates
    /// its individual payloads. The seal reopens mounts and repeats the whole walk.
    pub fn observe_native_graphics_query(
        &self,
        executable_sha256: &str,
    ) -> Result<ObservedNativeGraphicsQuery, String> {
        validate_sha256(executable_sha256, "native graphics executable")?;
        let result = if executable_sha256 != NATIVE_GRAPHICS_EXECUTABLE_SHA256 {
            Err(Unavailable::UnsupportedBuild)
        } else if !cfg!(windows) {
            Err(Unavailable::UnsupportedHost)
        } else {
            self.observe_native_graphics_query_bounded(&mut QueryBudget::default())
        };
        Ok(ObservedNativeGraphicsQuery {
            witness: result.unwrap_or_else(NativeGraphicsQueryWitness::unavailable),
            executable_sha256: executable_sha256.into(),
        })
    }

    fn observe_native_graphics_query_bounded(
        &self,
        budget: &mut QueryBudget,
    ) -> QueryResult<NativeGraphicsQueryWitness> {
        let fresh = self
            .reopen_bounded(NamespaceEnumerationLimits {
                winners: MAX_QUERY_ENTRIES,
                entries: MAX_QUERY_ENTRIES,
                bytes: MAX_QUERY_BYTES,
            })
            .map_err(|error| {
                if error.contains("bound_exceeded") {
                    Unavailable::BoundsExceeded
                } else {
                    Unavailable::InputUnavailable
                }
            })?;
        // The native enumerator filters immediate files through getRealDir and
        // a physical-root cutoff. The folded VFS's open-stop rule is not that
        // operation, so it cannot authorize this query at a replacement boundary.
        if fresh
            .layers
            .iter()
            .flat_map(|l| &l.replace_paths)
            .any(|p| p == "interface" || p.starts_with("interface/"))
        {
            return Err(Unavailable::UnprovenReplacement);
        }
        let layers = fresh
            .layers
            .iter()
            .map(|layer| {
                budget.path(&layer.layer_id)?;
                for path in &layer.replace_paths {
                    budget.path(path)?;
                }
                Ok(NativeGraphicsQueryLayer {
                    ordinal: layer.ordinal,
                    layer_kind: layer.layer_kind,
                    layer_id: layer.layer_id.clone(),
                    storage_kind: layer.storage_kind,
                    source_container_sha256: layer.source_container_sha256.clone(),
                    replace_paths: layer.replace_paths.iter().cloned().collect(),
                })
            })
            .collect::<QueryResult<Vec<_>>>()?;
        let mut archives = Vec::with_capacity(fresh.layers.len());
        for layer in &fresh.layers {
            archives.push(if layer.storage_kind == LayerStorageKind::ZipArchive {
                Some(native_archive_tree(layer, budget)?)
            } else {
                None
            });
        }
        let mut scope_hash = Sha256::new();
        let mut pending = vec![("interface".to_string(), 0usize)];
        let mut entries = Vec::new();
        let mut hashed: BTreeMap<(usize, String), UsedContentKey> = BTreeMap::new();
        let mut buffer = Vec::new();
        while let Some((directory, depth)) = pending.pop() {
            budget.path(&directory)?;
            if depth > budget.max_depth {
                return Err(Unavailable::BoundsExceeded);
            }
            let mut callbacks = DirectoryRows::new();
            for (index, layer) in fresh.layers.iter().enumerate() {
                let (stored_directory, rows) = if let Some(tree) = &archives[index] {
                    let rows = tree.get(&directory).cloned();
                    (
                        rows.as_ref().map(|_| directory.clone()),
                        rows.unwrap_or_default(),
                    )
                } else {
                    native_directory_rows(layer, &directory, budget)?
                };
                // Includes absent directories, actual directory casing, entry
                // classification and nonmatching suffixes. Lower-name changes
                // cannot hide behind unchanged effective winner hashes.
                let bytes =
                    serde_json::to_vec(&(layer.ordinal, &directory, stored_directory, &rows))
                        .map_err(|_| Unavailable::InputUnavailable)?;
                scope_hash.update((bytes.len() as u64).to_le_bytes());
                scope_hash.update(bytes);
                for (name, is_dir) in rows {
                    budget.path(&name)?;
                    if callbacks
                        .insert(name, is_dir)
                        .is_some_and(|prior| prior != is_dir)
                    {
                        return Err(Unavailable::UnprovenEntryKind);
                    }
                }
            }
            for (name, is_dir) in &callbacks {
                let asked = format!("{directory}/{name}");
                // ZIP can classify an unknown callback through either local or
                // global stat. Requiring agreement makes both native branches
                // equivalent without guessing the runtime symlink policy.
                fresh.require_native_query_kind(&asked, *is_dir, &archives, budget)?;
                if *is_dir || !name.ends_with(".gfx") {
                    continue;
                }
                budget.path(&asked)?;
                let entry = fresh
                    .native_graphics_entry(&asked, budget)?
                    .ok_or(Unavailable::InputUnavailable)?;
                let identity = (entry.layer, entry.logical_path.clone());
                let content = if let Some(content) = hashed.get(&identity) {
                    content.clone()
                } else {
                    let limit = crate::MAX_CONTENT_PAYLOAD_BYTES.min(
                        budget
                            .max_payload_bytes
                            .saturating_sub(budget.payload_bytes),
                    );
                    let (bytes, digest) = fresh
                        .hash_entry_bounded(&entry, &mut buffer, &mut |_| {}, limit)
                        .map_err(|error| {
                            if error.contains("bound") {
                                Unavailable::BoundsExceeded
                            } else {
                                Unavailable::InputUnavailable
                            }
                        })?;
                    if let EffectiveVfsEntryFacts::File { len, .. } = &entry.facts {
                        if *len != bytes {
                            return Err(Unavailable::InputUnavailable);
                        }
                    }
                    budget.payload_bytes = budget
                        .payload_bytes
                        .checked_add(bytes)
                        .filter(|n| *n <= budget.max_payload_bytes)
                        .ok_or(Unavailable::BoundsExceeded)?;
                    let content = fresh
                        .content_key(&entry, digest, bytes)
                        .map_err(|_| Unavailable::InputUnavailable)?;
                    hashed.insert(identity, content.clone());
                    content
                };
                if normalize_game_relative_path(&asked)
                    .map_err(|_| Unavailable::UnprovenPathSpelling)?
                    != content.normalized_game_relative_path
                {
                    return Err(Unavailable::UnprovenPathSpelling);
                }
                entries.push(NativeGraphicsQueryEntry {
                    original_query_spelling: asked,
                    content,
                });
            }
            // Stack reversal preserves byte order while avoiding native-sized
            // recursion on the host stack. Immediate files have already emitted.
            for (name, is_dir) in callbacks.into_iter().rev() {
                if is_dir {
                    let child = format!("{directory}/{name}");
                    budget.path(&child)?;
                    if depth == budget.max_depth {
                        return Err(Unavailable::BoundsExceeded);
                    }
                    pending.push((child, depth + 1));
                }
            }
        }
        NativeGraphicsQueryWitness::from_observation(
            fresh.authority_manifest_sha256.clone(),
            layers,
            format!("{:x}", scope_hash.finalize()),
            entries,
        )
        .map_err(|_| Unavailable::BoundsExceeded)
    }

    fn native_graphics_entry(
        &self,
        asked: &str,
        budget: &mut QueryBudget,
    ) -> QueryResult<Option<EffectiveVfsEntry>> {
        for (index, layer) in self.layers.iter().enumerate().rev() {
            budget.path(asked)?;
            let facts_and_path = match layer.storage_kind {
                LayerStorageKind::Directory => {
                    match resolve_literal_layer_path(&layer.local_source, asked)
                        .map_err(|_| Unavailable::InputUnavailable)?
                    {
                        Some((path, facts)) if facts.is_file => {
                            let canonical = fs::canonicalize(&path)
                                .map_err(|_| Unavailable::InputUnavailable)?;
                            if !canonical.starts_with(&layer.local_source) {
                                return Err(Unavailable::InputUnavailable);
                            }
                            let stored = stored_directory_path(&layer.local_source, &canonical)
                                .map_err(|_| Unavailable::UnprovenPathSpelling)?;
                            let len = fs::metadata(&canonical)
                                .map_err(|_| Unavailable::InputUnavailable)?
                                .len();
                            Some((
                                stored,
                                EffectiveVfsEntryFacts::File {
                                    local_path: canonical,
                                    len,
                                    modified_unix_secs: 0,
                                    modified_nanos: 0,
                                },
                            ))
                        }
                        Some(_) => return Err(Unavailable::UnprovenEntryKind),
                        None => None,
                    }
                }
                LayerStorageKind::ZipArchive => {
                    let zip = layer
                        .zip_index
                        .as_ref()
                        .ok_or(Unavailable::InputUnavailable)?;
                    zip.entries
                        .get(&asked.to_ascii_lowercase())
                        .filter(|(stored, _)| stored == asked)
                        .map(|(stored, _)| {
                            (
                                stored.clone(),
                                EffectiveVfsEntryFacts::ArchiveMember {
                                    archive_sha256: layer.source_container_sha256.clone().unwrap(),
                                    entry_name: stored.clone(),
                                },
                            )
                        })
                }
            };
            if let Some((logical_path, facts)) = facts_and_path {
                return Ok(Some(EffectiveVfsEntry {
                    logical_path,
                    layer_kind: layer.layer_kind,
                    layer: index,
                    facts,
                }));
            }
        }
        Ok(None)
    }

    fn require_native_query_kind(
        &self,
        asked: &str,
        expected_directory: bool,
        archives: &[Option<ArchiveTree>],
        budget: &mut QueryBudget,
    ) -> QueryResult<()> {
        if asked.contains('$') {
            return Err(Unavailable::UnprovenPathSpelling);
        }
        let (parent, name) = asked
            .rsplit_once('/')
            .ok_or(Unavailable::UnprovenPathSpelling)?;
        for (index, layer) in self.layers.iter().enumerate().rev() {
            budget.path(asked)?;
            let kind = if let Some(tree) = &archives[index] {
                tree.get(parent).and_then(|rows| rows.get(name)).copied()
            } else {
                resolve_literal_layer_path(&layer.local_source, asked)
                    .map_err(|_| Unavailable::InputUnavailable)?
                    .map(|(_, facts)| {
                        if !facts.is_dir && !facts.is_file {
                            return Err(Unavailable::UnprovenEntryKind);
                        }
                        Ok(facts.is_dir)
                    })
                    .transpose()?
            };
            if let Some(is_directory) = kind {
                return if is_directory == expected_directory {
                    Ok(())
                } else {
                    Err(Unavailable::UnprovenEntryKind)
                };
            }
        }
        Err(Unavailable::InputUnavailable)
    }
}

fn native_directory_rows(
    layer: &MountedLayer,
    query: &str,
    budget: &mut QueryBudget,
) -> QueryResult<(Option<String>, DirectoryRows)> {
    let Some((directory, facts)) = resolve_literal_layer_path(&layer.local_source, query)
        .map_err(|_| Unavailable::InputUnavailable)?
    else {
        return Ok((None, BTreeMap::new()));
    };
    if !facts.is_dir {
        return Err(Unavailable::UnprovenEntryKind);
    }
    let canonical = fs::canonicalize(directory).map_err(|_| Unavailable::InputUnavailable)?;
    if !canonical.starts_with(&layer.local_source) {
        return Err(Unavailable::InputUnavailable);
    }
    let stored = stored_directory_path(&layer.local_source, &canonical)
        .map_err(|_| Unavailable::UnprovenPathSpelling)?;
    if normalize_game_relative_path(&stored).map_err(|_| Unavailable::UnprovenPathSpelling)?
        != normalize_game_relative_path(query).map_err(|_| Unavailable::UnprovenPathSpelling)?
    {
        return Err(Unavailable::UnprovenPathSpelling);
    }
    budget.path(&stored)?;
    let mut rows = DirectoryRows::new();
    for row in fs::read_dir(canonical).map_err(|_| Unavailable::InputUnavailable)? {
        let row = row.map_err(|_| Unavailable::InputUnavailable)?;
        let name = row
            .file_name()
            .into_string()
            .map_err(|_| Unavailable::UnprovenPathSpelling)?;
        budget.path(&name)?;
        if name.is_empty()
            || name
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\'))
        {
            return Err(Unavailable::UnprovenPathSpelling);
        }
        let (is_dir, is_file, ..) =
            listed_facts(&row).map_err(|_| Unavailable::InputUnavailable)?;
        if !is_dir && !is_file {
            return Err(Unavailable::UnprovenEntryKind);
        }
        rows.insert(name, is_dir);
    }
    Ok((Some(stored), rows))
}

fn native_archive_tree(layer: &MountedLayer, budget: &mut QueryBudget) -> QueryResult<ArchiveTree> {
    let index = layer
        .zip_index
        .as_ref()
        .ok_or(Unavailable::InputUnavailable)?;
    let mut archive = index
        .archive
        .lock()
        .map_err(|_| Unavailable::InputUnavailable)?;
    let mut tree = ArchiveTree::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|_| Unavailable::InputUnavailable)?;
        let raw = entry.name();
        budget.path(raw)?;
        let stored = normalize_archive_entry(raw).map_err(|_| Unavailable::InputUnavailable)?;
        // A normalized ZIP index is insufficient to authorize rewriting the
        // native callback's original bytes or the archive's raw path grammar.
        let exact_stored_spelling = if entry.is_dir() {
            raw.strip_suffix('/') == Some(stored.as_str())
        } else {
            raw == stored
        };
        if !exact_stored_spelling || std::str::from_utf8(entry.name_raw()).ok() != Some(raw) {
            return Err(Unavailable::UnprovenPathSpelling);
        }
        if !stored.starts_with("interface/") && stored != "interface" {
            continue;
        }
        let components = stored.split('/').collect::<Vec<_>>();
        if components.len() > budget.max_depth + 2 {
            return Err(Unavailable::BoundsExceeded);
        }
        let mut directory = String::new();
        for (i, name) in components.iter().enumerate() {
            let is_dir = i + 1 < components.len() || entry.is_dir();
            budget.path(&directory)?;
            budget.path(name)?;
            let rows = tree.entry(directory.clone()).or_default();
            if rows
                .insert((*name).into(), is_dir)
                .is_some_and(|prior| prior != is_dir)
            {
                return Err(Unavailable::UnprovenEntryKind);
            }
            if !directory.is_empty() {
                directory.push('/');
            }
            directory.push_str(name);
            if is_dir {
                tree.entry(directory.clone()).or_default();
            }
        }
    }
    Ok(tree)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::Write;

    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "native_gfx_query_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(root.join("base")).unwrap();
            fs::create_dir_all(root.join("upper")).unwrap();
            Self { root }
        }
        fn write(&self, layer: &str, relative: &str, bytes: &[u8]) {
            let path = self.root.join(layer).join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        fn mounts(&self) -> Vec<EffectiveVfsLayerMount> {
            ["base", "upper"]
                .into_iter()
                .enumerate()
                .map(|(ordinal, name)| EffectiveVfsLayerMount {
                    ordinal: ordinal as u32,
                    layer_kind: if ordinal == 0 {
                        ContentLayerKind::Base
                    } else {
                        ContentLayerKind::Mod
                    },
                    layer_id: name.into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: self.root.join(name),
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                })
                .collect()
        }
        fn vfs(&self) -> EffectiveVfs {
            EffectiveVfs::new("f".repeat(64), self.mounts()).unwrap()
        }
        fn zip_vfs(&self, names: &[(&str, &[u8])], zip_on_top: bool) -> EffectiveVfs {
            let path = self.root.join("archive.zip");
            let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, payload) in names {
                writer.start_file(*name, options).unwrap();
                writer.write_all(payload).unwrap();
            }
            writer.finish().unwrap();
            let mut mounts = self.mounts();
            if zip_on_top {
                mounts[1].storage_kind = LayerStorageKind::ZipArchive;
                mounts[1].local_source = path.clone();
                mounts[1].source_container_sha256 = Some(sha256_bytes(&fs::read(&path).unwrap()));
            } else {
                mounts.insert(
                    1,
                    EffectiveVfsLayerMount {
                        ordinal: 1,
                        layer_kind: ContentLayerKind::Dlc,
                        layer_id: "archive".into(),
                        storage_kind: LayerStorageKind::ZipArchive,
                        local_source: path.clone(),
                        source_container_sha256: Some(sha256_bytes(&fs::read(&path).unwrap())),
                        replace_paths: Vec::new(),
                    },
                );
                mounts[2].ordinal = 2;
            }
            EffectiveVfs::new("f".repeat(64), mounts).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn observe(vfs: &EffectiveVfs) -> ObservedNativeGraphicsQuery {
        vfs.observe_native_graphics_query(NATIVE_GRAPHICS_EXECUTABLE_SHA256)
            .unwrap()
    }

    #[test]
    fn files_precede_recursive_directories_suffix_is_exact_and_callbacks_deduplicate() {
        let f = Fixture::new();
        for path in [
            "interface/z.gfx",
            "interface/a.gfx",
            "interface/skip.GFX",
            "interface/a/deep.gfx",
            "interface/a/z/last.gfx",
            "interface/b/first.gfx",
            "other/outside.gfx",
        ] {
            f.write("base", path, b"base");
        }
        f.write("upper", "interface/a.gfx", b"upper");
        let vfs = f.vfs();
        let sealed = observe(&vfs).seal(&vfs).unwrap();
        let query = sealed.witness().complete().unwrap();
        assert_eq!(
            query
                .entries()
                .iter()
                .map(|e| e.original_query_spelling())
                .collect::<Vec<_>>(),
            [
                "interface/a.gfx",
                "interface/z.gfx",
                "interface/a/deep.gfx",
                "interface/a/z/last.gfx",
                "interface/b/first.gfx",
            ]
        );
        assert_eq!(query.entries()[0].content().winning_layer_ordinal, 1);
        assert_eq!(
            query.entries()[0].content().payload_sha256,
            sha256_bytes(b"upper")
        );
        assert_eq!(query.specification().directory(), "interface");
        assert_eq!(query.specification().suffix(), ".gfx");
        assert_eq!(query.specification().contains(), None);
        assert_eq!(query.specification().depth(), -1);
    }

    #[test]
    fn only_complete_empty_observation_mints_an_empty_roster() {
        let f = Fixture::new();
        let vfs = f.vfs();
        let witnessed = observe(&vfs).seal(&vfs).unwrap();
        assert!(witnessed.witness().complete().unwrap().entries().is_empty());
        assert_eq!(
            witnessed
                .witness()
                .required_layer_ordinals()
                .collect::<Vec<_>>(),
            [0, 1]
        );
        let unknown = vfs
            .observe_native_graphics_query(&"a".repeat(64))
            .unwrap()
            .seal(&vfs)
            .unwrap();
        assert_eq!(
            unknown.witness().unavailable_cause(),
            Some(&Unavailable::UnsupportedBuild)
        );
        fs::remove_dir(f.root.join("upper")).unwrap();
        assert_eq!(
            observe(&vfs).witness().unavailable_cause(),
            Some(&Unavailable::InputUnavailable)
        );
    }

    #[test]
    fn seal_rejects_changed_payload_scope_mounts_and_empty_authority() {
        let f = Fixture::new();
        f.write("base", "interface/a.gfx", b"one");
        let vfs = f.vfs();
        let before = observe(&vfs);
        f.write("base", "interface/a.gfx", b"two");
        assert_eq!(
            before.seal(&vfs).unwrap().witness().unavailable_cause(),
            Some(&Unavailable::ChangedDuringObservation)
        );
        let before = observe(&vfs);
        f.write(
            "base",
            "interface/ignored.txt",
            b"scope changes even without a matching suffix",
        );
        assert_eq!(
            before.seal(&vfs).unwrap().witness().unavailable_cause(),
            Some(&Unavailable::ChangedDuringObservation)
        );
        let before = observe(&vfs);
        let mut changed_mounts = f.mounts();
        changed_mounts[1].layer_id = "changed-unused-mount".into();
        let after = EffectiveVfs::new("f".repeat(64), changed_mounts).unwrap();
        assert_eq!(
            before.seal(&after).unwrap().witness().unavailable_cause(),
            Some(&Unavailable::ChangedDuringObservation)
        );
        let empty = Fixture::new();
        let before = observe(&empty.vfs());
        let after = EffectiveVfs::new("e".repeat(64), empty.mounts()).unwrap();
        assert_eq!(
            before.seal(&after).unwrap().witness().unavailable_cause(),
            Some(&Unavailable::ChangedDuringObservation)
        );
    }

    #[test]
    fn all_mount_classification_is_part_of_the_scope_and_ambiguity_refuses() {
        let f = Fixture::new();
        f.write("base", "interface/a.gfx", b"base");
        let vfs = f.vfs();
        let before = observe(&vfs);
        fs::create_dir_all(f.root.join("upper/interface/a.gfx")).unwrap();
        assert_eq!(
            observe(&vfs).witness().unavailable_cause(),
            Some(&Unavailable::UnprovenEntryKind)
        );
        assert_eq!(
            before.seal(&vfs).unwrap().witness().unavailable_cause(),
            Some(&Unavailable::ChangedDuringObservation)
        );
    }

    #[test]
    fn aggregate_enumeration_metadata_depth_and_payload_are_bounded() {
        let f = Fixture::new();
        f.write("base", "interface/a.gfx", b"1234");
        f.write("upper", "interface/b.gfx", b"5678");
        let vfs = f.vfs();
        let mut budget = QueryBudget {
            max_entries: 2,
            ..QueryBudget::default()
        };
        assert_eq!(
            vfs.observe_native_graphics_query_bounded(&mut budget),
            Err(Unavailable::BoundsExceeded)
        );
        let mut budget = QueryBudget {
            max_metadata_bytes: 100,
            ..QueryBudget::default()
        };
        assert_eq!(
            vfs.observe_native_graphics_query_bounded(&mut budget),
            Err(Unavailable::BoundsExceeded)
        );
        let mut budget = QueryBudget {
            max_payload_bytes: 7,
            ..QueryBudget::default()
        };
        assert_eq!(
            vfs.observe_native_graphics_query_bounded(&mut budget),
            Err(Unavailable::BoundsExceeded)
        );
        let mut budget = QueryBudget {
            max_payload_bytes: 8,
            ..QueryBudget::default()
        };
        assert!(vfs
            .observe_native_graphics_query_bounded(&mut budget)
            .is_ok());
        f.write("base", "interface/deep/a.gfx", b"1");
        let mut budget = QueryBudget {
            max_depth: 0,
            ..QueryBudget::default()
        };
        assert_eq!(
            vfs.observe_native_graphics_query_bounded(&mut budget),
            Err(Unavailable::BoundsExceeded)
        );
    }

    #[test]
    fn directory_and_zip_spellings_preserve_distinct_payloads_for_one_folded_path() {
        let f = Fixture::new();
        f.write("base", "interface/a.gfx", b"lower");
        let vfs = f.zip_vfs(&[("interface/A.gfx", b"upper")], true);
        let sealed = observe(&vfs).seal(&vfs).unwrap();
        let rows = sealed.witness().complete().unwrap().entries();
        assert_eq!(
            rows.iter()
                .map(|e| e.original_query_spelling())
                .collect::<Vec<_>>(),
            ["interface/A.gfx", "interface/a.gfx"]
        );
        assert_eq!(
            rows[0].content().normalized_game_relative_path,
            rows[1].content().normalized_game_relative_path
        );
        assert_eq!(rows[0].content().winning_layer_ordinal, 1);
        assert_eq!(rows[0].content().payload_sha256, sha256_bytes(b"upper"));
        assert_eq!(rows[1].content().winning_layer_ordinal, 0);
        assert_eq!(rows[1].content().payload_sha256, sha256_bytes(b"lower"));
        assert_eq!(sealed.witness().required_content().len(), 2);
    }

    #[test]
    fn multiple_native_spellings_can_share_one_directory_payload() {
        let f = Fixture::new();
        f.write("upper", "interface/A.gfx", b"directory winner");
        let vfs = f.zip_vfs(&[("interface/a.gfx", b"hidden ZIP payload")], false);
        let witness = observe(&vfs).seal(&vfs).unwrap().into_witness();
        let rows = witness.complete().unwrap().entries();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].original_query_spelling(), "interface/A.gfx");
        assert_eq!(rows[1].original_query_spelling(), "interface/a.gfx");
        assert_eq!(rows[0].content(), rows[1].content());
        assert_eq!(rows[0].content().winning_layer_ordinal, 2);
        assert_eq!(witness.required_content().len(), 1);
    }

    #[test]
    fn roots_follow_directory_os_lookup_and_zip_byte_lookup() {
        let f = Fixture::new();
        f.write("base", "Interface/Sub/loose.gfx", b"loose");
        let vfs = f.zip_vfs(
            &[
                ("Interface/Sub/hidden.gfx", b"hidden"),
                ("interface/Sub/visible.gfx", b"visible"),
            ],
            true,
        );
        let witness = observe(&vfs).seal(&vfs).unwrap().into_witness();
        let rows = witness.complete().unwrap().entries();
        assert_eq!(
            rows.iter()
                .map(|e| e.original_query_spelling())
                .collect::<Vec<_>>(),
            ["interface/Sub/loose.gfx", "interface/Sub/visible.gfx"]
        );
        assert_eq!(
            rows[0].content().stored_game_relative_path,
            "Interface/Sub/loose.gfx"
        );
        assert_eq!(
            rows[1].content().stored_game_relative_path,
            "interface/Sub/visible.gfx"
        );
    }

    #[test]
    fn adding_a_lower_spelling_invalidates_seal_even_when_folded_winner_is_unchanged() {
        let f = Fixture::new();
        f.write("upper", "interface/A.gfx", b"upper");
        let vfs = f.vfs();
        let winner_before = vfs.resolve("interface/a.gfx").unwrap().unwrap().content;
        let before = observe(&vfs);
        f.write("base", "interface/a.gfx", b"lower");
        assert_eq!(
            vfs.resolve("interface/a.gfx").unwrap().unwrap().content,
            winner_before
        );
        assert_eq!(
            before.seal(&vfs).unwrap().witness().unavailable_cause(),
            Some(&Unavailable::ChangedDuringObservation)
        );
    }

    #[test]
    fn archive_payloads_share_one_decompression_budget_and_type_aliases_refuse() {
        let f = Fixture::new();
        let vfs = f.zip_vfs(
            &[("interface/a.gfx", b"1234"), ("interface/b.gfx", b"5678")],
            true,
        );
        let mut budget = QueryBudget {
            max_payload_bytes: 7,
            ..QueryBudget::default()
        };
        assert_eq!(
            vfs.observe_native_graphics_query_bounded(&mut budget),
            Err(Unavailable::BoundsExceeded)
        );
        drop(vfs);
        f.write("upper", "interface/A.gfx/child.gfx", b"directory");
        let vfs = f.zip_vfs(&[("interface/a.gfx", b"file")], false);
        assert_eq!(
            observe(&vfs).witness().unavailable_cause(),
            Some(&Unavailable::UnprovenEntryKind)
        );
    }

    #[test]
    fn replacement_and_archive_suffix_fallback_are_explicitly_unavailable() {
        let f = Fixture::new();
        f.write("base", "interface/sub/lower.gfx", b"lower");
        let mut mounts = f.mounts();
        mounts[1].replace_paths.push("interface".into());
        let vfs = EffectiveVfs::new("f".repeat(64), mounts).unwrap();
        assert_eq!(
            observe(&vfs).witness().unavailable_cause(),
            Some(&Unavailable::UnprovenReplacement)
        );
        let f = Fixture::new();
        let vfs = f.zip_vfs(&[("interface/a$variant.gfx", b"unknown suffix rule")], true);
        assert_eq!(
            observe(&vfs).witness().unavailable_cause(),
            Some(&Unavailable::UnprovenPathSpelling)
        );
    }

    #[test]
    fn archive_directory_records_require_exactly_one_terminal_separator() {
        for (directory, expected_complete) in [("interface/sub/", true), ("interface/sub//", false)]
        {
            let f = Fixture::new();
            let vfs = f.zip_vfs(&[(directory, b""), ("other/file.bin", b"ordinary")], true);
            let observation = observe(&vfs);
            if expected_complete {
                assert!(observation
                    .witness()
                    .complete()
                    .unwrap()
                    .entries()
                    .is_empty());
            } else {
                assert_eq!(
                    observation.witness().unavailable_cause(),
                    Some(&Unavailable::UnprovenPathSpelling)
                );
            }
        }
    }
}
