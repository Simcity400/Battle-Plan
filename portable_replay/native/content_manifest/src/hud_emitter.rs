//! Neutral authenticated HUD dependency-closure emitter.
//!
//! The emitter understands pointer-free GUI/GFX/font/localisation contracts.
//! Physical launcher and archive access is supplied by a product-owned VFS
//! adapter, so neither the recorder nor the replayer depends on the other.

use crate::effective_vfs::EffectiveVfs;
use crate::gfx::SpriteRegistry;
use crate::gui::{parse_gui_document, GuiElement};
use crate::hud_role::{
    AuthenticatedHudSurfaceClosure, EffectiveVfsHudRoleIndexDocument, HudSemanticRole, HudSurface,
    HudSurfaceActivationFrame, ResolvedHudRoleBinding,
};
use crate::paradox_text::{decode_text, parse_bytes, Value};
use crate::{normalize_game_relative_path, UsedContentKey};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedHudFile {
    pub content: UsedContentKey,
    pub bytes: Vec<u8>,
}

/// Minimal exact VFS surface required by the neutral HUD interpreter.
pub trait HudContentVfs {
    fn list_recursive(&self, directory: &str, extension: &str) -> Result<Vec<String>, String>;
    fn resolve(&self, logical_path: &str) -> Result<Option<ResolvedHudFile>, String>;
}

impl HudContentVfs for EffectiveVfs {
    fn list_recursive(&self, directory: &str, extension: &str) -> Result<Vec<String>, String> {
        EffectiveVfs::list_recursive(self, directory, Some(extension))
            .map(|files| files.into_iter().map(|file| file.logical_path).collect())
    }

    fn resolve(&self, logical_path: &str) -> Result<Option<ResolvedHudFile>, String> {
        EffectiveVfs::resolve(self, logical_path).map(|resolved| {
            resolved.map(|file| ResolvedHudFile {
                content: file.content,
                bytes: file.bytes,
            })
        })
    }
}

