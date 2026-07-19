//! In-process effective-VFS dependency-closure emitter for authenticated HUD roots.

use crate::font::bitmap_font_dependency_paths;
use crate::gfx::SpriteRegistry;
use crate::gui::{parse_gui_document, GuiElement};
use crate::paradox_text::{decode_text, parse_file, Value};
use map_viewer::{GameFileResolver, ResolvedGameFile};
use portable_content_manifest::hud_role::{
    AuthenticatedHudSurfaceClosure, EffectiveVfsHudRoleIndexDocument, HudSemanticRole, HudSurface,
    HudSurfaceActivationFrame, ResolvedHudRoleBinding,
};
use replay_core::{ContentLayerKind, RecordedContentLayer, UsedContentKey};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Build the recording-side effective VFS from the exact launch-corroborated
/// install/profile roots and emit the authenticated HUD closure in process.
/// The roots never enter the portable document.
pub fn emit_effective_vfs_hud_role_index_from_launch(
    game_root: &Path,
    user_dir: &Path,
    recorded_layers: &[RecordedContentLayer],
    activation: &HudSurfaceActivationFrame,
    source_inventory_sha256: &str,
    localisation_language: &str,
) -> Result<EffectiveVfsHudRoleIndexDocument, String> {
    let recorded = recorded_layers
        .iter()
        .filter(|layer| layer.layer_kind != ContentLayerKind::Base)
        .map(|layer| map_viewer::RecordedContentSource {
            ordinal: layer.ordinal,
            source_id: layer.layer_id.clone(),
        })
        .collect::<Vec<_>>();
    let resolver = GameFileResolver::from_recorded_content_layers(
        game_root,
        user_dir.to_path_buf(),
        &recorded,
    )
    .map_err(|error| format!("construct launch-bound HUD effective VFS failed: {error}"))?;
    emit_effective_vfs_hud_role_index(
        &resolver,
        activation,
        source_inventory_sha256,
        localisation_language,
    )
}

