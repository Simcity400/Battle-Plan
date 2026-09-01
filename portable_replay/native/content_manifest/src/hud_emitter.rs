use crate::authored_ui::{
    canonical_localisation_language, engine_recursive_path_cmp, safe_language_directory,
};
use crate::effective_vfs::EffectiveVfs;
use crate::gfx::{a729_effect_shader_path, xui07_builtin_shader_includes, SpriteRegistry};
use crate::gui::{parse_gui_document, GuiElement};
use crate::hud_role::{
    validate_sha256, AuthenticatedHudSurfaceClosure, EffectiveVfsHudRoleIndexDocument,
    HudSemanticRole, HudSurface, HudSurfaceActivationFrame, ResolvedHudRoleBinding,
};
use crate::paradox_text::{
    decode_text, destructive_event_count, destructive_losses, diagnose_text_decode,
    engine_tolerated_outer_eof, parse_bytes, Value,
};
use crate::{normalize_game_relative_path, UsedContentKey};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedHudFile {
    pub content: UsedContentKey,
    pub bytes: Vec<u8>,
}

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
    validate_sha256(source_inventory_sha256, "HUD source inventory authority")?;
    let gui_files = vfs.list_recursive("interface", "gui")?;
    let gfx_files = vfs.list_recursive("interface", "gfx")?;
    let mut decode_damaged = BTreeMap::<String, String>::new();
    let localisation = localisation_sources(vfs, localisation_language, &mut decode_damaged)?;

    let mut roots = BTreeMap::<String, Vec<(String, GuiElement)>>::new();
    let mut gui_parse_failed = BTreeMap::new();
    for rel in gui_files {
        let Some(file) = vfs.resolve(&rel)? else {
            return Err(format!("HUD VFS listing could not resolve {rel}"));
        };
        validate_resolved(&file, &rel, source_inventory_sha256)?;
        record_decode_damage(&mut decode_damaged, &file);
        let document = parse_bytes(&file.bytes);
        if !document.recoveries.is_empty() && !engine_tolerated_outer_eof(&document) {
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
        record_decode_damage(&mut decode_damaged, &file);
        let document = parse_bytes(&file.bytes);
        if !document.recoveries.is_empty() && !engine_tolerated_outer_eof(&document) {
            gfx_parse_failed.insert(rel, decode_text(&file.bytes).to_ascii_lowercase());
            continue;
        }
        sprites.add_document(&file.content.normalized_game_relative_path, &document);
    }
    let mut requested_fonts = BTreeSet::new();
    for candidates in roots.values() {
        for (_, root) in candidates {
            let mut refs = RootRefs::default();
            collect_element_refs(root, &mut refs, &neutral(&root.name), 0);
            requested_fonts.extend(refs.fonts.into_iter().map(|(_, font)| font));
        }
    }
    let font_registry = authored_bitmap_font_dependency_registry_paths(
        vfs,
        &requested_fonts,
        localisation_language,
        source_inventory_sha256,
        &mut decode_damaged,
    )?;

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
                &font_registry,
                source_inventory_sha256,
                &neutral(root_name),
                &root,
                &mut bindings,
                &mut all_content,
                &mut missing,
                &mut decode_damaged,
            )?;
        }
        bindings.sort_by_cached_key(|binding| {
            (
                serde_json::to_string(&binding.semantic_role)
                    .expect("HudSemanticRole serializes infallibly (string-field enum)"),
                binding.content.normalized_game_relative_path.clone(),
            )
        });
        append_decode_damage_blockers(&decode_damaged, &bindings, &mut missing);
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
    font_registry: &BTreeMap<String, Result<AuthoredBitmapFontDependencyPaths, String>>,
    authority: &str,
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
        let effect_path = match a729_effect_shader_path(sprite.effect_file.as_deref()) {
            Ok(path) => path,
            Err(error) => {
                missing.push(format!("sprite_effect:{element}:{sprite_id}:{error}"));
                continue;
            }
        };
        match resolve_key(vfs, &effect_path, authority) {
            Ok(effect_key) => {
                bindings.push(ResolvedHudRoleBinding {
                    semantic_role: HudSemanticRole::SpriteEffect {
                        element: element.clone(),
                        sprite_id: sprite_id.clone(),
                        effect_token: neutral(&effect_path),
                    },
                    content: effect_key.clone(),
                });
                all_content.insert(effect_key.normalized_game_relative_path.clone(), effect_key);
            }
            Err(error) => {
                missing.push(format!(
                    "sprite_effect:{element}:{sprite_id}:{effect_path}:{error}"
                ));
                continue;
            }
        }
        if let Some(includes) = xui07_builtin_shader_includes(&effect_path) {
            for (include_index, include_path) in includes.iter().enumerate() {
                match resolve_key(vfs, include_path, authority) {
                    Ok(include_key) => {
                        bindings.push(ResolvedHudRoleBinding {
                            semantic_role: HudSemanticRole::SpriteEffectInclude {
                                element: element.clone(),
                                sprite_id: sprite_id.clone(),
                                include_slot: format!("include_{include_index}"),
                            },
                            content: include_key.clone(),
                        });
                        all_content.insert(
                            include_key.normalized_game_relative_path.clone(),
                            include_key,
                        );
                    }
                    Err(error) => missing.push(format!(
                        "sprite_effect_include:{element}:{sprite_id}:{include_path}:{error}"
                    )),
                }
            }
        } else {
            missing.push(format!(
                "sprite_effect_include_closure:{element}:{sprite_id}:{effect_path}:unsupported"
            ));
        }
    }
    for (element, font_id) in refs.fonts {
        match font_registry.get(&font_id) {
            Some(Ok(paths)) => {
                for (registration_index, path) in paths.registration_sources.iter().enumerate() {
                    append_binding(
                        vfs,
                        authority,
                        &path,
                        HudSemanticRole::DefineOrScriptedConsumer {
                            consumer: neutral(&format!(
                                "font_registration_{element}_{font_id}_{registration_index}"
                            )),
                            source_identity: neutral(&font_id),
                        },
                        bindings,
                        all_content,
                    )?;
                }
                for (variant_index, variant) in paths.variants.iter().enumerate() {
                    for (descriptor_index, descriptor) in
                        variant.ordered_descriptors.iter().enumerate()
                    {
                        append_binding(
                            vfs,
                            authority,
                            &descriptor.descriptor,
                            HudSemanticRole::Font {
                                element: element.clone(),
                                font_id: format!(
                                    "{font_id}_variant_{variant_index}_descriptor_{descriptor_index}"
                                ),
                            },
                            bindings,
                            all_content,
                        )?;
                        for (page_index, path) in descriptor.pages.iter().enumerate() {
                            append_binding(
                                vfs,
                                authority,
                                &path,
                                HudSemanticRole::FontTexture {
                                    element: element.clone(),
                                    font_id: font_id.clone(),
                                    texture_slot: format!(
                                        "variant_{variant_index}_descriptor_{descriptor_index}_page_{page_index}"
                                    ),
                                },
                                bindings,
                                all_content,
                            )?;
                        }
                    }
                }
            }
            Some(Err(error)) => missing.push(format!("font:{element}:{font_id}:{error}")),
            None => missing.push(format!("font:{element}:{font_id}:unplanned")),
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
    damaged: &mut BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    let language_directory = safe_language_directory(language)?;
    let expected = canonical_localisation_language(language)?;
    let mut out = BTreeMap::new();
    for rel in vfs.list_recursive(&format!("localisation/{language_directory}"), "yml")? {
        let Some(file) = vfs.resolve(&rel)? else {
            continue;
        };
        record_decode_damage(damaged, &file);
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

#[cfg(test)]
#[derive(Clone, Debug)]
struct BitmapFontDependencyPaths {
    alias_source: Option<String>,
    descriptor: String,
    pages: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthoredBitmapFontDependencyPaths {
    pub(crate) registration_sources: Vec<String>,
    pub(crate) variants: Vec<AuthoredBitmapFontVariantDependencies>,
    pub(crate) selected_variant_index: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthoredBitmapFontVariantDependencies {
    pub(crate) ordered_languages: Vec<String>,
    pub(crate) ordered_descriptors: Vec<AuthoredBitmapFontDescriptorDependencies>,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthoredBitmapFontDescriptorDependencies {
    pub(crate) descriptor: String,
    pub(crate) pages: Vec<String>,
}

pub(crate) fn authored_bitmap_font_dependency_registry_paths(
    vfs: &impl HudContentVfs,
    symbolic_names: &BTreeSet<String>,
    localisation_language: &str,
    authority: &str,
    decode_damaged: &mut BTreeMap<String, String>,
) -> Result<BTreeMap<String, Result<AuthoredBitmapFontDependencyPaths, String>>, String> {
    let selected_language = canonical_localisation_language(localisation_language)?;
    let mut files = vfs.list_recursive("interface", "gfx")?;
    files.sort_by(|left, right| engine_recursive_path_cmp(left, right));
    let mut registration_sources = BTreeMap::<String, Vec<String>>::new();
    let mut pending = BTreeMap::<String, Vec<(Vec<String>, Vec<String>)>>::new();
    let mut failures = BTreeMap::<String, String>::new();
    for source in files {
        let Some(file) = vfs.resolve(&source)? else {
            continue;
        };
        validate_resolved(&file, &source, authority)?;
        record_decode_damage(decode_damaged, &file);
        let document = parse_bytes(&file.bytes);
        if !document.recoveries.is_empty() && !engine_tolerated_outer_eof(&document) {
            return Err(format!(
                "{source}: bitmap-font registry parse required recovery"
            ));
        }
        for container in document.root.values_ci("bitmapfonts") {
            for entry in container.fields() {
                let Some(kind) = entry.key.as_deref() else {
                    continue;
                };
                let is_base = kind.eq_ignore_ascii_case("bitmapfont");
                let is_override = kind.eq_ignore_ascii_case("bitmapfont_override");
                if !is_base && !is_override {
                    continue;
                }
                let name = portable_exact_single_scalar(&entry.value, "name")?;
                if !symbolic_names.contains(name) || failures.contains_key(name) {
                    continue;
                }
                let result = (|| {
                    let font_pending = pending.entry(name.to_string()).or_default();
                    if is_base {
                        if !font_pending.is_empty() {
                            return Err(format!(
                                "font {name:?} repeats its base CBitmapFont registration"
                            ));
                        }
                        let descriptors = portable_base_fontfiles(&entry.value)?;
                        if descriptors.is_empty() {
                            return Err(format!(
                                "font {name:?} base registration has no descriptor"
                            ));
                        }
                        font_pending.push((Vec::new(), descriptors));
                    } else {
                        if font_pending.is_empty() {
                            return Err(format!(
                                "font override {name:?} appears before its base registration"
                            ));
                        }
                        if entry.value.values_ci("path").next().is_some()
                            || entry.value.values_ci("color").next().is_some()
                            || entry.value.values_ci("border_color").next().is_some()
                            || entry.value.values_ci("textcolors").next().is_some()
                        {
                            return Err(format!(
                                "font override {name:?} carries a field not accepted by SBitmapFontOverride"
                            ));
                        }
                        let languages = portable_scalar_list(
                            portable_exact_single_value(&entry.value, "languages")?,
                            "bitmapfont_override languages",
                        )?;
                        let descriptors = portable_scalar_list(
                            portable_exact_single_value(&entry.value, "fontfiles")?,
                            "bitmapfont_override fontfiles",
                        )?
                        .into_iter()
                        .map(portable_font_descriptor)
                        .collect::<Vec<_>>();
                        if descriptors.is_empty() {
                            return Err(format!("font override {name:?} has no descriptor"));
                        }
                        font_pending.push((languages, descriptors));
                    }
                    registration_sources
                        .entry(name.to_string())
                        .or_default()
                        .push(source.clone());
                    Ok::<_, String>(())
                })();
                if let Err(error) = result {
                    failures.insert(name.to_string(), error);
                }
            }
        }
    }
    let mut planned = BTreeMap::new();
    for symbolic_name in symbolic_names {
        if let Some(error) = failures.remove(symbolic_name) {
            planned.insert(symbolic_name.clone(), Err(error));
            continue;
        }
        let Some(font_pending) = pending.remove(symbolic_name) else {
            planned.insert(
                symbolic_name.clone(),
                Err(format!(
                    "font {symbolic_name:?} has no exact-case CBitmapFont registration"
                )),
            );
            continue;
        };
        let result = (|| {
            let selected_variant_index = font_pending
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, (languages, _))| {
                    languages
                        .iter()
                        .any(|language| language == &selected_language)
                })
                .map(|(index, _)| index)
                .unwrap_or(0);
            let mut variants = Vec::new();
            for (ordered_languages, descriptors) in font_pending {
                let mut ordered_descriptors = Vec::new();
                for descriptor in descriptors {
                    ordered_descriptors.push(portable_plan_descriptor(
                        vfs,
                        symbolic_name,
                        &descriptor,
                        authority,
                        decode_damaged,
                    )?);
                }
                variants.push(AuthoredBitmapFontVariantDependencies {
                    ordered_languages,
                    ordered_descriptors,
                });
            }
            Ok(AuthoredBitmapFontDependencyPaths {
                registration_sources: registration_sources
                    .remove(symbolic_name)
                    .unwrap_or_default(),
                variants,
                selected_variant_index,
            })
        })();
        planned.insert(symbolic_name.clone(), result);
    }
    Ok(planned)
}

fn portable_exact_single_value<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    // Do not inherit `Value::values_ci`'s shared input lifetime: the lookup
    let mut values = value.fields().iter().filter_map(|entry| {
        entry
            .key
            .as_deref()
            .filter(|candidate| candidate.eq_ignore_ascii_case(key))
            .map(|_| &entry.value)
    });
    let first = values
        .next()
        .ok_or_else(|| format!("bitmap-font field {key:?} is absent"))?;
    if values.next().is_some() {
        return Err(format!("bitmap-font field {key:?} repeats"));
    }
    Ok(first)
}

fn portable_exact_single_scalar<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    portable_exact_single_value(value, key)?
        .as_scalar()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("bitmap-font field {key:?} is not scalar"))
}

fn portable_scalar_list(value: &Value, label: &str) -> Result<Vec<String>, String> {
    let entries = value
        .as_object()
        .ok_or_else(|| format!("{label} is not an ordered scalar list"))?;
    entries
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(format!("{label} contains a labeled entry"));
            }
            entry
                .value
                .as_scalar()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{label} contains a non-scalar or blank entry"))
        })
        .collect()
}

