//! Product adapter from the mod-aware game VFS to XUI01 definition admission.

use crate::Result;
use game_ui::assets::AssetResolver;
use game_ui::authored_role_index::{
    AuthenticatedGuiRoleIndex, AuthenticatedPanelSurface, AuthoredSpriteRole,
    StrictAuthoredPanelAdmission,
};
use game_ui::definition_admission::{
    load_effective_definition_catalog, GuiDefinitionCatalog, ResolvedGuiDefinitionFile,
};
use game_ui::frontend_meta::{
    admit_discovered_frontend_chrome, admit_discovered_frontend_chrome_with_bindings,
    admit_frontend_panel_presentation, admitted_frontend_family_ids,
    classified_frontend_family_for_surface, frontend_family_scope, DiscoveredFrontendComposition,
    FrontendBindingSnapshot, FrontendFamilyScope,
};
use game_ui::loc::Localisation;
use game_ui::window_manager_law::{
    DormantWindowDefinition, DormantWindowManagerRegistry, ReplayAuthenticatedElementSemantic,
    ReplayAuthenticatedFrontendCollectionsBinding, ReplayAuthenticatedPanelFrameInput,
    ReplayAuthenticatedProductionLinesBinding, ReplayAuthenticatedScrollBinding,
    ReplayElementPointerAuthority, ReplayOwnedWindowManager, ReplayPanelActivation,
    ReplayScrollAxis, ReplayTooltipAlternateSelection, ReplayViewerActionBinding,
    ReplayWindowManagerSnapshot,
};
use map_viewer::GameFileResolver;
use portable_content_manifest::authored_ui::{
    classify_authored_ui_path, decode_engine_path_spelling_consumer, safe_language_directory,
    AuthoredUiNamespace, DEPENDENCY_CLOSURE_CONSUMER, DEPENDENCY_CLOSURE_EMPTY_CONSUMER,
    ENGINE_PATH_SPELLING_CONSUMER_PREFIX, GFX_NAMESPACE_CONSUMER, GUI_NAMESPACE_CONSUMER,
    LOCALISATION_NAMESPACE_CONSUMER, SCRIPTED_LOCALISATION_NAMESPACE_CONSUMER,
};
use replay_core::ValidatedContentAdmission;
use std::collections::{BTreeMap, BTreeSet};

/// Semantic-consumer capabilities whose rows must cover the complete effective
/// GUI, GFX, active-language localisation, scripted-localisation, and resolved
/// authored dependency namespaces. Existing selected-Present-only recordings
/// do not carry these capabilities and therefore fail closed for observer
/// panels.
pub const AUTHORED_GUI_NAMESPACE_CONSUMER: &str = GUI_NAMESPACE_CONSUMER;
pub const AUTHORED_GFX_NAMESPACE_CONSUMER: &str = GFX_NAMESPACE_CONSUMER;
pub const AUTHORED_LOCALISATION_NAMESPACE_CONSUMER: &str = LOCALISATION_NAMESPACE_CONSUMER;
pub const AUTHORED_SCRIPTED_LOCALISATION_NAMESPACE_CONSUMER: &str =
    SCRIPTED_LOCALISATION_NAMESPACE_CONSUMER;
pub const AUTHORED_DEPENDENCY_CLOSURE_CONSUMER: &str = DEPENDENCY_CLOSURE_CONSUMER;
pub const AUTHORED_DEPENDENCY_CLOSURE_EMPTY_CONSUMER: &str = DEPENDENCY_CLOSURE_EMPTY_CONSUMER;

pub type OpenedAuthoredPanel = AuthenticatedPanelSurface;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredScrollBinding {
    pub element_id: String,
    pub axis: ReplayScrollAxis,
    pub min_offset_pixels: i32,
    pub max_offset_pixels: i32,
    pub offset_pixels: i32,
    pub geometry_source_record: String,
}

/// Complete typed product input for one authenticated authored surface.
///
/// Geometry, clipping, paint order, runtime tokens, and pixels stay private to
/// `game_ui`. Callers provide only the current viewport, a complete
/// manifest-derived element semantic census, and viewer-owned input state.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredPanelViewerBindings {
    pub viewport_width: usize,
    pub viewport_height: usize,
    pub semantics: BTreeMap<String, ReplayAuthenticatedElementSemantic>,
    pub activation: Option<ReplayPanelActivation>,
    pub element_pointer_bindings: BTreeMap<String, ReplayElementPointerAuthority>,
    pub viewer_actions: Vec<ReplayViewerActionBinding>,
    pub persistent_tooltip_targets: Vec<String>,
    pub tooltip_selectors: BTreeMap<String, ReplayTooltipAlternateSelection>,
    pub scroll_regions: Vec<AuthoredScrollBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposedAuthoredPanel {
    pub window_id: String,
    pub runtime_instance_token: String,
    pub frame_generation: u64,
    pub drawn_elements: usize,
    pub frontend: Option<ComposedFrontendAdmission>,
}

