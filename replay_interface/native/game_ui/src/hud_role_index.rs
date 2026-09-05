
use crate::definition_admission::{load_effective_definition_catalog, ResolvedGuiDefinitionFile};
use crate::engine_surface_roots::ENGINE_CONTROLLER_ROOTS;
use crate::font::authored_bitmap_font_registry_paths_for_language;
use crate::gfx::SpriteRegistry;
use crate::gui::GuiElement;
use crate::paradox_text::{
    decode_text, destructive_event_count, destructive_losses, diagnose_text_decode,
    engine_tolerated_outer_eof, parse_bytes, Value,
};
use map_viewer::{GameFileResolver, ResolvedGameFile};
use portable_content_manifest::hud_role::{
    validate_runtime_root_name, validate_sha256, validate_token, HudSemanticRole,
    ResolvedHudRoleBinding, MAX_BINDINGS_PER_SURFACE, MAX_ROOTS_PER_SNAPSHOT,
};
use portable_content_manifest::{
    authored_ui::{canonical_localisation_language, safe_language_directory},
    normalize_game_relative_path,
};
use replay_core::{ContentLayerKind, UsedContentKey};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::path::Path;

pub const REPLAY_ROOT_CONSUMER_PREFIX: &str = "hud_role/v1/root";
pub const REPLAY_ROOT_INDEX_SCHEMA: &str = "replay.effective-vfs-root-role-index.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedRootClosure {
    pub runtime_root_name: String,
    pub complete_dependency_closure: bool,
    pub dependency_closure_sha256: String,
    #[serde(default)]
    pub missing_roles: Vec<String>,
    pub bindings: Vec<ResolvedHudRoleBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveVfsRootRoleIndexDocument {
    pub schema: String,
    pub authority_manifest_sha256: String,
    pub source_inventory_sha256: String,
    #[serde(default)]
    pub recovery_parsed_gui_files: Vec<String>,
    pub winning_content: Vec<UsedContentKey>,
    pub closures: Vec<AuthenticatedRootClosure>,
}

impl EffectiveVfsRootRoleIndexDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != REPLAY_ROOT_INDEX_SCHEMA {
            return Err("replay root-role index schema is unsupported".into());
        }
        validate_sha256(
            &self.authority_manifest_sha256,
            "replay root-role index authority",
        )?;
        if self.source_inventory_sha256 != self.authority_manifest_sha256 {
            return Err("replay root-role index is not bound to its source inventory".into());
        }
        if self.closures.len() > MAX_ROOTS_PER_SNAPSHOT {
            return Err("replay root-role index exceeds its root bound".into());
        }
        if self.closures.len() != ENGINE_CONTROLLER_ROOTS.len() {
            return Err("replay root-role index does not cover every exact-a729 controller".into());
        }
        let mut recoveries = BTreeSet::new();
        for file in &self.recovery_parsed_gui_files {
            if file.trim().is_empty() || !recoveries.insert(file) {
                return Err("replay root-role index repeats or blanks a recovery report".into());
            }
        }
        let mut winners = BTreeMap::new();
        for content in &self.winning_content {
            content.validate()?;
            if content.effective_vfs_manifest_sha256 != self.authority_manifest_sha256 {
                return Err("root-role winning content names a different effective VFS".into());
            }
            if winners
                .insert(content.normalized_game_relative_path.as_str(), content)
                .is_some()
            {
                return Err("root-role winning content repeats a path".into());
            }
        }
        if winners.is_empty() {
            return Err("root-role winning-content inventory is empty".into());
        }
        let mut closure_roots = BTreeSet::new();
        for (closure, expected_root) in self
            .closures
            .iter()
            .zip(ENGINE_CONTROLLER_ROOTS.iter().copied())
        {
            validate_runtime_root_name(&closure.runtime_root_name)?;
            validate_sha256(
                &closure.dependency_closure_sha256,
                "replay root dependency closure",
            )?;
            if closure.runtime_root_name != expected_root {
                return Err("replay root-role index controller order/root identity drifted".into());
            }
            if !closure_roots.insert(closure.runtime_root_name.to_ascii_lowercase()) {
                return Err("root-role index repeats a runtime root".into());
            }
            if closure.complete_dependency_closure && !closure.missing_roles.is_empty() {
                return Err("complete root dependency closure reports missing roles".into());
            }
            if !closure.complete_dependency_closure && closure.missing_roles.is_empty() {
                return Err("incomplete root dependency closure has no typed blocker".into());
            }
            if closure.bindings.len() > MAX_BINDINGS_PER_SURFACE {
                return Err("root-role binding list exceeds its bound".into());
            }
            let mut consumers = BTreeSet::new();
            let mut gui_roots = 0usize;
            for binding in &closure.bindings {
                binding.content.validate()?;
                let expected = winners
                    .get(binding.content.normalized_game_relative_path.as_str())
                    .ok_or("root role is absent from the effective-VFS winning inventory")?;
                if *expected != &binding.content {
                    return Err("root role does not equal its effective-VFS winning row".into());
                }
                if matches!(binding.semantic_role, HudSemanticRole::GuiRoot { .. }) {
                    gui_roots += 1;
                }
                if !consumers.insert(root_role_consumer(
                    &closure.runtime_root_name,
                    &binding.semantic_role,
                )?) {
                    return Err("root dependency closure repeats a semantic consumer".into());
                }
            }
            if closure.complete_dependency_closure
                && (closure.bindings.is_empty() || gui_roots != 1)
            {
                return Err("complete root dependency closure lacks exactly one GUI root".into());
            }
            let expected_hash = Sha256::hex(
                &serde_json::to_vec(&closure.bindings)
                    .map_err(|error| format!("serialize replay root bindings: {error}"))?,
            );
            if closure.dependency_closure_sha256 != expected_hash {
                return Err(
                    "root dependency closure hash does not authenticate its bindings".into(),
                );
            }
        }
        Ok(())
    }
}