pub fn emit_effective_vfs_hud_role_index(
    resolver: &GameFileResolver,
    activation: &HudSurfaceActivationFrame,
    source_inventory_sha256: &str,
    localisation_language: &str,
) -> Result<EffectiveVfsHudRoleIndexDocument, String> {
    if activation.status_flags != 0 || activation.active_surfaces.is_empty() {
        return Err("HUD index emitter requires a nonempty latest-clean d79 frame".into());
    }
    validate_sha(source_inventory_sha256)?;
    let gui_files = resolver
        .resolve_dir_files_recursive("interface", Some("gui"))
        .map_err(|error| error.to_string())?;
    let gfx_files = resolver
        .resolve_dir_files_recursive("interface", Some("gfx"))
        .map_err(|error| error.to_string())?;
    let localisation = localisation_sources(resolver, localisation_language)?;

    let mut roots = BTreeMap::<String, Vec<(String, GuiElement)>>::new();
    let mut gui_parse_failed = BTreeMap::new();
    for (rel, path) in gui_files {
        let document = match parse_file(&path) {
            Ok(document) => document,
            Err(_) => {
                gui_parse_failed.insert(rel, folded_file_text(&path));
                continue;
            }
        };
        if !document.recoveries.is_empty() {
            gui_parse_failed.insert(rel, folded_file_text(&path));
            continue;
        }
        for root in parse_gui_document(&document) {
            roots
                .entry(root.name.to_ascii_lowercase())
                .or_default()
                .push((rel.clone(), root));
        }
    }

    let mut sprites = SpriteRegistry::new();
    let mut gfx_parse_failed = BTreeMap::new();
    for (rel, path) in gfx_files {
        let document = match parse_file(&path) {
            Ok(document) if document.recoveries.is_empty() => document,
            _ => {
                gfx_parse_failed.insert(rel, folded_file_text(&path));
                continue;
            }
        };
        sprites.add_document(&rel, &document);
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
        let required_roots = [(
            activation_row.runtime_root_name.clone(),
            activation_row.surface,
        )];
        let mut bindings = Vec::new();
        for (root_name, owned_surface) in required_roots {
            let candidates = roots
                .get(&root_name.to_ascii_lowercase())
                .cloned()
                .unwrap_or_default();
            if let Some(files) = failed_files_mentioning(&gui_parse_failed, &root_name) {
                missing.push(format!(
                    "compose_hud_root:{}:ambiguous_unparsed_files={}",
                    root_name, files
                ));
            }
            if candidates.len() != 1 {
                missing.push(format!(
                    "compose_hud_root:{}:effective_candidate_count={}",
                    root_name,
                    candidates.len()
                ));
                continue;
            }
            let (gui_path, root) = candidates.into_iter().next().unwrap();
            let gui_key = resolve_key(resolver, &gui_path, source_inventory_sha256)?;
            if owned_surface != activation_row.surface {
                return Err("HUD closure root ownership drifted".into());
            };
            bindings.push(ResolvedHudRoleBinding {
                semantic_role: HudSemanticRole::GuiRoot {
                    window_or_root: root_name.clone(),
                    layout_identity_sha256: gui_key.payload_sha256.clone(),
                },
                content: gui_key.clone(),
            });
            all_content.insert(gui_key.normalized_game_relative_path.clone(), gui_key);
            append_root_dependencies(
                resolver,
                &sprites,
                &gfx_parse_failed,
                &localisation,
                source_inventory_sha256,
                &neutral(&root_name),
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
            Sha256::hex(&serde_json::to_vec(&bindings).map_err(|error| error.to_string())?);
        closures.push(AuthenticatedHudSurfaceClosure {
            surface: activation_row.surface,
            runtime_root_name: activation_row.runtime_root_name.clone(),
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
    resolver: &GameFileResolver,
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
        let gfx_key = resolve_key(resolver, &sprite.source_file, authority)?;
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
            match resolve_texture_key(resolver, &texture_path, authority) {
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
                    "sprite_texture:{element}:{sprite_id}:{}:{}",
                    slot.key, texture_path
                )),
            }
        }
    }
    for (element, font_id) in refs.fonts {
        match bitmap_font_dependency_paths(resolver, &font_id) {
            Ok(paths) => {
                if let Some(path) = paths.alias_source {
                    let key = resolve_key(resolver, &path, authority)?;
                    bindings.push(ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::DefineOrScriptedConsumer {
                            consumer: neutral(&format!("font_alias_{element}_{font_id}")),
                            source_identity: neutral(&font_id),
                        },
                        content: key.clone(),
                    });
                    all_content.insert(key.normalized_game_relative_path.clone(), key);
                }
                let key = resolve_key(resolver, &paths.descriptor, authority)?;
                bindings.push(ResolvedHudRoleBinding {
                    semantic_role: HudSemanticRole::Font {
                        element: element.clone(),
                        font_id: font_id.clone(),
                    },
                    content: key.clone(),
                });
                all_content.insert(key.normalized_game_relative_path.clone(), key);
                for (index, path) in paths.pages.into_iter().enumerate() {
                    let key = resolve_key(resolver, &path, authority)?;
                    bindings.push(ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::FontTexture {
                            element: element.clone(),
                            font_id: font_id.clone(),
                            texture_slot: format!("page_{index}"),
                        },
                        content: key.clone(),
                    });
                    all_content.insert(key.normalized_game_relative_path.clone(), key);
                }
            }
            Err(error) => missing.push(format!("font:{element}:{font_id}:{}", error.message)),
        }
    }
    for (consumer, loc_key) in refs.texts {
        if let Some(path) = localisation.get(&loc_key.to_ascii_lowercase()) {
            let key = resolve_key(resolver, path, authority)?;
            bindings.push(ResolvedHudRoleBinding {
                semantic_role: HudSemanticRole::Localisation {
                    consumer,
                    localisation_key: loc_key,
                },
                content: key.clone(),
            });
            all_content.insert(key.normalized_game_relative_path.clone(), key);
        }
    }
    Ok(())
}