fn portable_font_descriptor(path: String) -> String {
    let mut path = normalize_path(&path);
    if !path.to_ascii_lowercase().ends_with(".fnt") {
        path.push_str(".fnt");
    }
    path
}

fn portable_base_fontfiles(value: &Value) -> Result<Vec<String>, String> {
    let mut descriptors = Vec::new();
    for field in value.fields() {
        let Some(key) = field.key.as_deref() else {
            continue;
        };
        if key.eq_ignore_ascii_case("path") {
            let path = field
                .value
                .as_scalar()
                .ok_or("bitmapfont path is not an authored scalar")?;
            descriptors.push(portable_font_descriptor(path.to_string()));
        } else if key.eq_ignore_ascii_case("fontfiles") {
            descriptors.clear();
            descriptors.extend(
                portable_scalar_list(&field.value, "bitmapfont fontfiles")?
                    .into_iter()
                    .map(portable_font_descriptor),
            );
        }
    }
    Ok(descriptors)
}

fn portable_plan_descriptor(
    vfs: &impl HudContentVfs,
    symbolic_name: &str,
    descriptor: &str,
    authority: &str,
    decode_damaged: &mut BTreeMap<String, String>,
) -> Result<AuthoredBitmapFontDescriptorDependencies, String> {
    let resolved = vfs
        .resolve(descriptor)?
        .ok_or_else(|| format!("missing bitmap-font descriptor {descriptor}"))?;
    validate_resolved(&resolved, descriptor, authority)?;
    record_decode_damage(decode_damaged, &resolved);
    let stem = Path::new(descriptor)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(symbolic_name)
        .to_string();
    let parent = Path::new(descriptor)
        .parent()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    let parsed = bmfont_page_files(&resolved.bytes)?;
    if parsed.lossy_page_names != 0 {
        record_decode_damage_summary(
            decode_damaged,
            descriptor,
            parsed.lossy_page_names,
            "binary BMFont page name is not valid UTF-8; decoded with replacement characters",
        );
    }
    let mut pages = Vec::new();
    if parsed.pages.is_empty() {
        pages.push(
            first_resolved_font_page(vfs, inferred_font_page_candidates(&parent, &stem, 0))?
                .ok_or_else(|| format!("font {symbolic_name} has no inferred atlas page"))?,
        );
    } else {
        for (page_id, page) in parsed.pages {
            reject_unsafe_font_page(symbolic_name, page_id, &page)?;
            let declared = normalize_path(&page);
            let selected = match first_resolved_font_page(
                vfs,
                declared_font_page_candidates(&parent, &declared),
            )? {
                Some(path) => Some(path),
                None => first_resolved_font_page(
                    vfs,
                    inferred_font_page_candidates(&parent, &stem, page_id),
                )?,
            }
            .ok_or_else(|| {
                format!("bitmap-font page {page} and inferred {stem} page {page_id} are absent")
            })?;
            pages.push(selected);
        }
    }
    Ok(AuthoredBitmapFontDescriptorDependencies {
        descriptor: descriptor.to_string(),
        pages,
    })
}

