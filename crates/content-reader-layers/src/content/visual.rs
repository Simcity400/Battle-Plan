use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

const WORKSHOP_APP_ID: &str = "394360";

fn workshop_request(item: &str) -> String {
    format!("mod/ugc_{item}.mod")
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledMod {
    pub id: String,
    pub name: String,
    pub recorded_id: String,
    pub source: String,
    pub problem: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Choice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub order: Vec<Choice>,
    pub warnings: Vec<String>,
    pub revision: String,
}

pub(super) struct Candidate {
    pub(super) item: InstalledMod,
    request: String,
    pub(super) descriptor: PathBuf,
    document: Option<Document>,
    pub(super) source: Option<LayerSource>,
    pub(super) replace_paths: Vec<LayerPath>,
    pub(super) workshop_item: Option<String>,
}

fn cancelled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Mod scan cancelled",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn workshop_root(game_dir: &Path) -> Option<PathBuf> {
    let game = fs::canonicalize(game_dir).ok()?;
    let common = game.parent()?;
    if !common.file_name()?.to_str()?.eq_ignore_ascii_case("common") {
        return None;
    }
    let root = common
        .parent()?
        .join("workshop")
        .join("content")
        .join(WORKSHOP_APP_ID);
    root.is_dir().then_some(root)
}

fn source_text(source: &LayerSource) -> String {
    let (LayerSource::Folder(path) | LayerSource::Archive(path)) = source;
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\")
        .map(str::to_owned)
        .unwrap_or(text)
}

fn source_key(source: &LayerSource) -> PathBuf {
    let (LayerSource::Folder(path) | LayerSource::Archive(path)) = source;
    path.clone()
}

fn describe(
    candidate: &mut Candidate,
    document: Document,
    locate: impl FnOnce(&Document) -> io::Result<LayerSource>,
) {
    let read = (|| -> io::Result<_> {
        if let Some(name) = scalar(&document, b"name")?.filter(|name| !name.is_empty()) {
            candidate.item.name = name;
        }
        candidate.item.recorded_id = mod_id(&document, Path::new(&candidate.request))?;
        candidate.replace_paths = document
            .get_all(b"replace_path")
            .map(|value| {
                value
                    .scalar()
                    .ok_or_else(|| invalid("replace_path is not a scalar"))
                    .and_then(replace_path)
            })
            .collect::<io::Result<_>>()?;
        locate(&document)
    })();
    match read {
        Ok(source) => {
            candidate.item.source = source_text(&source);
            candidate.source = Some(source);
        }
        Err(error) => candidate.item.problem = Some(error.to_string()),
    }
    candidate.document = Some(document);
}

fn local(user_dir: &Path, path: PathBuf) -> Candidate {
    let file = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let request = format!("mod/{file}");
    let mut candidate = Candidate {
        item: InstalledMod {
            id: request.clone(),
            name: file.clone(),
            recorded_id: file,
            source: String::new(),
            problem: None,
        },
        request,
        descriptor: path.clone(),
        document: None,
        source: None,
        replace_paths: Vec::new(),
        workshop_item: None,
    };
    match read_document(&path) {
        Ok(document) => describe(&mut candidate, document, |document| {
            let declared = |key: &[u8]| scalar(document, key);
            match (declared(b"path")?, declared(b"archive")?) {
                (Some(folder), None) => {
                    source_path(user_dir, folder.trim_end_matches(['/', '\\']), true)
                        .map(LayerSource::Folder)
                        .map_err(|error| {
                            invalid(format!("its folder {folder} cannot be read: {error}"))
                        })
                }
                (None, Some(archive)) => source_path(user_dir, &archive, true)
                    .map(LayerSource::Archive)
                    .map_err(|error| {
                        invalid(format!("its archive {archive} cannot be read: {error}"))
                    }),
                _ => Err(invalid(
                    "its descriptor must declare exactly one of path or archive",
                )),
            }
        }),
        Err(error) => candidate.item.problem = Some(error.to_string()),
    }
    candidate
}