fn folded_file_text(path: &Path) -> String {
    std::fs::read(path)
        .map(|bytes| decode_text(&bytes).to_ascii_lowercase())
        .unwrap_or_default()
}

fn failed_files_mentioning(failed: &BTreeMap<String, String>, identifier: &str) -> Option<String> {
    let needle = identifier.to_ascii_lowercase();
    let files = failed
        .iter()
        .filter_map(|(path, text)| text.contains(&needle).then_some(path.as_str()))
        .collect::<Vec<_>>();
    (!files.is_empty()).then(|| files.join(","))
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
    resolver: &GameFileResolver,
    language: &str,
) -> Result<BTreeMap<String, String>, String> {
    let expected = if language.starts_with("l_") {
        language.to_string()
    } else {
        format!("l_{language}")
    };
    let files = resolver
        .resolve_dir_files_recursive("localisation", Some("yml"))
        .map_err(|error| error.to_string())?;
    let mut out = BTreeMap::new();
    for (rel, path) in files {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let text = decode_text(&bytes);
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

fn resolve_key(
    resolver: &GameFileResolver,
    path: &str,
    authority: &str,
) -> Result<UsedContentKey, String> {
    let resolved = resolver
        .resolve_required_with_provenance(path, "HUD dependency")
        .map_err(|error| error.to_string())?;
    key_from_resolved(&resolved, authority)
}

/// Match the production UI texture loader: authored `.tga` and `.dds`
/// identities may resolve to the other physical extension, but no unrelated
/// path or vanilla fallback is admitted.
fn resolve_texture_key(
    resolver: &GameFileResolver,
    path: &str,
    authority: &str,
) -> Result<UsedContentKey, String> {
    let normalized = normalize_path(path);
    if let Some(resolved) = resolver
        .resolve_with_provenance(&normalized)
        .map_err(|error| error.to_string())?
    {
        return key_from_resolved(&resolved, authority);
    }
    let Some((stem, extension)) = normalized.rsplit_once('.') else {
        return Err(format!("missing HUD texture dependency {normalized}"));
    };
    let alternate = match extension.to_ascii_lowercase().as_str() {
        "tga" => format!("{stem}.dds"),
        "dds" => format!("{stem}.tga"),
        _ => return Err(format!("missing HUD texture dependency {normalized}")),
    };
    let resolved = resolver
        .resolve_with_provenance(&alternate)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            format!("missing HUD texture dependency {normalized} (also tried {alternate})")
        })?;
    key_from_resolved(&resolved, authority)
}

fn key_from_resolved(
    resolved: &ResolvedGameFile,
    authority: &str,
) -> Result<UsedContentKey, String> {
    let layer_kind = match resolved.source.kind.as_str() {
        "base" => ContentLayerKind::Base,
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        value => return Err(format!("unsupported HUD VFS source kind {value}")),
    };
    Ok(UsedContentKey {
        effective_vfs_manifest_sha256: authority.into(),
        normalized_game_relative_path: normalize_path(&resolved.logical_path),
        winning_layer_ordinal: u32::try_from(resolved.source.ordinal)
            .map_err(|_| "HUD VFS ordinal exceeds u32")?,
        layer_kind,
        layer_id: resolved.source.source_id.clone(),
        source_sha256: resolved
            .source
            .archive_sha256
            .clone()
            .unwrap_or_else(|| resolved.sha256.clone()),
        payload_sha256: resolved.sha256.clone(),
        payload_bytes: resolved.byte_len,
    })
}

