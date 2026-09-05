
use crate::assets::AssetResolver;
use crate::definition_admission::{GuiDefinitionCatalog, ResolvedGuiDefinitionFile};
use crate::frontend_meta::DiscoveredFrontendComposition;
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, Measure, MeasurePair};
use crate::loc::Localisation;
use crate::paradox_text::{Scalar, Value};
use portable_content_manifest::{
    authored_ui::{
        canonical_localisation_language, engine_recursive_path_cmp, safe_language_directory,
        AUTHORED_GRAPHICS_DEFINES_PATH,
    },
    normalize_game_relative_path, UsedContentKey,
};
use replay_core::ValidatedContentAdmission;
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const AUTHORED_ROLE_INDEX_SCHEMA: &str = "replay.authored-gui-role-index.v8";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredElementRole {
    pub element_id: String,
    pub element_type: String,
    pub authored_name: String,
    pub child_path: Vec<u32>,
    pub authored_one_based_frame: u32,
    pub sprite_refs: Vec<String>,
    pub sprites: Vec<AuthoredSpriteRole>,
    pub direct_texture_refs: Vec<AuthoredDirectTextureRef>,
    pub direct_textures: Vec<AuthoredDirectTextureRole>,
    pub font_ref: Option<String>,
    pub font: Option<AuthoredFontRole>,
    pub localisation_or_literal_ref: Option<String>,
    pub localisation_content: Option<UsedContentKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredSpriteRole {
    pub sprite_name: String,
    pub sprite_kind: String,
    pub frame_count: u32,
    pub authored_effect_file: Option<String>,
    pub effective_shader_file: String,
    pub definition_content: UsedContentKey,
    pub texture_slots: Vec<AuthoredSpriteTextureRole>,
    pub effect_content: UsedContentKey,
    pub effect_include_content: Vec<UsedContentKey>,
    pub authored_color_rgba_f32_bits: [u32; 4],
    pub authored_color_was_explicit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredSpriteTextureRole {
    pub authored_slot_key: String,
    /// The normalized texture path the definition declared; the bound content
    /// must be this path's exact or unambiguous same-stem resolution.
    pub authored_path: String,
    pub content: UsedContentKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredDirectTextureRole {
    pub authored_field: String,
    pub authored_path: String,
    pub content: UsedContentKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredDirectTextureRef {
    pub authored_field: String,
    pub authored_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredFontRole {
    pub font_id: String,
    pub selected_language: String,
    pub selected_variant_index: usize,
    pub variants: Vec<AuthoredFontVariantRole>,
    pub base_color: Option<AuthoredPackedColorRole>,
    pub base_border_color: Option<AuthoredPackedColorRole>,
    pub font_local_textcolors: Option<AuthoredTextColorTableRole>,
    pub global_textcolor_tables: Vec<AuthoredTextColorTableRole>,
    pub text_effect_content: UsedContentKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredFontVariantRole {
    pub registration_source_content: UsedContentKey,
    pub ordered_languages: Vec<String>,
    pub ordered_descriptors: Vec<AuthoredFontDescriptorRole>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredFontDescriptorRole {
    pub descriptor_content: UsedContentKey,
    pub ordered_pages: Vec<AuthoredFontAtlasRole>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredFontAtlasRole {
    pub page_id: u32,
    pub content: UsedContentKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredPackedColorRole {
    pub source_content: UsedContentKey,
    pub authored_token: String,
    pub argb: u32,
}

impl AuthoredPackedColorRole {
    pub fn rgba8(&self) -> [u8; 4] {
        [
            ((self.argb >> 16) & 0xff) as u8,
            ((self.argb >> 8) & 0xff) as u8,
            (self.argb & 0xff) as u8,
            ((self.argb >> 24) & 0xff) as u8,
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredTextColorTableRole {
    pub source_content: UsedContentKey,
    pub ordered_entries: Vec<AuthoredTextColorEntryRole>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredTextColorEntryRole {
    pub code: String,
    pub ordered_components: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredSurfaceRole {
    pub surface_id: String,
    pub root_window_id: String,
    pub source_file: String,
    pub source_content: UsedContentKey,
    pub authored_tree_identity_sha256: String,
    pub complete_dependency_closure: bool,
    pub missing_dependencies: Vec<String>,
    pub dependency_content: Vec<UsedContentKey>,
    pub elements: Vec<AuthoredElementRole>,
    pub tooltip_shell: Option<AuthoredTooltipShellRole>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredTooltipShellRole {
    pub graphics_defines_content: UsedContentKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedGuiRoleIndex {
    pub schema: String,
    pub manifest_digest_sha256: String,
    pub product_modset_sha256: String,
    pub localisation_language: String,
    pub index_identity_sha256: String,
    pub surfaces: Vec<AuthoredSurfaceRole>,
    pub dynamic_sprites: Vec<AuthoredSpriteRole>,
    pub dynamic_sprite_missing_dependencies: Vec<String>,
    pub dynamic_dependency_content: Vec<UsedContentKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrictAuthoredPanelAdmission {
    index_identity_sha256: String,
    window_id: String,
    surface_id: String,
    source_content: UsedContentKey,
    dependency_content: Vec<UsedContentKey>,
    element_roles: Vec<AuthoredElementRole>,
    tooltip_shell: Option<AuthoredTooltipShellRole>,
    authored_tree_identity_sha256: String,
    authored_root: GuiElement,
}

impl Eq for StrictAuthoredPanelAdmission {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedPanelSurface {
    pub(crate) panel: StrictAuthoredPanelAdmission,
    pub(crate) index: AuthenticatedGuiRoleIndex,
    pub(crate) frontend: DiscoveredFrontendComposition,
    pub(crate) assets: AssetResolver,
    pub(crate) admission: ValidatedContentAdmission,
    pub(crate) localisation_language: String,
    pub(crate) localisation: Localisation,
    manifest_digest_sha256: String,
    product_modset_sha256: String,
}

impl AuthenticatedPanelSurface {
    pub fn manifest_digest_sha256(&self) -> &str {
        &self.manifest_digest_sha256
    }

    pub fn product_modset_sha256(&self) -> &str {
        &self.product_modset_sha256
    }

    pub fn window_id(&self) -> &str {
        self.panel.window_id()
    }

    pub fn surface_id(&self) -> &str {
        self.panel.surface_id()
    }

    pub fn source_content(&self) -> &UsedContentKey {
        self.panel.source_content()
    }

    pub fn element_roles(&self) -> &[AuthoredElementRole] {
        self.panel.element_roles()
    }

    /// that independent lifetime authority.
    pub fn frontend_composition(&self) -> &DiscoveredFrontendComposition {
        &self.frontend
    }

    pub(crate) fn element_role(&self, child_path: &[u32]) -> Option<&AuthoredElementRole> {
        self.panel.element_role(child_path)
    }

    pub(crate) fn authored_element(&self, child_path: &[u32]) -> Option<&GuiElement> {
        self.panel.authored_element(child_path)
    }

    pub(crate) fn font_binding(&self, font_id: &str) -> Result<Option<&AuthoredFontRole>, String> {
        self.panel.font_binding(font_id)
    }

    pub(crate) fn tooltip_shell(&self) -> Option<&AuthoredTooltipShellRole> {
        self.panel.tooltip_shell()
    }

    pub(crate) fn dynamic_sprite(&self, sprite_name: &str) -> Option<&AuthoredSpriteRole> {
        self.index
            .dynamic_sprites
            .iter()
            .find(|sprite| sprite.sprite_name.eq_ignore_ascii_case(sprite_name))
    }
}

impl StrictAuthoredPanelAdmission {
    pub fn index_identity_sha256(&self) -> &str {
        &self.index_identity_sha256
    }

    pub fn window_id(&self) -> &str {
        &self.window_id
    }

    pub fn surface_id(&self) -> &str {
        &self.surface_id
    }

    pub fn source_content(&self) -> &UsedContentKey {
        &self.source_content
    }

    pub fn dependency_content(&self) -> &[UsedContentKey] {
        &self.dependency_content
    }

    pub fn element_roles(&self) -> &[AuthoredElementRole] {
        &self.element_roles
    }

    pub fn element_role(&self, child_path: &[u32]) -> Option<&AuthoredElementRole> {
        self.element_roles
            .iter()
            .find(|element| element.child_path == child_path)
    }

    pub fn authored_element(&self, child_path: &[u32]) -> Option<&GuiElement> {
        let mut element = &self.authored_root;
        for &ordinal in child_path {
            element = element.children.get(usize::try_from(ordinal).ok()?)?;
        }
        Some(element)
    }

    pub fn font_binding(&self, font_id: &str) -> Result<Option<&AuthoredFontRole>, String> {
        let mut binding = None;
        for element in &self.element_roles {
            let Some(candidate) = element.font.as_ref() else {
                continue;
            };
            if candidate.font_id != font_id {
                continue;
            }
            if binding.is_some_and(|prior| prior != candidate) {
                return Err(format!(
                    "authored panel carries inconsistent bindings for font {font_id:?}"
                ));
            }
            binding = Some(candidate);
        }
        Ok(binding)
    }

    pub fn authored_root(&self) -> &GuiElement {
        &self.authored_root
    }

    pub fn authored_tree_identity_sha256(&self) -> &str {
        &self.authored_tree_identity_sha256
    }

    pub fn tooltip_shell(&self) -> Option<&AuthoredTooltipShellRole> {
        self.tooltip_shell.as_ref()
    }
}

impl AuthenticatedGuiRoleIndex {
    pub fn seal_panel_surface(
        &self,
        resolver: &AssetResolver,
        localisation_language: &str,
        localisation: &Localisation,
        admission: &ValidatedContentAdmission,
        panel: StrictAuthoredPanelAdmission,
        frontend: DiscoveredFrontendComposition,
    ) -> Result<AuthenticatedPanelSurface, String> {
        self.validate_panel_token(admission, &panel)?;
        if !resolver.is_admitted_for(admission) {
            return Err("authored panel resolver belongs to a different replay".into());
        }
        let language = canonical_localisation_language(localisation_language)?;
        if language != self.localisation_language {
            return Err("authored panel localisation belongs to a different language".into());
        }
        if !crate::frontend_meta::revalidate_discovered_frontend_chrome(
            &frontend, self, admission, &panel,
        ) {
            return Err(
                "authored panel frontend composition differs from the exact panel/content admission"
                    .into(),
            );
        }
        Ok(AuthenticatedPanelSurface {
            panel,
            index: self.clone(),
            frontend,
            assets: resolver.clone(),
            admission: admission.clone(),
            localisation_language: language,
            localisation: localisation.clone(),
            manifest_digest_sha256: admission.manifest_digest_sha256().to_string(),
            product_modset_sha256: admission.product_modset_sha256().to_string(),
        })
    }

    pub fn emit_with_game_resolver_and_language(
        resolver: &AssetResolver,
        admission: &ValidatedContentAdmission,
        game_resolver: Option<&map_viewer::GameFileResolver>,
        localisation_language: &str,
    ) -> Result<(Self, GuiDefinitionCatalog), String> {
        if !resolver.is_admitted_for(admission) {
            return Err("authored GUI role index requires the matching content admission".into());
        }
        let gui_files = resolver
            .list_files_recursive("interface", Some("gui"))
            .map_err(|error| error.to_string())?;
        let catalog = crate::definition_admission::load_effective_definition_catalog(
            gui_files
                .iter()
                .map(|file| {
                    ResolvedGuiDefinitionFile::read(resolver, file)
                })
                .collect::<crate::Result<Vec<_>>>().map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;

        let mut sprites = SpriteRegistry::new();
        let mut gfx_files = resolver
            .list_files_recursive("interface", Some("gfx"))
            .map_err(|error| error.to_string())?;
        gfx_files.sort_by(|left, right| engine_recursive_path_cmp(&left.rel_path, &right.rel_path));
        for file in gfx_files {
            sprites
                .parse_content(&resolver.read_file(&file).map_err(|error| error.to_string())?, &file.rel_path)
                .map_err(|error| error.to_string())?;
        }
        if sprites.decode_loss_events() != 0 {
            return Err(format!(
                "authored GUI role index refuses destructive GFX decode loss: {:?}",
                sprites.decode_lossy_files()
            ));
        }

        let mut font_ids = BTreeSet::new();
        for root in catalog.effective_roots.iter().filter(|root| root.admitted) {
            collect_font_refs(&root.root, &mut font_ids);
        }
        let font_paths = font_ids
            .into_iter()
            .map(|font_id| {
                let paths = match game_resolver {
                    Some(game_resolver) => {
                        crate::font::authored_bitmap_font_registry_paths_for_language(
                            game_resolver,
                            &font_id,
                            localisation_language,
                        )
                        .map_err(|error| error.to_string())
                    }
                    None => Err(format!(
                        "font:{font_id}:central effective-VFS font resolver unavailable"
                    )),
                };
                (font_id, paths)
            })
            .collect::<BTreeMap<_, _>>();
        let localisation = localisation_sources(resolver, localisation_language)?;
        let canonical_language = canonical_localisation_language(localisation_language)?;
        let index = Self::from_catalog(
            &catalog,
            admission,
            |path| {
                resolver
                    .resolve_optional_file(path)
                    .map_err(|error| error.to_string())
            },
            &sprites,
            &font_paths,
            &localisation,
            &canonical_language,
        )?;
        Ok((index, catalog))
    }

    fn from_catalog<F>(
        catalog: &GuiDefinitionCatalog,
        admission: &ValidatedContentAdmission,
        mut resolve_optional: F,
        sprites: &SpriteRegistry,
        font_paths: &BTreeMap<String, Result<crate::font::AuthoredBitmapFontRegistryPaths, String>>,
        localisation_sources: &BTreeMap<String, String>,
        localisation_language: &str,
    ) -> Result<Self, String>
    where
        F: FnMut(&str) -> Result<Option<crate::assets::ResolvedFile>, String>,
    {
        if catalog.effective_roots.is_empty() {
            return Err("authored GUI role index has no effective roots".into());
        }
        let mut folded_roots = BTreeMap::<String, (String, String)>::new();
        let mut surfaces = Vec::new();
        for root in catalog.effective_roots.iter().filter(|root| root.admitted) {
            let folded = root.root.name.to_ascii_lowercase();
            if let Some((prior_file, prior_id)) =
                folded_roots.insert(folded, (root.source_file.clone(), root.window_id.clone()))
            {
                return Err(format!(
                    "authored GUI root {:?} is ambiguous between {} ({}) and {} ({})",
                    root.root.name, prior_file, prior_id, root.source_file, root.window_id
                ));
            }
            let source_content = admitted_key(admission, &root.source_file)?.clone();
            let mut elements = Vec::new();
            let element_collection_error = collect_elements(&root.root, &[], &mut elements).err();

            let mut dependencies = BTreeMap::<String, UsedContentKey>::new();
            dependencies.insert(
                source_content.normalized_game_relative_path.clone(),
                source_content.clone(),
            );
            let mut missing = BTreeSet::new();
            if let Some(error) = element_collection_error {
                missing.insert(format!("authored-element-census:{error}"));
                elements.clear();
            }
            for element in &mut elements {
                for sprite_name in &element.sprite_refs {
                    let Some(sprite) = sprites.get(sprite_name) else {
                        missing.insert(format!("sprite:{sprite_name}"));
                        continue;
                    };
                    let definition_content = admitted_key(admission, &sprite.source_file)?.clone();
                    if element.authored_one_based_frame == 0
                        || element.authored_one_based_frame > sprite.no_of_frames
                    {
                        missing.insert(format!(
                            "sprite-frame:{sprite_name}:{}-of-{}",
                            element.authored_one_based_frame, sprite.no_of_frames
                        ));
                        continue;
                    }
                    if let Some(role) = resolve_sprite_role(
                        sprite,
                        definition_content,
                        admission,
                        &mut resolve_optional,
                        &mut dependencies,
                        &mut missing,
                    )? {
                        element.sprites.push(role);
                    }
                }
                for texture in &element.direct_texture_refs {
                    let normalized = match normalize_game_relative_path(&texture.authored_path) {
                        Ok(path) => path,
                        Err(error) => {
                            missing.insert(format!(
                                "direct-texture:{:?}:{error}",
                                texture.authored_path
                            ));
                            continue;
                        }
                    };
                    let resolved = match resolve_optional(&normalized)? {
                        Some(file) => Some(file),
                        None => {
                            match unambiguous_same_stem_texture(&normalized, &mut resolve_optional)? {
                                SameStemTexture::Resolved(file) => Some(file),
                                SameStemTexture::Ambiguous => {
                                    missing.insert(format!(
                                        "direct-texture:{normalized}:ambiguous-same-stem"
                                    ));
                                    continue;
                                }
                                SameStemTexture::Absent => None,
                            }
                        }
                    };
                    match resolved {
                        Some(file) => {
                            let key = admitted_key(admission, &file.rel_path)?.clone();
                            dependencies
                                .insert(key.normalized_game_relative_path.clone(), key.clone());
                            element.direct_textures.push(AuthoredDirectTextureRole {
                                authored_field: texture.authored_field.clone(),
                                authored_path: texture.authored_path.clone(),
                                content: key,
                            });
                        }
                        None => {
                            missing.insert(format!("direct-texture:{normalized}"));
                        }
                    }
                }
                if let Some(font_id) = element.font_ref.as_deref() {
                    match font_paths.get(font_id) {
                        Some(Ok(paths)) => {
                            element.font = resolve_font_role(
                                paths,
                                admission,
                                &mut resolve_optional,
                                &mut dependencies,
                                &mut missing,
                            )?;
                        }
                        Some(Err(error)) => {
                            missing.insert(error.clone());
                        }
                        None => {
                            missing.insert(format!("font:{font_id}:unplanned"));
                        }
                    }
                }
                if let Some(text) = element.localisation_or_literal_ref.as_deref() {
                    if let Some(path) = localisation_sources.get(&text.to_ascii_lowercase()) {
                        match admission.used_content(path) {
                            Some(key) => {
                                dependencies
                                    .insert(key.normalized_game_relative_path.clone(), key.clone());
                                element.localisation_content = Some(key.clone());
                            }
                            None => {
                                missing.insert(format!("localisation:{path}"));
                            }
                        }
                    }
                }
            }
            let is_tooltip_shell = if root.source_file.eq_ignore_ascii_case("interface/core.gui") {
                match is_tooltip_shell_root(&root.root) {
                    Ok(is_shell) => is_shell,
                    Err(error) => {
                        missing.insert(format!("tooltip-shell-structure:{error}"));
                        false
                    }
                }
            } else {
                false
            };
            let tooltip_shell = if is_tooltip_shell {
                match resolve_optional(AUTHORED_GRAPHICS_DEFINES_PATH)? {
                    Some(file) => {
                        let key = admitted_key(admission, &file.rel_path)?.clone();
                        dependencies.insert(key.normalized_game_relative_path.clone(), key.clone());
                        Some(AuthoredTooltipShellRole {
                            graphics_defines_content: key,
                        })
                    }
                    None => {
                        missing.insert(format!(
                            "tooltip-graphics-defines:{AUTHORED_GRAPHICS_DEFINES_PATH}"
                        ));
                        None
                    }
                }
            } else {
                None
            };
            let dependency_content = dependencies.into_values().collect::<Vec<_>>();
            surfaces.push(AuthoredSurfaceRole {
                surface_id: root.root.name.clone(),
                root_window_id: root.window_id.clone(),
                source_file: root.source_file.clone(),
                source_content,
                authored_tree_identity_sha256: authored_tree_identity_sha256(&root.root),
                complete_dependency_closure: missing.is_empty(),
                missing_dependencies: missing.into_iter().collect(),
                dependency_content,
                elements,
                tooltip_shell,
            });
        }
        if surfaces
            .iter()
            .filter(|surface| surface.tooltip_shell.is_some())
            .count()
            > 1
        {
            return Err(
                "interface/core.gui has multiple structural tooltip-shell candidates".into(),
            );
        }
        let mut dynamic_sprites = Vec::new();
        let mut dynamic_dependencies = BTreeMap::<String, UsedContentKey>::new();
        let mut dynamic_missing = BTreeSet::new();
        for sprite in sprites.effective_sprites() {
            let definition_content = admitted_key(admission, &sprite.source_file)?.clone();
            if let Some(role) = resolve_sprite_role(
                sprite,
                definition_content,
                admission,
                &mut resolve_optional,
                &mut dynamic_dependencies,
                &mut dynamic_missing,
            )? {
                dynamic_sprites.push(role);
            }
        }
        surfaces.sort_by(|left, right| {
            left.surface_id
                .to_ascii_lowercase()
                .cmp(&right.surface_id.to_ascii_lowercase())
                .then_with(|| left.surface_id.cmp(&right.surface_id))
        });

        let mut index = Self {
            schema: AUTHORED_ROLE_INDEX_SCHEMA.into(),
            manifest_digest_sha256: admission.manifest_digest_sha256().into(),
            product_modset_sha256: admission.product_modset_sha256().into(),
            localisation_language: localisation_language.to_string(),
            index_identity_sha256: String::new(),
            surfaces,
            dynamic_sprites,
            dynamic_sprite_missing_dependencies: dynamic_missing.into_iter().collect(),
            dynamic_dependency_content: dynamic_dependencies.into_values().collect(),
        };
        index.index_identity_sha256 = index.computed_identity_sha256()?;
        index.validate(admission)?;
        Ok(index)
    }

    pub fn validate(&self, admission: &ValidatedContentAdmission) -> Result<(), String> {
        if self.schema != AUTHORED_ROLE_INDEX_SCHEMA {
            return Err("authored GUI role index schema is unsupported".into());
        }
        if self.manifest_digest_sha256 != admission.manifest_digest_sha256()
            || self.product_modset_sha256 != admission.product_modset_sha256()
        {
            return Err("authored GUI role index names a different replay manifest/modset".into());
        }
        if canonical_localisation_language(&self.localisation_language)?
            != self.localisation_language
        {
            return Err("authored GUI role index language is not canonical".into());
        }
        if self.index_identity_sha256 != self.computed_identity_sha256()? {
            return Err("authored GUI role index identity digest mismatches its rows".into());
        }
        for content in &self.dynamic_dependency_content {
            content.validate()?;
            if admission.require_used_content(&content.normalized_game_relative_path)? != content {
                return Err("dynamic sprite dependency differs from manifest winner".into());
            }
        }
        let mut dynamic_names = BTreeSet::new();
        for sprite in &self.dynamic_sprites {
            if !dynamic_names.insert(sprite.sprite_name.to_ascii_lowercase()) {
                return Err("dynamic sprite registry repeats a folded identity".into());
            }
            validate_sprite_role(sprite, &self.dynamic_dependency_content)?;
        }
        let mut roots = BTreeSet::new();
        let mut windows = BTreeSet::new();
        for surface in &self.surfaces {
            if surface.surface_id.trim().is_empty()
                || !roots.insert(surface.surface_id.to_ascii_lowercase())
                || !windows.insert(surface.root_window_id.as_str())
            {
                return Err("authored GUI role index repeats or blanks a surface identity".into());
            }
            if surface.complete_dependency_closure != surface.missing_dependencies.is_empty() {
                return Err("authored GUI role index closure completeness is inconsistent".into());
            }
            if surface.authored_tree_identity_sha256.len() != 64
                || !surface
                    .authored_tree_identity_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                return Err("authored GUI role index has an invalid tree identity".into());
            }
            for content in &surface.dependency_content {
                content.validate()?;
                let admitted =
                    admission.require_used_content(&content.normalized_game_relative_path)?;
                if admitted != content {
                    return Err("authored GUI role dependency differs from manifest winner".into());
                }
            }
            if !surface
                .dependency_content
                .iter()
                .any(|content| content == &surface.source_content)
            {
                return Err("authored GUI role closure omits its definition source".into());
            }
            if let Some(shell) = &surface.tooltip_shell {
                if !surface
                    .source_file
                    .eq_ignore_ascii_case("interface/core.gui")
                    || !shell
                        .graphics_defines_content
                        .normalized_game_relative_path
                        .eq_ignore_ascii_case(AUTHORED_GRAPHICS_DEFINES_PATH)
                    || !surface
                        .dependency_content
                        .iter()
                        .any(|content| content == &shell.graphics_defines_content)
                {
                    return Err("authored tooltip shell content join is inconsistent".into());
                }
            }
            for element in &surface.elements {
                if element.authored_one_based_frame == 0 {
                    return Err("authored GUI role carries zero sprite frame".into());
                }
                if surface.complete_dependency_closure
                    && (element.sprites.len() != element.sprite_refs.len()
                        || element.direct_textures.len() != element.direct_texture_refs.len())
                {
                    return Err(
                        "complete authored GUI element has an incomplete asset binding".into(),
                    );
                }
                for (reference, texture) in element
                    .direct_texture_refs
                    .iter()
                    .zip(&element.direct_textures)
                {
                    if reference.authored_field != texture.authored_field
                        || reference.authored_path != texture.authored_path
                        || !surface
                            .dependency_content
                            .iter()
                            .any(|dependency| dependency == &texture.content)
                    {
                        return Err("authored direct texture binding is inconsistent".into());
                    }
                }
                for sprite in &element.sprites {
                    if element.authored_one_based_frame > sprite.frame_count {
                        return Err("authored sprite kind/frame binding is invalid".into());
                    }
                    validate_sprite_role(sprite, &surface.dependency_content)?;
                }
                if let Some(content) = &element.localisation_content {
                    if element.localisation_or_literal_ref.is_none()
                        || !surface
                            .dependency_content
                            .iter()
                            .any(|dependency| dependency == content)
                    {
                        return Err(
                            "authored localisation binding is absent from surface closure".into(),
                        );
                    }
                }
                match (&element.font_ref, &element.font) {
                    (None, None) => {}
                    (Some(font_id), Some(font)) if font.font_id == font_id.as_str() => {
                        if font.selected_language != self.localisation_language {
                            return Err(
                                "authored font selected language differs from role index".into()
                            );
                        }
                        validate_font_role(font, &surface.dependency_content)?;
                    }
                    (Some(_), None) if !surface.complete_dependency_closure => {}
                    _ => {
                        return Err("authored element font identity/binding is inconsistent".into())
                    }
                }
            }
        }
        Ok(())
    }

    pub fn surface_by_id(&self, surface_id: &str) -> Option<&AuthoredSurfaceRole> {
        self.surfaces
            .iter()
            .find(|surface| surface.surface_id.eq_ignore_ascii_case(surface_id))
    }

    pub fn tooltip_shell_surface(&self) -> Option<&AuthoredSurfaceRole> {
        self.surfaces
            .iter()
            .find(|surface| surface.tooltip_shell.is_some())
    }

    pub fn admit_dynamic_sprite<'a>(
        &'a self,
        admission: &ValidatedContentAdmission,
        sprite_name: &str,
    ) -> Result<&'a AuthoredSpriteRole, String> {
        self.validate(admission)?;
        self.dynamic_sprites
            .iter()
            .find(|sprite| sprite.sprite_name.eq_ignore_ascii_case(sprite_name))
            .ok_or_else(|| {
                format!(
                    "dynamic authored sprite {sprite_name:?} has no complete authenticated binding"
                )
            })
    }

    pub fn validate_panel_token<'a>(
        &'a self,
        admission: &ValidatedContentAdmission,
        panel: &StrictAuthoredPanelAdmission,
    ) -> Result<&'a AuthoredSurfaceRole, String> {
        self.validate(admission)?;
        if panel.index_identity_sha256 != self.index_identity_sha256 {
            return Err("authored GUI panel token belongs to a different role index".into());
        }
        let surface = self
            .surface_by_id(panel.surface_id())
            .ok_or_else(|| "authored GUI panel token names an unknown surface".to_string())?;
        if !surface.complete_dependency_closure
            || surface.source_content != panel.source_content
            || surface.dependency_content != panel.dependency_content
            || surface.elements != panel.element_roles
            || surface.tooltip_shell != panel.tooltip_shell
            || surface.authored_tree_identity_sha256 != panel.authored_tree_identity_sha256
            || authored_tree_identity_sha256(&panel.authored_root)
                != surface.authored_tree_identity_sha256
            || !panel
                .authored_root
                .name
                .eq_ignore_ascii_case(&surface.surface_id)
        {
            return Err("authored GUI panel token differs from its indexed closure".into());
        }
        Ok(surface)
    }

    pub fn surface_for_window<'a>(
        &'a self,
        catalog: &GuiDefinitionCatalog,
        window_id: &str,
    ) -> Option<&'a AuthoredSurfaceRole> {
        let window = catalog
            .admitted_windows
            .iter()
            .find(|window| window.window_id == window_id)?;
        let root_name = window.hierarchy_path.split('/').next()?;
        self.surfaces.iter().find(|surface| {
            surface.source_file == window.source_file
                && surface.surface_id.eq_ignore_ascii_case(root_name)
        })
    }

    pub fn admit_surface(
        &self,
        catalog: &GuiDefinitionCatalog,
        admission: &ValidatedContentAdmission,
        source_content: &UsedContentKey,
        surface_id: &str,
    ) -> Result<StrictAuthoredPanelAdmission, String> {
        self.validate(admission)?;
        let mut matches = self.surfaces.iter().filter(|surface| {
            &surface.source_content == source_content
                && surface.surface_id.eq_ignore_ascii_case(surface_id)
        });
        let surface = matches
            .next()
            .ok_or_else(|| "authored GUI source/root identity is not discovered".to_string())?;
        if matches.next().is_some() {
            return Err("authored GUI source/root identity is ambiguous".into());
        }
        self.admit_panel(catalog, admission, &surface.root_window_id)
    }

    pub fn admit_all_surfaces(
        &self,
        catalog: &GuiDefinitionCatalog,
        admission: &ValidatedContentAdmission,
    ) -> Result<Vec<StrictAuthoredPanelAdmission>, String> {
        self.validate(admission)?;
        self.surfaces
            .iter()
            .map(|surface| self.admit_panel(catalog, admission, &surface.root_window_id))
            .collect()
    }

    pub fn admit_panel(
        &self,
        catalog: &GuiDefinitionCatalog,
        admission: &ValidatedContentAdmission,
        window_id: &str,
    ) -> Result<StrictAuthoredPanelAdmission, String> {
        self.validate(admission)?;
        let surface = self
            .surface_for_window(catalog, window_id)
            .ok_or_else(|| format!("authored GUI window is not discovered: {window_id}"))?;
        if !surface.complete_dependency_closure {
            return Err(format!(
                "authored GUI surface {:?} has incomplete dependency closure: {:?}",
                surface.surface_id, surface.missing_dependencies
            ));
        }
        let authored_root = catalog
            .effective_roots
            .iter()
            .find(|root| {
                root.admitted
                    && root.window_id == window_id
                    && root.root.name.eq_ignore_ascii_case(&surface.surface_id)
                    && root.source_file == surface.source_file
            })
            .ok_or_else(|| {
                format!(
                    "authored GUI window has no admitted parsed root retained by its catalog: {window_id}"
                )
            })?
            .root
            .clone();
        let tree_identity = authored_tree_identity_sha256(&authored_root);
        if tree_identity != surface.authored_tree_identity_sha256 {
            return Err(format!(
                "authored GUI window tree differs from the manifest-bound role index: {window_id}"
            ));
        }
        Ok(StrictAuthoredPanelAdmission {
            index_identity_sha256: self.index_identity_sha256.clone(),
            window_id: window_id.to_string(),
            surface_id: surface.surface_id.clone(),
            source_content: surface.source_content.clone(),
            dependency_content: surface.dependency_content.clone(),
            element_roles: surface.elements.clone(),
            tooltip_shell: surface.tooltip_shell.clone(),
            authored_tree_identity_sha256: tree_identity,
            authored_root,
        })
    }

    fn computed_identity_sha256(&self) -> Result<String, String> {
        #[derive(Serialize)]
        struct Payload<'a> {
            schema: &'a str,
            manifest_digest_sha256: &'a str,
            product_modset_sha256: &'a str,
            localisation_language: &'a str,
            surfaces: &'a [AuthoredSurfaceRole],
            dynamic_sprites: &'a [AuthoredSpriteRole],
            dynamic_sprite_missing_dependencies: &'a [String],
            dynamic_dependency_content: &'a [UsedContentKey],
        }
        serde_json::to_vec(&Payload {
            schema: &self.schema,
            manifest_digest_sha256: &self.manifest_digest_sha256,
            product_modset_sha256: &self.product_modset_sha256,
            localisation_language: &self.localisation_language,
            surfaces: &self.surfaces,
            dynamic_sprites: &self.dynamic_sprites,
            dynamic_sprite_missing_dependencies: &self.dynamic_sprite_missing_dependencies,
            dynamic_dependency_content: &self.dynamic_dependency_content,
        })
        .map(|bytes| Sha256::hex(&bytes))
        .map_err(|error| error.to_string())
    }
}

pub(crate) fn authored_tree_identity_sha256(root: &GuiElement) -> String {
    fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(bytes);
    }

    fn push_string(out: &mut Vec<u8>, value: &str) {
        push_bytes(out, value.as_bytes());
    }

    fn push_optional_string(out: &mut Vec<u8>, value: Option<&str>) {
        out.push(u8::from(value.is_some()));
        if let Some(value) = value {
            push_string(out, value);
        }
    }

    fn push_measure(out: &mut Vec<u8>, value: &Measure) {
        push_string(out, &value.raw);
        push_optional_string(out, value.value.as_deref());
        out.push(u8::from(value.is_percent));
    }

    fn push_measure_pair(out: &mut Vec<u8>, value: Option<&MeasurePair>) {
        out.push(u8::from(value.is_some()));
        if let Some(value) = value {
            push_measure(out, &value.x);
            push_measure(out, &value.y);
        }
    }

    fn push_value(out: &mut Vec<u8>, value: &Value) {
        match value {
            Value::Scalar(Scalar::Bare(value)) => {
                out.push(0);
                push_string(out, value);
            }
            Value::Scalar(Scalar::Quoted(value)) => {
                out.push(1);
                push_string(out, value);
            }
            Value::Object(entries) => {
                out.push(2);
                out.extend_from_slice(&(entries.len() as u64).to_le_bytes());
                for entry in entries {
                    push_optional_string(out, entry.key.as_deref());
                    out.extend_from_slice(&(entry.line as u64).to_le_bytes());
                    out.extend_from_slice(&(entry.column as u64).to_le_bytes());
                    push_value(out, &entry.value);
                }
            }
        }
    }

    fn push_element(out: &mut Vec<u8>, element: &GuiElement) {
        push_string(out, &element.element_type);
        push_string(out, &element.name);
        push_measure_pair(out, element.position.as_ref());
        push_measure_pair(out, element.size.as_ref());
        push_optional_string(out, element.orientation.as_deref());
        out.extend_from_slice(&(element.sprite_refs.len() as u64).to_le_bytes());
        for sprite in &element.sprite_refs {
            push_string(out, sprite);
        }
        push_optional_string(out, element.font.as_deref());
        push_optional_string(out, element.text.as_deref());
        push_optional_string(out, element.format.as_deref());
        out.extend_from_slice(&(element.children.len() as u64).to_le_bytes());
        for child in &element.children {
            push_element(out, child);
        }
        push_value(out, &element.raw);
    }

    let mut bytes = Vec::new();
    push_element(&mut bytes, root);
    Sha256::hex(&bytes)
}

fn resolve_font_role<F>(
    paths: &crate::font::AuthoredBitmapFontRegistryPaths,
    admission: &ValidatedContentAdmission,
    resolve_optional: &mut F,
    dependencies: &mut BTreeMap<String, UsedContentKey>,
    missing: &mut BTreeSet<String>,
) -> Result<Option<AuthoredFontRole>, String>
where
    F: FnMut(&str) -> Result<Option<crate::assets::ResolvedFile>, String>,
{
    let mut complete = true;
    let mut variants = Vec::new();
    for variant in &paths.variants {
        let registration_source_content = match resolve_optional(&variant.registration_source)? {
            Some(file) => {
                let key = admitted_key(admission, &file.rel_path)?.clone();
                dependencies.insert(key.normalized_game_relative_path.clone(), key.clone());
                Some(key)
            }
            None => {
                missing.insert(format!(
                    "font:{}:registration:{}",
                    paths.font_id, variant.registration_source
                ));
                complete = false;
                None
            }
        };
        let mut ordered_descriptors = Vec::new();
        for descriptor in &variant.ordered_descriptors {
            let descriptor_content = match resolve_optional(&descriptor.descriptor)? {
                Some(file) => {
                    let key = admitted_key(admission, &file.rel_path)?.clone();
                    dependencies.insert(key.normalized_game_relative_path.clone(), key.clone());
                    Some(key)
                }
                None => {
                    missing.insert(format!(
                        "font:{}:descriptor:{}",
                        paths.font_id, descriptor.descriptor
                    ));
                    complete = false;
                    None
                }
            };
            let mut ordered_pages = Vec::new();
            for page in &descriptor.ordered_pages {
                match resolve_optional(&page.path)? {
                    Some(file) => {
                        let key = admitted_key(admission, &file.rel_path)?.clone();
                        dependencies.insert(key.normalized_game_relative_path.clone(), key.clone());
                        ordered_pages.push(AuthoredFontAtlasRole {
                            page_id: page.page_id,
                            content: key,
                        });
                    }
                    None => {
                        missing.insert(format!("font:{}:page:{}", paths.font_id, page.path));
                        complete = false;
                    }
                }
            }
            if let Some(descriptor_content) = descriptor_content {
                if ordered_pages.len() == descriptor.ordered_pages.len() {
                    ordered_descriptors.push(AuthoredFontDescriptorRole {
                        descriptor_content,
                        ordered_pages,
                    });
                }
            }
        }
        if let Some(registration_source_content) = registration_source_content {
            if ordered_descriptors.len() == variant.ordered_descriptors.len() {
                variants.push(AuthoredFontVariantRole {
                    registration_source_content,
                    ordered_languages: variant.ordered_languages.clone(),
                    ordered_descriptors,
                });
            }
        }
    }
    let text_effect_content = match resolve_optional(
        portable_content_manifest::authored_ui::AUTHORED_BITMAP_FONT_TEXT_EFFECT_PATH,
    )? {
        Some(file) => {
            let key = admitted_key(admission, &file.rel_path)?.clone();
            dependencies.insert(key.normalized_game_relative_path.clone(), key.clone());
            Some(key)
        }
        None => {
            missing.insert(format!(
                "font:{}:{}",
                paths.font_id,
                portable_content_manifest::authored_ui::AUTHORED_BITMAP_FONT_TEXT_EFFECT_PATH
            ));
            complete = false;
            None
        }
    };
    let Some(base_source) = variants
        .first()
        .map(|variant| variant.registration_source_content.clone())
    else {
        return Ok(None);
    };
    let base_color = paths
        .base_color
        .as_ref()
        .map(|color| AuthoredPackedColorRole {
            source_content: base_source.clone(),
            authored_token: color.authored_token.clone(),
            argb: color.argb,
        });
    let base_border_color = paths
        .base_border_color
        .as_ref()
        .map(|color| AuthoredPackedColorRole {
            source_content: base_source.clone(),
            authored_token: color.authored_token.clone(),
            argb: color.argb,
        });
    let font_local_textcolors =
        paths
            .font_local_textcolors
            .as_ref()
            .map(|entries| AuthoredTextColorTableRole {
                source_content: base_source,
                ordered_entries: convert_textcolors(entries),
            });
    let mut global_textcolor_tables = Vec::new();
    for table in &paths.global_textcolor_tables {
        match resolve_optional(&table.registration_source)? {
            Some(file) => {
                let key = admitted_key(admission, &file.rel_path)?.clone();
                dependencies.insert(key.normalized_game_relative_path.clone(), key.clone());
                global_textcolor_tables.push(AuthoredTextColorTableRole {
                    source_content: key,
                    ordered_entries: convert_textcolors(&table.ordered_entries),
                });
            }
            None => {
                missing.insert(format!(
                    "font:{}:global-textcolors:{}",
                    paths.font_id, table.registration_source
                ));
                complete = false;
            }
        }
    }
    if !complete
        || variants.len() != paths.variants.len()
        || text_effect_content.is_none()
        || paths.selected_variant_index >= variants.len()
    {
        return Ok(None);
    }
    Ok(Some(AuthoredFontRole {
        font_id: paths.font_id.clone(),
        selected_language: paths.selected_language.clone(),
        selected_variant_index: paths.selected_variant_index,
        variants,
        base_color,
        base_border_color,
        font_local_textcolors,
        global_textcolor_tables,
        text_effect_content: text_effect_content.expect("presence checked above"),
    }))
}

fn convert_textcolors(
    entries: &[crate::font::AuthoredTextColorEntry],
) -> Vec<AuthoredTextColorEntryRole> {
    entries
        .iter()
        .map(|entry| AuthoredTextColorEntryRole {
            code: entry.code.clone(),
            ordered_components: entry.ordered_components.clone(),
        })
        .collect()
}

fn validate_font_role(
    font: &AuthoredFontRole,
    dependency_content: &[UsedContentKey],
) -> Result<(), String> {
    if font.font_id.trim().is_empty()
        || font.selected_language.trim().is_empty()
        || font.variants.is_empty()
        || !font.variants[0].ordered_languages.is_empty()
    {
        return Err("authored font identity/base variant is invalid".into());
    }
    let expected_selected = font
        .variants
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, variant)| {
            variant
                .ordered_languages
                .iter()
                .any(|language| language == &font.selected_language)
        })
        .map(|(index, _)| index)
        .unwrap_or(0);
    if font.selected_variant_index != expected_selected {
        return Err("authored font selected variant violates first exact match".into());
    }
    let base_source = &font.variants[0].registration_source_content;
    let mut require_content = |content: &UsedContentKey| -> Result<(), String> {
        if dependency_content
            .iter()
            .any(|dependency| dependency == content)
        {
            Ok(())
        } else {
            Err("authored font binding is absent from surface closure".into())
        }
    };
    for variant in &font.variants {
        require_content(&variant.registration_source_content)?;
        if variant.ordered_descriptors.is_empty()
            || variant
                .ordered_languages
                .iter()
                .any(|language| language.is_empty())
        {
            return Err("authored font variant has an empty language/descriptor".into());
        }
        for descriptor in &variant.ordered_descriptors {
            require_content(&descriptor.descriptor_content)?;
            if descriptor.ordered_pages.is_empty()
                || descriptor
                    .ordered_pages
                    .windows(2)
                    .any(|pair| pair[0].page_id >= pair[1].page_id)
            {
                return Err("authored font page IDs are absent, repeated, or reordered".into());
            }
            for page in &descriptor.ordered_pages {
                require_content(&page.content)?;
            }
        }
    }
    for color in font.base_color.iter().chain(font.base_border_color.iter()) {
        if &color.source_content != base_source
            || parse_packed_color_token(&color.authored_token) != Some(color.argb)
        {
            return Err("authored font packed color provenance is inconsistent".into());
        }
        require_content(&color.source_content)?;
    }
    for table in font
        .font_local_textcolors
        .iter()
        .chain(font.global_textcolor_tables.iter())
    {
        require_content(&table.source_content)?;
        for entry in &table.ordered_entries {
            if entry.code.chars().count() != 1
                || entry.ordered_components.len() != 3
                || entry.ordered_components.iter().any(|component| {
                    component
                        .parse::<u32>()
                        .ok()
                        .map_or(true, |component| component > 255)
                })
            {
                return Err("authored font text-color table is malformed".into());
            }
        }
    }
    if font
        .font_local_textcolors
        .as_ref()
        .is_some_and(|table| &table.source_content != base_source)
    {
        return Err("authored font-local text colors do not name the base source".into());
    }
    if !font
        .text_effect_content
        .normalized_game_relative_path
        .eq_ignore_ascii_case(
            portable_content_manifest::authored_ui::AUTHORED_BITMAP_FONT_TEXT_EFFECT_PATH,
        )
    {
        return Err("authored font text effect source is inconsistent".into());
    }
    require_content(&font.text_effect_content)?;
    Ok(())
}

fn parse_packed_color_token(token: &str) -> Option<u32> {
    let token = token.trim();
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else {
        token.parse().ok()
    }
}

fn resolve_sprite_role<F>(
    sprite: &crate::gfx::SpriteDefinition,
    definition_content: UsedContentKey,
    admission: &ValidatedContentAdmission,
    resolve_optional: &mut F,
    dependencies: &mut BTreeMap<String, UsedContentKey>,
    missing: &mut BTreeSet<String>,
) -> Result<Option<AuthoredSpriteRole>, String>
where
    F: FnMut(&str) -> Result<Option<crate::assets::ResolvedFile>, String>,
{
    dependencies.insert(
        definition_content.normalized_game_relative_path.clone(),
        definition_content.clone(),
    );
    let mut texture_slots = Vec::new();
    for texture in &sprite.texture_files {
        let normalized = match normalize_game_relative_path(&texture.path) {
            Ok(path) => path,
            Err(error) => {
                missing.insert(format!("sprite:{}:texture-path:{error}", sprite.name));
                return Ok(None);
            }
        };
        let resolved = match resolve_optional(&normalized)? {
            Some(file) => Some(file),
            None => match unambiguous_same_stem_texture(&normalized, resolve_optional)? {
                SameStemTexture::Resolved(file) => Some(file),
                SameStemTexture::Ambiguous => {
                    missing.insert(format!(
                        "sprite:{}:texture:{normalized}:ambiguous-same-stem",
                        sprite.name
                    ));
                    continue;
                }
                SameStemTexture::Absent => None,
            },
        };
        match resolved {
            Some(file) => {
                let key = admitted_key(admission, &file.rel_path)?.clone();
                dependencies.insert(key.normalized_game_relative_path.clone(), key.clone());
                texture_slots.push(AuthoredSpriteTextureRole {
                    authored_slot_key: texture.key.clone(),
                    authored_path: normalized,
                    content: key,
                });
            }
            None => {
                missing.insert(format!("sprite:{}:texture:{normalized}", sprite.name));
            }
        }
    }
    let effective_shader_file =
        match crate::gfx::a729_effect_shader_path(sprite.effect_file.as_deref()) {
            Ok(path) => path,
            Err(error) => {
                missing.insert(format!("sprite:{}:effect:{error}", sprite.name));
                return Ok(None);
            }
        };
    let Some(effect_file) = resolve_optional(&effective_shader_file)? else {
        missing.insert(format!(
            "sprite:{}:effect:{effective_shader_file}",
            sprite.name
        ));
        return Ok(None);
    };
    let effect_content = admitted_key(admission, &effect_file.rel_path)?.clone();
    dependencies.insert(
        effect_content.normalized_game_relative_path.clone(),
        effect_content.clone(),
    );
    let Some(includes) = crate::gfx::xui07_builtin_shader_includes(&effective_shader_file) else {
        missing.insert(format!(
            "sprite:{}:unsupported-effect-include-closure:{effective_shader_file}",
            sprite.name
        ));
        return Ok(None);
    };
    let mut effect_include_content = Vec::new();
    for include in includes {
        let Some(file) = resolve_optional(include)? else {
            missing.insert(format!("sprite:{}:effect-include:{include}", sprite.name));
            continue;
        };
        let key = admitted_key(admission, &file.rel_path)?.clone();
        dependencies.insert(key.normalized_game_relative_path.clone(), key.clone());
        effect_include_content.push(key);
    }
    if texture_slots.len() != sprite.texture_files.len()
        || effect_include_content.len() != includes.len()
    {
        return Ok(None);
    }
    let authored_color_rgba_f32_bits =
        match crate::gfx::a729_sprite_color_rgba_f32_bits(&sprite.raw) {
            Ok(color) => color,
            Err(error) => {
                missing.insert(format!("sprite:{}:color:{error}", sprite.name));
                return Ok(None);
            }
        };
    Ok(Some(AuthoredSpriteRole {
        sprite_name: sprite.name.clone(),
        sprite_kind: sprite.kind.clone(),
        frame_count: sprite.no_of_frames,
        authored_effect_file: sprite.effect_file.clone(),
        effective_shader_file,
        definition_content,
        texture_slots,
        effect_content,
        effect_include_content,
        authored_color_rgba_f32_bits,
        authored_color_was_explicit: sprite.raw.values_ci("color").next().is_some(),
    }))
}

fn validate_sprite_role(
    sprite: &AuthoredSpriteRole,
    dependency_content: &[UsedContentKey],
) -> Result<(), String> {
    if sprite.sprite_name.trim().is_empty()
        || sprite.sprite_kind.trim().is_empty()
        || sprite.frame_count == 0
    {
        return Err("authored sprite identity/kind/frame count is invalid".into());
    }
    let expected_shader =
        crate::gfx::a729_effect_shader_path(sprite.authored_effect_file.as_deref())?;
    if !sprite
        .effective_shader_file
        .eq_ignore_ascii_case(&expected_shader)
        || !sprite
            .effect_content
            .normalized_game_relative_path
            .eq_ignore_ascii_case(&expected_shader)
    {
        return Err("authored sprite effect path is inconsistent".into());
    }
    let expected_includes = crate::gfx::xui07_builtin_shader_includes(&expected_shader)
        .ok_or_else(|| "authored sprite uses unsupported effect closure".to_string())?;
    if sprite.effect_include_content.len() != expected_includes.len()
        || sprite
            .effect_include_content
            .iter()
            .zip(expected_includes)
            .any(|(content, expected)| {
                !content
                    .normalized_game_relative_path
                    .eq_ignore_ascii_case(expected)
            })
    {
        return Err("authored sprite effect include order is inconsistent".into());
    }
    if sprite
        .authored_color_rgba_f32_bits
        .map(f32::from_bits)
        .iter()
        .any(|channel| !channel.is_finite() || !(0.0..=1.0).contains(channel))
    {
        return Err("authored sprite color is outside the finite unit range".into());
    }
    for content in std::iter::once(&sprite.definition_content)
        .chain(sprite.texture_slots.iter().map(|slot| &slot.content))
        .chain(std::iter::once(&sprite.effect_content))
        .chain(sprite.effect_include_content.iter())
    {
        if !dependency_content
            .iter()
            .any(|dependency| dependency == content)
        {
            return Err("authored sprite binding is absent from dependency closure".into());
        }
    }
    Ok(())
}

fn admitted_key<'a>(
    admission: &'a ValidatedContentAdmission,
    path: &str,
) -> Result<&'a UsedContentKey, String> {
    admission.require_used_content(
        &normalize_game_relative_path(path)
            .map_err(|error| format!("authored GUI content path: {error}"))?,
    )
}

/// Outcome of looking up a texture's same-stem fallback candidates.
enum SameStemTexture {
    Resolved(crate::assets::ResolvedFile),
    Absent,
    Ambiguous,
}

/// The engine resolves textures by stem across its shipped formats, but its
/// precedence between same-stem files in different layers is unproven, so an
/// ambiguous stem refuses instead of guessing.
fn unambiguous_same_stem_texture<F>(
    normalized: &str,
    resolve_optional: &mut F,
) -> Result<SameStemTexture, String>
where
    F: FnMut(&str) -> Result<Option<crate::assets::ResolvedFile>, String>,
{
    let mut hits: Vec<crate::assets::ResolvedFile> = Vec::new();
    for candidate in crate::assets::texture_stem_fallback_candidates(normalized) {
        if let Some(file) = resolve_optional(&candidate)? {
            if !hits
                .iter()
                .any(|row: &crate::assets::ResolvedFile| row.path == file.path)
            {
                hits.push(file);
            }
        }
    }
    match hits.len() {
        0 => Ok(SameStemTexture::Absent),
        1 => Ok(SameStemTexture::Resolved(
            hits.swap_remove(0),
        )),
        _ => Ok(SameStemTexture::Ambiguous),
    }
}

fn collect_elements(
    element: &GuiElement,
    child_path: &[u32],
    out: &mut Vec<AuthoredElementRole>,
) -> Result<(), String> {
    if element.name.trim().is_empty() || element.element_type.trim().is_empty() {
        return Err("authored GUI element has blank type or name".into());
    }
    let element_id = if child_path.is_empty() {
        element.name.clone()
    } else {
        format!(
            "{}@{}",
            element.name,
            child_path
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(".")
        )
    };
    let authored_one_based_frame = {
        let mut authored = element.raw.values_ci("frame");
        let first = authored.next();
        if authored.next().is_some() {
            return Err(format!(
                "authored GUI element {:?} repeats frame",
                element.name
            ));
        }
        match first {
            None => 1,
            Some(value) => crate::paradox_text::scalar_u32(value)
                .filter(|frame| *frame != 0)
                .ok_or_else(|| {
                    format!(
                        "authored GUI element {:?} has an invalid one-based frame",
                        element.name
                    )
                })?,
        }
    };
    let mut direct_texture_refs = Vec::new();
    for field in element.raw.fields() {
        let Some(key) = field.key.as_deref() else {
            continue;
        };
        if !key.eq_ignore_ascii_case("textureFile") {
            continue;
        }
        let path = field.value.as_scalar().ok_or_else(|| {
            format!(
                "authored GUI element {:?} textureFile is not scalar",
                element.name
            )
        })?;
        if path.trim().is_empty() {
            continue;
        }
        direct_texture_refs.push(AuthoredDirectTextureRef {
            authored_field: key.to_string(),
            authored_path: path.to_string(),
        });
    }
    out.push(AuthoredElementRole {
        element_id,
        element_type: element.element_type.clone(),
        authored_name: element.name.clone(),
        child_path: child_path.to_vec(),
        authored_one_based_frame,
        sprite_refs: element.sprite_refs.clone(),
        sprites: Vec::new(),
        direct_texture_refs,
        direct_textures: Vec::new(),
        font_ref: element.font.clone(),
        font: None,
        localisation_or_literal_ref: element.text.clone(),
        localisation_content: None,
    });
    for (ordinal, child) in element.children.iter().enumerate() {
        let ordinal = u32::try_from(ordinal)
            .map_err(|_| "authored GUI child ordinal exceeds u32".to_string())?;
        let mut path = child_path.to_vec();
        path.push(ordinal);
        collect_elements(child, &path, out)?;
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

fn collect_font_refs(element: &GuiElement, out: &mut BTreeSet<String>) {
    if let Some(font) = element.font.as_ref().filter(|font| !font.trim().is_empty()) {
        out.insert(font.clone());
    }
    for child in &element.children {
        collect_font_refs(child, out);
    }
}

fn localisation_sources(
    resolver: &AssetResolver,
    localisation_language: &str,
) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let language_directory = safe_language_directory(localisation_language)?;
    let expected_language = canonical_localisation_language(localisation_language)?;
    let mut files = resolver
        .list_files_recursive(&format!("localisation/{language_directory}"), Some("yml"))
        .map_err(|error| error.to_string())?;
    files.sort_by(|left, right| engine_recursive_path_cmp(&left.rel_path, &right.rel_path));
    for file in files {
        let bytes = std::fs::read(&file.path).map_err(|error| error.to_string())?;
        let losses = crate::paradox_text::diagnose_text_decode(&bytes);
        let destructive = crate::paradox_text::destructive_event_count(&losses);
        if destructive != 0 {
            return Err(format!(
                "authored GUI localisation {} destroyed {destructive} byte sequences",
                file.rel_path
            ));
        }
        let text = crate::paradox_text::decode_text(&bytes);
        let mut lines = text.lines().filter(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#')
        });
        let Some(header) = lines.next() else {
            continue;
        };
        if !header
            .trim()
            .trim_end_matches(':')
            .eq_ignore_ascii_case(&expected_language)
        {
            continue;
        }
        for line in lines {
            let line = line.trim_start();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, _)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim();
            if !key.is_empty() {
                out.insert(key.to_ascii_lowercase(), file.rel_path.clone());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::paradox_text::Value;
    use portable_content_manifest::ContentLayerKind;

    fn element(name: &str, children: Vec<GuiElement>) -> GuiElement {
        GuiElement {
            element_type: "containerWindowType".into(),
            name: name.into(),
            position: None,
            size: None,
            orientation: None,
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children,
            raw: Value::Object(Vec::new()),
        }
    }

    fn content(path: &str, payload: char) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "f".repeat(64),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 2,
            layer_kind: ContentLayerKind::Mod,
            layer_id: "mod-final".into(),
            source_sha256: payload.to_string().repeat(64),
            payload_sha256: payload.to_string().repeat(64),
            payload_bytes: 17,
        }
    }

    fn index(payload: char) -> AuthenticatedGuiRoleIndex {
        let source = content("interface/mod_panel.gui", payload);
        AuthenticatedGuiRoleIndex {
            schema: AUTHORED_ROLE_INDEX_SCHEMA.into(),
            manifest_digest_sha256: "a".repeat(64),
            product_modset_sha256: "b".repeat(64),
            localisation_language: "l_english".into(),
            index_identity_sha256: String::new(),
            surfaces: vec![AuthoredSurfaceRole {
                surface_id: "mod_panel".into(),
                root_window_id: "interface/mod_panel.gui#00001".into(),
                source_file: "interface/mod_panel.gui".into(),
                source_content: source.clone(),
                authored_tree_identity_sha256: "e".repeat(64),
                complete_dependency_closure: true,
                missing_dependencies: Vec::new(),
                dependency_content: vec![source],
                elements: Vec::new(),
                tooltip_shell: None,
            }],
            dynamic_sprites: Vec::new(),
            dynamic_sprite_missing_dependencies: Vec::new(),
            dynamic_dependency_content: Vec::new(),
        }
    }

    #[test]
    fn dynamic_element_ids_are_derived_from_authored_tree_not_a_frozen_inventory() {
        let root = element(
            "mod_panel",
            vec![
                element("same_name", Vec::new()),
                element("same_name", vec![element("leaf", Vec::new())]),
            ],
        );
        let mut roles = Vec::new();
        collect_elements(&root, &[], &mut roles).unwrap();
        assert_eq!(
            roles
                .iter()
                .map(|role| role.element_id.as_str())
                .collect::<Vec<_>>(),
            vec!["mod_panel", "same_name@0", "same_name@1", "leaf@1.0"]
        );
    }

    #[test]
    fn authored_tree_identity_seals_parsed_and_raw_child_fields() {
        let mut root = element("mod_panel", vec![element("child", Vec::new())]);
        let original = authored_tree_identity_sha256(&root);
        root.children[0].position = Some(MeasurePair {
            x: Measure {
                raw: "17".into(),
                value: Some("17".into()),
                is_percent: false,
            },
            y: Measure {
                raw: "23".into(),
                value: Some("23".into()),
                is_percent: false,
            },
        });
        assert_ne!(original, authored_tree_identity_sha256(&root));

        let parsed_only = authored_tree_identity_sha256(&root);
        root.raw = Value::Object(vec![crate::paradox_text::Entry {
            key: Some("show_position".into()),
            value: Value::Object(Vec::new()),
            line: 8,
            column: 4,
        }]);
        assert_ne!(parsed_only, authored_tree_identity_sha256(&root));
    }

    #[test]
    fn tooltip_shell_and_direct_texture_are_structural_not_name_based() {
        let document = crate::paradox_text::parse_str(
            "guiTypes={ textBoxType={ name=ModRenamedTip \
             textureFile=\"gfx/interface/mod_tip.tga\" borderSize={ x=7 y=9 } \
             font=Alias maxWidth=321 maxHeight=123 } }",
        );
        let mut roots = crate::gui::parse_gui_document(&document);
        let root = roots.pop().expect("tooltip root");
        assert!(is_tooltip_shell_root(&root).unwrap());
        let mut roles = Vec::new();
        collect_elements(&root, &[], &mut roles).unwrap();
        assert_eq!(roles[0].direct_texture_refs.len(), 1);
        assert_eq!(
            roles[0].direct_texture_refs[0].authored_path,
            "gfx/interface/mod_tip.tga"
        );
    }

    #[test]
    fn blank_optional_texture_is_retained_in_raw_tree_but_is_not_an_asset_ref() {
        let document = crate::paradox_text::parse_str(
            "guiTypes={ instantTextBoxType={ name=DateText textureFile=\"\" font=Alias } }",
        );
        let mut roots = crate::gui::parse_gui_document(&document);
        let root = roots.pop().expect("text root");
        assert_eq!(root.raw.first_scalar_ci("textureFile"), Some(""));
        let mut roles = Vec::new();
        collect_elements(&root, &[], &mut roles).unwrap();
        assert!(roles[0].direct_texture_refs.is_empty());
    }

    #[test]
    fn portable_identity_contains_no_physical_root_and_changes_with_vfs_winner() {
        let base = index('c');
        let relocated = base.clone();
        assert_eq!(
            base.computed_identity_sha256().unwrap(),
            relocated.computed_identity_sha256().unwrap()
        );
        let overridden = index('d');
        assert_ne!(
            base.computed_identity_sha256().unwrap(),
            overridden.computed_identity_sha256().unwrap()
        );
        let encoded = serde_json::to_string(&base).unwrap();
        assert!(!encoded.contains("C:\\"));
        assert!(!encoded.contains("/home/"));
    }

    #[test]
    fn incomplete_closure_state_is_explicit_and_hash_bound() {
        let mut value = index('e');
        value.surfaces[0].complete_dependency_closure = false;
        value.surfaces[0].missing_dependencies = vec!["texture:gfx/missing.dds".into()];
        let first = value.computed_identity_sha256().unwrap();
        value.surfaces[0].missing_dependencies[0] = "effect:fx/missing.shader".into();
        assert_ne!(first, value.computed_identity_sha256().unwrap());
    }
}
