use crate::dds::{self, RgbaImage};
use crate::gfx::SpriteDefinition;
use crate::tga;
use crate::{ErrorKind, GameUiError, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetResolver {
    roots: Vec<PathBuf>,
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
        Self {
            roots: ordered_roots,
        }
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
        for root in &self.roots {
            if !root.is_dir() {
                continue;
            }
            if let Some(path) = find_case_insensitive(root, &normalized)? {
                return Ok(Some(ResolvedFile {
                    rel_path: normalized,
                    path,
                }));
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
        let mut effective: BTreeMap<String, ResolvedFile> = BTreeMap::new();
        for root in &self.roots {
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
                effective.entry(folded).or_insert(ResolvedFile {
                    rel_path: rel,
                    path,
                });
            }
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