pub fn emit_effective_vfs_hud_role_index(
    vfs: &impl HudContentVfs,
    activation: &HudSurfaceActivationFrame,
    source_inventory_sha256: &str,
    localisation_language: &str,
) -> Result<EffectiveVfsHudRoleIndexDocument, String> {
    if activation.status_flags != 0 || activation.active_surfaces.is_empty() {
        return Err("HUD index emitter requires a nonempty latest-clean d79 frame".into());
    }
    validate_sha(source_inventory_sha256)?;
    let gui_files = vfs.list_recursive("interface", "gui")?;
    let gfx_files = vfs.list_recursive("interface", "gfx")?;
    let localisation = localisation_sources(vfs, localisation_language)?;

    let mut roots = BTreeMap::<String, Vec<(String, GuiElement)>>::new();
    let mut gui_parse_failed = BTreeMap::new();
    for rel in gui_files {
        let Some(file) = vfs.resolve(&rel)? else {
            return Err(format!("HUD VFS listing could not resolve {rel}"));
        };
        validate_resolved(&file, &rel, source_inventory_sha256)?;
        let document = parse_bytes(&file.bytes);
        if !document.recoveries.is_empty() {
            gui_parse_failed.insert(rel, decode_text(&file.bytes).to_ascii_lowercase());
            continue;
        }
        for root in parse_gui_document(&document) {
            roots
                .entry(root.name.to_ascii_lowercase())
                .or_default()
                .push((file.content.normalized_game_relative_path.clone(), root));
        }
    }

    let mut sprites = SpriteRegistry::new();
    let mut gfx_parse_failed = BTreeMap::new();
    for rel in gfx_files {
        let Some(file) = vfs.resolve(&rel)? else {
            return Err(format!("HUD VFS listing could not resolve {rel}"));
        };
        validate_resolved(&file, &rel, source_inventory_sha256)?;
        let document = parse_bytes(&file.bytes);
        if !document.recoveries.is_empty() {
            gfx_parse_failed.insert(rel, decode_text(&file.bytes).to_ascii_lowercase());
            continue;
        }
        sprites.add_document(&file.content.normalized_game_relative_path, &document);
    }

    let core_surfaces = [
        HudSurface::Topbar,
        HudSurface::Minimap,
        HudSurface::Mapmodes,
        HudSurface::TheatreSelector,
    ];
    let activation_by_surface = activation
        .active_surfaces
        .iter()
        .map(|row| (row.surface, row))
        .collect::<BTreeMap<_, _>>();
    let mut all_content = BTreeMap::<String, UsedContentKey>::new();
    let mut closures = Vec::new();
    for activation_row in &activation.active_surfaces {
        let mut missing = Vec::new();
        if core_surfaces.contains(&activation_row.surface) {
            for surface in core_surfaces {
                if !activation_by_surface.contains_key(&surface) {
                    missing.push(format!(
                        "compose_hud_activation:{}",
                        neutral(surface.token())
                    ));
                }
            }
        }
        let root_name = &activation_row.runtime_root_name;
        let candidates = roots
            .get(&root_name.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default();
        if let Some(files) = failed_files_mentioning(&gui_parse_failed, root_name) {
            missing.push(format!(
                "compose_hud_root:{root_name}:ambiguous_unparsed_files={files}"
            ));
        }
        let mut bindings = Vec::new();
        if candidates.len() != 1 {
            missing.push(format!(
                "compose_hud_root:{root_name}:effective_candidate_count={}",
                candidates.len()
            ));
        } else {
            let (gui_path, root) = candidates.into_iter().next().unwrap();
            let gui_key = resolve_key(vfs, &gui_path, source_inventory_sha256)?;
            bindings.push(ResolvedHudRoleBinding {
                semantic_role: HudSemanticRole::GuiRoot {
                    window_or_root: root_name.clone(),
                    layout_identity_sha256: gui_key.payload_sha256.clone(),
                },
                content: gui_key.clone(),
            });
            all_content.insert(gui_key.normalized_game_relative_path.clone(), gui_key);
            append_root_dependencies(
                vfs,
                &sprites,
                &gfx_parse_failed,
                &localisation,
                source_inventory_sha256,
                &neutral(root_name),
                &root,
                &mut bindings,
                &mut all_content,
                &mut missing,
            )?;
        }
        bindings.sort_by(|left, right| {
            let left_role = serde_json::to_string(&left.semantic_role).unwrap();
            let right_role = serde_json::to_string(&right.semantic_role).unwrap();
            left_role.cmp(&right_role).then_with(|| {
                left.content
                    .normalized_game_relative_path
                    .cmp(&right.content.normalized_game_relative_path)
            })
        });
        let closure_hash =
            hex_sha256(&serde_json::to_vec(&bindings).map_err(|error| error.to_string())?);
        closures.push(AuthenticatedHudSurfaceClosure {
            surface: activation_row.surface,
            runtime_root_name: root_name.clone(),
            complete_dependency_closure: missing.is_empty() && !bindings.is_empty(),
            dependency_closure_sha256: closure_hash,
            missing_roles: missing,
            bindings,
        });
    }
    let document = EffectiveVfsHudRoleIndexDocument {
        schema: "extractor.effective-vfs-hud-role-index.v1".into(),
        authority_manifest_sha256: source_inventory_sha256.into(),
        source_inventory_sha256: source_inventory_sha256.into(),
        winning_content: all_content.into_values().collect(),
        closures,
    };
    document.validate()?;
    Ok(document)
}

#[allow(clippy::too_many_arguments)]
fn append_root_dependencies(
    vfs: &impl HudContentVfs,
    sprites: &SpriteRegistry,
    gfx_parse_failed: &BTreeMap<String, String>,
    localisation: &BTreeMap<String, String>,
    authority: &str,
    root_scope: &str,
    root: &GuiElement,
    bindings: &mut Vec<ResolvedHudRoleBinding>,
    all_content: &mut BTreeMap<String, UsedContentKey>,
    missing: &mut Vec<String>,
) -> Result<(), String> {
    let mut refs = RootRefs::default();
    collect_element_refs(root, &mut refs, root_scope, 0);
    for (element, sprite_id) in refs.sprites {
        if let Some(files) = failed_files_mentioning(gfx_parse_failed, &sprite_id) {
            missing.push(format!(
                "sprite:{element}:{sprite_id}:ambiguous_unparsed_files={files}"
            ));
        }
        let Some(sprite) = sprites.get(&sprite_id) else {
            missing.push(format!("sprite:{element}:{sprite_id}"));
            continue;
        };
        let gfx_key = resolve_key(vfs, &sprite.source_file, authority)?;
        bindings.push(ResolvedHudRoleBinding {
            semantic_role: HudSemanticRole::Sprite {
                element: element.clone(),
                sprite_id: sprite_id.clone(),
            },
            content: gfx_key.clone(),
        });
        all_content.insert(gfx_key.normalized_game_relative_path.clone(), gfx_key);
        for (slot_index, slot) in sprite.texture_files.iter().enumerate() {
            let texture_path = normalize_path(&slot.path);
            match resolve_texture_key(vfs, &texture_path, authority) {
                Ok(texture_key) => {
                    bindings.push(ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::SpriteTexture {
                            element: element.clone(),
                            sprite_id: sprite_id.clone(),
                            texture_slot: neutral(&format!("{}_{}", slot.key, slot_index)),
                        },
                        content: texture_key.clone(),
                    });
                    all_content.insert(
                        texture_key.normalized_game_relative_path.clone(),
                        texture_key,
                    );
                }
                Err(_) => missing.push(format!(
                    "sprite_texture:{element}:{sprite_id}:{}:{texture_path}",
                    slot.key
                )),
            }
        }
    }
    for (element, font_id) in refs.fonts {
        match bitmap_font_dependency_paths(vfs, &font_id, authority) {
            Ok(paths) => {
                if let Some(path) = paths.alias_source {
                    append_binding(
                        vfs,
                        authority,
                        &path,
                        HudSemanticRole::DefineOrScriptedConsumer {
                            consumer: neutral(&format!("font_alias_{element}_{font_id}")),
                            source_identity: neutral(&font_id),
                        },
                        bindings,
                        all_content,
                    )?;
                }
                append_binding(
                    vfs,
                    authority,
                    &paths.descriptor,
                    HudSemanticRole::Font {
                        element: element.clone(),
                        font_id: font_id.clone(),
                    },
                    bindings,
                    all_content,
                )?;
                for (index, path) in paths.pages.into_iter().enumerate() {
                    append_binding(
                        vfs,
                        authority,
                        &path,
                        HudSemanticRole::FontTexture {
                            element: element.clone(),
                            font_id: font_id.clone(),
                            texture_slot: format!("page_{index}"),
                        },
                        bindings,
                        all_content,
                    )?;
                }
            }
            Err(error) => missing.push(format!("font:{element}:{font_id}:{error}")),
        }
    }
    for (consumer, loc_key) in refs.texts {
        if let Some(path) = localisation.get(&loc_key.to_ascii_lowercase()) {
            append_binding(
                vfs,
                authority,
                path,
                HudSemanticRole::Localisation {
                    consumer,
                    localisation_key: loc_key,
                },
                bindings,
                all_content,
            )?;
        }
    }
    Ok(())
}

