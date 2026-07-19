use crate::dds::{self, RgbaImage};
use crate::gfx::SpriteDefinition;
use crate::tga;
use crate::{ErrorKind, GameUiError, Result};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetResolver {
    roots: Vec<PathBuf>,
    replace_paths: Vec<BTreeSet<String>>,
    restricted_files: Option<BTreeMap<String, ResolvedFile>>,
    content_manifest_digest: Option<String>,
    admitted_content: Option<BTreeMap<String, replay_core::UsedContentKey>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedFile {
    pub rel_path: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedTexture {
    pub rel_path: String,
    pub path: PathBuf,
    pub format: String,
    pub image: RgbaImage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedSprite {
    pub sprite_name: String,
    pub texture: DecodedTexture,
    pub frames: Vec<RgbaImage>,
}

impl AssetResolver {
    pub fn new(ordered_roots: Vec<PathBuf>) -> Self {
        let replace_paths = vec![BTreeSet::new(); ordered_roots.len()];
        Self {
            roots: ordered_roots,
            replace_paths,
            restricted_files: None,
            content_manifest_digest: None,
            admitted_content: None,
        }
    }

    /// Adapt the central launcher-equivalent resolver to the path-based UI
    /// decoders while preserving the replay manifest's exact allowlist.
    pub fn from_restricted_game_files(
        resolver: &map_viewer::RestrictedGameFileResolver<'_>,
    ) -> Result<Self> {
        let mut restricted_files = BTreeMap::new();
        for row in resolver
            .resolve_dir_files_recursive("", None)
            .map_err(|error| GameUiError::missing(error.to_string()))?
        {
            let rel_path = normalize_rel_path(&row.logical_path);
            if restricted_files
                .insert(
                    rel_path.to_ascii_lowercase(),
                    ResolvedFile {
                        rel_path,
                        path: row.local_path,
                    },
                )
                .is_some()
            {
                return Err(GameUiError::parse(
                    "restricted UI content repeats a case-folded path",
                ));
            }
        }
        if restricted_files.is_empty() {
            return Err(GameUiError::missing(
                "replay authorizes no production UI content",
            ));
        }
        Ok(Self {
            roots: Vec::new(),
            replace_paths: Vec::new(),
            restricted_files: Some(restricted_files),
            content_manifest_digest: None,
            admitted_content: None,
        })
    }

    pub fn from_content_admission(
        resolver: &map_viewer::GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
    ) -> Result<Self> {
        if !resolver.has_content_admission() {
            return Err(GameUiError::missing(
                "production UI requires the central VFS content-admission capability",
            ));
        }
        let required = admission
            .required_paths()
            .map(|logical_path| {
                let content = admission
                    .require_used_content(logical_path)
                    .expect("required_paths only yields admitted content");
                map_viewer::RequiredGameContent {
                    logical_path: logical_path.to_string(),
                    sha256: content.payload_sha256.clone(),
                    byte_len: content.payload_bytes,
                }
            })
            .collect::<Vec<_>>();
        let restricted = resolver
            .restrict_to(required)
            .map_err(|error| GameUiError::missing(error.to_string()))?;
        let mut assets = Self::from_restricted_game_files(&restricted)?;
        assets.content_manifest_digest = Some(admission.manifest_digest_sha256().to_string());
        assets.admitted_content = Some(
            admission
                .required_paths()
                .map(|path| {
                    (
                        normalize_rel_path(path).to_ascii_lowercase(),
                        admission
                            .require_used_content(path)
                            .expect("required path is admitted")
                            .clone(),
                    )
                })
                .collect(),
        );
        Ok(assets)
    }

    /// Restrict UI decoding to an exact set of portable used-content rows.
    /// Every row must be the central admission's winner for the same logical
    /// path; installed-but-unrecorded content is never made reachable.
    pub fn from_used_content_rows(
        resolver: &map_viewer::GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        rows: impl IntoIterator<Item = replay_core::UsedContentKey>,
    ) -> Result<Self> {
        if !resolver.has_content_admission() {
            return Err(GameUiError::missing(
                "production UI requires the central VFS content-admission capability",
            ));
        }
        let mut selected = BTreeMap::new();
        for row in rows {
            let path = normalize_rel_path(&row.normalized_game_relative_path);
            let admitted = admission
                .require_used_content(&path)
                .map_err(GameUiError::missing)?;
            if admitted != &row {
                return Err(GameUiError::missing(format!(
                    "selected-Present UI content row disagrees with central admission for {path}"
                )));
            }
            if let Some(prior) = selected.insert(path.clone(), row.clone()) {
                if prior != row {
                    return Err(GameUiError::parse(format!(
                        "selected-Present UI content rows disagree on {path}"
                    )));
                }
            }
        }
        if selected.is_empty() {
            return Err(GameUiError::missing(
                "selected-Present UI pass authorizes no HUD content",
            ));
        }
        let required = selected
            .values()
            .map(|content| map_viewer::RequiredGameContent {
                logical_path: content.normalized_game_relative_path.clone(),
                sha256: content.payload_sha256.clone(),
                byte_len: content.payload_bytes,
            })
            .collect::<Vec<_>>();
        let restricted = resolver
            .restrict_to(required)
            .map_err(|error| GameUiError::missing(error.to_string()))?;
        let mut assets = Self::from_restricted_game_files(&restricted)?;
        assets.content_manifest_digest = Some(admission.manifest_digest_sha256().to_string());
        assets.admitted_content = Some(
            selected
                .into_iter()
                .map(|(path, content)| (path.to_ascii_lowercase(), content))
                .collect(),
        );
        Ok(assets)
    }

    /// Restrict UI decoding to the exact content rows recorded for a set of
    /// semantic-consumer prefixes. This is the product HUD adapter: non-UI
    /// content and unrelated dormant UI files remain inaccessible even when
    /// they are required elsewhere by the same replay.
    pub fn from_content_consumer_prefixes(
        resolver: &map_viewer::GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        consumer_prefixes: &[&str],
    ) -> Result<Self> {
        if !resolver.has_content_admission() {
            return Err(GameUiError::missing(
                "production UI requires the central VFS content-admission capability",
            ));
        }
        if consumer_prefixes.is_empty()
            || consumer_prefixes
                .iter()
                .any(|prefix| prefix.trim().is_empty())
        {
            return Err(GameUiError::parse(
                "production UI semantic-consumer prefixes are empty",
            ));
        }

        let mut selected = BTreeMap::new();
        for consumer in admission.required_consumers().filter(|consumer| {
            consumer_prefixes
                .iter()
                .any(|prefix| consumer.starts_with(prefix))
        }) {
            for content in admission
                .require_used_content_for_consumer(consumer)
                .map_err(GameUiError::missing)?
            {
                let path = content.normalized_game_relative_path.clone();
                if selected
                    .insert(path.clone(), content.clone())
                    .is_some_and(|prior| prior != *content)
                {
                    return Err(GameUiError::parse(format!(
                        "production UI semantic consumers disagree on {path}"
                    )));
                }
            }
        }
        if selected.is_empty() {
            return Err(GameUiError::missing(
                "replay authorizes no content for the requested production UI semantic consumers",
            ));
        }

        let required = selected
            .values()
            .cloned()
            .map(|content| map_viewer::RequiredGameContent {
                logical_path: content.normalized_game_relative_path,
                sha256: content.payload_sha256,
                byte_len: content.payload_bytes,
            })
            .collect::<Vec<_>>();
        let restricted = resolver
            .restrict_to(required)
            .map_err(|error| GameUiError::missing(error.to_string()))?;
        let mut assets = Self::from_restricted_game_files(&restricted)?;
        assets.content_manifest_digest = Some(admission.manifest_digest_sha256().to_string());
        assets.admitted_content = Some(
            selected
                .into_iter()
                .map(|(path, content)| (normalize_rel_path(&path).to_ascii_lowercase(), content))
                .collect(),
        );
        Ok(assets)
    }

    pub fn is_admitted_for(&self, admission: &replay_core::ValidatedContentAdmission) -> bool {
        self.content_manifest_digest.as_deref() == Some(admission.manifest_digest_sha256())
    }

    pub fn admitted_content_key(&self, rel_path: &str) -> Option<&replay_core::UsedContentKey> {
        self.admitted_content
            .as_ref()?
            .get(&normalize_rel_path(rel_path).to_ascii_lowercase())
    }

    pub fn from_runtime_userdir(game_root: &Path, userdata: &Path) -> Result<Self> {
        let staging_path = userdata.join("static_mod_staging_manifest.json");
        let receipt_path = userdata.join("ui14_engine_vfs.json");
        let staging: serde_json::Value = serde_json::from_slice(&fs::read(&staging_path)?)
            .map_err(|error| {
                GameUiError::parse(format!("invalid runtime staging manifest: {error}"))
            })?;
        let receipt: serde_json::Value = serde_json::from_slice(&fs::read(&receipt_path)?)
            .map_err(|error| {
                GameUiError::parse(format!("invalid UI14 engine VFS receipt: {error}"))
            })?;
        if receipt.get("schema").and_then(|v| v.as_str()) != Some("extractor.ui14_engine_vfs.v1") {
            return Err(GameUiError::parse(
                "unsupported UI14 engine VFS receipt schema",
            ));
        }
        let receipt_sha = Sha256::hex(
            &serde_json::to_vec(&receipt).map_err(|error| GameUiError::parse(error.to_string()))?,
        );
        if staging
            .get("ui14_engine_vfs_sha256")
            .and_then(|v| v.as_str())
            != Some(&receipt_sha)
        {
            return Err(GameUiError::parse(
                "runtime staging manifest does not bind UI14 receipt",
            ));
        }
        let mod_rows = staging
            .get("mods")
            .and_then(|v| v.as_array())
            .ok_or_else(|| GameUiError::parse("runtime staging manifest lacks mods"))?;
        let dlc_rows = staging
            .get("dlc_archives")
            .and_then(|v| v.as_array())
            .ok_or_else(|| GameUiError::parse("runtime staging manifest lacks dlc_archives"))?;
        let receipt_layers = receipt
            .get("layers")
            .and_then(|v| v.as_array())
            .ok_or_else(|| GameUiError::parse("UI14 receipt lacks layers"))?;
        let mut mods_by_id = BTreeMap::new();
        for row in mod_rows {
            let id = json_str(row, "id")?;
            if id.starts_with("_ui15_dlc/") || mods_by_id.insert(id.to_string(), row).is_some() {
                return Err(GameUiError::parse(
                    "runtime mod staging identity is invalid",
                ));
            }
        }
        let mut ordered_mods = Vec::new();
        let mut mount_ordinals = BTreeSet::new();
        for layer in receipt_layers {
            let id = json_str(layer, "id")?;
            let mount = layer
                .get("engine_mount_ordinal")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| GameUiError::parse("UI14 layer lacks engine mount ordinal"))?;
            if !mount_ordinals.insert(mount) {
                return Err(GameUiError::parse(
                    "UI14 receipt repeats an engine mount ordinal",
                ));
            }
            let row = mods_by_id
                .remove(id)
                .ok_or_else(|| GameUiError::parse(format!("UI14 layer not staged: {id}")))?;
            ordered_mods.push((mount, row));
        }
        if !mods_by_id.is_empty() {
            return Err(GameUiError::parse("staged mod is absent from UI14 receipt"));
        }
        ordered_mods.sort_by_key(|row| std::cmp::Reverse(row.0));
        let mut roots = Vec::new();
        let mut replace_paths = Vec::new();
        for (_, row) in ordered_mods {
            push_staged_layer(userdata, row, false, &mut roots, &mut replace_paths)?;
        }
        let mut ordered_dlc = Vec::new();
        let mut dlc_ordinals = BTreeSet::new();
        for row in dlc_rows {
            let ordinal = row
                .get("ordinal")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| GameUiError::parse("UI15 DLC layer lacks staged ordinal"))?;
            if !dlc_ordinals.insert(ordinal) {
                return Err(GameUiError::parse("UI15 DLC staging repeats an ordinal"));
            }
            ordered_dlc.push((ordinal, row));
        }
        ordered_dlc.sort_by_key(|row| std::cmp::Reverse(row.0));
        for (_, row) in ordered_dlc {
            push_staged_layer(userdata, row, true, &mut roots, &mut replace_paths)?;
        }
        if !game_root.is_dir() {
            return Err(GameUiError::missing("game root is missing"));
        }
        roots.push(game_root.to_path_buf());
        replace_paths.push(BTreeSet::new());
        Ok(Self {
            roots,
            replace_paths,
            restricted_files: None,
            content_manifest_digest: None,
            admitted_content: None,
        })
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn resolve_file(&self, rel: &str) -> Result<ResolvedFile> {
        self.resolve_optional_file(rel)?.ok_or_else(|| {
            GameUiError::missing(format!("missing asset: {}", normalize_rel_path(rel)))
        })
    }

    pub fn resolve_optional_file(&self, rel: &str) -> Result<Option<ResolvedFile>> {
        let normalized = normalize_rel_path(rel);
        if normalized.is_empty() {
            return Ok(None);
        }
        if let Some(files) = &self.restricted_files {
            return Ok(files.get(&normalized.to_ascii_lowercase()).cloned());
        }
        let parent = normalized
            .rsplit_once('/')
            .map(|row| row.0)
            .unwrap_or("")
            .to_ascii_lowercase();
        for (index, root) in self.roots.iter().enumerate() {
            if !root.is_dir() {
                continue;
            }
            if let Some(path) = find_case_insensitive(root, &normalized)? {
                return Ok(Some(ResolvedFile {
                    rel_path: normalized,
                    path,
                }));
            }
            if self.replace_paths[index].contains(&parent) {
                return Ok(None);
            }
        }
        Ok(None)
    }

    pub fn list_files_recursive(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<ResolvedFile>> {
        let rel_dir = normalize_rel_path(rel_dir);
        let wanted_ext = extension.map(|ext| ext.trim_start_matches('.').to_ascii_lowercase());
        if let Some(files) = &self.restricted_files {
            let prefix = if rel_dir.is_empty() {
                String::new()
            } else {
                format!("{}/", rel_dir.to_ascii_lowercase())
            };
            return Ok(files
                .iter()
                .filter(|(folded, row)| {
                    folded.starts_with(&prefix)
                        && wanted_ext.as_ref().is_none_or(|wanted| {
                            row.path
                                .extension()
                                .and_then(|value| value.to_str())
                                .is_some_and(|actual| actual.eq_ignore_ascii_case(wanted))
                        })
                })
                .map(|(_, row)| row.clone())
                .collect());
        }
        let mut effective: BTreeMap<String, ResolvedFile> = BTreeMap::new();
        let mut replaced: BTreeSet<String> = BTreeSet::new();
        for (index, root) in self.roots.iter().enumerate() {
            if !root.is_dir() {
                continue;
            }
            let Some(dir) = find_dir_case_insensitive(root, &rel_dir)? else {
                continue;
            };
            let mut files = Vec::new();
            collect_files(&dir, &mut files)?;
            for path in files {
                if let Some(ext) = &wanted_ext {
                    let actual = path
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(|value| value.to_ascii_lowercase());
                    if actual.as_deref() != Some(ext.as_str()) {
                        continue;
                    }
                }
                let Ok(suffix) = path.strip_prefix(&dir) else {
                    continue;
                };
                let rel = join_rel(&rel_dir, suffix);
                let folded = rel.to_ascii_lowercase();
                let parent = folded.rsplit_once('/').map(|row| row.0).unwrap_or("");
                if replaced.contains(parent) {
                    continue;
                }
                effective.entry(folded).or_insert(ResolvedFile {
                    rel_path: rel,
                    path,
                });
            }
            replaced.extend(self.replace_paths[index].iter().cloned());
        }
        Ok(effective.into_values().collect())
    }

    pub fn decode_texture(&self, rel: &str) -> Result<DecodedTexture> {
        let resolved = self.resolve_texture_file(rel)?;
        let ext = resolved
            .path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let (format, image) = match ext.as_str() {
            "dds" => {
                let decoded = dds::decode_file(&resolved.path)?;
                (decoded.format, decoded.image)
            }
            "tga" => {
                let decoded = tga::decode_file(&resolved.path)?;
                (decoded.format, decoded.image)
            }
            _ => {
                return Err(GameUiError::unsupported_texture_extension(
                    if ext.is_empty() {
                        "<none>"
                    } else {
                        ext.as_str()
                    },
                    format!(
                        "{} uses unsupported texture extension .{}",
                        resolved.rel_path,
                        if ext.is_empty() {
                            "<none>"
                        } else {
                            ext.as_str()
                        }
                    ),
                ));
            }
        };
        Ok(DecodedTexture {
            rel_path: resolved.rel_path,
            path: resolved.path,
            format,
            image,
        })
    }

    pub fn resolve_texture_file(&self, rel: &str) -> Result<ResolvedFile> {
        let normalized = normalize_rel_path(rel);
        if let Some(resolved) = self.resolve_optional_file(&normalized)? {
            return Ok(resolved);
        }
        let Some((stem, ext)) = normalized.rsplit_once('.') else {
            return Err(GameUiError::missing(format!(
                "missing texture asset: {normalized}"
            )));
        };
        let alternate = match ext.to_ascii_lowercase().as_str() {
            "tga" => Some(format!("{stem}.dds")),
            "dds" => Some(format!("{stem}.tga")),
            _ => None,
        };
        if let Some(alternate) = alternate {
            if let Some(resolved) = self.resolve_optional_file(&alternate)? {
                return Ok(resolved);
            }
        }
        Err(GameUiError::missing(format!(
            "missing texture asset: {normalized}"
        )))
    }

    pub fn decode_sprite(&self, sprite: &SpriteDefinition) -> Result<DecodedSprite> {
        let texture_path = sprite.primary_texture().ok_or_else(|| {
            GameUiError::new(
                ErrorKind::Missing,
                format!("sprite {} has no texturefile", sprite.name),
            )
        })?;
        let texture = self.decode_texture(texture_path)?;
        let frames = slice_horizontal_frames(&texture.image, sprite.no_of_frames)?;
        Ok(DecodedSprite {
            sprite_name: sprite.name.clone(),
            texture,
            frames,
        })
    }
}

fn json_str<'a>(row: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    row.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| GameUiError::parse(format!("runtime staging row lacks {key}")))
}

fn push_staged_layer(
    userdata: &Path,
    row: &serde_json::Value,
    ui15_dlc: bool,
    roots: &mut Vec<PathBuf>,
    replace_sets: &mut Vec<BTreeSet<String>>,
) -> Result<()> {
    let id = json_str(row, "id")?;
    if ui15_dlc != id.starts_with("_ui15_dlc/") {
        return Err(GameUiError::parse("UI15 DLC staging namespace is invalid"));
    }
    let descriptor_rel = json_str(row, "descriptor_relpath")?;
    if descriptor_rel != id {
        return Err(GameUiError::parse("staged descriptor identity changed"));
    }
    let descriptor =
        userdata.join(descriptor_rel.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
    if Sha256::hex(&fs::read(&descriptor)?) != json_str(row, "descriptor_sha256")? {
        return Err(GameUiError::parse(format!(
            "staged descriptor hash mismatch: {id}"
        )));
    }
    let content = PathBuf::from(json_str(row, "content_path")?);
    if !content.is_dir() {
        return Err(GameUiError::missing(format!(
            "staged content root missing: {id}"
        )));
    }
    let doc = crate::paradox_text::parse_file(&descriptor)?;
    let mut replacements = BTreeSet::new();
    for value in doc.root.values_ci("replace_path") {
        let raw = value
            .as_scalar()
            .ok_or_else(|| GameUiError::parse("replace_path must be scalar"))?;
        let normalized = normalize_rel_path(raw).to_ascii_lowercase();
        if normalized.is_empty() || normalized.split('/').any(|part| part == "..") {
            return Err(GameUiError::parse(
                "unsafe replace_path in staged descriptor",
            ));
        }
        replacements.insert(normalized);
    }
    roots.push(content);
    replace_sets.push(replacements);
    Ok(())
}

pub fn slice_horizontal_frames(image: &RgbaImage, frame_count: u32) -> Result<Vec<RgbaImage>> {
    let frame_count = frame_count.max(1) as usize;
    if frame_count == 1 {
        return Ok(vec![image.clone()]);
    }
    if image.width % frame_count != 0 {
        return Err(GameUiError::new(
            ErrorKind::InvalidTexture("frames".to_string()),
            format!(
                "cannot split {}x{} image into {frame_count} equal horizontal frames",
                image.width, image.height
            ),
        ));
    }
    let frame_width = image.width / frame_count;
    let mut frames = Vec::with_capacity(frame_count);
    for frame in 0..frame_count {
        let mut pixels = vec![0u8; frame_width * image.height * 4];
        for y in 0..image.height {
            let src = (y * image.width + frame * frame_width) * 4;
            let dst = y * frame_width * 4;
            pixels[dst..dst + frame_width * 4]
                .copy_from_slice(&image.pixels[src..src + frame_width * 4]);
        }
        frames.push(RgbaImage {
            width: frame_width,
            height: image.height,
            pixels,
        });
    }
    Ok(frames)
}

pub fn normalize_rel_path(value: &str) -> String {
    let mut out = value
        .replace('\\', "/")
        .trim()
        .trim_matches('"')
        .trim_start_matches("./")
        .to_string();
    while out.contains("//") {
        out = out.replace("//", "/");
    }
    out.trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn find_case_insensitive(root: &Path, rel: &str) -> Result<Option<PathBuf>> {
    let exact = rel
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part));
    if exact.is_file() {
        return Ok(Some(exact));
    }
    let Some((parent, file)) = rel.rsplit_once('/') else {
        return find_file_in_dir(root, rel);
    };
    let Some(dir) = find_dir_case_insensitive(root, parent)? else {
        return Ok(None);
    };
    find_file_in_dir(&dir, file)
}

fn find_dir_case_insensitive(root: &Path, rel: &str) -> Result<Option<PathBuf>> {
    let rel = normalize_rel_path(rel);
    if rel.is_empty() {
        return Ok(Some(root.to_path_buf()));
    }
    let exact = rel
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part));
    if exact.is_dir() {
        return Ok(Some(exact));
    }
    let mut current = root.to_path_buf();
    for part in rel.split('/').filter(|part| !part.is_empty()) {
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

fn find_file_in_dir(dir: &Path, file: &str) -> Result<Option<PathBuf>> {
    if !dir.is_dir() {
        return Ok(None);
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if name.eq_ignore_ascii_case(file) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn join_rel(rel_dir: &str, suffix: &Path) -> String {
    let suffix = suffix
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/");
    if rel_dir.is_empty() {
        normalize_rel_path(&suffix)
    } else {
        normalize_rel_path(&format!("{rel_dir}/{suffix}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolver_uses_first_root_and_ignores_case() {
        let root = temp_root("resolver_case");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        fs::create_dir_all(vanilla.join("GFX/Interface")).unwrap();
        fs::create_dir_all(mod_root.join("gfx/interface")).unwrap();
        fs::write(vanilla.join("GFX/Interface/icon.dds"), b"vanilla").unwrap();
        fs::write(mod_root.join("gfx/interface/ICON.dds"), b"mod").unwrap();
        let resolver = AssetResolver::new(vec![mod_root, vanilla]);
        let resolved = resolver.resolve_file("gfx\\interface\\icon.dds").unwrap();
        assert_eq!(fs::read(resolved.path).unwrap(), b"mod");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn frame_slicing_uses_horizontal_strips() {
        let image = RgbaImage {
            width: 4,
            height: 1,
            pixels: vec![1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255],
        };
        let frames = slice_horizontal_frames(&image, 2).unwrap();
        assert_eq!(frames[0].width, 2);
        assert_eq!(frames[0].pixels, vec![1, 0, 0, 255, 2, 0, 0, 255]);
        assert_eq!(frames[1].pixels, vec![3, 0, 0, 255, 4, 0, 0, 255]);
    }

    fn temp_root(label: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("game_ui_{label}_{nonce}"));
        path
    }
}