fn workshop(folder: PathBuf, item: String) -> Candidate {
    let descriptor = folder.join("descriptor.mod");
    let request = workshop_request(&item);
    let mut candidate = Candidate {
        item: InstalledMod {
            id: format!("workshop/{item}"),
            name: item.clone(),
            recorded_id: item.clone(),
            source: source_text(&LayerSource::Folder(folder.clone())),
            problem: None,
        },
        request,
        descriptor: descriptor.clone(),
        document: None,
        source: None,
        replace_paths: Vec::new(),
        workshop_item: Some(item),
    };
    match read_document(&descriptor) {
        Ok(document) => describe(&mut candidate, document, |document| {
            match scalar(document, b"archive")? {
                Some(archive) => {
                    let name = Path::new(&archive).file_name().ok_or_else(|| {
                        invalid(format!("its archive {archive} has no file name"))
                    })?;
                    fs::canonicalize(folder.join(name))
                        .map(LayerSource::Archive)
                        .map_err(|error| {
                            invalid(format!("its archive {archive} cannot be read: {error}"))
                        })
                }
                None => Ok(LayerSource::Folder(folder)),
            }
        }),
        Err(error) => candidate.item.problem = Some(error.to_string()),
    }
    candidate
}

fn unreadable(id: String, name: String, problem: String) -> Candidate {
    Candidate {
        item: InstalledMod {
            id,
            recorded_id: name.clone(),
            name,
            source: String::new(),
            problem: Some(problem),
        },
        request: String::new(),
        descriptor: PathBuf::new(),
        document: None,
        source: None,
        replace_paths: Vec::new(),
        workshop_item: None,
    }
}

fn entries(directory: &Path, problems: &mut Vec<Candidate>) -> Vec<fs::DirEntry> {
    let named = || directory.display().to_string();
    let listed = match fs::read_dir(directory) {
        Ok(listed) => listed,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            problems.push(unreadable(
                format!("unreadable/{}", named()),
                named(),
                format!("this folder cannot be listed: {error}"),
            ));
            return Vec::new();
        }
    };
    let (mut entries, mut failed, mut first) = (Vec::new(), 0, None);
    for entry in listed {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(error) => {
                failed += 1;
                first.get_or_insert(error);
            }
        }
    }
    if let Some(error) = first {
        problems.push(unreadable(
            format!("unreadable/{}", named()),
            named(),
            format!("{failed} entries in this folder cannot be read, the first: {error}"),
        ));
    }
    entries.sort_by_key(|entry| entry.file_name());
    entries
}

fn discover(options: &ResolveOptions, cancel: &AtomicBool) -> io::Result<Vec<Candidate>> {
    discover_installed(Some(&options.game_dir), Some(&options.user_dir), cancel)
}

