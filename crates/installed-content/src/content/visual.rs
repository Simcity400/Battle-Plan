//! Explicit local cosmetic overlays, separate from recorded content verification.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualMod {
    pub id: String,
    pub name: String,
    pub recorded_id: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct VisualLayer {
    source: LayerSource,
    files: BTreeSet<String>,
}

struct Rule {
    directory: String,
    recursive: bool,
    extension: String,
}

fn rules(bytes: &[u8]) -> io::Result<Vec<Rule>> {
    let doc = parser::parse(bytes)?;
    let mut out = Vec::new();
    let mut block = Document::default();
    let finish = |block: &Document| -> io::Result<Rule> {
        if block.items.len() != 3 {
            return Err(invalid("Unsupported checksum manifest rule"));
        }
        let directory =
            scalar(block, b"name")?.ok_or_else(|| invalid("Missing checksum directory"))?;
        let extension = scalar(block, b"file_extension")?
            .ok_or_else(|| invalid("Missing checksum extension"))?;
        let recursive = match scalar(block, b"sub_directories")?.as_deref() {
            Some("yes") => true,
            Some("no") => false,
            _ => return Err(invalid("Unsupported checksum recursion rule")),
        };
        if !extension.starts_with('.') || extension.contains(['/', '\\', '*']) {
            return Err(invalid("Unsupported checksum extension"));
        }
        Ok(Rule {
            directory: normalize_relative(&directory)?.to_ascii_lowercase(),
            extension: extension.to_ascii_lowercase(),
            recursive,
        })
    };
    let mut started = false;
    for item in doc.items {
        if item.key.is_none() {
            if item.value.scalar_str() != Some("directory") {
                return Err(invalid("Unsupported checksum manifest section"));
            }
            if started {
                out.push(finish(&block)?);
                block.items.clear();
            }
            started = true;
        } else if started {
            block.items.push(item);
        } else {
            return Err(invalid("Checksum rule without a directory section"));
        }
    }
    if started {
        out.push(finish(&block)?);
    }
    if out.is_empty() {
        return Err(invalid("Empty checksum manifest"));
    }
    Ok(out)
}

fn affects_checksum(path: &str, rules: &[Rule]) -> bool {
    let path = path.to_ascii_lowercase();
    rules.iter().any(|rule| {
        path.strip_prefix(&format!("{}/", rule.directory))
            .is_some_and(|relative| {
                relative.ends_with(&rule.extension) && (rule.recursive || !relative.contains('/'))
            })
    })
}

fn supported(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    (path.starts_with("map/terrain/") || path.starts_with("gfx/"))
        && [".dds", ".tga", ".png", ".bmp"]
            .iter()
            .any(|extension| path.ends_with(extension))
}

fn metadata(path: &str) -> bool {
    !path.contains('/')
        && (path == "descriptor.mod"
            || path.starts_with("thumbnail")
            || path.starts_with("preview")
            || path.to_ascii_lowercase().starts_with("readme")
            || path.to_ascii_lowercase().starts_with("license")
            || path.starts_with('.'))
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

fn files(
    source: &LayerSource,
    rules: &[Rule],
    cancel: &AtomicBool,
) -> io::Result<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    let admit = |path: String, files: &mut BTreeSet<String>| -> io::Result<()> {
        if affects_checksum(&path, rules) {
            return Err(invalid(format!(
                "Contains checksum-sensitive files: {path}"
            )));
        }
        files.insert(path);
        Ok(())
    };
    match source {
        LayerSource::Folder(root) => {
            let mut pending = vec![root.clone()];
            let mut count = 0;
            while let Some(directory) = pending.pop() {
                for entry in fs::read_dir(directory)? {
                    cancelled(cancel)?;
                    count += 1;
                    if count > 20_000 {
                        return Err(invalid("Mod exceeds the cosmetic scan limit"));
                    }
                    let entry = entry?;
                    let kind = entry.file_type()?;
                    if kind.is_symlink() {
                        return Err(invalid("Linked mod files are not supported"));
                    }
                    if kind.is_dir() {
                        pending.push(entry.path());
                    } else if kind.is_file() {
                        let path = entry.path();
                        admit(
                            normalize_relative(text_path(
                                path.strip_prefix(root)
                                    .map_err(|_| invalid("Mod path escaped its folder"))?,
                            )?)?,
                            &mut files,
                        )?;
                    }
                }
            }
        }
        LayerSource::Archive(path) => {
            let mut archive = zip::ZipArchive::new(File::open(path)?)?;
            if archive.len() > 20_000 {
                return Err(invalid("Mod exceeds the cosmetic scan limit"));
            }
            for index in 0..archive.len() {
                cancelled(cancel)?;
                let entry = archive.by_index(index)?;
                if !entry.is_dir() {
                    admit(normalize_relative(entry.name())?, &mut files)?;
                }
            }
        }
    }
    Ok(files)
}

