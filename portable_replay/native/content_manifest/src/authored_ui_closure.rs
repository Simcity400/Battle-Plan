use crate::authored_ui::{
    classify_authored_ui_path, engine_path_spelling_consumer, engine_recursive_path_cmp,
    safe_language_directory, AuthoredUiNamespace, AUTHORED_BITMAP_FONT_TEXT_EFFECT_PATH,
    AUTHORED_GRAPHICS_DEFINES_PATH, DEPENDENCY_CLOSURE_CONSUMER, DEPENDENCY_CLOSURE_EMPTY_CONSUMER,
    GFX_EMPTY_NAMESPACE_CONSUMER, GUI_EMPTY_NAMESPACE_CONSUMER,
    LOCALISATION_EMPTY_NAMESPACE_CONSUMER, SCRIPTED_LOCALISATION_EMPTY_NAMESPACE_CONSUMER,
};
use crate::effective_vfs::{
    EffectiveVfs, EffectiveVfsResolvedFile, MAX_LISTED_FILES, MAX_LISTED_TOTAL_BYTES,
};
use crate::gfx::{a729_effect_shader_path, xui07_builtin_shader_includes, SpriteRegistry};
use crate::gui::{parse_gui_document, GuiElement};
use crate::hud_emitter::authored_bitmap_font_dependency_registry_paths;
use crate::hud_role::validate_sha256;
use crate::paradox_text::{destructive_event_count, diagnose_text_decode, parse_bytes};
use crate::{normalize_game_relative_path, UsedContentKey};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredUiContentObservation {
    pub consumer: String,
    pub content: UsedContentKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredUiContentClosure {
    pub observations: Vec<AuthoredUiContentObservation>,
    pub empty_namespace_consumers: Vec<String>,
}

pub fn emit_effective_vfs_authored_ui_closure(
    vfs: &EffectiveVfs,
    authority_manifest_sha256: &str,
    localisation_language: &str,
) -> Result<AuthoredUiContentClosure, String> {
    validate_sha256(
        authority_manifest_sha256,
        "authored UI effective-VFS authority",
    )?;
    if vfs.authority_manifest_sha256() != authority_manifest_sha256 {
        return Err("authored UI VFS authority differs from source inventory".into());
    }

    let mut listing_budget = ClosureListingBudget::default();
    let all = list_authored_namespace_candidates(vfs, localisation_language, &mut listing_budget)?;
    let mut namespace_files = Vec::new();
    let mut observations = BTreeMap::<(String, String), AuthoredUiContentObservation>::new();
    for file in all {
        let Some(namespace) = classify_authored_ui_path(&file.logical_path, localisation_language)?
        else {
            continue;
        };
        validate_file(&file, authority_manifest_sha256)?;
        insert_observation(&mut observations, namespace.consumer(), &file.content)?;
        insert_observation(
            &mut observations,
            &engine_path_spelling_consumer(&file.logical_path)?,
            &file.content,
        )?;
        insert_observation(
            &mut observations,
            DEPENDENCY_CLOSURE_CONSUMER,
            &file.content,
        )?;
        namespace_files.push((namespace, file));
    }
    let authored_namespaces = namespace_files
        .iter()
        .map(|(namespace, _)| *namespace)
        .collect::<BTreeSet<_>>();
    let mut empty_namespace_consumers = Vec::new();
    for (namespace, empty_consumer) in [
        (AuthoredUiNamespace::Gui, GUI_EMPTY_NAMESPACE_CONSUMER),
        (AuthoredUiNamespace::Gfx, GFX_EMPTY_NAMESPACE_CONSUMER),
        (
            AuthoredUiNamespace::Localisation,
            LOCALISATION_EMPTY_NAMESPACE_CONSUMER,
        ),
        (
            AuthoredUiNamespace::ScriptedLocalisation,
            SCRIPTED_LOCALISATION_EMPTY_NAMESPACE_CONSUMER,
        ),
    ] {
        if !authored_namespaces.contains(&namespace) {
            empty_namespace_consumers.push(empty_consumer.to_string());
        }
    }
    let mut fonts = BTreeSet::new();
    let mut direct_textures = BTreeSet::new();
    let mut effective_gui_roots = BTreeMap::<String, (String, GuiElement)>::new();
    for (_, file) in namespace_files
        .iter()
        .filter(|(namespace, _)| *namespace == AuthoredUiNamespace::Gui)
    {
        refuse_destructive_text_decode(file, "authored GUI")?;
        let document = parse_bytes(&file.bytes);
        for root in parse_gui_document(&document) {
            effective_gui_roots.insert(
                root.name.to_ascii_lowercase(),
                (file.logical_path.clone(), root),
            );
        }
    }
    let requires_graphics_defines =
        effective_gui_roots
            .values()
            .try_fold(false, |required, (source, root)| {
                Ok::<_, String>(
                    required
                        || (source.eq_ignore_ascii_case("interface/core.gui")
                            && is_tooltip_shell_root(root)?),
                )
            })?;
    for (_, root) in effective_gui_roots.into_values() {
        collect_element_dependencies(&root, &mut fonts, &mut direct_textures)?;
    }

    let mut sprites = SpriteRegistry::new();
    for (_, file) in namespace_files
        .iter()
        .filter(|(namespace, _)| *namespace == AuthoredUiNamespace::Gfx)
    {
        refuse_destructive_text_decode(file, "authored GFX")?;
        let document = parse_bytes(&file.bytes);
        if !document.recoveries.is_empty() {
            return Err(format!(
                "authored GFX parse required recovery in {}",
                file.logical_path
            ));
        }
        sprites.add_document(&file.logical_path, &document);
    }
    // needed from this point onward, so release all listing payloads before
    drop(namespace_files);

    let mut dependency_cache = BTreeMap::<String, Option<UsedContentKey>>::new();
    for sprite in sprites.effective_sprites() {
        for slot in &sprite.texture_files {
            let Some(content) = resolve_texture_cached(
                vfs,
                &slot.path,
                authority_manifest_sha256,
                &mut dependency_cache,
            )?
            else {
                continue;
            };
            insert_observation(&mut observations, DEPENDENCY_CLOSURE_CONSUMER, &content)?;
        }
        if let Ok(effect) = a729_effect_shader_path(sprite.effect_file.as_deref()) {
            let Some(content) = resolve_dependency_cached(
                vfs,
                &effect,
                authority_manifest_sha256,
                &mut dependency_cache,
            )?
            else {
                continue;
            };
            insert_observation(&mut observations, DEPENDENCY_CLOSURE_CONSUMER, &content)?;
            if let Some(includes) = xui07_builtin_shader_includes(&effect) {
                for include in includes {
                    let Some(content) = resolve_dependency_cached(
                        vfs,
                        include,
                        authority_manifest_sha256,
                        &mut dependency_cache,
                    )?
                    else {
                        continue;
                    };
                    insert_observation(&mut observations, DEPENDENCY_CLOSURE_CONSUMER, &content)?;
                }
            }
        }
    }
    for texture in direct_textures {
        if let Some(content) = resolve_texture_cached(
            vfs,
            &texture,
            authority_manifest_sha256,
            &mut dependency_cache,
        )? {
            insert_observation(&mut observations, DEPENDENCY_CLOSURE_CONSUMER, &content)?;
        }
    }
    if requires_graphics_defines {
        if let Some(content) = resolve_dependency_cached(
            vfs,
            AUTHORED_GRAPHICS_DEFINES_PATH,
            authority_manifest_sha256,
            &mut dependency_cache,
        )? {
            insert_observation(&mut observations, DEPENDENCY_CLOSURE_CONSUMER, &content)?;
        }
    }

    if !fonts.is_empty() {
        if let Some(content) = resolve_dependency_cached(
            vfs,
            AUTHORED_BITMAP_FONT_TEXT_EFFECT_PATH,
            authority_manifest_sha256,
            &mut dependency_cache,
        )? {
            insert_observation(&mut observations, DEPENDENCY_CLOSURE_CONSUMER, &content)?;
        }
    }
    let font_registry = authored_bitmap_font_dependency_registry_paths(
        vfs,
        &fonts,
        localisation_language,
        authority_manifest_sha256,
        &mut BTreeMap::new(),
    )?;
    for font in fonts {
        let Some(Ok(paths)) = font_registry.get(&font) else {
            continue;
        };
        for path in paths.registration_sources.iter().cloned().chain(
            paths
                .variants
                .iter()
                .flat_map(|variant| variant.ordered_descriptors.iter())
                .flat_map(|descriptor| {
                    std::iter::once(descriptor.descriptor.clone())
                        .chain(descriptor.pages.iter().cloned())
                }),
        ) {
            let content = resolve_dependency_cached(
                vfs,
                &path,
                authority_manifest_sha256,
                &mut dependency_cache,
            )?
            .ok_or_else(|| format!("authored font {font} lost dependency {path}"))?;
            insert_observation(&mut observations, DEPENDENCY_CLOSURE_CONSUMER, &content)?;
        }
    }

    if !observations
        .keys()
        .any(|(consumer, _)| consumer == DEPENDENCY_CLOSURE_CONSUMER)
    {
        empty_namespace_consumers.push(DEPENDENCY_CLOSURE_EMPTY_CONSUMER.to_string());
    }
    empty_namespace_consumers.sort();
    Ok(AuthoredUiContentClosure {
        observations: observations.into_values().collect(),
        empty_namespace_consumers,
    })
}

fn list_authored_namespace_candidates(
    vfs: &EffectiveVfs,
    localisation_language: &str,
    budget: &mut ClosureListingBudget,
) -> Result<Vec<EffectiveVfsResolvedFile>, String> {
    let mut files = BTreeMap::new();
    let selected_localisation = format!(
        "localisation/{}",
        safe_language_directory(localisation_language)?
    );
    for (prefix, extension) in [
        ("interface", "gui"),
        ("interface", "gfx"),
        (selected_localisation.as_str(), "yml"),
        ("common/scripted_localisation", "txt"),
    ] {
        for file in budget.list(vfs, prefix, extension)? {
            let key = file.logical_path.to_ascii_lowercase();
            if let Some(prior) = files.get(&key) {
                if prior != &file {
                    return Err(format!(
                        "authored UI namespace listing disagrees for {}",
                        file.logical_path
                    ));
                }
                continue;
            }
            files.insert(key, file);
        }
    }
    let mut files = files.into_values().collect::<Vec<_>>();
    files.sort_by(|left, right| engine_recursive_path_cmp(&left.logical_path, &right.logical_path));
    Ok(files)
}

struct ClosureListingBudget {
    remaining_files: usize,
    remaining_bytes: u64,
}

impl Default for ClosureListingBudget {
    fn default() -> Self {
        Self {
            remaining_files: MAX_LISTED_FILES,
            remaining_bytes: MAX_LISTED_TOTAL_BYTES,
        }
    }
}

impl ClosureListingBudget {
    fn list(
        &mut self,
        vfs: &EffectiveVfs,
        prefix: &str,
        extension: &str,
    ) -> Result<Vec<EffectiveVfsResolvedFile>, String> {
        let files = vfs.list_recursive_bounded(
            prefix,
            Some(extension),
            self.remaining_files,
            self.remaining_bytes,
        )?;
        let bytes = files.iter().try_fold(0u64, |total, file| {
            total
                .checked_add(file.content.payload_bytes)
                .ok_or_else(|| "authored UI closure listing byte count overflowed".to_string())
        })?;
        self.remaining_files = self
            .remaining_files
            .checked_sub(files.len())
            .ok_or_else(|| "authored UI closure listing file budget underflowed".to_string())?;
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(bytes)
            .ok_or_else(|| "authored UI closure listing byte budget underflowed".to_string())?;
        Ok(files)
    }
}

fn insert_observation(
    observations: &mut BTreeMap<(String, String), AuthoredUiContentObservation>,
    consumer: &str,
    content: &UsedContentKey,
) -> Result<(), String> {
    let key = (
        consumer.to_string(),
        content.normalized_game_relative_path.clone(),
    );
    let row = AuthoredUiContentObservation {
        consumer: consumer.to_string(),
        content: content.clone(),
    };
    if observations
        .insert(key, row.clone())
        .is_some_and(|prior| prior != row)
    {
        return Err(format!(
            "authored UI closure disagrees on {}",
            content.normalized_game_relative_path
        ));
    }
    Ok(())
}

fn validate_file(
    file: &EffectiveVfsResolvedFile,
    authority_manifest_sha256: &str,
) -> Result<(), String> {
    file.content.validate()?;
    let normalized_logical_path = normalize_game_relative_path(&file.logical_path)?;
    if file.content.effective_vfs_manifest_sha256 != authority_manifest_sha256
        || file.content.normalized_game_relative_path != normalized_logical_path
    {
        return Err(format!(
            "authored UI resolved identity disagrees for {}",
            file.logical_path
        ));
    }
    Ok(())
}

fn refuse_destructive_text_decode(
    file: &EffectiveVfsResolvedFile,
    label: &str,
) -> Result<(), String> {
    let events = destructive_event_count(&diagnose_text_decode(&file.bytes));
    if events != 0 {
        return Err(format!(
            "{label} decode destroyed {events} byte sequences in {}",
            file.logical_path
        ));
    }
    Ok(())
}

fn resolve_dependency_cached(
    vfs: &EffectiveVfs,
    path: &str,
    authority_manifest_sha256: &str,
    cache: &mut BTreeMap<String, Option<UsedContentKey>>,
) -> Result<Option<UsedContentKey>, String> {
    let normalized = normalize_game_relative_path(path)?;
    if let Some(content) = cache.get(&normalized) {
        return Ok(content.clone());
    }
    let content = match vfs.resolve(&normalized)? {
        Some(file) => {
            validate_file(&file, authority_manifest_sha256)?;
            Some(file.content)
        }
        None => None,
    };
    cache.insert(normalized, content.clone());
    Ok(content)
}

fn resolve_texture_cached(
    vfs: &EffectiveVfs,
    path: &str,
    authority_manifest_sha256: &str,
    cache: &mut BTreeMap<String, Option<UsedContentKey>>,
) -> Result<Option<UsedContentKey>, String> {
    if let Some(content) = resolve_dependency_cached(vfs, path, authority_manifest_sha256, cache)? {
        return Ok(Some(content));
    }
    let normalized = path.replace('\\', "/");
    let Some((stem, extension)) = normalized.rsplit_once('.') else {
        return Ok(None);
    };
    let alternate = match extension.to_ascii_lowercase().as_str() {
        "dds" => format!("{stem}.tga"),
        "tga" => format!("{stem}.dds"),
        _ => return Ok(None),
    };
    resolve_dependency_cached(vfs, &alternate, authority_manifest_sha256, cache)
}

fn collect_element_dependencies(
    element: &GuiElement,
    fonts: &mut BTreeSet<String>,
    direct_textures: &mut BTreeSet<String>,
) -> Result<(), String> {
    if let Some(font) = element.font.as_ref().filter(|font| !font.trim().is_empty()) {
        fonts.insert(font.clone());
    }
    for value in element.raw.values_ci("textureFile") {
        let texture = value.as_scalar().ok_or_else(|| {
            format!(
                "authored GUI element {:?} textureFile is not scalar",
                element.name
            )
        })?;
        if texture.trim().is_empty() {
            continue;
        }
        direct_textures.insert(texture.to_string());
    }
    for child in &element.children {
        collect_element_dependencies(child, fonts, direct_textures)?;
    }
    Ok(())
}

fn is_tooltip_shell_root(root: &GuiElement) -> Result<bool, String> {
    if !root.element_type.eq_ignore_ascii_case("textBoxType") {
        return Ok(false);
    }
    for required in ["textureFile", "borderSize", "font", "maxWidth", "maxHeight"] {
        let mut values = root.raw.values_ci(required);
        let Some(value) = values.next() else {
            return Ok(false);
        };
        if values.next().is_some() {
            return Err(format!(
                "core.gui structural tooltip candidate {:?} repeats {required}",
                root.name
            ));
        }
        if required.eq_ignore_ascii_case("borderSize") {
            if value.as_object().is_none() {
                return Ok(false);
            }
            continue;
        }
        let Some(value) = value.as_scalar() else {
            return Ok(false);
        };
        if value.trim().is_empty() {
            return Err(format!(
                "core.gui structural tooltip candidate {:?} blanks {required}",
                root.name
            ));
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authored_ui::{
        GFX_NAMESPACE_CONSUMER, GUI_NAMESPACE_CONSUMER, LOCALISATION_NAMESPACE_CONSUMER,
        SCRIPTED_LOCALISATION_NAMESPACE_CONSUMER,
    };
    use crate::effective_vfs::EffectiveVfsLayerMount;
    use crate::{ContentLayerKind, LayerStorageKind};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "xui24_authored_closure_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let path = rel
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn dormant_mod_window_emits_definitions_and_transitive_assets_without_activation() {
        let root = temp_root();
        let base = root.join("base");
        let modification = root.join("mod");
        write(
            &base,
            "interface/base.gui",
            br#"guiTypes={ containerWindowType={ name=base } }"#,
        );
        write(
            &base,
            "interface/base.gfx",
            br#"spriteTypes={ spriteType={ name=GFX_base texturefile="gfx/interface/base.dds" } }"#,
        );
        write(&base, "gfx/interface/base.dds", b"base texture");
        write(
            &base,
            "localisation/english/base_l_english.yml",
            b"l_english:\n BASE:0 \"Base\"\n",
        );
        write(
            &modification,
            "interface/mod_panel.gui",
            br#"guiTypes={ containerWindowType={ name=mod_panel iconType={ name=icon spriteType=GFX_mod } instantTextBoxType={ name=label font=modfont text=MOD_LABEL } } }"#,
        );
        write(
            &modification,
            "interface/mod_panel.gfx",
            br#"
                spriteTypes={
                    spriteType={
                        name=GFX_mod
                        texturefile="gfx/interface/mod_panel.dds"
                    }
                }
                bitmapfonts={
                    bitmapfont={
                        name=modfont
                        path="gfx/fonts/modfont"
                    }
                }
            "#,
        );
        write(&modification, "gfx/interface/mod_panel.dds", b"mod texture");
        write(
            &modification,
            "gfx/fonts/modfont.fnt",
            b"info face=\"mod\" size=16\npage id=0 file=\"modfont_0.dds\"\n",
        );
        write(&modification, "gfx/fonts/modfont_0.dds", b"font atlas");
        write(&modification, "gfx/FX/text.shader", b"font text effect");
        write(
            &modification,
            "localisation/english/mod_l_english.yml",
            b"l_english:\n MOD_LABEL:0 \"Mod label\"\n",
        );
        write(
            &modification,
            "common/scripted_localisation/mod.txt",
            b"defined_text={ name=MOD_DYNAMIC text={ localization_key=MOD_LABEL } }",
        );
        let authority = "a".repeat(64);
        let vfs = EffectiveVfs::new(
            authority.clone(),
            [
                EffectiveVfsLayerMount {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: base,
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
                EffectiveVfsLayerMount {
                    ordinal: 1,
                    layer_kind: ContentLayerKind::Mod,
                    layer_id: "mod/test.mod".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: modification,
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
            ],
        )
        .unwrap();
        let closure =
            emit_effective_vfs_authored_ui_closure(&vfs, &authority, "l_english").unwrap();
        let paths = closure
            .observations
            .iter()
            .map(|row| {
                (
                    row.consumer.as_str(),
                    row.content.normalized_game_relative_path.as_str(),
                )
            })
            .collect::<BTreeSet<_>>();
        for expected in [
            (GUI_NAMESPACE_CONSUMER, "interface/mod_panel.gui"),
            (GFX_NAMESPACE_CONSUMER, "interface/mod_panel.gfx"),
            (
                LOCALISATION_NAMESPACE_CONSUMER,
                "localisation/english/mod_l_english.yml",
            ),
            (
                SCRIPTED_LOCALISATION_NAMESPACE_CONSUMER,
                "common/scripted_localisation/mod.txt",
            ),
            (DEPENDENCY_CLOSURE_CONSUMER, "gfx/interface/mod_panel.dds"),
            (DEPENDENCY_CLOSURE_CONSUMER, "gfx/fonts/modfont.fnt"),
            (DEPENDENCY_CLOSURE_CONSUMER, "gfx/fonts/modfont_0.dds"),
            (DEPENDENCY_CLOSURE_CONSUMER, "gfx/fx/text.shader"),
        ] {
            assert!(paths.contains(&expected), "missing {expected:?}");
        }
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selected_dlc_content_uses_mounted_root_identity_and_winning_layer() {
        let root = temp_root();
        let base = root.join("base");
        let selected_dlc = root.join("selected_dlc");
        write(
            &base,
            "interface/base.gui",
            br#"guiTypes={ containerWindowType={ name=base } }"#,
        );
        write(&base, "interface/base.gfx", b"spriteTypes = { }");
        write(
            &base,
            "localisation/english/base_l_english.yml",
            b"l_english:\n BASE:0 \"Base\"\n",
        );
        write(
            &selected_dlc,
            "interface/dlc_panel.gui",
            br#"guiTypes={ containerWindowType={ name=selected_dlc_panel } }"#,
        );
        let authority = "b".repeat(64);
        let vfs = EffectiveVfs::new(
            authority.clone(),
            [
                EffectiveVfsLayerMount {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: base,
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
                EffectiveVfsLayerMount {
                    ordinal: 1,
                    layer_kind: ContentLayerKind::Dlc,
                    layer_id: "dlc/dlc_selected/dlc_selected.dlc".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: selected_dlc,
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
            ],
        )
        .unwrap();
        let closure = emit_effective_vfs_authored_ui_closure(&vfs, &authority, "english").unwrap();
        let selected_panel = closure
            .observations
            .iter()
            .find(|row| {
                row.consumer == GUI_NAMESPACE_CONSUMER
                    && row.content.normalized_game_relative_path == "interface/dlc_panel.gui"
            })
            .unwrap();
        assert_eq!(selected_panel.content.winning_layer_ordinal, 1);
        assert_eq!(selected_panel.content.layer_kind, ContentLayerKind::Dlc);
        assert_eq!(
            selected_panel.content.layer_id,
            "dlc/dlc_selected/dlc_selected.dlc"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unresolved_dormant_surface_does_not_suppress_complete_surface_closure() {
        let root = temp_root();
        let base = root.join("base");
        write(
            &base,
            "interface/panels.gui",
            br#"guiTypes={
                containerWindowType={
                    name=complete
                    iconType={ name=icon spriteType=GFX_complete }
                }
                containerWindowType={
                    name=incomplete
                    iconType={ name=icon spriteType=GFX_absent }
                }
            }"#,
        );
        write(
            &base,
            "interface/panels.gfx",
            br#"spriteTypes={
                spriteType={
                    name=GFX_complete
                    texturefile="gfx/interface/complete.dds"
                }
            }"#,
        );
        write(&base, "gfx/interface/complete.dds", b"complete texture");
        write(
            &base,
            "localisation/english/base_l_english.yml",
            b"l_english:\n COMPLETE:0 \"Complete\"\n",
        );
        let authority = "c".repeat(64);
        let vfs = EffectiveVfs::new(
            authority.clone(),
            [EffectiveVfsLayerMount {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                storage_kind: LayerStorageKind::Directory,
                local_source: base,
                source_container_sha256: None,
                replace_paths: Vec::new(),
            }],
        )
        .unwrap();
        let closure = emit_effective_vfs_authored_ui_closure(&vfs, &authority, "english").unwrap();
        assert!(closure.observations.iter().any(|row| {
            row.consumer == DEPENDENCY_CLOSURE_CONSUMER
                && row.content.normalized_game_relative_path == "gfx/interface/complete.dds"
        }));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gui_type_dependency_closure_uses_case_insensitive_last_parsed_root_winner() {
        let root = temp_root();
        let base = root.join("base");
        write(
            &base,
            "interface/Z.gui",
            br#"guiTypes={
                containerWindowType={
                    name=SharedRoot
                    iconType={ name=icon textureFile="gfx/interface/losing.dds" }
                }
            }"#,
        );
        write(
            &base,
            "interface/a.gui",
            br#"guiTypes={
                containerWindowType={
                    name=sharedroot
                    iconType={ name=icon textureFile="gfx/interface/winner.dds" }
                }
            }"#,
        );
        write(&base, "gfx/interface/losing.dds", b"losing");
        write(&base, "gfx/interface/winner.dds", b"winner");
        write(
            &base,
            "localisation/english/base_l_english.yml",
            b"l_english:\n BASE:0 \"Base\"\n",
        );
        let authority = "d".repeat(64);
        let vfs = EffectiveVfs::new(
            authority.clone(),
            [EffectiveVfsLayerMount {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                storage_kind: LayerStorageKind::Directory,
                local_source: base,
                source_container_sha256: None,
                replace_paths: Vec::new(),
            }],
        )
        .unwrap();

        let closure = emit_effective_vfs_authored_ui_closure(&vfs, &authority, "english").unwrap();
        let dependency_paths = closure
            .observations
            .iter()
            .filter(|row| row.consumer == DEPENDENCY_CLOSURE_CONSUMER)
            .map(|row| row.content.normalized_game_relative_path.as_str())
            .collect::<BTreeSet<_>>();
        assert!(dependency_paths.contains("gfx/interface/winner.dds"));
        assert!(!dependency_paths.contains("gfx/interface/losing.dds"));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_empty_namespace_emits_a_generic_semantic_marker() {
        let root = temp_root();
        let base = root.join("base");
        write(&base, "common/countries.txt", b"countries = { }");
        write(
            &base,
            AUTHORED_GRAPHICS_DEFINES_PATH,
            b"NGraphics = { TOOLTIP_DELAYED_TIME = 500 }",
        );
        let authority = "e".repeat(64);
        let vfs = EffectiveVfs::new(
            authority.clone(),
            [EffectiveVfsLayerMount {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                storage_kind: LayerStorageKind::Directory,
                local_source: base,
                source_container_sha256: None,
                replace_paths: Vec::new(),
            }],
        )
        .unwrap();

        let closure =
            emit_effective_vfs_authored_ui_closure(&vfs, &authority, "l_english").unwrap();
        assert!(closure.observations.is_empty());
        assert_eq!(
            closure.empty_namespace_consumers,
            [
                GFX_EMPTY_NAMESPACE_CONSUMER,
                GUI_EMPTY_NAMESPACE_CONSUMER,
                LOCALISATION_EMPTY_NAMESPACE_CONSUMER,
                SCRIPTED_LOCALISATION_EMPTY_NAMESPACE_CONSUMER,
                DEPENDENCY_CLOSURE_EMPTY_CONSUMER,
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blank_optional_texture_is_not_a_dependency() {
        let document = parse_bytes(
            br#"guiTypes={
                instantTextBoxType={
                    name=DateText
                    textureFile=""
                    font=Alias
                }
            }"#,
        );
        let root = parse_gui_document(&document).pop().expect("text root");
        let mut fonts = BTreeSet::new();
        let mut textures = BTreeSet::new();
        collect_element_dependencies(&root, &mut fonts, &mut textures).unwrap();
        assert_eq!(fonts, BTreeSet::from(["Alias".to_string()]));
        assert!(textures.is_empty());
    }
}