pub(super) fn discover_installed(
    game_dir: Option<&Path>,
    user_dir: Option<&Path>,
    cancel: &AtomicBool,
) -> io::Result<Vec<Candidate>> {
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut problems = Vec::new();
    if let Some(user_dir) = user_dir {
        for entry in entries(&user_dir.join("mod"), &mut problems) {
            cancelled(cancel)?;
            let path = entry.path();
            if !listed_mod_descriptor(&entry.file_name().to_string_lossy()) {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_file() => candidates.push(local(user_dir, path)),
                Ok(_) => {}
                Err(error) => {
                    let file = entry.file_name().to_string_lossy().into_owned();
                    problems.push(unreadable(
                        format!("mod/{file}"),
                        file,
                        format!("its descriptor cannot be read: {error}"),
                    ));
                }
            }
        }
    }
    let root = game_dir.and_then(workshop_root);
    if let Some(root) = &root {
        for candidate in &mut candidates {
            let folder = match &candidate.source {
                Some(LayerSource::Folder(folder)) => Some(folder.as_path()),
                Some(LayerSource::Archive(archive)) => archive.parent(),
                None => None,
            };
            if let Some(folder) = folder
                && folder.parent() == Some(root.as_path())
                && let Some(item) = folder.file_name().and_then(|name| name.to_str())
            {
                candidate.item.id = format!("workshop/{item}");
                candidate.workshop_item = Some(item.to_owned());
            }
        }
    }
    let mut sources: BTreeSet<PathBuf> = BTreeSet::new();
    let mut ids: BTreeSet<String> = BTreeSet::new();
    candidates.retain(|candidate| {
        candidate
            .source
            .as_ref()
            .is_none_or(|source| sources.insert(source_key(source)))
            && ids.insert(candidate.item.id.clone())
    });
    if let Some(root) = root {
        for entry in entries(&root, &mut problems) {
            cancelled(cancel)?;
            let Some(item) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let folder = match entry.file_type() {
                Ok(kind) if !kind.is_dir() => continue,
                Ok(_) => fs::canonicalize(entry.path()),
                Err(error) => Err(error),
            };
            let candidate = match folder {
                Ok(folder) => workshop(folder, item),
                Err(error) => unreadable(
                    format!("workshop/{item}"),
                    item,
                    format!("its Workshop folder cannot be read: {error}"),
                ),
            };
            if candidate
                .source
                .as_ref()
                .is_none_or(|source| !sources.contains(&source_key(source)))
                && ids.insert(candidate.item.id.clone())
            {
                if let Some(source) = &candidate.source {
                    sources.insert(source_key(source));
                }
                candidates.push(candidate);
            }
        }
    }
    for problem in problems {
        if ids.insert(problem.item.id.clone()) {
            candidates.push(problem);
        }
    }
    Ok(candidates)
}