fn inspect(
    user: &Path,
    id: &str,
    rules: &[Rule],
    cancel: &AtomicBool,
) -> io::Result<(VisualMod, VisualLayer)> {
    if id.len() > 512
        || normalize_relative(id)? != id
        || !id.starts_with("mod/")
        || !id.ends_with(".mod")
        || id[4..].contains('/')
    {
        return Err(invalid("Invalid installed mod descriptor"));
    }
    let descriptor = user.join(id);
    let (layer, doc) = descriptor_layer(id.into(), descriptor.clone(), user, true)?;
    let recorded_id = mod_id(&doc, &descriptor)?;
    let name = scalar(&doc, b"name")?.unwrap_or_else(|| id.into());
    let files = match files(&layer.source, rules, cancel) {
        Ok(files) => files,
        Err(error) if error.kind() != io::ErrorKind::Interrupted => {
            return Ok((
                VisualMod {
                    id: id.into(),
                    name,
                    recorded_id,
                    reason: Some(error.to_string()),
                },
                VisualLayer {
                    source: layer.source,
                    files: BTreeSet::new(),
                },
            ));
        }
        Err(error) => return Err(error),
    };
    let reason = if let Some(path) = files.iter().find(|path| affects_checksum(path, rules)) {
        Some(format!("Contains checksum-sensitive files: {path}"))
    } else if !layer.replace_paths.is_empty() {
        Some("Uses replace_path; requires separate compatibility support".into())
    } else if let Some(path) = files
        .iter()
        .find(|path| !supported(path) && !metadata(path))
    {
        Some(format!("Requires unsupported visual features: {path}"))
    } else if !files.iter().any(|path| supported(path)) {
        Some("No supported map artwork".into())
    } else {
        None
    };
    Ok((
        VisualMod {
            id: id.into(),
            name,
            recorded_id,
            reason,
        },
        VisualLayer {
            source: layer.source,
            files: files.into_iter().filter(|path| supported(path)).collect(),
        },
    ))
}