/// XUI26 content facts attached only after the exact panel has successfully
/// composed under a current manager instance and staged frame generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposedFrontendAdmission {
    pub family_id: String,
    pub source_file: String,
    pub admitted_bindings: Vec<(String, String)>,
    pub unavailable_bindings: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontendPanelRouteState {
    pub window_id: String,
    pub family_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontendFamilyDisposition {
    pub window_id: String,
    pub family_id: String,
    pub source_file: String,
    pub unavailable_bindings: Vec<(String, String)>,
    pub scope: FrontendFamilyScope,
}

#[derive(Clone, Debug, PartialEq)]
struct PersistentHudPanelBindings {
    authenticated: game_ui::window_manager_law::ReplayAuthenticatedPersistentHudBinding,
    theatre: Option<game_ui::window_manager_law::ReplayAuthenticatedTheatreBinding>,
    scroll_regions: Vec<AuthoredScrollBinding>,
    tooltip_targets: Vec<String>,
    tooltip_selectors: BTreeMap<String, ReplayTooltipAlternateSelection>,
}

/// Replay-owned observer window namespace initialized from the replay's exact
/// effective content admission.
///
/// This is deliberately independent of recorder POV and selected-Present panel
/// activation. Recorder pixels are oracle history and never participate in
/// viewer-owned panel activation.
pub struct AuthoredObserverUi {
    assets: AssetResolver,
    localisation_language: String,
    localisation: Localisation,
    index: AuthenticatedGuiRoleIndex,
    catalog: GuiDefinitionCatalog,
    manager: ReplayOwnedWindowManager,
    opened: BTreeMap<String, (AuthenticatedPanelSurface, AuthoredPanelViewerBindings)>,
    opened_order: Vec<String>,
    production_lines_bindings: BTreeMap<String, ReplayAuthenticatedProductionLinesBinding>,
    frontend_collection_bindings: BTreeMap<String, ReplayAuthenticatedFrontendCollectionsBinding>,
    frontend_frame_hours: BTreeMap<String, i64>,
    persistent_panels: BTreeMap<String, (AuthenticatedPanelSurface, PersistentHudPanelBindings)>,
    persistent_panel_order: Vec<String>,
    presented_desired_checkpoint: Box<AuthoredObserverUiDesiredCheckpoint>,
    pending_desired_checkpoint: Option<Box<AuthoredObserverUiDesiredCheckpoint>>,
}

#[derive(Clone, Debug, PartialEq)]
struct AuthoredObserverUiDesiredCheckpoint {
    opened: BTreeMap<String, (AuthenticatedPanelSurface, AuthoredPanelViewerBindings)>,
    opened_order: Vec<String>,
    production_lines_bindings: BTreeMap<String, ReplayAuthenticatedProductionLinesBinding>,
    frontend_collection_bindings: BTreeMap<String, ReplayAuthenticatedFrontendCollectionsBinding>,
    frontend_frame_hours: BTreeMap<String, i64>,
    persistent_panels: BTreeMap<String, (AuthenticatedPanelSurface, PersistentHudPanelBindings)>,
    persistent_panel_order: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthoredObserverUiInputCheckpoint {
    desired: AuthoredObserverUiDesiredCheckpoint,
    manager: game_ui::window_manager_law::ReplayWindowManagerInputCheckpoint,
}

impl AuthoredObserverUiInputCheckpoint {
    pub(crate) fn apply_capture_loss(&mut self) {
        self.manager.apply_capture_loss();
    }
}

impl AuthoredObserverUi {
    pub(crate) fn input_checkpoint(&self) -> Result<AuthoredObserverUiInputCheckpoint> {
        Ok(AuthoredObserverUiInputCheckpoint {
            desired: self.desired_checkpoint(),
            manager: self.manager.input_checkpoint()?,
        })
    }

    pub(crate) fn restore_input_checkpoint(
        &mut self,
        checkpoint: AuthoredObserverUiInputCheckpoint,
    ) -> Result<()> {
        self.manager.restore_input_checkpoint(checkpoint.manager)?;
        self.restore_desired_checkpoint(checkpoint.desired);
        Ok(())
    }

    pub fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        localisation_language: &str,
    ) -> Result<Self> {
        require_complete_authored_namespaces(resolver, admission, localisation_language)?;
        // Materialize only the authenticated authored-UI namespaces and their
        // dependency closure. In particular, other localisation languages
        // required by unrelated replay consumers never enter the UI resolver
        // and therefore cannot become accidental localisation winners.
        let mut authored_rows = BTreeMap::new();
        let mut namespace_paths = BTreeSet::new();
        for consumer in [
            AUTHORED_GUI_NAMESPACE_CONSUMER,
            AUTHORED_GFX_NAMESPACE_CONSUMER,
            AUTHORED_LOCALISATION_NAMESPACE_CONSUMER,
            AUTHORED_SCRIPTED_LOCALISATION_NAMESPACE_CONSUMER,
            AUTHORED_DEPENDENCY_CLOSURE_CONSUMER,
            AUTHORED_DEPENDENCY_CLOSURE_EMPTY_CONSUMER,
        ] {
            if let Ok(rows) = admission.require_used_content_for_consumer(consumer) {
                for row in rows {
                    let path = row.normalized_game_relative_path.to_ascii_lowercase();
                    if consumer != AUTHORED_DEPENDENCY_CLOSURE_CONSUMER
                        && consumer != AUTHORED_DEPENDENCY_CLOSURE_EMPTY_CONSUMER
                    {
                        namespace_paths.insert(path.clone());
                    }
                    if authored_rows
                        .insert(path.clone(), row.clone())
                        .is_some_and(|prior| prior != *row)
                    {
                        return Err(format!(
                            "authored UI capabilities disagree on content identity for {path:?}"
                        )
                        .into());
                    }
                }
            }
        }
        let mut engine_spellings = BTreeMap::new();
        for consumer in admission
            .required_consumers()
            .filter(|consumer| consumer.starts_with(ENGINE_PATH_SPELLING_CONSUMER_PREFIX))
        {
            let rows = admission.require_used_content_for_consumer(consumer)?;
            if rows.len() != 1 {
                return Err(format!(
                    "authored engine spelling consumer {consumer:?} does not bind exactly one file"
                )
                .into());
            }
            let row = &rows[0];
            let path = row.normalized_game_relative_path.to_ascii_lowercase();
            if !namespace_paths.contains(&path) {
                return Err(format!(
                    "authored engine spelling consumer names non-namespace content {path:?}"
                )
                .into());
            }
            let spelling = decode_engine_path_spelling_consumer(consumer, &path)?
                .ok_or_else(|| "authored engine spelling prefix failed to decode".to_string())?;
            if engine_spellings.insert(path.clone(), spelling).is_some() {
                return Err(format!("authored engine spelling repeats {path:?}").into());
            }
        }
        if engine_spellings.len() != namespace_paths.len()
            || namespace_paths
                .iter()
                .any(|path| !engine_spellings.contains_key(path))
        {
            return Err(
                "authored UI namespace is missing authenticated engine path spellings".into(),
            );
        }
        let assets = AssetResolver::from_used_content_rows_with_engine_spellings(
            resolver,
            admission,
            authored_rows.into_values(),
            engine_spellings,
        )?;
        let localisation_language =
            portable_content_manifest::authored_ui::canonical_localisation_language(
                localisation_language,
            )?;
        let localisation = if admission
            .require_used_content_for_consumer(AUTHORED_LOCALISATION_NAMESPACE_CONSUMER)
            .is_ok()
        {
            Localisation::load_admitted_language(&assets, &localisation_language)?
        } else {
            // `require_complete_authored_namespaces` already proved the
            // authenticated empty capability. Retain an exact empty catalogue;
            // never reopen an ambient profile or fall back to English.
            Localisation::default()
        };
        let (index, catalog) = AuthenticatedGuiRoleIndex::emit_with_game_resolver_and_language(
            &assets,
            admission,
            Some(resolver),
            &localisation_language,
        )?;
        validate_authored_dependency_capability(&index, admission)?;
        admitted_frontend_family_ids(&index, admission)
            .map_err(|error| format!("XUI26 effective frontend inventory rejected: {error:?}"))?;
        let registry = register_admitted_authored_roots(&index, &catalog)?;
        let manager = ReplayOwnedWindowManager::new(&registry)?;
        Ok(Self {
            assets,
            localisation_language,
            localisation,
            index,
            catalog,
            manager,
            opened: BTreeMap::new(),
            opened_order: Vec::new(),
            production_lines_bindings: BTreeMap::new(),
            frontend_collection_bindings: BTreeMap::new(),
            frontend_frame_hours: BTreeMap::new(),
            persistent_panels: BTreeMap::new(),
            persistent_panel_order: Vec::new(),
            presented_desired_checkpoint: Box::new(AuthoredObserverUiDesiredCheckpoint {
                opened: BTreeMap::new(),
                opened_order: Vec::new(),
                production_lines_bindings: BTreeMap::new(),
                frontend_collection_bindings: BTreeMap::new(),
                frontend_frame_hours: BTreeMap::new(),
                persistent_panels: BTreeMap::new(),
                persistent_panel_order: Vec::new(),
            }),
            pending_desired_checkpoint: None,
        })
    }

    fn desired_checkpoint(&self) -> AuthoredObserverUiDesiredCheckpoint {
        AuthoredObserverUiDesiredCheckpoint {
            opened: self.opened.clone(),
            opened_order: self.opened_order.clone(),
            production_lines_bindings: self.production_lines_bindings.clone(),
            frontend_collection_bindings: self.frontend_collection_bindings.clone(),
            frontend_frame_hours: self.frontend_frame_hours.clone(),
            persistent_panels: self.persistent_panels.clone(),
            persistent_panel_order: self.persistent_panel_order.clone(),
        }
    }

    fn restore_desired_checkpoint(&mut self, checkpoint: AuthoredObserverUiDesiredCheckpoint) {
        self.opened = checkpoint.opened;
        self.opened_order = checkpoint.opened_order;
        self.production_lines_bindings = checkpoint.production_lines_bindings;
        self.frontend_collection_bindings = checkpoint.frontend_collection_bindings;
        self.frontend_frame_hours = checkpoint.frontend_frame_hours;
        self.persistent_panels = checkpoint.persistent_panels;
        self.persistent_panel_order = checkpoint.persistent_panel_order;
    }

    fn require_no_pending_desired_presentation(&self) -> Result<()> {
        if self.pending_desired_checkpoint.is_some() {
            return Err("XUI02 authored desired state awaits Present disposition".into());
        }
        Ok(())
    }

    fn desired_state_matches_presented(&self) -> bool {
        self.opened == self.presented_desired_checkpoint.opened
            && self.opened_order == self.presented_desired_checkpoint.opened_order
            && self.production_lines_bindings
                == self.presented_desired_checkpoint.production_lines_bindings
            && self.frontend_collection_bindings
                == self
                    .presented_desired_checkpoint
                    .frontend_collection_bindings
            && self.frontend_frame_hours == self.presented_desired_checkpoint.frontend_frame_hours
            && self.persistent_panels == self.presented_desired_checkpoint.persistent_panels
            && self.persistent_panel_order
                == self.presented_desired_checkpoint.persistent_panel_order
    }

    pub fn index(&self) -> &AuthenticatedGuiRoleIndex {
        &self.index
    }

    /// Exact effective-VFS catalogue used to join XUI22's authenticated d84
    /// manager census to the authored scripted-GUI definitions.
    pub fn catalog(&self) -> &GuiDefinitionCatalog {
        &self.catalog
    }

    /// Exact manifest-restricted resolver used to issue every strict token.
    /// Compositors must use this authority rather than rebuilding an
    /// unrestricted filesystem resolver.
    pub fn assets(&self) -> &AssetResolver {
        &self.assets
    }

    pub fn localisation_language(&self) -> &str {
        &self.localisation_language
    }

    /// Cached exact selected-language catalogue loaded only from the
    /// manifest-restricted authored namespace.
    pub fn localisation(&self) -> &Localisation {
        &self.localisation
    }

    pub fn admit_dynamic_sprite<'a>(
        &'a self,
        admission: &ValidatedContentAdmission,
        sprite_name: &str,
    ) -> Result<&'a AuthoredSpriteRole> {
        Ok(self.index.admit_dynamic_sprite(admission, sprite_name)?)
    }

    pub fn discovered_window_ids(&self) -> impl Iterator<Item = &str> {
        self.index
            .surfaces
            .iter()
            .map(|surface| surface.root_window_id.as_str())
    }

    pub fn manager(&self) -> &ReplayOwnedWindowManager {
        &self.manager
    }

    pub fn admitted_frontend_family_ids(
        &self,
        admission: &ValidatedContentAdmission,
    ) -> Result<Vec<String>> {
        Ok(admitted_frontend_family_ids(&self.index, admission)
            .map_err(|error| format!("XUI26 effective frontend inventory rejected: {error:?}"))?)
    }

    /// Complete content-authenticated XUI26 disposition census. This is
    /// emitted at shipping initialization so dynamic families remain explicit
    /// unavailable states even though the viewer cannot open them without a
    /// complete typed producer.
    pub fn frontend_family_dispositions(
        &self,
        admission: &ValidatedContentAdmission,
    ) -> Result<Vec<FrontendFamilyDisposition>> {
        let mut out = Vec::new();
        for role in &self.index.surfaces {
            let classified =
                classified_frontend_family_for_surface(&role.source_content, &role.surface_id)
                    .map_err(|error| format!("XUI26 disposition classifier rejected: {error:?}"))?;
            if classified.is_none() {
                continue;
            }
            let panel = self
                .index
                .admit_panel(&self.catalog, admission, &role.root_window_id)?;
            let frontend = admit_discovered_frontend_chrome(&self.index, admission, &panel)
                .map_err(|error| format!("XUI26 disposition rejected: {error:?}"))?;
            let Some(family) = frontend.family() else {
                continue;
            };
            out.push(FrontendFamilyDisposition {
                window_id: panel.window_id().to_string(),
                family_id: family.family_id().to_string(),
                source_file: family.source_file().to_string(),
                unavailable_bindings: family
                    .unavailable_bindings()
                    .iter()
                    .map(|binding| (binding.slot_id.clone(), binding.reason_code.clone()))
                    .collect(),
                scope: frontend_family_scope(family.family_id())
                    .map_err(|error| format!("XUI26 family scope rejected: {error:?}"))?,
            });
        }
        Ok(out)
    }

    /// Complete immutable root-token census for persistent-HUD/atomic
    /// consumers. Empty authored namespaces return an authenticated empty
    /// vector after index/admission validation.
    pub(crate) fn strict_root_census(
        &self,
        admission: &ValidatedContentAdmission,
    ) -> Result<Vec<StrictAuthoredPanelAdmission>> {
        Ok(self.index.admit_all_surfaces(&self.catalog, admission)?)
    }

    pub fn admit_surface(
        &self,
        admission: &ValidatedContentAdmission,
        source_content: &replay_core::UsedContentKey,
        surface_id: &str,
    ) -> Result<AuthenticatedPanelSurface> {
        let panel =
            self.index
                .admit_surface(&self.catalog, admission, source_content, surface_id)?;
        let frontend = admit_discovered_frontend_chrome(&self.index, admission, &panel)
            .map_err(|error| format!("viewer-owned chrome rejected: {error:?}"))?;
        self.seal_surface(admission, panel, frontend)
    }

    fn admit_surface_with_frontend_bindings(
        &self,
        admission: &ValidatedContentAdmission,
        source_content: &replay_core::UsedContentKey,
        surface_id: &str,
        snapshots: &[FrontendBindingSnapshot],
    ) -> Result<AuthenticatedPanelSurface> {
        let panel =
            self.index
                .admit_surface(&self.catalog, admission, source_content, surface_id)?;
        let frontend = admit_discovered_frontend_chrome_with_bindings(
            &self.index,
            admission,
            &panel,
            snapshots,
        )
        .map_err(|error| format!("viewer-owned XUI26 binding rejected: {error:?}"))?;
        self.seal_surface(admission, panel, frontend)
    }

    /// Gate shipping compositor source observations against the exact
    /// effective winning authored-root registry.
    pub fn validate_composed_roots(&self, roots: &[game_ui::hud::HudSourceWindow]) -> Result<()> {
        if roots.is_empty() {
            return Err("XUI01 shipping compositor reported no authored roots".into());
        }
        let mut seen = BTreeSet::new();
        for root in roots {
            let folded = root.window_name.to_ascii_lowercase();
            if !seen.insert(folded) {
                return Err(format!(
                    "XUI01 shipping compositor repeated authored root {:?}",
                    root.window_name
                )
                .into());
            }
            let surface = self
                .index
                .surfaces
                .iter()
                .find(|surface| surface.surface_id.eq_ignore_ascii_case(&root.window_name))
                .ok_or_else(|| {
                    format!(
                        "XUI01 shipping compositor used root {:?} absent from the effective winning catalogue",
                        root.window_name
                    )
                })?;
            if !surface.complete_dependency_closure {
                return Err(format!(
                    "XUI01 shipping compositor root {:?} has incomplete authored dependency closure: {:?}",
                    root.window_name, surface.missing_dependencies
                )
                .into());
            }
            if !surface.source_file.eq_ignore_ascii_case(&root.source_file) {
                return Err(format!(
                    "XUI01 shipping compositor loaded {:?} from {:?}, but the effective last-parsed winner is {:?}",
                    root.window_name, root.source_file, surface.source_file
                )
                .into());
            }
        }
        Ok(())
    }

    fn pointer_geometry_ready(&self) -> Result<bool> {
        match self.manager.composed_snapshot() {
            Ok(_) => Ok(true),
            Err(
                game_ui::window_manager_law::ReplayWindowManagerRejection::MissingComposedGeometry(
                    _,
                ),
            ) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Request a replay-viewer panel by a discovered dynamic window identity.
    /// Manager mutation is delayed until the complete frame transaction.
    pub fn open_panel_with_bindings(
        &mut self,
        admission: &ValidatedContentAdmission,
        window_id: &str,
        mut bindings: AuthoredPanelViewerBindings,
        _viewer_clock_micros: u64,
    ) -> Result<AuthenticatedPanelSurface> {
        self.require_no_pending_desired_presentation()?;
        let panel = self
            .index
            .admit_panel(&self.catalog, admission, window_id)?;
        let frontend = admit_discovered_frontend_chrome(&self.index, admission, &panel)
            .map_err(|error| format!("XUI26 viewer-owned chrome rejected: {error:?}"))?;
        let surface = self.seal_surface(admission, panel, frontend)?;
        self.open_admitted_surface_with_bindings(surface, bindings)
    }

    fn open_admitted_surface_with_bindings(
        &mut self,
        surface: AuthenticatedPanelSurface,
        mut bindings: AuthoredPanelViewerBindings,
    ) -> Result<AuthenticatedPanelSurface> {
        let window_id = surface.window_id().to_string();
        install_derived_tooltip_bindings(&surface, &mut bindings)?;
        validate_viewer_bindings(&surface, &bindings, false)?;
        if self.opened.contains_key(&window_id) {
            return Err(format!("XUI02 authored panel {window_id:?} is already requested").into());
        }
        self.opened
            .insert(window_id.clone(), (surface.clone(), bindings));
        self.opened_order.push(window_id);
        Ok(surface)
    }

    pub fn close_panel(&mut self, window_id: &str) -> Result<()> {
        self.require_no_pending_desired_presentation()?;
        if self.opened.remove(window_id).is_none() {
            return Err(format!("XUI02 authored panel {window_id:?} was not requested").into());
        }
        self.opened_order.retain(|id| id != window_id);
        self.production_lines_bindings.remove(window_id);
        self.frontend_collection_bindings.remove(window_id);
        self.frontend_frame_hours.remove(window_id);
        Ok(())
    }

    /// Install or refresh the complete typed XUI12 canvas in the ordinary
    /// replay-owned panel stack. The exact owner and every row template are
    /// re-admitted from the effective content identity before any pixels or
    /// input geometry can be produced.
    pub fn set_production_lines_canvas(
        &mut self,
        admission: &ValidatedContentAdmission,
        viewport_width: usize,
        viewport_height: usize,
        input: &game_ui::production_lines::StrictXui12CanvasInput,
        viewer_clock_micros: u64,
    ) -> Result<String> {
        let checkpoint = self.desired_checkpoint();
        let result = self.set_production_lines_canvas_inner(
            admission,
            viewport_width,
            viewport_height,
            input,
            viewer_clock_micros,
        );
        if result.is_err() {
            self.restore_desired_checkpoint(checkpoint);
        }
        result
    }

    fn set_production_lines_canvas_inner(
        &mut self,
        admission: &ValidatedContentAdmission,
        viewport_width: usize,
        viewport_height: usize,
        input: &game_ui::production_lines::StrictXui12CanvasInput,
        viewer_clock_micros: u64,
    ) -> Result<String> {
        self.require_no_pending_desired_presentation()?;
        let candidates = self
            .index
            .surfaces
            .iter()
            .filter(|surface| {
                surface
                    .surface_id
                    .eq_ignore_ascii_case(game_ui::production_lines::COUNTRY_PRODUCTION_LINE_ROOT)
            })
            .map(|surface| surface.root_window_id.clone())
            .collect::<Vec<_>>();
        let [window_id] = candidates.as_slice() else {
            return Err(format!(
                "XUI12 authenticated countryproductionlineview root count is {}",
                candidates.len()
            )
            .into());
        };
        let panel = self
            .index
            .admit_panel(&self.catalog, admission, window_id)?;
        let frontend = admit_discovered_frontend_chrome(&self.index, admission, &panel)
            .map_err(|error| format!("XUI12 viewer-owned chrome rejected: {error:?}"))?;
        let surface = self.seal_surface(admission, panel, frontend)?;
        let capability = game_ui::window_manager_law::derive_exact_a729_production_lines_binding(
            &surface,
            &self.index,
            &self.catalog,
            admission,
            viewport_width,
            viewport_height,
            input,
        )?;
        if capability.owner_window_id() != window_id
            || capability.viewport_size() != [viewport_width, viewport_height]
            || capability.country_tag() != input.country_tag
        {
            return Err(
                "XUI12 typed capability changed owner, viewport, or country identity".into(),
            );
        }
        let bindings = AuthoredPanelViewerBindings {
            viewport_width,
            viewport_height,
            semantics: capability.owner_semantics().clone(),
            activation: Some(ReplayPanelActivation {
                window_id: window_id.clone(),
                modal_owner: None,
                initial_focus: None,
                keyboard_traversal: Vec::new(),
                controller_traversal: Vec::new(),
                tooltip_targets: Vec::new(),
            }),
            element_pointer_bindings: capability.owner_pointer_bindings().clone(),
            viewer_actions: capability.owner_viewer_actions().to_vec(),
            persistent_tooltip_targets: Vec::new(),
            tooltip_selectors: BTreeMap::new(),
            scroll_regions: Vec::new(),
        };
        if self.opened.contains_key(window_id) {
            self.refresh_open_panel_bindings(admission, window_id, bindings)?;
        } else {
            let opened =
                self.open_panel_with_bindings(admission, window_id, bindings, viewer_clock_micros)?;
            if opened != surface {
                self.opened.remove(window_id);
                self.opened_order.retain(|id| id != window_id);
                return Err("XUI12 owner surface changed between admission and open".into());
            }
        }
        self.production_lines_bindings
            .insert(window_id.clone(), capability);
        Ok(window_id.clone())
    }

    /// Feature-local XUI12 rejection must never retain an older drawable
    /// production canvas. This deliberately touches only the effective
    /// country-production root(s); unrelated authored/persistent UI remains.
    pub(crate) fn reject_production_lines_canvas_feature_locally(&mut self) {
        let window_ids = self
            .index
            .surfaces
            .iter()
            .filter(|surface| {
                surface
                    .surface_id
                    .eq_ignore_ascii_case(game_ui::production_lines::COUNTRY_PRODUCTION_LINE_ROOT)
            })
            .map(|surface| surface.root_window_id.clone())
            .collect::<BTreeSet<_>>();
        self.production_lines_bindings
            .retain(|window_id, _| !window_ids.contains(window_id));
        self.opened
            .retain(|window_id, _| !window_ids.contains(window_id));
        self.opened_order
            .retain(|window_id| !window_ids.contains(window_id));
    }

    pub fn close_production_lines_canvas(&mut self) -> Result<bool> {
        self.require_no_pending_desired_presentation()?;
        let ids = self
            .production_lines_bindings
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(false);
        }
        if ids.len() != 1 {
            return Err("XUI12 production-line owner census is ambiguous".into());
        }
        self.close_panel(&ids[0])?;
        Ok(true)
    }

    /// Viewer-owned shipping route for XUI26 chrome. Repeated requests advance
    /// through the exact effective-VFS order, then close the final panel.
    /// Static families are always eligible; replay families require a sealed
    /// current-hour carrier. Host-runtime and service families are explicit
    /// nonshipping inventory because this product owns no truthful producer.
    pub fn cycle_frontend_panel(
        &mut self,
        admission: &ValidatedContentAdmission,
        world: &replay_core::WorldFrame,
        viewport_width: usize,
        viewport_height: usize,
        viewer_clock_micros: u64,
    ) -> Result<Option<FrontendPanelRouteState>> {
        let checkpoint = self.desired_checkpoint();
        let result = self.cycle_frontend_panel_inner(
            admission,
            world,
            viewport_width,
            viewport_height,
            viewer_clock_micros,
        );
        if result.is_err() {
            self.restore_desired_checkpoint(checkpoint);
        }
        result
    }

    fn cycle_frontend_panel_inner(
        &mut self,
        admission: &ValidatedContentAdmission,
        world: &replay_core::WorldFrame,
        viewport_width: usize,
        viewport_height: usize,
        _viewer_clock_micros: u64,
    ) -> Result<Option<FrontendPanelRouteState>> {
        self.require_no_pending_desired_presentation()?;
        let identities = self
            .index
            .surfaces
            .iter()
            .filter_map(|surface| {
                match classified_frontend_family_for_surface(
                    &surface.source_content,
                    &surface.surface_id,
                ) {
                    Ok(Some(family_id)) => Some(Ok((
                        surface.source_content.clone(),
                        surface.surface_id.clone(),
                        surface.root_window_id.clone(),
                        family_id,
                    ))),
                    Ok(None) => None,
                    Err(error) => Some(Err(format!(
                        "XUI26 frontend classifier rejected: {error:?}"
                    ))),
                }
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let classified_window_ids = identities
            .iter()
            .map(|(_, _, window_id, _)| window_id.clone())
            .collect::<BTreeSet<_>>();
        let mut candidates = Vec::new();
        for (source_content, surface_id, window_id, family_id) in identities {
            let scope = frontend_family_scope(&family_id)
                .map_err(|error| format!("XUI26 family scope rejected: {error:?}"))?;
            if matches!(
                scope,
                FrontendFamilyScope::HostRuntimeNonshipping
                    | FrontendFamilyScope::ExternalServiceNonshipping
            ) {
                continue;
            }
            let (snapshots, frame_hour) = match scope {
                FrontendFamilyScope::StaticShipping => (Vec::new(), None),
                FrontendFamilyScope::ReplayPayloadShipping => {
                    let Some(carrier) = world.xui26_frontend().carriers.get(&family_id) else {
                        continue;
                    };
                    (
                        vec![FrontendBindingSnapshot::from_replay_carrier(
                            carrier,
                            admission,
                            &source_content,
                            &self.localisation_language,
                            world.xui26_frontend(),
                        )
                        .map_err(|error| format!("XUI26 replay carrier rejected: {error:?}"))?],
                        Some(world.hour_index()),
                    )
                }
                FrontendFamilyScope::HostRuntimeNonshipping
                | FrontendFamilyScope::ExternalServiceNonshipping => unreachable!(),
            };
            let surface = self.admit_surface_with_frontend_bindings(
                admission,
                &source_content,
                &surface_id,
                &snapshots,
            )?;
            let presentation = admit_frontend_panel_presentation(&surface).map_err(
                |error| {
                    format!(
                        "XUI26 classified shipping frontend {window_id:?} rejected its exact presentation: {error:?}"
                    )
                },
            )?;
            candidates.push((window_id, family_id, surface, presentation, frame_hour));
        }
        let current = candidates
            .iter()
            .position(|(window_id, _, _, _, _)| self.opened.contains_key(window_id));
        let open_classified = classified_window_ids
            .iter()
            .filter(|window_id| self.opened.contains_key(*window_id))
            .cloned()
            .collect::<Vec<_>>();
        for window_id in open_classified {
            self.close_panel(&window_id)?;
        }
        if candidates.is_empty() || current.is_some_and(|ordinal| ordinal + 1 == candidates.len()) {
            return Ok(None);
        }
        let next = current.map_or(0, |ordinal| ordinal + 1);
        let (window_id, family_id, surface, presentation, frame_hour) = candidates
            .into_iter()
            .nth(next)
            .expect("next XUI26 candidate was bounded above");
        let collections = game_ui::window_manager_law::derive_xui26_frontend_collections_binding(
            &surface,
            &self.index,
            &self.catalog,
            admission,
            viewport_width,
            viewport_height,
        )?;
        let bindings = AuthoredPanelViewerBindings {
            viewport_width,
            viewport_height,
            semantics: presentation.semantics,
            activation: Some(ReplayPanelActivation {
                window_id: window_id.clone(),
                modal_owner: None,
                initial_focus: None,
                keyboard_traversal: Vec::new(),
                controller_traversal: Vec::new(),
                tooltip_targets: Vec::new(),
            }),
            element_pointer_bindings: presentation.pointer_bindings,
            viewer_actions: Vec::new(),
            persistent_tooltip_targets: Vec::new(),
            tooltip_selectors: BTreeMap::new(),
            scroll_regions: Vec::new(),
        };
        let opened = self.open_admitted_surface_with_bindings(surface.clone(), bindings)?;
        if opened != surface {
            return Err("XUI26 surface changed between carrier admission and manager open".into());
        }
        if let Some(capability) = collections {
            self.frontend_collection_bindings
                .insert(window_id.clone(), capability);
        }
        if let Some(frame_hour) = frame_hour {
            self.frontend_frame_hours
                .insert(window_id.clone(), frame_hour);
        }
        Ok(Some(FrontendPanelRouteState {
            window_id,
            family_id,
        }))
    }

    pub fn close_frontend_panels(&mut self) -> Result<bool> {
        let ids = self
            .index
            .surfaces
            .iter()
            .filter_map(|surface| {
                classified_frontend_family_for_surface(&surface.source_content, &surface.surface_id)
                    .ok()
                    .flatten()
                    .map(|_| surface.root_window_id.clone())
            })
            .filter(|window_id| self.opened.contains_key(window_id))
            .collect::<Vec<_>>();
        let closed = !ids.is_empty();
        for id in ids {
            self.close_panel(&id)?;
        }
        Ok(closed)
    }

    /// Dynamic XUI26 semantics and materialized membership are exact-hour
    /// observations. An ordinary playback tick therefore closes an older
    /// panel before composition; only an explicit viewer cycle can admit and
    /// open the current-hour replacement.
    pub fn reconcile_frontend_frame(&mut self, world: &replay_core::WorldFrame) -> Result<bool> {
        self.require_no_pending_desired_presentation()?;
        let stale = self
            .frontend_frame_hours
            .iter()
            .filter(|(_, frame_hour)| **frame_hour != world.hour_index())
            .map(|(window_id, _)| window_id.clone())
            .collect::<Vec<_>>();
        for window_id in &stale {
            self.close_panel(window_id)?;
        }
        Ok(!stale.is_empty())
    }

    pub fn refresh_open_panel_bindings(
        &mut self,
        admission: &ValidatedContentAdmission,
        window_id: &str,
        mut bindings: AuthoredPanelViewerBindings,
    ) -> Result<()> {
        self.require_no_pending_desired_presentation()?;
        let (surface, previous) = self
            .opened
            .get(window_id)
            .ok_or_else(|| format!("XUI02 authored panel {window_id:?} is not requested"))?;
        if surface.manifest_digest_sha256() != admission.manifest_digest_sha256()
            || surface.product_modset_sha256() != admission.product_modset_sha256()
        {
            return Err("XUI02 panel surface belongs to a different content admission".into());
        }
        install_derived_tooltip_bindings(surface, &mut bindings)?;
        validate_viewer_bindings(surface, &bindings, false)?;
        if bindings.activation != previous.activation {
            return Err(format!(
                "XUI02 authored panel {window_id:?} attempted to change its open-lifetime activation contract"
            )
            .into());
        }
        retain_viewer_scroll_offsets(previous, &mut bindings);
        self.opened
            .get_mut(window_id)
            .expect("requested panel existence checked above")
            .1 = bindings;
        Ok(())
    }

    /// Re-admit the shipping HUD source observations and install them beneath
    /// all viewer-opened panels. Geometry is derived only inside game_ui from
    /// the sealed surface and current viewport.
    pub fn bind_persistent_hud_roots(
        &mut self,
        admission: &ValidatedContentAdmission,
        viewport_width: usize,
        viewport_height: usize,
        observations: &[game_ui::hud::HudSourceWindow],
        hud: &game_ui::hud::HudBinding,
        theatre: Option<game_ui::production_renderer::ObserverTheatreSurface>,
        theatre_mode: Option<game_ui::production_renderer::ObserverTheatreMode>,
        theatre_settings_land_id: Option<u32>,
    ) -> Result<()> {
        self.require_no_pending_desired_presentation()?;
        let mut sealed = Vec::with_capacity(observations.len());
        for observation in observations {
            let mut matches = self.index.surfaces.iter().filter(|surface| {
                surface
                    .source_file
                    .eq_ignore_ascii_case(&observation.source_file)
                    && surface
                        .surface_id
                        .eq_ignore_ascii_case(&observation.window_name)
            });
            let role = matches.next().ok_or_else(|| {
                format!(
                    "XUI02 HUD root {}:{} is absent from the authenticated role index",
                    observation.source_file, observation.window_name
                )
            })?;
            if matches.next().is_some() {
                return Err(format!(
                    "XUI02 HUD root {}:{} is ambiguous",
                    observation.source_file, observation.window_name
                )
                .into());
            }
            let panel = self
                .index
                .admit_panel(&self.catalog, admission, &role.root_window_id)?;
            let frontend = admit_discovered_frontend_chrome(&self.index, admission, &panel)
                .map_err(|error| format!("XUI26 persistent chrome rejected: {error:?}"))?;
            let surface = self.seal_surface(admission, panel, frontend)?;
            sealed.push(surface);
        }
        let theatre_available = theatre.is_some();
        let authenticated = game_ui::window_manager_law::derive_exact_a729_persistent_hud_bindings(
            &sealed,
            hud,
            viewport_width,
            viewport_height,
            theatre_available,
        )?;
        let retained_theatre_scroll = self.persistent_panels.values().find_map(|(_, bindings)| {
            bindings.theatre.as_ref().map(|_| {
                bindings
                    .scroll_regions
                    .first()
                    .map_or(0, |row| row.offset_pixels)
            })
        });
        let theatre = theatre
            .map(|surface| {
                game_ui::window_manager_law::derive_exact_a729_theatre_binding(
                    &sealed,
                    &self.index,
                    &self.catalog,
                    admission,
                    viewport_width,
                    viewport_height,
                    surface,
                    theatre_mode,
                    theatre_settings_land_id,
                    retained_theatre_scroll.unwrap_or(0),
                )
            })
            .transpose()?;
        if authenticated.len() != sealed.len() {
            return Err("XUI07 persistent HUD provider returned an incomplete census".into());
        }
        let mut panels = BTreeMap::new();
        let mut panel_order = Vec::with_capacity(sealed.len());
        for (surface, authenticated) in sealed.into_iter().zip(authenticated) {
            let window_id = surface.window_id().to_string();
            let (tooltip_targets, tooltip_selectors) =
                game_ui::tooltip_compositor::derive_authenticated_tooltip_bindings(
                    &surface,
                    authenticated.element_pointer_bindings(),
                )?;
            let theatre = theatre
                .as_ref()
                .filter(|binding| {
                    surface
                        .surface_id()
                        .eq_ignore_ascii_case("theatre_selection_window")
                        && binding.owner_window_id() == window_id
                })
                .cloned();
            let scroll_regions = if let Some(theatre) = theatre.as_ref() {
                let matches = surface
                    .element_roles()
                    .iter()
                    .filter(|role| role.authored_name.eq_ignore_ascii_case("elements_list"))
                    .collect::<Vec<_>>();
                let [role] = matches.as_slice() else {
                    return Err(
                        "XUI07 theatre selector has no unique authenticated elements_list".into(),
                    );
                };
                vec![AuthoredScrollBinding {
                    element_id: role.element_id.clone(),
                    axis: ReplayScrollAxis::Vertical,
                    min_offset_pixels: 0,
                    max_offset_pixels: i32::MAX,
                    offset_pixels: theatre.scroll_offset_pixels(),
                    geometry_source_record:
                        "XUI07/exact-a729-theatre-elements-list/controller-owned".to_string(),
                }]
            } else {
                Vec::new()
            };
            if panels
                .insert(
                    window_id.clone(),
                    (
                        surface,
                        PersistentHudPanelBindings {
                            authenticated,
                            theatre,
                            scroll_regions,
                            tooltip_targets,
                            tooltip_selectors,
                        },
                    ),
                )
                .is_some()
            {
                return Err(format!("XUI02 persistent HUD root {window_id:?} is repeated").into());
            }
            panel_order.push(window_id);
        }
        self.persistent_panels = panels;
        self.persistent_panel_order = panel_order;
        Ok(())
    }

    pub fn pointer_move(
        &mut self,
        _admission: &ValidatedContentAdmission,
        x: i32,
        y: i32,
        _viewer_clock_micros: u64,
    ) -> Result<Option<game_ui::window_manager_law::ReplayPointerTarget>> {
        if !self.desired_state_matches_presented() || !self.pointer_geometry_ready()? {
            self.manager.record_pending_pointer_position(x, y)?;
            return Ok(None);
        }
        let target = self.manager.pointer_move(x, y)?.cloned();
        let regions = self
            .production_lines_bindings
            .iter()
            .map(|(window_id, binding)| (window_id.clone(), binding.list_element_id().to_string()))
            .collect::<Vec<_>>();
        for (window_id, element_id) in regions {
            if let Some(offset) = self.manager.scroll_region_offset(&window_id, &element_id)? {
                self.production_lines_bindings
                    .get_mut(&window_id)
                    .expect("XUI12 binding was enumerated above")
                    .set_scroll_offset_pixels(offset)?;
            }
        }
        Ok(target)
    }

    pub fn production_lines_scroll_offset(&self) -> Result<Option<i32>> {
        let offsets = self
            .production_lines_bindings
            .values()
            .map(ReplayAuthenticatedProductionLinesBinding::scroll_offset_pixels)
            .collect::<Vec<_>>();
        match offsets.as_slice() {
            [] => Ok(None),
            [offset] => Ok(Some(*offset)),
            _ => Err("XUI12 production-line owner census is ambiguous".into()),
        }
    }

    pub fn blocks_lower_pointer_input_at(&self, x: i32, y: i32) -> Result<bool> {
        if !self.pointer_geometry_ready()? {
            return Ok(false);
        }
        Ok(self.manager.blocks_lower_pointer_input_at(x, y)?)
    }

    pub fn pointer_press(
        &mut self,
        _admission: &ValidatedContentAdmission,
        x: i32,
        y: i32,
        _viewer_clock_micros: u64,
    ) -> Result<Option<game_ui::window_manager_law::ReplayPointerTarget>> {
        if !self.desired_state_matches_presented() || !self.pointer_geometry_ready()? {
            self.manager.record_pending_pointer_position(x, y)?;
            return Ok(None);
        }
        Ok(self.manager.pointer_press(x, y)?)
    }

    pub fn pointer_release(
        &mut self,
        _admission: &ValidatedContentAdmission,
        x: i32,
        y: i32,
        _viewer_clock_micros: u64,
    ) -> Result<game_ui::window_manager_law::ReplayPointerRelease> {
        if !self.desired_state_matches_presented() || !self.pointer_geometry_ready()? {
            self.cancel_pointer_capture()?;
            self.manager.record_pending_pointer_position(x, y)?;
            return Ok(game_ui::window_manager_law::ReplayPointerRelease::unavailable(x, y));
        }
        Ok(self.manager.pointer_release(x, y)?)
    }

    pub fn cancel_pointer_capture(&mut self) -> Result<()> {
        Ok(self.manager.cancel_pointer_capture()?)
    }

    pub fn authenticated_tooltip_owner_at_cursor(
        &self,
    ) -> Result<Option<game_ui::window_manager_law::AuthenticatedReplayTooltipOwner>> {
        if !self.desired_state_matches_presented() {
            return Ok(None);
        }
        Ok(self.manager.authenticated_tooltip_owner_at_cursor()?)
    }

    /// Join the current Presented hover to the exact sealed hovered surface,
    /// structural tooltip shell, and viewer pointer. No caller geometry or
    /// authored name is accepted.
    pub fn authenticated_tooltip_context_at_cursor(
        &self,
        admission: &ValidatedContentAdmission,
    ) -> Result<Option<game_ui::tooltip_compositor::AuthenticatedReplayTooltipContext>> {
        if !self.desired_state_matches_presented() {
            return Ok(None);
        }
        let Some(owner) = self.manager.authenticated_tooltip_owner_at_cursor()? else {
            return Ok(None);
        };
        let cursor = self
            .manager
            .pointer_position()
            .ok_or("XUI05 authenticated hover has no viewer pointer position")?;
        let hovered_surface = self
            .opened
            .get(owner.window_id())
            .map(|(surface, _)| surface)
            .or_else(|| {
                self.persistent_panels
                    .get(owner.window_id())
                    .map(|(surface, _)| surface)
            })
            .ok_or("XUI05 authenticated owner has no sealed authored surface")?
            .clone();
        let shell = self
            .index
            .tooltip_shell_surface()
            .ok_or("XUI05 effective authored namespace has no structural tooltip shell")?;
        let shell_panel =
            self.index
                .admit_panel(&self.catalog, admission, &shell.root_window_id)?;
        let shell_frontend = admit_discovered_frontend_chrome(&self.index, admission, &shell_panel)
            .map_err(|error| format!("XUI05 tooltip-shell chrome rejected: {error:?}"))?;
        let tooltip_surface = self.seal_surface(admission, shell_panel, shell_frontend)?;
        Ok(Some(
            game_ui::tooltip_compositor::authenticate_replay_tooltip_context(
                owner,
                hovered_surface,
                tooltip_surface,
                cursor,
            )?,
        ))
    }

    /// Join XUI05 to the exact newly staged geometry and selector that will be
    /// displayed if the current framebuffer Presents. This does not expose
    /// staged geometry to viewer input.
    pub fn authenticated_pending_tooltip_context_at_cursor(
        &self,
        admission: &ValidatedContentAdmission,
    ) -> Result<Option<game_ui::tooltip_compositor::AuthenticatedReplayTooltipContext>> {
        let Some(owner) = self
            .manager
            .authenticated_pending_tooltip_owner_at_cursor()?
        else {
            return Ok(None);
        };
        let cursor = self
            .manager
            .pointer_position()
            .ok_or("XUI05 pending authenticated hover has no viewer pointer position")?;
        let hovered_surface = self
            .opened
            .get(owner.window_id())
            .map(|(surface, _)| surface)
            .or_else(|| {
                self.persistent_panels
                    .get(owner.window_id())
                    .map(|(surface, _)| surface)
            })
            .ok_or("XUI05 pending authenticated owner has no sealed authored surface")?
            .clone();
        let shell = self
            .index
            .tooltip_shell_surface()
            .ok_or("XUI05 effective authored namespace has no structural tooltip shell")?;
        let shell_panel =
            self.index
                .admit_panel(&self.catalog, admission, &shell.root_window_id)?;
        let shell_frontend = admit_discovered_frontend_chrome(&self.index, admission, &shell_panel)
            .map_err(|error| format!("XUI05 tooltip-shell chrome rejected: {error:?}"))?;
        let tooltip_surface = self.seal_surface(admission, shell_panel, shell_frontend)?;
        Ok(Some(
            game_ui::tooltip_compositor::authenticate_replay_tooltip_context(
                owner,
                hovered_surface,
                tooltip_surface,
                cursor,
            )?,
        ))
    }

    /// Seal the same structural XUI05 tooltip shell used by authenticated
    /// manager hover, for packet-owned ExactSemanticPayload producers.
    pub fn authenticated_tooltip_shell_surface(
        &self,
        admission: &ValidatedContentAdmission,
    ) -> Result<AuthenticatedPanelSurface> {
        let shell = self
            .index
            .tooltip_shell_surface()
            .ok_or("XUI05 effective authored namespace has no structural tooltip shell")?;
        let shell_panel =
            self.index
                .admit_panel(&self.catalog, admission, &shell.root_window_id)?;
        let shell_frontend = admit_discovered_frontend_chrome(&self.index, admission, &shell_panel)
            .map_err(|error| format!("XUI05 tooltip-shell chrome rejected: {error:?}"))?;
        self.seal_surface(admission, shell_panel, shell_frontend)
    }

    pub fn set_authenticated_tooltip_owner(
        &mut self,
        owner: &game_ui::window_manager_law::AuthenticatedReplayTooltipOwner,
        tooltip_instance_record: &str,
    ) -> Result<()> {
        Ok(self
            .manager
            .set_authenticated_tooltip_owner(owner, tooltip_instance_record)?)
    }

    pub fn clear_authenticated_tooltip_owner(&mut self) -> Result<()> {
        Ok(self.manager.clear_tooltip()?)
    }

    pub fn consume_authenticated_viewer_action(
        &mut self,
        action: &game_ui::window_manager_law::AuthenticatedReplayViewerAction,
    ) -> Result<game_ui::window_manager_law::ReplayViewerAction> {
        Ok(self.manager.consume_authenticated_viewer_action(action)?)
    }

    pub fn keyboard_traverse(
        &mut self,
        direction: game_ui::window_manager_law::ReplayTraversalDirection,
    ) -> Result<bool> {
        if !self.desired_state_matches_presented() {
            return Ok(self.manager.input_window_id().is_some());
        }
        let Some(window_id) = self.manager.input_window_id() else {
            return Ok(false);
        };
        if self
            .manager
            .panel(window_id)
            .is_none_or(|panel| panel.keyboard_traversal.is_empty())
        {
            return Ok(true);
        }
        self.manager.traverse(
            game_ui::window_manager_law::ReplayTraversalDevice::Keyboard,
            direction,
        )?;
        Ok(true)
    }

    pub fn has_input_window(&self) -> bool {
        self.manager.input_window_id().is_some()
    }

    pub fn activate_focused_viewer_action(
        &mut self,
    ) -> Result<Option<game_ui::window_manager_law::AuthenticatedReplayViewerAction>> {
        if !self.desired_state_matches_presented() {
            return Ok(None);
        }
        Ok(self.manager.activate_focused_viewer_action()?)
    }

    pub fn invalidate_composed_frame(&mut self) {
        if let Some(checkpoint) = self.pending_desired_checkpoint.take() {
            self.restore_desired_checkpoint(*checkpoint);
        }
        self.manager.invalidate_composed_frame();
        self.presented_desired_checkpoint = Box::new(self.desired_checkpoint());
    }

    pub fn confirm_presented_frame(&mut self) -> Result<()> {
        let checkpoint = self.pending_desired_checkpoint.take().ok_or(
            "XUI02 desired-state checkpoint is absent for a Presented authenticated frame",
        )?;
        if let Err(error) = self.manager.confirm_presented_frame() {
            self.pending_desired_checkpoint = Some(checkpoint);
            return Err(error.into());
        }
        self.presented_desired_checkpoint = Box::new(self.desired_checkpoint());
        Ok(())
    }

    pub fn reject_unpresented_frame(&mut self) -> Result<()> {
        let checkpoint = self
            .pending_desired_checkpoint
            .take()
            .ok_or("XUI02 desired-state checkpoint is absent for a rejected authenticated frame")?;
        if let Err(error) = self.manager.reject_unpresented_frame() {
            self.pending_desired_checkpoint = Some(checkpoint);
            return Err(error.into());
        }
        self.restore_desired_checkpoint(*checkpoint);
        Ok(())
    }

    pub fn scroll_at(
        &mut self,
        x: i32,
        y: i32,
        delta_pixels: game_ui::window_manager_law::ReplayScrollDeltaPixels,
    ) -> Result<game_ui::window_manager_law::ReplayScrollOutcome> {
        let outcome = self.manager.scroll_at(x, y, delta_pixels)?;
        self.apply_scroll_outcome(&outcome)?;
        Ok(outcome)
    }

    pub fn scroll_wheel_notches_at(
        &mut self,
        x: i32,
        y: i32,
        emitted_notches: i32,
    ) -> Result<Option<game_ui::window_manager_law::ReplayScrollOutcome>> {
        let Some(outcome) = self
            .manager
            .scroll_wheel_notches_at(x, y, emitted_notches)?
        else {
            return Ok(None);
        };
        self.apply_scroll_outcome(&outcome)?;
        Ok(Some(outcome))
    }

    pub fn production_lines_scroll_offset_for_outcome(
        &self,
        outcome: &game_ui::window_manager_law::ReplayScrollOutcome,
    ) -> Result<Option<i32>> {
        let Some(binding) = self.production_lines_bindings.get(&outcome.window_id) else {
            return Ok(None);
        };
        if outcome.element_id != binding.list_element_id() {
            return Err(format!(
                "XUI12 scroll outcome names foreign region {:?}:{:?}",
                outcome.window_id, outcome.element_id
            )
            .into());
        }
        if outcome.offset_pixels != binding.scroll_offset_pixels() {
            return Err("XUI12 authored binding did not retain the manager scroll outcome".into());
        }
        Ok(Some(outcome.offset_pixels))
    }

    fn apply_scroll_outcome(
        &mut self,
        outcome: &game_ui::window_manager_law::ReplayScrollOutcome,
    ) -> Result<()> {
        if let Some(binding) = self
            .frontend_collection_bindings
            .get_mut(&outcome.window_id)
        {
            binding.set_scroll_offset_pixels(&outcome.element_id, outcome.offset_pixels)?;
            return Ok(());
        }
        if let Some(binding) = self.production_lines_bindings.get_mut(&outcome.window_id) {
            if outcome.element_id != binding.list_element_id() {
                return Err(format!(
                    "XUI12 scroll outcome names foreign region {:?}:{:?}",
                    outcome.window_id, outcome.element_id
                )
                .into());
            }
            binding.set_scroll_offset_pixels(outcome.offset_pixels)?;
            return Ok(());
        }
        let scroll_regions = if let Some((_, bindings)) = self.opened.get_mut(&outcome.window_id) {
            &mut bindings.scroll_regions
        } else if let Some((_, bindings)) = self.persistent_panels.get_mut(&outcome.window_id) {
            &mut bindings.scroll_regions
        } else {
            return Err(format!(
                "XUI02 scroll outcome names unowned window {:?}",
                outcome.window_id
            )
            .into());
        };
        let binding = scroll_regions
            .iter_mut()
            .find(|binding| binding.element_id == outcome.element_id)
            .ok_or_else(|| {
                format!(
                    "XUI02 scroll outcome names unowned region {:?}:{:?}",
                    outcome.window_id, outcome.element_id
                )
            })?;
        binding.offset_pixels = outcome.offset_pixels;
        Ok(())
    }

    fn persist_effective_scroll_offsets(
        &mut self,
        composed: &[game_ui::window_manager_law::ReplayAuthenticatedComposedPanel],
    ) -> Result<()> {
        if composed.len() != self.persistent_panels.len() + self.opened.len() {
            return Err("XUI02 composed scroll-offset census is incomplete".into());
        }
        for panel in composed {
            if let Some(binding) = self.frontend_collection_bindings.get_mut(panel.window_id()) {
                binding.apply_effective_scroll_offsets(panel.effective_scroll_offsets())?;
                continue;
            }
            if let Some(binding) = self.production_lines_bindings.get_mut(panel.window_id()) {
                let effective = panel.effective_scroll_offsets();
                if effective.len() != 1 || effective.get(binding.list_element_id()).is_none() {
                    return Err(format!(
                        "XUI12 composed scroll-offset census differs for window {:?}",
                        panel.window_id()
                    )
                    .into());
                }
                binding.set_scroll_offset_pixels(
                    *effective
                        .get(binding.list_element_id())
                        .expect("XUI12 exact effective offset checked above"),
                )?;
                continue;
            }
            let bindings = if let Some((_, bindings)) = self.opened.get_mut(panel.window_id()) {
                &mut bindings.scroll_regions
            } else if let Some((_, bindings)) = self.persistent_panels.get_mut(panel.window_id()) {
                &mut bindings.scroll_regions
            } else {
                return Err(format!(
                    "XUI02 composed scroll-offset census names unowned window {:?}",
                    panel.window_id()
                )
                .into());
            };
            let effective = panel.effective_scroll_offsets();
            if let Err(error) = apply_effective_scroll_offsets(bindings, effective) {
                return Err(format!(
                    "XUI02 composed scroll-offset census differs for window {:?}: {error}",
                    panel.window_id(),
                )
                .into());
            }
        }
        Ok(())
    }

    pub fn current_window_snapshot(
        &mut self,
        _admission: &ValidatedContentAdmission,
        _viewer_clock_micros: u64,
    ) -> Result<ReplayWindowManagerSnapshot> {
        Ok(self.manager.composed_snapshot()?)
    }

    pub fn compose_open_panels_into(
        &mut self,
        final_framebuffer: &mut game_ui::dds::RgbaImage,
        admission: &ValidatedContentAdmission,
        viewer_clock_micros: u64,
    ) -> Result<Vec<ComposedAuthoredPanel>> {
        self.require_no_pending_desired_presentation()?;
        let viewport_width = final_framebuffer.width;
        let viewport_height = final_framebuffer.height;
        if viewport_width == 0 || viewport_height == 0 {
            return Err("XUI02 viewport is empty".into());
        }
        #[derive(Clone, Copy)]
        enum OwnedBindings<'a> {
            Persistent(&'a PersistentHudPanelBindings),
            Viewer(&'a AuthoredPanelViewerBindings),
        }
        struct OwnedFrameInput<'a> {
            surface: &'a AuthenticatedPanelSurface,
            bindings: OwnedBindings<'a>,
            scroll_bindings: Vec<ReplayAuthenticatedScrollBinding>,
            production_lines: Option<&'a ReplayAuthenticatedProductionLinesBinding>,
            frontend_collections: Option<&'a ReplayAuthenticatedFrontendCollectionsBinding>,
        }
        let mut owned = Vec::with_capacity(self.persistent_panels.len() + self.opened.len());
        for window_id in &self.persistent_panel_order {
            let (surface, bindings) = self.persistent_panels.get(window_id).ok_or_else(|| {
                format!("XUI02 persistent HUD order names missing root {window_id:?}")
            })?;
            owned.push(OwnedFrameInput {
                surface,
                bindings: OwnedBindings::Persistent(bindings),
                scroll_bindings: authenticated_scroll_regions(&bindings.scroll_regions),
                production_lines: None,
                frontend_collections: None,
            });
        }
        for window_id in &self.opened_order {
            let (surface, bindings) = self.opened.get(window_id).ok_or_else(|| {
                format!("XUI02 opened-panel order names missing panel {window_id:?}")
            })?;
            owned.push(OwnedFrameInput {
                surface,
                bindings: OwnedBindings::Viewer(bindings),
                scroll_bindings: authenticated_scroll_regions(&bindings.scroll_regions),
                production_lines: self.production_lines_bindings.get(window_id),
                frontend_collections: self.frontend_collection_bindings.get(window_id),
            });
        }
        for input in &owned {
            if let OwnedBindings::Viewer(bindings) = input.bindings {
                if bindings.viewport_width == viewport_width
                    && bindings.viewport_height == viewport_height
                {
                    continue;
                }
                return Err(format!(
                    "XUI02 panel {:?} semantic viewport {}x{} differs from framebuffer {}x{}",
                    input.surface.window_id(),
                    bindings.viewport_width,
                    bindings.viewport_height,
                    viewport_width,
                    viewport_height
                )
                .into());
            }
        }
        let inputs = owned
            .iter()
            .map(|input| match input.bindings {
                OwnedBindings::Persistent(bindings) => {
                    ReplayAuthenticatedPanelFrameInput::persistent_hud(
                        input.surface,
                        &bindings.authenticated,
                        &input.scroll_bindings,
                        &bindings.tooltip_targets,
                        &bindings.tooltip_selectors,
                        bindings.theatre.as_ref(),
                    )
                }
                OwnedBindings::Viewer(bindings) => {
                    let activation = bindings
                        .activation
                        .as_ref()
                        .expect("viewer bindings were validated before storage");
                    if let Some(capability) = input.production_lines {
                        ReplayAuthenticatedPanelFrameInput::production_lines(
                            input.surface,
                            &bindings.tooltip_selectors,
                            activation,
                            capability,
                        )
                    } else if let Some(capability) = input.frontend_collections {
                        ReplayAuthenticatedPanelFrameInput::frontend_collections(
                            input.surface,
                            viewport_width,
                            viewport_height,
                            &bindings.semantics,
                            &bindings.element_pointer_bindings,
                            &bindings.tooltip_selectors,
                            activation,
                            capability,
                        )
                    } else {
                        ReplayAuthenticatedPanelFrameInput::new(
                            input.surface,
                            viewport_width,
                            viewport_height,
                            &bindings.semantics,
                            &bindings.element_pointer_bindings,
                            &input.scroll_bindings,
                            &bindings.viewer_actions,
                            &bindings.tooltip_selectors,
                            activation,
                        )
                    }
                }
            })
            .collect::<Vec<_>>();
        let frontend_by_window = owned
            .iter()
            .filter_map(|input| {
                input.surface.frontend_composition().family().map(|family| {
                    (
                        input.surface.window_id().to_string(),
                        ComposedFrontendAdmission {
                            family_id: family.family_id().to_string(),
                            source_file: family.source_file().to_string(),
                            admitted_bindings: family
                                .admitted_bindings()
                                .iter()
                                .map(|binding| {
                                    (binding.slot_id.clone(), binding.source_record_id.clone())
                                })
                                .collect(),
                            unavailable_bindings: family
                                .unavailable_bindings()
                                .iter()
                                .map(|binding| {
                                    (binding.slot_id.clone(), binding.reason_code.clone())
                                })
                                .collect(),
                        },
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();
        let rollback = (*self.presented_desired_checkpoint).clone();
        let composed_result = self.manager.compose_authenticated_panel_frame(
            final_framebuffer,
            &self.index,
            admission,
            &inputs,
            viewer_clock_micros,
        );
        drop(inputs);
        drop(owned);
        let composed = match composed_result {
            Ok(composed) => composed,
            Err(error) => {
                self.restore_desired_checkpoint(rollback);
                return Err(error.into());
            }
        };
        if let Err(error) = self.persist_effective_scroll_offsets(&composed) {
            let rollback_error = self.manager.reject_unpresented_frame().err();
            self.restore_desired_checkpoint(rollback);
            if let Some(rollback_error) = rollback_error {
                return Err(format!(
                    "{error}; XUI02 staged scroll-offset rollback also failed: {rollback_error}"
                )
                .into());
            }
            return Err(error);
        }
        self.pending_desired_checkpoint = Some(Box::new(rollback));
        Ok(composed
            .into_iter()
            .map(|panel| ComposedAuthoredPanel {
                window_id: panel.window_id().to_string(),
                runtime_instance_token: panel.runtime_instance_token().to_string(),
                frame_generation: panel.frame_generation(),
                drawn_elements: panel.drawn_elements(),
                frontend: frontend_by_window.get(panel.window_id()).cloned(),
            })
            .collect())
    }

    /// Compose the complete authenticated panel batch while containing a late
    /// XUI12 materialization failure. The manager's compose call is atomic;
    /// after a failed batch we restore the exact pre-attempt desired state,
    /// remove only the production root, clear the scratch framebuffer, and
    /// retry the unrelated batch. A second failure is therefore independent of
    /// XUI12 and remains a whole-runtime error.
    pub(crate) fn compose_open_panels_with_xui12_feature_local_retry(
        &mut self,
        final_framebuffer: &mut game_ui::dds::RgbaImage,
        admission: &ValidatedContentAdmission,
        viewer_clock_micros: u64,
    ) -> Result<(Vec<ComposedAuthoredPanel>, Option<String>)> {
        let desired_before_attempt = self.desired_checkpoint();
        let had_xui12 = !self.production_lines_bindings.is_empty();
        match self.compose_open_panels_into(final_framebuffer, admission, viewer_clock_micros) {
            Ok(panels) => Ok((panels, None)),
            Err(first_error) if had_xui12 => {
                self.restore_desired_checkpoint(desired_before_attempt);
                self.reject_production_lines_canvas_feature_locally();
                *final_framebuffer =
                    game_ui::font::blank_image(final_framebuffer.width, final_framebuffer.height);
                let panels = self
                    .compose_open_panels_into(
                        final_framebuffer,
                        admission,
                        viewer_clock_micros,
                    )
                    .map_err(|retry_error| {
                        format!(
                            "observer authored UI failed with XUI12 present ({first_error}); unrelated retry also failed ({retry_error})"
                        )
                    })?;
                Ok((panels, Some(first_error.to_string())))
            }
            Err(error) => Err(error),
        }
    }

    fn seal_surface(
        &self,
        admission: &ValidatedContentAdmission,
        panel: StrictAuthoredPanelAdmission,
        frontend: DiscoveredFrontendComposition,
    ) -> Result<AuthenticatedPanelSurface> {
        Ok(self.index.seal_panel_surface(
            &self.assets,
            &self.localisation_language,
            &self.localisation,
            admission,
            panel,
            frontend,
        )?)
    }
}

fn authenticated_scroll_regions(
    scroll_regions: &[AuthoredScrollBinding],
) -> Vec<ReplayAuthenticatedScrollBinding> {
    scroll_regions
        .iter()
        .map(|scroll| ReplayAuthenticatedScrollBinding {
            element_id: scroll.element_id.clone(),
            axis: scroll.axis,
            min_offset_pixels: scroll.min_offset_pixels,
            max_offset_pixels: scroll.max_offset_pixels,
            offset_pixels: scroll.offset_pixels,
            producer_record: scroll.geometry_source_record.clone(),
        })
        .collect()
}

fn apply_effective_scroll_offsets(
    bindings: &mut [AuthoredScrollBinding],
    effective: &BTreeMap<String, i32>,
) -> Result<()> {
    let binding_ids = bindings
        .iter()
        .map(|binding| binding.element_id.as_str())
        .collect::<BTreeSet<_>>();
    if binding_ids.len() != bindings.len()
        || effective.len() != bindings.len()
        || !binding_ids
            .iter()
            .all(|element_id| effective.contains_key(*element_id))
    {
        return Err("effective scroll-offset census is incomplete or foreign".into());
    }
    for binding in bindings {
        binding.offset_pixels = *effective
            .get(&binding.element_id)
            .expect("complete effective scroll-offset census checked above");
    }
    Ok(())
}

fn retain_viewer_scroll_offsets(
    previous: &AuthoredPanelViewerBindings,
    next: &mut AuthoredPanelViewerBindings,
) {
    for next_region in &mut next.scroll_regions {
        let Some(previous_region) = previous.scroll_regions.iter().find(|previous_region| {
            previous_region.element_id == next_region.element_id
                && previous_region.axis == next_region.axis
                && previous_region.geometry_source_record == next_region.geometry_source_record
        }) else {
            continue;
        };
        next_region.offset_pixels = previous_region
            .offset_pixels
            .clamp(next_region.min_offset_pixels, next_region.max_offset_pixels);
    }
}

fn install_derived_tooltip_bindings(
    surface: &AuthenticatedPanelSurface,
    bindings: &mut AuthoredPanelViewerBindings,
) -> Result<()> {
    if !bindings.persistent_tooltip_targets.is_empty()
        || !bindings.tooltip_selectors.is_empty()
        || bindings
            .activation
            .as_ref()
            .is_some_and(|activation| !activation.tooltip_targets.is_empty())
    {
        return Err(
            "XUI05 tooltip targets/selectors are sealed-surface outputs, not caller inputs".into(),
        );
    }
    let (targets, selectors) = game_ui::tooltip_compositor::derive_authenticated_tooltip_bindings(
        surface,
        &bindings.element_pointer_bindings,
    )?;
    let activation = bindings
        .activation
        .as_mut()
        .ok_or("XUI05 viewer panel has no activation contract for derived tooltip targets")?;
    activation.tooltip_targets = targets;
    bindings.tooltip_selectors = selectors;
    Ok(())
}

fn validate_viewer_bindings(
    surface: &AuthenticatedPanelSurface,
    bindings: &AuthoredPanelViewerBindings,
    persistent_hud: bool,
) -> Result<()> {
    if bindings.viewport_width == 0 || bindings.viewport_height == 0 {
        return Err("XUI02 viewer binding viewport is empty".into());
    }
    if !bindings.persistent_tooltip_targets.is_empty() {
        return Err("XUI02 viewer panel carries persistent tooltip targets".into());
    }
    match (persistent_hud, bindings.activation.as_ref()) {
        (true, None) => {}
        (false, Some(activation)) if activation.window_id == surface.window_id() => {}
        (true, Some(_)) => {
            return Err("XUI02 persistent HUD root cannot carry viewer-panel activation".into())
        }
        (false, None) => {
            return Err("XUI02 viewer-opened panel lacks exact activation semantics".into())
        }
        (false, Some(_)) => {
            return Err("XUI02 activation identity differs from the admitted panel root".into())
        }
    }
    if bindings
        .activation
        .as_ref()
        .is_some_and(|activation| !activation.controller_traversal.is_empty())
    {
        return Err(
            "XUI25 shipping MouseKeyboard client refuses controller traversal bindings".into(),
        );
    }
    let mut scroll_ids = BTreeSet::new();
    for scroll in &bindings.scroll_regions {
        if scroll.min_offset_pixels > scroll.max_offset_pixels
            || scroll.offset_pixels < scroll.min_offset_pixels
            || scroll.offset_pixels > scroll.max_offset_pixels
            || scroll.geometry_source_record.is_empty()
            || !scroll_ids.insert(scroll.element_id.as_str())
        {
            return Err(format!(
                "XUI02 panel {:?} carries invalid or repeated scroll state for {:?}",
                surface.window_id(),
                scroll.element_id
            )
            .into());
        }
    }
    Ok(())
}

fn register_admitted_authored_roots(
    index: &AuthenticatedGuiRoleIndex,
    catalog: &game_ui::definition_admission::GuiDefinitionCatalog,
) -> Result<DormantWindowManagerRegistry> {
    let mut windows = Vec::with_capacity(index.surfaces.len());
    let mut seen = BTreeSet::new();
    for surface in &index.surfaces {
        if !seen.insert(surface.root_window_id.as_str()) {
            return Err(format!(
                "authored observer UI repeats root window {:?}",
                surface.root_window_id
            )
            .into());
        }
        let row = catalog
            .admitted_windows
            .iter()
            .find(|row| {
                row.window_id == surface.root_window_id
                    && row.source_file == surface.source_file
                    && row.name.eq_ignore_ascii_case(&surface.surface_id)
            })
            .ok_or_else(|| {
                format!(
                    "authored observer root {:?} has no matching admitted root definition",
                    surface.root_window_id
                )
            })?;
        windows.push(DormantWindowDefinition {
            window_id: row.window_id.clone(),
            source_file: row.source_file.clone(),
            element_type: row.element_type.clone(),
            name: row.name.clone(),
            hierarchy_path: row.hierarchy_path.clone(),
            downstream_runtime_dependencies: row.downstream_runtime_dependencies.clone(),
            open: false,
            visible: false,
            composed: false,
            modal_owner: None,
            stack_ordinal: None,
            focus_owner: None,
            restore_focus_to: None,
            tooltip_owner: None,
            keyboard_traversal: Vec::new(),
            controller_traversal: Vec::new(),
        });
    }
    Ok(DormantWindowManagerRegistry { windows })
}

fn validate_authored_dependency_capability(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
) -> Result<()> {
    let complete = admission
        .require_used_content_for_consumer(AUTHORED_DEPENDENCY_CLOSURE_CONSUMER)
        .ok();
    let empty = admission
        .require_used_content_for_consumer(AUTHORED_DEPENDENCY_CLOSURE_EMPTY_CONSUMER)
        .ok();
    if complete.is_some() == empty.is_some() {
        return Err(format!(
            "authored observer UI requires exactly one of dependency capabilities {:?} or {:?}",
            AUTHORED_DEPENDENCY_CLOSURE_CONSUMER, AUTHORED_DEPENDENCY_CLOSURE_EMPTY_CONSUMER
        )
        .into());
    }
    if let Some(carriers) = empty {
        if carriers.len() != 1
            || !index.surfaces.is_empty()
            || !index.dynamic_sprites.is_empty()
            || !index.dynamic_dependency_content.is_empty()
        {
            return Err(
                "authored dependency closure claims empty while effective authored surfaces exist"
                    .into(),
            );
        }
        return Ok(());
    }
    let declared = complete
        .expect("complete-or-empty partition checked above")
        .into_iter()
        .map(|content| content.normalized_game_relative_path.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    for surface in &index.surfaces {
        for content in &surface.dependency_content {
            let path = content.normalized_game_relative_path.to_ascii_lowercase();
            if !declared.contains(&path) {
                return Err(format!(
                    "authored UI dependency capability omits {path:?} required by surface {:?}",
                    surface.surface_id
                )
                .into());
            }
        }
    }
    for content in &index.dynamic_dependency_content {
        let path = content.normalized_game_relative_path.to_ascii_lowercase();
        if !declared.contains(&path) {
            return Err(format!(
                "authored UI dependency capability omits dynamic sprite dependency {path:?}"
            )
            .into());
        }
    }
    Ok(())
}

fn require_complete_authored_namespaces(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    localisation_language: &str,
) -> Result<()> {
    require_complete_namespace(
        resolver,
        admission,
        AUTHORED_GUI_NAMESPACE_CONSUMER,
        AuthoredUiNamespace::Gui,
        localisation_language,
    )?;
    require_complete_namespace(
        resolver,
        admission,
        AUTHORED_GFX_NAMESPACE_CONSUMER,
        AuthoredUiNamespace::Gfx,
        localisation_language,
    )?;
    require_complete_namespace(
        resolver,
        admission,
        AUTHORED_LOCALISATION_NAMESPACE_CONSUMER,
        AuthoredUiNamespace::Localisation,
        localisation_language,
    )?;
    require_complete_namespace(
        resolver,
        admission,
        AUTHORED_SCRIPTED_LOCALISATION_NAMESPACE_CONSUMER,
        AuthoredUiNamespace::ScriptedLocalisation,
        localisation_language,
    )?;
    let dependency_complete = admission
        .require_used_content_for_consumer(AUTHORED_DEPENDENCY_CLOSURE_CONSUMER)
        .is_ok();
    let dependency_empty = admission
        .require_used_content_for_consumer(AUTHORED_DEPENDENCY_CLOSURE_EMPTY_CONSUMER)
        .is_ok();
    if dependency_complete == dependency_empty {
        return Err(format!(
            "authored observer UI requires exactly one of dependency capabilities {:?} or {:?}",
            AUTHORED_DEPENDENCY_CLOSURE_CONSUMER, AUTHORED_DEPENDENCY_CLOSURE_EMPTY_CONSUMER
        )
        .into());
    }
    Ok(())
}

fn require_complete_namespace(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    consumer: &str,
    namespace: AuthoredUiNamespace,
    localisation_language: &str,
) -> Result<()> {
    let complete = admission.require_used_content_for_consumer(consumer).ok();
    let empty = admission
        .require_used_content_for_consumer(namespace.empty_consumer())
        .ok();
    if complete.is_some() == empty.is_some() {
        return Err(format!(
            "authored namespace {namespace:?} requires exactly one of complete capability {consumer:?} or authenticated empty capability {:?}",
            namespace.empty_consumer()
        )
        .into());
    }
    let mut declared_paths = BTreeSet::new();
    if let Some(declared) = complete {
        for content in declared {
            let path = content.normalized_game_relative_path.to_ascii_lowercase();
            if classify_authored_ui_path(&path, localisation_language)? != Some(namespace) {
                return Err(format!(
                    "authored namespace capability {consumer:?} contains an out-of-namespace row {path:?}"
                )
                .into());
            }
            if !declared_paths.insert(path.clone()) {
                return Err(format!(
                    "authored namespace capability {consumer:?} repeats case-folded path {path:?}"
                )
                .into());
            }
        }
    }
    let admitted_gui_paths = admission
        .required_paths()
        .filter_map(|path| {
            let folded = path.to_ascii_lowercase();
            (classify_authored_ui_path(&folded, localisation_language)
                .ok()
                .flatten()
                == Some(namespace))
            .then_some(folded)
        })
        .collect::<BTreeSet<_>>();
    let (directory, extension) = match namespace {
        AuthoredUiNamespace::Gui => ("interface".to_string(), "gui"),
        AuthoredUiNamespace::Gfx => ("interface".to_string(), "gfx"),
        AuthoredUiNamespace::Localisation => (
            format!(
                "localisation/{}",
                safe_language_directory(localisation_language)?
            ),
            "yml",
        ),
        AuthoredUiNamespace::ScriptedLocalisation => {
            ("common/scripted_localisation".to_string(), "txt")
        }
    };
    let resolved_gui_paths = resolver
        .resolve_dir_files_recursive(&directory, Some(extension))?
        .into_iter()
        .map(|(path, _)| path.to_ascii_lowercase())
        .filter(|path| {
            classify_authored_ui_path(path, localisation_language)
                .ok()
                .flatten()
                == Some(namespace)
        })
        .collect::<BTreeSet<_>>();
    if let Some(carriers) = empty {
        if carriers.len() != 1 {
            return Err(format!(
                "authored empty namespace capability {:?} must have exactly one authenticated carrier",
                namespace.empty_consumer()
            )
            .into());
        }
        if !declared_paths.is_empty()
            || !admitted_gui_paths.is_empty()
            || !resolved_gui_paths.is_empty()
        {
            return Err(format!(
                "authored namespace {namespace:?} claims empty but effective or admitted files exist"
            )
            .into());
        }
        return Ok(());
    }
    validate_complete_gui_namespace_sets(
        consumer,
        &declared_paths,
        &admitted_gui_paths,
        &resolved_gui_paths,
    )
}

fn validate_complete_gui_namespace_sets(
    consumer: &str,
    declared_paths: &BTreeSet<String>,
    admitted_gui_paths: &BTreeSet<String>,
    resolved_gui_paths: &BTreeSet<String>,
) -> Result<()> {
    if declared_paths.is_empty() {
        return Err("authored GUI namespace capability is empty".into());
    }
    if admitted_gui_paths != declared_paths {
        return Err(format!(
            "authored namespace capability {consumer:?} differs from admitted rows: declared={declared_paths:?} admitted={admitted_gui_paths:?}"
        )
        .into());
    }
    if resolved_gui_paths != declared_paths {
        return Err(format!(
            "authored namespace capability {consumer:?} differs from effective admitted VFS: declared={declared_paths:?} resolved={resolved_gui_paths:?}"
        )
        .into());
    }
    Ok(())
}

pub fn load_product_gui_definitions(
    resolver: Option<&GameFileResolver>,
) -> Result<Option<GuiDefinitionCatalog>> {
    let Some(resolver) = resolver else {
        return Ok(None);
    };
    let rows = resolver
        .resolve_dir_files_recursive("interface", Some("gui"))?
        .into_iter()
        .map(|(rel_path, path)| ResolvedGuiDefinitionFile::new(rel_path, path))
        .collect::<Vec<_>>();

    let catalog = load_effective_definition_catalog(rows)?;
    eprintln!(
        "replay_app: effective XUI01 GUI definitions admitted: {}/{} files, {}/{} windows; {} windows rejected for explicit unsupported constructs",
        catalog.admitted_file_count(),
        catalog.files.len(),
        catalog.admitted_windows.len(),
        catalog.total_window_count(),
        catalog.rejected_windows.len(),
    );
    Ok(Some(catalog))
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_viewer::PlaybackEnvironment;
    use replay_core::{
        ContentLayerKind, ContentManifest, LayerStorageKind, RawFrame, RecordedContentLayer,
        ReplaySource, RequiredContent, RequiredContentKind, SourceError, SourceGameIdentity,
        SourceProfileIdentity, UsedContentKey, WorldTimeline,
    };
    use serde_json::Map;
    use std::collections::BTreeMap;
    use std::fs;

    fn exact_a729_text_fixture(bytes: &[u8]) -> Vec<u8> {
        String::from_utf8(bytes.to_vec())
            .expect("exact-a729 text fixture is UTF-8")
            .replace("\r\n", "\n")
            .into_bytes()
    }

    fn xui12_fixture() -> (
        AuthoredObserverUi,
        replay_core::ValidatedContentAdmission,
        std::path::PathBuf,
    ) {
        let root = std::env::temp_dir().join(format!(
            "xui12_manager_fixture_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("interface")).unwrap();
        fs::create_dir_all(root.join("gfx/interface")).unwrap();
        fs::create_dir_all(root.join("gfx/FX")).unwrap();
        fs::write(
            root.join("interface/countryproductionlineview.gui"),
            br#"guiTypes = {
                containerWindowType = {
                    name = "countryproductionlineview" position = { x = -160 y = 0 } show_position = { x = 0 y = 0 }
                    show_animation_type = decelerated hide_animation_type = accelerated animation_time = 300 show_sound = menu_open_window
                    size = { width = 160 height = 120 }
                    buttonType = { name = "close_button" position = { x = 140 y = 0 } size = { width = 16 height = 16 } quadTextureSprite = "GFX_xui12_test" shortcut = "ESCAPE" clicksound = click_close pdx_tooltip = "CLOSE" }
                    containerWindowType = {
                        name = "production_lines" position = { x = 0 y = 20 } size = { width = 100 height = 60 }
                        margin = { top = 5 bottom = 5 } verticalScrollbar = "right_vertical_slider" vertical_scroll_step = 55
                        clipping = yes scroll_wheel_factor = 20 smooth_scrolling = yes
                        gridBoxType = { name = "item_grid" position = { x = 0 y = 5 } size = { width = 100 height = 50 } slotsize = { width = 100 height = 1 } format = UPPER_LEFT max_slots_horizontal = 1 add_horizontal = no }
                    }
                }
                containerWindowType = {
                    name = "production_military_line_entry" size = { width = 100 height = 40 }
                    iconType = { name = "row_bg" position = { x = 0 y = 0 } size = { width = 100 height = 40 } spriteType = "GFX_xui12_test" frame = 1 }
                    buttonType = { name = "expand" position = { x = 2 y = 2 } size = { width = 12 height = 12 } quadTextureSprite = "GFX_xui12_test" scale = 1.4 clicksound = click_default }
                }
                containerWindowType = {
                    name = "production_military_line_entry_collapsed" size = { width = 100 height = 20 }
                    iconType = { name = "row_bg" position = { x = 0 y = 0 } size = { width = 100 height = 20 } spriteType = "GFX_xui12_test" frame = 1 }
                    buttonType = { name = "expand" position = { x = 2 y = 2 } size = { width = 12 height = 12 } quadTextureSprite = "GFX_xui12_test" scale = 1.4 clicksound = click_default }
                }
                containerWindowType = {
                    name = "xui12_unrelated_panel" position = { x = 0 y = 0 } size = { width = 32 height = 32 }
                    iconType = { name = "unrelated_icon" position = { x = 0 y = 0 } size = { width = 32 height = 32 } spriteType = "GFX_xui12_test" frame = 1 }
                }
                extendedScrollbarType = {
                    name = "right_vertical_slider" position = { x = -5 y = 0 } size = { width = 18 height = 18 }
                    startValue = 0 orientation = upper_right origo = upper_right smooth_scrolling = 0.25
                    background = { name = "Background" quadTextureSprite = "GFX_xui12_test" }
                    slider = { name = "Slider" quadTextureSprite = "GFX_xui12_test" position = { x = -2 y = -2 } }
                    track = { name = "Track" quadTextureSprite = "GFX_xui12_test" position = { x = 3 y = 0 } alwaystransparent = yes }
                    decreaseButton = { name = "Decrease" quadTextureSprite = "GFX_xui12_test" position = { x = 2 y = 3 } }
                    increaseButton = { name = "Increase" quadTextureSprite = "GFX_xui12_test" position = { x = -17 y = -19 } }
                }
            }"#,
        )
        .unwrap();
        fs::write(
            root.join("interface/xui12_test.gfx"),
            br#"spriteTypes = { spriteType = { name = "GFX_xui12_test" texturefile = "gfx/interface/xui12_test.tga" noOfFrames = 1 } }"#,
        )
        .unwrap();
        fs::write(
            root.join("interface/hotjoin.gui"),
            br#"guiTypes = {
                containerWindowType = {
                    name = "hotjoin_window" position = { x = 0 y = 0 } size = { width = 32 height = 32 }
                    iconType = { name = "background" position = { x = 0 y = 0 } size = { width = 32 height = 32 } spriteType = "GFX_xui12_test" frame = 1 }
                    iconType = { name = "hotjoin_state" position = { x = 0 y = 0 } size = { width = 32 height = 32 } spriteType = "GFX_xui26_test" frame = 1 }
                    gridBoxType = { name = "requests_grid" position = { x = 0 y = 16 } size = { width = 32 height = 4 } slotsize = { width = 32 height = 8 } format = "UPPER_LEFT" scroll_wheel_factor = 8 }
                }
                containerWindowType = {
                    name = "hotjoin_request_entry" position = { x = 0 y = 0 } size = { width = 32 height = 8 }
                    iconType = { name = "event_icon" position = { x = 0 y = 0 } size = { width = 8 height = 8 } spriteType = "GFX_xui12_test" frame = 1 }
                }
            }"#,
        ).unwrap();
        fs::write(
            root.join("interface/xui26_test.gfx"),
            br#"spriteTypes = { spriteType = { name = "GFX_xui26_test" texturefile = "gfx/interface/xui26_test.tga" noOfFrames = 2 } }"#,
        ).unwrap();
        let pixel = [
            0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 0x20, 30, 180, 220, 255,
        ];
        fs::write(root.join("gfx/interface/xui12_test.tga"), pixel).unwrap();
        let xui26_pixel = [
            0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 32, 0x20, 0, 0, 255, 255, 0, 255, 0,
            255,
        ];
        fs::write(root.join("gfx/interface/xui26_test.tga"), xui26_pixel).unwrap();
        fs::write(
            root.join("gfx/FX/buttonstate.shader"),
            exact_a729_text_fixture(include_bytes!(
                "../tests/fixtures/exact_a729/buttonstate.shader"
            )),
        )
        .unwrap();
        fs::write(
            root.join("gfx/FX/buttonstate.fxh"),
            exact_a729_text_fixture(include_bytes!(
                "../tests/fixtures/exact_a729/buttonstate.fxh"
            )),
        )
        .unwrap();
        fs::write(
            root.join("gfx/FX/sprite_animation.fxh"),
            exact_a729_text_fixture(include_bytes!(
                "../tests/fixtures/exact_a729/sprite_animation.fxh"
            )),
        )
        .unwrap();

        let resolver = GameFileResolver::vanilla(&root).unwrap();
        let resolved = resolver
            .resolve_used_content([
                "interface/countryproductionlineview.gui".to_string(),
                "interface/xui12_test.gfx".to_string(),
                "interface/hotjoin.gui".to_string(),
                "interface/xui26_test.gfx".to_string(),
                "gfx/interface/xui12_test.tga".to_string(),
                "gfx/interface/xui26_test.tga".to_string(),
                "gfx/FX/buttonstate.shader".to_string(),
                "gfx/FX/buttonstate.fxh".to_string(),
                "gfx/FX/sprite_animation.fxh".to_string(),
            ])
            .unwrap();
        let vfs = "e".repeat(64);
        let used_rows = resolved
            .iter()
            .map(|row| UsedContentKey {
                effective_vfs_manifest_sha256: vfs.clone(),
                normalized_game_relative_path: row.logical_path.clone(),
                winning_layer_ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                source_sha256: row.sha256.clone(),
                payload_sha256: row.sha256.clone(),
                payload_bytes: row.byte_len,
            })
            .collect::<Vec<_>>();
        let required_content = used_rows
            .iter()
            .cloned()
            .map(|content| RequiredContent {
                content_key: content.normalized_game_relative_path.clone(),
                kind: RequiredContentKind::Other,
                observation_count: 1,
                consumers: vec!["xui12/manager-fixture".into()],
                content,
            })
            .collect();
        let source_game = SourceGameIdentity {
            version: "1.19.2".into(),
            build: "a729".into(),
            executable_sha256: "8".repeat(64),
            build_profile: "release".into(),
        };
        let mut manifest = ContentManifest {
            schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
            product_modset_sha256: "9".repeat(64),
            source_game: source_game.clone(),
            source_profile: SourceProfileIdentity {
                profile_id: "xui12-fixture".into(),
                playset_id: "xui12-fixture".into(),
                selection_sha256: "0".repeat(64),
            },
            effective_vfs_manifest_sha256: vfs,
            ordered_layers: vec![RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "Base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            }],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            required_content,
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest = manifest.with_computed_digest().unwrap();
        let inventory = resolver
            .playback_inventory(
                &manifest,
                PlaybackEnvironment {
                    source_game,
                    source_profile: manifest.source_profile.clone(),
                    dlc_selection: Vec::new(),
                    localisation_language: "l_english".into(),
                    settings: BTreeMap::new(),
                },
            )
            .unwrap();
        let admission = manifest.admit_playback(&inventory).unwrap();
        let resolver = resolver.with_content_admission(&admission).unwrap();
        let assets =
            AssetResolver::from_used_content_rows(&resolver, &admission, used_rows.into_iter())
                .unwrap();
        let (index, catalog) = AuthenticatedGuiRoleIndex::emit_with_game_resolver_and_language(
            &assets,
            &admission,
            None,
            "l_english",
        )
        .unwrap();
        let registry = register_admitted_authored_roots(&index, &catalog).unwrap();
        let manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        let empty = AuthoredObserverUiDesiredCheckpoint {
            opened: BTreeMap::new(),
            opened_order: Vec::new(),
            production_lines_bindings: BTreeMap::new(),
            frontend_collection_bindings: BTreeMap::new(),
            frontend_frame_hours: BTreeMap::new(),
            persistent_panels: BTreeMap::new(),
            persistent_panel_order: Vec::new(),
        };
        (
            AuthoredObserverUi {
                assets,
                localisation_language: "l_english".into(),
                localisation: Localisation::default(),
                index,
                catalog,
                manager,
                opened: BTreeMap::new(),
                opened_order: Vec::new(),
                production_lines_bindings: BTreeMap::new(),
                frontend_collection_bindings: BTreeMap::new(),
                frontend_frame_hours: BTreeMap::new(),
                persistent_panels: BTreeMap::new(),
                persistent_panel_order: Vec::new(),
                presented_desired_checkpoint: Box::new(empty),
                pending_desired_checkpoint: None,
            },
            admission,
            root,
        )
    }

    fn xui12_semantics(
        panel: &StrictAuthoredPanelAdmission,
        button_name: &str,
    ) -> BTreeMap<String, ReplayAuthenticatedElementSemantic> {
        panel
            .element_roles()
            .iter()
            .map(|role| {
                let semantic = if role.authored_name.eq_ignore_ascii_case(button_name) {
                    if button_name.eq_ignore_ascii_case("close_button") {
                        ReplayAuthenticatedElementSemantic::StandardButton
                    } else {
                        ReplayAuthenticatedElementSemantic::ReadOnlyButton {
                            one_based_frame: 1,
                            interaction: game_ui::runtime_graphics::ButtonVisualState::Up,
                        }
                    }
                } else if role.element_type.eq_ignore_ascii_case("iconType") {
                    ReplayAuthenticatedElementSemantic::PassiveSprite { one_based_frame: 1 }
                } else {
                    ReplayAuthenticatedElementSemantic::Structural
                };
                (role.element_id.clone(), semantic)
            })
            .collect()
    }

    struct Xui12FrameSource {
        hours: [i64; 1],
        rest: Map<String, serde_json::Value>,
    }

    impl ReplaySource for Xui12FrameSource {
        fn game_id(&self) -> &str {
            "xui12-manager-authority"
        }

        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: self.rest.clone(),
            })
        }
    }

    fn decoded_xui12_frame(
        rest: Map<String, serde_json::Value>,
    ) -> std::sync::Arc<replay_core::WorldFrame> {
        decoded_frame_at(rest, 1)
    }

    fn decoded_frame_at(
        rest: Map<String, serde_json::Value>,
        hour: i64,
    ) -> std::sync::Arc<replay_core::WorldFrame> {
        WorldTimeline::open(Box::new(Xui12FrameSource {
            hours: [hour],
            rest,
        }))
        .unwrap()
        .state_at(hour)
        .unwrap()
    }

    fn xui26_hotjoin_frame(
        authored: &AuthoredObserverUi,
        admission: &replay_core::ValidatedContentAdmission,
        source_sha256: &str,
        semantic: serde_json::Value,
    ) -> std::sync::Arc<replay_core::WorldFrame> {
        let owner = authored
            .index
            .surfaces
            .iter()
            .find(|surface| surface.surface_id == "hotjoin_window")
            .unwrap();
        let template = authored
            .index
            .surfaces
            .iter()
            .find(|surface| surface.surface_id == "hotjoin_request_entry")
            .unwrap();
        let owner_semantics = owner
            .elements
            .iter()
            .map(|element| {
                let value = match element.authored_name.as_str() {
                    "hotjoin_state" => semantic.clone(),
                    "background" => serde_json::json!({
                        "kind": "passive_sprite", "one_based_frame": 1
                    }),
                    _ => serde_json::json!({"kind": "structural"}),
                };
                (element.element_id.clone(), value)
            })
            .collect::<BTreeMap<_, _>>();
        let row_semantics = template
            .elements
            .iter()
            .map(|element| {
                let semantic = if element.authored_name == "event_icon" {
                    serde_json::json!({"kind": "passive_sprite", "one_based_frame": 1})
                } else {
                    serde_json::json!({"kind": "structural"})
                };
                (element.element_id.clone(), semantic)
            })
            .collect::<BTreeMap<_, _>>();
        let collection_element_id = owner
            .elements
            .iter()
            .find(|element| element.authored_name == "requests_grid")
            .unwrap()
            .element_id
            .clone();
        let mut rest = Map::new();
        let mut carrier = serde_json::json!({
            "family_id": "hotjoin",
            "slot_id": "hotjoin_state",
            "source_record_id": "d57:hotjoin:1",
            "source_content_sha256": source_sha256,
            "complete": true,
            "record_sha256": "",
            "payload": { "kind": "hotjoin_state", "event_record_ids": ["event:1"] },
            "element_semantics": owner_semantics,
            "collections": [{
                "group_id": "hotjoin_requests",
                "collection_element_id": collection_element_id,
                "complete": true,
                "empty_template_source_path": "interface/hotjoin.gui",
                "empty_template_source_content_sha256": source_sha256,
                "empty_template_surface_id": "hotjoin_request_entry",
                "rows": [{
                    "stable_item_id": "event:event:1",
                    "subtype_id": "hotjoin_event",
                    "template_source_path": "interface/hotjoin.gui",
                    "template_source_content_sha256": source_sha256,
                    "template_surface_id": "hotjoin_request_entry",
                    "element_semantics": row_semantics
                }]
            }]
        });
        let typed: replay_core::Xui26ReplayFamilyCarrier =
            serde_json::from_value(carrier.clone()).unwrap();
        carrier["record_sha256"] =
            serde_json::json!(replay_core::xui26_canonical_record_sha256(&typed).unwrap());
        rest.insert(
            replay_core::XUI26_FRONTEND_CARRIER_KEY.into(),
            serde_json::json!({
                "schema": replay_core::XUI26_FRONTEND_CARRIER_SCHEMA,
                "hour_index": 1,
                "authority": "replay_payload",
                "complete": true,
                "product_modset_sha256": admission.product_modset_sha256(),
                "localisation_language": "l_english",
                "carriers": [carrier]
            }),
        );
        decoded_xui12_frame(rest)
    }

    fn xui12_canvas(
        authored: &AuthoredObserverUi,
        admission: &replay_core::ValidatedContentAdmission,
        first_template: &str,
        first_height: i32,
        scroll_offset_px: i32,
    ) -> game_ui::production_lines::StrictXui12CanvasInput {
        xui12_canvas_with_row_count(
            authored,
            admission,
            first_template,
            first_height,
            scroll_offset_px,
            3,
        )
    }

    fn xui12_canvas_with_row_count(
        authored: &AuthoredObserverUi,
        admission: &replay_core::ValidatedContentAdmission,
        first_template: &str,
        first_height: i32,
        scroll_offset_px: i32,
        row_count: usize,
    ) -> game_ui::production_lines::StrictXui12CanvasInput {
        let owner_role = authored
            .index
            .surfaces
            .iter()
            .find(|surface| surface.surface_id == "countryproductionlineview")
            .unwrap();
        let owner = authored
            .index
            .admit_panel(&authored.catalog, admission, &owner_role.root_window_id)
            .unwrap();
        let presentation = |template: &str, height: i32| {
            let role = authored
                .index
                .surfaces
                .iter()
                .find(|surface| surface.surface_id == template)
                .unwrap();
            let panel = authored
                .index
                .admit_panel(&authored.catalog, admission, &role.root_window_id)
                .unwrap();
            game_ui::production_lines::Xui12ProducerRowPresentation {
                template_name: template.into(),
                resolved_width_px: 100,
                resolved_height_px: height,
                semantics: xui12_semantics(&panel, "expand"),
            }
        };
        let row = |identity: i64, ordinal: u32, expanded_height: i32, collapsed_height: i32| {
            game_ui::production_lines::Xui12ProducerCanvasRow {
                row_identity: game_ui::production_lines::Xui12RowIdentity::ExistingLine {
                    kind: game_ui::production_lines::Xui12ProductionKind::Military,
                    line_id: identity,
                },
                global_source_ordinal: ordinal,
                presentations: BTreeMap::from([
                    (
                        game_ui::production_lines::ViewerRowPresentation::Expanded,
                        presentation("production_military_line_entry", expanded_height),
                    ),
                    (
                        game_ui::production_lines::ViewerRowPresentation::Collapsed,
                        presentation("production_military_line_entry_collapsed", collapsed_height),
                    ),
                ]),
            }
        };
        let mut viewer = game_ui::production_lines::Xui12ViewerState::default();
        viewer.scroll_offset_px = scroll_offset_px;
        if first_template.ends_with("_collapsed") {
            viewer.set_row_presentation(
                game_ui::production_lines::Xui12RowIdentity::ExistingLine {
                    kind: game_ui::production_lines::Xui12ProductionKind::Military,
                    line_id: 101,
                },
                game_ui::production_lines::ViewerRowPresentation::Collapsed,
            );
        }
        let first_expanded_height = if first_template.ends_with("_collapsed") {
            40
        } else {
            first_height
        };
        let first_collapsed_height = if first_template.ends_with("_collapsed") {
            first_height
        } else {
            20
        };
        let mut base_rest = Map::new();
        base_rest.insert(
            "countries".into(),
            serde_json::json!({"GER": {"_r2_tag_db_id": 1}}),
        );
        let frame_identity = game_ui::production_lines::xui12_replay_frame_identity_sha256(
            decoded_xui12_frame(base_rest.clone()).as_ref(),
        )
        .unwrap();
        let mut rows = vec![
            row(101, 0, first_expanded_height, first_collapsed_height),
            row(202, 1, 40, 20),
            row(303, 2, 40, 20),
        ];
        rows.truncate(row_count);
        let country = game_ui::production_lines::Xui12ProducerCanvasPayload {
            country_tag: "GER".into(),
            tag_db_id: 1,
            source_ordinal: 0,
            measured_empty: rows.is_empty(),
            owner_semantics: xui12_semantics(&owner, "close_button"),
            rows,
        };
        let completeness = game_ui::production_lines::Xui12ProducerCompletenessAuthority {
            countries: vec![
                game_ui::production_lines::Xui12ProducerCountryCompletenessAuthority {
                    country_tag: "GER".into(),
                    tag_db_id: 1,
                    source_ordinal: 0,
                    row_identities: country
                        .rows
                        .iter()
                        .map(|row| row.row_identity.clone())
                        .collect(),
                },
            ],
        };
        let payload = game_ui::production_lines::Xui12ProducerFramePayload {
            executable_sha256: map_viewer::XMAP03_A729_EXECUTABLE_SHA256.into(),
            producer_manifest_sha256: admission.manifest_digest_sha256().into(),
            replay_frame_identity_sha256: frame_identity,
            completeness_record_sha256: game_ui::production_lines::xui12_completeness_record_sha256(
                &completeness,
            ),
            countries: vec![country],
        };
        let mut rest = base_rest;
        rest.insert(
            game_ui::production_lines::XUI12_PRODUCER_PAYLOAD_FIELD.into(),
            serde_json::to_value(&payload).unwrap(),
        );
        rest.insert(
            game_ui::production_lines::XUI12_PRODUCER_AUTHORITY_FIELD.into(),
            payload.replay_authority_record(1, &completeness),
        );
        let frame = decoded_xui12_frame(rest);
        game_ui::production_lines::seal_xui12_producer_canvas_for_frame_and_viewer(
            payload,
            frame.as_ref(),
            admission,
            "GER",
            &viewer,
        )
        .unwrap()
    }

    fn paths(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn effective_extent_clamp_replaces_retained_scroll_state_without_resurrection() {
        let mut bindings = vec![AuthoredScrollBinding {
            element_id: "theatre-elements-list".into(),
            axis: ReplayScrollAxis::Vertical,
            min_offset_pixels: 0,
            max_offset_pixels: i32::MAX,
            offset_pixels: 180,
            geometry_source_record: "XUI07/controller-owned".into(),
        }];
        apply_effective_scroll_offsets(
            &mut bindings,
            &BTreeMap::from([("theatre-elements-list".into(), 100)]),
        )
        .unwrap();
        assert_eq!(bindings[0].offset_pixels, 100);

        let previous = AuthoredPanelViewerBindings {
            viewport_width: 1920,
            viewport_height: 1080,
            semantics: BTreeMap::new(),
            activation: None,
            element_pointer_bindings: BTreeMap::new(),
            viewer_actions: Vec::new(),
            persistent_tooltip_targets: Vec::new(),
            tooltip_selectors: BTreeMap::new(),
            scroll_regions: bindings,
        };
        let mut regrown = previous.clone();
        regrown.scroll_regions[0].max_offset_pixels = 300;
        regrown.scroll_regions[0].offset_pixels = 0;
        retain_viewer_scroll_offsets(&previous, &mut regrown);
        assert_eq!(
            regrown.scroll_regions[0].offset_pixels, 100,
            "content regrowth must retain the last presented effective offset, not the stale raw 180"
        );
    }

    #[test]
    fn effective_scroll_census_refusal_is_transactional() {
        let mut bindings = vec![AuthoredScrollBinding {
            element_id: "owned".into(),
            axis: ReplayScrollAxis::Vertical,
            min_offset_pixels: 0,
            max_offset_pixels: 200,
            offset_pixels: 75,
            geometry_source_record: "test".into(),
        }];
        assert!(apply_effective_scroll_offsets(
            &mut bindings,
            &BTreeMap::from([("foreign".into(), 10)]),
        )
        .is_err());
        assert_eq!(bindings[0].offset_pixels, 75);
    }

    #[test]
    fn complete_namespace_accepts_mod_additions_and_remove_path_absence() {
        let effective = paths(&[
            "interface/base_survivor.gui",
            "interface/mod_added_panel.gui",
        ]);
        validate_complete_gui_namespace_sets(
            AUTHORED_GUI_NAMESPACE_CONSUMER,
            &effective,
            &effective,
            &effective,
        )
        .unwrap();
        assert!(!effective.contains("interface/base_removed_by_replace_path.gui"));
    }

    #[test]
    fn complete_namespace_refuses_an_omitted_dormant_mod_panel() {
        let declared = paths(&["interface/base.gui"]);
        let effective = paths(&["interface/base.gui", "interface/mod_added_panel.gui"]);
        assert!(validate_complete_gui_namespace_sets(
            AUTHORED_GUI_NAMESPACE_CONSUMER,
            &declared,
            &effective,
            &effective
        )
        .is_err());
    }

    #[test]
    fn authored_localisation_namespace_follows_safe_playback_language() {
        assert_eq!(
            portable_content_manifest::authored_ui::safe_language_directory("l_braz_por").unwrap(),
            "braz_por"
        );
        assert_eq!(
            portable_content_manifest::authored_ui::safe_language_directory("english").unwrap(),
            "english"
        );
        assert_eq!(
            portable_content_manifest::authored_ui::safe_language_directory("l_ENGLISH").unwrap(),
            "english"
        );
        for invalid in ["", "l_", "../english", "l-english"] {
            assert!(
                portable_content_manifest::authored_ui::safe_language_directory(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn xui12_complete_canvas_draws_scrolls_activates_and_reflows_by_stable_identity() {
        let (mut authored, admission, root) = xui12_fixture();
        let expanded = xui12_canvas(
            &authored,
            &admission,
            "production_military_line_entry",
            40,
            0,
        );
        let window_id = authored
            .set_production_lines_canvas(&admission, 160, 120, &expanded, 1)
            .unwrap();
        let mut framebuffer = game_ui::font::blank_image(160, 120);
        let reports = authored
            .compose_open_panels_into(&mut framebuffer, &admission, 1)
            .unwrap();
        assert!(reports.iter().any(|panel| panel.window_id == window_id));
        assert!(framebuffer
            .pixels
            .chunks_exact(4)
            .any(|pixel| pixel[3] != 0));
        authored.confirm_presented_frame().unwrap();

        let snapshot = authored.current_window_snapshot(&admission, 1).unwrap();
        let panel = snapshot
            .stack
            .iter()
            .find(|panel| panel.window_id == window_id)
            .unwrap();
        assert_eq!(panel.scroll_regions.len(), 1);
        assert_eq!(panel.scroll_regions[0].wheel_step_pixels, Some(20));
        assert!(panel.scroll_regions[0].smooth_scrolling);
        assert_eq!(panel.screen_rect.x(), 0);
        assert_eq!(panel.scroll_regions[0].viewport_rect.y(), 25);
        assert_eq!(panel.scroll_regions[0].viewport_rect.h(), 50);
        let scrollbar = panel.scroll_regions[0]
            .extended_scrollbar
            .as_ref()
            .expect("XUI12 exact extended scrollbar geometry");
        assert!(scrollbar.thumb_travel_pixels > 0);
        assert!(panel.hit_targets.iter().any(|target| {
            target.element_type == "extendedScrollbarSlider"
                && target.screen_rect == scrollbar.slider_rect
        }));
        let second_y_expanded = panel
            .hit_targets
            .iter()
            .find(|target| target.authored_name == "expand" && target.child_path.get(1) == Some(&1))
            .unwrap()
            .screen_rect
            .y();
        let first_expand = panel
            .hit_targets
            .iter()
            .find(|target| target.authored_name == "expand" && target.child_path.get(1) == Some(&0))
            .unwrap();
        let x = first_expand.screen_rect.x() + 1;
        let y = first_expand.screen_rect.y() + 1;
        authored.pointer_move(&admission, x, y, 1).unwrap();
        authored.pointer_press(&admission, x, y, 1).unwrap();
        let release = authored.pointer_release(&admission, x, y, 1).unwrap();
        assert!(matches!(
            release.activated_action().map(|action| action.action()),
            Some(
                game_ui::window_manager_law::ReplayViewerAction::ProductionLines(
                    game_ui::window_manager_law::ProductionLinesViewerAction::ToggleRow(
                        game_ui::production_lines::Xui12RowIdentity::ExistingLine {
                            kind: game_ui::production_lines::Xui12ProductionKind::Military,
                            line_id: 101,
                        }
                    )
                )
            )
        ));

        let region = &panel.scroll_regions[0];
        let scroll_x = region.viewport_rect.x() + 1;
        let scroll_y = region.viewport_rect.y() + 1;
        let scroll = authored
            .scroll_wheel_notches_at(scroll_x, scroll_y, -1)
            .unwrap()
            .unwrap();
        assert_eq!(scroll.offset_pixels, 20);
        assert_eq!(
            authored
                .production_lines_scroll_offset_for_outcome(&scroll)
                .unwrap(),
            Some(20)
        );

        let scrolled_expanded = xui12_canvas(
            &authored,
            &admission,
            "production_military_line_entry",
            40,
            20,
        );
        authored
            .set_production_lines_canvas(&admission, 160, 120, &scrolled_expanded, 2)
            .unwrap();
        let mut framebuffer = game_ui::font::blank_image(160, 120);
        authored
            .compose_open_panels_into(&mut framebuffer, &admission, 2)
            .unwrap();
        authored.confirm_presented_frame().unwrap();
        let snapshot = authored.current_window_snapshot(&admission, 2).unwrap();
        let panel = snapshot
            .stack
            .iter()
            .find(|panel| panel.window_id == window_id)
            .unwrap();
        let second_y_scrolled_expanded = panel
            .hit_targets
            .iter()
            .find(|target| target.authored_name == "expand" && target.child_path.get(1) == Some(&1))
            .unwrap()
            .screen_rect
            .y();

        let collapsed = xui12_canvas(
            &authored,
            &admission,
            "production_military_line_entry_collapsed",
            20,
            20,
        );
        authored
            .set_production_lines_canvas(&admission, 160, 120, &collapsed, 3)
            .unwrap();
        let mut framebuffer = game_ui::font::blank_image(160, 120);
        authored
            .compose_open_panels_into(&mut framebuffer, &admission, 3)
            .unwrap();
        authored.confirm_presented_frame().unwrap();
        let snapshot = authored.current_window_snapshot(&admission, 3).unwrap();
        let panel = snapshot
            .stack
            .iter()
            .find(|panel| panel.window_id == window_id)
            .unwrap();
        let slider = panel
            .hit_targets
            .iter()
            .find(|target| target.element_type == "extendedScrollbarSlider")
            .unwrap();
        let slider_x = slider.screen_rect.x();
        let slider_y = slider.screen_rect.y();
        authored
            .pointer_move(&admission, slider_x, slider_y, 3)
            .unwrap();
        authored
            .pointer_press(&admission, slider_x, slider_y, 3)
            .unwrap();
        authored
            .pointer_move(&admission, slider_x, slider_y + 12, 3)
            .unwrap();
        let drag_release = authored
            .pointer_release(&admission, slider_x, slider_y + 12, 3)
            .unwrap();
        assert!(!drag_release.activated());
        let dragged_offset = authored
            .production_lines_scroll_offset()
            .unwrap()
            .expect("XUI12 binding retains the exact drag offset");
        assert!(dragged_offset > 20);
        let second_y_collapsed = panel
            .hit_targets
            .iter()
            .find(|target| target.authored_name == "expand" && target.child_path.get(1) == Some(&1))
            .unwrap()
            .screen_rect
            .y();
        assert_eq!(second_y_expanded - second_y_scrolled_expanded, 20);
        assert_eq!(second_y_scrolled_expanded - second_y_collapsed, 20);

        let dragged = xui12_canvas(
            &authored,
            &admission,
            "production_military_line_entry_collapsed",
            20,
            dragged_offset,
        );
        authored
            .set_production_lines_canvas(&admission, 160, 120, &dragged, 4)
            .unwrap();
        let mut framebuffer = game_ui::font::blank_image(160, 120);
        authored
            .compose_open_panels_into(&mut framebuffer, &admission, 4)
            .unwrap();
        authored.confirm_presented_frame().unwrap();
        let snapshot = authored.current_window_snapshot(&admission, 4).unwrap();
        let panel = snapshot
            .stack
            .iter()
            .find(|panel| panel.window_id == window_id)
            .unwrap();
        let close = panel
            .hit_targets
            .iter()
            .find(|target| target.authored_name == "close_button")
            .unwrap();
        let close_x = close.screen_rect.x() + 1;
        let close_y = close.screen_rect.y() + 1;
        authored
            .pointer_move(&admission, close_x, close_y, 4)
            .unwrap();
        let tooltip_owner = authored
            .authenticated_tooltip_owner_at_cursor()
            .unwrap()
            .expect("sealed close-button pdx_tooltip is a shipping hover target");
        assert_eq!(tooltip_owner.authored_name(), "close_button");
        assert_eq!(
            tooltip_owner.alternate_selection(),
            ReplayTooltipAlternateSelection::AuthoredDefault
        );
        let pending_canvas = xui12_canvas(
            &authored,
            &admission,
            "production_military_line_entry_collapsed",
            20,
            dragged_offset,
        );
        authored
            .set_production_lines_canvas(&admission, 160, 120, &pending_canvas, 5)
            .unwrap();
        let mut pending_framebuffer = game_ui::font::blank_image(160, 120);
        authored
            .compose_open_panels_into(&mut pending_framebuffer, &admission, 5)
            .unwrap();
        let staged_owner = authored
            .manager
            .authenticated_pending_tooltip_owner_at_cursor()
            .unwrap()
            .expect("the staged framebuffer retains an exact tooltip owner");
        assert!(staged_owner.frame_generation() > tooltip_owner.frame_generation());
        assert_eq!(staged_owner.screen_rect(), tooltip_owner.screen_rect());
        assert_eq!(
            staged_owner.alternate_selection(),
            tooltip_owner.alternate_selection()
        );
        authored.reject_unpresented_frame().unwrap();
        authored
            .pointer_press(&admission, close_x, close_y, 4)
            .unwrap();
        let close_release = authored
            .pointer_release(&admission, close_x, close_y, 4)
            .unwrap();
        assert!(
            matches!(
                close_release
                    .activated_action()
                    .map(|action| action.action()),
                Some(
                    game_ui::window_manager_law::ReplayViewerAction::ProductionLines(
                        game_ui::window_manager_law::ProductionLinesViewerAction::Close
                    )
                )
            ),
            "unexpected close release after scrollbar drag: {close_release:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xui26_typed_carrier_changes_pixels_blocks_input_and_reopen_changes_lifetime() {
        let (mut authored, admission, root) = xui12_fixture();
        let unrelated = authored
            .index
            .surfaces
            .iter()
            .find(|surface| surface.surface_id == "xui12_unrelated_panel")
            .unwrap()
            .clone();
        let unrelated_panel = authored
            .index
            .admit_panel(&authored.catalog, &admission, &unrelated.root_window_id)
            .unwrap();
        let unrelated_bindings = AuthoredPanelViewerBindings {
            viewport_width: 64,
            viewport_height: 64,
            semantics: xui12_semantics(&unrelated_panel, "no_interactive_element"),
            activation: Some(ReplayPanelActivation {
                window_id: unrelated.root_window_id.clone(),
                modal_owner: None,
                initial_focus: None,
                keyboard_traversal: Vec::new(),
                controller_traversal: Vec::new(),
                tooltip_targets: Vec::new(),
            }),
            element_pointer_bindings: unrelated_panel
                .element_roles()
                .iter()
                .map(|role| {
                    (
                        role.element_id.clone(),
                        ReplayElementPointerAuthority::PassThrough,
                    )
                })
                .collect(),
            viewer_actions: Vec::new(),
            persistent_tooltip_targets: Vec::new(),
            tooltip_selectors: BTreeMap::new(),
            scroll_regions: Vec::new(),
        };
        authored
            .open_panel_with_bindings(&admission, &unrelated.root_window_id, unrelated_bindings, 0)
            .unwrap();
        let role = authored
            .index
            .surfaces
            .iter()
            .find(|surface| surface.surface_id == "hotjoin_window")
            .unwrap();
        let source_sha256 = role.source_content.payload_sha256.clone();
        let frame_two = xui26_hotjoin_frame(
            &authored,
            &admission,
            &source_sha256,
            serde_json::json!({"kind":"passive_sprite","one_based_frame":2}),
        );
        let opened = authored
            .cycle_frontend_panel(&admission, &frame_two, 64, 64, 1)
            .unwrap()
            .unwrap();
        assert_eq!(opened.family_id, "hotjoin");
        let mut first = game_ui::font::blank_image(64, 64);
        let first_reports = authored
            .compose_open_panels_into(&mut first, &admission, 1)
            .unwrap();
        let first_report = first_reports
            .iter()
            .find(|report| report.window_id == opened.window_id)
            .unwrap();
        let first_token = first_report.runtime_instance_token.clone();
        assert!(
            first_report.drawn_elements >= 3,
            "hotjoin owner sprites plus the authenticated event row must reach the compositor"
        );
        authored.confirm_presented_frame().unwrap();
        let initial_snapshot = authored.current_window_snapshot(&admission, 1).unwrap();
        let initial_panel = initial_snapshot
            .stack
            .iter()
            .find(|panel| panel.window_id == opened.window_id)
            .unwrap();
        assert_eq!(initial_panel.scroll_regions.len(), 1);
        let region = &initial_panel.scroll_regions[0];
        assert!(region
            .geometry_source_record
            .contains("group=hotjoin_requests"));
        assert_eq!(region.max_offset_pixels, 4);
        assert_eq!(region.offset_pixels, 0);
        assert_eq!(region.wheel_step_pixels, Some(8));
        let scroll = authored
            .scroll_wheel_notches_at(
                region.viewport_rect.x() + 1,
                region.viewport_rect.y() + 1,
                -1,
            )
            .unwrap()
            .unwrap();
        assert_eq!(scroll.offset_pixels, 4);
        let mut scrolled = game_ui::font::blank_image(64, 64);
        authored
            .compose_open_panels_into(&mut scrolled, &admission, 2)
            .unwrap();
        authored.confirm_presented_frame().unwrap();
        let scrolled_snapshot = authored.current_window_snapshot(&admission, 2).unwrap();
        let scrolled_panel = scrolled_snapshot
            .stack
            .iter()
            .find(|panel| panel.window_id == opened.window_id)
            .unwrap();
        assert_eq!(scrolled_panel.scroll_regions[0].offset_pixels, 4);
        let target = authored.pointer_move(&admission, 4, 4, 2).unwrap().unwrap();
        assert_eq!(
            target.disposition,
            game_ui::window_manager_law::ReplayPointerDisposition::BlockOnly
        );

        let next_hour_without_carrier = decoded_frame_at(Map::new(), 2);
        assert!(authored
            .reconcile_frontend_frame(&next_hour_without_carrier)
            .unwrap());
        assert!(!authored.opened.contains_key(&opened.window_id));

        authored.close_frontend_panels().unwrap();
        let mut closed = game_ui::font::blank_image(64, 64);
        let closed_reports = authored
            .compose_open_panels_into(&mut closed, &admission, 3)
            .unwrap();
        assert!(closed_reports
            .iter()
            .all(|report| report.window_id != opened.window_id));
        authored.confirm_presented_frame().unwrap();
        let hidden = xui26_hotjoin_frame(
            &authored,
            &admission,
            &source_sha256,
            serde_json::json!({"kind":"hidden"}),
        );
        let reopened = authored
            .cycle_frontend_panel(&admission, &hidden, 64, 64, 4)
            .unwrap()
            .unwrap();
        let mut second = game_ui::font::blank_image(64, 64);
        let second_reports = authored
            .compose_open_panels_into(&mut second, &admission, 4)
            .unwrap();
        let second_token = second_reports
            .iter()
            .find(|report| report.window_id == reopened.window_id)
            .unwrap()
            .runtime_instance_token
            .clone();
        assert_ne!(first_token, second_token);
        assert_ne!(
            first.pixels, second.pixels,
            "typed XUI26 semantics must affect compositor pixels"
        );
        authored.reject_unpresented_frame().unwrap();
        let snapshot = authored.current_window_snapshot(&admission, 5).unwrap();
        assert!(snapshot
            .stack
            .iter()
            .all(|panel| panel.window_id != reopened.window_id));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xui12_rejection_is_atomic_and_feature_local_cleanup_removes_stale_canvas() {
        let (mut authored, admission, root) = xui12_fixture();
        let valid = xui12_canvas(
            &authored,
            &admission,
            "production_military_line_entry",
            40,
            0,
        );
        authored
            .set_production_lines_canvas(&admission, 160, 120, &valid, 1)
            .unwrap();

        let unrelated_role = authored
            .index
            .surfaces
            .iter()
            .find(|surface| surface.surface_id == "xui12_unrelated_panel")
            .unwrap()
            .clone();
        let unrelated_panel = authored
            .index
            .admit_panel(
                &authored.catalog,
                &admission,
                &unrelated_role.root_window_id,
            )
            .unwrap();
        let unrelated_frontend =
            admit_discovered_frontend_chrome(&authored.index, &admission, &unrelated_panel)
                .unwrap();
        let unrelated_surface = authored
            .seal_surface(&admission, unrelated_panel, unrelated_frontend)
            .unwrap();
        authored.opened.insert(
            unrelated_role.root_window_id.clone(),
            (
                unrelated_surface,
                AuthoredPanelViewerBindings {
                    viewport_width: 160,
                    viewport_height: 120,
                    semantics: BTreeMap::new(),
                    activation: None,
                    element_pointer_bindings: BTreeMap::new(),
                    viewer_actions: Vec::new(),
                    persistent_tooltip_targets: Vec::new(),
                    tooltip_selectors: BTreeMap::new(),
                    scroll_regions: Vec::new(),
                },
            ),
        );
        authored
            .opened_order
            .push(unrelated_role.root_window_id.clone());
        let before_rejection = authored.desired_checkpoint();

        let mut tampered = valid;
        tampered.rows[0].template_name.push_str("_tampered");
        assert!(authored
            .set_production_lines_canvas(&admission, 160, 120, &tampered, 2)
            .is_err());
        assert!(authored.desired_checkpoint() == before_rejection);

        authored.reject_production_lines_canvas_feature_locally();
        assert!(authored.production_lines_bindings.is_empty());
        assert!(authored.opened.contains_key(&unrelated_role.root_window_id));
        assert!(authored.opened.values().all(|panel| {
            !panel
                .0
                .surface_id()
                .eq_ignore_ascii_case(game_ui::production_lines::COUNTRY_PRODUCTION_LINE_ROOT)
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn late_xui12_materialization_failure_recomposes_unrelated_authored_panel() {
        let (mut authored, admission, root) = xui12_fixture();
        let unrelated_role = authored
            .index
            .surfaces
            .iter()
            .find(|surface| surface.surface_id == "xui12_unrelated_panel")
            .unwrap()
            .clone();
        let unrelated_panel = authored
            .index
            .admit_panel(
                &authored.catalog,
                &admission,
                &unrelated_role.root_window_id,
            )
            .unwrap();
        let unrelated_bindings = AuthoredPanelViewerBindings {
            viewport_width: 160,
            viewport_height: 120,
            semantics: xui12_semantics(&unrelated_panel, "no_interactive_element"),
            activation: Some(ReplayPanelActivation {
                window_id: unrelated_role.root_window_id.clone(),
                modal_owner: None,
                initial_focus: None,
                keyboard_traversal: Vec::new(),
                controller_traversal: Vec::new(),
                tooltip_targets: Vec::new(),
            }),
            element_pointer_bindings: unrelated_panel
                .element_roles()
                .iter()
                .map(|role| {
                    (
                        role.element_id.clone(),
                        game_ui::window_manager_law::ReplayElementPointerAuthority::PassThrough,
                    )
                })
                .collect(),
            viewer_actions: Vec::new(),
            persistent_tooltip_targets: Vec::new(),
            tooltip_selectors: BTreeMap::new(),
            scroll_regions: Vec::new(),
        };
        authored
            .open_panel_with_bindings(
                &admission,
                &unrelated_role.root_window_id,
                unrelated_bindings,
                1,
            )
            .unwrap();

        // This canvas passes producer/content admission but deliberately names
        // a viewport that differs from the actual framebuffer. The failure is
        // late and XUI12-local; the unrelated panel must still recompose.
        let wrong_viewport = xui12_canvas(
            &authored,
            &admission,
            "production_military_line_entry",
            40,
            0,
        );
        authored
            .set_production_lines_canvas(&admission, 161, 120, &wrong_viewport, 1)
            .unwrap();
        let mut framebuffer = game_ui::font::blank_image(160, 120);
        let (panels, late_rejection) = authored
            .compose_open_panels_with_xui12_feature_local_retry(&mut framebuffer, &admission, 1)
            .unwrap();

        assert!(late_rejection
            .as_deref()
            .is_some_and(|error| error.contains("semantic viewport")));
        assert!(authored.production_lines_bindings.is_empty());
        assert!(panels
            .iter()
            .any(|panel| panel.window_id == unrelated_role.root_window_id));
        assert!(framebuffer
            .pixels
            .chunks_exact(4)
            .any(|pixel| pixel[3] != 0));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xui12_authenticated_extent_clamps_retained_offset_after_collapse_and_row_removal() {
        let (mut authored, admission, root) = xui12_fixture();
        let expanded = xui12_canvas(
            &authored,
            &admission,
            "production_military_line_entry",
            40,
            70,
        );
        authored
            .set_production_lines_canvas(&admission, 160, 120, &expanded, 1)
            .unwrap();
        let mut framebuffer = game_ui::font::blank_image(160, 120);
        let (_, rejection) = authored
            .compose_open_panels_with_xui12_feature_local_retry(&mut framebuffer, &admission, 1)
            .unwrap();
        assert_eq!(rejection, None);
        assert_eq!(authored.production_lines_scroll_offset().unwrap(), Some(70));
        authored.confirm_presented_frame().unwrap();

        let collapsed = xui12_canvas(
            &authored,
            &admission,
            "production_military_line_entry_collapsed",
            20,
            70,
        );
        authored
            .set_production_lines_canvas(&admission, 160, 120, &collapsed, 2)
            .unwrap();
        let mut framebuffer = game_ui::font::blank_image(160, 120);
        let (_, rejection) = authored
            .compose_open_panels_with_xui12_feature_local_retry(&mut framebuffer, &admission, 2)
            .unwrap();
        assert_eq!(rejection, None);
        assert_eq!(authored.production_lines_scroll_offset().unwrap(), Some(50));
        authored.confirm_presented_frame().unwrap();

        let one_row = xui12_canvas_with_row_count(
            &authored,
            &admission,
            "production_military_line_entry_collapsed",
            20,
            50,
            1,
        );
        authored
            .set_production_lines_canvas(&admission, 160, 120, &one_row, 3)
            .unwrap();
        let mut framebuffer = game_ui::font::blank_image(160, 120);
        let (_, rejection) = authored
            .compose_open_panels_with_xui12_feature_local_retry(&mut framebuffer, &admission, 3)
            .unwrap();
        assert_eq!(rejection, None);
        assert_eq!(authored.production_lines_scroll_offset().unwrap(), Some(0));
        authored.confirm_presented_frame().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn authored_observer_initializes_from_the_non_english_playback_capability() {
        let root = std::env::temp_dir().join(format!(
            "xui24_non_english_init_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("interface")).unwrap();
        fs::create_dir_all(root.join("localisation/japanese")).unwrap();
        fs::create_dir_all(root.join("gfx/fonts")).unwrap();
        fs::create_dir_all(root.join("gfx/FX")).unwrap();
        fs::write(
            root.join("interface/panel.gui"),
            b"guiTypes={ containerWindowType={ name=mod_panel iconType={ name=icon spriteType=GFX_test frame=1 } instantTextBoxType={ name=label text=JP_LABEL font=Alias } } }",
        )
        .unwrap();
        let pixel = [
            0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 0x20, 255, 255, 255, 255,
        ];
        fs::write(
            root.join("interface/core.gfx"),
            b"bitmapfonts={ textcolors={ G={ 0 159 3 } } bitmapfont={ name=Alias path=\"gfx/fonts/base\" color=0xff010203 border_color=0xff040506 } bitmapfont_override={ name=Alias fontfiles={ \"gfx/fonts/jp_primary\" \"gfx/fonts/jp_fallback\" } languages={ \"l_japanese\" } } } spriteTypes={ spriteType={ name=GFX_test texturefile1=\"gfx/interface/icon.tga\" } }",
        )
        .unwrap();
        fs::write(
            root.join("gfx/fonts/base.fnt"),
            b"info face=\"base\" size=16\ncommon lineHeight=16 base=12 scaleW=1 scaleH=1 pages=1\npage id=0 file=\"base.tga\"\nchars count=1\nchar id=65 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0 chnl=15\n",
        )
        .unwrap();
        fs::write(root.join("gfx/fonts/base.tga"), pixel).unwrap();
        fs::write(
            root.join("gfx/fonts/jp_primary.fnt"),
            b"info face=\"jp_primary\" size=16\ncommon lineHeight=16 base=12 scaleW=1 scaleH=1 pages=1\npage id=0 file=\"jp_primary.tga\"\nchars count=1\nchar id=65 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=2 page=0 chnl=15\n",
        )
        .unwrap();
        fs::write(root.join("gfx/fonts/jp_primary.tga"), pixel).unwrap();
        fs::write(
            root.join("gfx/fonts/jp_fallback.fnt"),
            b"info face=\"jp_fallback\" size=16\ncommon lineHeight=16 base=12 scaleW=1 scaleH=1 pages=1\npage id=0 file=\"jp_fallback.tga\"\nchars count=2\nchar id=65 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=9 page=0 chnl=15\nchar id=66 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=3 page=0 chnl=15\n",
        )
        .unwrap();
        fs::write(root.join("gfx/fonts/jp_fallback.tga"), pixel).unwrap();
        fs::write(root.join("gfx/FX/text.shader"), b"bitmap text effect").unwrap();
        fs::create_dir_all(root.join("gfx/interface")).unwrap();
        fs::write(root.join("gfx/interface/icon.tga"), pixel).unwrap();
        fs::write(
            root.join("gfx/FX/buttonstate.shader"),
            b"buttonstate shader",
        )
        .unwrap();
        fs::write(root.join("gfx/FX/buttonstate.fxh"), b"buttonstate include").unwrap();
        fs::write(
            root.join("gfx/FX/sprite_animation.fxh"),
            b"sprite animation include",
        )
        .unwrap();
        fs::write(
            root.join("localisation/japanese/panel_l_japanese.yml"),
            b"l_japanese:\n JP_LABEL:0 \"Panel\"\n",
        )
        .unwrap();
        fs::write(
            root.join("localisation/japanese/Z.yml"),
            b"l_japanese:\n CASE_ORDER:0 \"uppercase-first\"\n",
        )
        .unwrap();
        fs::write(
            root.join("localisation/japanese/a.yml"),
            b"l_japanese:\n CASE_ORDER:0 \"lowercase-last\"\n",
        )
        .unwrap();

        let resolver = GameFileResolver::vanilla(&root).unwrap();
        let resolved = resolver
            .resolve_used_content([
                "interface/panel.gui".to_string(),
                "interface/core.gfx".to_string(),
                "gfx/fonts/base.fnt".to_string(),
                "gfx/fonts/base.tga".to_string(),
                "gfx/fonts/jp_primary.fnt".to_string(),
                "gfx/fonts/jp_primary.tga".to_string(),
                "gfx/fonts/jp_fallback.fnt".to_string(),
                "gfx/fonts/jp_fallback.tga".to_string(),
                "gfx/FX/text.shader".to_string(),
                "gfx/interface/icon.tga".to_string(),
                "gfx/FX/buttonstate.shader".to_string(),
                "gfx/FX/buttonstate.fxh".to_string(),
                "gfx/FX/sprite_animation.fxh".to_string(),
                "localisation/japanese/panel_l_japanese.yml".to_string(),
                "localisation/japanese/Z.yml".to_string(),
                "localisation/japanese/a.yml".to_string(),
            ])
            .unwrap();
        assert!(resolved.iter().all(|row| {
            row.logical_path
                == portable_content_manifest::normalize_game_relative_path(&row.logical_path)
                    .unwrap()
        }));
        let authority = "e".repeat(64);
        let required_content = resolved
            .into_iter()
            .map(|row| {
                let mut consumers = vec![DEPENDENCY_CLOSURE_CONSUMER.to_string()];
                if row.logical_path.ends_with(".gui") {
                    consumers.extend([
                        GUI_NAMESPACE_CONSUMER.to_string(),
                        portable_content_manifest::authored_ui::SCRIPTED_LOCALISATION_EMPTY_NAMESPACE_CONSUMER
                            .to_string(),
                    ]);
                } else if row.logical_path.ends_with(".gfx") {
                    consumers.push(GFX_NAMESPACE_CONSUMER.to_string());
                } else {
                    if row.logical_path.ends_with(".yml") {
                        consumers.push(LOCALISATION_NAMESPACE_CONSUMER.to_string());
                    }
                }
                if row.logical_path.ends_with(".gui")
                    || row.logical_path.ends_with(".gfx")
                    || row.logical_path.ends_with(".yml")
                {
                    let engine_spelling = if row.logical_path
                        == "localisation/japanese/z.yml"
                    {
                        "localisation/japanese/Z.yml"
                    } else {
                        &row.logical_path
                    };
                    consumers.push(
                        portable_content_manifest::authored_ui::engine_path_spelling_consumer(
                            engine_spelling,
                        )
                        .unwrap(),
                    );
                }
                consumers.sort();
                consumers.dedup();
                RequiredContent {
                    content_key: row.logical_path.clone(),
                    kind: RequiredContentKind::Other,
                    observation_count: 1,
                    consumers,
                    content: UsedContentKey {
                        effective_vfs_manifest_sha256: authority.clone(),
                        normalized_game_relative_path: row.logical_path,
                        winning_layer_ordinal: 0,
                        layer_kind: ContentLayerKind::Base,
                        layer_id: "base".into(),
                        source_sha256: row.sha256.clone(),
                        payload_sha256: row.sha256,
                        payload_bytes: row.byte_len,
                    },
                }
            })
            .collect();
        let source_game = SourceGameIdentity {
            version: "1.19.2".into(),
            build: "a729".into(),
            executable_sha256: "8".repeat(64),
            build_profile: "release".into(),
        };
        let mut manifest = ContentManifest {
            schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
            product_modset_sha256: "9".repeat(64),
            source_game: source_game.clone(),
            source_profile: SourceProfileIdentity {
                profile_id: "xui24-japanese".into(),
                playset_id: "xui24-japanese".into(),
                selection_sha256: "0".repeat(64),
            },
            effective_vfs_manifest_sha256: authority,
            ordered_layers: vec![RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "Base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            }],
            dlc_selection: Vec::new(),
            localisation_language: "l_japanese".into(),
            required_settings: BTreeMap::new(),
            required_content,
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest = manifest.with_computed_digest().unwrap();
        let inventory = resolver
            .playback_inventory(
                &manifest,
                PlaybackEnvironment {
                    source_game,
                    source_profile: manifest.source_profile.clone(),
                    dlc_selection: Vec::new(),
                    localisation_language: "l_japanese".into(),
                    settings: BTreeMap::new(),
                },
            )
            .unwrap();
        let admission = manifest.admit_playback(&inventory).unwrap();
        for consumer in admission.required_consumers().filter(|consumer| {
            consumer.starts_with(
                portable_content_manifest::authored_ui::ENGINE_PATH_SPELLING_CONSUMER_PREFIX,
            )
        }) {
            assert_eq!(
                admission
                    .require_used_content_for_consumer(consumer)
                    .unwrap()
                    .len(),
                1,
                "each authenticated engine spelling carrier must bind exactly one file"
            );
        }
        let resolver = resolver.with_content_admission(&admission).unwrap();
        let authored = AuthoredObserverUi::new(&resolver, &admission, "  L_JAPANESE  ").unwrap();
        assert_eq!(authored.localisation().get("JP_LABEL"), Some("Panel"));
        assert_eq!(
            authored
                .localisation()
                .entries
                .get("CASE_ORDER")
                .map(String::as_str),
            Some("lowercase-last")
        );
        assert_eq!(
            authored.localisation().get("CASE_ORDER"),
            None,
            "an engine-ordered diagnostic winner is not production authority for a duplicate key"
        );
        let windows = authored.discovered_window_ids().collect::<Vec<_>>();
        assert_eq!(windows.len(), 1);
        assert!(windows[0].starts_with("interface/panel.gui#"));
        let font = authored.index().surfaces[0]
            .elements
            .iter()
            .find_map(|element| element.font.as_ref())
            .expect("selected-language font binding");
        assert_eq!(font.font_id, "Alias");
        assert_eq!(font.selected_language, "l_japanese");
        assert_eq!(font.selected_variant_index, 1);
        assert_eq!(font.variants.len(), 2);
        assert_eq!(font.variants[1].ordered_descriptors.len(), 2);
        assert_eq!(
            font.base_color.as_ref().map(|color| color.argb),
            Some(0xff010203)
        );
        assert_eq!(
            font.base_color.as_ref().map(|color| color.rgba8()),
            Some([1, 2, 3, 255])
        );
        assert_eq!(
            font.base_border_color.as_ref().map(|color| color.argb),
            Some(0xff040506)
        );
        assert!(font.font_local_textcolors.is_none());
        assert_eq!(font.global_textcolor_tables.len(), 1);
        assert_eq!(
            font.variants[0].ordered_descriptors[0]
                .descriptor_content
                .normalized_game_relative_path,
            "gfx/fonts/base.fnt"
        );
        assert_eq!(
            font.variants[0].ordered_descriptors[0].ordered_pages.len(),
            1
        );
        assert_eq!(
            font.variants[0].ordered_descriptors[0].ordered_pages[0].page_id,
            0
        );
        assert_eq!(
            font.text_effect_content.normalized_game_relative_path,
            "gfx/fx/text.shader"
        );
        let sprite = authored
            .admit_dynamic_sprite(&admission, "GFX_test")
            .expect("complete authenticated dynamic sprite");
        assert_eq!(sprite.sprite_kind, "spriteType");
        assert_eq!(sprite.texture_slots[0].authored_slot_key, "texturefile1");
        assert!(!sprite.authored_color_was_explicit);
        assert_eq!(
            sprite.effect_content.normalized_game_relative_path,
            "gfx/fx/buttonstate.shader"
        );
        assert_eq!(sprite.effect_include_content.len(), 2);
        let assets = AssetResolver::from_content_admission(&resolver, &admission).unwrap();
        let loaded = game_ui::font::BitmapFont::load_from_authored_binding(&assets, font).unwrap();
        assert_eq!(loaded.source_file, "gfx/fonts/jp_primary.fnt");
        assert_eq!(loaded.chars[&65].xadvance, 2);
        assert_eq!(loaded.chars[&66].xadvance, 3);
        assert_eq!(loaded.pages.len(), 2);
        assert_eq!(loaded.authored_base_color_rgba8, Some([1, 2, 3, 255]));
        assert_eq!(
            loaded.authored_base_border_color_rgba8,
            Some([4, 5, 6, 255])
        );
        let roots = authored.strict_root_census(&admission).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].authored_root().name, "mod_panel");
        assert_eq!(
            roots[0]
                .element_role(&[])
                .map(|role| role.authored_name.as_str()),
            Some("mod_panel")
        );
        assert_eq!(
            roots[0]
                .authored_element(&[0])
                .map(|element| element.name.as_str()),
            Some("icon")
        );
        assert_eq!(
            roots[0]
                .authored_element(&[1])
                .map(|element| element.name.as_str()),
            Some("label")
        );
        assert_eq!(
            roots[0].font_binding("Alias").unwrap().map(|binding| {
                binding.variants[0].ordered_descriptors[0]
                    .descriptor_content
                    .normalized_game_relative_path
                    .as_str()
            }),
            Some("gfx/fonts/base.fnt")
        );
        let frontend =
            admit_discovered_frontend_chrome(&authored.index, &admission, &roots[0]).unwrap();
        let unbound_assets = AssetResolver::new(vec![root.clone()]);
        let mismatch = authored
            .index
            .seal_panel_surface(
                &unbound_assets,
                &authored.localisation_language,
                &authored.localisation,
                &admission,
                roots[0].clone(),
                frontend.clone(),
            )
            .unwrap_err();
        assert!(mismatch.contains("different replay"), "{mismatch}");
        let mismatch = authored
            .index
            .seal_panel_surface(
                &authored.assets,
                "l_english",
                &authored.localisation,
                &admission,
                roots[0].clone(),
                frontend,
            )
            .unwrap_err();
        assert!(mismatch.contains("different language"), "{mismatch}");
        let sealed = authored
            .admit_surface(&admission, roots[0].source_content(), roots[0].surface_id())
            .unwrap();
        assert_eq!(sealed.window_id(), roots[0].window_id());
        assert_eq!(sealed.surface_id(), roots[0].surface_id());
        assert_eq!(sealed.source_content(), roots[0].source_content());
        assert_eq!(
            sealed.manifest_digest_sha256(),
            admission.manifest_digest_sha256()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
    )]
    fn product_resolver_adapter_uses_effective_namespace_without_frozen_counts() {
        let root = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(root).expect("retained game resolver");
        let catalog = load_product_gui_definitions(Some(&resolver))
            .expect("product GUI definition load")
            .expect("game resolver yields a catalog");
        assert!(catalog.admitted_file_count() > 0);
        assert_eq!(catalog.rejected_file_count(), 0);
        assert!(!catalog.admitted_windows.is_empty());
        assert_eq!(catalog.rejected_windows.len(), 0);
    }
}