pub fn installed(options: &ResolveOptions, cancel: &AtomicBool) -> io::Result<Vec<InstalledMod>> {
    let mut out: Vec<_> = discover(options, cancel)?
        .into_iter()
        .map(|candidate| candidate.item)
        .collect();
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedMod {
    pub id: String,
    pub name: String,
}

fn ordered(
    mut catalogue: Vec<Candidate>,
    selected: &[Choice],
    recorded: &[RecordedMod],
    warnings: &mut Vec<String>,
) -> Vec<Candidate> {
    let mut chosen = Vec::new();
    let mut seen = BTreeSet::new();
    for choice in selected {
        if !seen.insert(&choice.id) {
            continue;
        }
        let Some(index) = catalogue
            .iter()
            .position(|candidate| candidate.item.id == choice.id)
        else {
            warnings.push(format!(
                "Visual mod {} is not installed, so it is not shown.",
                choice.name
            ));
            continue;
        };
        let mut candidate = catalogue.swap_remove(index);
        if let Some(problem) = &candidate.item.problem {
            warnings.push(format!(
                "Visual mod {} is not shown: {problem}.",
                candidate.item.name
            ));
            continue;
        }
        if recorded.iter().any(|recorded| {
            recorded.name == candidate.item.name && recorded.id != candidate.item.recorded_id
        }) {
            warnings.push(format!(
                "Visual mod {} is not shown: the recording's mod of that name is mounted instead, as the game mounts one mod of a name.",
                candidate.item.name
            ));
            continue;
        }
        let Some(document) = candidate.document.take() else {
            continue;
        };
        match native_mod_order(
            vec![(candidate.request.clone(), (), document.clone())],
            &mut Vec::new(),
        ) {
            Ok(_) => chosen.push((candidate.request.clone(), candidate, document)),
            Err(error) => warnings.push(format!(
                "Visual mod {} is not shown: {error}.",
                candidate.item.name
            )),
        }
    }
    match native_mod_order(chosen, &mut Vec::new()) {
        Ok(order) => {
            let mut names = BTreeSet::new();
            order
                .into_iter()
                .map(|(_, candidate, _)| candidate)
                .filter(|candidate| {
                    let first = names.insert(candidate.item.name.clone());
                    if !first {
                        warnings.push(format!(
                            "Visual mod {} ({}) is not shown: another selected mod has that name, and the game mounts one mod of a name.",
                            candidate.item.name, candidate.item.id
                        ));
                    }
                    first
                })
                .collect()
        }
        Err(error) => {
            warnings.push(format!(
                "Visual mods are not shown: the engine cannot order this selection ({error})."
            ));
            Vec::new()
        }
    }
}

fn choice(candidate: &Candidate) -> Choice {
    Choice {
        id: candidate.item.id.clone(),
        name: candidate.item.name.clone(),
    }
}

pub fn plan(
    options: &ResolveOptions,
    selected: &[Choice],
    recorded: &[RecordedMod],
    cancel: &AtomicBool,
) -> io::Result<Plan> {
    let mut plan = Plan::default();
    plan.order = ordered(
        discover(options, cancel)?,
        selected,
        recorded,
        &mut plan.warnings,
    )
    .iter()
    .map(choice)
    .collect();
    Ok(plan)
}

pub fn revision(
    options: &ResolveOptions,
    selected: &[Choice],
    recorded: &[RecordedMod],
    cancel: &AtomicBool,
) -> io::Result<String> {
    if selected.is_empty() {
        return Ok(String::new());
    }
    let mut warnings = Vec::new();
    let mounted = ordered(
        discover(options, cancel)?,
        selected,
        recorded,
        &mut warnings,
    );
    let mut digest = Sha256::new();
    let mut field = |bytes: &[u8]| {
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    };
    for warning in &warnings {
        field(warning.as_bytes());
    }
    let stamp = |path: &Path| match Stamp::read(path) {
        Ok(stamp) => format!(
            "{} {} {}",
            stamp.size, stamp.modified_seconds, stamp.modified_nanos
        ),
        Err(error) => format!("unreadable: {error}"),
    };
    for candidate in mounted {
        cancelled(cancel)?;
        field(candidate.item.id.as_bytes());
        field(candidate.item.source.as_bytes());
        field(stamp(&candidate.descriptor).as_bytes());
        match &candidate.source {
            Some(LayerSource::Archive(path)) => field(stamp(path).as_bytes()),
            Some(LayerSource::Folder(root)) => {
                let root = match fs::canonicalize(root) {
                    Ok(root) => root,
                    Err(error) => {
                        field(format!("unreadable {}: {error}", root.display()).as_bytes());
                        continue;
                    }
                };
                let root = root.as_path();
                let kind = |entry: &fs::DirEntry| -> io::Result<Listed> {
                    cancelled(cancel)?;
                    let listed = entry_kind(entry)?;
                    let lower = entry
                        .path()
                        .strip_prefix(root)
                        .map(|relative| {
                            relative
                                .to_string_lossy()
                                .replace('\\', "/")
                                .to_ascii_lowercase()
                        })
                        .unwrap_or_default();
                    let shown = |root: &str| {
                        below(lower.as_bytes(), root.as_bytes())
                            || below(root.as_bytes(), lower.as_bytes())
                    };
                    Ok(match listed {
                        Listed::Directory | Listed::LinkedDirectory
                            if !(PRESENTATION_ROOTS.iter().any(|root| shown(root))
                                || shown(DEFINES_ROOT)) =>
                        {
                            Listed::Other
                        }
                        listed => listed,
                    })
                };
                let walked = walk_folder(root, root, true, &kind, &mut |relative, entry| {
                    if overlay_sees(true, &relative.to_ascii_lowercase()) {
                        field(relative.as_slice());
                        field(stamp(&entry.path()).as_bytes());
                    }
                    Ok(())
                });
                if let Err(error) = walked {
                    cancelled(cancel)?;
                    field(format!("unreadable {}: {error}", root.display()).as_bytes());
                }
            }
            None => {}
        }
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn apply(
    layers: &mut ResolvedLayers,
    options: &ResolveOptions,
    selected: &[Choice],
    recorded: &[RecordedMod],
    cancel: &AtomicBool,
) -> io::Result<Plan> {
    apply_with(layers, options, selected, recorded, cancel, || {
        revision(options, selected, recorded, cancel)
    })
}

fn apply_with(
    layers: &mut ResolvedLayers,
    options: &ResolveOptions,
    selected: &[Choice],
    recorded: &[RecordedMod],
    cancel: &AtomicBool,
    mut revision: impl FnMut() -> io::Result<String>,
) -> io::Result<Plan> {
    if selected.is_empty() {
        layers.presentation = None;
        layers.presentation_defines = None;
        return Ok(Plan::default());
    }
    loop {
        let before = revision()?;
        let mut plan = mount(layers, options, selected, recorded, cancel)?;
        if revision()? == before {
            plan.revision = before;
            return Ok(plan);
        }
    }
}

fn mount(
    layers: &mut ResolvedLayers,
    options: &ResolveOptions,
    selected: &[Choice],
    recorded: &[RecordedMod],
    cancel: &AtomicBool,
) -> io::Result<Plan> {
    layers.presentation = None;
    layers.presentation_defines = None;
    let mut plan = Plan::default();
    let mut overlays = Vec::new();
    for candidate in ordered(
        discover(options, cancel)?,
        selected,
        recorded,
        &mut plan.warnings,
    ) {
        cancelled(cancel)?;
        let Some(source) = candidate.source.clone() else {
            continue;
        };
        let overlay = |verified| {
            Some(Overlay {
                name: candidate.item.name.clone(),
                verified,
            })
        };
        let id = format!("visual:{}", candidate.item.id);
        let recorded = layers.layers.iter().find(|layer| {
            layer.overlay.is_none() && source_key(&layer.source) == source_key(&source)
        });
        let layer = match recorded {
            Some(recorded) => ResolvedLayer {
                id,
                overlay: overlay(true),
                ..recorded.clone()
            },
            None => {
                let mut layer = ResolvedLayer {
                    content_digest: [0; 32],
                    inventory_witnesses: Default::default(),
                    inventory_paths: Default::default(),
                    coverage: layers.coverage,
                    role: super::LayerRole::Visual,
                    facts: None,
                    id,
                    source,
                    descriptor: Some(candidate.descriptor.clone()),
                    replace_paths: candidate.replace_paths.clone(),
                    files: Default::default(),
                    lookup: Default::default(),
                    hashes: Default::default(),
                    archive_stamp: None,
                    descriptor_hash: None,
                    overlay: overlay(false),
                    display_name: Some(candidate.item.name.clone()),
                    art_unmounted: None,
                };
                if let Err(error) = layer.inventory() {
                    plan.warnings.push(format!(
                        "Visual mod {} is not shown: its files cannot be listed ({error}).",
                        candidate.item.name
                    ));
                    continue;
                }
                layer
            }
        };
        plan.order.push(choice(&candidate));
        overlays.push(layer);
    }
    if overlays.is_empty() {
        return Ok(plan);
    }
    let (presentation, refused) = layers.with_overlays(overlays);
    for (layer, error) in refused {
        let name = layer
            .overlay
            .map(|overlay| overlay.name)
            .unwrap_or_default();
        plan.order.retain(|choice| choice.name != name);
        plan.warnings.push(format!(
            "Visual mod {name} is not shown: it cannot be mounted with the others ({error})."
        ));
    }
    layers.presentation_defines = match presentation.as_ref().map(ResolvedLayers::with_defines) {
        Some(Ok(defines)) => Some(std::sync::Arc::new(defines)),
        Some(Err(error)) => {
            plan.warnings.push(format!(
                "The defines of visual mods {} are not shown: their scripts cannot be mounted                  ({error}), so graphics defines are the recording's.",
                plan.order
                    .iter()
                    .map(|choice| choice.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            None
        }
        None => None,
    };
    layers.presentation = presentation.map(std::sync::Arc::new);
    Ok(plan)
}

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "layer-fixtures"))]
pub mod fixtures;