#[cfg(test)]
fn bitmap_font_dependency_paths(
    vfs: &impl HudContentVfs,
    symbolic_name: &str,
    localisation_language: &str,
    authority: &str,
    decode_damaged: &mut BTreeMap<String, String>,
) -> Result<BitmapFontDependencyPaths, String> {
    if symbolic_name.trim().is_empty() {
        return Err("blank bitmap-font identity".into());
    }
    let direct = format!("gfx/fonts/{symbolic_name}.fnt");
    let direct_resolved = vfs.resolve(&direct)?;
    let (alias_source, resolved) = if let Some(resolved) = direct_resolved {
        (None, resolved)
    } else {
        let core = vfs
            .resolve("interface/core.gfx")?
            .ok_or_else(|| format!("font {symbolic_name} has no descriptor or core.gfx alias"))?;
        validate_resolved(&core, "interface/core.gfx", authority)?;
        let document = parse_bytes(&core.bytes);
        if !document.recoveries.is_empty() && !engine_tolerated_outer_eof(&document) {
            return Err("core.gfx font alias parse required recovery".into());
        }
        let resolved_language = canonical_localisation_language(localisation_language)?;
        let mut base = Vec::new();
        for block in document.root.values_ci("bitmapfonts") {
            for entry in block.fields() {
                let Some(kind) = entry.key.as_deref() else {
                    continue;
                };
                let is_override = kind.eq_ignore_ascii_case("bitmapfont_override");
                if !is_override && !kind.eq_ignore_ascii_case("bitmapfont") {
                    continue;
                }
                if entry
                    .value
                    .first_scalar_ci("name")
                    .is_some_and(|name| name.eq_ignore_ascii_case(symbolic_name))
                {
                    if is_override {
                        if font_override_applies(&entry.value, &resolved_language)? {
                            return Err(format!(
                                "font {symbolic_name} has an applicable bitmapfont_override whose registration-population law is unproved"
                            ));
                        }
                        continue;
                    }
                    if let Some(path) = entry.value.first_scalar_ci("path") {
                        let mut path = normalize_path(path);
                        if !path.to_ascii_lowercase().ends_with(".fnt") {
                            path.push_str(".fnt");
                        }
                        base.push(path);
                    }
                }
            }
        }
        let mut aliases = base;
        aliases.sort();
        aliases.dedup();
        if aliases.len() != 1 {
            return Err(format!(
                "font {symbolic_name} has {} unresolved base aliases",
                aliases.len()
            ));
        }
        let descriptor = aliases.remove(0);
        drop(document);
        drop(core);
        let resolved = vfs
            .resolve(&descriptor)?
            .ok_or_else(|| format!("missing bitmap-font descriptor {descriptor}"))?;
        (Some("interface/core.gfx".to_string()), resolved)
    };
    let descriptor = resolved.content.normalized_game_relative_path.clone();
    validate_resolved(&resolved, &descriptor, authority)?;
    record_decode_damage(decode_damaged, &resolved);
    let stem = Path::new(&resolved.content.normalized_game_relative_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(symbolic_name)
        .to_string();
    let parent = Path::new(&resolved.content.normalized_game_relative_path)
        .parent()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    let parsed = bmfont_page_files(&resolved.bytes)?;
    if parsed.lossy_page_names != 0 {
        record_decode_damage_summary(
            decode_damaged,
            &resolved.content.normalized_game_relative_path,
            parsed.lossy_page_names,
            "binary BMFont page name is not valid UTF-8; decoded with replacement characters",
        );
    }
    let page_files = parsed.pages;
    // release descriptor bytes before any atlas candidate is materialized.
    drop(resolved);
    let mut pages = Vec::new();
    if page_files.is_empty() {
        pages.push(
            first_resolved_font_page(vfs, inferred_font_page_candidates(&parent, &stem, 0))?
                .ok_or_else(|| format!("font {symbolic_name} has no inferred atlas page"))?,
        );
    } else {
        for (id, page) in page_files {
            reject_unsafe_font_page(symbolic_name, id, &page)?;
            let declared = normalize_path(&page);
            let selected = match first_resolved_font_page(
                vfs,
                declared_font_page_candidates(&parent, &declared),
            )? {
                Some(path) => Some(path),
                None => first_resolved_font_page(
                    vfs,
                    inferred_font_page_candidates(&parent, &stem, id),
                )?,
            };
            pages.push(selected.ok_or_else(|| {
                format!("bitmap-font page {page} and inferred {stem} page {id} are absent")
            })?);
        }
    }
    pages.sort();
    pages.dedup();
    Ok(BitmapFontDependencyPaths {
        alias_source,
        descriptor,
        pages,
    })
}

#[cfg(test)]
fn font_override_applies(entry: &Value, resolved_language: &str) -> Result<bool, String> {
    let Some(languages) = entry.first_value_ci("languages") else {
        return Ok(true);
    };
    let entries = languages
        .as_object()
        .ok_or("bitmapfont_override languages is not an unlabeled scalar list")?;
    if entries.is_empty()
        || entries
            .iter()
            .any(|entry| entry.key.is_some() || entry.value.as_scalar().is_none())
    {
        return Err("bitmapfont_override languages is not an unlabeled scalar list".into());
    }
    Ok(entries
        .iter()
        .filter_map(|entry| entry.value.as_scalar())
        .any(|language| language == resolved_language))
}

const LEGACY_FONT_ROOT: &str = "gfx/fonts";

fn reject_unsafe_font_page(font: &str, id: u32, page: &str) -> Result<(), String> {
    let normalized = page.replace('\\', "/");
    let raw = Path::new(page);
    let normalized_path = Path::new(&normalized);
    let unsafe_component = |path: &Path| {
        path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    };
    if raw.is_absolute()
        || normalized_path.is_absolute()
        || page.starts_with('/')
        || page.starts_with('\\')
        || page.contains(':')
        || unsafe_component(raw)
        || unsafe_component(normalized_path)
    {
        return Err(format!(
            "font {font} page {id} has unsafe declared path {page}"
        ));
    }
    Ok(())
}

fn declared_font_page_candidates(parent: &str, declared: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let explicit = declared
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("gfx/"));
    if !explicit && !parent.is_empty() {
        candidates.push(normalize_path(&format!("{parent}/{declared}")));
    }
    candidates.push(if declared.contains('/') {
        declared.to_string()
    } else {
        normalize_path(&format!("{LEGACY_FONT_ROOT}/{declared}"))
    });
    dedup_font_candidates(&mut candidates);
    candidates
}

