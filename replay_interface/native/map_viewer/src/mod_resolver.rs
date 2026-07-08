use crate::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameFileResolverOptions {
    pub mod_paths: Vec<PathBuf>,
    pub use_dlc_load: bool,
    pub userdata: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BundleModSelectionOptions {
    pub(crate) mods: Vec<PathBuf>,
    pub(crate) use_dlc_load: bool,
    pub(crate) userdata: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct BundleResolverSetup {
    pub(crate) resolver: FileResolver,
    explicit_modset: bool,
    selection_source: String,
    warnings: Vec<String>,
}

impl BundleResolverSetup {
    pub(crate) fn from_options(
        vanilla_root: PathBuf,
        options: &BundleModSelectionOptions,
    ) -> Result<Self> {
        let vanilla_root = require_dir(&vanilla_root, "game directory")?;
        let mut warnings = Vec::new();
        if options.use_dlc_load {
            let userdata = options
                .userdata
                .as_ref()
                .ok_or("pass --userdata <the game user-data dir> with --use-dlc-load")?;
            let mods = mods_from_dlc_load(userdata, &mut warnings)?;
            let resolver = FileResolver::with_mod_layers(vanilla_root, mods);
            Ok(Self {
                resolver,
                explicit_modset: true,
                selection_source: "dlc_load.json".to_string(),
                warnings,
            })
        } else if !options.mods.is_empty() {
            let mods = options
                .mods
                .iter()
                .enumerate()
                .filter_map(|(idx, root)| explicit_mod_layer(idx, root, &mut warnings))
                .collect::<Vec<_>>();
            let resolver = FileResolver::with_mod_layers(vanilla_root, mods);
            Ok(Self {
                resolver,
                explicit_modset: true,
                selection_source: "explicit --mod".to_string(),
                warnings,
            })
        } else {
            Ok(Self {
                resolver: FileResolver::vanilla(vanilla_root),
                explicit_modset: false,
                selection_source: "vanilla-only".to_string(),
                warnings,
            })
        }
    }

    pub(crate) fn manifest_modset(&self) -> Option<BundleModSet> {
        if !self.explicit_modset {
            return None;
        }
        let stats = self.resolver.stats();
        let mods = self.resolver.mods();
        Some(BundleModSet {
            modset_id: modset_id(&mods),
            selection_source: self.selection_source.clone(),
            mod_count: mods.len(),
            mods,
            overridden_file_count: stats.overridden_file_count,
            replace_path_excluded_file_count: stats.replace_path_excluded_file_count,
            warnings: self.warnings.clone(),
            note: "Ordered lowest-priority mod to highest-priority mod over vanilla; file-level override by relative path with later mods winning; replace_path removes lower-layer files only in the exact directory, not subdirectories.".to_string(),
        })
    }

    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ModContentLayer {
    pub(crate) root: PathBuf,
    pub(crate) manifest: BundleMod,
}

#[derive(Clone, Debug)]
struct ResolverLayer {
    root: PathBuf,
    manifest: Option<BundleMod>,
    replace_paths: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct FileResolver {
    layers: Vec<ResolverLayer>,
    stats: RefCell<ResolverStats>,
}

#[derive(Clone, Debug, Default)]
struct ResolverStats {
    overridden_files: BTreeSet<String>,
    replace_path_excluded_files: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct FileResolverStats {
    pub(crate) overridden_file_count: usize,
    pub(crate) replace_path_excluded_file_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameFileResolverStats {
    pub overridden_file_count: usize,
    pub replace_path_excluded_file_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensitiveFileOverrideReport {
    pub overridden: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct GameFileResolver {
    resolver: FileResolver,
    selection_source: String,
    explicit_modset: bool,
    warnings: Vec<String>,
}

#[derive(Clone, Debug)]
struct ResolvedLayerFile {
    real_rel: String,
    path: PathBuf,
    layer_index: usize,
}

impl GameFileResolver {
    pub fn vanilla(vanilla_root: impl Into<PathBuf>) -> Result<Self> {
        Self::from_options(vanilla_root, GameFileResolverOptions::default())
    }

    pub fn with_mod_paths(
        vanilla_root: impl Into<PathBuf>,
        mod_paths: Vec<PathBuf>,
    ) -> Result<Self> {
        Self::from_options(
            vanilla_root,
            GameFileResolverOptions {
                mod_paths,
                use_dlc_load: false,
                userdata: None,
            },
        )
    }

    pub fn from_dlc_load(vanilla_root: impl Into<PathBuf>, userdata: PathBuf) -> Result<Self> {
        Self::from_options(
            vanilla_root,
            GameFileResolverOptions {
                mod_paths: Vec::new(),
                use_dlc_load: true,
                userdata: Some(userdata),
            },
        )
    }

    pub fn from_options(
        vanilla_root: impl Into<PathBuf>,
        options: GameFileResolverOptions,
    ) -> Result<Self> {
        if options.use_dlc_load && !options.mod_paths.is_empty() {
            return Err(
                "pass either repeated --mod content dirs or --use-dlc-load, not both".into(),
            );
        }
        if options.use_dlc_load && options.userdata.is_none() {
            return Err("pass --userdata <the game user-data dir> with --use-dlc-load".into());
        }
        let setup = BundleResolverSetup::from_options(
            vanilla_root.into(),
            &BundleModSelectionOptions {
                mods: options.mod_paths,
                use_dlc_load: options.use_dlc_load,
                userdata: options.userdata,
            },
        )?;
        Ok(Self {
            resolver: setup.resolver,
            selection_source: setup.selection_source,
            explicit_modset: setup.explicit_modset,
            warnings: setup.warnings,
        })
    }

    pub fn game_dir(&self) -> &Path {
        self.resolver
            .layers
            .first()
            .map(|layer| layer.root.as_path())
            .unwrap_or_else(|| Path::new(""))
    }

    pub fn selection_source(&self) -> &str {
        &self.selection_source
    }

    pub fn has_explicit_modset(&self) -> bool {
        self.explicit_modset
    }

    pub fn resolve(&self, rel_path: &str) -> Result<Option<PathBuf>> {
        self.resolver.resolve_optional_file(rel_path)
    }

    pub fn resolve_required(&self, rel_path: &str, label: &str) -> Result<PathBuf> {
        self.resolver.resolve_file(rel_path, label)
    }

    pub fn resolve_dir_files(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<(String, PathBuf)>> {
        self.resolver.resolve_dir_files(rel_dir, extension)
    }

    /// Recursive variant of [`Self::resolve_dir_files`]: enumerates files under
    /// `rel_dir` and all subdirectories across every content layer with the
    /// same override semantics (later mods win by relative path,
    /// `replace_path` removes lower-layer files in the exact directory only).
    pub fn resolve_dir_files_recursive(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<(String, PathBuf)>> {
        self.resolver
            .resolve_dir_files_recursive(rel_dir, extension)
    }

    pub fn stats(&self) -> GameFileResolverStats {
        let stats = self.resolver.stats();
        GameFileResolverStats {
            overridden_file_count: stats.overridden_file_count,
            replace_path_excluded_file_count: stats.replace_path_excluded_file_count,
        }
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn resolves_to_mod_layer(&self, rel_path: &str) -> Result<bool> {
        Ok(self
            .resolver
            .resolve_optional_file_with_layer(rel_path)?
            .map(|resolved| resolved.layer_index > 0)
            .unwrap_or(false))
    }
}

pub fn sensitive_file_overrides(
    resolver: &GameFileResolver,
    rel_paths: &[&str],
) -> Result<SensitiveFileOverrideReport> {
    let mut overridden = Vec::new();
    for rel_path in rel_paths {
        if resolver.resolves_to_mod_layer(rel_path)? {
            overridden.push(normalize_rel_path(rel_path));
        }
    }
    overridden.sort();
    overridden.dedup();
    Ok(SensitiveFileOverrideReport { overridden })
}

impl FileResolver {
    pub(crate) fn vanilla(vanilla_root: PathBuf) -> Self {
        Self {
            layers: vec![ResolverLayer {
                root: vanilla_root,
                manifest: None,
                replace_paths: BTreeSet::new(),
            }],
            stats: RefCell::new(ResolverStats::default()),
        }
    }

    pub(crate) fn with_mod_layers(vanilla_root: PathBuf, mods: Vec<ModContentLayer>) -> Self {
        let mut layers = vec![ResolverLayer {
            root: vanilla_root,
            manifest: None,
            replace_paths: BTreeSet::new(),
        }];
        for layer in mods {
            let replace_paths = layer
                .manifest
                .replace_path
                .iter()
                .map(|path| normalize_rel_dir(path).to_ascii_lowercase())
                .filter(|path| !path.is_empty())
                .collect();
            layers.push(ResolverLayer {
                root: layer.root,
                manifest: Some(layer.manifest),
                replace_paths,
            });
        }
        Self {
            layers,
            stats: RefCell::new(ResolverStats::default()),
        }
    }

    pub(crate) fn resolve_file(&self, rel: &str, label: &str) -> Result<PathBuf> {
        self.resolve_optional_file(rel)?
            .ok_or_else(|| format!("missing {label}: {rel}").into())
    }

    pub(crate) fn resolve_optional_file(&self, rel: &str) -> Result<Option<PathBuf>> {
        Ok(self
            .resolve_optional_file_with_layer(rel)?
            .map(|resolved| resolved.path))
    }

    fn resolve_optional_file_with_layer(&self, rel: &str) -> Result<Option<ResolvedLayerFile>> {
        let rel = normalize_rel_path(rel);
        let folded = rel.to_ascii_lowercase();
        let parent = immediate_parent(&folded);
        let mut candidate: Option<ResolvedLayerFile> = None;

        for (idx, layer) in self.layers.iter().enumerate() {
            if idx > 0 && layer.replace_paths.contains(&parent) {
                if let Some(replaced) = candidate.take() {
                    self.stats
                        .borrow_mut()
                        .replace_path_excluded_files
                        .insert(replaced.real_rel);
                }
            }
            if let Some((real_rel, path)) = find_file_in_layer(&layer.root, &rel)? {
                if idx > 0 && candidate.is_some() {
                    self.stats
                        .borrow_mut()
                        .overridden_files
                        .insert(real_rel.clone());
                }
                candidate = Some(ResolvedLayerFile {
                    real_rel,
                    path,
                    layer_index: idx,
                });
            }
        }
        Ok(candidate)
    }

    pub(crate) fn resolve_dir_files(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<(String, PathBuf)>> {
        let rel_dir = normalize_rel_dir(rel_dir);
        let mut effective: BTreeMap<String, (String, PathBuf, usize)> = BTreeMap::new();
        let mut any_dir = false;
        let wanted_extension =
            extension.map(|ext| ext.trim_start_matches('.').to_ascii_lowercase());

        for (idx, layer) in self.layers.iter().enumerate() {
            if idx > 0 && !layer.replace_paths.is_empty() {
                for folded in effective.keys().cloned().collect::<Vec<_>>() {
                    if layer.replace_paths.contains(&immediate_parent(&folded)) {
                        if let Some((real, _, _)) = effective.remove(&folded) {
                            self.stats
                                .borrow_mut()
                                .replace_path_excluded_files
                                .insert(real);
                        }
                    }
                }
            }

            let Some(dir) = find_dir_in_layer(&layer.root, &rel_dir)? else {
                continue;
            };
            any_dir = true;
            for entry in fs::read_dir(&dir)? {
                let path = entry?.path();
                if !path.is_file() {
                    continue;
                }
                if let Some(ext) = &wanted_extension {
                    let actual = path
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(|value| value.to_ascii_lowercase());
                    if actual.as_deref() != Some(ext.as_str()) {
                        continue;
                    }
                }
                let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                    continue;
                };
                let real_rel = if rel_dir.is_empty() {
                    file_name.to_string()
                } else {
                    format!("{rel_dir}/{file_name}")
                };
                let folded = real_rel.to_ascii_lowercase();
                if idx > 0 && effective.contains_key(&folded) {
                    self.stats
                        .borrow_mut()
                        .overridden_files
                        .insert(real_rel.clone());
                }
                effective.insert(folded, (real_rel, path, idx));
            }
        }

        if !any_dir {
            return Err(format!("missing directory: {rel_dir}").into());
        }

        let mut out = effective
            .into_values()
            .map(|(rel, path, _)| (rel, path))
            .collect::<Vec<_>>();
        out.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(out)
    }

    pub(crate) fn resolve_dir_files_recursive(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<(String, PathBuf)>> {
        let rel_dir = normalize_rel_dir(rel_dir);
        let mut effective: BTreeMap<String, (String, PathBuf, usize)> = BTreeMap::new();
        let mut any_dir = false;
        let wanted_extension =
            extension.map(|ext| ext.trim_start_matches('.').to_ascii_lowercase());

        for (idx, layer) in self.layers.iter().enumerate() {
            if idx > 0 && !layer.replace_paths.is_empty() {
                for folded in effective.keys().cloned().collect::<Vec<_>>() {
                    if layer.replace_paths.contains(&immediate_parent(&folded)) {
                        if let Some((real, _, _)) = effective.remove(&folded) {
                            self.stats
                                .borrow_mut()
                                .replace_path_excluded_files
                                .insert(real);
                        }
                    }
                }
            }

            let Some(dir) = find_dir_in_layer(&layer.root, &rel_dir)? else {
                continue;
            };
            any_dir = true;
            let mut layer_files = Vec::new();
            collect_layer_files_recursive(
                &dir,
                &rel_dir,
                wanted_extension.as_deref(),
                &mut layer_files,
            )?;
            for (real_rel, path) in layer_files {
                let folded = real_rel.to_ascii_lowercase();
                if idx > 0 && effective.contains_key(&folded) {
                    self.stats
                        .borrow_mut()
                        .overridden_files
                        .insert(real_rel.clone());
                }
                effective.insert(folded, (real_rel, path, idx));
            }
        }

        if !any_dir {
            return Err(format!("missing directory: {rel_dir}").into());
        }

        let mut out = effective
            .into_values()
            .map(|(rel, path, _)| (rel, path))
            .collect::<Vec<_>>();
        out.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(out)
    }

    pub(crate) fn stats(&self) -> FileResolverStats {
        let stats = self.stats.borrow();
        FileResolverStats {
            overridden_file_count: stats.overridden_files.len(),
            replace_path_excluded_file_count: stats.replace_path_excluded_files.len(),
        }
    }

    pub(crate) fn mods(&self) -> Vec<BundleMod> {
        self.layers
            .iter()
            .filter_map(|layer| layer.manifest.clone())
            .collect()
    }
}

fn mods_from_dlc_load(userdata: &Path, warnings: &mut Vec<String>) -> Result<Vec<ModContentLayer>> {
    let dlc_load = userdata.join("dlc_load.json");
    let text = match fs::read_to_string(&dlc_load) {
        Ok(text) => text,
        Err(err) => {
            warnings.push(format!("could not read dlc_load.json: {err}"));
            return Ok(Vec::new());
        }
    };
    let value = match serde_json::from_str::<Value>(&text) {
        Ok(value) => value,
        Err(err) => {
            warnings.push(format!("could not parse dlc_load.json: {err}"));
            return Ok(Vec::new());
        }
    };
    let Some(enabled_mods) = value.get("enabled_mods").and_then(|value| value.as_array()) else {
        warnings.push("dlc_load.json has no enabled_mods array".to_string());
        return Ok(Vec::new());
    };

    let mut mods = Vec::new();
    for item in enabled_mods {
        let Some(rel) = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let descriptor = path_from_slash_rel(userdata, rel);
        match mod_layer_from_descriptor(mods.len(), &descriptor, userdata) {
            Ok(Some(layer)) => mods.push(layer),
            Ok(None) => {}
            Err(err) => warnings.push(err.to_string()),
        }
    }
    Ok(mods)
}

fn explicit_mod_layer(
    load_order: usize,
    root: &Path,
    warnings: &mut Vec<String>,
) -> Option<ModContentLayer> {
    let root = match require_dir(root, "mod content directory") {
        Ok(root) => root,
        Err(_) => {
            warnings.push(format!(
                "mod content directory not found: {}",
                path_basename(root)
            ));
            return None;
        }
    };
    let descriptor = root.join("descriptor.mod");
    let parsed = descriptor
        .is_file()
        .then(|| parse_mod_descriptor(&descriptor))
        .transpose();
    match parsed {
        Ok(Some(descriptor)) => Some(ModContentLayer {
            root,
            manifest: descriptor.to_bundle_mod(load_order),
        }),
        Ok(None) => Some(ModContentLayer {
            manifest: BundleMod {
                load_order,
                name: root
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.to_string()),
                version: None,
                supported_version: None,
                workshop_id: None,
                descriptor: None,
                replace_path: Vec::new(),
            },
            root,
        }),
        Err(err) => {
            warnings.push(err.to_string());
            None
        }
    }
}

fn path_basename(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<unknown>".to_string())
}

fn mod_layer_from_descriptor(
    load_order: usize,
    descriptor_path: &Path,
    userdata: &Path,
) -> Result<Option<ModContentLayer>> {
    if !descriptor_path.is_file() {
        return Err(format!(
            "mod descriptor not found: {}",
            descriptor_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("<unknown>")
        )
        .into());
    }
    let descriptor = parse_mod_descriptor(descriptor_path)?;
    let Some(path_value) = descriptor.path.as_ref() else {
        if descriptor.archive.is_some() {
            return Err(format!(
                "archive-only mod skipped: {}",
                descriptor
                    .name
                    .clone()
                    .or_else(|| descriptor.descriptor.clone())
                    .unwrap_or_else(|| "<unknown>".to_string())
            )
            .into());
        }
        return Err(format!(
            "mod path missing in descriptor: {}",
            descriptor
                .descriptor
                .clone()
                .unwrap_or_else(|| "<unknown>".to_string())
        )
        .into());
    };
    let root = resolve_descriptor_content_root(path_value, userdata);
    if !root.is_dir() {
        return Err(format!(
            "mod path unresolvable, skipped: {}",
            descriptor.name.as_deref().unwrap_or("<unnamed>")
        )
        .into());
    }
    Ok(Some(ModContentLayer {
        root,
        manifest: descriptor.to_bundle_mod(load_order),
    }))
}

#[derive(Clone, Debug, Default)]
struct ParsedModDescriptor {
    name: Option<String>,
    path: Option<String>,
    archive: Option<String>,
    version: Option<String>,
    supported_version: Option<String>,
    remote_file_id: Option<String>,
    replace_path: Vec<String>,
    descriptor: Option<String>,
}

impl ParsedModDescriptor {
    fn to_bundle_mod(&self, load_order: usize) -> BundleMod {
        BundleMod {
            load_order,
            name: self.name.clone(),
            version: self.version.clone(),
            supported_version: self.supported_version.clone(),
            workshop_id: self.remote_file_id.clone(),
            descriptor: self.descriptor.clone(),
            replace_path: self.replace_path.clone(),
        }
    }
}

fn parse_mod_descriptor(path: &Path) -> Result<ParsedModDescriptor> {
    let text = read_latin1(path)?;
    let mut descriptor = parse_mod_descriptor_text(&text);
    descriptor.descriptor = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string());
    Ok(descriptor)
}

fn parse_mod_descriptor_text(text: &str) -> ParsedModDescriptor {
    let mut out = ParsedModDescriptor::default();
    for raw_line in text.lines() {
        let line = raw_line
            .split_once('#')
            .map(|(head, _)| head)
            .unwrap_or(raw_line)
            .trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = parse_descriptor_scalar(value.trim()) else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "name" => out.name = Some(value),
            "path" => out.path = Some(value),
            "archive" => out.archive = Some(value),
            "version" => out.version = Some(value),
            "supported_version" => out.supported_version = Some(value),
            "remote_file_id" => out.remote_file_id = Some(value),
            "replace_path" => {
                let normalized = normalize_rel_dir(&value);
                if !normalized.is_empty() {
                    out.replace_path.push(normalized);
                }
            }
            _ => {}
        }
    }
    out
}

fn parse_descriptor_scalar(value: &str) -> Option<String> {
    if value.starts_with('{') {
        return None;
    }
    if let Some(rest) = value.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    value
        .split_whitespace()
        .next()
        .map(|value| value.trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_descriptor_content_root(value: &str, userdata: &Path) -> PathBuf {
    let path = PathBuf::from(value.replace('\\', "/"));
    if path.is_absolute() {
        path
    } else {
        path_from_slash_rel(userdata, value)
    }
}

fn path_from_slash_rel(root: &Path, rel: &str) -> PathBuf {
    normalize_rel_path(rel)
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

pub(crate) fn modset_id(mods: &[BundleMod]) -> String {
    if mods.is_empty() {
        return "vanilla".to_string();
    }
    let raw = canonical_modset_json(mods);
    sha1_hex_16(raw.as_bytes())
}

fn canonical_modset_json(mods: &[BundleMod]) -> String {
    let mut out = String::from("[");
    for (idx, item) in mods.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let workshop = item.workshop_id.as_deref().unwrap_or_default().trim();
        let ident = if workshop.is_empty() {
            format!("name:{}", item.name.as_deref().unwrap_or_default().trim())
        } else {
            format!("workshop:{workshop}")
        };
        let version = item.version.as_deref().unwrap_or_default().trim();
        out.push('[');
        out.push_str(&json_string_ascii(&ident));
        out.push(',');
        out.push_str(&json_string_ascii(version));
        out.push(']');
    }
    out.push(']');
    out
}

fn json_string_ascii(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch < ' ' => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch if ch.is_ascii() => out.push(ch),
            ch => {
                let code = ch as u32;
                if code <= 0xffff {
                    out.push_str(&format!("\\u{code:04x}"));
                } else {
                    let value = code - 0x1_0000;
                    let high = 0xd800 + (value >> 10);
                    let low = 0xdc00 + (value & 0x3ff);
                    out.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
}

fn sha1_hex_16(input: &[u8]) -> String {
    let mut h0 = 0x6745_2301u32;
    let mut h1 = 0xefcd_ab89u32;
    let mut h2 = 0x98ba_dcfeu32;
    let mut h3 = 0x1032_5476u32;
    let mut h4 = 0xc3d2_e1f0u32;

    let bit_len = (input.len() as u64) * 8;
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (idx, word) in w.iter_mut().take(16).enumerate() {
            let start = idx * 4;
            *word = u32::from_be_bytes([
                chunk[start],
                chunk[start + 1],
                chunk[start + 2],
                chunk[start + 3],
            ]);
        }
        for idx in 16..80 {
            w[idx] = (w[idx - 3] ^ w[idx - 8] ^ w[idx - 14] ^ w[idx - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;
        for (idx, word) in w.iter().enumerate() {
            let (f, k) = match idx {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    format!("{h0:08x}{h1:08x}{h2:08x}{h3:08x}{h4:08x}")
        .chars()
        .take(16)
        .collect()
}

fn find_file_in_layer(root: &Path, rel: &str) -> Result<Option<(String, PathBuf)>> {
    let exact = path_from_slash_rel(root, rel);
    if exact.is_file() {
        return Ok(Some((rel.to_string(), exact)));
    }

    let Some((parent_rel, file_name)) = split_parent_file(rel) else {
        return Ok(None);
    };
    let Some(parent) = find_dir_in_layer(root, parent_rel)? else {
        return Ok(None);
    };
    for entry in fs::read_dir(parent)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(actual) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if actual.eq_ignore_ascii_case(file_name) {
            let real_rel = if parent_rel.is_empty() {
                actual.to_string()
            } else {
                format!("{parent_rel}/{actual}")
            };
            return Ok(Some((real_rel, path)));
        }
    }
    Ok(None)
}

fn find_dir_in_layer(root: &Path, rel_dir: &str) -> Result<Option<PathBuf>> {
    let exact = path_from_slash_rel(root, rel_dir);
    if exact.is_dir() {
        return Ok(Some(exact));
    }
    let mut current = root.to_path_buf();
    for part in normalize_rel_dir(rel_dir)
        .split('/')
        .filter(|part| !part.is_empty())
    {
        let mut next = None;
        if !current.is_dir() {
            return Ok(None);
        }
        for entry in fs::read_dir(&current)? {
            let path = entry?.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name.eq_ignore_ascii_case(part) {
                next = Some(path);
                break;
            }
        }
        let Some(path) = next else {
            return Ok(None);
        };
        current = path;
    }
    Ok(Some(current))
}

fn split_parent_file(rel: &str) -> Option<(&str, &str)> {
    let idx = rel.rfind('/')?;
    Some((&rel[..idx], &rel[idx + 1..]))
}

fn collect_layer_files_recursive(
    dir: &Path,
    rel_prefix: &str,
    wanted_extension: Option<&str>,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let child_rel = if rel_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{rel_prefix}/{name}")
        };
        if path.is_dir() {
            collect_layer_files_recursive(&path, &child_rel, wanted_extension, out)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        if let Some(ext) = wanted_extension {
            let actual = path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase());
            if actual.as_deref() != Some(ext) {
                continue;
            }
        }
        out.push((child_rel, path));
    }
    Ok(())
}

fn immediate_parent(rel: &str) -> String {
    rel.rfind('/')
        .map(|idx| rel[..idx].to_string())
        .unwrap_or_default()
}

pub(crate) fn normalize_rel_dir(value: &str) -> String {
    normalize_rel_path(value)
}

pub(crate) fn normalize_rel_path(value: &str) -> String {
    let mut value = value
        .replace('\\', "/")
        .trim()
        .trim_matches('"')
        .to_string();
    while value.contains("//") {
        value = value.replace("//", "/");
    }
    value
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn write_text(root: &Path, rel: &str, text: &str) {
        let path = path_from_slash_rel(root, rel);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, text).expect("write fixture file");
    }

    fn read_resolved(resolver: &FileResolver, rel: &str) -> String {
        let path = resolver
            .resolve_file(rel, rel)
            .expect("resolve fixture file");
        read_latin1(&path).expect("read fixture file")
    }

    #[test]
    fn resolver_file_override_uses_mod_over_vanilla() {
        let root = temp_test_dir("resolver_file_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        write_text(&mod_root, "map/definition.csv", "modded");

        let resolver = FileResolver::with_mod_layers(
            vanilla.clone(),
            vec![ModContentLayer {
                root: mod_root,
                manifest: BundleMod {
                    load_order: 0,
                    name: Some("Override".to_string()),
                    version: None,
                    supported_version: None,
                    workshop_id: None,
                    descriptor: None,
                    replace_path: Vec::new(),
                },
            }],
        );

        assert_eq!(
            read_resolved(&resolver, "map/definition.csv"),
            "modded".to_string()
        );
        assert_eq!(resolver.stats().overridden_file_count, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolver_directory_merge_and_replace_path_are_non_recursive() {
        let root = temp_test_dir("resolver_replace_path");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, "history/states/1-a.txt", "vanilla-a");
        write_text(&vanilla, "history/states/2-b.txt", "vanilla-b");
        write_text(
            &vanilla,
            "history/states/sub/10-nested.txt",
            "vanilla-nested",
        );
        write_text(&mod_root, "history/states/2-b.txt", "mod-b");
        write_text(&mod_root, "history/states/3-c.txt", "mod-c");

        let resolver = FileResolver::with_mod_layers(
            vanilla,
            vec![ModContentLayer {
                root: mod_root,
                manifest: BundleMod {
                    load_order: 0,
                    name: Some("ReplaceStates".to_string()),
                    version: None,
                    supported_version: None,
                    workshop_id: None,
                    descriptor: None,
                    replace_path: vec!["history/states".to_string()],
                },
            }],
        );

        let files = resolver
            .resolve_dir_files("history/states", Some("txt"))
            .expect("resolve state dir");
        assert_eq!(
            files
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec!["history/states/2-b.txt", "history/states/3-c.txt"]
        );
        assert_eq!(read_latin1(&files[0].1).unwrap(), "mod-b");

        let nested = resolver
            .resolve_dir_files("history/states/sub", Some("txt"))
            .expect("resolve nested state dir");
        assert_eq!(
            nested
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec!["history/states/sub/10-nested.txt"]
        );
        assert_eq!(read_latin1(&nested[0].1).unwrap(), "vanilla-nested");
        assert_eq!(resolver.stats().replace_path_excluded_file_count, 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolver_recursive_directory_merge_overrides_and_replaces_exact_dir_only() {
        let root = temp_test_dir("resolver_recursive_dir");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, "gfx/models/a.mesh", "vanilla-a");
        write_text(&vanilla, "gfx/models/buildings/b.mesh", "vanilla-b");
        write_text(&vanilla, "gfx/models/buildings/deep/c.mesh", "vanilla-c");
        write_text(&vanilla, "gfx/models/readme.txt", "not-a-mesh");
        write_text(&mod_root, "gfx/models/buildings/b.mesh", "mod-b");
        write_text(&mod_root, "gfx/models/new/d.mesh", "mod-d");

        let resolver = FileResolver::with_mod_layers(
            vanilla.clone(),
            vec![ModContentLayer {
                root: mod_root.clone(),
                manifest: BundleMod {
                    load_order: 0,
                    name: Some("Models".to_string()),
                    version: None,
                    supported_version: None,
                    workshop_id: None,
                    descriptor: None,
                    replace_path: Vec::new(),
                },
            }],
        );

        let files = resolver
            .resolve_dir_files_recursive("gfx/models", Some("mesh"))
            .expect("recursive model listing");
        assert_eq!(
            files
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec![
                "gfx/models/a.mesh",
                "gfx/models/buildings/b.mesh",
                "gfx/models/buildings/deep/c.mesh",
                "gfx/models/new/d.mesh",
            ]
        );
        let modded = files
            .iter()
            .find(|(rel, _)| rel == "gfx/models/buildings/b.mesh")
            .expect("overridden entry");
        assert_eq!(read_latin1(&modded.1).unwrap(), "mod-b");
        assert_eq!(resolver.stats().overridden_file_count, 1);

        let replace_resolver = FileResolver::with_mod_layers(
            vanilla,
            vec![ModContentLayer {
                root: mod_root,
                manifest: BundleMod {
                    load_order: 0,
                    name: Some("ReplaceBuildings".to_string()),
                    version: None,
                    supported_version: None,
                    workshop_id: None,
                    descriptor: None,
                    replace_path: vec!["gfx/models/buildings".to_string()],
                },
            }],
        );
        let replaced = replace_resolver
            .resolve_dir_files_recursive("gfx/models", Some("mesh"))
            .expect("recursive model listing with replace_path");
        assert_eq!(
            replaced
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec![
                "gfx/models/a.mesh",
                "gfx/models/buildings/b.mesh",
                "gfx/models/buildings/deep/c.mesh",
                "gfx/models/new/d.mesh",
            ],
            "replace_path removes the exact dir from lower layers, keeps subdirs, and the mod re-adds its own b.mesh"
        );
        let replaced_b = replaced
            .iter()
            .find(|(rel, _)| rel == "gfx/models/buildings/b.mesh")
            .expect("replaced entry");
        assert_eq!(read_latin1(&replaced_b.1).unwrap(), "mod-b");
        assert_eq!(
            replace_resolver.stats().replace_path_excluded_file_count,
            1,
            "vanilla gfx/models/buildings/b.mesh is excluded by replace_path before the mod layer re-adds it"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolver_load_order_later_mod_wins() {
        let root = temp_test_dir("resolver_load_order");
        let vanilla = root.join("vanilla");
        let mod_low = root.join("mod_low");
        let mod_high = root.join("mod_high");
        write_text(&vanilla, "common/countries/colors.txt", "vanilla");
        write_text(&mod_low, "common/countries/colors.txt", "low");
        write_text(&mod_high, "common/countries/colors.txt", "high");

        let layer = |root: PathBuf, load_order, name: &str| ModContentLayer {
            root,
            manifest: BundleMod {
                load_order,
                name: Some(name.to_string()),
                version: None,
                supported_version: None,
                workshop_id: None,
                descriptor: None,
                replace_path: Vec::new(),
            },
        };
        let resolver = FileResolver::with_mod_layers(
            vanilla,
            vec![
                layer(mod_low, 0, "LowPriority"),
                layer(mod_high, 1, "HighPriority"),
            ],
        );

        assert_eq!(
            read_resolved(&resolver, "common/countries/colors.txt"),
            "high".to_string()
        );
        assert_eq!(resolver.stats().overridden_file_count, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolver_vanilla_only_returns_vanilla_paths_and_file_lists() {
        let root = temp_test_dir("resolver_vanilla_only");
        let vanilla = root.join("vanilla");
        write_text(&vanilla, "history/states/1-a.txt", "a");
        write_text(&vanilla, "history/states/2-b.txt", "b");
        let resolver = FileResolver::vanilla(vanilla.clone());

        let path = resolver
            .resolve_file("history/states/1-a.txt", "state")
            .expect("resolve vanilla state");
        assert_eq!(path, vanilla.join("history").join("states").join("1-a.txt"));
        let files = resolver
            .resolve_dir_files("history/states", Some("txt"))
            .expect("resolve vanilla state dir");
        assert_eq!(
            files
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec!["history/states/1-a.txt", "history/states/2-b.txt"]
        );
        assert_eq!(resolver.stats().overridden_file_count, 0);
        assert_eq!(resolver.stats().replace_path_excluded_file_count, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn public_game_file_resolver_merges_dir_files_with_override_dedup() {
        let root = temp_test_dir("game_file_resolver_dir_merge");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, "common/test/a.txt", "vanilla-a");
        write_text(&vanilla, "common/test/b.txt", "vanilla-b");
        write_text(&mod_root, "common/test/b.txt", "mod-b");
        write_text(&mod_root, "common/test/c.txt", "mod-c");

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let files = resolver
            .resolve_dir_files("common/test", Some("txt"))
            .expect("resolve merged dir");

        assert_eq!(
            files
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec![
                "common/test/a.txt",
                "common/test/b.txt",
                "common/test/c.txt"
            ]
        );
        assert_eq!(read_latin1(&files[1].1).unwrap(), "mod-b");
        assert_eq!(resolver.stats().overridden_file_count, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sensitive_file_override_report_lists_modded_sensitive_inputs() {
        let root = temp_test_dir("game_file_resolver_sensitive");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, "gfx/FX/constants.fxh", "vanilla constants");
        write_text(&vanilla, "gfx/FX/pdxmap.shader", "vanilla shader");
        write_text(&vanilla, "map/seasons.txt", "vanilla seasons");
        write_text(&mod_root, "gfx/FX/constants.fxh", "mod constants");
        write_text(&mod_root, "map/seasons.txt", "mod seasons");

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let report = sensitive_file_overrides(
            &resolver,
            &[
                "gfx/FX/constants.fxh",
                "gfx/FX/pdxmap.shader",
                "map/seasons.txt",
            ],
        )
        .expect("override report");

        assert_eq!(
            report.overridden,
            vec![
                "gfx/FX/constants.fxh".to_string(),
                "map/seasons.txt".to_string()
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn modset_digest_matches_python_schema_fixture() {
        let mods = vec![
            BundleMod {
                load_order: 0,
                name: Some("Ignored When Workshop Exists".to_string()),
                version: Some("1.2.3".to_string()),
                supported_version: None,
                workshop_id: Some("12345".to_string()),
                descriptor: Some("workshop.mod".to_string()),
                replace_path: Vec::new(),
            },
            BundleMod {
                load_order: 1,
                name: Some("Local Test".to_string()),
                version: Some("2026.7".to_string()),
                supported_version: None,
                workshop_id: None,
                descriptor: Some("local.mod".to_string()),
                replace_path: Vec::new(),
            },
        ];
        // Python schema.modset_id input:
        // [["workshop:12345","1.2.3"],["name:Local Test","2026.7"]]
        assert_eq!(
            canonical_modset_json(&mods),
            "[[\"workshop:12345\",\"1.2.3\"],[\"name:Local Test\",\"2026.7\"]]"
        );
        assert_eq!(modset_id(&mods), "35bc3f7d45155d2b");
        assert_eq!(modset_id(&[]), "vanilla");
    }

    #[test]
    fn descriptor_parser_redacts_paths_and_keeps_replace_paths() {
        let parsed = parse_mod_descriptor_text(
            r#"
            name="AMSB"
            path="C:/Users/Example/Documents/Paradox Interactive/Hearts of Iron IV/mod/amsb"
            version="1.0"
            supported_version="1.16.*"
            remote_file_id="3470184538"
            replace_path="history/states"
            replace_path="common/countries"
            "#,
        );
        let manifest = parsed.to_bundle_mod(3);
        assert_eq!(manifest.load_order, 3);
        assert_eq!(manifest.name.as_deref(), Some("AMSB"));
        assert_eq!(manifest.version.as_deref(), Some("1.0"));
        assert_eq!(manifest.workshop_id.as_deref(), Some("3470184538"));
        assert_eq!(
            manifest.replace_path,
            vec!["history/states".to_string(), "common/countries".to_string()]
        );
        assert_eq!(manifest.descriptor, None);
    }

    #[test]
    fn dlc_load_resolution_uses_descriptor_order_and_redacted_manifest() {
        let root = temp_test_dir("resolver_dlc_load");
        let vanilla = root.join("vanilla");
        let userdata = root.join("userdata");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        write_text(&userdata, "mod/low/content/map/definition.csv", "low");
        write_text(&userdata, "mod/high/content/map/definition.csv", "high");
        write_text(
            &userdata,
            "mod/low.mod",
            r#"
            name="Low Mod"
            path="mod/low/content"
            version="1"
            replace_path="history/states"
            "#,
        );
        write_text(
            &userdata,
            "mod/high.mod",
            r#"
            name="High Mod"
            path="mod/high/content"
            version="2"
            remote_file_id="999"
            "#,
        );
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":["mod/low.mod","mod/high.mod"]}"#,
        );

        let setup = BundleResolverSetup::from_options(
            vanilla,
            &BundleModSelectionOptions {
                mods: Vec::new(),
                use_dlc_load: true,
                userdata: Some(userdata),
            },
        )
        .expect("resolve dlc_load mod set");

        assert_eq!(
            read_resolved(&setup.resolver, "map/definition.csv"),
            "high".to_string()
        );
        let modset = setup.manifest_modset().expect("explicit modset manifest");
        assert_eq!(modset.selection_source, "dlc_load.json");
        assert_eq!(modset.mods.len(), 2);
        assert_eq!(modset.mods[0].load_order, 0);
        assert_eq!(modset.mods[0].descriptor.as_deref(), Some("low.mod"));
        assert_eq!(modset.mods[0].replace_path, vec!["history/states"]);
        assert_eq!(modset.mods[1].workshop_id.as_deref(), Some("999"));
        assert!(!serde_json::to_string(&modset)
            .expect("serialize modset")
            .contains(&path_basename(&root)));
        let _ = fs::remove_dir_all(&root);
    }
}