pub fn installed(options: &ResolveOptions, cancel: &AtomicBool) -> io::Result<Vec<VisualMod>> {
    let rules = rules(&fs::read(options.game_dir.join("checksum_manifest.txt"))?)?;
    let directory = options.user_dir.join("mod");
    if !directory.try_exists()? {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(directory)? {
        cancelled(cancel)?;
        let entry = entry?;
        if !entry.file_type()?.is_file() || entry.path().extension().is_none_or(|ext| ext != "mod")
        {
            continue;
        }
        if out.len() >= 256 {
            return Err(invalid("Too many installed mod descriptors"));
        }
        let id = format!("mod/{}", entry.file_name().to_string_lossy());
        out.push(match inspect(&options.user_dir, &id, &rules, cancel) {
            Ok((item, _)) => item,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => VisualMod {
                name: entry.file_name().to_string_lossy().into_owned(),
                id,
                recorded_id: String::new(),
                reason: Some(error.to_string()),
            },
        });
    }
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    Ok(out)
}

/// Verify the original recording first. Only then replace explicitly selected cosmetic artwork.
pub fn apply(
    layers: &mut ResolvedLayers,
    options: &ResolveOptions,
    selected: &[String],
    cancel: &AtomicBool,
) -> io::Result<()> {
    if selected.len() > 64 {
        return Err(invalid("Too many visual mods"));
    }
    let rules = rules(&fs::read(options.game_dir.join("checksum_manifest.txt"))?)?;
    let mut overlays = Vec::new();
    let mut seen = BTreeSet::new();
    for id in selected {
        if !seen.insert(id) {
            return Err(invalid("Duplicate visual mod"));
        }
        let (item, layer) = inspect(&options.user_dir, id, &rules, cancel)?;
        if let Some(reason) = item.reason {
            return Err(invalid(format!("{}: {reason}", item.name)));
        }
        overlays.push(layer);
    }
    // Disable eligible recorded cosmetic layers when an explicit selection exists.
    // Gameplay layers and unsupported recorded layers remain verified and intact.
    let mut remove = BTreeSet::new();
    for layer in &layers.layers {
        if let Some(descriptor) = &layer.descriptor
            && let Ok(relative) = descriptor.strip_prefix(&options.user_dir)
            && let Ok(id) = normalize_relative(text_path(relative)?)
            && let Ok((item, _)) = inspect(&options.user_dir, &id, &rules, cancel)
            && item.reason.is_none()
        {
            remove.insert(layer.id.clone());
        }
    }
    layers.layers.retain(|layer| !remove.contains(&layer.id));
    layers.finish()?;
    layers.visual = overlays;
    Ok(())
}

pub(super) fn read(layers: &[VisualLayer], relative: &str) -> io::Result<Option<Vec<u8>>> {
    for layer in layers.iter().rev() {
        let matched = match &layer.source {
            LayerSource::Folder(_) => layer
                .files
                .iter()
                .find(|name| name.eq_ignore_ascii_case(relative)),
            LayerSource::Archive(_) => layer.files.get(relative),
        };
        let Some(name) = matched else {
            continue;
        };
        let bytes = match &layer.source {
            LayerSource::Folder(root) => {
                let path = root.join(name).canonicalize()?;
                if !path.starts_with(root.canonicalize()?) {
                    return Err(invalid("Visual asset escaped its mod folder"));
                }
                if fs::metadata(&path)?.len() > 256 * 1024 * 1024 {
                    return Err(invalid("Visual asset is too large"));
                }
                fs::read(path)?
            }
            LayerSource::Archive(path) => {
                let mut archive = zip::ZipArchive::new(File::open(path)?)?;
                let entry = archive.by_name(name)?;
                if entry.size() > 256 * 1024 * 1024 {
                    return Err(invalid("Visual asset is too large"));
                }
                let mut bytes = Vec::new();
                entry.take(256 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > 256 * 1024 * 1024 {
                    return Err(invalid("Visual asset is too large"));
                }
                bytes
            }
        };
        return Ok(Some(bytes));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checksum_rules_are_distinct_from_renderer_support() {
        let rules = rules(b"directory\nname=map\nsub_directories=yes\nfile_extension=.bmp\ndirectory\nname=common\nsub_directories=yes\nfile_extension=.txt").unwrap();
        assert!(affects_checksum("map/provinces.bmp", &rules));
        assert!(affects_checksum("COMMON/countries/colors.TXT", &rules));
        assert!(!affects_checksum("map/terrain/colormap.dds", &rules));
        assert!(supported("map/terrain/colormap.dds"));
        assert!(!supported("gfx/FX/terrain.shader"));
        assert!(!supported("interface/country.gui"));
    }
    #[test]
    fn unknown_rules_are_not_treated_as_checksum_neutral() {
        assert!(rules(b"file\nname=map/provinces.bmp").is_err());
        assert!(rules(b"").is_err());
        assert!(rules(b"directory\nname=map\nsub_directories=maybe\nfile_extension=.txt").is_err());
    }
}