pub fn root_role_consumer(root_name: &str, role: &HudSemanticRole) -> Result<String, String> {
    validate_runtime_root_name(root_name)?;
    let suffix = match role {
        HudSemanticRole::GuiRoot {
            window_or_root,
            layout_identity_sha256,
        } => {
            validate_token(window_or_root, "root-role window/root")?;
            validate_sha256(layout_identity_sha256, "root-role layout identity")?;
            format!("gui/{window_or_root}/{layout_identity_sha256}")
        }
        HudSemanticRole::Sprite { element, sprite_id } => {
            validate_token(element, "root-role sprite element")?;
            validate_token(sprite_id, "root-role sprite id")?;
            format!("sprite/{element}/{sprite_id}")
        }
        HudSemanticRole::SpriteTexture {
            element,
            sprite_id,
            texture_slot,
        } => {
            validate_token(element, "root-role sprite-texture element")?;
            validate_token(sprite_id, "root-role sprite-texture id")?;
            validate_token(texture_slot, "root-role sprite-texture slot")?;
            format!("sprite_texture/{element}/{sprite_id}/{texture_slot}")
        }
        HudSemanticRole::SpriteEffect {
            element,
            sprite_id,
            effect_token,
        } => {
            validate_token(element, "root-role sprite-effect element")?;
            validate_token(sprite_id, "root-role sprite-effect id")?;
            validate_token(effect_token, "root-role sprite-effect token")?;
            format!("sprite_effect/{element}/{sprite_id}/{effect_token}")
        }
        HudSemanticRole::SpriteEffectInclude {
            element,
            sprite_id,
            include_slot,
        } => {
            validate_token(element, "root-role sprite-effect-include element")?;
            validate_token(sprite_id, "root-role sprite-effect-include id")?;
            validate_token(include_slot, "root-role sprite-effect-include slot")?;
            format!("sprite_effect_include/{element}/{sprite_id}/{include_slot}")
        }
        HudSemanticRole::Font { element, font_id } => {
            validate_token(element, "root-role font element")?;
            validate_token(font_id, "root-role font id")?;
            format!("font/{element}/{font_id}")
        }
        HudSemanticRole::FontTexture {
            element,
            font_id,
            texture_slot,
        } => {
            validate_token(element, "root-role font-texture element")?;
            validate_token(font_id, "root-role font-texture id")?;
            validate_token(texture_slot, "root-role font-texture slot")?;
            format!("font_texture/{element}/{font_id}/{texture_slot}")
        }
        HudSemanticRole::Localisation {
            consumer,
            localisation_key,
        } => {
            validate_token(consumer, "root-role localisation consumer")?;
            validate_token(localisation_key, "root-role localisation key")?;
            format!("localisation/{consumer}/{localisation_key}")
        }
        HudSemanticRole::DefineOrScriptedConsumer {
            consumer,
            source_identity,
        } => {
            validate_token(consumer, "root-role define/scripted consumer")?;
            validate_token(source_identity, "root-role define/scripted source identity")?;
            format!("define_or_scripted/{consumer}/{source_identity}")
        }
    };
    Ok(format!(
        "{REPLAY_ROOT_CONSUMER_PREFIX}/{root_name}/{suffix}"
    ))
}