fn append_binding(
    vfs: &impl HudContentVfs,
    authority: &str,
    path: &str,
    semantic_role: HudSemanticRole,
    bindings: &mut Vec<ResolvedHudRoleBinding>,
    all_content: &mut BTreeMap<String, UsedContentKey>,
) -> Result<(), String> {
    let key = resolve_key(vfs, path, authority)?;
    bindings.push(ResolvedHudRoleBinding {
        semantic_role,
        content: key.clone(),
    });
    all_content.insert(key.normalized_game_relative_path.clone(), key);
    Ok(())
}

#[derive(Default)]
struct RootRefs {
    sprites: BTreeSet<(String, String)>,
    fonts: BTreeSet<(String, String)>,
    texts: BTreeSet<(String, String)>,
}

fn collect_element_refs(
    element: &GuiElement,
    refs: &mut RootRefs,
    root_scope: &str,
    ordinal: usize,
) {
    let identity = neutral(&format!("{root_scope}_e{ordinal}_{}", element.name));
    for sprite in &element.sprite_refs {
        if !sprite.trim().is_empty() {
            refs.sprites.insert((identity.clone(), sprite.clone()));
        }
    }
    if let Some(font) = element
        .font
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        refs.fonts.insert((identity.clone(), font.clone()));
    }
    if let Some(text) = element
        .text
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        refs.texts.insert((identity.clone(), text.clone()));
    }
    collect_raw_refs(&element.raw, &identity, refs);
    for (index, child) in element.children.iter().enumerate() {
        collect_element_refs(
            child,
            refs,
            root_scope,
            ordinal.saturating_mul(4096).saturating_add(index + 1),
        );
    }
}