fn inferred_font_page_candidates(parent: &str, stem: &str, id: u32) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut push_dir = |directory: &str| {
        if id == 0 {
            candidates.push(normalize_path(&format!("{directory}/{stem}.dds")));
            candidates.push(normalize_path(&format!("{directory}/{stem}.tga")));
        }
        candidates.push(normalize_path(&format!("{directory}/{stem}_{id}.dds")));
        candidates.push(normalize_path(&format!("{directory}/{stem}_{id}.tga")));
    };
    if !parent.is_empty() {
        push_dir(parent);
    }
    push_dir(LEGACY_FONT_ROOT);
    dedup_font_candidates(&mut candidates);
    candidates
}

fn dedup_font_candidates(candidates: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    candidates.retain(|candidate| seen.insert(candidate.clone()));
}

fn first_resolved_font_page(
    vfs: &impl HudContentVfs,
    candidates: Vec<String>,
) -> Result<Option<String>, String> {
    for candidate in candidates {
        if vfs.resolve(&candidate)?.is_some() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

#[derive(Debug, Default)]
struct BmfontPages {
    pages: BTreeMap<u32, String>,
    lossy_page_names: usize,
}

fn bmfont_page_files(bytes: &[u8]) -> Result<BmfontPages, String> {
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
                let mut out = BmfontPages::default();
                for (index, page) in block
                    .split(|byte| *byte == 0)
                    .filter(|page| !page.is_empty())
                    .enumerate()
                {
                    if std::str::from_utf8(page).is_err() {
                        out.lossy_page_names += 1;
                    }
                    out.pages
                        .insert(index as u32, String::from_utf8_lossy(page).into_owned());
                }
                return Ok(out);
            }
            position += size;
        }
        return Ok(BmfontPages::default());
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
    Ok(BmfontPages {
        pages,
        lossy_page_names: 0,
    })
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

fn record_decode_damage(damaged: &mut BTreeMap<String, String>, file: &ResolvedHudFile) {
    let losses = diagnose_text_decode(&file.bytes);
    let destructive = destructive_losses(&losses);
    let Some(first) = destructive.first() else {
        return;
    };
    record_decode_damage_summary(
        damaged,
        &file.content.normalized_game_relative_path,
        destructive_event_count(&losses),
        &first.to_string(),
    );
}

fn record_decode_damage_summary(
    damaged: &mut BTreeMap<String, String>,
    path: &str,
    events: usize,
    first: &str,
) {
    if events == 0 {
        return;
    }
    let fragment = neutral(&format!("destructive_events={events}:first={first}"));
    match damaged.get_mut(path) {
        Some(existing) => {
            if !existing.contains(fragment.as_str()) {
                existing.push('|');
                existing.push_str(&fragment);
            }
        }
        None => {
            damaged.insert(path.to_string(), fragment);
        }
    }
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

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud_role::HudSurfaceActivation;
    use crate::ContentLayerKind;

    fn authority() -> String {
        "a".repeat(64)
    }

    fn resolved_file(path: &str, bytes: &[u8]) -> ResolvedHudFile {
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: authority(),
            normalized_game_relative_path: path.to_string(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: hex_sha256(bytes),
            payload_sha256: hex_sha256(bytes),
            payload_bytes: bytes.len() as u64,
        };
        content.validate().expect("fixture content key");
        ResolvedHudFile {
            content,
            bytes: bytes.to_vec(),
        }
    }

    struct MapVfs {
        files: BTreeMap<String, Vec<u8>>,
    }

    impl MapVfs {
        fn new(entries: &[(&str, &[u8])]) -> Self {
            Self {
                files: entries
                    .iter()
                    .map(|(path, bytes)| (path.to_string(), bytes.to_vec()))
                    .collect(),
            }
        }
    }

    impl HudContentVfs for MapVfs {
        fn list_recursive(&self, directory: &str, extension: &str) -> Result<Vec<String>, String> {
            let prefix = format!("{directory}/");
            let suffix = format!(".{extension}");
            Ok(self
                .files
                .keys()
                .filter(|path| path.starts_with(&prefix) && path.ends_with(&suffix))
                .cloned()
                .collect())
        }

        fn resolve(&self, logical_path: &str) -> Result<Option<ResolvedHudFile>, String> {
            let normalized = normalize_path(logical_path);
            Ok(self
                .files
                .get(&normalized)
                .map(|bytes| resolved_file(&normalized, bytes)))
        }
    }

    fn topbar_activation(root_name: &str, status_flags: u32) -> HudSurfaceActivationFrame {
        HudSurfaceActivationFrame {
            frame_hour: 40,
            status_flags,
            observation_sequence: 1,
            active_surfaces: vec![HudSurfaceActivation {
                surface: HudSurface::Topbar,
                runtime_root_name: root_name.to_string(),
            }],
        }
    }

    #[test]
    fn emitter_round_trip_binds_the_unique_gui_root() {
        let vfs = MapVfs::new(&[(
            "interface/fixture.gui",
            b"guiTypes = {\n\tcontainerWindowType = {\n\t\tname = fixture_topbar_hud\n\t}\n}\n"
                .as_slice(),
        )]);
        let document = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 0),
            &authority(),
            "english",
        )
        .expect("emit");
        assert_eq!(document.closures.len(), 1);
        let closure = &document.closures[0];
        assert_eq!(closure.surface, HudSurface::Topbar);
        assert_eq!(closure.bindings.len(), 1);
        assert!(matches!(
            closure.bindings[0].semantic_role,
            HudSemanticRole::GuiRoot { .. }
        ));
        assert!(!closure.complete_dependency_closure);
        assert_eq!(closure.missing_roles.len(), 3);
        assert!(closure
            .missing_roles
            .iter()
            .all(|role| role.starts_with("compose_hud_activation:")));
        assert_eq!(document.winning_content.len(), 1);
        assert_eq!(
            document.winning_content[0].normalized_game_relative_path,
            "interface/fixture.gui"
        );
    }

    #[test]
    fn emitter_binds_default_effect_shader_and_include_closure() {
        let vfs = MapVfs::new(&[
            (
                "interface/fixture.gui",
                br#"guiTypes = {
                    containerWindowType = {
                        name = fixture_topbar_hud
                        iconType = { name = icon spriteType = GFX_fixture }
                    }
                }"#,
            ),
            (
                "interface/fixture.gfx",
                br#"spriteTypes = {
                    spriteType = {
                        name = GFX_fixture
                        textureFile = "gfx/interface/fixture.dds"
                    }
                }"#,
            ),
            ("gfx/interface/fixture.dds", b"texture"),
            ("gfx/fx/buttonstate.shader", b"shader"),
            ("gfx/fx/buttonstate.fxh", b"button include"),
            ("gfx/fx/sprite_animation.fxh", b"animation include"),
        ]);
        let document = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 0),
            &authority(),
            "english",
        )
        .expect("emit default effect closure");
        let bindings = &document.closures[0].bindings;
        assert!(bindings.iter().any(|binding| matches!(
            binding.semantic_role,
            HudSemanticRole::SpriteEffect { .. }
        ) && binding.content.normalized_game_relative_path
            == "gfx/fx/buttonstate.shader"));
        let include_bindings = bindings
            .iter()
            .filter(|binding| {
                matches!(
                    binding.semantic_role,
                    HudSemanticRole::SpriteEffectInclude { .. }
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(include_bindings.len(), 6);
        let mut include_consumers = BTreeMap::<&str, BTreeSet<String>>::new();
        let mut include_content = BTreeMap::<&str, BTreeSet<UsedContentKey>>::new();
        for binding in include_bindings {
            let path = binding.content.normalized_game_relative_path.as_str();
            include_consumers.entry(path).or_default().insert(
                serde_json::to_string(&binding.semantic_role)
                    .expect("include semantic role serializes"),
            );
            include_content
                .entry(path)
                .or_default()
                .insert(binding.content.clone());
        }
        assert_eq!(
            include_consumers.keys().copied().collect::<BTreeSet<_>>(),
            BTreeSet::from(["gfx/fx/buttonstate.fxh", "gfx/fx/sprite_animation.fxh"])
        );
        assert!(include_consumers
            .values()
            .all(|consumers| consumers.len() == 3));
        assert!(include_content
            .values()
            .all(|content_identities| content_identities.len() == 1));
    }

    #[test]
    fn destroyed_bytes_block_closure_completeness_without_dropping_the_binding() {
        let mut bytes =
            b"\xef\xbb\xbfguiTypes = {\n\tcontainerWindowType = {\n\t\tname = fixture_topbar_hud\n\t\ttext = \"".to_vec();
        bytes.push(0xff);
        bytes.extend_from_slice(b"\"\n\t}\n}\n");

        assert!(
            parse_bytes(&bytes).recoveries.is_empty(),
            "the parse must be clean, or this test proves nothing about the decode channel"
        );

        let vfs = MapVfs::new(&[("interface/fixture.gui", bytes.as_slice())]);
        let document = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 0),
            &authority(),
            "english",
        )
        .expect("emit");
        let closure = &document.closures[0];

        assert_eq!(closure.bindings.len(), 1);
        assert_eq!(document.winning_content.len(), 1);

        assert!(!closure.complete_dependency_closure);
        let decode_roles = closure
            .missing_roles
            .iter()
            .filter(|role| role.starts_with("decode_loss:"))
            .collect::<Vec<_>>();
        assert_eq!(decode_roles.len(), 1, "{:?}", closure.missing_roles);
        assert!(
            decode_roles[0].contains("interface/fixture.gui")
                && decode_roles[0].contains("destructive_events=1")
                && decode_roles[0].contains("invalid UTF-8"),
            "{}",
            decode_roles[0]
        );
    }

    #[test]
    fn latin1_encoded_content_is_not_treated_as_decode_damage() {
        let mut bytes =
            b"guiTypes = {\n\tcontainerWindowType = {\n\t\tname = fixture_topbar_hud\n\t\ttext = \"Email"
                .to_vec();
        bytes.push(0xa7);
        bytes.extend_from_slice(b"R\"\n\t}\n}\n");

        let losses = diagnose_text_decode(&bytes);
        assert_eq!(losses.len(), 1);
        assert!(!losses[0].discards_input());
        assert!(destructive_losses(&losses).is_empty());

        let vfs = MapVfs::new(&[("interface/fixture.gui", bytes.as_slice())]);
        let document = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 0),
            &authority(),
            "english",
        )
        .expect("emit");
        let closure = &document.closures[0];
        assert_eq!(closure.bindings.len(), 1);
        assert!(
            !closure
                .missing_roles
                .iter()
                .any(|role| role.starts_with("decode_loss:")),
            "{:?}",
            closure.missing_roles
        );
    }

    #[test]
    fn destroyed_bytes_in_a_bound_bmfont_descriptor_block_the_closure() {
        let mut fnt =
            b"\xef\xbb\xbfinfo face=\"f\" size=16\npage id=0 file=\"myfont_0.dds\"\n".to_vec();
        fnt.push(0xff);
        fnt.push(b'\n');

        let vfs = MapVfs::new(&[
            (
                "interface/fixture.gui",
                b"guiTypes = {\n\tcontainerWindowType = {\n\t\tname = fixture_topbar_hud\n\t\tinstantTextBoxType = {\n\t\t\tname = label\n\t\t\tfont = \"myfont\"\n\t\t}\n\t}\n}\n"
                    .as_slice(),
            ),
            (
                "interface/core.gfx",
                b"bitmapfonts={ bitmapfont={ name=myfont path=\"gfx/fonts/myfont\" } }"
                    .as_slice(),
            ),
            ("gfx/fonts/myfont.fnt", fnt.as_slice()),
            ("gfx/fonts/myfont_0.dds", b"\x00binary-atlas".as_slice()),
        ]);
        let document = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 0),
            &authority(),
            "english",
        )
        .expect("emit");
        let closure = &document.closures[0];

        assert!(
            closure
                .bindings
                .iter()
                .any(|b| b.content.normalized_game_relative_path == "gfx/fonts/myfont.fnt"),
            "{:?}",
            closure
                .bindings
                .iter()
                .map(|b| &b.content.normalized_game_relative_path)
                .collect::<Vec<_>>()
        );
        assert!(!closure.complete_dependency_closure);
        assert!(
            closure
                .missing_roles
                .iter()
                .any(|role| role.starts_with("decode_loss:gfx/fonts/myfont.fnt:")),
            "{:?}",
            closure.missing_roles
        );
        assert!(
            !closure
                .missing_roles
                .iter()
                .any(|role| role.contains("myfont_0.dds")),
            "{:?}",
            closure.missing_roles
        );
    }

    #[test]
    fn gfx_relying_on_the_engine_tolerated_eof_close_still_binds_sprite_and_texture() {
        // kills: dropping the `&& !engine_tolerated_outer_eof(&document)` guard from the interface/**/*.gfx sprite-scan gate in
        // emit_effective_vfs_hud_role_index re-blackholes the tolerated file.
        let vfs = MapVfs::new(&[
            (
                "interface/fixture.gui",
                br#"guiTypes = {
                    containerWindowType = {
                        name = fixture_topbar_hud
                        iconType = { name = icon spriteType = GFX_powerbalance }
                    }
                }"#
                    .as_slice(),
            ),
            (
                "interface/fixture.gfx",
                br#"spriteTypes = {
                    spriteType = {
                        name = GFX_powerbalance
                        textureFile = "gfx/interface/powerbalance.dds"
                    }"#
                    .as_slice(),
            ),
            ("gfx/interface/powerbalance.dds", b"texture"),
        ]);
        let document = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 0),
            &authority(),
            "english",
        )
        .expect("the engine-tolerated EOF close must not blackhole the sprite registry");
        let closure = &document.closures[0];
        assert!(
            closure.bindings.iter().any(|binding| matches!(
                &binding.semantic_role,
                HudSemanticRole::Sprite { sprite_id, .. } if sprite_id == "GFX_powerbalance"
            )),
            "{:?}",
            closure.bindings
        );
        assert!(
            closure.bindings.iter().any(|binding| matches!(
                &binding.semantic_role,
                HudSemanticRole::SpriteTexture { sprite_id, .. } if sprite_id == "GFX_powerbalance"
            ) && binding.content.normalized_game_relative_path
                == "gfx/interface/powerbalance.dds"),
            "{:?}",
            closure.bindings
        );
        assert!(
            closure
                .missing_roles
                .iter()
                .all(|role| !role.starts_with("sprite:")),
            "{:?}",
            closure.missing_roles
        );
    }

    #[test]
    fn gui_relying_on_the_engine_tolerated_eof_close_still_binds_the_root() {
        // kills: dropping the same tolerated-EOF guard from the
        // interface/**/*.gui scan gate re-blackholes the root.
        let vfs = MapVfs::new(&[(
            "interface/fixture.gui",
            br#"guiTypes = {
                containerWindowType = {
                    name = fixture_topbar_hud
                }"#
                .as_slice(),
        )]);
        let document = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 0),
            &authority(),
            "english",
        )
        .expect("the engine-tolerated EOF close must not drop the GUI root");
        let closure = &document.closures[0];
        assert_eq!(closure.bindings.len(), 1);
        assert!(matches!(
            &closure.bindings[0].semantic_role,
            HudSemanticRole::GuiRoot { window_or_root, .. } if window_or_root == "fixture_topbar_hud"
        ));
        assert!(
            closure
                .missing_roles
                .iter()
                .all(|role| !role.starts_with("compose_hud_root:")),
            "{:?}",
            closure.missing_roles
        );
        assert_eq!(
            document.winning_content[0].normalized_game_relative_path,
            "interface/fixture.gui"
        );
    }

    #[test]
    fn bitmapfont_registry_on_an_engine_tolerated_eof_close_still_plans_fonts() {
        // kills: reverting the font-registry scan gate to refusing on any
        // recovery fails the whole emitter again on powerbalance-shaped input.
        let vfs = MapVfs::new(&[
            (
                "interface/fixture.gui",
                b"guiTypes = {\n\tcontainerWindowType = {\n\t\tname = fixture_topbar_hud\n\t\tinstantTextBoxType = {\n\t\t\tname = label\n\t\t\tfont = \"myfont\"\n\t\t}\n\t}\n}\n"
                    .as_slice(),
            ),
            (
                "interface/core.gfx",
                b"bitmapfonts={\n\tbitmapfont={\n\t\tname=myfont\n\t\tpath=\"gfx/fonts/myfont\"\n\t}"
                    .as_slice(),
            ),
            (
                "gfx/fonts/myfont.fnt",
                b"info face=\"myfont\" size=16\npage id=0 file=\"myfont_0.dds\"\n".as_slice(),
            ),
            ("gfx/fonts/myfont_0.dds", b"\x00binary-atlas".as_slice()),
        ]);
        let document = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 0),
            &authority(),
            "english",
        )
        .expect("the engine-tolerated EOF close must not refuse the font registry");
        let closure = &document.closures[0];
        assert!(closure.bindings.iter().any(|binding| matches!(
            &binding.semantic_role,
            HudSemanticRole::Font { .. }
        ) && binding.content.normalized_game_relative_path == "gfx/fonts/myfont.fnt"));
        assert!(closure.bindings.iter().any(|binding| matches!(
            &binding.semantic_role,
            HudSemanticRole::FontTexture { font_id, .. } if font_id == "myfont"
        )));
        assert!(
            closure
                .missing_roles
                .iter()
                .all(|role| !role.starts_with("font:")),
            "{:?}",
            closure.missing_roles
        );
    }

    #[test]
    fn genuinely_damaged_gfx_still_refuses_the_font_registry_scan_by_name() {
        // Guard: the tolerance admits only the engine-admitted class; an
        // unterminated quoted string is real damage and must refuse by name.
        let vfs = MapVfs::new(&[(
            "interface/fontreg.gfx",
            b"bitmapfonts={\n\tbitmapfont={\n\t\tname=myfont\n\t\tpath=\"gfx/fonts/myfont\n\t}\n}\n"
                .as_slice(),
        )]);
        let error = authored_bitmap_font_dependency_registry_paths(
            &vfs,
            &BTreeSet::from(["myfont".to_string()]),
            "l_english",
            &authority(),
            &mut BTreeMap::new(),
        )
        .expect_err("an unterminated quoted string is real damage");
        assert_eq!(
            error,
            "interface/fontreg.gfx: bitmap-font registry parse required recovery"
        );
    }

    #[test]
    fn genuinely_damaged_gfx_still_fails_the_emitter_by_name() {
        // Guard: genuine damage keeps refusing the whole emitter by name (the
        // bitmap-font registry scan covers the same interface/**/*.gfx set, so
        // an emit-level Ok-with-blockers is not a contract the emitter has).
        let vfs = MapVfs::new(&[
            (
                "interface/fixture.gui",
                br#"guiTypes = {
                    containerWindowType = {
                        name = fixture_topbar_hud
                        iconType = { name = icon spriteType = GFX_broken }
                    }
                }"#
                    .as_slice(),
            ),
            (
                "interface/broken.gfx",
                b"spriteTypes={\n\tspriteType={\n\t\tname=GFX_broken\n\t\ttextureFile=\"gfx/broken.dds\n\t}\n}\n"
                    .as_slice(),
            ),
            ("gfx/broken.dds", b"texture"),
        ]);
        let error = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 0),
            &authority(),
            "english",
        )
        .expect_err("an unterminated quoted string is real damage");
        assert_eq!(
            error,
            "interface/broken.gfx: bitmap-font registry parse required recovery"
        );
    }

    #[test]
    fn core_gfx_alias_on_an_engine_tolerated_eof_close_still_resolves_fonts() {
        // kills: reverting the cfg(test) core.gfx alias gate to refusing on
        // any recovery breaks alias resolution for tolerated registries.
        let vfs = MapVfs::new(&[
            (
                "interface/core.gfx",
                b"bitmapfonts={\n\tbitmapfont={\n\t\tname=myfont\n\t\tpath=\"gfx/fonts/alias_source\"\n\t}"
                    .as_slice(),
            ),
            (
                "gfx/fonts/alias_source.fnt",
                b"info face=\"myfont\" size=16\npage id=0 file=\"alias_source_0.dds\"\n".as_slice(),
            ),
            ("gfx/fonts/alias_source_0.dds", b"\x00binary-atlas".as_slice()),
        ]);
        let paths = bitmap_font_dependency_paths(
            &vfs,
            "myfont",
            "l_english",
            &authority(),
            &mut BTreeMap::new(),
        )
        .expect("the engine-tolerated EOF close must not refuse the core.gfx alias");
        assert_eq!(paths.alias_source.as_deref(), Some("interface/core.gfx"));
        assert_eq!(paths.descriptor, "gfx/fonts/alias_source.fnt");
        assert_eq!(paths.pages, vec!["gfx/fonts/alias_source_0.dds"]);
    }

    #[test]
    fn genuinely_damaged_core_gfx_still_refuses_the_alias_by_name() {
        // Guard: alias resolution keeps refusing genuine damage by name.
        let vfs = MapVfs::new(&[(
            "interface/core.gfx",
            b"bitmapfonts={\n\tbitmapfont={\n\t\tname=myfont\n\t\tpath=\"gfx/fonts/myfont\n\t}\n}\n"
                .as_slice(),
        )]);
        let error = bitmap_font_dependency_paths(
            &vfs,
            "myfont",
            "l_english",
            &authority(),
            &mut BTreeMap::new(),
        )
        .expect_err("an unterminated quoted string is real damage");
        assert_eq!(error, "core.gfx font alias parse required recovery");
    }

    #[test]
    fn font_dependency_planner_uses_loader_page_priority_not_candidate_uniqueness() {
        let vfs = MapVfs::new(&[
            ("gfx/fonts/dual.fnt", b"info face=\"dual\" size=16\n"),
            ("gfx/fonts/dual.dds", b"dds"),
            ("gfx/fonts/dual.tga", b"tga"),
        ]);
        let paths = bitmap_font_dependency_paths(
            &vfs,
            "dual",
            "l_english",
            &authority(),
            &mut BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(paths.descriptor, "gfx/fonts/dual.fnt");
        assert_eq!(paths.pages, vec!["gfx/fonts/dual.dds"]);
    }

    #[test]
    fn authored_font_dependency_planner_preserves_ordered_variants_and_selection() {
        let core = br#"
            bitmapfonts = {
                bitmapfont = { name=Alias path="gfx/fonts/base" }
                bitmapfont_override = {
                    name=Alias
                    fontfiles={ "gfx/fonts/japanese/alias" "gfx/fonts/japanese/fallback" }
                    languages={ "l_japanese" }
                }
                bitmapfont_override = {
                    name=Alias
                    fontfiles={ "gfx/fonts/english/alias" }
                    languages={ "l_english" }
                }
            }
        "#;
        let vfs = MapVfs::new(&[
            ("interface/core.gfx", core),
            ("gfx/fonts/base.fnt", b"info face=\"base\" size=16\n"),
            ("gfx/fonts/base.dds", b"base"),
            (
                "gfx/fonts/japanese/alias.fnt",
                b"info face=\"jp\" size=16\n",
            ),
            ("gfx/fonts/japanese/alias.dds", b"jp"),
            (
                "gfx/fonts/japanese/fallback.fnt",
                b"info face=\"jp2\" size=16\n",
            ),
            ("gfx/fonts/japanese/fallback.dds", b"jp2"),
            ("gfx/fonts/english/alias.fnt", b"info face=\"en\" size=16\n"),
            ("gfx/fonts/english/alias.dds", b"en"),
        ]);
        let english = authored_bitmap_font_dependency_registry_paths(
            &vfs,
            &BTreeSet::from(["Alias".to_string()]),
            "english",
            &authority(),
            &mut BTreeMap::new(),
        )
        .unwrap()
        .remove("Alias")
        .unwrap()
        .unwrap();
        assert_eq!(english.selected_variant_index, 2);
        let japanese = authored_bitmap_font_dependency_registry_paths(
            &vfs,
            &BTreeSet::from(["Alias".to_string()]),
            "l_japanese",
            &authority(),
            &mut BTreeMap::new(),
        )
        .unwrap()
        .remove("Alias")
        .unwrap()
        .unwrap();
        assert_eq!(japanese.selected_variant_index, 1);
        assert_eq!(japanese.variants.len(), 3);
        assert_eq!(japanese.variants[1].ordered_descriptors.len(), 2);
        assert_eq!(japanese.variants[1].ordered_languages, vec!["l_japanese"]);
    }

    #[test]
    fn authored_font_registry_scans_gfx_once_for_multiple_fonts() {
        use std::cell::Cell;

        struct CountingVfs {
            inner: MapVfs,
            gfx_lists: Cell<usize>,
        }
        impl HudContentVfs for CountingVfs {
            fn list_recursive(
                &self,
                directory: &str,
                extension: &str,
            ) -> Result<Vec<String>, String> {
                if directory == "interface" && extension == "gfx" {
                    self.gfx_lists.set(self.gfx_lists.get() + 1);
                }
                self.inner.list_recursive(directory, extension)
            }

            fn resolve(&self, logical_path: &str) -> Result<Option<ResolvedHudFile>, String> {
                self.inner.resolve(logical_path)
            }
        }

        let vfs = CountingVfs {
            inner: MapVfs::new(&[
                (
                    "interface/fonts.gfx",
                    b"bitmapfonts={ bitmapfont={ name=One path=\"gfx/fonts/one\" } bitmapfont={ name=Two path=\"gfx/fonts/two\" } }",
                ),
                ("gfx/fonts/one.fnt", b"info face=\"one\" size=16\n"),
                ("gfx/fonts/one.dds", b"one"),
                ("gfx/fonts/two.fnt", b"info face=\"two\" size=16\n"),
                ("gfx/fonts/two.dds", b"two"),
            ]),
            gfx_lists: Cell::new(0),
        };
        let planned = authored_bitmap_font_dependency_registry_paths(
            &vfs,
            &BTreeSet::from(["One".to_string(), "Two".to_string()]),
            "l_english",
            &authority(),
            &mut BTreeMap::new(),
        )
        .unwrap();
        assert!(planned["One"].is_ok());
        assert!(planned["Two"].is_ok());
        assert_eq!(vfs.gfx_lists.get(), 1);
    }

    #[test]
    fn repeated_observations_of_one_damaged_file_do_not_multiply() {
        let mut damaged = BTreeMap::new();
        for _ in 0..4 {
            record_decode_damage_summary(&mut damaged, "gfx/fonts/f.fnt", 2, "invalid UTF-8");
        }
        assert_eq!(damaged.len(), 1);
        let summary = &damaged["gfx/fonts/f.fnt"];
        assert_eq!(
            summary.matches("destructive_events=2").count(),
            1,
            "{summary}"
        );

        record_decode_damage_summary(&mut damaged, "gfx/fonts/f.fnt", 1, "binary page name");
        let summary = &damaged["gfx/fonts/f.fnt"];
        assert!(summary.contains("destructive_events=2"), "{summary}");
        assert!(summary.contains("destructive_events=1"), "{summary}");

        record_decode_damage_summary(&mut damaged, "other.gui", 0, "unused");
        assert!(!damaged.contains_key("other.gui"));
    }

    #[test]
    fn clean_content_admission_is_unchanged_by_the_decode_channel() {
        let vfs = MapVfs::new(&[(
            "interface/fixture.gui",
            b"guiTypes = {\n\tcontainerWindowType = {\n\t\tname = fixture_topbar_hud\n\t}\n}\n"
                .as_slice(),
        )]);
        let document = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 0),
            &authority(),
            "english",
        )
        .expect("emit");
        let closure = &document.closures[0];
        assert_eq!(closure.bindings.len(), 1);
        assert_eq!(document.winning_content.len(), 1);
        assert_eq!(closure.missing_roles.len(), 3);
        assert!(closure
            .missing_roles
            .iter()
            .all(|role| role.starts_with("compose_hud_activation:")));
    }

    #[test]
    fn emitter_refuses_dirty_frames_and_ambiguous_roots() {
        let vfs = MapVfs::new(&[(
            "interface/fixture.gui",
            b"guiTypes = {\n\tcontainerWindowType = {\n\t\tname = fixture_topbar_hud\n\t}\n}\n"
                .as_slice(),
        )]);
        let error = emit_effective_vfs_hud_role_index(
            &vfs,
            &topbar_activation("fixture_topbar_hud", 1),
            &authority(),
            "english",
        )
        .unwrap_err();
        assert!(error.contains("latest-clean"));

        let activation = HudSurfaceActivationFrame {
            frame_hour: 40,
            status_flags: 0,
            observation_sequence: 1,
            active_surfaces: vec![
                HudSurfaceActivation {
                    surface: HudSurface::Topbar,
                    runtime_root_name: "fixture_topbar_hud".into(),
                },
                HudSurfaceActivation {
                    surface: HudSurface::Minimap,
                    runtime_root_name: "unknown_root".into(),
                },
            ],
        };
        let document =
            emit_effective_vfs_hud_role_index(&vfs, &activation, &authority(), "english")
                .expect("emit");
        let minimap = document
            .closures
            .iter()
            .find(|closure| closure.surface == HudSurface::Minimap)
            .expect("minimap closure");
        assert!(minimap.bindings.is_empty());
        assert!(!minimap.complete_dependency_closure);
        assert!(minimap
            .missing_roles
            .iter()
            .any(|role| role.contains("effective_candidate_count=0")));
    }

    #[test]
    fn binary_bmfont_pages_parse_and_truncations_fail_closed() {
        let mut valid = b"BMF\x03".to_vec();
        let pages = b"page_0.dds\0page_1.dds\0";
        valid.push(3);
        valid.extend_from_slice(&(pages.len() as u32).to_le_bytes());
        valid.extend_from_slice(pages);
        let parsed = bmfont_page_files(&valid).expect("valid binary BMFont");
        assert_eq!(parsed.pages.get(&0).map(String::as_str), Some("page_0.dds"));
        assert_eq!(parsed.pages.get(&1).map(String::as_str), Some("page_1.dds"));
        assert_eq!(parsed.lossy_page_names, 0);

        assert_eq!(
            bmfont_page_files(b"BMF").unwrap_err(),
            "binary BMFont header is truncated"
        );
        assert_eq!(
            bmfont_page_files(b"BMF\x03\x03").unwrap_err(),
            "binary BMFont block header is truncated"
        );
        let mut oversized = b"BMF\x03".to_vec();
        oversized.push(3);
        oversized.extend_from_slice(&64u32.to_le_bytes());
        oversized.extend_from_slice(b"tiny");
        assert_eq!(
            bmfont_page_files(&oversized).unwrap_err(),
            "binary BMFont block is truncated"
        );
        let mut no_pages = b"BMF\x03".to_vec();
        no_pages.push(1);
        no_pages.extend_from_slice(&1u32.to_le_bytes());
        no_pages.push(0);
        assert!(bmfont_page_files(&no_pages).unwrap().pages.is_empty());
    }

    #[test]
    fn binary_bmfont_page_names_that_lose_bytes_are_counted() {
        let mut bytes = b"BMF\x03".to_vec();
        let mut pages = b"good_0.dds\0".to_vec();
        pages.extend_from_slice(b"bad_\xff_1.dds\0");
        bytes.push(3);
        bytes.extend_from_slice(&(pages.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&pages);

        assert_eq!(destructive_event_count(&diagnose_text_decode(&bytes)), 0);
        let parsed = bmfont_page_files(&bytes).expect("binary BMFont");
        assert_eq!(parsed.lossy_page_names, 1);
        assert_eq!(parsed.pages.get(&0).map(String::as_str), Some("good_0.dds"));
        assert!(parsed.pages[&1].contains('\u{fffd}'));
    }

    #[test]
    fn text_bmfont_pages_and_kv_fields_parse() {
        let text = b"info face=\"font\" size=16\npage id=0 file=\"font_0.dds\"\npage id=1 file=font_1.dds\n";
        let parsed = bmfont_page_files(text).expect("text BMFont");
        assert_eq!(parsed.pages.get(&0).map(String::as_str), Some("font_0.dds"));
        assert_eq!(parsed.pages.get(&1).map(String::as_str), Some("font_1.dds"));

        let fields = parse_kv_fields("id=3 file=\"a b.dds\" bare orphan= last=x");
        assert_eq!(fields.get("id").map(String::as_str), Some("3"));
        assert_eq!(fields.get("file").map(String::as_str), Some("a b.dds"));
        assert_eq!(fields.get("orphan").map(String::as_str), Some(""));
        assert_eq!(fields.get("last").map(String::as_str), Some("x"));
        assert!(!fields.contains_key("bare"));
    }

    #[test]
    fn texture_resolution_falls_back_between_dds_and_tga_only() {
        let vfs = MapVfs::new(&[("gfx/interface/icon.tga", b"tga-bytes".as_slice())]);
        let key = resolve_texture_key(&vfs, "gfx/interface/icon.dds", &authority())
            .expect("dds->tga fallback");
        assert_eq!(key.normalized_game_relative_path, "gfx/interface/icon.tga");
        let error = resolve_texture_key(&vfs, "gfx/interface/none.dds", &authority()).unwrap_err();
        assert!(error.contains("missing HUD texture dependency"));
        let error = resolve_texture_key(&vfs, "gfx/interface/icon.png", &authority()).unwrap_err();
        assert!(error.contains("missing HUD texture dependency"));
    }

    #[test]
    fn localisation_sources_match_only_the_expected_language_header() {
        let vfs = MapVfs::new(&[
            (
                "localisation/english/fixture_l_english.yml",
                b"l_english:\n KEY_ALPHA:0 \"Alpha\"\n # comment\n KEY_BETA:0 \"Beta\"\n"
                    .as_slice(),
            ),
            (
                "localisation/french/fixture_l_french.yml",
                b"l_french:\n KEY_ALPHA:0 \"Alpha\"\n".as_slice(),
            ),
        ]);
        let mut damaged = BTreeMap::new();
        let sources = localisation_sources(&vfs, "english", &mut damaged).expect("localisation");
        assert!(damaged.is_empty(), "clean localisation records no damage");
        assert_eq!(
            sources.get("key_alpha").map(String::as_str),
            Some("localisation/english/fixture_l_english.yml")
        );
        assert_eq!(
            sources.get("key_beta").map(String::as_str),
            Some("localisation/english/fixture_l_english.yml")
        );
        assert_eq!(sources.len(), 2);
    }
}