pub fn emit_effective_vfs_hud_role_index(
    resolver: &GameFileResolver,
    source_inventory_sha256: &str,
    localisation_language: &str,
) -> Result<EffectiveVfsRootRoleIndexDocument, String> {
    resolver.in_resolution_session(|resolver| {
    validate_sha(source_inventory_sha256)?;
    let gui_files = resolver
        .resolve_dir_files_recursive("interface", Some("gui"))
        .map_err(|error| error.to_string())?;
    let gfx_files = resolver
        .resolve_dir_files_recursive("interface", Some("gfx"))
        .map_err(|error| error.to_string())?;
    let mut decode_damaged = BTreeMap::<String, String>::new();
    let localisation = localisation_sources(resolver, localisation_language, &mut decode_damaged)?;

    let gui_catalog = load_effective_definition_catalog(
        gui_files
            .iter()
            .map(|(rel, path)| resolver.read_resolved_bytes(path).map(|bytes| ResolvedGuiDefinitionFile::new(rel.clone(), bytes)))
            .collect::<map_viewer::Result<Vec<_>>>().map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let recovery_parsed_gui_files = gui_catalog.recovery_parsed_files();
    for file in &gui_catalog.files {
        if let Some(events) = file.decode_losses.filter(|events| *events != 0) {
            decode_damaged.insert(
                portable_content_manifest::normalize_game_relative_path(&file.source_file)?,
                neutral(&format!("destructive_events={events}:first=gui decode")),
            );
        }
    }
    let mut roots = BTreeMap::new();
    for root in gui_catalog.effective_roots {
        roots
            .entry(root.root.name.to_ascii_lowercase())
            .or_insert_with(Vec::new)
            .push(root);
    }

    let mut sprites = SpriteRegistry::new();
    let mut gfx_parse_failed = BTreeMap::new();
    for (rel, _) in gfx_files {
        let bytes = resolver.read_required_bytes(&rel, "HUD sprite definitions")
            .map_err(|error| error.to_string())?;
        record_decode_damage(&mut decode_damaged, &rel, &bytes)?;
        let document = parse_bytes(&bytes);
        if !document.recoveries.is_empty() && !engine_tolerated_outer_eof(&document) {
            gfx_parse_failed.insert(rel, decode_text(&bytes).to_ascii_lowercase());
            continue;
        }
        sprites.add_document(&rel, &document);
    }

    let mut all_content = BTreeMap::<String, UsedContentKey>::new();
    let mut closures = Vec::new();
    for &runtime_root_name in ENGINE_CONTROLLER_ROOTS {
        let mut missing = Vec::new();
        let mut bindings = Vec::new();
        for root_name in [runtime_root_name] {
            let candidates = roots
                .get(&root_name.to_ascii_lowercase())
                .cloned()
                .unwrap_or_default();
            let Some(winner) = candidates.into_iter().last() else {
                missing.push(format!("compose_hud_root:{}:undefined_gui_type", root_name));
                continue;
            };
            if !winner.admitted {
                missing.push(format!(
                    "compose_hud_root:{}:definition_admission_rejected:{:?}",
                    root_name, winner.unsupported
                ));
                continue;
            }
            let gui_path = winner.source_file;
            let root = winner.root;
            let gui_key = resolve_key(resolver, &gui_path, source_inventory_sha256)?;
            bindings.push(ResolvedHudRoleBinding {
                semantic_role: HudSemanticRole::GuiRoot {
                    window_or_root: root_name.to_string(),
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
                localisation_language,
                &neutral(&root_name),
                &root,
                &mut bindings,
                &mut all_content,
                &mut missing,
                &mut decode_damaged,
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
        append_decode_damage_blockers(&decode_damaged, &bindings, &mut missing);
        let closure_hash =
            Sha256::hex(&serde_json::to_vec(&bindings).map_err(|error| error.to_string())?);
        closures.push(AuthenticatedRootClosure {
            runtime_root_name: runtime_root_name.into(),
            complete_dependency_closure: missing.is_empty() && !bindings.is_empty(),
            dependency_closure_sha256: closure_hash,
            missing_roles: missing,
            bindings,
        });
    }
    let document = EffectiveVfsRootRoleIndexDocument {
        schema: REPLAY_ROOT_INDEX_SCHEMA.into(),
        authority_manifest_sha256: source_inventory_sha256.into(),
        source_inventory_sha256: source_inventory_sha256.into(),
        recovery_parsed_gui_files,
        winning_content: all_content.into_values().collect(),
        closures,
    };
    document.validate()?;
    Ok(document)
    })
}

#[allow(clippy::too_many_arguments)]
fn append_root_dependencies(
    resolver: &GameFileResolver,
    sprites: &SpriteRegistry,
    gfx_parse_failed: &BTreeMap<String, String>,
    localisation: &BTreeMap<String, String>,
    authority: &str,
    localisation_language: &str,
    root_scope: &str,
    root: &GuiElement,
    bindings: &mut Vec<ResolvedHudRoleBinding>,
    all_content: &mut BTreeMap<String, UsedContentKey>,
    missing: &mut Vec<String>,
    decode_damaged: &mut BTreeMap<String, String>,
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
                Err(error) => missing.push(format!(
                    "sprite_texture:{element}:{sprite_id}:{}:{}:{}",
                    slot.key, texture_path, error
                )),
            }
        }
    }
    for (element, font_id) in refs.fonts {
        match authored_bitmap_font_registry_paths_for_language(
            resolver,
            &font_id,
            localisation_language,
        ) {
            Ok(paths) => {
                for (variant_index, variant) in paths.variants.into_iter().enumerate() {
                    let key = resolve_key(resolver, &variant.registration_source, authority)?;
                    bindings.push(ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::DefineOrScriptedConsumer {
                            consumer: neutral(&format!(
                                "font_registration_{element}_{font_id}_{variant_index}"
                            )),
                            source_identity: neutral(&font_id),
                        },
                        content: key.clone(),
                    });
                    all_content.insert(key.normalized_game_relative_path.clone(), key);
                    for (descriptor_index, descriptor) in
                        variant.ordered_descriptors.into_iter().enumerate()
                    {
                        let key = resolve_key(resolver, &descriptor.descriptor, authority)?;
                        bindings.push(ResolvedHudRoleBinding {
                            semantic_role: HudSemanticRole::Font {
                                element: element.clone(),
                                font_id: format!(
                                    "{font_id}_variant_{variant_index}_descriptor_{descriptor_index}"
                                ),
                            },
                            content: key.clone(),
                        });
                        all_content.insert(key.normalized_game_relative_path.clone(), key);
                        for (page_index, page) in descriptor.ordered_pages.into_iter().enumerate() {
                            let key = resolve_key(resolver, &page.path, authority)?;
                            bindings.push(ResolvedHudRoleBinding {
                                semantic_role: HudSemanticRole::FontTexture {
                                    element: element.clone(),
                                    font_id: font_id.clone(),
                                    texture_slot: format!(
                                        "variant_{variant_index}_descriptor_{descriptor_index}_page_{page_index}"
                                    ),
                                },
                                content: key.clone(),
                            });
                            all_content.insert(key.normalized_game_relative_path.clone(), key);
                        }
                    }
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
    decode_damaged: &mut BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    let language_directory = safe_language_directory(language)?;
    let expected = canonical_localisation_language(language)?;
    let directory = format!("localisation/{language_directory}");
    let files = match resolver.resolve_dir_files_recursive(&directory, Some("yml")) {
        Ok(files) => files,
        Err(error) if error.to_string() == format!("missing directory: {directory}") => Vec::new(),
        Err(error) => return Err(error.to_string()),
    };
    let mut out = BTreeMap::new();
    for (rel, _) in files {
        let bytes = resolver
            .read_required_bytes(&rel, "HUD localisation")
            .map_err(|error| error.to_string())?;
        record_decode_damage(decode_damaged, &rel, &bytes)?;
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
    let candidates = crate::assets::texture_stem_fallback_candidates(&normalized);
    if candidates.is_empty() {
        return Err(format!("missing HUD texture dependency {normalized}"));
    }
    // The engine's precedence between same-stem formats in different layers is
    // unproven, so an ambiguous stem refuses instead of guessing a payload.
    let mut hits = Vec::new();
    for candidate in &candidates {
        if let Some(resolved) = resolver
            .resolve_with_provenance(candidate)
            .map_err(|error| error.to_string())?
        {
            let identity = (
                normalize_path(&resolved.logical_path),
                resolved.source.ordinal,
                resolved.source.source_id.clone(),
            );
            if !hits.iter().any(|(seen, _)| *seen == identity) {
                hits.push((identity, resolved));
            }
        }
    }
    match hits.len() {
        0 => Err(format!(
            "missing HUD texture dependency {normalized} (also tried {})",
            candidates.join(", ")
        )),
        1 => key_from_resolved(&hits.swap_remove(0).1, authority),
        _ => Err(format!(
            "ambiguous HUD texture dependency {normalized}: {}",
            hits.into_iter()
                .map(|(identity, _)| identity.0)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
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
    let resolution_path = normalize_path(&resolved.logical_path);
    let normalized_game_relative_path = normalize_game_relative_path(&resolution_path)?;
    Ok(UsedContentKey {
        effective_vfs_manifest_sha256: authority.into(),
        normalized_game_relative_path,
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

fn record_decode_damage(
    damaged: &mut BTreeMap<String, String>,
    rel: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let losses = diagnose_text_decode(bytes);
    let events = destructive_event_count(&losses);
    if events == 0 {
        return Ok(());
    }
    let first = destructive_losses(&losses)
        .first()
        .map(ToString::to_string)
        .unwrap_or_default();
    damaged.insert(
        normalize_game_relative_path(rel)?,
        neutral(&format!("destructive_events={events}:first={first}")),
    );
    Ok(())
}

fn append_decode_damage_blockers(
    damaged: &BTreeMap<String, String>,
    bindings: &[ResolvedHudRoleBinding],
    missing: &mut Vec<String>,
) {
    if damaged.is_empty() {
        return;
    }
    let mut seen = BTreeSet::new();
    for binding in bindings {
        let path = binding.content.normalized_game_relative_path.as_str();
        let Some(summary) = damaged.get(path) else {
            continue;
        };
        if seen.insert(path) {
            missing.push(format!("decode_loss:{path}:{summary}"));
        }
    }
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
    if crate::is_lowercase_sha256(value) {
        Ok(())
    } else {
        Err("HUD source inventory authority is not lowercase SHA-256".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::RecordedContentLayer;
    use crate::engine_surface_roots::ENGINE_CONTROLLER_ROOTS;
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
            game.join("interface/equipmentdesigner.gui"),
            gui(&["countryequipmentdesignerview"]),
        )
        .unwrap();
        fs::write(
            game.join("interface/tankdesigner.gui"),
            gui(&["tank_designer_view"]),
        )
        .unwrap();
        fs::write(
            game.join("interface/planedesigner.gui"),
            gui(&["plane_designer_view"]),
        )
        .unwrap();
        fs::write(
            game.join("interface/divisiondesigner.gui"),
            gui(&["countrydivisiondesignerview"]),
        )
        .unwrap();
        fs::write(
            game.join("interface/equipmentupgradedesigner.gui"),
            gui(&["equipment_upgrade_designer_window"]),
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
    fn complete_engine_root_closure_uses_mod_winner_and_is_relocation_stable_twice() {
        let first = temp("first");
        let second = temp("second");
        let (game_a, mod_a) = fixture(&first, true);
        let (game_b, mod_b) = fixture(&second, true);
        let authority = "a".repeat(64);
        let emit = |game: &Path, mod_root: &Path| {
            let resolver =
                GameFileResolver::with_mod_paths(game, vec![mod_root.to_path_buf()]).unwrap();
            emit_effective_vfs_hud_role_index(&resolver, &authority, "english").unwrap()
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
            ENGINE_CONTROLLER_ROOTS.len()
        );
        let topbar = index_a1
            .winning_content
            .iter()
            .find(|row| row.normalized_game_relative_path == "interface/topbar.gui")
            .unwrap();
        assert_eq!(topbar.layer_kind, ContentLayerKind::Mod);
        assert_eq!(topbar.layer_id, "active_mod");
        index_a1.validate().unwrap();
        let mut omitted_controller = index_a1.clone();
        omitted_controller.closures.pop();
        assert!(omitted_controller
            .validate()
            .unwrap_err()
            .contains("does not cover every exact-a729 controller"));
        let mut unauthenticated_bindings = index_a1.clone();
        unauthenticated_bindings.closures[0].dependency_closure_sha256 = "f".repeat(64);
        assert!(unauthenticated_bindings
            .validate()
            .unwrap_err()
            .contains("does not authenticate its bindings"));
        let mut unexplained_incomplete = index_a1.clone();
        unexplained_incomplete.closures[0].complete_dependency_closure = false;
        assert!(unexplained_incomplete
            .validate()
            .unwrap_err()
            .contains("has no typed blocker"));
        assert!(index_a1.closures.iter().all(|closure| {
            closure.bindings.iter().all(|binding| {
                root_role_consumer(&closure.runtime_root_name, &binding.semantic_role)
                    .is_ok_and(|consumer| consumer.starts_with("hud_role/v1/root/"))
            })
        }));
        portable_test_fs_retry::remove_dir_all(first).unwrap();
        portable_test_fs_retry::remove_dir_all(second).unwrap();
    }

    #[test]
    fn replay_font_dependency_closure_uses_the_recorded_localisation_language() {
        let root = temp("selected_language_font");
        let (game, mod_root) = fixture(&root, true);
        fs::create_dir_all(game.join("gfx/fonts")).unwrap();
        fs::write(
            game.join("interface/core.gfx"),
            r#"bitmapfonts = {
                bitmapfont = {
                    name = "Alias"
                    path = "gfx/fonts/base"
                }
                bitmapfont_override = {
                    name = "Alias"
                    fontfiles = {
                        "gfx/fonts/japanese/alias"
                        "gfx/fonts/japanese/fallback"
                    }
                    languages = { "l_japanese" }
                }
            }"#,
        )
        .unwrap();
        fs::write(
            game.join("gfx/fonts/base.fnt"),
            "info face=\"base\" size=16\n\
             common lineHeight=16 base=12 scaleW=1 scaleH=1 pages=1\n\
             page id=0 file=\"base.dds\"\n\
             chars count=3\n\
             char id=49 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n\
             char id=50 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n\
             char id=51 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n",
        )
        .unwrap();
        fs::write(game.join("gfx/fonts/base.dds"), b"base").unwrap();
        fs::create_dir_all(game.join("gfx/fonts/japanese")).unwrap();
        fs::write(
            game.join("gfx/fonts/japanese/alias.fnt"),
            "info face=\"japanese\" size=16\n\
             common lineHeight=16 base=12 scaleW=1 scaleH=1 pages=1\n\
             page id=0 file=\"alias.dds\"\n\
             chars count=3\n\
             char id=49 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n\
             char id=50 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n\
             char id=51 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n",
        )
        .unwrap();
        fs::write(game.join("gfx/fonts/japanese/alias.dds"), b"japanese").unwrap();
        fs::write(
            game.join("gfx/fonts/japanese/fallback.fnt"),
            "info face=\"fallback\" size=16\n\
             common lineHeight=16 base=12 scaleW=1 scaleH=1 pages=1\n\
             page id=0 file=\"fallback.dds\"\n\
             chars count=3\n\
             char id=49 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n\
             char id=50 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n\
             char id=51 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n",
        )
        .unwrap();
        fs::write(game.join("gfx/fonts/japanese/fallback.dds"), b"fallback").unwrap();
        fs::write(
            mod_root.join("interface/topbar.gui"),
            r#"guiTypes = {
                containerWindowType = {
                    name = "topbar"
                    instantTextBoxType = {
                        name = "selected_language_font"
                        text = "123"
                        font = "Alias"
                    }
                }
            }"#,
        )
        .unwrap();

        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let authority = "e".repeat(64);
        let english = emit_effective_vfs_hud_role_index(&resolver, &authority, "english").unwrap();
        let japanese =
            emit_effective_vfs_hud_role_index(&resolver, &authority, "  L_JAPANESE  ").unwrap();
        let english_topbar = english
            .closures
            .iter()
            .find(|closure| closure.runtime_root_name == "topbar")
            .unwrap();
        let japanese_topbar = japanese
            .closures
            .iter()
            .find(|closure| closure.runtime_root_name == "topbar")
            .unwrap();
        assert!(
            english_topbar.complete_dependency_closure,
            "{:?}",
            english_topbar.missing_roles
        );
        assert!(
            japanese_topbar.complete_dependency_closure,
            "{:?}",
            japanese_topbar.missing_roles
        );
        let japanese_paths = japanese_topbar
            .bindings
            .iter()
            .map(|binding| binding.content.normalized_game_relative_path.as_str())
            .collect::<BTreeSet<_>>();
        assert!(japanese_paths.contains("interface/core.gfx"));
        assert!(japanese_paths.contains("gfx/fonts/base.fnt"));
        assert!(japanese_paths.contains("gfx/fonts/base.dds"));
        assert!(japanese_paths.contains("gfx/fonts/japanese/alias.fnt"));
        assert!(japanese_paths.contains("gfx/fonts/japanese/alias.dds"));
        assert!(japanese_paths.contains("gfx/fonts/japanese/fallback.fnt"));
        assert!(japanese_paths.contains("gfx/fonts/japanese/fallback.dds"));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replay_localisation_listing_uses_only_the_canonical_selected_directory() {
        let root = temp("selected_language_localisation");
        let (game, mod_root) = fixture(&root, true);
        fs::create_dir_all(game.join("localisation/japanese")).unwrap();
        fs::create_dir_all(game.join("localisation/french")).unwrap();
        fs::write(
            game.join("localisation/japanese/panel_l_japanese.yml"),
            b"l_japanese:\n JP_KEY:0 \"Japanese\"\n",
        )
        .unwrap();
        let mut unrelated_damage = b"l_french:\n FR_KEY:0 \"".to_vec();
        unrelated_damage.extend_from_slice(&[0xe2, 0x82]);
        fs::write(
            game.join("localisation/french/damaged_l_french.yml"),
            unrelated_damage,
        )
        .unwrap();

        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let mut damaged = BTreeMap::new();
        let sources = localisation_sources(&resolver, "  L_JAPANESE  ", &mut damaged).unwrap();
        assert_eq!(
            sources.get("jp_key").map(String::as_str),
            Some("localisation/japanese/panel_l_japanese.yml")
        );
        assert!(
            damaged.is_empty(),
            "unselected-language decode damage entered the selected closure: {damaged:?}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn localisation_sources_read_only_the_admission_snapshot() {
        let root = temp("admitted_localisation_snapshot");
        let game = root.join("game");
        let source = "localisation/english/panel_l_english.yml";
        fs::create_dir_all(game.join("localisation/english")).unwrap();
        fs::write(game.join(source), b"l_english:\n PANEL_KEY:0 \"Panel\"\n").unwrap();
        let (resolver, _admission) =
            crate::test_admission::admit_vanilla(&game, "l_english", &[source], "hud_loc");
        let mut damaged = BTreeMap::new();
        let sources = localisation_sources(&resolver, "l_english", &mut damaged).unwrap();
        assert_eq!(sources.get("panel_key").map(String::as_str), Some(source));
        let (_, snapshot) = resolver
            .resolve_dir_files_recursive("localisation/english", Some("yml"))
            .unwrap()
            .remove(0);
        assert!(!snapshot.starts_with(&game), "admitted listing names the live file");
        crate::test_admission::overwrite_readonly(
            &snapshot,
            b"l_english:\n PANEL_KEY:0 \"Tampered\"\n",
        );
        let error = localisation_sources(&resolver, "l_english", &mut damaged).unwrap_err();
        assert!(
            error.contains("immutable admitted content snapshot changed"),
            "{error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uppercase_authored_texture_emits_normalized_content_identity() {
        let root = temp("uppercase_identity");
        let (game, mod_root) = fixture(&root, true);
        fs::create_dir_all(game.join("gfx/interface")).unwrap();
        fs::write(
            game.join("gfx/interface/WT_anim_strip.dds"),
            b"uppercase-authored texture",
        )
        .unwrap();
        fs::write(
            game.join("interface/uppercase_texture.gfx"),
            r#"spriteTypes = {
                spriteType = {
                    name = GFX_uppercase_identity
                    texturefile = "gfx/interface/WT_anim_strip.dds"
                }
            }"#,
        )
        .unwrap();
        fs::write(
            mod_root.join("interface/topbar.gui"),
            r#"guiTypes = {
                containerWindowType = {
                    name = "topbar"
                    iconType = {
                        name = "uppercase_identity"
                        spriteType = "GFX_uppercase_identity"
                    }
                }
            }"#,
        )
        .unwrap();

        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index =
            emit_effective_vfs_hud_role_index(&resolver, &"c".repeat(64), "english").unwrap();
        let content = index
            .winning_content
            .iter()
            .find(|content| {
                content.normalized_game_relative_path == "gfx/interface/wt_anim_strip.dds"
            })
            .expect("uppercase-authored texture identity");
        assert_eq!(
            portable_content_manifest::normalize_game_relative_path(
                &content.normalized_game_relative_path
            ),
            Ok(content.normalized_game_relative_path.clone())
        );
        content.validate().unwrap();
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uppercase_damaged_bound_gui_withdraws_completeness() {
        let root = temp("uppercase_decode_damage");
        let (game, mod_root) = fixture(&root, true);
        portable_test_fs_retry::remove_file(mod_root.join("interface/topbar.gui")).unwrap();
        let mut damaged = [
            b"\xef\xbb\xbf".as_slice(),
            br#"guiTypes = {
                containerWindowType = {
                    name = "topbar"
                }
            }
            # destructive decode damage: "#,
        ]
        .concat();
        damaged.extend_from_slice(&[0xe2, 0x82]);
        damaged.push(b'\n');
        assert_ne!(
            destructive_event_count(&diagnose_text_decode(&damaged)),
            0,
            "fixture must carry real destructive decode damage"
        );
        fs::write(mod_root.join("interface/TopBar.gui"), &damaged).unwrap();

        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index =
            emit_effective_vfs_hud_role_index(&resolver, &"d".repeat(64), "english").unwrap();
        let topbar = index
            .closures
            .iter()
            .find(|closure| closure.runtime_root_name.eq_ignore_ascii_case("topbar"))
            .expect("topbar closure");
        assert!(
            !topbar.complete_dependency_closure,
            "uppercase damaged GUI must withdraw closure completeness"
        );
        assert!(
            topbar.missing_roles.iter().any(
                |role| role.starts_with("decode_loss:interface/topbar.gui:destructive_events=")
            ),
            "normalized decode-loss blocker missing: {:?}",
            topbar.missing_roles
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_engine_root_makes_only_that_root_incomplete() {
        let root = temp("missing_core");
        let (game, mod_root) = fixture(&root, true);
        portable_test_fs_retry::remove_file(game.join("interface/minimap.gui")).unwrap();
        let authority = "a".repeat(64);
        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index = emit_effective_vfs_hud_role_index(&resolver, &authority, "english").unwrap();
        assert_eq!(
            index
                .closures
                .iter()
                .filter(|closure| closure.complete_dependency_closure)
                .count(),
            ENGINE_CONTROLLER_ROOTS.len() - 1
        );
        assert!(index.closures.iter().any(|closure| {
            closure.runtime_root_name == "mini_map_interface"
                && !closure.complete_dependency_closure
                && closure
                    .missing_roles
                    .iter()
                    .any(|role| role == "compose_hud_root:mini_map_interface:undefined_gui_type")
        }));
        assert_eq!(
            index
                .closures
                .iter()
                .filter(|closure| !closure.complete_dependency_closure)
                .count(),
            1
        );
        index.validate().unwrap();
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gfx_relying_on_the_engine_tolerated_eof_close_still_closes_hud_roots() {
        let root = temp("tolerated_eof_gfx");
        let (game, mod_root) = fixture(&root, true);
        fs::create_dir_all(game.join("gfx/interface")).unwrap();
        fs::write(game.join("gfx/interface/tolerated_eof.dds"), b"eof").unwrap();
        // Shipped-vanilla shape (cf. interface/powerbalanceview.gfx): the outer
        // block never closes and the last non-whitespace byte is the final
        // entry's closing brace.
        fs::write(
            game.join("interface/tolerated_eof.gfx"),
            r#"spriteTypes = {
                spriteType = {
                    name = GFX_tolerated_eof
                    texturefile = "gfx/interface/tolerated_eof.dds"
                }
            "#,
        )
        .unwrap();
        fs::write(
            game.join("interface/core.gfx"),
            r#"bitmapfonts = {
                bitmapfont = {
                    name = "Alias"
                    path = "gfx/fonts/base"
                }
            }"#,
        )
        .unwrap();
        fs::create_dir_all(game.join("gfx/fonts")).unwrap();
        fs::write(
            game.join("gfx/fonts/base.fnt"),
            "info face=\"base\" size=16\n\
             common lineHeight=16 base=12 scaleW=1 scaleH=1 pages=1\n\
             page id=0 file=\"base.dds\"\n\
             chars count=3\n\
             char id=49 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n\
             char id=50 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n\
             char id=51 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n",
        )
        .unwrap();
        fs::write(game.join("gfx/fonts/base.dds"), b"base").unwrap();
        fs::write(
            mod_root.join("interface/topbar.gui"),
            r#"guiTypes = {
                containerWindowType = {
                    name = "topbar"
                    iconType = {
                        name = "tolerated_eof_sprite"
                        spriteType = "GFX_tolerated_eof"
                    }
                    instantTextBoxType = {
                        name = "tolerated_eof_text"
                        text = "123"
                        font = "Alias"
                    }
                }
            }"#,
        )
        .unwrap();

        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index =
            emit_effective_vfs_hud_role_index(&resolver, &"f".repeat(64), "english").unwrap();
        let topbar = index
            .closures
            .iter()
            .find(|closure| closure.runtime_root_name == "topbar")
            .unwrap();
        assert!(
            topbar.complete_dependency_closure,
            "{:?}",
            topbar.missing_roles
        );
        let paths = topbar
            .bindings
            .iter()
            .map(|binding| binding.content.normalized_game_relative_path.as_str())
            .collect::<BTreeSet<_>>();
        assert!(paths.contains("interface/core.gfx"));
        assert!(paths.contains("gfx/interface/tolerated_eof.dds"));
        assert!(paths.contains("gfx/fonts/base.fnt"));
        assert_eq!(
            index
                .closures
                .iter()
                .filter(|closure| closure.complete_dependency_closure)
                .count(),
            ENGINE_CONTROLLER_ROOTS.len()
        );
        index.validate().unwrap();
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn texture_declared_png_resolves_through_the_engine_stem_fallback() {
        let root = temp("png_declared_dds_file");
        let (game, mod_root) = fixture(&root, true);
        fs::create_dir_all(game.join("gfx/interface")).unwrap();
        // Shipped-vanilla shape (cf. gfx/interface/divisiondesigner/
        // division_icon_bg_transp): the .gfx declares a .png that does not
        // exist and only the same-stem .dds ships.
        fs::write(game.join("gfx/interface/stem_fallback.dds"), b"dds").unwrap();
        fs::write(
            game.join("interface/stem_fallback.gfx"),
            r#"spriteTypes = {
                spriteType = {
                    name = GFX_stem_fallback
                    texturefile = "gfx/interface/stem_fallback.png"
                }
            }"#,
        )
        .unwrap();
        fs::write(
            mod_root.join("interface/topbar.gui"),
            r#"guiTypes = {
                containerWindowType = {
                    name = "topbar"
                    iconType = {
                        name = "stem_fallback_sprite"
                        spriteType = "GFX_stem_fallback"
                    }
                }
            }"#,
        )
        .unwrap();

        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index =
            emit_effective_vfs_hud_role_index(&resolver, &"a".repeat(64), "english").unwrap();
        let topbar = index
            .closures
            .iter()
            .find(|closure| closure.runtime_root_name == "topbar")
            .unwrap();
        assert!(
            topbar.complete_dependency_closure,
            "{:?}",
            topbar.missing_roles
        );
        assert!(topbar.bindings.iter().any(|binding| {
            binding.content.normalized_game_relative_path == "gfx/interface/stem_fallback.dds"
        }));
        index.validate().unwrap();
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ambiguous_texture_stem_withdraws_closure_completeness() {
        let root = temp("ambiguous_stem");
        let (game, mod_root) = fixture(&root, true);
        fs::create_dir_all(game.join("gfx/interface")).unwrap();
        fs::write(game.join("gfx/interface/stem_ambiguous.dds"), b"dds").unwrap();
        fs::write(game.join("gfx/interface/stem_ambiguous.tga"), b"tga").unwrap();
        fs::write(
            game.join("interface/stem_ambiguous.gfx"),
            r#"spriteTypes = {
                spriteType = {
                    name = GFX_stem_ambiguous
                    texturefile = "gfx/interface/stem_ambiguous.png"
                }
            }"#,
        )
        .unwrap();
        fs::write(
            mod_root.join("interface/topbar.gui"),
            r#"guiTypes = {
                containerWindowType = {
                    name = "topbar"
                    iconType = {
                        name = "stem_ambiguous_sprite"
                        spriteType = "GFX_stem_ambiguous"
                    }
                }
            }"#,
        )
        .unwrap();

        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index =
            emit_effective_vfs_hud_role_index(&resolver, &"b".repeat(64), "english").unwrap();
        let topbar = index
            .closures
            .iter()
            .find(|closure| closure.runtime_root_name == "topbar")
            .unwrap();
        assert!(
            !topbar.complete_dependency_closure,
            "an unproven same-stem precedence must not be guessed into a binding"
        );
        assert!(
            topbar
                .missing_roles
                .iter()
                .any(|role| role.starts_with("sprite_texture:")
                    && role.contains("ambiguous HUD texture dependency")),
            "{:?}",
            topbar.missing_roles
        );
        index.validate().unwrap();
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn modded_designer_root_at_a_different_path_still_closes() {
        let root = temp("designer");
        let (game, mod_root) = fixture(&root, true);
        fs::write(
            mod_root.join("interface/tankdesigner.gui"),
            "guiTypes = {}\n",
        )
        .unwrap();
        fs::write(
            mod_root.join("interface/modded_tank_designer_location.gui"),
            gui(&["tank_designer_view"]),
        )
        .unwrap();
        let authority = "b".repeat(64);
        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index = emit_effective_vfs_hud_role_index(&resolver, &authority, "english").unwrap();
        let tank = index
            .closures
            .iter()
            .find(|closure| closure.runtime_root_name == "tank_designer_view")
            .unwrap();
        assert!(tank.complete_dependency_closure);
        assert!(tank.missing_roles.is_empty());
        assert!(tank.bindings.iter().any(|binding| {
            binding.content.normalized_game_relative_path
                == "interface/modded_tank_designer_location.gui"
        }));
        index.validate().unwrap();
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_root_uses_case_sensitive_a729_parse_order_and_last_definition_in_file() {
        let root = temp("duplicate_root");
        let (game, mod_root) = fixture(&root, true);
        fs::write(
            mod_root.join("interface/planedesigner.gui"),
            "guiTypes = {}\n",
        )
        .unwrap();
        fs::write(
            mod_root.join("interface/Z_upper_plane_designer.gui"),
            gui(&["plane_designer_view"]),
        )
        .unwrap();
        fs::write(
            mod_root.join("interface/a_lower_plane_designer.gui"),
            r#"guiTypes = {
                containerWindowType = {
                    name = "plane_designer_view"
                    iconType = {
                        name = "losing_duplicate"
                        spriteType = "GFX_missing_if_first_definition_survives"
                    }
                }
                containerWindowType = {
                    name = "plane_designer_view"
                }
            }"#,
        )
        .unwrap();
        let authority = "c".repeat(64);
        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index = emit_effective_vfs_hud_role_index(&resolver, &authority, "english").unwrap();
        let plane = index
            .closures
            .iter()
            .find(|closure| closure.runtime_root_name == "plane_designer_view")
            .unwrap();
        assert!(
            plane.complete_dependency_closure,
            "{:?}",
            plane.missing_roles
        );
        assert!(plane.missing_roles.is_empty());
        assert!(plane.bindings.iter().any(|binding| {
            binding.content.normalized_game_relative_path == "interface/a_lower_plane_designer.gui"
                && binding.content.layer_kind == ContentLayerKind::Mod
        }));
        index.validate().unwrap();
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_relative_path_override_uses_the_mod_file_before_root_registration() {
        let root = temp("same_path_override");
        let (game, mod_root) = fixture(&root, true);
        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index =
            emit_effective_vfs_hud_role_index(&resolver, &"c".repeat(64), "english").unwrap();
        let topbar = index
            .closures
            .iter()
            .find(|closure| closure.runtime_root_name == "topbar")
            .unwrap();
        assert!(
            topbar.complete_dependency_closure,
            "{:?}",
            topbar.missing_roles
        );
        let gui = topbar
            .bindings
            .iter()
            .find(|binding| matches!(binding.semantic_role, HudSemanticRole::GuiRoot { .. }))
            .unwrap();
        assert_eq!(
            gui.content.normalized_game_relative_path,
            "interface/topbar.gui"
        );
        assert_eq!(gui.content.layer_kind, ContentLayerKind::Mod);
        assert_eq!(gui.content.layer_id, "active_mod");
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_parsed_gui_root_stays_complete_and_reports_the_recovery() {
        let root = temp("recovery_root");
        let (game, mod_root) = fixture(&root, true);
        fs::write(
            mod_root.join("interface/tankdesigner.gui"),
            "guiTypes = { containerWindowType = { name = \"tank_designer_view\" }",
        )
        .unwrap();
        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let index =
            emit_effective_vfs_hud_role_index(&resolver, &"c".repeat(64), "english").unwrap();
        let tank = index
            .closures
            .iter()
            .find(|closure| closure.runtime_root_name == "tank_designer_view")
            .unwrap();
        assert!(tank.complete_dependency_closure, "{:?}", tank.missing_roles);
        assert!(index
            .recovery_parsed_gui_files
            .iter()
            .any(|file| file.starts_with("interface/tankdesigner.gui (")));
        index.validate().unwrap();
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn engine_closed_set_does_not_synthesise_a_controller_for_a_registered_mod_root() {
        let root = temp("mod_only_designer");
        let (game, mod_root) = fixture(&root, true);
        fs::write(
            mod_root.join("interface/mod_only_designer.gui"),
            gui(&["my_fourth_designer_view"]),
        )
        .unwrap();
        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root]).unwrap();
        let catalog = load_effective_definition_catalog(
            resolver
                .resolve_dir_files_recursive("interface", Some("gui"))
                .unwrap()
                .iter()
                .map(|(rel, path)| ResolvedGuiDefinitionFile::new(rel.clone(), std::fs::read(path).unwrap()))
                .collect(),
        )
        .unwrap();
        assert!(catalog
            .effective_roots
            .iter()
            .any(|root| root.root.name == "my_fourth_designer_view" && root.admitted));
        let index =
            emit_effective_vfs_hud_role_index(&resolver, &"d".repeat(64), "english").unwrap();
        assert!(!index
            .closures
            .iter()
            .any(|closure| closure.runtime_root_name == "my_fourth_designer_view"));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

}