fn collect_raw_refs(value: &Value, identity: &str, refs: &mut RootRefs) {
    for entry in value.fields() {
        if let Some(key) = entry.key.as_deref() {
            if let Some(scalar) = entry
                .value
                .as_scalar()
                .filter(|value| !value.trim().is_empty())
            {
                let folded = key.to_ascii_lowercase();
                if folded == "buttontext" {
                    refs.texts
                        .insert((neutral(&format!("{identity}_{key}")), scalar.to_string()));
                } else if folded == "font" {
                    refs.fonts
                        .insert((identity.to_string(), scalar.to_string()));
                } else if folded == "quadtexturesprite"
                    || folded == "spritetype"
                    || folded.ends_with("spritetype")
                {
                    refs.sprites
                        .insert((neutral(&format!("{identity}_{key}")), scalar.to_string()));
                }
            }
        }
        if entry.value.as_object().is_some() {
            collect_raw_refs(&entry.value, identity, refs);
        }
    }
}

fn localisation_sources(
    vfs: &impl HudContentVfs,
    language: &str,
) -> Result<BTreeMap<String, String>, String> {
    let expected = if language.starts_with("l_") {
        language.to_string()
    } else {
        format!("l_{language}")
    };
    let mut out = BTreeMap::new();
    for rel in vfs.list_recursive("localisation", "yml")? {
        let Some(file) = vfs.resolve(&rel)? else {
            continue;
        };
        let text = decode_text(&file.bytes);
        let mut lines = text.lines().filter(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#')
        });
        let Some(header) = lines.next() else { continue };
        if !header
            .trim()
            .trim_end_matches(':')
            .eq_ignore_ascii_case(&expected)
        {
            continue;
        }
        for line in lines {
            let line = line.trim_start();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let Some((key, _)) = line.split_once(':') else {
                continue;
            };
            if !key.trim().is_empty() {
                out.insert(key.trim().to_ascii_lowercase(), rel.clone());
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Debug)]
struct BitmapFontDependencyPaths {
    alias_source: Option<String>,
    descriptor: String,
    pages: Vec<String>,
}

fn bitmap_font_dependency_paths(
    vfs: &impl HudContentVfs,
    symbolic_name: &str,
    authority: &str,
) -> Result<BitmapFontDependencyPaths, String> {
    if symbolic_name.trim().is_empty() {
        return Err("blank bitmap-font identity".into());
    }
    let direct = format!("gfx/fonts/{symbolic_name}.fnt");
    let mut used_alias = false;
    let descriptor = if vfs.resolve(&direct)?.is_some() {
        direct
    } else {
        used_alias = true;
        let core = vfs
            .resolve("interface/core.gfx")?
            .ok_or_else(|| format!("font {symbolic_name} has no descriptor or core.gfx alias"))?;
        validate_resolved(&core, "interface/core.gfx", authority)?;
        let document = parse_bytes(&core.bytes);
        if !document.recoveries.is_empty() {
            return Err("core.gfx font alias parse required recovery".into());
        }
        let mut aliases = Vec::new();
        for block in document.root.values_ci("bitmapfonts") {
            for entry in block.fields() {
                let Some(kind) = entry.key.as_deref() else {
                    continue;
                };
                if !kind.eq_ignore_ascii_case("bitmapfont")
                    && !kind.eq_ignore_ascii_case("bitmapfont_override")
                {
                    continue;
                }
                if entry
                    .value
                    .first_scalar_ci("name")
                    .is_some_and(|name| name.eq_ignore_ascii_case(symbolic_name))
                {
                    if let Some(path) = entry.value.first_scalar_ci("path") {
                        let mut path = normalize_path(path);
                        if !path.to_ascii_lowercase().ends_with(".fnt") {
                            path.push_str(".fnt");
                        }
                        aliases.push(path);
                    }
                }
            }
        }
        aliases.sort();
        aliases.dedup();
        if aliases.len() != 1 {
            return Err(format!(
                "font {symbolic_name} has {} effective aliases",
                aliases.len()
            ));
        }
        aliases.remove(0)
    };
    let resolved = vfs
        .resolve(&descriptor)?
        .ok_or_else(|| format!("missing bitmap-font descriptor {descriptor}"))?;
    validate_resolved(&resolved, &descriptor, authority)?;
    let stem = Path::new(&resolved.content.normalized_game_relative_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(symbolic_name);
    let parent = Path::new(&resolved.content.normalized_game_relative_path)
        .parent()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let page_files = bmfont_page_files(&resolved.bytes)?;
    let mut pages = Vec::new();
    if page_files.is_empty() {
        let candidates = [
            format!("gfx/fonts/{stem}.dds"),
            format!("gfx/fonts/{stem}.tga"),
            format!("gfx/fonts/{stem}_0.dds"),
            format!("gfx/fonts/{stem}_0.tga"),
        ];
        let mut found = Vec::new();
        for candidate in candidates {
            if vfs.resolve(&candidate)?.is_some() {
                found.push(candidate);
            }
        }
        if found.len() != 1 {
            return Err(format!(
                "font {symbolic_name} has {} inferred atlas candidates",
                found.len()
            ));
        }
        pages = found;
    } else {
        for (id, page) in page_files {
            let path = normalize_path(&format!("{parent}/{page}"));
            if vfs.resolve(&path)?.is_some() {
                pages.push(path);
                continue;
            }
            let mut fallback = None;
            for candidate in [
                format!("gfx/fonts/{stem}.dds"),
                format!("gfx/fonts/{stem}.tga"),
                format!("gfx/fonts/{stem}_{id}.dds"),
                format!("gfx/fonts/{stem}_{id}.tga"),
            ] {
                if vfs.resolve(&candidate)?.is_some() {
                    fallback = Some(candidate);
                    break;
                }
            }
            pages.push(fallback.ok_or_else(|| {
                format!("bitmap-font page {path} and inferred {stem} page {id} are absent")
            })?);
        }
    }
    pages.sort();
    pages.dedup();
    Ok(BitmapFontDependencyPaths {
        alias_source: used_alias.then(|| "interface/core.gfx".into()),
        descriptor: resolved.content.normalized_game_relative_path,
        pages,
    })
}

fn bmfont_page_files(bytes: &[u8]) -> Result<BTreeMap<u32, String>, String> {
    if bytes.get(0..3) == Some(b"BMF") {
        if bytes.len() < 4 {
            return Err("binary BMFont header is truncated".into());
        }
        let mut position = 4usize;
        while position < bytes.len() {
            if position + 5 > bytes.len() {
                return Err("binary BMFont block header is truncated".into());
            }
            let block_type = bytes[position];
            let size =
                u32::from_le_bytes(bytes[position + 1..position + 5].try_into().unwrap()) as usize;
            position += 5;
            let block = bytes
                .get(position..position + size)
                .ok_or("binary BMFont block is truncated")?;
            if block_type == 3 {
                return Ok(block
                    .split(|byte| *byte == 0)
                    .filter(|page| !page.is_empty())
                    .enumerate()
                    .map(|(index, page)| (index as u32, String::from_utf8_lossy(page).into_owned()))
                    .collect());
            }
            position += size;
        }
        return Ok(BTreeMap::new());
    }
    let mut pages = BTreeMap::new();
    for line in decode_text(bytes).lines().map(str::trim) {
        let Some(rest) = line.strip_prefix("page ") else {
            continue;
        };
        let fields = parse_kv_fields(rest);
        if let (Some(id), Some(file)) = (
            fields.get("id").and_then(|value| value.parse::<u32>().ok()),
            fields.get("file"),
        ) {
            pages.insert(id, file.clone());
        }
    }
    Ok(pages)
}

fn parse_kv_fields(text: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let mut position = 0usize;
    let bytes = text.as_bytes();
    while position < bytes.len() {
        while position < bytes.len() && bytes[position].is_ascii_whitespace() {
            position += 1;
        }
        let key_start = position;
        while position < bytes.len()
            && bytes[position] != b'='
            && !bytes[position].is_ascii_whitespace()
        {
            position += 1;
        }
        if position >= bytes.len() || bytes[position] != b'=' {
            while position < bytes.len() && !bytes[position].is_ascii_whitespace() {
                position += 1;
            }
            continue;
        }
        let key = &text[key_start..position];
        position += 1;
        let value = if position < bytes.len() && bytes[position] == b'"' {
            position += 1;
            let start = position;
            while position < bytes.len() && bytes[position] != b'"' {
                position += 1;
            }
            let value = text[start..position].to_string();
            position = (position + 1).min(bytes.len());
            value
        } else {
            let start = position;
            while position < bytes.len() && !bytes[position].is_ascii_whitespace() {
                position += 1;
            }
            text[start..position].to_string()
        };
        fields.insert(key.to_string(), value);
    }
    fields
}

fn resolve_key(
    vfs: &impl HudContentVfs,
    path: &str,
    authority: &str,
) -> Result<UsedContentKey, String> {
    let file = vfs
        .resolve(path)?
        .ok_or_else(|| format!("missing HUD dependency {path}"))?;
    validate_resolved(&file, path, authority)?;
    Ok(file.content)
}

fn resolve_texture_key(
    vfs: &impl HudContentVfs,
    path: &str,
    authority: &str,
) -> Result<UsedContentKey, String> {
    let normalized = normalize_path(path);
    if let Some(file) = vfs.resolve(&normalized)? {
        validate_resolved(&file, &normalized, authority)?;
        return Ok(file.content);
    }
    let Some((stem, extension)) = normalized.rsplit_once('.') else {
        return Err(format!("missing HUD texture dependency {normalized}"));
    };
    let alternate = match extension.to_ascii_lowercase().as_str() {
        "tga" => format!("{stem}.dds"),
        "dds" => format!("{stem}.tga"),
        _ => return Err(format!("missing HUD texture dependency {normalized}")),
    };
    let file = vfs.resolve(&alternate)?.ok_or_else(|| {
        format!("missing HUD texture dependency {normalized} (also tried {alternate})")
    })?;
    validate_resolved(&file, &alternate, authority)?;
    Ok(file.content)
}

fn validate_resolved(
    file: &ResolvedHudFile,
    requested_path: &str,
    authority: &str,
) -> Result<(), String> {
    file.content.validate()?;
    let requested = normalize_path(requested_path);
    if file.content.normalized_game_relative_path != requested
        || file.content.effective_vfs_manifest_sha256 != authority
        || file.content.payload_bytes != file.bytes.len() as u64
        || file.content.payload_sha256 != hex_sha256(&file.bytes)
    {
        return Err(format!(
            "HUD VFS resolved identity/bytes drift for {requested_path}"
        ));
    }
    Ok(())
}

fn failed_files_mentioning(failed: &BTreeMap<String, String>, identifier: &str) -> Option<String> {
    let needle = identifier.to_ascii_lowercase();
    let files = failed
        .iter()
        .filter_map(|(path, text)| text.contains(&needle).then_some(path.as_str()))
        .collect::<Vec<_>>();
    (!files.is_empty()).then(|| files.join(","))
}

fn normalize_path(value: &str) -> String {
    normalize_game_relative_path(value).unwrap_or_else(|_| {
        value
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_ascii_lowercase()
    })
}

fn neutral(value: &str) -> String {
    let value = value
        .chars()
        .map(|ch| {
            if ch == '/' || ch == '\\' || ch.is_control() {
                '_'
            } else {
                ch
            }
        })
        .collect::<String>();
    if value.trim().is_empty() {
        "unnamed".into()
    } else {
        value
    }
}

fn validate_sha(value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err("HUD source inventory authority is not lowercase SHA-256".into())
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