fn normalize_path(value: &str) -> String {
    crate::assets::normalize_rel_path(value)
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

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::hud_role::HudSurfaceActivation;
    use std::fs;
    use std::path::PathBuf;

    fn gui(names: &[&str]) -> String {
        let roots = names
            .iter()
            .map(|name| format!("containerWindowType = {{ name = \"{name}\" }}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("guiTypes = {{\n{roots}\n}}\n")
    }

    fn activation() -> HudSurfaceActivationFrame {
        HudSurfaceActivationFrame {
            frame_hour: 10,
            status_flags: 0,
            observation_sequence: 11,
            active_surfaces: vec![
                HudSurfaceActivation {
                    surface: HudSurface::Topbar,
                    runtime_root_name: "topbar".into(),
                },
                HudSurfaceActivation {
                    surface: HudSurface::Minimap,
                    runtime_root_name: "mini_map_interface".into(),
                },
                HudSurfaceActivation {
                    surface: HudSurface::Mapmodes,
                    runtime_root_name: "MapmodesInterface_Ingame".into(),
                },
                HudSurfaceActivation {
                    surface: HudSurface::TheatreSelector,
                    runtime_root_name: "theatre_selection_window".into(),
                },
            ],
        }
    }

    fn fixture(parent: &Path, include_tooltip: bool) -> (PathBuf, PathBuf) {
        let game = parent.join("game");
        let mod_root = parent.join("active_mod");
        fs::create_dir_all(game.join("interface")).unwrap();
        fs::create_dir_all(game.join("localisation")).unwrap();
        fs::create_dir_all(mod_root.join("interface")).unwrap();
        fs::write(game.join("interface/topbar.gui"), gui(&["topbar"])).unwrap();
        fs::write(
            game.join("interface/minimap.gui"),
            gui(&["mini_map_interface"]),
        )
        .unwrap();
        fs::write(
            game.join("interface/mapmodes_interface.gui"),
            gui(&["MapmodesInterface_Ingame"]),
        )
        .unwrap();
        fs::write(
            game.join("interface/theatreselector.gui"),
            gui(&["theatre_selection_window", "theatre_selection_entry"]),
        )
        .unwrap();
        fs::write(
            game.join("interface/menubar.gui"),
            gui(&["zoom_in", "zoom_out"]),
        )
        .unwrap();
        if include_tooltip {
            fs::write(game.join("interface/core.gui"), gui(&["ToolTip"])).unwrap();
        }
        // The active mod changes the authenticated controller root while all
        // sibling/template roots continue to resolve through the same VFS.
        fs::write(
            mod_root.join("interface/topbar.gui"),
            "guiTypes = { containerWindowType = { name = \"topbar\" instantTextBoxType = { name = \"literal_runtime_text\" text = \"123\" } } }",
        )
        .unwrap();
        (game, mod_root)
    }

    fn temp(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hud_role_index_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn complete_compose_hud_closure_uses_mod_winner_and_is_relocation_stable_twice() {
        let first = temp("first");
        let second = temp("second");
        let (game_a, mod_a) = fixture(&first, true);
        let (game_b, mod_b) = fixture(&second, true);
        let authority = "a".repeat(64);
        let emit = |game: &Path, mod_root: &Path| {
            let resolver =
                GameFileResolver::with_mod_paths(game, vec![mod_root.to_path_buf()]).unwrap();
            emit_effective_vfs_hud_role_index(&resolver, &activation(), &authority, "english")
                .unwrap()
        };
        let index_a1 = emit(&game_a, &mod_a);
        let index_a2 = emit(&game_a, &mod_a);
        let index_b = emit(&game_b, &mod_b);
        assert_eq!(index_a1, index_a2);
        assert_eq!(index_a1, index_b);
        assert_eq!(
            index_a1
                .closures
                .iter()
                .filter(|closure| closure.complete_dependency_closure)
                .count(),
            4
        );
        let topbar = index_a1
            .winning_content
            .iter()
            .find(|row| row.normalized_game_relative_path == "interface/topbar.gui")
            .unwrap();
        assert_eq!(topbar.layer_kind, ContentLayerKind::Mod);
        assert_eq!(topbar.layer_id, "active_mod");
        index_a1.validate().unwrap();
        assert!(index_a1.closures.iter().all(|closure| {
            closure.bindings.iter().all(|binding| {
                binding
                    .semantic_role
                    .consumer(closure.surface)
                    .is_ok_and(|consumer| consumer.starts_with("hud_role/v1/"))
            })
        }));
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }

    #[test]
    fn missing_authenticated_core_root_omits_that_surface() {
        let root = temp("missing_core");
        let (game, mod_root) = fixture(&root, true);
        fs::remove_file(game.join("interface/minimap.gui")).unwrap();
        let authority = "a".repeat(64);
        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index =
            emit_effective_vfs_hud_role_index(&resolver, &activation(), &authority, "english")
                .unwrap();
        assert_eq!(
            index
                .closures
                .iter()
                .filter(|closure| closure.complete_dependency_closure)
                .count(),
            3
        );
        assert!(index.closures.iter().any(|closure| {
            closure.surface == HudSurface::Minimap
                && !closure.complete_dependency_closure
                && closure
                    .missing_roles
                    .iter()
                    .any(|role| role.starts_with("compose_hud_root:mini_map_interface:"))
        }));
        index.validate().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn active_designer_root_closes_without_an_unrelated_core_hud_requirement() {
        let root = temp("designer");
        let (game, mod_root) = fixture(&root, true);
        fs::write(
            mod_root.join("interface/tank_designer.gui"),
            gui(&["modded_tank_designer_root"]),
        )
        .unwrap();
        let designer_activation = HudSurfaceActivationFrame {
            frame_hour: 12,
            status_flags: 0,
            observation_sequence: 13,
            active_surfaces: vec![HudSurfaceActivation {
                surface: HudSurface::DesignerTank,
                runtime_root_name: "modded_tank_designer_root".into(),
            }],
        };
        let authority = "b".repeat(64);
        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index = emit_effective_vfs_hud_role_index(
            &resolver,
            &designer_activation,
            &authority,
            "english",
        )
        .unwrap();
        assert_eq!(index.closures.len(), 1);
        assert!(index.closures[0].complete_dependency_closure);
        assert!(index.closures[0].missing_roles.is_empty());
        index.validate().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR"]
    fn installed_effective_vfs_closes_all_always_on_hud_surfaces() {
        let game = PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).unwrap());
        let user = PathBuf::from(std::env::var_os("BATTLE_PLAN_USER_DIR").or_else(|| std::env::var_os("HOI4_USER_DIR")).unwrap());
        let discovered = GameFileResolver::from_dlc_load(&game, user.clone()).unwrap();
        let recorded_layers = discovered
            .active_sources()
            .into_iter()
            .map(|source| RecordedContentLayer {
                ordinal: source.ordinal as u32,
                layer_kind: match source.kind.as_str() {
                    "base" => ContentLayerKind::Base,
                    "integrated_dlc" => ContentLayerKind::IntegratedDlc,
                    "dlc" => ContentLayerKind::Dlc,
                    "mod" => ContentLayerKind::Mod,
                    other => panic!("unexpected installed layer kind {other}"),
                },
                layer_id: source.source_id,
                display_name: "installed-test".into(),
                descriptor_sha256: source.descriptor_sha256,
                source_container_sha256: source.archive_sha256.clone(),
                storage_kind: if source.archive_sha256.is_some() {
                    replay_core::LayerStorageKind::ZipArchive
                } else {
                    replay_core::LayerStorageKind::Directory
                },
                replace_paths: source.replace_path,
                required_for_playback: false,
            })
            .collect::<Vec<_>>();
        let index = emit_effective_vfs_hud_role_index_from_launch(
            &game,
            &user,
            &recorded_layers,
            &activation(),
            &"a".repeat(64),
            "english",
        )
        .unwrap();
        for closure in &index.closures {
            eprintln!(
                "installed HUD {:?}: complete={} bindings={} missing={:?}",
                closure.surface,
                closure.complete_dependency_closure,
                closure.bindings.len(),
                closure.missing_roles
            );
        }
        assert_eq!(
            index
                .closures
                .iter()
                .filter(|closure| closure.complete_dependency_closure)
                .count(),
            4
        );
    }
}
