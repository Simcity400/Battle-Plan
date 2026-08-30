use crate::assets::{slice_horizontal_frames, AssetResolver, DecodedSprite, DecodedTexture};
use crate::dds::RgbaImage;
use crate::engine_text_layout::{
    engine_inline_icon_frame_index, layout_engine_text_with_icons, EngineTextBorder,
    EngineTextLayout, ResolvedInlineIconMetrics,
};
use crate::font::{
    alpha_blend, blank_image, BitmapFont, RuntimeGlyphDraw, TextAlign, TextShaderRejection,
};
use crate::gfx::{Size, SpriteDefinition, SpriteRegistry};
use crate::gui::{GuiElement, Measure, MeasurePair};
use crate::loc::{is_localisation_key, Localisation};
use crate::paradox_text::{scalar_f32, scalar_i32, scalar_u32, Value};
use crate::runtime_graphics::{
    admit_authenticated_dynamic_sprite_runtime_graphics, admit_authenticated_progress_draw,
    admit_authenticated_semantic_runtime_graphics, admit_authenticated_widget_runtime_graphics,
    blend_sprite_animation, draw_authenticated_progress_pixels, draw_authenticated_sprite_pixels,
    shade_buttonstate, AdmittedRuntimeGraphics, AuthenticatedSpriteGeometry, ButtonVisualState,
    ClipRect, RgbaF32, RuntimeGraphicsRejection, SpriteAnimationBlendMode, TEXT_SHADER_BYTES,
    TEXT_SHADER_SHA256,
};
use crate::text_law::{
    resolve_element_bound_exact_runtime, resolve_localisation_preserving_inline_icons,
    resolve_template_preserving_inline_icons, RuntimeTextContext, TextIssue,
};
use crate::widget_visual_state::{
    admit_authenticated_widget_graphics_state, admit_widget_identity,
    AuthenticatedViewerWidgetState, ProgressFill, WidgetFamily,
};
use crate::window_manager_law::{
    AuthenticatedReplayFrameAuthority, AuthenticatedReplayPanelLease, ReplayOwnedWindowManager,
};
use crate::{GameUiError, Result};
use replay_core::{RuntimeTextLayout, RuntimeTextSource, ValidatedContentAdmission};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct ComposeOptions {
    pub viewport_width: usize,
    pub viewport_height: usize,
    pub root_width: usize,
    pub root_height: usize,
    pub text_color: [u8; 4],
    pub text_overrides: BTreeMap<String, String>,
    pub localisation: BTreeMap<String, String>,
    pub localisation_source_by_key: BTreeMap<String, String>,
    pub localisation_ambiguous_sources_by_key: BTreeMap<String, Vec<String>>,
    pub require_localisation_provenance: bool,
    pub position_overrides: BTreeMap<String, [i32; 2]>,
    pub position_overrides_by_child_path: BTreeMap<Vec<u32>, [i32; 2]>,
    pub position_x_overrides: BTreeMap<String, i32>,
    pub position_y_overrides: BTreeMap<String, i32>,
    pub size_overrides: BTreeMap<String, [i32; 2]>,
    pub sprite_name_overrides: BTreeMap<String, String>,
    pub sprite_name_overrides_by_child_path: BTreeMap<Vec<u32>, String>,
    pub sprite_overrides: BTreeMap<String, RgbaImage>,
    pub color_overrides: BTreeMap<String, [u8; 4]>,
    pub frame_overrides: BTreeMap<String, usize>,
    pub progress_overrides: BTreeMap<String, f32>,
    pub pie_chart_overrides: BTreeMap<String, Vec<PieChartSlice>>,
    pub animation_elapsed_micros: u64,
    pub disabled_button_elements: BTreeSet<String>,
    pub xui23_buttonstate_elements: BTreeSet<String>,
    pub xui23_progress_elements: BTreeSet<String>,
    pub xui23_require_scoped_progress: bool,
    pub hidden_elements: BTreeSet<String>,
    pub hidden_element_paths: BTreeSet<Vec<u32>>,
    pub shown_elements: BTreeSet<String>,
    pub runtime_text: RuntimeTextContext,
    pub runtime_text_by_element: BTreeMap<String, RuntimeTextContext>,
    pub runtime_text_authored_element_by_clone: BTreeMap<String, String>,
    pub runtime_view_id: Option<String>,
}

impl ComposeOptions {
    pub fn new(viewport_width: usize, viewport_height: usize) -> Self {
        Self {
            viewport_width,
            viewport_height,
            root_width: viewport_width,
            root_height: viewport_height,
            text_color: [221, 214, 190, 255],
            text_overrides: BTreeMap::new(),
            localisation: BTreeMap::new(),
            localisation_source_by_key: BTreeMap::new(),
            localisation_ambiguous_sources_by_key: BTreeMap::new(),
            require_localisation_provenance: false,
            position_overrides: BTreeMap::new(),
            position_overrides_by_child_path: BTreeMap::new(),
            position_x_overrides: BTreeMap::new(),
            position_y_overrides: BTreeMap::new(),
            size_overrides: BTreeMap::new(),
            sprite_name_overrides: BTreeMap::new(),
            sprite_name_overrides_by_child_path: BTreeMap::new(),
            sprite_overrides: BTreeMap::new(),
            color_overrides: BTreeMap::new(),
            frame_overrides: BTreeMap::new(),
            progress_overrides: BTreeMap::new(),
            pie_chart_overrides: BTreeMap::new(),
            animation_elapsed_micros: 0,
            disabled_button_elements: BTreeSet::new(),
            xui23_buttonstate_elements: BTreeSet::new(),
            xui23_progress_elements: BTreeSet::new(),
            xui23_require_scoped_progress: false,
            hidden_elements: BTreeSet::new(),
            hidden_element_paths: BTreeSet::new(),
            shown_elements: BTreeSet::new(),
            runtime_text: RuntimeTextContext::default(),
            runtime_text_by_element: BTreeMap::new(),
            runtime_text_authored_element_by_clone: BTreeMap::new(),
            runtime_view_id: None,
        }
    }

    pub fn with_root_size(mut self, width: usize, height: usize) -> Self {
        self.root_width = width.max(1);
        self.root_height = height.max(1);
        self
    }

    pub fn with_localisation(mut self, localisation: &Localisation) -> Self {
        self.localisation = localisation.entries.clone();
        self.localisation_source_by_key = localisation.source_by_key.clone();
        self.localisation_ambiguous_sources_by_key = localisation.ambiguous_sources_by_key.clone();
        self.require_localisation_provenance = localisation.provenance_complete;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposeReport {
    pub drawn_elements: usize,
    pub missing_sprites: BTreeSet<String>,
    pub undecodable_sprites: Vec<String>,
    pub missing_fonts: BTreeSet<String>,
    pub missing_glyphs: BTreeMap<String, BTreeSet<char>>,
    pub unresolved_loc_keys: BTreeSet<String>,
    pub localisation_sources_used: BTreeMap<String, String>,
    pub unresolved_text_inputs: BTreeMap<String, Vec<TextIssue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComposeOutput {
    pub image: RgbaImage,
    pub report: ComposeReport,
    pub debug_rects: Vec<DebugRect>,
}

/// for the manager's current panel lifetime and reconstructs the one allowed
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedPanelComposeRequest {
    window_id: String,
    runtime_instance_token: String,
    frame_generation: u64,
    viewport_width: usize,
    viewport_height: usize,
}

impl AuthenticatedPanelComposeRequest {
    pub(crate) fn seal(
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        panel_lease: &AuthenticatedReplayPanelLease<'_>,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Self, AuthenticatedComposeRejection> {
        if viewport_width == 0
            || viewport_height == 0
            || !frame_authority.remains_staged_for(manager)
            || !panel_lease.remains_current()
            || panel_lease.window_id() != panel.window_id()
            || panel_lease.runtime_instance_token().trim().is_empty()
        {
            return Err(AuthenticatedComposeRejection::PanelAuthentication);
        }
        Ok(Self {
            window_id: panel.window_id().to_string(),
            runtime_instance_token: panel_lease.runtime_instance_token().to_string(),
            frame_generation: frame_authority.frame_generation(),
            viewport_width,
            viewport_height,
        })
    }

    fn options(&self) -> ComposeOptions {
        ComposeOptions::new(self.viewport_width, self.viewport_height)
    }

    fn remains_current(
        &self,
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        panel_lease: &AuthenticatedReplayPanelLease<'_>,
    ) -> bool {
        frame_authority.remains_staged_for(manager)
            && self.frame_generation == frame_authority.frame_generation()
            && self.window_id == panel.window_id()
            && self.runtime_instance_token == panel_lease.runtime_instance_token()
            && panel_lease.remains_current()
            && panel_lease.window_id() == panel.window_id()
            && self.viewport_width != 0
            && self.viewport_height != 0
    }
}

const PERSISTENT_HUD_ROOTS: [(&str, &str); 4] = [
    ("interface/topbar.gui", "topbar"),
    ("interface/minimap.gui", "mini_map_interface"),
    (
        "interface/mapmodes_interface.gui",
        "MapmodesInterface_Ingame",
    ),
    ("interface/theatreselector.gui", "theatre_selection_window"),
];
pub(crate) struct AuthenticatedPersistentHudPanels {
    panels: [crate::authored_role_index::StrictAuthoredPanelAdmission; 4],
}

impl AuthenticatedPersistentHudPanels {
    pub(crate) fn panels(&self) -> &[crate::authored_role_index::StrictAuthoredPanelAdmission; 4] {
        &self.panels
    }
}

pub(crate) fn authenticate_persistent_hud_surfaces(
    surfaces: &[crate::authored_role_index::AuthenticatedPanelSurface],
) -> std::result::Result<AuthenticatedPersistentHudPanels, AuthenticatedComposeRejection> {
    if surfaces.len() != PERSISTENT_HUD_ROOTS.len() {
        return Err(AuthenticatedComposeRejection::PanelAuthentication);
    }
    let mut panels = Vec::with_capacity(PERSISTENT_HUD_ROOTS.len());
    for (surface, (source_path, surface_id)) in surfaces.iter().zip(PERSISTENT_HUD_ROOTS) {
        if !surface
            .source_content()
            .normalized_game_relative_path
            .eq_ignore_ascii_case(source_path)
            || !surface.surface_id().eq_ignore_ascii_case(surface_id)
        {
            return Err(AuthenticatedComposeRejection::PanelAuthentication);
        }
        panels.push(surface.panel.clone());
    }
    let panels = panels
        .try_into()
        .map_err(|_| AuthenticatedComposeRejection::PanelAuthentication)?;
    Ok(AuthenticatedPersistentHudPanels { panels })
}

pub(crate) fn authenticate_persistent_hud_panels(
    index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
    catalog: &crate::definition_admission::GuiDefinitionCatalog,
    admission: &ValidatedContentAdmission,
) -> std::result::Result<AuthenticatedPersistentHudPanels, AuthenticatedComposeRejection> {
    let mut admitted = Vec::with_capacity(PERSISTENT_HUD_ROOTS.len());
    for (source_path, surface_id) in PERSISTENT_HUD_ROOTS {
        let source_content = admission
            .used_content(source_path)
            .ok_or(AuthenticatedComposeRejection::PanelAuthentication)?;
        let panel = index
            .admit_surface(catalog, admission, source_content, surface_id)
            .map_err(|_| AuthenticatedComposeRejection::PanelAuthentication)?;
        let rebound = index
            .validate_panel_token(admission, &panel)
            .map_err(|_| AuthenticatedComposeRejection::PanelAuthentication)?;
        if !rebound.source_file.eq_ignore_ascii_case(source_path)
            || !rebound.surface_id.eq_ignore_ascii_case(surface_id)
        {
            return Err(AuthenticatedComposeRejection::PanelAuthentication);
        }
        admitted.push(panel);
    }
    let panels: [_; 4] = admitted
        .try_into()
        .map_err(|_| AuthenticatedComposeRejection::PanelAuthentication)?;

    Ok(AuthenticatedPersistentHudPanels { panels })
}

/// different lifetime, preflight, or canonical state census. XUI02 constructs
pub(crate) struct AuthenticatedPanelBatchInput<'a> {
    request: &'a AuthenticatedPanelComposeRequest,
    surface: &'a crate::authored_role_index::AuthenticatedPanelSurface,
    panel_lease: &'a AuthenticatedReplayPanelLease<'a>,
    preflight: &'a AuthenticatedLayoutPreflight,
    widget_states: &'a BTreeMap<Vec<u32>, AuthenticatedViewerWidgetState>,
    semantics: &'a AuthenticatedPanelSemanticCensus,
    materialized_instances: Option<&'a AuthenticatedMaterializationPlan>,
}

impl<'a> AuthenticatedPanelBatchInput<'a> {
    pub(crate) fn new(
        request: &'a AuthenticatedPanelComposeRequest,
        surface: &'a crate::authored_role_index::AuthenticatedPanelSurface,
        panel_lease: &'a AuthenticatedReplayPanelLease<'a>,
        preflight: &'a AuthenticatedLayoutPreflight,
        widget_states: &'a BTreeMap<Vec<u32>, AuthenticatedViewerWidgetState>,
        semantics: &'a AuthenticatedPanelSemanticCensus,
        materialized_instances: Option<&'a AuthenticatedMaterializationPlan>,
    ) -> Self {
        Self {
            request,
            surface,
            panel_lease,
            preflight,
            widget_states,
            semantics,
            materialized_instances,
        }
    }
}

/// producer for one authenticated panel lifetime.
/// lifetime before XUI07 accepts it. Static authored paint is still explicit:
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AuthenticatedElementSemantic {
    Hidden,
    Structural,
    StandardButton,
    ReadOnlyButton {
        one_based_frame: u32,
        interaction: crate::runtime_graphics::ButtonVisualState,
    },
    PassiveSprite {
        one_based_frame: u32,
    },
    Text {
        resolved_text: String,
    },
    Progress {
        fill: ProgressFill,
    },
    BlendFrames {
        current_one_based_frame: u32,
        next_one_based_frame: u32,
        animation_time: f32,
    },
}

/// path map against another modset, authored root, or open lifetime.
#[derive(Debug, PartialEq)]
pub(crate) struct AuthenticatedPanelSemanticCensus {
    index_identity_sha256: String,
    window_id: String,
    surface_id: String,
    source_content: replay_core::UsedContentKey,
    runtime_instance_token: String,
    frame_generation: u64,
    elements: BTreeMap<Vec<u32>, AuthenticatedElementSemantic>,
}

impl AuthenticatedPanelSemanticCensus {
    pub(crate) fn elements(&self) -> &BTreeMap<Vec<u32>, AuthenticatedElementSemantic> {
        &self.elements
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AuthenticatedRenderedInstancePath {
    instance_ordinal: u32,
    template_child_path: Vec<u32>,
}

impl AuthenticatedRenderedInstancePath {
    pub(crate) fn instance_ordinal(&self) -> u32 {
        self.instance_ordinal
    }

    pub(crate) fn template_child_path(&self) -> &[u32] {
        &self.template_child_path
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct AuthenticatedMaterializedInstancePlan {
    instance_key: String,
    instance_ordinal: u32,
    template_panel: crate::authored_role_index::StrictAuthoredPanelAdmission,
    global_root_sibling: bool,
    host_instance_ordinal: Option<u32>,
    host_child_path: Vec<u32>,
    owner_child_insertion_ordinal: u32,
    parent_rect: AuthenticatedElementRect,
    required_clip: AuthenticatedElementRect,
    semantics: BTreeMap<Vec<u32>, AuthenticatedElementSemantic>,
    viewer_actions: BTreeMap<Vec<u32>, crate::window_manager_law::ReplayViewerAction>,
}

impl AuthenticatedMaterializedInstancePlan {
    pub(crate) fn new(
        instance_key: String,
        instance_ordinal: u32,
        template_panel: crate::authored_role_index::StrictAuthoredPanelAdmission,
        global_root_sibling: bool,
        host_instance_ordinal: Option<u32>,
        host_child_path: Vec<u32>,
        owner_child_insertion_ordinal: u32,
        parent_rect: AuthenticatedElementRect,
        required_clip: AuthenticatedElementRect,
        semantics: BTreeMap<Vec<u32>, AuthenticatedElementSemantic>,
    ) -> Self {
        Self {
            instance_key,
            instance_ordinal,
            template_panel,
            global_root_sibling,
            host_instance_ordinal,
            host_child_path,
            owner_child_insertion_ordinal,
            parent_rect,
            required_clip,
            semantics,
            viewer_actions: BTreeMap::new(),
        }
    }

    pub(crate) fn bind_viewer_action(
        &mut self,
        template_child_path: Vec<u32>,
        action: crate::window_manager_law::ReplayViewerAction,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        if self
            .viewer_actions
            .insert(template_child_path, action)
            .is_some()
        {
            return Err(AuthenticatedComposeRejection::MaterializationIdentity);
        }
        Ok(())
    }

    pub(crate) fn instance_ordinal(&self) -> u32 {
        self.instance_ordinal
    }

    pub(crate) fn instance_key(&self) -> &str {
        &self.instance_key
    }

    pub(crate) fn global_root_sibling(&self) -> bool {
        self.global_root_sibling
    }

    pub(crate) fn template_panel(
        &self,
    ) -> &crate::authored_role_index::StrictAuthoredPanelAdmission {
        &self.template_panel
    }

    pub(crate) fn viewer_actions(
        &self,
    ) -> &BTreeMap<Vec<u32>, crate::window_manager_law::ReplayViewerAction> {
        &self.viewer_actions
    }

    pub(crate) fn semantic(
        &self,
        template_child_path: &[u32],
    ) -> Option<&AuthenticatedElementSemantic> {
        self.semantics.get(template_child_path)
    }

    pub(crate) fn set_read_only_button_interaction(
        &mut self,
        template_child_path: &[u32],
        next_interaction: crate::runtime_graphics::ButtonVisualState,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        let Some(AuthenticatedElementSemantic::ReadOnlyButton { interaction, .. }) =
            self.semantics.get_mut(template_child_path)
        else {
            return Err(AuthenticatedComposeRejection::MaterializationIdentity);
        };
        *interaction = next_interaction;
        Ok(())
    }
}

/// Authenticated dynamic-instance plan for one owner panel lifetime.
#[derive(Debug, PartialEq)]
pub(crate) struct AuthenticatedMaterializationPlan {
    index_identity_sha256: String,
    owner_window_id: String,
    owner_runtime_instance_token: String,
    frame_generation: u64,
    viewport_rect: AuthenticatedElementRect,
    content_rect: AuthenticatedElementRect,
    scroll_offset_pixels: i32,
    scroll_extent_identity_sha256: String,
    materialization_digest_sha256: String,
    instances: Vec<AuthenticatedMaterializedInstancePlan>,
    extended_scrollbar: Option<AuthenticatedExtendedScrollbarPlan>,
    collection_scroll_regions: Vec<AuthenticatedCollectionScrollRegionPlan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedCollectionScrollRegionPlan {
    pub(crate) element_id: String,
    pub(crate) child_path: Vec<u32>,
    pub(crate) viewport_rect: AuthenticatedElementRect,
    pub(crate) max_offset_pixels: i32,
    pub(crate) offset_pixels: i32,
    pub(crate) wheel_step_pixels: i32,
    pub(crate) smooth_scrolling: bool,
    pub(crate) geometry_source_record: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedScrollbarPartPlan {
    pub(crate) sprite_name: String,
    pub(crate) rect: AuthenticatedElementRect,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AuthenticatedExtendedScrollbarPlan {
    pub(crate) style_root: GuiElement,
    pub(crate) source_content: replay_core::UsedContentKey,
    pub(crate) clip_rect: AuthenticatedElementRect,
    pub(crate) background: AuthenticatedScrollbarPartPlan,
    pub(crate) track: AuthenticatedScrollbarPartPlan,
    pub(crate) decrease: AuthenticatedScrollbarPartPlan,
    pub(crate) increase: AuthenticatedScrollbarPartPlan,
    pub(crate) slider: AuthenticatedScrollbarPartPlan,
    pub(crate) thumb_base_pixels: i32,
    pub(crate) thumb_travel_pixels: i32,
    pub(crate) thumb_authored_offset_pixels: i32,
}

impl AuthenticatedMaterializationPlan {
    pub(crate) fn instances(&self) -> &[AuthenticatedMaterializedInstancePlan] {
        &self.instances
    }

    pub(crate) fn viewport_rect(&self) -> AuthenticatedElementRect {
        self.viewport_rect
    }

    pub(crate) fn content_rect(&self) -> AuthenticatedElementRect {
        self.content_rect
    }

    pub(crate) fn scroll_offset_pixels(&self) -> i32 {
        self.scroll_offset_pixels
    }

    pub(crate) fn scroll_extent_identity_sha256(&self) -> &str {
        &self.scroll_extent_identity_sha256
    }

    pub(crate) fn materialization_digest_sha256(&self) -> &str {
        &self.materialization_digest_sha256
    }

    pub(crate) fn extended_scrollbar(&self) -> Option<&AuthenticatedExtendedScrollbarPlan> {
        self.extended_scrollbar.as_ref()
    }

    pub(crate) fn collection_scroll_regions(&self) -> &[AuthenticatedCollectionScrollRegionPlan] {
        &self.collection_scroll_regions
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedMaterializedInstanceElement {
    rendered_path: AuthenticatedRenderedInstancePath,
    element: AuthenticatedComposedElement,
}

impl AuthenticatedMaterializedInstanceElement {
    pub(crate) fn rendered_path(&self) -> &AuthenticatedRenderedInstancePath {
        &self.rendered_path
    }

    pub(crate) fn element(&self) -> &AuthenticatedComposedElement {
        &self.element
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedMaterializedInstanceCensus {
    instance_key: String,
    template_window_id: String,
    template_surface_id: String,
    template_source_content: replay_core::UsedContentKey,
    template_tree_identity_sha256: String,
    rendered_root_path: AuthenticatedRenderedInstancePath,
    elements: Vec<AuthenticatedMaterializedInstanceElement>,
}

impl AuthenticatedMaterializedInstanceCensus {
    pub(crate) fn instance_key(&self) -> &str {
        &self.instance_key
    }

    pub(crate) fn template_tree_identity_sha256(&self) -> &str {
        &self.template_tree_identity_sha256
    }

    pub(crate) fn rendered_root_path(&self) -> &AuthenticatedRenderedInstancePath {
        &self.rendered_root_path
    }

    pub(crate) fn elements(&self) -> &[AuthenticatedMaterializedInstanceElement] {
        &self.elements
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn seal_authenticated_materialization_plan(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    owner_panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
    owner_lease: &AuthenticatedReplayPanelLease<'_>,
    viewport_rect: AuthenticatedElementRect,
    content_rect: AuthenticatedElementRect,
    scroll_offset_pixels: i32,
    scroll_extent_identity_sha256: String,
    instances: Vec<AuthenticatedMaterializedInstancePlan>,
    extended_scrollbar: Option<AuthenticatedExtendedScrollbarPlan>,
    collection_scroll_regions: Vec<AuthenticatedCollectionScrollRegionPlan>,
) -> std::result::Result<AuthenticatedMaterializationPlan, AuthenticatedComposeRejection> {
    if !frame_authority.remains_staged_for(manager)
        || !owner_lease.remains_current()
        || owner_lease.window_id() != owner_panel.window_id()
        || owner_lease.runtime_instance_token().trim().is_empty()
        || index.validate_panel_token(admission, owner_panel).is_err()
        || viewport_rect.x != 0
        || viewport_rect.y != 0
        || !authenticated_rect_is_valid(viewport_rect)
        || !authenticated_rect_is_valid(content_rect)
        || !authenticated_rects_intersect(viewport_rect, content_rect)
        || scroll_extent_identity_sha256.len() != 64
        || !scroll_extent_identity_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AuthenticatedComposeRejection::MaterializationIdentity);
    }
    validate_extended_scrollbar_plan(index, admission, extended_scrollbar.as_ref())?;
    let mut scroll_ids = BTreeSet::new();
    let mut scroll_paths = BTreeSet::new();
    for scroll in &collection_scroll_regions {
        let role = owner_panel
            .element_role(&scroll.child_path)
            .ok_or(AuthenticatedComposeRejection::MaterializationIdentity)?;
        if role.element_id != scroll.element_id
            || !scroll_ids.insert(scroll.element_id.as_str())
            || !scroll_paths.insert(scroll.child_path.as_slice())
            || !authenticated_rect_is_valid(scroll.viewport_rect)
            || !authenticated_rect_contains(viewport_rect, scroll.viewport_rect)
            || scroll.max_offset_pixels < 0
            || !(0..=scroll.max_offset_pixels).contains(&scroll.offset_pixels)
            || scroll.wheel_step_pixels <= 0
            || scroll.geometry_source_record.trim().is_empty()
        {
            return Err(AuthenticatedComposeRejection::MaterializationIdentity);
        }
    }

    let mut instance_keys = BTreeSet::new();
    for (expected_ordinal, instance) in instances.iter().enumerate() {
        let expected_ordinal = u32::try_from(expected_ordinal)
            .map_err(|_| AuthenticatedComposeRejection::MaterializationIdentity)?;
        if instance.instance_key.trim().is_empty() {
            return Err(AuthenticatedComposeRejection::MaterializationIdentity);
        }
        if !instance_keys.insert(instance.instance_key.as_str()) {
            return Err(
                AuthenticatedComposeRejection::DuplicateMaterializedInstance(
                    instance.instance_key.clone(),
                ),
            );
        }
        let host = if instance.global_root_sibling {
            if instance.host_instance_ordinal.is_some()
                || !instance.host_child_path.is_empty()
                || instance.owner_child_insertion_ordinal != 0
            {
                return Err(AuthenticatedComposeRejection::MaterializationIdentity);
            }
            None
        } else {
            let host_panel = match instance.host_instance_ordinal {
                None => owner_panel,
                Some(parent_ordinal) if parent_ordinal < instance.instance_ordinal => instances
                    .get(
                        usize::try_from(parent_ordinal)
                            .map_err(|_| AuthenticatedComposeRejection::MaterializationIdentity)?,
                    )
                    .filter(|parent| parent.instance_ordinal == parent_ordinal)
                    .map(|parent| &parent.template_panel)
                    .ok_or(AuthenticatedComposeRejection::MaterializationIdentity)?,
                Some(_) => return Err(AuthenticatedComposeRejection::MaterializationIdentity),
            };
            Some(
                host_panel
                    .authored_element(&instance.host_child_path)
                    .ok_or(AuthenticatedComposeRejection::MaterializationIdentity)?,
            )
        };
        if instance.instance_ordinal != expected_ordinal
            || host.is_some_and(|host| {
                usize::try_from(instance.owner_child_insertion_ordinal)
                    .ok()
                    .map_or(true, |ordinal| ordinal > host.children.len())
            })
            || !authenticated_rect_is_valid(instance.parent_rect)
            || !authenticated_rect_is_valid(instance.required_clip)
            || !authenticated_rect_contains(viewport_rect, instance.required_clip)
        {
            return Err(AuthenticatedComposeRejection::MaterializationGeometry);
        }
        validate_materialized_template_semantics(
            index,
            admission,
            &instance.template_panel,
            &instance.semantics,
        )?;
        for path in instance.viewer_actions.keys() {
            let role = instance
                .template_panel
                .element_role(path)
                .ok_or(AuthenticatedComposeRejection::MaterializationIdentity)?;
            let authored = instance
                .template_panel
                .authored_element(path)
                .ok_or(AuthenticatedComposeRejection::MaterializationIdentity)?;
            if !WidgetFamily::from_element_type(&authored.element_type)
                .is_some_and(WidgetFamily::standard_button_carrier)
                || !matches!(
                    instance.semantics.get(path),
                    Some(AuthenticatedElementSemantic::ReadOnlyButton {
                        interaction: crate::runtime_graphics::ButtonVisualState::Up
                            | crate::runtime_graphics::ButtonVisualState::Down
                            | crate::runtime_graphics::ButtonVisualState::Over,
                        ..
                    })
                )
                || role.sprites.len() != 1
            {
                return Err(AuthenticatedComposeRejection::MaterializationIdentity);
            }
        }
    }

    let materialization_digest_sha256 = materialization_plan_digest(
        index,
        owner_panel,
        owner_lease.runtime_instance_token(),
        frame_authority.frame_generation(),
        viewport_rect,
        content_rect,
        scroll_offset_pixels,
        &scroll_extent_identity_sha256,
        &instances,
        extended_scrollbar.as_ref(),
        &collection_scroll_regions,
    );
    Ok(AuthenticatedMaterializationPlan {
        index_identity_sha256: index.index_identity_sha256.clone(),
        owner_window_id: owner_panel.window_id().to_string(),
        owner_runtime_instance_token: owner_lease.runtime_instance_token().to_string(),
        frame_generation: frame_authority.frame_generation(),
        viewport_rect,
        content_rect,
        scroll_offset_pixels,
        scroll_extent_identity_sha256,
        materialization_digest_sha256,
        instances,
        extended_scrollbar,
        collection_scroll_regions,
    })
}

fn validate_extended_scrollbar_plan(
    index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    plan: Option<&AuthenticatedExtendedScrollbarPlan>,
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    let Some(plan) = plan else {
        return Ok(());
    };
    if !plan
        .style_root
        .element_type
        .eq_ignore_ascii_case("extendedScrollbarType")
        || crate::authored_role_index::authored_tree_identity_sha256(&plan.style_root).len() != 64
        || admission
            .require_used_content(&plan.source_content.normalized_game_relative_path)
            .ok()
            != Some(&plan.source_content)
        || !authenticated_rect_is_valid(plan.clip_rect)
        || plan.thumb_base_pixels < 0
        || plan.thumb_travel_pixels < 0
    {
        return Err(AuthenticatedComposeRejection::MaterializationIdentity);
    }
    for part in [
        &plan.background,
        &plan.track,
        &plan.decrease,
        &plan.increase,
        &plan.slider,
    ] {
        if part.sprite_name.trim().is_empty()
            || !authenticated_rect_is_valid(part.rect)
            || index
                .admit_dynamic_sprite(admission, &part.sprite_name)
                .is_err()
        {
            return Err(AuthenticatedComposeRejection::MaterializationIdentity);
        }
    }
    Ok(())
}

fn validate_materialized_template_semantics(
    index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    template_panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
    semantics: &BTreeMap<Vec<u32>, AuthenticatedElementSemantic>,
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    let surface = index
        .validate_panel_token(admission, template_panel)
        .map_err(|_| AuthenticatedComposeRejection::MaterializationIdentity)?;
    validate_authenticated_tree(template_panel.authored_root(), surface)?;
    if semantics.len() != surface.elements.len()
        || semantics
            .keys()
            .any(|path| !surface.elements.iter().any(|role| role.child_path == *path))
    {
        return Err(AuthenticatedComposeRejection::MaterializationIdentity);
    }
    for role in &surface.elements {
        let semantic = semantics.get(&role.child_path).ok_or_else(|| {
            AuthenticatedComposeRejection::MissingSemanticState(role.child_path.clone())
        })?;
        let authored = template_panel
            .authored_element(&role.child_path)
            .ok_or(AuthenticatedComposeRejection::MaterializationIdentity)?;
        validate_element_semantic(role, authored, semantic)?;
        if matches!(semantic, AuthenticatedElementSemantic::Hidden)
            && surface.elements.iter().any(|descendant| {
                descendant.child_path.len() > role.child_path.len()
                    && descendant.child_path.starts_with(&role.child_path)
                    && !matches!(
                        semantics.get(&descendant.child_path),
                        Some(AuthenticatedElementSemantic::Hidden)
                    )
            })
        {
            return Err(AuthenticatedComposeRejection::IncompatibleSemanticState(
                role.child_path.clone(),
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_materialization_plan(
    plan: &AuthenticatedMaterializationPlan,
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    owner_panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
    owner_lease: &AuthenticatedReplayPanelLease<'_>,
    viewport_width: usize,
    viewport_height: usize,
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    let expected_viewport = AuthenticatedElementRect::new(
        0,
        0,
        i32::try_from(viewport_width)
            .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?,
        i32::try_from(viewport_height)
            .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?,
    );
    if !frame_authority.remains_staged_for(manager)
        || !owner_lease.remains_current()
        || plan.index_identity_sha256 != index.index_identity_sha256
        || plan.owner_window_id != owner_panel.window_id()
        || plan.owner_runtime_instance_token != owner_lease.runtime_instance_token()
        || plan.frame_generation != frame_authority.frame_generation()
        || plan.viewport_rect != expected_viewport
    {
        return Err(AuthenticatedComposeRejection::MaterializationIdentity);
    }
    for (expected_ordinal, instance) in plan.instances.iter().enumerate() {
        if usize::try_from(instance.instance_ordinal).ok() != Some(expected_ordinal) {
            return Err(AuthenticatedComposeRejection::MaterializationIdentity);
        }
        validate_materialized_template_semantics(
            index,
            admission,
            &instance.template_panel,
            &instance.semantics,
        )?;
    }
    let expected_digest = materialization_plan_digest(
        index,
        owner_panel,
        owner_lease.runtime_instance_token(),
        frame_authority.frame_generation(),
        plan.viewport_rect,
        plan.content_rect,
        plan.scroll_offset_pixels,
        &plan.scroll_extent_identity_sha256,
        &plan.instances,
        plan.extended_scrollbar.as_ref(),
        &plan.collection_scroll_regions,
    );
    if plan.materialization_digest_sha256 != expected_digest {
        return Err(AuthenticatedComposeRejection::MaterializationIdentity);
    }
    Ok(())
}

fn validate_materialized_result_order(
    plan: &AuthenticatedMaterializationPlan,
    census: &[AuthenticatedMaterializedInstanceCensus],
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    if plan.instances.len() != census.len() {
        return Err(AuthenticatedComposeRejection::MaterializationIdentity);
    }
    for (planned, rendered) in plan.instances.iter().zip(census) {
        if rendered.instance_key != planned.instance_key
            || rendered.rendered_root_path.instance_ordinal != planned.instance_ordinal
            || !rendered.rendered_root_path.template_child_path.is_empty()
            || rendered.template_window_id != planned.template_panel.window_id()
            || rendered.template_surface_id != planned.template_panel.surface_id()
            || rendered.template_source_content != *planned.template_panel.source_content()
            || rendered.template_tree_identity_sha256
                != planned.template_panel.authored_tree_identity_sha256()
            || rendered.elements.len() != planned.template_panel.element_roles().len()
        {
            return Err(AuthenticatedComposeRejection::MaterializationIdentity);
        }
        let mut paths = BTreeSet::new();
        for element in &rendered.elements {
            if element.rendered_path.instance_ordinal != planned.instance_ordinal
                || element.rendered_path.template_child_path != element.element.child_path
                || !paths.insert(element.rendered_path.template_child_path.as_slice())
                || planned
                    .template_panel
                    .element_role(&element.rendered_path.template_child_path)
                    .is_none()
            {
                return Err(AuthenticatedComposeRejection::MaterializationIdentity);
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn materialization_plan_digest(
    index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
    owner_panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
    owner_runtime_instance_token: &str,
    frame_generation: u64,
    viewport_rect: AuthenticatedElementRect,
    content_rect: AuthenticatedElementRect,
    scroll_offset_pixels: i32,
    scroll_extent_identity_sha256: &str,
    instances: &[AuthenticatedMaterializedInstancePlan],
    extended_scrollbar: Option<&AuthenticatedExtendedScrollbarPlan>,
    collection_scroll_regions: &[AuthenticatedCollectionScrollRegionPlan],
) -> String {
    let mut bytes = Vec::new();
    digest_field(&mut bytes, b"authenticated-materialization-plan-v3");
    digest_field(&mut bytes, index.index_identity_sha256.as_bytes());
    digest_field(&mut bytes, owner_panel.window_id().as_bytes());
    digest_field(&mut bytes, owner_runtime_instance_token.as_bytes());
    bytes.extend_from_slice(&frame_generation.to_le_bytes());
    digest_rect(&mut bytes, viewport_rect);
    digest_rect(&mut bytes, content_rect);
    bytes.extend_from_slice(&scroll_offset_pixels.to_le_bytes());
    digest_field(&mut bytes, scroll_extent_identity_sha256.as_bytes());
    bytes.extend_from_slice(&(instances.len() as u64).to_le_bytes());
    for instance in instances {
        digest_field(&mut bytes, instance.instance_key.as_bytes());
        bytes.extend_from_slice(&instance.instance_ordinal.to_le_bytes());
        digest_field(&mut bytes, instance.template_panel.window_id().as_bytes());
        digest_field(&mut bytes, instance.template_panel.surface_id().as_bytes());
        digest_field(
            &mut bytes,
            instance
                .template_panel
                .authored_tree_identity_sha256()
                .as_bytes(),
        );
        digest_field(
            &mut bytes,
            instance
                .template_panel
                .source_content()
                .normalized_game_relative_path
                .as_bytes(),
        );
        digest_field(
            &mut bytes,
            instance
                .template_panel
                .source_content()
                .payload_sha256
                .as_bytes(),
        );
        bytes.push(u8::from(instance.global_root_sibling));
        match instance.host_instance_ordinal {
            Some(ordinal) => {
                bytes.push(1);
                bytes.extend_from_slice(&ordinal.to_le_bytes());
            }
            None => bytes.push(0),
        }
        bytes.extend_from_slice(&(instance.host_child_path.len() as u64).to_le_bytes());
        for ordinal in &instance.host_child_path {
            bytes.extend_from_slice(&ordinal.to_le_bytes());
        }
        bytes.extend_from_slice(&instance.owner_child_insertion_ordinal.to_le_bytes());
        digest_rect(&mut bytes, instance.parent_rect);
        digest_rect(&mut bytes, instance.required_clip);
        bytes.extend_from_slice(&(instance.semantics.len() as u64).to_le_bytes());
        for (path, semantic) in &instance.semantics {
            bytes.extend_from_slice(&(path.len() as u64).to_le_bytes());
            for ordinal in path {
                bytes.extend_from_slice(&ordinal.to_le_bytes());
            }
            digest_semantic(&mut bytes, semantic);
        }
        bytes.extend_from_slice(&(instance.viewer_actions.len() as u64).to_le_bytes());
        for (path, action) in &instance.viewer_actions {
            bytes.extend_from_slice(&(path.len() as u64).to_le_bytes());
            for ordinal in path {
                bytes.extend_from_slice(&ordinal.to_le_bytes());
            }
            digest_field(&mut bytes, &action.digest_record());
        }
    }
    match extended_scrollbar {
        None => bytes.push(0),
        Some(scrollbar) => {
            bytes.push(1);
            digest_field(
                &mut bytes,
                scrollbar
                    .source_content
                    .normalized_game_relative_path
                    .as_bytes(),
            );
            digest_field(
                &mut bytes,
                scrollbar.source_content.payload_sha256.as_bytes(),
            );
            digest_field(
                &mut bytes,
                crate::authored_role_index::authored_tree_identity_sha256(&scrollbar.style_root)
                    .as_bytes(),
            );
            digest_rect(&mut bytes, scrollbar.clip_rect);
            bytes.extend_from_slice(&scrollbar.thumb_base_pixels.to_le_bytes());
            bytes.extend_from_slice(&scrollbar.thumb_travel_pixels.to_le_bytes());
            bytes.extend_from_slice(&scrollbar.thumb_authored_offset_pixels.to_le_bytes());
            for part in [
                &scrollbar.background,
                &scrollbar.track,
                &scrollbar.decrease,
                &scrollbar.increase,
                &scrollbar.slider,
            ] {
                digest_field(&mut bytes, part.sprite_name.as_bytes());
                digest_rect(&mut bytes, part.rect);
            }
        }
    }
    bytes.extend_from_slice(&(collection_scroll_regions.len() as u64).to_le_bytes());
    for scroll in collection_scroll_regions {
        digest_field(&mut bytes, scroll.element_id.as_bytes());
        bytes.extend_from_slice(&(scroll.child_path.len() as u64).to_le_bytes());
        for ordinal in &scroll.child_path {
            bytes.extend_from_slice(&ordinal.to_le_bytes());
        }
        digest_rect(&mut bytes, scroll.viewport_rect);
        bytes.extend_from_slice(&scroll.max_offset_pixels.to_le_bytes());
        bytes.extend_from_slice(&scroll.offset_pixels.to_le_bytes());
        bytes.extend_from_slice(&scroll.wheel_step_pixels.to_le_bytes());
        bytes.push(u8::from(scroll.smooth_scrolling));
        digest_field(&mut bytes, scroll.geometry_source_record.as_bytes());
    }
    Sha256::hex(&bytes)
}

fn digest_field(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value);
}

fn digest_rect(bytes: &mut Vec<u8>, rect: AuthenticatedElementRect) {
    bytes.extend_from_slice(&rect.x.to_le_bytes());
    bytes.extend_from_slice(&rect.y.to_le_bytes());
    bytes.extend_from_slice(&rect.width.to_le_bytes());
    bytes.extend_from_slice(&rect.height.to_le_bytes());
}

fn digest_semantic(bytes: &mut Vec<u8>, semantic: &AuthenticatedElementSemantic) {
    match semantic {
        AuthenticatedElementSemantic::Hidden => bytes.push(0),
        AuthenticatedElementSemantic::Structural => bytes.push(1),
        AuthenticatedElementSemantic::StandardButton => bytes.push(2),
        AuthenticatedElementSemantic::ReadOnlyButton {
            one_based_frame,
            interaction,
        } => {
            bytes.push(7);
            bytes.extend_from_slice(&one_based_frame.to_le_bytes());
            bytes.push(match interaction {
                crate::runtime_graphics::ButtonVisualState::Up => 0,
                crate::runtime_graphics::ButtonVisualState::Down => 1,
                crate::runtime_graphics::ButtonVisualState::Over => 2,
                crate::runtime_graphics::ButtonVisualState::Disable => 3,
            });
        }
        AuthenticatedElementSemantic::PassiveSprite { one_based_frame } => {
            bytes.push(3);
            bytes.extend_from_slice(&one_based_frame.to_le_bytes());
        }
        AuthenticatedElementSemantic::Text { resolved_text } => {
            bytes.push(4);
            digest_field(bytes, resolved_text.as_bytes());
        }
        AuthenticatedElementSemantic::Progress { fill } => {
            bytes.push(5);
            bytes.push(match fill.effect {
                crate::widget_visual_state::ProgressEffect::Linear => 0,
                crate::widget_visual_state::ProgressEffect::StartEnd => 1,
                crate::widget_visual_state::ProgressEffect::Reverse => 2,
                crate::widget_visual_state::ProgressEffect::MinMaxColor => 3,
                crate::widget_visual_state::ProgressEffect::MinMaxTexture => 4,
                crate::widget_visual_state::ProgressEffect::Radial => 5,
                crate::widget_visual_state::ProgressEffect::Circular => 6,
            });
            bytes.extend_from_slice(&fill.current_state.to_bits().to_le_bytes());
            match fill.circular_mix {
                Some(value) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
                }
                None => bytes.push(0),
            }
        }
        AuthenticatedElementSemantic::BlendFrames {
            current_one_based_frame,
            next_one_based_frame,
            animation_time,
        } => {
            bytes.push(6);
            bytes.extend_from_slice(&current_one_based_frame.to_le_bytes());
            bytes.extend_from_slice(&next_one_based_frame.to_le_bytes());
            bytes.extend_from_slice(&animation_time.to_bits().to_le_bytes());
        }
    }
}

fn authenticated_rect_is_valid(rect: AuthenticatedElementRect) -> bool {
    rect.width > 0
        && rect.height > 0
        && i64::from(rect.x) + i64::from(rect.width) <= i64::from(i32::MAX)
        && i64::from(rect.y) + i64::from(rect.height) <= i64::from(i32::MAX)
}

fn authenticated_rect_contains(
    outer: AuthenticatedElementRect,
    inner: AuthenticatedElementRect,
) -> bool {
    i64::from(inner.x) >= i64::from(outer.x)
        && i64::from(inner.y) >= i64::from(outer.y)
        && i64::from(inner.x) + i64::from(inner.width)
            <= i64::from(outer.x) + i64::from(outer.width)
        && i64::from(inner.y) + i64::from(inner.height)
            <= i64::from(outer.y) + i64::from(outer.height)
}

fn authenticated_rects_intersect(
    left: AuthenticatedElementRect,
    right: AuthenticatedElementRect,
) -> bool {
    i64::from(left.x) < i64::from(right.x) + i64::from(right.width)
        && i64::from(right.x) < i64::from(left.x) + i64::from(left.width)
        && i64::from(left.y) < i64::from(right.y) + i64::from(right.height)
        && i64::from(right.y) < i64::from(left.y) + i64::from(left.height)
}

pub(crate) fn seal_authenticated_panel_semantics(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
    panel_lease: &AuthenticatedReplayPanelLease<'_>,
    elements: BTreeMap<Vec<u32>, AuthenticatedElementSemantic>,
) -> std::result::Result<AuthenticatedPanelSemanticCensus, AuthenticatedComposeRejection> {
    let surface = index
        .validate_panel_token(admission, panel)
        .map_err(|_| AuthenticatedComposeRejection::PanelAuthentication)?;
    if !frame_authority.remains_staged_for(manager)
        || !panel_lease.remains_current()
        || panel_lease.window_id() != panel.window_id()
        || panel_lease.runtime_instance_token().trim().is_empty()
        || elements.len() != surface.elements.len()
    {
        return Err(AuthenticatedComposeRejection::SemanticCensusIdentity);
    }
    validate_authenticated_tree(panel.authored_root(), surface)?;
    for role in &surface.elements {
        let semantic = elements.get(&role.child_path).ok_or_else(|| {
            AuthenticatedComposeRejection::MissingSemanticState(role.child_path.clone())
        })?;
        let authored = element_at_child_path(panel.authored_root(), &role.child_path)
            .ok_or(AuthenticatedComposeRejection::SemanticCensusIdentity)?;
        validate_element_semantic(role, authored, semantic)?;
        if matches!(semantic, AuthenticatedElementSemantic::Hidden)
            && surface.elements.iter().any(|descendant| {
                descendant.child_path.len() > role.child_path.len()
                    && descendant.child_path.starts_with(&role.child_path)
                    && !matches!(
                        elements.get(&descendant.child_path),
                        Some(AuthenticatedElementSemantic::Hidden)
                    )
            })
        {
            return Err(AuthenticatedComposeRejection::IncompatibleSemanticState(
                role.child_path.clone(),
            ));
        }
    }
    if elements
        .keys()
        .any(|path| !surface.elements.iter().any(|role| role.child_path == *path))
    {
        return Err(AuthenticatedComposeRejection::SemanticCensusIdentity);
    }
    Ok(AuthenticatedPanelSemanticCensus {
        index_identity_sha256: index.index_identity_sha256.clone(),
        window_id: panel.window_id().to_string(),
        surface_id: surface.surface_id.clone(),
        source_content: surface.source_content.clone(),
        runtime_instance_token: panel_lease.runtime_instance_token().to_string(),
        frame_generation: frame_authority.frame_generation(),
        elements,
    })
}

pub(crate) fn validate_element_semantic(
    role: &crate::authored_role_index::AuthoredElementRole,
    authored: &GuiElement,
    semantic: &AuthenticatedElementSemantic,
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    let reject =
        || AuthenticatedComposeRejection::IncompatibleSemanticState(role.child_path.clone());
    let standard_button = WidgetFamily::from_element_type(&authored.element_type)
        .is_some_and(WidgetFamily::standard_button_carrier);
    match semantic {
        AuthenticatedElementSemantic::Hidden => Ok(()),
        AuthenticatedElementSemantic::Structural => {
            if role.sprites.is_empty()
                && role.direct_textures.is_empty()
                && authored.font.is_none()
                && authored.text.is_none()
                && !authored.element_type.to_ascii_lowercase().contains("text")
            {
                Ok(())
            } else {
                Err(reject())
            }
        }
        AuthenticatedElementSemantic::StandardButton => {
            if standard_button && role.sprites.len() == 1 {
                Ok(())
            } else {
                Err(reject())
            }
        }
        AuthenticatedElementSemantic::ReadOnlyButton {
            one_based_frame, ..
        } => {
            if !standard_button || role.sprites.len() != 1 {
                return Err(reject());
            }
            let frame_count = role.sprites[0].frame_count;
            if *one_based_frame == 0 || *one_based_frame > frame_count {
                return Err(reject());
            }
            Ok(())
        }
        AuthenticatedElementSemantic::PassiveSprite { one_based_frame } => {
            if standard_button || role.sprites.len() != 1 {
                return Err(reject());
            }
            let frame_count = role.sprites[0].frame_count;
            if *one_based_frame == 0 || *one_based_frame > frame_count {
                return Err(reject());
            }
            Ok(())
        }
        AuthenticatedElementSemantic::Text { resolved_text, .. } => {
            if !resolved_text.is_empty()
                && authored.font.is_some()
                && role.font.is_some()
                && (authored.text.is_some()
                    || authored.element_type.to_ascii_lowercase().contains("text"))
            {
                Ok(())
            } else {
                Err(reject())
            }
        }
        AuthenticatedElementSemantic::Progress { fill } => {
            if role.sprites.len() != 1
                || role.sprites[0].texture_slots.len() != 2
                || !role.sprites[0]
                    .sprite_kind
                    .eq_ignore_ascii_case("progressbartype")
                || !fill.valid()
            {
                return Err(reject());
            }
            let shader = &role.sprites[0].effective_shader_file;
            if shader.to_ascii_lowercase().contains("progress") {
                Ok(())
            } else {
                Err(reject())
            }
        }
        AuthenticatedElementSemantic::BlendFrames {
            current_one_based_frame,
            next_one_based_frame,
            animation_time,
        } => {
            if role.sprites.len() != 1
                || *current_one_based_frame == 0
                || *next_one_based_frame == 0
                || *current_one_based_frame > role.sprites[0].frame_count
                || *next_one_based_frame > role.sprites[0].frame_count
                || !animation_time.is_finite()
                || !(0.0..=1.0).contains(animation_time)
                || !role.sprites[0]
                    .effective_shader_file
                    .eq_ignore_ascii_case("gfx/FX/buttonstate_blendframes.shader")
            {
                Err(reject())
            } else {
                Ok(())
            }
        }
    }
}

fn element_at_child_path<'a>(root: &'a GuiElement, child_path: &[u32]) -> Option<&'a GuiElement> {
    let mut current = root;
    for ordinal in child_path {
        current = current.children.get(usize::try_from(*ordinal).ok()?)?;
    }
    Some(current)
}

pub(crate) struct AuthenticatedUiBatch {
    framebuffer: RgbaImage,
    reports: Vec<AuthenticatedComposeReport>,
}

impl AuthenticatedUiBatch {
    pub(crate) fn into_parts(self) -> (RgbaImage, Vec<AuthenticatedComposeReport>) {
        (self.framebuffer, self.reports)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct AuthenticatedComposeReport {
    report: ComposeReport,
    debug_rects: Vec<DebugRect>,
    index_identity_sha256: String,
    window_id: String,
    surface_id: String,
    source_content: replay_core::UsedContentKey,
    runtime_instance_token: String,
    viewport_width: usize,
    viewport_height: usize,
    root_width: usize,
    root_height: usize,
    frame_generation: u64,
    elements: Vec<AuthenticatedComposedElement>,
    materialized_instances: Vec<AuthenticatedMaterializedInstanceCensus>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AuthenticatedLayoutPreflight {
    index_identity_sha256: String,
    window_id: String,
    surface_id: String,
    source_content: replay_core::UsedContentKey,
    runtime_instance_token: String,
    viewport_width: usize,
    viewport_height: usize,
    root_width: usize,
    root_height: usize,
    frame_generation: u64,
    elements: Vec<AuthenticatedComposedElement>,
}

/// bytes, authored geometry, current panel lifetime, and runtime state.
pub(crate) struct AuthenticatedSpriteDraw<'a> {
    atlas: &'a RgbaImage,
    frame_count: usize,
    geometry: AuthenticatedSpriteGeometry,
    admitted: AdmittedRuntimeGraphics,
}

struct AuthenticatedInlineIconDraw {
    sprite_name: String,
    zero_based_frame: u32,
    frame_count: usize,
    atlas: Arc<RgbaImage>,
    border: Option<(usize, usize)>,
}

impl<'a> AuthenticatedSpriteDraw<'a> {
    fn new(
        atlas: &'a RgbaImage,
        frame_count: usize,
        geometry: AuthenticatedSpriteGeometry,
        admitted: AdmittedRuntimeGraphics,
    ) -> Self {
        Self {
            atlas,
            frame_count,
            geometry,
            admitted,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        &'a RgbaImage,
        usize,
        AuthenticatedSpriteGeometry,
        AdmittedRuntimeGraphics,
    ) {
        (self.atlas, self.frame_count, self.geometry, self.admitted)
    }
}

impl AuthenticatedLayoutPreflight {
    pub(crate) fn index_identity_sha256(&self) -> &str {
        &self.index_identity_sha256
    }

    pub(crate) fn runtime_instance_token(&self) -> &str {
        &self.runtime_instance_token
    }

    pub(crate) fn window_id(&self) -> &str {
        &self.window_id
    }

    pub(crate) fn surface_id(&self) -> &str {
        &self.surface_id
    }

    pub(crate) fn source_content(&self) -> &replay_core::UsedContentKey {
        &self.source_content
    }

    pub(crate) fn elements(&self) -> &[AuthenticatedComposedElement] {
        &self.elements
    }

    pub(crate) fn viewport_size(&self) -> [usize; 2] {
        [self.viewport_width, self.viewport_height]
    }

    pub(crate) fn root_size(&self) -> [usize; 2] {
        [self.root_width, self.root_height]
    }

    pub(crate) fn frame_generation(&self) -> u64 {
        self.frame_generation
    }
}

impl AuthenticatedComposeReport {
    pub(crate) fn report(&self) -> &ComposeReport {
        &self.report
    }

    pub(crate) fn debug_rects(&self) -> &[DebugRect] {
        &self.debug_rects
    }

    pub(crate) fn runtime_instance_token(&self) -> &str {
        &self.runtime_instance_token
    }

    pub(crate) fn index_identity_sha256(&self) -> &str {
        &self.index_identity_sha256
    }

    pub(crate) fn window_id(&self) -> &str {
        &self.window_id
    }

    pub(crate) fn surface_id(&self) -> &str {
        &self.surface_id
    }

    pub(crate) fn source_content(&self) -> &replay_core::UsedContentKey {
        &self.source_content
    }

    pub(crate) fn viewport_size(&self) -> [usize; 2] {
        [self.viewport_width, self.viewport_height]
    }

    pub(crate) fn root_size(&self) -> [usize; 2] {
        [self.root_width, self.root_height]
    }

    pub(crate) fn elements(&self) -> &[AuthenticatedComposedElement] {
        &self.elements
    }

    pub(crate) fn materialized_instances(&self) -> &[AuthenticatedMaterializedInstanceCensus] {
        &self.materialized_instances
    }

    pub(crate) fn frame_generation(&self) -> u64 {
        self.frame_generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedElementRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl AuthenticatedElementRect {
    pub(crate) fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(crate) fn x(self) -> i32 {
        self.x
    }

    pub(crate) fn y(self) -> i32 {
        self.y
    }

    pub(crate) fn width(self) -> i32 {
        self.width
    }

    pub(crate) fn height(self) -> i32 {
        self.height
    }
}

pub(crate) fn authenticated_template_root_extent(
    panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
    parent_width: i32,
    parent_height: i32,
) -> std::result::Result<(i32, i32), AuthenticatedComposeRejection> {
    if parent_width <= 0 || parent_height <= 0 {
        return Err(AuthenticatedComposeRejection::MaterializationGeometry);
    }
    let size = panel
        .authored_root()
        .size
        .as_ref()
        .ok_or(AuthenticatedComposeRejection::MaterializationGeometry)?;
    let width = authenticated_size_measure_to_pixels(&size.x, parent_width)
        .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?;
    let height = authenticated_size_measure_to_pixels(&size.y, parent_height)
        .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?;
    if width <= 0 || height <= 0 {
        return Err(AuthenticatedComposeRejection::MaterializationGeometry);
    }
    Ok((width, height))
}

pub(crate) fn authenticated_template_child_rect(
    panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
    child_path: &[u32],
    root_rect: AuthenticatedElementRect,
) -> std::result::Result<AuthenticatedElementRect, AuthenticatedComposeRejection> {
    if child_path.is_empty() || !authenticated_rect_is_valid(root_rect) {
        return Err(AuthenticatedComposeRejection::MaterializationGeometry);
    }
    let mut parent = rect_from_authenticated(root_rect);
    let mut path = Vec::with_capacity(child_path.len());
    for &ordinal in child_path {
        path.push(ordinal);
        let element = panel
            .authored_element(&path)
            .ok_or(AuthenticatedComposeRejection::MaterializationIdentity)?;
        if element.size.is_none() {
            return Err(AuthenticatedComposeRejection::MaterializationGeometry);
        }
        parent = resolve_authenticated_rect(element, parent, (0, 0))
            .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?;
    }
    Ok(authenticated_element_rect(parent))
}

pub(crate) fn authenticated_template_grid_slot_rect(
    panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
    grid_child_path: &[u32],
    root_rect: AuthenticatedElementRect,
    slot_ordinal: usize,
) -> std::result::Result<AuthenticatedElementRect, AuthenticatedComposeRejection> {
    let grid = panel
        .authored_element(grid_child_path)
        .ok_or(AuthenticatedComposeRejection::MaterializationIdentity)?;
    let grid_parent_path = &grid_child_path[..grid_child_path.len().saturating_sub(1)];
    let parent_rect = if grid_parent_path.is_empty() {
        root_rect
    } else {
        authenticated_template_child_rect(panel, grid_parent_path, root_rect)?
    };
    let mut options = ComposeOptions::new(
        usize::try_from(root_rect.width)
            .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?,
        usize::try_from(root_rect.height)
            .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?,
    );
    options.root_width = options.viewport_width;
    options.root_height = options.viewport_height;
    let layout =
        crate::widget_layout::gridbox_layout(grid, rect_from_authenticated(parent_rect), &options)
            .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?;
    let rect = layout
        .slot_rect(slot_ordinal)
        .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?;
    Ok(authenticated_element_rect(rect))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedComposedElement {
    child_path: Vec<u32>,
    element_id: String,
    authored_name: String,
    element_type: String,
    visible: bool,
    screen_rect: Option<AuthenticatedElementRect>,
    clip_rect: Option<AuthenticatedElementRect>,
    paint_ordinal: Option<u32>,
}

impl AuthenticatedComposedElement {
    pub(crate) fn child_path(&self) -> &[u32] {
        &self.child_path
    }

    pub(crate) fn element_id(&self) -> &str {
        &self.element_id
    }

    pub(crate) fn authored_name(&self) -> &str {
        &self.authored_name
    }

    pub(crate) fn element_type(&self) -> &str {
        &self.element_type
    }

    pub(crate) fn visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn screen_rect(&self) -> Option<AuthenticatedElementRect> {
        self.screen_rect
    }

    pub(crate) fn clip_rect(&self) -> Option<AuthenticatedElementRect> {
        self.clip_rect
    }

    pub(crate) fn paint_ordinal(&self) -> Option<u32> {
        self.paint_ordinal
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthenticatedComposeRejection {
    EmptyAuthenticatedBatch,
    DuplicateAuthenticatedBatchPanel(String),
    AuthenticatedBatchViewportMismatch(String),
    PanelAuthentication,
    RootIdentityMismatch,
    MaterializationIdentity,
    MaterializationGeometry,
    DuplicateMaterializedInstance(String),
    SemanticCensusIdentity,
    MissingSemanticState(Vec<u32>),
    IncompatibleSemanticState(Vec<u32>),
    UnexpectedRuntimeGraphics(Vec<u32>),
    MissingRuntimeGraphics(Vec<u32>),
    RuntimeGraphics {
        child_path: Vec<u32>,
        reason: RuntimeGraphicsRejection,
    },
    MissingSprite {
        child_path: Vec<u32>,
        sprite_name: String,
    },
    SpriteDefinitionMismatch(Vec<u32>),
    UnauthenticatedNameKeyedSpriteOverride(Vec<u32>),
    UnauthenticatedNameKeyedLayoutOverride,
    UnsupportedAnimatedVariant(Vec<u32>),
    UnsupportedMaskingVariant(Vec<u32>),
    UnsupportedMultiTextureVariant(Vec<u32>),
    UnsupportedColorOpacityOrClipVariant(Vec<u32>),
    UnsupportedLayoutVariant(Vec<u32>),
    UnsupportedVisibilityVariant(Vec<u32>),
    UnsupportedTextVariant(Vec<u32>),
    UnsupportedElementFamily(Vec<u32>),
    UnsupportedVariableReference(Vec<u32>),
    MissingAuthenticatedPassiveState(Vec<u32>),
    DuplicateOrMalformedAuthoredField {
        child_path: Vec<u32>,
        field: String,
    },
    StaleRuntimeInstance(Vec<u32>),
    SpriteDecode {
        child_path: Vec<u32>,
        message: String,
    },
    TextDraw {
        child_path: Vec<u32>,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugRect {
    pub name: String,
    pub element_type: String,
    pub child_path: Vec<u32>,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub depth: usize,
}

pub(crate) fn unique_inline_sprite_definition<'a>(
    registry: &'a SpriteRegistry,
    name: &str,
) -> Result<&'a SpriteDefinition> {
    let matches = registry
        .sprites()
        .iter()
        .filter(|sprite| sprite.name.eq_ignore_ascii_case(name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(GameUiError::missing(format!(
            "inline sprite {name:?} has no admitted definition"
        ))),
        [definition] => Ok(*definition),
        _ => Err(GameUiError::missing(format!(
            "inline sprite {name:?} is ambiguous across admitted definitions [{}]",
            matches
                .iter()
                .map(|definition| definition.source_file.as_str())
                .collect::<Vec<_>>()
                .join("|")
        ))),
    }
}

#[derive(Clone, Debug, Default)]
pub struct UiAssetCache {
    sprites: BTreeMap<String, Result<Arc<DecodedSprite>>>,
    textures: BTreeMap<String, Result<Arc<RgbaImage>>>,
    fonts: BTreeMap<String, Result<Arc<BitmapFont>>>,
}

#[derive(Debug)]
enum CacheSlot<'a> {
    Owned(UiAssetCache),
    Borrowed(&'a mut UiAssetCache),
}

impl CacheSlot<'_> {
    fn get_mut(&mut self) -> &mut UiAssetCache {
        match self {
            CacheSlot::Owned(cache) => cache,
            CacheSlot::Borrowed(cache) => cache,
        }
    }
}

#[derive(Debug)]
pub struct Compositor<'a> {
    resolver: &'a AssetResolver,
    registry: &'a SpriteRegistry,
    cache: CacheSlot<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn x(&self) -> i32 {
        self.x
    }

    pub fn y(&self) -> i32 {
        self.y
    }

    pub fn w(&self) -> i32 {
        self.w
    }

    pub fn h(&self) -> i32 {
        self.h
    }
}

pub(crate) fn compose_authenticated_surface_batch(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    viewport_width: usize,
    viewport_height: usize,
    ordered_inputs: &[AuthenticatedPanelBatchInput<'_>],
) -> std::result::Result<AuthenticatedUiBatch, AuthenticatedComposeRejection> {
    let first = ordered_inputs
        .first()
        .ok_or(AuthenticatedComposeRejection::EmptyAuthenticatedBatch)?;
    let empty_legacy_registry = SpriteRegistry::new();
    let mut compositor = Compositor::new(&first.surface.assets, &empty_legacy_registry);
    compositor.compose_authenticated_batch(
        manager,
        frame_authority,
        viewport_width,
        viewport_height,
        ordered_inputs,
    )
}

pub(crate) fn authenticated_dynamic_sprite_natural_size(
    surface: &crate::authored_role_index::AuthenticatedPanelSurface,
    index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    sprite_name: &str,
) -> std::result::Result<[i32; 2], String> {
    index
        .validate_panel_token(admission, &surface.panel)
        .map_err(|error| error.to_string())?;
    let role = index
        .admit_dynamic_sprite(admission, sprite_name)
        .map_err(|error| error.to_string())?;
    let empty_registry = SpriteRegistry::new();
    let mut compositor = Compositor::new(&surface.assets, &empty_registry);
    let definition = compositor
        .authenticated_dynamic_sprite_definition(admission, role, &[])
        .map_err(|error| format!("{error:?}"))?;
    let decoded = compositor
        .decoded_authenticated_sprite(&definition, role, admission, &index.index_identity_sha256)
        .map_err(|error| error.to_string())?;
    let frame = decoded
        .frames
        .first()
        .ok_or_else(|| format!("authenticated sprite {sprite_name:?} has no frames"))?;
    Ok([
        i32::try_from(frame.width).map_err(|_| "sprite width exceeds i32".to_string())?,
        i32::try_from(frame.height).map_err(|_| "sprite height exceeds i32".to_string())?,
    ])
}

#[derive(Clone, Debug)]
struct PreparedEngineInlineIcon {
    sprite: String,
    frame: u32,
    image: RgbaImage,
}

#[allow(clippy::too_many_arguments)]
fn draw_engine_layout(
    font: &BitmapFont,
    layout: &EngineTextLayout,
    inline_icons: &[PreparedEngineInlineIcon],
    image: &mut RgbaImage,
    origin_x: i32,
    origin_y: i32,
    alignment_width: i32,
    align: TextAlign,
    original_text: &str,
    base_color: [u8; 4],
) -> Result<()> {
    if alignment_width <= 0 {
        return Err(GameUiError::parse("TEXT1 alignment width must be positive"));
    }
    if layout.inline_icons.len() != inline_icons.len() {
        return Err(GameUiError::parse(
            "TEXT1 layout/decoded inline-icon counts differ",
        ));
    }
    let rich = font.colorize(original_text, base_color);
    for glyph in &layout.glyphs {
        let line = layout
            .lines
            .get(glyph.line_index)
            .ok_or_else(|| GameUiError::parse("TEXT1 glyph references a missing line"))?;
        let shift = horizontal_line_shift(alignment_width, line.width, align);
        let tint = rich
            .colors
            .get(glyph.visible_char_index)
            .copied()
            .ok_or_else(|| {
                GameUiError::parse("TEXT1 colour stream is shorter than its visible glyph stream")
            })?;
        match font.draw_runtime_glyph(
            image,
            glyph.ch as u32,
            origin_x.saturating_add(glyph.draw_x).saturating_add(shift),
            origin_y.saturating_add(glyph.draw_y),
            tint,
            None,
        ) {
            RuntimeGlyphDraw::Drawn => {}
            RuntimeGlyphDraw::GlyphAbsent => {
                return Err(GameUiError::missing(format!(
                    "TEXT1 font {} lost glyph {:?} after layout",
                    font.name, glyph.ch
                )))
            }
            RuntimeGlyphDraw::PageAbsent { page } => {
                return Err(GameUiError::missing(format!(
                    "TEXT1 font {} lost atlas page {page} after layout",
                    font.name
                )))
            }
        }
    }
    for (planned, prepared) in layout.inline_icons.iter().zip(inline_icons) {
        if planned.sprite != prepared.sprite || planned.frame != prepared.frame {
            return Err(GameUiError::parse(
                "TEXT1 inline-icon identity/order changed between layout and draw",
            ));
        }
        let line = layout
            .lines
            .get(planned.line_index)
            .ok_or_else(|| GameUiError::parse("TEXT1 inline icon references a missing line"))?;
        let shift = horizontal_line_shift(alignment_width, line.width, align);
        draw_image(
            image,
            &prepared.image,
            origin_x.saturating_add(planned.x).saturating_add(shift),
            origin_y
                .saturating_add(planned.line_y)
                .saturating_sub(planned.engine_center_offset_y),
        );
    }
    Ok(())
}

fn horizontal_line_shift(container_width: i32, line_width: i32, align: TextAlign) -> i32 {
    match align {
        TextAlign::Left => 0,
        TextAlign::Center => container_width.saturating_sub(line_width) / 2,
        TextAlign::Right => container_width.saturating_sub(line_width),
    }
}

/// The authored text-effect row a font role must carry.
///
/// The path is compared through the manifest's own normalizer: the authored
/// constant carries the engine's `gfx/FX/` spelling while
/// `normalized_game_relative_path` is lower-cased, so a verbatim comparison
/// rejects every real font role in production while a fixture written in the
/// authored spelling still passes.
fn text_effect_row_is_the_authored_shader(
    content: &portable_content_manifest::UsedContentKey,
    shader_bytes: usize,
) -> bool {
    let Ok(key) = portable_content_manifest::normalize_game_relative_path(
        portable_content_manifest::authored_ui::AUTHORED_BITMAP_FONT_TEXT_EFFECT_PATH,
    ) else {
        return false;
    };
    content.normalized_game_relative_path == key
        && content.payload_sha256 == TEXT_SHADER_SHA256
        && u64::try_from(shader_bytes).ok() == Some(TEXT_SHADER_BYTES)
}

impl<'a> Compositor<'a> {
    pub fn measure_resolved_text_height(
        &mut self,
        element: &GuiElement,
        text: &str,
        runtime: &RuntimeTextContext,
    ) -> Result<i32> {
        if let Some(exact) = runtime.exact.as_ref() {
            if let RuntimeTextLayout::CapturedRuntime(layout) = &exact.layout {
                let mut glyph_bottom = 0;
                for glyph in &layout.glyphs {
                    let font = self.font(&glyph.font)?;
                    let authored = font.chars.get(&glyph.glyph_id).ok_or_else(|| {
                        GameUiError::missing(format!(
                            "captured runtime glyph {} is absent from effective font {}",
                            glyph.glyph_id, glyph.font
                        ))
                    })?;
                    let height = i32::try_from(authored.height).map_err(|_| {
                        GameUiError::parse("captured runtime glyph height exceeds i32")
                    })?;
                    glyph_bottom = glyph_bottom.max(glyph.y.saturating_add(height));
                }
                let mut icon_bottom = 0;
                for icon in &layout.inline_icons {
                    let decoded = self.decoded_sprite(&icon.sprite)?;
                    let frame = usize::try_from(icon.frame)
                        .ok()
                        .and_then(|index| decoded.frames.get(index))
                        .ok_or_else(|| {
                            GameUiError::missing(format!(
                                "captured runtime inline sprite {} frame {} is absent",
                                icon.sprite, icon.frame
                            ))
                        })?;
                    let height = i32::try_from(frame.height).map_err(|_| {
                        GameUiError::parse("captured runtime inline sprite height exceeds i32")
                    })?;
                    icon_bottom = icon_bottom.max(icon.y.saturating_add(height));
                }
                return Ok(glyph_bottom.max(icon_bottom));
            }
        }
        let font_name = element
            .font
            .as_deref()
            .ok_or_else(|| GameUiError::missing("TEXT1 height query has no authored font"))?;
        let font = self.font(font_name)?;
        let max_width = element
            .raw
            .first_value_ci("maxWidth")
            .and_then(|_| scalar_i32_ci(&element.raw, "maxWidth"))
            .or_else(|| {
                element
                    .size
                    .as_ref()
                    .and_then(|size| size.x.raw.parse::<i32>().ok())
            })
            .filter(|width| *width > 0)
            .ok_or_else(|| {
                GameUiError::parse("TEXT1 height query has no positive authored width")
            })?;
        let border = match element.raw.first_value_ci("borderSize") {
            Some(value) => parse_engine_text_border(value)
                .map(|(x, y)| EngineTextBorder::Authored { x, y })
                .ok_or_else(|| GameUiError::parse("TEXT1 height query has malformed borderSize"))?,
            None => EngineTextBorder::ExplicitlyAbsent,
        };
        let (icon_metrics, _) = self.resolve_engine_inline_icons(&font, text)?;
        let layout = layout_engine_text_with_icons(&font, text, max_width, border, &icon_metrics)
            .map_err(|error| GameUiError::parse(error.to_string()))?;
        Ok(layout.total_height)
    }

    pub fn new(resolver: &'a AssetResolver, registry: &'a SpriteRegistry) -> Self {
        Self {
            resolver,
            registry,
            cache: CacheSlot::Owned(UiAssetCache::default()),
        }
    }

    pub fn with_cache(
        resolver: &'a AssetResolver,
        registry: &'a SpriteRegistry,
        cache: &'a mut UiAssetCache,
    ) -> Self {
        Self {
            resolver,
            registry,
            cache: CacheSlot::Borrowed(cache),
        }
    }

    pub fn compose(&mut self, root: &GuiElement, options: &ComposeOptions) -> ComposeOutput {
        let mut image = blank_image(options.viewport_width, options.viewport_height);
        let mut report = ComposeReport::default();
        let mut debug_rects = Vec::new();
        let parent = Rect {
            x: 0,
            y: 0,
            w: options.root_width as i32,
            h: options.root_height as i32,
        };
        self.draw_element(
            root,
            parent,
            0,
            &[],
            &mut image,
            options,
            &mut report,
            &mut debug_rects,
        );
        ComposeOutput {
            image,
            report,
            debug_rects,
        }
    }

    /// panels and final lifetime revalidation discard the private staged image;
    fn compose_authenticated_batch(
        &mut self,
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        viewport_width: usize,
        viewport_height: usize,
        ordered_inputs: &[AuthenticatedPanelBatchInput<'_>],
    ) -> std::result::Result<AuthenticatedUiBatch, AuthenticatedComposeRejection> {
        if viewport_width == 0 || viewport_height == 0 || ordered_inputs.is_empty() {
            return Err(AuthenticatedComposeRejection::EmptyAuthenticatedBatch);
        }
        let first_surface = ordered_inputs[0].surface;
        let mut seen_windows = BTreeSet::new();
        for input in ordered_inputs {
            let index = &input.surface.index;
            let admission = &input.surface.admission;
            let panel = &input.surface.panel;
            let window_id = panel.window_id();
            let options = input.request.options();
            if input.surface.manifest_digest_sha256() != first_surface.manifest_digest_sha256()
                || input.surface.product_modset_sha256() != first_surface.product_modset_sha256()
                || input.surface.index.index_identity_sha256
                    != first_surface.index.index_identity_sha256
                || input.surface.localisation_language != first_surface.localisation_language
                || !self.resolver.is_admitted_for(admission)
            {
                return Err(AuthenticatedComposeRejection::PanelAuthentication);
            }
            if !seen_windows.insert(window_id.to_string()) {
                return Err(
                    AuthenticatedComposeRejection::DuplicateAuthenticatedBatchPanel(
                        window_id.to_string(),
                    ),
                );
            }
            if !input
                .request
                .remains_current(manager, frame_authority, panel, input.panel_lease)
                || options.viewport_width != viewport_width
                || options.viewport_height != viewport_height
                || options.root_width != viewport_width
                || options.root_height != viewport_height
            {
                return Err(
                    AuthenticatedComposeRejection::AuthenticatedBatchViewportMismatch(
                        window_id.to_string(),
                    ),
                );
            }
            validate_preflight(
                input.preflight,
                manager,
                frame_authority,
                index,
                panel,
                input.panel_lease,
                &options,
            )?;
            validate_semantic_census(
                input.semantics,
                manager,
                frame_authority,
                index,
                admission,
                panel,
                input.panel_lease,
            )?;
            if let Some(plan) = input.materialized_instances {
                validate_materialization_plan(
                    plan,
                    manager,
                    frame_authority,
                    index,
                    admission,
                    panel,
                    input.panel_lease,
                    viewport_width,
                    viewport_height,
                )?;
            }
        }

        let mut framebuffer = blank_image(viewport_width, viewport_height);
        let mut reports = Vec::with_capacity(ordered_inputs.len());
        for input in ordered_inputs {
            let index = &input.surface.index;
            let admission = &input.surface.admission;
            let panel = &input.surface.panel;
            let options = input.request.options();
            reports.push(self.compose_authenticated_into(
                manager,
                frame_authority,
                &mut framebuffer,
                &options,
                index,
                admission,
                panel,
                input.panel_lease,
                input.preflight,
                input.widget_states,
                input.semantics,
                input.materialized_instances,
            )?);
        }
        for (input, report) in ordered_inputs.iter().zip(reports.iter_mut()) {
            if let Some(plan) = input.materialized_instances {
                self.compose_authenticated_global_materialized_into(
                    manager,
                    frame_authority,
                    &mut framebuffer,
                    &input.request.options(),
                    &input.surface.index,
                    &input.surface.admission,
                    &input.surface.panel,
                    input.panel_lease,
                    plan,
                    report,
                )?;
            }
        }
        for input in ordered_inputs {
            let index = &input.surface.index;
            let admission = &input.surface.admission;
            let panel = &input.surface.panel;
            let options = input.request.options();
            validate_preflight(
                input.preflight,
                manager,
                frame_authority,
                index,
                panel,
                input.panel_lease,
                &options,
            )?;
            validate_semantic_census(
                input.semantics,
                manager,
                frame_authority,
                index,
                admission,
                panel,
                input.panel_lease,
            )?;
            if let Some(plan) = input.materialized_instances {
                validate_materialization_plan(
                    plan,
                    manager,
                    frame_authority,
                    index,
                    admission,
                    panel,
                    input.panel_lease,
                    viewport_width,
                    viewport_height,
                )?;
            }
        }
        Ok(AuthenticatedUiBatch {
            framebuffer,
            reports,
        })
    }

    /// current open lifetime without drawing pixels or requiring pointer
    pub(crate) fn preflight_authenticated_layout(
        &mut self,
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        request: &AuthenticatedPanelComposeRequest,
        index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        panel_lease: &AuthenticatedReplayPanelLease<'_>,
        semantics: &AuthenticatedPanelSemanticCensus,
    ) -> std::result::Result<AuthenticatedLayoutPreflight, AuthenticatedComposeRejection> {
        let options = request.options();
        let surface = index
            .validate_panel_token(admission, panel)
            .map_err(|_| AuthenticatedComposeRejection::PanelAuthentication)?;
        if !frame_authority.remains_staged_for(manager)
            || !self.resolver.has_portable_content_admission()
            || !self.resolver.is_admitted_for(admission)
            || !request.remains_current(manager, frame_authority, panel, panel_lease)
            || !authenticated_options_are_static(&options)
        {
            return Err(AuthenticatedComposeRejection::PanelAuthentication);
        }
        let root = panel.authored_root();
        if !panel_lease.remains_current()
            || panel_lease.window_id() != panel.window_id()
            || panel_lease.runtime_instance_token().trim().is_empty()
            || !root.name.eq_ignore_ascii_case(&surface.surface_id)
        {
            return Err(AuthenticatedComposeRejection::RootIdentityMismatch);
        }
        validate_authenticated_tree(root, surface)?;
        validate_semantic_census(
            semantics,
            manager,
            frame_authority,
            index,
            admission,
            panel,
            panel_lease,
        )?;
        validate_authenticated_dimensions(&options, None)?;
        let parent = Rect {
            x: 0,
            y: 0,
            w: options.root_width as i32,
            h: options.root_height as i32,
        };
        let mut elements = Vec::with_capacity(surface.elements.len());
        self.preflight_element_authenticated(
            root,
            parent,
            &[],
            admission,
            &index.index_identity_sha256,
            surface,
            semantics,
            &mut elements,
            Rect {
                x: 0,
                y: 0,
                w: options.viewport_width as i32,
                h: options.viewport_height as i32,
            },
        )?;
        if elements.len() != surface.elements.len() {
            return Err(AuthenticatedComposeRejection::RootIdentityMismatch);
        }
        let mut next_paint_ordinal = 0_u32;
        for element in &mut elements {
            let semantic = semantics.elements.get(&element.child_path).ok_or_else(|| {
                AuthenticatedComposeRejection::MissingSemanticState(element.child_path.clone())
            })?;
            if element.visible
                && matches!(
                    semantic,
                    AuthenticatedElementSemantic::StandardButton
                        | AuthenticatedElementSemantic::ReadOnlyButton { .. }
                        | AuthenticatedElementSemantic::PassiveSprite { .. }
                        | AuthenticatedElementSemantic::Text { .. }
                        | AuthenticatedElementSemantic::Progress { .. }
                        | AuthenticatedElementSemantic::BlendFrames { .. }
                )
            {
                element.paint_ordinal = Some(next_paint_ordinal);
                next_paint_ordinal = next_paint_ordinal
                    .checked_add(1)
                    .ok_or(AuthenticatedComposeRejection::RootIdentityMismatch)?;
            }
        }
        Ok(AuthenticatedLayoutPreflight {
            index_identity_sha256: index.index_identity_sha256.clone(),
            window_id: panel.window_id().to_string(),
            surface_id: surface.surface_id.clone(),
            source_content: surface.source_content.clone(),
            runtime_instance_token: panel_lease.runtime_instance_token().to_string(),
            viewport_width: options.viewport_width,
            viewport_height: options.viewport_height,
            root_width: options.root_width,
            root_height: options.root_height,
            frame_generation: frame_authority.frame_generation(),
            elements,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn preflight_element_authenticated(
        &mut self,
        element: &GuiElement,
        parent: Rect,
        child_path: &[u32],
        admission: &ValidatedContentAdmission,
        index_identity_sha256: &str,
        surface: &crate::authored_role_index::AuthoredSurfaceRole,
        semantics: &AuthenticatedPanelSemanticCensus,
        elements: &mut Vec<AuthenticatedComposedElement>,
        required_containment: Rect,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        let role = surface
            .elements
            .iter()
            .find(|role| role.child_path == child_path)
            .ok_or(AuthenticatedComposeRejection::RootIdentityMismatch)?;
        let semantic = semantics.elements.get(child_path).ok_or_else(|| {
            AuthenticatedComposeRejection::MissingSemanticState(child_path.to_vec())
        })?;
        if matches!(semantic, AuthenticatedElementSemantic::Hidden) {
            record_hidden_authenticated_subtree(element, child_path, surface, elements)?;
            return Ok(());
        }
        validate_authenticated_render_fields(element, role, child_path)?;
        if role.sprites.len() > 1 {
            return Err(
                AuthenticatedComposeRejection::UnsupportedMultiTextureVariant(child_path.to_vec()),
            );
        }
        let standard_button = WidgetFamily::from_element_type(&element.element_type)
            .is_some_and(WidgetFamily::standard_button_carrier);
        if standard_button && role.sprites.is_empty() {
            return Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                child_path.to_vec(),
            ));
        }
        let sprite = role
            .sprites
            .first()
            .map(|sprite_role| {
                self.authenticated_sprite_definition(admission, sprite_role, child_path)
            })
            .transpose()?;
        let natural = self.natural_size_authenticated(
            element,
            sprite.as_ref(),
            role.sprites.first(),
            child_path,
            admission,
            index_identity_sha256,
        )?;
        let rect = if child_path.is_empty()
            && element.position.is_none()
            && element.size.is_none()
            && element.orientation.is_none()
        {
            parent
        } else {
            resolve_authenticated_open_rect(element, parent, natural, child_path.is_empty())
                .map_err(|field| {
                    AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                        child_path: child_path.to_vec(),
                        field,
                    }
                })?
        };
        let element_clip = intersect_rect(required_containment, rect).ok_or_else(|| {
            AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(child_path.to_vec())
        })?;
        let element_clip = authenticated_content_clip(element, element_clip, child_path)?;
        elements.push(AuthenticatedComposedElement {
            child_path: child_path.to_vec(),
            element_id: role.element_id.clone(),
            authored_name: role.authored_name.clone(),
            element_type: role.element_type.clone(),
            visible: true,
            screen_rect: Some(authenticated_element_rect(rect)),
            clip_rect: Some(authenticated_element_rect(element_clip)),
            paint_ordinal: None,
        });
        let child_containment = match authored_clipping_mode(element, child_path)? {
            AuthoredClippingMode::No => required_containment,
            AuthoredClippingMode::YesOrUnspecified => element_clip,
        };
        for (ordinal, child) in element.children.iter().enumerate() {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| AuthenticatedComposeRejection::RootIdentityMismatch)?;
            let mut path = child_path.to_vec();
            path.push(ordinal);
            self.preflight_element_authenticated(
                child,
                rect,
                &path,
                admission,
                index_identity_sha256,
                surface,
                semantics,
                elements,
                child_containment,
            )?;
        }
        Ok(())
    }

    fn compose_authenticated_into(
        &mut self,
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        final_framebuffer: &mut RgbaImage,
        options: &ComposeOptions,
        index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        panel_lease: &AuthenticatedReplayPanelLease<'_>,
        preflight: &AuthenticatedLayoutPreflight,
        widget_states: &BTreeMap<Vec<u32>, AuthenticatedViewerWidgetState>,
        semantics: &AuthenticatedPanelSemanticCensus,
        materialized_instances: Option<&AuthenticatedMaterializationPlan>,
    ) -> std::result::Result<AuthenticatedComposeReport, AuthenticatedComposeRejection> {
        let surface = index
            .validate_panel_token(admission, panel)
            .map_err(|_| AuthenticatedComposeRejection::PanelAuthentication)?;
        if !frame_authority.remains_staged_for(manager)
            || !self.resolver.has_portable_content_admission()
            || !self.resolver.is_admitted_for(admission)
            || !authenticated_options_are_static(options)
        {
            return Err(AuthenticatedComposeRejection::PanelAuthentication);
        }
        let root = panel.authored_root();
        if !root.name.eq_ignore_ascii_case(&surface.surface_id)
            || !panel_lease.remains_current()
            || panel_lease.window_id() != panel.window_id()
            || panel_lease.runtime_instance_token().trim().is_empty()
        {
            return Err(AuthenticatedComposeRejection::RootIdentityMismatch);
        }
        validate_preflight(
            preflight,
            manager,
            frame_authority,
            index,
            panel,
            panel_lease,
            options,
        )?;
        validate_semantic_census(
            semantics,
            manager,
            frame_authority,
            index,
            admission,
            panel,
            panel_lease,
        )?;
        validate_authenticated_tree(root, surface)?;
        if let Some(name) = options.sprite_name_overrides.keys().next() {
            let child_path = surface
                .elements
                .iter()
                .find(|element| element.authored_name == *name)
                .map(|element| element.child_path.clone())
                .unwrap_or_default();
            return Err(
                AuthenticatedComposeRejection::UnauthenticatedNameKeyedSpriteOverride(child_path),
            );
        }
        if !options.position_overrides.is_empty()
            || !options.hidden_elements.is_empty()
            || !options.frame_overrides.is_empty()
            || !options.text_overrides.is_empty()
            || !options.progress_overrides.is_empty()
            || !options.runtime_text_by_element.is_empty()
            || !options.sprite_overrides.is_empty()
            || !options.position_overrides_by_child_path.is_empty()
            || !options.hidden_element_paths.is_empty()
            || !options.sprite_name_overrides_by_child_path.is_empty()
        {
            return Err(AuthenticatedComposeRejection::UnauthenticatedNameKeyedLayoutOverride);
        }
        for (child_path, state) in widget_states {
            let Some(element) = surface
                .elements
                .iter()
                .find(|element| element.child_path == *child_path)
            else {
                return Err(AuthenticatedComposeRejection::UnexpectedRuntimeGraphics(
                    child_path.clone(),
                ));
            };
            let standard_button = WidgetFamily::from_element_type(&element.element_type)
                .is_some_and(WidgetFamily::standard_button_carrier);
            let semantic = semantics.elements.get(child_path).ok_or_else(|| {
                AuthenticatedComposeRejection::MissingSemanticState(child_path.clone())
            })?;
            let current_identity = admit_widget_identity(index, admission, panel, child_path)
                .map_err(|_| {
                    AuthenticatedComposeRejection::StaleRuntimeInstance(child_path.clone())
                })?;
            if !standard_button
                || !matches!(semantic, AuthenticatedElementSemantic::StandardButton)
                || state.identity() != &current_identity
                || state.runtime_instance_token() != panel_lease.runtime_instance_token()
                || state.frame_generation() != frame_authority.frame_generation()
            {
                return Err(AuthenticatedComposeRejection::StaleRuntimeInstance(
                    child_path.clone(),
                ));
            }
        }
        for element in &surface.elements {
            let standard_button = WidgetFamily::from_element_type(&element.element_type)
                .is_some_and(WidgetFamily::standard_button_carrier);
            let semantic = semantics.elements.get(&element.child_path).ok_or_else(|| {
                AuthenticatedComposeRejection::MissingSemanticState(element.child_path.clone())
            })?;
            if standard_button
                && matches!(semantic, AuthenticatedElementSemantic::StandardButton)
                && !widget_states.contains_key(&element.child_path)
            {
                return Err(AuthenticatedComposeRejection::MissingRuntimeGraphics(
                    element.child_path.clone(),
                ));
            }
        }

        validate_authenticated_dimensions(options, Some(final_framebuffer))?;
        let mut report = ComposeReport::default();
        let mut debug_rects = Vec::new();
        let mut elements = Vec::with_capacity(surface.elements.len());
        let mut materialized_census = Vec::new();
        let parent = Rect {
            x: 0,
            y: 0,
            w: options.root_width as i32,
            h: options.root_height as i32,
        };
        compose_rgba_transaction(final_framebuffer, |staged_framebuffer| {
            self.draw_element_authenticated(
                root,
                parent,
                0,
                &[],
                manager,
                frame_authority,
                staged_framebuffer,
                options,
                &mut report,
                &mut debug_rects,
                index,
                admission,
                panel,
                panel,
                panel_lease,
                widget_states,
                semantics,
                &mut elements,
                materialized_instances,
                None,
                &mut materialized_census,
                Rect {
                    x: 0,
                    y: 0,
                    w: options.viewport_width as i32,
                    h: options.viewport_height as i32,
                },
            )?;
            if let Some(scrollbar) = materialized_instances
                .and_then(AuthenticatedMaterializationPlan::extended_scrollbar)
            {
                self.draw_authenticated_extended_scrollbar(
                    manager,
                    frame_authority,
                    staged_framebuffer,
                    &mut report,
                    index,
                    admission,
                    panel,
                    panel_lease,
                    scrollbar,
                )?;
            }
            validate_final_census_against_preflight(&elements, preflight)?;
            if materialized_instances.is_none() && !materialized_census.is_empty() {
                return Err(AuthenticatedComposeRejection::MaterializationIdentity);
            }
            Ok(())
        })?;
        Ok(AuthenticatedComposeReport {
            report,
            debug_rects,
            index_identity_sha256: index.index_identity_sha256.clone(),
            window_id: panel.window_id().to_string(),
            surface_id: surface.surface_id.clone(),
            source_content: surface.source_content.clone(),
            runtime_instance_token: panel_lease.runtime_instance_token().to_string(),
            viewport_width: options.viewport_width,
            viewport_height: options.viewport_height,
            root_width: options.root_width,
            root_height: options.root_height,
            frame_generation: frame_authority.frame_generation(),
            elements,
            materialized_instances: materialized_census,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn compose_authenticated_global_materialized_into(
        &mut self,
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        final_framebuffer: &mut RgbaImage,
        options: &ComposeOptions,
        index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        panel_lease: &AuthenticatedReplayPanelLease<'_>,
        plan: &AuthenticatedMaterializationPlan,
        report: &mut AuthenticatedComposeReport,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        if report.window_id != panel.window_id()
            || report.runtime_instance_token != panel_lease.runtime_instance_token()
            || report.frame_generation != frame_authority.frame_generation()
            || !frame_authority.remains_staged_for(manager)
            || !panel_lease.remains_current()
        {
            return Err(AuthenticatedComposeRejection::MaterializationIdentity);
        }
        compose_rgba_transaction(final_framebuffer, |staged_framebuffer| {
            self.draw_materialized_instances_at(
                plan,
                Some(u32::MAX),
                &[],
                0,
                manager,
                frame_authority,
                staged_framebuffer,
                options,
                &mut report.report,
                &mut report.debug_rects,
                index,
                admission,
                panel,
                panel_lease,
                &mut report.materialized_instances,
                1,
            )?;
            report
                .materialized_instances
                .sort_by_key(|instance| instance.rendered_root_path.instance_ordinal);
            validate_materialized_result_order(plan, &report.materialized_instances)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_element_authenticated(
        &mut self,
        element: &GuiElement,
        parent: Rect,
        depth: usize,
        child_path: &[u32],
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        image: &mut RgbaImage,
        options: &ComposeOptions,
        report: &mut ComposeReport,
        debug_rects: &mut Vec<DebugRect>,
        index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        runtime_owner_panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        panel_lease: &AuthenticatedReplayPanelLease<'_>,
        widget_states: &BTreeMap<Vec<u32>, AuthenticatedViewerWidgetState>,
        semantics: &AuthenticatedPanelSemanticCensus,
        elements: &mut Vec<AuthenticatedComposedElement>,
        materialization_plan: Option<&AuthenticatedMaterializationPlan>,
        current_instance_ordinal: Option<u32>,
        materialized_census: &mut Vec<AuthenticatedMaterializedInstanceCensus>,
        required_containment: Rect,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        let surface = index
            .validate_panel_token(admission, panel)
            .map_err(|_| AuthenticatedComposeRejection::PanelAuthentication)?;
        let role = surface
            .elements
            .iter()
            .find(|role| role.child_path == child_path)
            .ok_or_else(|| {
                AuthenticatedComposeRejection::UnexpectedRuntimeGraphics(child_path.to_vec())
            })?;
        let semantic = semantics.elements.get(child_path).ok_or_else(|| {
            AuthenticatedComposeRejection::MissingSemanticState(child_path.to_vec())
        })?;
        if matches!(semantic, AuthenticatedElementSemantic::Hidden) {
            record_hidden_authenticated_subtree(element, child_path, surface, elements)?;
            return Ok(());
        }
        validate_authenticated_render_fields(element, role, child_path)?;
        if role.sprites.len() > 1 {
            return Err(
                AuthenticatedComposeRejection::UnsupportedMultiTextureVariant(child_path.to_vec()),
            );
        }
        let standard_button = WidgetFamily::from_element_type(&element.element_type)
            .is_some_and(WidgetFamily::standard_button_carrier);
        if standard_button && role.sprites.is_empty() {
            return Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                child_path.to_vec(),
            ));
        }
        let widget_state = if matches!(semantic, AuthenticatedElementSemantic::StandardButton) {
            let state = widget_states.get(child_path).ok_or_else(|| {
                AuthenticatedComposeRejection::MissingRuntimeGraphics(child_path.to_vec())
            })?;
            if !panel_lease.remains_current()
                || state.runtime_instance_token() != panel_lease.runtime_instance_token()
            {
                return Err(AuthenticatedComposeRejection::StaleRuntimeInstance(
                    child_path.to_vec(),
                ));
            }
            if !state.lifecycle().visible {
                return Err(AuthenticatedComposeRejection::IncompatibleSemanticState(
                    child_path.to_vec(),
                ));
            }
            Some(state)
        } else {
            if standard_button
                && !matches!(
                    semantic,
                    AuthenticatedElementSemantic::ReadOnlyButton { .. }
                )
            {
                return Err(AuthenticatedComposeRejection::IncompatibleSemanticState(
                    child_path.to_vec(),
                ));
            }
            None
        };
        let sprite_name = role
            .sprites
            .first()
            .map(|sprite| sprite.sprite_name.as_str());
        let authenticated_sprite = role
            .sprites
            .first()
            .map(|sprite_role| {
                self.authenticated_sprite_definition(admission, sprite_role, child_path)
            })
            .transpose()?;
        let natural = self.natural_size_authenticated(
            element,
            authenticated_sprite.as_ref(),
            role.sprites.first(),
            child_path,
            admission,
            &index.index_identity_sha256,
        )?;
        let rect = if (current_instance_ordinal.is_some() && child_path.is_empty())
            || (child_path.is_empty()
                && element.position.is_none()
                && element.size.is_none()
                && element.orientation.is_none())
        {
            parent
        } else {
            resolve_authenticated_open_rect(element, parent, natural, child_path.is_empty())
                .map_err(|field| {
                    AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                        child_path: child_path.to_vec(),
                        field,
                    }
                })?
        };
        let element_clip = intersect_rect(required_containment, rect)
            .map(|clip| authenticated_content_clip(element, clip, child_path))
            .transpose()?;
        let element_census_index = elements.len();
        elements.push(AuthenticatedComposedElement {
            child_path: child_path.to_vec(),
            element_id: role.element_id.clone(),
            authored_name: role.authored_name.clone(),
            element_type: role.element_type.clone(),
            visible: true,
            screen_rect: Some(authenticated_element_rect(rect)),
            clip_rect: element_clip.map(authenticated_element_rect),
            paint_ordinal: None,
        });
        debug_rects.push(DebugRect {
            name: element.name.clone(),
            element_type: element.element_type.clone(),
            child_path: child_path.to_vec(),
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
            depth,
        });

        if options.sprite_overrides.contains_key(&element.name) {
            return Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                child_path.to_vec(),
            ));
        }
        if let Some(element_clip) = element_clip {
            match semantic {
                AuthenticatedElementSemantic::Hidden => {
                    unreachable!("hidden semantics return before geometry")
                }
                AuthenticatedElementSemantic::Structural => {}
                AuthenticatedElementSemantic::StandardButton
                | AuthenticatedElementSemantic::ReadOnlyButton { .. }
                | AuthenticatedElementSemantic::PassiveSprite { .. }
                | AuthenticatedElementSemantic::BlendFrames { .. } => {
                    let sprite_name = sprite_name.ok_or_else(|| {
                        AuthenticatedComposeRejection::SpriteDefinitionMismatch(child_path.to_vec())
                    })?;
                    let paint_ordinal = u32::try_from(report.drawn_elements)
                        .map_err(|_| AuthenticatedComposeRejection::RootIdentityMismatch)?;
                    self.draw_sprite_authenticated(
                        element,
                        child_path,
                        manager,
                        frame_authority,
                        sprite_name,
                        authenticated_sprite.as_ref().ok_or_else(|| {
                            AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                                child_path.to_vec(),
                            )
                        })?,
                        rect,
                        image,
                        report,
                        index,
                        admission,
                        panel,
                        runtime_owner_panel,
                        panel_lease,
                        widget_state,
                        semantic,
                        required_containment,
                    )?;
                    elements[element_census_index].paint_ordinal = Some(paint_ordinal);
                }
                AuthenticatedElementSemantic::Text { resolved_text } => {
                    let paint_ordinal = u32::try_from(report.drawn_elements)
                        .map_err(|_| AuthenticatedComposeRejection::RootIdentityMismatch)?;
                    self.draw_text_authenticated(
                        element,
                        role,
                        child_path,
                        resolved_text,
                        rect,
                        element_clip,
                        image,
                        report,
                        manager,
                        frame_authority,
                        index,
                        admission,
                        runtime_owner_panel,
                        panel_lease,
                    )?;
                    elements[element_census_index].paint_ordinal = Some(paint_ordinal);
                }
                AuthenticatedElementSemantic::Progress { fill } => {
                    let paint_ordinal = u32::try_from(report.drawn_elements)
                        .map_err(|_| AuthenticatedComposeRejection::RootIdentityMismatch)?;
                    self.draw_progress_authenticated(
                        element,
                        role,
                        child_path,
                        *fill,
                        rect,
                        element_clip,
                        image,
                        report,
                        admission,
                        &index.index_identity_sha256,
                    )?;
                    elements[element_census_index].paint_ordinal = Some(paint_ordinal);
                }
            }
        }

        let child_containment = match (element_clip, authored_clipping_mode(element, child_path)?) {
            (None, _) => Rect::new(0, 0, 0, 0),
            (Some(_), AuthoredClippingMode::No) => required_containment,
            (Some(element_clip), AuthoredClippingMode::YesOrUnspecified) => element_clip,
        };
        for insertion_ordinal in 0..=element.children.len() {
            if let Some(plan) = materialization_plan {
                self.draw_materialized_instances_at(
                    plan,
                    current_instance_ordinal,
                    child_path,
                    u32::try_from(insertion_ordinal)
                        .map_err(|_| AuthenticatedComposeRejection::MaterializationIdentity)?,
                    manager,
                    frame_authority,
                    image,
                    options,
                    report,
                    debug_rects,
                    index,
                    admission,
                    runtime_owner_panel,
                    panel_lease,
                    materialized_census,
                    depth + 1,
                )?;
            }
            let Some(child) = element.children.get(insertion_ordinal) else {
                continue;
            };
            let ordinal = u32::try_from(insertion_ordinal)
                .map_err(|_| AuthenticatedComposeRejection::RootIdentityMismatch)?;
            let mut path = child_path.to_vec();
            path.push(ordinal);
            self.draw_element_authenticated(
                child,
                rect,
                depth + 1,
                &path,
                manager,
                frame_authority,
                image,
                options,
                report,
                debug_rects,
                index,
                admission,
                panel,
                runtime_owner_panel,
                panel_lease,
                widget_states,
                semantics,
                elements,
                materialization_plan,
                current_instance_ordinal,
                materialized_census,
                child_containment,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_materialized_instances_at(
        &mut self,
        plan: &AuthenticatedMaterializationPlan,
        host_instance_ordinal: Option<u32>,
        host_child_path: &[u32],
        insertion_ordinal: u32,
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        image: &mut RgbaImage,
        options: &ComposeOptions,
        report: &mut ComposeReport,
        debug_rects: &mut Vec<DebugRect>,
        index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        runtime_owner_panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        panel_lease: &AuthenticatedReplayPanelLease<'_>,
        materialized_census: &mut Vec<AuthenticatedMaterializedInstanceCensus>,
        depth: usize,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        let mut matching = plan
            .instances
            .iter()
            .enumerate()
            .filter(|(_, instance)| {
                if host_instance_ordinal == Some(u32::MAX) {
                    instance.global_root_sibling
                } else {
                    !instance.global_root_sibling
                        && instance.host_instance_ordinal == host_instance_ordinal
                        && instance.host_child_path == host_child_path
                        && instance.owner_child_insertion_ordinal == insertion_ordinal
                }
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if host_instance_ordinal == Some(u32::MAX) {
            matching.sort_by_key(|index| {
                let instance = &plan.instances[*index];
                (
                    u8::from(!instance.viewer_actions.is_empty()),
                    instance.instance_ordinal,
                )
            });
        }
        for instance_index in matching {
            let instance = &plan.instances[instance_index];
            if materialized_census
                .iter()
                .any(|drawn| drawn.rendered_root_path.instance_ordinal == instance.instance_ordinal)
            {
                return Err(AuthenticatedComposeRejection::MaterializationIdentity);
            }
            let template_surface = index
                .validate_panel_token(admission, &instance.template_panel)
                .map_err(|_| AuthenticatedComposeRejection::MaterializationIdentity)?;
            let semantics = AuthenticatedPanelSemanticCensus {
                index_identity_sha256: index.index_identity_sha256.clone(),
                window_id: instance.template_panel.window_id().to_string(),
                surface_id: template_surface.surface_id.clone(),
                source_content: template_surface.source_content.clone(),
                runtime_instance_token: panel_lease.runtime_instance_token().to_string(),
                frame_generation: frame_authority.frame_generation(),
                elements: instance.semantics.clone(),
            };
            let mut elements = Vec::with_capacity(template_surface.elements.len());
            let debug_start = debug_rects.len();
            let empty_widget_states = BTreeMap::new();
            let census_index = materialized_census.len();
            materialized_census.push(AuthenticatedMaterializedInstanceCensus {
                instance_key: instance.instance_key.clone(),
                template_window_id: instance.template_panel.window_id().to_string(),
                template_surface_id: instance.template_panel.surface_id().to_string(),
                template_source_content: instance.template_panel.source_content().clone(),
                template_tree_identity_sha256: instance
                    .template_panel
                    .authored_tree_identity_sha256()
                    .to_string(),
                rendered_root_path: AuthenticatedRenderedInstancePath {
                    instance_ordinal: instance.instance_ordinal,
                    template_child_path: Vec::new(),
                },
                elements: Vec::new(),
            });
            self.draw_element_authenticated(
                instance.template_panel.authored_root(),
                rect_from_authenticated(instance.parent_rect),
                depth,
                &[],
                manager,
                frame_authority,
                image,
                options,
                report,
                debug_rects,
                index,
                admission,
                &instance.template_panel,
                runtime_owner_panel,
                panel_lease,
                &empty_widget_states,
                &semantics,
                &mut elements,
                Some(plan),
                Some(instance.instance_ordinal),
                materialized_census,
                rect_from_authenticated(instance.required_clip),
            )?;
            for debug in &mut debug_rects[debug_start..] {
                let mut path = Vec::with_capacity(debug.child_path.len() + 2);
                path.push(u32::MAX);
                path.push(instance.instance_ordinal);
                path.extend_from_slice(&debug.child_path);
                debug.child_path = path;
            }
            let elements = elements
                .into_iter()
                .map(|element| AuthenticatedMaterializedInstanceElement {
                    rendered_path: AuthenticatedRenderedInstancePath {
                        instance_ordinal: instance.instance_ordinal,
                        template_child_path: element.child_path.clone(),
                    },
                    element,
                })
                .collect();
            materialized_census[census_index].elements = elements;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text_authenticated(
        &mut self,
        element: &GuiElement,
        role: &crate::authored_role_index::AuthoredElementRole,
        child_path: &[u32],
        resolved_text: &str,
        rect: Rect,
        clip: Rect,
        image: &mut RgbaImage,
        report: &mut ComposeReport,
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        runtime_owner_panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        panel_lease: &AuthenticatedReplayPanelLease<'_>,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        if resolved_text.is_empty() {
            return Err(AuthenticatedComposeRejection::IncompatibleSemanticState(
                child_path.to_vec(),
            ));
        }
        let font_name = element.font.as_deref().ok_or_else(|| {
            AuthenticatedComposeRejection::UnsupportedTextVariant(child_path.to_vec())
        })?;
        let font_role = role.font.as_ref().ok_or_else(|| {
            AuthenticatedComposeRejection::UnsupportedTextVariant(child_path.to_vec())
        })?;
        if role.font_ref.as_deref() != Some(font_name)
            || font_role.font_id != font_name
            || admission
                .require_used_content(&font_role.text_effect_content.normalized_game_relative_path)
                .ok()
                != Some(&font_role.text_effect_content)
            || self
                .resolver
                .admitted_content_key(&font_role.text_effect_content.normalized_game_relative_path)
                != Some(&font_role.text_effect_content)
        {
            return Err(AuthenticatedComposeRejection::UnsupportedTextVariant(
                child_path.to_vec(),
            ));
        }
        let (_, shader_bytes) =
            self.read_verified_content_bytes(&font_role.text_effect_content, child_path)?;
        if !text_effect_row_is_the_authored_shader(
            &font_role.text_effect_content,
            shader_bytes.len(),
        ) {
            return Err(AuthenticatedComposeRejection::UnsupportedTextVariant(
                child_path.to_vec(),
            ));
        }
        let font = self
            .authenticated_font(font_role, admission)
            .map_err(|error| AuthenticatedComposeRejection::TextDraw {
                child_path: child_path.to_vec(),
                message: error.message,
            })?;
        if font.authored_base_color_rgba8
            != font_role
                .base_color
                .as_ref()
                .map(crate::authored_role_index::AuthoredPackedColorRole::rgba8)
            || font.authored_base_border_color_rgba8
                != font_role
                    .base_border_color
                    .as_ref()
                    .map(crate::authored_role_index::AuthoredPackedColorRole::rgba8)
        {
            return Err(AuthenticatedComposeRejection::UnsupportedTextVariant(
                child_path.to_vec(),
            ));
        }
        let base_color = font.authored_base_color_rgba8.unwrap_or([0, 0, 0, 0]);
        let (icon_metrics, inline_icons) = self.resolve_authenticated_inline_icons(
            index,
            admission,
            &font,
            resolved_text,
            child_path,
        )?;
        let border = authenticated_text_border(element, child_path)?;
        let layout =
            layout_engine_text_with_icons(&font, resolved_text, rect.w, border, &icon_metrics)
                .map_err(|error| AuthenticatedComposeRejection::TextDraw {
                    child_path: child_path.to_vec(),
                    message: format!("authenticated engine text layout refused: {error}"),
                })?;
        let rich = font.colorize(resolved_text, base_color);
        let align = TextAlign::parse(element.format.as_deref());
        let clip_pixels = Some([clip.x, clip.y, clip.w, clip.h]);
        for glyph in &layout.glyphs {
            let line = layout.lines.get(glyph.line_index).ok_or_else(|| {
                AuthenticatedComposeRejection::TextDraw {
                    child_path: child_path.to_vec(),
                    message: "authenticated glyph names an absent text line".to_string(),
                }
            })?;
            let x = rect.x.saturating_add(glyph.draw_x).saturating_add(
                authenticated_text_line_alignment(rect.w, line.width, border, align)?,
            );
            let y = rect.y.saturating_add(glyph.draw_y);
            let tint = rich
                .colors
                .get(glyph.visible_char_index)
                .copied()
                .unwrap_or(base_color);
            let drawn = font
                .draw_runtime_glyph_a729(image, glyph.ch as u32, x, y, tint, clip_pixels)
                .map_err(|reason| authenticated_text_shader_rejection(child_path, reason))?;
            if drawn != RuntimeGlyphDraw::Drawn {
                return Err(AuthenticatedComposeRejection::TextDraw {
                    child_path: child_path.to_vec(),
                    message: format!("authenticated font lost glyph {:?}", glyph.ch),
                });
            }
        }
        if layout.inline_icons.len() != inline_icons.len() {
            return Err(AuthenticatedComposeRejection::TextDraw {
                child_path: child_path.to_vec(),
                message: "authenticated inline-icon layout/result count differs".to_string(),
            });
        }
        for (placed, prepared) in layout.inline_icons.iter().zip(&inline_icons) {
            if placed.sprite != prepared.sprite_name || placed.frame != prepared.zero_based_frame {
                return Err(AuthenticatedComposeRejection::TextDraw {
                    child_path: child_path.to_vec(),
                    message: "authenticated inline-icon layout/result identity differs".to_string(),
                });
            }
            let line = layout.lines.get(placed.line_index).ok_or_else(|| {
                AuthenticatedComposeRejection::TextDraw {
                    child_path: child_path.to_vec(),
                    message: "authenticated inline icon names an absent text line".to_string(),
                }
            })?;
            let x =
                rect.x
                    .saturating_add(placed.x)
                    .saturating_add(authenticated_text_line_alignment(
                        rect.w, line.width, border, align,
                    )?);
            let y = rect
                .y
                .saturating_add(placed.line_y)
                .saturating_sub(placed.engine_center_offset_y);
            let admitted = admit_authenticated_dynamic_sprite_runtime_graphics(
                manager,
                frame_authority,
                index,
                admission,
                runtime_owner_panel,
                panel_lease,
                &prepared.sprite_name,
                prepared.zero_based_frame.checked_add(1).ok_or_else(|| {
                    AuthenticatedComposeRejection::TextDraw {
                        child_path: child_path.to_vec(),
                        message: "authenticated inline-icon frame overflow".to_string(),
                    }
                })?,
                ClipRect {
                    left: clip.x,
                    top: clip.y,
                    right: clip.x.saturating_add(clip.w),
                    bottom: clip.y.saturating_add(clip.h),
                },
            )
            .map_err(|reason| AuthenticatedComposeRejection::RuntimeGraphics {
                child_path: child_path.to_vec(),
                reason,
            })?;
            draw_authenticated_sprite_pixels(
                image,
                AuthenticatedSpriteDraw::new(
                    &prepared.atlas,
                    prepared.frame_count,
                    AuthenticatedSpriteGeometry {
                        x,
                        y,
                        width: usize::try_from(placed.advance).map_err(|_| {
                            AuthenticatedComposeRejection::UnsupportedTextVariant(
                                child_path.to_vec(),
                            )
                        })?,
                        height: usize::try_from(placed.height).map_err(|_| {
                            AuthenticatedComposeRejection::UnsupportedTextVariant(
                                child_path.to_vec(),
                            )
                        })?,
                        border: prepared.border,
                    },
                    admitted,
                ),
            )
            .map_err(|reason| AuthenticatedComposeRejection::RuntimeGraphics {
                child_path: child_path.to_vec(),
                reason,
            })?;
        }
        report.drawn_elements = report
            .drawn_elements
            .checked_add(1)
            .ok_or(AuthenticatedComposeRejection::RootIdentityMismatch)?;
        Ok(())
    }

    fn resolve_authenticated_inline_icons(
        &mut self,
        index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        font: &BitmapFont,
        text: &str,
        child_path: &[u32],
    ) -> std::result::Result<
        (
            Vec<ResolvedInlineIconMetrics>,
            Vec<AuthenticatedInlineIconDraw>,
        ),
        AuthenticatedComposeRejection,
    > {
        let empty_colors = crate::markup::TextColorTable::default();
        let colors = font.color_table.as_ref().unwrap_or(&empty_colors);
        let mut metrics = Vec::new();
        let mut prepared = Vec::new();
        for token in crate::markup::parse(text, colors) {
            let (key, declared_frame) = match token {
                crate::markup::Token::Icon { key, frame } => (key, frame),
                crate::markup::Token::Variable { key } => {
                    return Err(AuthenticatedComposeRejection::TextDraw {
                        child_path: child_path.to_vec(),
                        message: format!(
                            "unresolved localisation variable ${key}$ reached authenticated text"
                        ),
                    })
                }
                crate::markup::Token::DataFunction { expr } => {
                    return Err(AuthenticatedComposeRejection::TextDraw {
                        child_path: child_path.to_vec(),
                        message: format!(
                            "unresolved data function [{expr}] reached authenticated text"
                        ),
                    })
                }
                crate::markup::Token::Text { .. } => continue,
            };
            let zero_based_frame = declared_frame.unwrap_or(0);
            let metric_token = match declared_frame {
                Some(frame) => format!("{key}|{frame}"),
                None => key.clone(),
            };
            let sprite_name = if key.starts_with("GFX_") {
                key.clone()
            } else {
                format!("GFX_{key}")
            };
            let sprite_role = index
                .admit_dynamic_sprite(admission, &sprite_name)
                .map_err(|message| AuthenticatedComposeRejection::TextDraw {
                    child_path: child_path.to_vec(),
                    message,
                })?
                .clone();
            if sprite_role.texture_slots.len() != 1 {
                return Err(
                    AuthenticatedComposeRejection::UnsupportedMultiTextureVariant(
                        child_path.to_vec(),
                    ),
                );
            }
            let definition =
                self.authenticated_dynamic_sprite_definition(admission, &sprite_role, child_path)?;
            let decoded = self
                .decoded_authenticated_sprite(
                    &definition,
                    &sprite_role,
                    admission,
                    &index.index_identity_sha256,
                )
                .map_err(|error| AuthenticatedComposeRejection::SpriteDecode {
                    child_path: child_path.to_vec(),
                    message: error.message,
                })?;
            let frame = decoded
                .frames
                .get(usize::try_from(zero_based_frame).map_err(|_| {
                    AuthenticatedComposeRejection::UnsupportedAnimatedVariant(child_path.to_vec())
                })?)
                .ok_or_else(|| {
                    AuthenticatedComposeRejection::UnsupportedAnimatedVariant(child_path.to_vec())
                })?;
            let advance = i32::try_from(frame.width).map_err(|_| {
                AuthenticatedComposeRejection::UnsupportedTextVariant(child_path.to_vec())
            })?;
            let height = i32::try_from(frame.height).map_err(|_| {
                AuthenticatedComposeRejection::UnsupportedTextVariant(child_path.to_vec())
            })?;
            metrics.push(ResolvedInlineIconMetrics {
                token: metric_token,
                sprite: sprite_name.clone(),
                frame: zero_based_frame,
                advance,
                height,
            });
            let border = definition
                .border_size
                .as_ref()
                .map(parse_authenticated_sprite_size)
                .transpose()
                .map_err(|message| AuthenticatedComposeRejection::SpriteDecode {
                    child_path: child_path.to_vec(),
                    message,
                })?;
            prepared.push(AuthenticatedInlineIconDraw {
                sprite_name,
                zero_based_frame,
                frame_count: decoded.frames.len(),
                atlas: Arc::new(decoded.texture.image.clone()),
                border,
            });
        }
        Ok((metrics, prepared))
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_progress_authenticated(
        &mut self,
        element: &GuiElement,
        role: &crate::authored_role_index::AuthoredElementRole,
        child_path: &[u32],
        fill: ProgressFill,
        rect: Rect,
        clip: Rect,
        image: &mut RgbaImage,
        report: &mut ComposeReport,
        admission: &ValidatedContentAdmission,
        index_identity_sha256: &str,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        let sprite_role = role.sprites.first().ok_or_else(|| {
            AuthenticatedComposeRejection::SpriteDefinitionMismatch(child_path.to_vec())
        })?;
        if role.sprites.len() != 1
            || sprite_role.texture_slots.len() != 2
            || element
                .raw
                .first_scalar_ci("scale")
                .is_some_and(|value| value.trim() != "1" && value.trim() != "1.0")
        {
            return Err(
                AuthenticatedComposeRejection::UnsupportedMultiTextureVariant(child_path.to_vec()),
            );
        }
        let foreground = self
            .decoded_authenticated_texture(
                &sprite_role.texture_slots[0].content,
                admission,
                index_identity_sha256,
                child_path,
            )
            .map_err(|error| AuthenticatedComposeRejection::SpriteDecode {
                child_path: child_path.to_vec(),
                message: error.message,
            })?;
        let background = self
            .decoded_authenticated_texture(
                &sprite_role.texture_slots[1].content,
                admission,
                index_identity_sha256,
                child_path,
            )
            .map_err(|error| AuthenticatedComposeRejection::SpriteDecode {
                child_path: child_path.to_vec(),
                message: error.message,
            })?;
        let geometry = AuthenticatedSpriteGeometry {
            x: rect.x,
            y: rect.y,
            width: usize::try_from(rect.w).map_err(|_| {
                AuthenticatedComposeRejection::UnsupportedLayoutVariant(child_path.to_vec())
            })?,
            height: usize::try_from(rect.h).map_err(|_| {
                AuthenticatedComposeRejection::UnsupportedLayoutVariant(child_path.to_vec())
            })?,
            border: None,
        };
        let admitted = admit_authenticated_progress_draw(
            sprite_role,
            &foreground,
            &background,
            geometry,
            ClipRect {
                left: clip.x,
                top: clip.y,
                right: clip.x.saturating_add(clip.w),
                bottom: clip.y.saturating_add(clip.h),
            },
            fill,
        )
        .map_err(|reason| AuthenticatedComposeRejection::RuntimeGraphics {
            child_path: child_path.to_vec(),
            reason,
        })?;
        draw_authenticated_progress_pixels(image, admitted).map_err(|reason| {
            AuthenticatedComposeRejection::RuntimeGraphics {
                child_path: child_path.to_vec(),
                reason,
            }
        })?;
        report.drawn_elements = report
            .drawn_elements
            .checked_add(1)
            .ok_or(AuthenticatedComposeRejection::RootIdentityMismatch)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_authenticated_extended_scrollbar(
        &mut self,
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        image: &mut RgbaImage,
        report: &mut ComposeReport,
        index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        owner_panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        panel_lease: &AuthenticatedReplayPanelLease<'_>,
        scrollbar: &AuthenticatedExtendedScrollbarPlan,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        validate_extended_scrollbar_plan(index, admission, Some(scrollbar))?;
        for (ordinal, part) in [
            &scrollbar.background,
            &scrollbar.track,
            &scrollbar.decrease,
            &scrollbar.increase,
            &scrollbar.slider,
        ]
        .into_iter()
        .enumerate()
        {
            let child_path = [u32::MAX, u32::MAX, ordinal as u32];
            let role = index
                .admit_dynamic_sprite(admission, &part.sprite_name)
                .map_err(|_| AuthenticatedComposeRejection::MaterializationIdentity)?;
            let sprite =
                self.authenticated_dynamic_sprite_definition(admission, role, &child_path)?;
            let decoded = self
                .decoded_authenticated_sprite(
                    &sprite,
                    role,
                    admission,
                    &index.index_identity_sha256,
                )
                .map_err(|error| AuthenticatedComposeRejection::SpriteDecode {
                    child_path: child_path.to_vec(),
                    message: error.message,
                })?;
            let rect = part.rect;
            let clip = scrollbar.clip_rect;
            let admitted = admit_authenticated_dynamic_sprite_runtime_graphics(
                manager,
                frame_authority,
                index,
                admission,
                owner_panel,
                panel_lease,
                &part.sprite_name,
                1,
                ClipRect {
                    left: clip.x(),
                    top: clip.y(),
                    right: clip.x().saturating_add(clip.width()),
                    bottom: clip.y().saturating_add(clip.height()),
                },
            )
            .map_err(|reason| AuthenticatedComposeRejection::RuntimeGraphics {
                child_path: child_path.to_vec(),
                reason,
            })?;
            let border = sprite
                .border_size
                .as_ref()
                .map(parse_authenticated_sprite_size)
                .transpose()
                .map_err(|message| AuthenticatedComposeRejection::SpriteDecode {
                    child_path: child_path.to_vec(),
                    message,
                })?;
            draw_authenticated_sprite_pixels(
                image,
                AuthenticatedSpriteDraw::new(
                    &decoded.texture.image,
                    decoded.frames.len(),
                    AuthenticatedSpriteGeometry {
                        x: rect.x(),
                        y: rect.y(),
                        width: usize::try_from(rect.width())
                            .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?,
                        height: usize::try_from(rect.height())
                            .map_err(|_| AuthenticatedComposeRejection::MaterializationGeometry)?,
                        border,
                    },
                    admitted,
                ),
            )
            .map_err(|reason| AuthenticatedComposeRejection::RuntimeGraphics {
                child_path: child_path.to_vec(),
                reason,
            })?;
            report.drawn_elements = report
                .drawn_elements
                .checked_add(1)
                .ok_or(AuthenticatedComposeRejection::MaterializationGeometry)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_sprite_authenticated(
        &mut self,
        element: &GuiElement,
        child_path: &[u32],
        manager: &ReplayOwnedWindowManager,
        frame_authority: &AuthenticatedReplayFrameAuthority,
        sprite_name: &str,
        sprite: &SpriteDefinition,
        rect: Rect,
        image: &mut RgbaImage,
        report: &mut ComposeReport,
        index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        runtime_owner_panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
        panel_lease: &AuthenticatedReplayPanelLease<'_>,
        widget_state: Option<&AuthenticatedViewerWidgetState>,
        semantic: &AuthenticatedElementSemantic,
        required_containment: Rect,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        let surface = index
            .validate_panel_token(admission, panel)
            .map_err(|_| AuthenticatedComposeRejection::PanelAuthentication)?;
        let role = surface
            .elements
            .iter()
            .find(|role| role.child_path == child_path)
            .ok_or_else(|| {
                AuthenticatedComposeRejection::UnexpectedRuntimeGraphics(child_path.to_vec())
            })?;
        let sprite_role = role
            .sprites
            .iter()
            .find(|sprite| sprite.sprite_name.eq_ignore_ascii_case(sprite_name))
            .ok_or_else(|| {
                AuthenticatedComposeRejection::SpriteDefinitionMismatch(child_path.to_vec())
            })?;
        let definition_matches = sprite
            .source_file
            .eq_ignore_ascii_case(&sprite_role.definition_content.normalized_game_relative_path)
            && sprite.no_of_frames == sprite_role.frame_count
            && sprite.texture_files.len() == sprite_role.texture_slots.len()
            && sprite
                .texture_files
                .iter()
                .zip(&sprite_role.texture_slots)
                .all(|(slot, texture_role)| {
                    matches!(
                        portable_content_manifest::normalize_game_relative_path(&slot.path),
                        Ok(authored) if authored.eq_ignore_ascii_case(&texture_role.authored_path)
                    ) && crate::assets::texture_slot_paths_match(
                        &texture_role.authored_path,
                        &texture_role.content.normalized_game_relative_path,
                    )
                });
        if !definition_matches {
            return Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                child_path.to_vec(),
            ));
        }
        if sprite.raw.first_value_ci("animation").is_some() {
            return Err(AuthenticatedComposeRejection::UnsupportedAnimatedVariant(
                child_path.to_vec(),
            ));
        }
        if sprite.kind.to_ascii_lowercase().contains("masked")
            || raw_contains_mask_carrier(&sprite.raw)
        {
            return Err(AuthenticatedComposeRejection::UnsupportedMaskingVariant(
                child_path.to_vec(),
            ));
        }
        if sprite.texture_files.len() != 1 {
            return Err(
                AuthenticatedComposeRejection::UnsupportedMultiTextureVariant(child_path.to_vec()),
            );
        }
        if sprite.border_size.is_some() {
            return Err(
                AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(
                    child_path.to_vec(),
                ),
            );
        }
        if raw_has_top_level_fragment(&element.raw, &["color", "alpha", "opacity", "clip"]) {
            return Err(
                AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(
                    child_path.to_vec(),
                ),
            );
        }
        let image_width = i32::try_from(image.width).map_err(|_| {
            AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(child_path.to_vec())
        })?;
        let image_height = i32::try_from(image.height).map_err(|_| {
            AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(child_path.to_vec())
        })?;
        let clip = intersect_rect(
            required_containment,
            Rect {
                x: 0,
                y: 0,
                w: image_width,
                h: image_height,
            },
        )
        .map(|clip| ClipRect {
            left: clip.x,
            top: clip.y,
            right: clip.x.saturating_add(clip.w),
            bottom: clip.y.saturating_add(clip.h),
        })
        .ok_or_else(|| {
            AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(child_path.to_vec())
        })?;
        let standard_button = WidgetFamily::from_element_type(&element.element_type)
            .is_some_and(WidgetFamily::standard_button_carrier);
        let admitted = if standard_button
            && matches!(semantic, AuthenticatedElementSemantic::StandardButton)
        {
            if panel.window_id() != runtime_owner_panel.window_id() {
                return Err(AuthenticatedComposeRejection::MissingRuntimeGraphics(
                    child_path.to_vec(),
                ));
            }
            let state = widget_state.ok_or_else(|| {
                AuthenticatedComposeRejection::MissingRuntimeGraphics(child_path.to_vec())
            })?;
            if !panel_lease.remains_current()
                || state.runtime_instance_token() != panel_lease.runtime_instance_token()
            {
                return Err(AuthenticatedComposeRejection::StaleRuntimeInstance(
                    child_path.to_vec(),
                ));
            }
            let graphics = admit_authenticated_widget_graphics_state(
                manager,
                frame_authority,
                index,
                admission,
                panel,
                panel_lease,
                state,
            )
            .map_err(|_| {
                AuthenticatedComposeRejection::StaleRuntimeInstance(child_path.to_vec())
            })?;
            admit_authenticated_widget_runtime_graphics(
                index,
                admission,
                panel,
                panel_lease,
                &graphics,
                sprite_name,
                clip,
            )
        } else {
            if widget_state.is_some() {
                return Err(AuthenticatedComposeRejection::UnexpectedRuntimeGraphics(
                    child_path.to_vec(),
                ));
            }
            let (one_based_frame, next_one_based_frame, animation_time, interaction) =
                match semantic {
                    AuthenticatedElementSemantic::PassiveSprite { one_based_frame } => (
                        *one_based_frame,
                        None,
                        0.0,
                        crate::runtime_graphics::ButtonVisualState::Up,
                    ),
                    AuthenticatedElementSemantic::ReadOnlyButton {
                        one_based_frame,
                        interaction,
                    } => {
                        if !standard_button {
                            return Err(AuthenticatedComposeRejection::IncompatibleSemanticState(
                                child_path.to_vec(),
                            ));
                        }
                        (*one_based_frame, None, 0.0, *interaction)
                    }
                    AuthenticatedElementSemantic::BlendFrames {
                        current_one_based_frame,
                        next_one_based_frame,
                        animation_time,
                    } => (
                        *current_one_based_frame,
                        Some(*next_one_based_frame),
                        *animation_time,
                        crate::runtime_graphics::ButtonVisualState::Up,
                    ),
                    _ => {
                        return Err(
                            AuthenticatedComposeRejection::MissingAuthenticatedPassiveState(
                                child_path.to_vec(),
                            ),
                        )
                    }
                };
            admit_authenticated_semantic_runtime_graphics(
                manager,
                frame_authority,
                index,
                admission,
                panel,
                runtime_owner_panel,
                panel_lease,
                child_path,
                sprite_name,
                one_based_frame,
                next_one_based_frame,
                animation_time,
                interaction,
                clip,
            )
        }
        .map_err(|reason| AuthenticatedComposeRejection::RuntimeGraphics {
            child_path: child_path.to_vec(),
            reason,
        })?;
        let border = sprite
            .border_size
            .as_ref()
            .map(parse_authenticated_sprite_size)
            .transpose()
            .map_err(|message| AuthenticatedComposeRejection::SpriteDecode {
                child_path: child_path.to_vec(),
                message,
            })?;
        let decoded = self
            .decoded_authenticated_sprite(
                sprite,
                sprite_role,
                admission,
                &index.index_identity_sha256,
            )
            .map_err(|error| AuthenticatedComposeRejection::SpriteDecode {
                child_path: child_path.to_vec(),
                message: error.message,
            })?;
        let Some(first_frame) = decoded.frames.first() else {
            return Err(AuthenticatedComposeRejection::SpriteDecode {
                child_path: child_path.to_vec(),
                message: format!("sprite {} has no frames", sprite.name),
            });
        };
        let target = target_size(rect, first_frame.width, first_frame.height);
        let scale = scalar_f32_ci(&element.raw, "scale").unwrap_or(1.0);
        let scaled_width = (target.0 as f32) * scale;
        let scaled_height = (target.1 as f32) * scale;
        if !scaled_width.is_finite()
            || !scaled_height.is_finite()
            || scaled_width <= 0.0
            || scaled_height <= 0.0
            || scaled_width > i32::MAX as f32
            || scaled_height > i32::MAX as f32
        {
            return Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                child_path.to_vec(),
            ));
        }
        let target_width = scaled_width.round().max(1.0) as usize;
        let target_height = scaled_height.round().max(1.0) as usize;
        draw_authenticated_sprite_pixels(
            image,
            AuthenticatedSpriteDraw::new(
                &decoded.texture.image,
                decoded.frames.len(),
                AuthenticatedSpriteGeometry {
                    x: rect.x,
                    y: rect.y,
                    width: target_width,
                    height: target_height,
                    border,
                },
                admitted,
            ),
        )
        .map_err(|reason| AuthenticatedComposeRejection::RuntimeGraphics {
            child_path: child_path.to_vec(),
            reason,
        })?;
        report.drawn_elements += 1;
        Ok(())
    }

    fn natural_size_authenticated(
        &mut self,
        element: &GuiElement,
        sprite: Option<&SpriteDefinition>,
        sprite_role: Option<&crate::authored_role_index::AuthoredSpriteRole>,
        child_path: &[u32],
        admission: &ValidatedContentAdmission,
        index_identity_sha256: &str,
    ) -> std::result::Result<(usize, usize), AuthenticatedComposeRejection> {
        if element.size.is_some() {
            return Ok((0, 0));
        }
        let Some(sprite) = sprite else {
            return Ok((0, 0));
        };
        let Some(sprite_role) = sprite_role else {
            return Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                child_path.to_vec(),
            ));
        };
        if let Some(size) = &sprite.size {
            return parse_authenticated_sprite_size(size).map_err(|message| {
                AuthenticatedComposeRejection::SpriteDecode {
                    child_path: child_path.to_vec(),
                    message,
                }
            });
        }
        if sprite_role.texture_slots.len() == 2 {
            let first = self
                .decoded_authenticated_texture(
                    &sprite_role.texture_slots[0].content,
                    admission,
                    index_identity_sha256,
                    child_path,
                )
                .map_err(|error| AuthenticatedComposeRejection::SpriteDecode {
                    child_path: child_path.to_vec(),
                    message: error.message,
                })?;
            return Ok((first.width, first.height));
        }
        let decoded = self
            .decoded_authenticated_sprite(sprite, sprite_role, admission, index_identity_sha256)
            .map_err(|error| AuthenticatedComposeRejection::SpriteDecode {
                child_path: child_path.to_vec(),
                message: error.message,
            })?;
        decoded
            .frames
            .first()
            .map(|frame| (frame.width, frame.height))
            .ok_or_else(|| AuthenticatedComposeRejection::SpriteDecode {
                child_path: child_path.to_vec(),
                message: format!("sprite {} has no frames", sprite.name),
            })
    }

    fn authenticated_sprite_definition(
        &self,
        admission: &ValidatedContentAdmission,
        sprite_role: &crate::authored_role_index::AuthoredSpriteRole,
        child_path: &[u32],
    ) -> std::result::Result<SpriteDefinition, AuthenticatedComposeRejection> {
        self.authenticated_sprite_definition_with_mode(admission, sprite_role, child_path, true)
    }

    fn authenticated_dynamic_sprite_definition(
        &self,
        admission: &ValidatedContentAdmission,
        sprite_role: &crate::authored_role_index::AuthoredSpriteRole,
        child_path: &[u32],
    ) -> std::result::Result<SpriteDefinition, AuthenticatedComposeRejection> {
        self.authenticated_sprite_definition_with_mode(admission, sprite_role, child_path, false)
    }

    fn authenticated_sprite_definition_with_mode(
        &self,
        admission: &ValidatedContentAdmission,
        sprite_role: &crate::authored_role_index::AuthoredSpriteRole,
        child_path: &[u32],
        static_element_subset: bool,
    ) -> std::result::Result<SpriteDefinition, AuthenticatedComposeRejection> {
        let reject =
            || AuthenticatedComposeRejection::SpriteDefinitionMismatch(child_path.to_vec());
        for content in std::iter::once(&sprite_role.definition_content)
            .chain(sprite_role.texture_slots.iter().map(|slot| &slot.content))
            .chain(std::iter::once(&sprite_role.effect_content))
            .chain(&sprite_role.effect_include_content)
        {
            if admission
                .require_used_content(&content.normalized_game_relative_path)
                .ok()
                != Some(content)
                || self
                    .resolver
                    .admitted_content_key(&content.normalized_game_relative_path)
                    != Some(content)
            {
                return Err(reject());
            }
        }
        let (_, definition_bytes) =
            self.read_verified_content_bytes(&sprite_role.definition_content, child_path)?;
        self.read_verified_content_bytes(&sprite_role.effect_content, child_path)?;
        for include in &sprite_role.effect_include_content {
            self.read_verified_content_bytes(include, child_path)?;
        }
        if crate::paradox_text::destructive_event_count(&crate::paradox_text::diagnose_text_decode(
            &definition_bytes,
        )) != 0
        {
            return Err(reject());
        }
        let mut registry = SpriteRegistry::new();
        let document = crate::paradox_text::parse_bytes(&definition_bytes);
        registry.add_document(
            &sprite_role.definition_content.normalized_game_relative_path,
            &document,
        );
        if registry.decode_loss_events() != 0 {
            return Err(reject());
        }
        let mut matches = registry
            .sprites()
            .iter()
            .filter(|sprite| sprite.name.eq_ignore_ascii_case(&sprite_role.sprite_name));
        let sprite = matches.next().cloned().ok_or_else(reject)?;
        validate_authenticated_sprite_raw_fields(&sprite, child_path)?;
        if !sprite.kind.eq_ignore_ascii_case("spriteType") {
            return Err(reject());
        }
        if static_element_subset {
            validate_authenticated_sprite_executable_subset(&sprite, sprite_role, child_path)?;
        }
        if matches.next().is_some()
            || sprite.no_of_frames != sprite_role.frame_count
            || sprite.texture_files.len() != sprite_role.texture_slots.len()
            || sprite
                .texture_files
                .iter()
                .zip(&sprite_role.texture_slots)
                .any(|(slot, texture_role)| {
                    !matches!(
                        portable_content_manifest::normalize_game_relative_path(&slot.path),
                        Ok(authored) if authored.eq_ignore_ascii_case(&texture_role.authored_path)
                    ) || !crate::assets::texture_slot_paths_match(
                        &texture_role.authored_path,
                        &texture_role.content.normalized_game_relative_path,
                    )
                })
            || crate::gfx::a729_effect_shader_path(sprite.effect_file.as_deref())
                .ok()
                .as_deref()
                != Some(sprite_role.effective_shader_file.as_str())
            || crate::gfx::a729_sprite_color_rgba_f32_bits(&sprite.raw).ok()
                != Some(sprite_role.authored_color_rgba_f32_bits)
        {
            return Err(reject());
        }
        Ok(sprite)
    }

    fn read_verified_content_bytes(
        &self,
        content: &replay_core::UsedContentKey,
        child_path: &[u32],
    ) -> std::result::Result<(crate::assets::ResolvedFile, Vec<u8>), AuthenticatedComposeRejection>
    {
        let resolved = self
            .resolver
            .resolve_file(&content.normalized_game_relative_path)
            .map_err(|error| AuthenticatedComposeRejection::SpriteDecode {
                child_path: child_path.to_vec(),
                message: error.message,
            })?;
        let bytes = fs::read(&resolved.path).map_err(|error| {
            AuthenticatedComposeRejection::SpriteDecode {
                child_path: child_path.to_vec(),
                message: format!(
                    "failed to read admitted content {}: {error}",
                    content.normalized_game_relative_path
                ),
            }
        })?;
        if u64::try_from(bytes.len()).ok() != Some(content.payload_bytes)
            || Sha256::hex(&bytes) != content.payload_sha256
        {
            return Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                child_path.to_vec(),
            ));
        }
        Ok((resolved, bytes))
    }

    fn decoded_authenticated_sprite(
        &mut self,
        sprite: &SpriteDefinition,
        sprite_role: &crate::authored_role_index::AuthoredSpriteRole,
        admission: &ValidatedContentAdmission,
        index_identity_sha256: &str,
    ) -> Result<Arc<DecodedSprite>> {
        let cache_key =
            authenticated_sprite_cache_key(sprite, sprite_role, admission, index_identity_sha256)?;
        let texture_content = &sprite_role
            .texture_slots
            .first()
            .ok_or_else(|| {
                GameUiError::parse("authenticated sprite has no admitted texture payload")
            })?
            .content;
        let resolved = self
            .resolver
            .resolve_file(&texture_content.normalized_game_relative_path)?;
        let texture_bytes = fs::read(&resolved.path)?;
        if u64::try_from(texture_bytes.len()).ok() != Some(texture_content.payload_bytes)
            || Sha256::hex(&texture_bytes) != texture_content.payload_sha256
        {
            return Err(GameUiError::parse(format!(
                "authenticated texture payload changed after admission: {}",
                texture_content.normalized_game_relative_path
            )));
        }
        let cache = self.cache.get_mut();
        if !cache.sprites.contains_key(&cache_key) {
            let extension = resolved
                .path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let decoded = match extension.as_str() {
                "dds" => crate::dds::decode_bytes(&texture_bytes)
                    .map(|decoded| (decoded.format, decoded.image)),
                "tga" => crate::tga::decode_bytes(&texture_bytes)
                    .map(|decoded| (decoded.format, decoded.image)),
                "png" => crate::png::decode_bytes(&texture_bytes)
                    .map(|decoded| (decoded.format, decoded.image)),
                _ => Err(GameUiError::unsupported_texture_extension(
                    if extension.is_empty() {
                        "<none>"
                    } else {
                        extension.as_str()
                    },
                    format!(
                        "{} uses unsupported authenticated texture extension",
                        texture_content.normalized_game_relative_path
                    ),
                )),
            }
            .and_then(|(format, image)| {
                let frames = slice_horizontal_frames(&image, sprite.no_of_frames)?;
                Ok(DecodedSprite {
                    sprite_name: sprite.name.clone(),
                    texture: DecodedTexture {
                        rel_path: resolved.rel_path.clone(),
                        path: resolved.path.clone(),
                        format,
                        image,
                    },
                    frames,
                })
            })
            .map(Arc::new);
            cache.sprites.insert(cache_key.clone(), decoded);
        }
        cache
            .sprites
            .get(&cache_key)
            .expect("authenticated cache populated")
            .as_ref()
            .map(Arc::clone)
            .map_err(Clone::clone)
    }

    fn decoded_authenticated_texture(
        &mut self,
        content: &replay_core::UsedContentKey,
        admission: &ValidatedContentAdmission,
        index_identity_sha256: &str,
        child_path: &[u32],
    ) -> Result<Arc<RgbaImage>> {
        if admission
            .require_used_content(&content.normalized_game_relative_path)
            .ok()
            != Some(content)
            || self
                .resolver
                .admitted_content_key(&content.normalized_game_relative_path)
                != Some(content)
        {
            return Err(GameUiError::parse(
                "authenticated texture does not match the admitted content key",
            ));
        }
        let cache_key = serde_json::to_string(&(
            "authenticated-texture-v1",
            admission.manifest_digest_sha256(),
            admission.product_modset_sha256(),
            index_identity_sha256,
            content,
        ))
        .map_err(|error| {
            GameUiError::parse(format!(
                "failed to seal authenticated texture cache identity: {error}"
            ))
        })?;
        if !self.cache.get_mut().textures.contains_key(&cache_key) {
            let (resolved, bytes) = self
                .read_verified_content_bytes(content, child_path)
                .map_err(|error| GameUiError::parse(format!("{error:?}")))?;
            let extension = resolved
                .path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let decoded = match extension.as_str() {
                "dds" => crate::dds::decode_bytes(&bytes).map(|decoded| decoded.image),
                "tga" => crate::tga::decode_bytes(&bytes).map(|decoded| decoded.image),
                "png" => crate::png::decode_bytes(&bytes).map(|decoded| decoded.image),
                _ => Err(GameUiError::unsupported_texture_extension(
                    if extension.is_empty() {
                        "<none>"
                    } else {
                        extension.as_str()
                    },
                    format!(
                        "{} uses unsupported authenticated texture extension",
                        content.normalized_game_relative_path
                    ),
                )),
            }
            .map(Arc::new);
            self.cache
                .get_mut()
                .textures
                .insert(cache_key.clone(), decoded);
        }
        self.cache
            .get_mut()
            .textures
            .get(&cache_key)
            .expect("authenticated texture cache populated")
            .as_ref()
            .map(Arc::clone)
            .map_err(Clone::clone)
    }

    fn authenticated_font(
        &mut self,
        font_role: &crate::authored_role_index::AuthoredFontRole,
        admission: &ValidatedContentAdmission,
    ) -> Result<Arc<BitmapFont>> {
        let cache_key = serde_json::to_string(&(
            "authenticated-font-v1",
            admission.manifest_digest_sha256(),
            admission.product_modset_sha256(),
            font_role,
        ))
        .map_err(|error| {
            GameUiError::parse(format!(
                "failed to seal authenticated font cache identity: {error}"
            ))
        })?;
        if !self.cache.get_mut().fonts.contains_key(&cache_key) {
            let loaded =
                BitmapFont::load_from_authored_binding(self.resolver, font_role).map(Arc::new);
            self.cache.get_mut().fonts.insert(cache_key.clone(), loaded);
        }
        self.cache
            .get_mut()
            .fonts
            .get(&cache_key)
            .expect("authenticated font cache populated")
            .as_ref()
            .map(Arc::clone)
            .map_err(Clone::clone)
    }

    fn draw_element(
        &mut self,
        element: &GuiElement,
        parent: Rect,
        depth: usize,
        child_path: &[u32],
        image: &mut RgbaImage,
        options: &ComposeOptions,
        report: &mut ComposeReport,
        debug_rects: &mut Vec<DebugRect>,
    ) {
        if options.hidden_elements.contains(&element.name)
            || (is_hidden(element) && !options.shown_elements.contains(&element.name))
        {
            return;
        }
        let natural = self.natural_size(element, options);
        let mut rect = resolve_element_rect(element, parent, natural, options);
        if let Some([width, height]) = options.size_overrides.get(&element.name).copied() {
            rect.w = width.max(0);
            rect.h = height.max(0);
        }
        if let Some([x, y]) = options.position_overrides.get(&element.name).copied() {
            rect.x = parent.x + x;
            rect.y = parent.y + y;
        }
        if let Some(x) = options.position_x_overrides.get(&element.name).copied() {
            rect.x = parent.x + x;
        }
        if let Some(y) = options.position_y_overrides.get(&element.name).copied() {
            rect.y = parent.y + y;
        }
        debug_rects.push(DebugRect {
            name: element.name.clone(),
            element_type: element.element_type.clone(),
            child_path: child_path.to_vec(),
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
            depth,
        });

        if let Some(override_image) = options.sprite_overrides.get(&element.name) {
            draw_image(image, override_image, rect.x, rect.y);
            report.drawn_elements += 1;
        } else if element
            .element_type
            .eq_ignore_ascii_case("instantTextBoxType")
        {
            self.draw_text(element, rect, image, options, report);
        } else if let Some(sprite_name) = options
            .sprite_name_overrides
            .get(&element.name)
            .map(String::as_str)
            .or_else(|| primary_sprite_ref(element))
        {
            self.draw_sprite(element, sprite_name, rect, image, options, report);
        } else if let Some(rgba) = options.color_overrides.get(&element.name).copied() {
            fill_rect(image, rect, rgba);
            report.drawn_elements += 1;
        }

        for (ordinal, child) in element.children.iter().enumerate() {
            let ordinal =
                u32::try_from(ordinal).expect("authored child count cannot exceed address space");
            let mut path = child_path.to_vec();
            path.push(ordinal);
            self.draw_element(
                child,
                rect,
                depth + 1,
                &path,
                image,
                options,
                report,
                debug_rects,
            );
        }
    }

    fn draw_text(
        &mut self,
        element: &GuiElement,
        rect: Rect,
        image: &mut RgbaImage,
        options: &ComposeOptions,
        report: &mut ComposeReport,
    ) {
        let has_override = options.text_overrides.contains_key(&element.name);
        let mut text = options
            .text_overrides
            .get(&element.name)
            .cloned()
            .or_else(|| element.text.clone())
            .unwrap_or_default();
        if text.is_empty() {
            return;
        }
        let element_bound_runtime = options.runtime_text_by_element.get(&element.name);
        let runtime = element_bound_runtime.unwrap_or(&options.runtime_text);
        let runtime_element_id = options
            .runtime_text_authored_element_by_clone
            .get(&element.name)
            .map(String::as_str)
            .unwrap_or(&element.name);
        if runtime.exact.is_some()
            && !runtime.exact_for_element(options.runtime_view_id.as_deref(), runtime_element_id)
        {
            report.unresolved_text_inputs.insert(
                element.name.clone(),
                vec![TextIssue {
                    kind: crate::text_law::TextIssueKind::RuntimeTextIdentityMismatch,
                    token: element.name.clone(),
                }],
            );
            return;
        }
        let element_bound_source = element_bound_runtime
            .and_then(|runtime| runtime.exact.as_ref())
            .map(|exact| &exact.source);
        let resolution_text = if element_bound_runtime.is_some() && runtime.exact.is_some() {
            element.text.as_deref().unwrap_or("")
        } else {
            text.as_str()
        };
        let (resolved, requested_loc_key) = match element_bound_source {
            Some(RuntimeTextSource::SetterCallerRva(_)) => {
                (resolve_element_bound_exact_runtime(runtime), None)
            }
            Some(RuntimeTextSource::LocalisationKey(_)) => {
                let key = resolution_text.trim().trim_matches('$');
                (
                    resolve_localisation_preserving_inline_icons(
                        &options.localisation,
                        key,
                        runtime,
                    ),
                    Some(key.to_string()),
                )
            }
            Some(RuntimeTextSource::Literal(_)) => (
                resolve_template_preserving_inline_icons(
                    &options.localisation,
                    resolution_text,
                    runtime,
                ),
                None,
            ),
            Some(RuntimeTextSource::AuthoredToken(_)) => (
                resolve_template_preserving_inline_icons(
                    &options.localisation,
                    resolution_text,
                    runtime,
                ),
                None,
            ),
            None if !has_override && is_localisation_key(resolution_text) => {
                let key = resolution_text.trim().trim_matches('$');
                (
                    resolve_localisation_preserving_inline_icons(
                        &options.localisation,
                        key,
                        runtime,
                    ),
                    Some(key.to_string()),
                )
            }
            None => (
                resolve_template_preserving_inline_icons(
                    &options.localisation,
                    resolution_text,
                    runtime,
                ),
                None,
            ),
        };
        if let Some(key) = requested_loc_key {
            if !options.localisation.contains_key(&key) {
                report.unresolved_loc_keys.insert(key);
            }
        }
        if !resolved.admitted() {
            report
                .unresolved_text_inputs
                .insert(element.name.clone(), resolved.issues);
            return;
        }
        if options.require_localisation_provenance {
            let mut missing = Vec::new();
            for key in &resolved.source_keys {
                if let Some(sources) = options.localisation_ambiguous_sources_by_key.get(key) {
                    missing.push(TextIssue {
                        kind: crate::text_law::TextIssueKind::AmbiguousLocalisationProvenance,
                        token: format!("{key}:{}", sources.join("|")),
                    });
                    continue;
                }
                match options.localisation_source_by_key.get(key) {
                    Some(source) => {
                        report
                            .localisation_sources_used
                            .insert(key.clone(), source.clone());
                    }
                    None => missing.push(TextIssue {
                        kind: crate::text_law::TextIssueKind::MissingLocalisationProvenance,
                        token: key.clone(),
                    }),
                }
            }
            if !missing.is_empty() {
                report
                    .unresolved_text_inputs
                    .insert(element.name.clone(), missing);
                return;
            }
        }
        text = resolved.text;
        if let Some(exact) = runtime.exact.as_ref() {
            if let RuntimeTextLayout::CapturedRuntime(layout) = &exact.layout {
                self.draw_runtime_text_layout(layout, rect, image, report);
                return;
            }
        }
        let Some(font_name) = element.font.as_deref() else {
            return;
        };
        let font = match self.font(font_name) {
            Ok(font) => font,
            Err(err) => {
                report
                    .missing_fonts
                    .insert(format!("{font_name}: {}", err.message));
                return;
            }
        };
        let max_width = match element.raw.first_value_ci("maxWidth") {
            Some(_) => match scalar_i32_ci(&element.raw, "maxWidth") {
                Some(value) => value,
                None => {
                    report.unresolved_text_inputs.insert(
                        element.name.clone(),
                        vec![TextIssue {
                            kind: crate::text_law::TextIssueKind::EngineTextLayoutRefused,
                            token: "authored maxWidth is unparseable or outside i32".into(),
                        }],
                    );
                    return;
                }
            },
            None => rect.w,
        };
        if max_width <= 0 {
            report.unresolved_text_inputs.insert(
                element.name.clone(),
                vec![TextIssue {
                    kind: crate::text_law::TextIssueKind::EngineTextLayoutRefused,
                    token: format!("non-positive effective maxWidth {max_width}"),
                }],
            );
            return;
        }
        let border = match element.raw.first_value_ci("borderSize") {
            Some(value) => match parse_engine_text_border(value) {
                Some((x, y)) => EngineTextBorder::Authored { x, y },
                None => {
                    report.unresolved_text_inputs.insert(
                        element.name.clone(),
                        vec![TextIssue {
                            kind: crate::text_law::TextIssueKind::EngineTextLayoutRefused,
                            token: "authored borderSize is malformed or outside i32".into(),
                        }],
                    );
                    return;
                }
            },
            None => EngineTextBorder::ExplicitlyAbsent,
        };
        let (icon_metrics, inline_icons) = match self.resolve_engine_inline_icons(&font, &text) {
            Ok(value) => value,
            Err(error) => {
                report.unresolved_text_inputs.insert(
                    element.name.clone(),
                    vec![TextIssue {
                        kind: crate::text_law::TextIssueKind::EngineTextLayoutRefused,
                        token: error.message,
                    }],
                );
                return;
            }
        };
        let layout =
            match layout_engine_text_with_icons(&font, &text, max_width, border, &icon_metrics) {
                Ok(layout) => layout,
                Err(error) => {
                    report.unresolved_text_inputs.insert(
                        element.name.clone(),
                        vec![TextIssue {
                            kind: crate::text_law::TextIssueKind::EngineTextLayoutRefused,
                            token: error.to_string(),
                        }],
                    );
                    return;
                }
            };
        let align = TextAlign::parse(element.format.as_deref());
        if let Err(error) = draw_engine_layout(
            &font,
            &layout,
            &inline_icons,
            image,
            rect.x,
            rect.y,
            layout.wrap_width,
            align,
            &text,
            options.text_color,
        ) {
            report.unresolved_text_inputs.insert(
                element.name.clone(),
                vec![TextIssue {
                    kind: crate::text_law::TextIssueKind::EngineTextLayoutRefused,
                    token: error.message,
                }],
            );
            return;
        }
        report.drawn_elements += 1;
    }

    fn resolve_engine_inline_icons(
        &mut self,
        font: &BitmapFont,
        text: &str,
    ) -> Result<(
        Vec<ResolvedInlineIconMetrics>,
        Vec<PreparedEngineInlineIcon>,
    )> {
        let empty_colors = crate::markup::TextColorTable::default();
        let color_table = font.color_table.as_ref().unwrap_or(&empty_colors);
        let mut metrics = Vec::new();
        let mut prepared = Vec::new();
        for token in crate::markup::parse(text, color_table) {
            let (key, frame) = match token {
                crate::markup::Token::Icon { key, frame } => (key, frame),
                crate::markup::Token::Variable { key } => {
                    return Err(GameUiError::missing(format!(
                        "unresolved localisation variable ${key}$ reached TEXT1 layout"
                    )))
                }
                crate::markup::Token::DataFunction { expr } => {
                    return Err(GameUiError::missing(format!(
                        "unresolved data function [{expr}] reached TEXT1 layout"
                    )))
                }
                crate::markup::Token::Text { .. } => continue,
            };
            let metric_token = match frame {
                Some(frame) => format!("{key}|{frame}"),
                None => key.clone(),
            };
            let sprite_name = if key.starts_with("GFX_") {
                key
            } else {
                format!("GFX_{key}")
            };
            let frame = engine_inline_icon_frame_index(frame).ok_or_else(|| {
                GameUiError::parse(format!(
                    "inline-icon {metric_token:?} uses invalid authored frame zero"
                ))
            })?;
            unique_inline_sprite_definition(self.registry, &sprite_name)?;
            let decoded = self.decoded_sprite(&sprite_name)?;
            let image = decoded
                .frames
                .get(
                    usize::try_from(frame)
                        .map_err(|_| GameUiError::parse("inline-icon frame is outside usize"))?,
                )
                .ok_or_else(|| {
                    GameUiError::missing(format!(
                        "inline sprite {sprite_name:?} frame {frame} is outside 0..{}",
                        decoded.frames.len()
                    ))
                })?
                .clone();
            let advance = i32::try_from(image.width)
                .map_err(|_| GameUiError::parse("inline-icon width is outside i32"))?;
            let height = i32::try_from(image.height)
                .map_err(|_| GameUiError::parse("inline-icon height is outside i32"))?;
            if advance <= 0 || height <= 0 {
                return Err(GameUiError::parse(
                    "inline-icon decoded frame has a non-positive extent",
                ));
            }
            metrics.push(ResolvedInlineIconMetrics {
                token: metric_token,
                sprite: sprite_name.clone(),
                frame,
                advance,
                height,
            });
            prepared.push(PreparedEngineInlineIcon {
                sprite: sprite_name,
                frame,
                image,
            });
        }
        Ok((metrics, prepared))
    }

    fn draw_runtime_text_layout(
        &mut self,
        layout: &replay_core::RuntimeTextCapturedLayout,
        rect: Rect,
        image: &mut RgbaImage,
        report: &mut ComposeReport,
    ) {
        let clip = layout
            .clip
            .map(|[x, y, w, h]| [rect.x + x, rect.y + y, w, h]);
        enum CapturedRun<'a> {
            Glyph(&'a replay_core::RuntimeGlyphRun),
            InlineIcon(&'a replay_core::RuntimeInlineIconRun),
        }
        let mut ordered_runs = layout
            .glyphs
            .iter()
            .map(|run| (run.run_ordinal, CapturedRun::Glyph(run)))
            .chain(
                layout
                    .inline_icons
                    .iter()
                    .map(|run| (run.run_ordinal, CapturedRun::InlineIcon(run))),
            )
            .collect::<Vec<_>>();
        ordered_runs.sort_by_key(|(ordinal, _)| *ordinal);
        for (_, run) in ordered_runs {
            match run {
                CapturedRun::Glyph(glyph) => {
                    let font = match self.font(&glyph.font) {
                        Ok(font) => font,
                        Err(error) => {
                            report
                                .missing_fonts
                                .insert(format!("{}: {}", glyph.font, error.message));
                            continue;
                        }
                    };
                    match font.draw_runtime_glyph(
                        image,
                        glyph.glyph_id,
                        rect.x + glyph.x,
                        rect.y + glyph.y,
                        glyph.rgba,
                        clip,
                    ) {
                        crate::font::RuntimeGlyphDraw::Drawn => {}
                        crate::font::RuntimeGlyphDraw::GlyphAbsent => {
                            report
                                .missing_glyphs
                                .entry(glyph.font.clone())
                                .or_default()
                                .insert(char::from_u32(glyph.glyph_id).unwrap_or('\u{fffd}'));
                        }
                        crate::font::RuntimeGlyphDraw::PageAbsent { page } => {
                            report.missing_fonts.insert(format!(
                                "{}: glyph {} references atlas page {page}, which is not loaded",
                                glyph.font, glyph.glyph_id
                            ));
                        }
                    }
                }
                CapturedRun::InlineIcon(icon) => {
                    if let Err(error) = unique_inline_sprite_definition(self.registry, &icon.sprite)
                    {
                        report
                            .undecodable_sprites
                            .push(format!("{}: {}", icon.sprite, error.message));
                        continue;
                    }
                    let decoded = match self.decoded_sprite(&icon.sprite) {
                        Ok(decoded) => decoded,
                        Err(error) => {
                            report
                                .undecodable_sprites
                                .push(format!("{}: {}", icon.sprite, error.message));
                            continue;
                        }
                    };
                    let Some(frame) = decoded.frames.get(icon.frame as usize) else {
                        report.undecodable_sprites.push(format!(
                            "{}: captured frame {} outside {} frames",
                            icon.sprite,
                            icon.frame,
                            decoded.frames.len()
                        ));
                        continue;
                    };
                    draw_image_clipped(image, frame, rect.x + icon.x, rect.y + icon.y, clip);
                }
            }
        }
        report.drawn_elements += 1;
    }

    fn draw_sprite(
        &mut self,
        element: &GuiElement,
        sprite_name: &str,
        rect: Rect,
        image: &mut RgbaImage,
        options: &ComposeOptions,
        report: &mut ComposeReport,
    ) {
        let Some(sprite) = self.registry.get(sprite_name) else {
            report.missing_sprites.insert(sprite_name.to_string());
            return;
        };
        let draw_result = if sprite.kind.eq_ignore_ascii_case("pieChartType") {
            self.draw_pie_chart(element, sprite, rect, image, options)
        } else if is_progress_bar(sprite) {
            self.draw_progress_bar(element, sprite, rect, image, options)
        } else {
            self.draw_plain_sprite(element, sprite, rect, image, options)
        };
        match draw_result {
            Ok(did_draw) => {
                if did_draw {
                    report.drawn_elements += 1;
                }
            }
            Err(err) => report
                .undecodable_sprites
                .push(format!("{sprite_name}: {}", err.message)),
        }
    }

    fn draw_plain_sprite(
        &mut self,
        element: &GuiElement,
        sprite: &SpriteDefinition,
        rect: Rect,
        image: &mut RgbaImage,
        options: &ComposeOptions,
    ) -> Result<bool> {
        let decoded = self.decoded_sprite(&sprite.name)?;
        let frame_index = frame_index(element, options, decoded.frames.len());
        let frame = decoded
            .frames
            .get(frame_index)
            .or_else(|| decoded.frames.first())
            .ok_or_else(|| GameUiError::missing(format!("sprite {} has no frames", sprite.name)))?;
        let shaded;
        let tinted;
        let xui23_authorized = options.xui23_buttonstate_elements.contains(&element.name);
        let frame = if xui23_authorized {
            shaded = self.shade_button_sprite(
                sprite,
                frame,
                frame_index + 1,
                options.animation_elapsed_micros,
                options.disabled_button_elements.contains(&element.name),
            )?;
            &shaded
        } else {
            frame
        };
        let frame = if let Some(color) = options.color_overrides.get(&element.name).copied() {
            tinted = tint_sprite(frame, color);
            &tinted
        } else {
            frame
        };
        let target = target_size(rect, frame.width, frame.height);
        let scale = scalar_f32_ci(&element.raw, "scale").unwrap_or(1.0);
        let target_w = ((target.0 as f32) * scale).round().max(1.0) as usize;
        let target_h = ((target.1 as f32) * scale).round().max(1.0) as usize;
        if let Some(border) = &sprite.border_size {
            let (bx, by) = parse_sprite_size(border).unwrap_or((0, 0));
            draw_nine_slice(image, frame, rect.x, rect.y, target_w, target_h, bx, by);
        } else if target_w != frame.width || target_h != frame.height {
            draw_scaled(image, frame, rect.x, rect.y, target_w, target_h);
        } else {
            draw_image(image, frame, rect.x, rect.y);
        }
        Ok(true)
    }

    fn shade_button_sprite(
        &mut self,
        sprite: &SpriteDefinition,
        base: &RgbaImage,
        one_based_frame: usize,
        elapsed_micros: u64,
        disabled: bool,
    ) -> Result<RgbaImage> {
        crate::runtime_graphics::authenticate_xui23_buttonstate_shader(self.resolver, sprite)
            .map_err(|reason| {
                GameUiError::parse(format!(
                    "XUI23 sprite {} buttonstate shader closure rejected: {reason:?}",
                    sprite.name
                ))
            })?;
        let mut output = base.clone();
        let animations = sprite.raw.values_ci("animation").collect::<Vec<_>>();
        for animation in &animations {
            validate_xui23_animation_shape(animation, &sprite.name)?;
            validate_xui23_scrolling_add_animation(animation, &sprite.name)?;
        }
        let applicable =
            xui23_first_two_applicable_animations(&animations, &sprite.name, one_based_frame)?;
        let elapsed_seconds = elapsed_micros as f64 / 1_000_000.0;
        let mut prepared = Vec::with_capacity(applicable.len());
        for animation in applicable {
            let mask_path = required_scalar_ci(animation, "animationmaskfile", &sprite.name)?;
            let animated_path =
                required_scalar_ci(animation, "animationtexturefile", &sprite.name)?;
            let animation_time = required_f32_ci(animation, "animationtime", &sprite.name)?;
            let animation_delay = required_f32_ci(animation, "animationdelay", &sprite.name)?;
            let rotation = required_f32_ci(animation, "animationrotation", &sprite.name)?;
            let rotation_offset =
                required_xy_ci(animation, "animationrotationoffset", &sprite.name)?;
            let texture_scale = required_xy_ci(animation, "animationtexturescale", &sprite.name)?;
            let looping = required_yes_no_ci(animation, "animationlooping", &sprite.name)?;
            let animation_type = required_scalar_ci(animation, "animationtype", &sprite.name)?;
            let blend_mode = required_scalar_ci(animation, "animationblendmode", &sprite.name)?;
            if !animation_time.is_finite()
                || animation_time <= 0.0
                || !animation_delay.is_finite()
                || animation_delay < 0.0
                || rotation != 0.0
                || rotation_offset != [0.0, 0.0]
                || texture_scale != [1.0, 1.0]
                || !looping
                || !animation_type.eq_ignore_ascii_case("scrolling")
                || !blend_mode.eq_ignore_ascii_case("add")
            {
                return Err(GameUiError::parse(format!(
                    "animated sprite {} is outside the exact XUI23 scrolling/add law",
                    sprite.name
                )));
            }
            let mask = self.decoded_texture(mask_path)?;
            let animated = self.decoded_texture(animated_path)?;
            let offset = if elapsed_seconds <= f64::from(animation_delay) {
                0.0
            } else {
                ((elapsed_seconds - f64::from(animation_delay)) / f64::from(animation_time)).fract()
                    as f32
            };
            prepared.push((mask, animated, offset));
        }
        for y in 0..output.height {
            for x in 0..output.width {
                let u = (x as f32 + 0.5) / output.width as f32;
                let v = (y as f32 + 0.5) / output.height as f32;
                let pixel = shade_xui23_button_animation_pixel(
                    image_pixel_f32(base, x, y),
                    disabled,
                    prepared.iter().map(|(mask, animated, offset)| {
                        (
                            sample_linear(mask, u, v, false),
                            sample_linear(animated, u, v + *offset, true),
                        )
                    }),
                );
                set_image_pixel_f32(&mut output, x, y, pixel);
            }
        }
        Ok(output)
    }

    fn draw_progress_bar(
        &mut self,
        element: &GuiElement,
        sprite: &SpriteDefinition,
        rect: Rect,
        image: &mut RgbaImage,
        options: &ComposeOptions,
    ) -> Result<bool> {
        let Some(fg_rel) = sprite.texture_files.first().map(|slot| slot.path.clone()) else {
            return Err(GameUiError::missing(format!(
                "progress bar {} has no foreground texture",
                sprite.name
            )));
        };
        let bg_rel = sprite
            .texture_files
            .get(1)
            .map(|slot| slot.path.clone())
            .unwrap_or_else(|| fg_rel.clone());
        let fg = self.decoded_texture(&fg_rel)?;
        let bg = self.decoded_texture(&bg_rel)?;
        let size = sprite
            .size
            .as_ref()
            .and_then(parse_sprite_size)
            .unwrap_or((fg.width, fg.height));
        let target = target_size(rect, size.0, size.1);
        let progress = options
            .progress_overrides
            .get(&element.name)
            .copied()
            .unwrap_or_else(|| scalar_u32_ci(&element.raw, "frame").unwrap_or(100) as f32 / 100.0)
            .clamp(0.0, 1.0);
        let horizontal = progress_bar_horizontal(sprite)?;
        let xui23_authorized = options.xui23_progress_elements.contains(&element.name);
        if options.xui23_require_scoped_progress && !xui23_authorized {
            return Err(GameUiError::parse(format!(
                "XUI23 surface contains unscoped progress element {}",
                element.name
            )));
        }
        if xui23_authorized {
            crate::runtime_graphics::authenticate_xui23_progress_shader(self.resolver, sprite)
                .map_err(|reason| {
                    GameUiError::parse(format!(
                        "XUI23 sprite {} progress shader closure rejected: {reason:?}",
                        sprite.name
                    ))
                })?;
        }
        if !xui23_authorized
            && sprite
                .effect_file
                .as_deref()
                .is_some_and(|effect| effect.to_ascii_lowercase().contains("progress_minmax"))
        {
            draw_progress_minmax_axis(
                image, &fg, &bg, rect.x, rect.y, target.0, target.1, progress, horizontal,
            );
        } else {
            draw_progress_normal_axis(
                image, &fg, &bg, rect.x, rect.y, target.0, target.1, progress, horizontal,
            );
        }
        Ok(true)
    }

    fn draw_pie_chart(
        &mut self,
        element: &GuiElement,
        sprite: &SpriteDefinition,
        rect: Rect,
        image: &mut RgbaImage,
        options: &ComposeOptions,
    ) -> Result<bool> {
        let rows = options
            .pie_chart_overrides
            .get(&element.name)
            .ok_or_else(|| {
                GameUiError::missing(format!(
                    "pie chart {} has no exact ordered runtime slices",
                    sprite.name
                ))
            })?;
        let normalized = normalize_pie_slices(rows)?;
        let radius = sprite
            .raw
            .first_value_ci("size")
            .and_then(scalar_i32)
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                GameUiError::missing(format!(
                    "pie chart {} has no positive authored radius",
                    sprite.name
                ))
            })?;
        draw_exact_pie(image, rect.x, rect.y, radius, &normalized);
        Ok(true)
    }

    fn natural_size(&mut self, element: &GuiElement, options: &ComposeOptions) -> (usize, usize) {
        if let Some(size) = &element.size {
            return (
                measure_to_pixels(&size.x, 0).max(0) as usize,
                measure_to_pixels(&size.y, 0).max(0) as usize,
            );
        }
        let Some(sprite_name) = options
            .sprite_name_overrides
            .get(&element.name)
            .map(String::as_str)
            .or_else(|| primary_sprite_ref(element))
        else {
            return (0, 0);
        };
        let Some(sprite) = self.registry.get(sprite_name) else {
            return (0, 0);
        };
        if let Some(size) = &sprite.size {
            if let Some(parsed) = parse_sprite_size(size) {
                return parsed;
            }
        }
        if sprite.kind.eq_ignore_ascii_case("pieChartType") {
            if let Some(radius) = sprite
                .raw
                .first_value_ci("size")
                .and_then(scalar_i32)
                .filter(|value| *value > 0)
            {
                let diameter = radius.saturating_mul(2) as usize;
                return (diameter, diameter);
            }
        }
        match self.decoded_sprite(&sprite.name) {
            Ok(decoded) => decoded
                .frames
                .first()
                .map(|frame| (frame.width, frame.height))
                .unwrap_or((0, 0)),
            Err(_) => (0, 0),
        }
    }

    fn decoded_sprite(&mut self, sprite_name: &str) -> Result<Arc<DecodedSprite>> {
        let resolver = self.resolver;
        let registry = self.registry;
        let cache = self.cache.get_mut();
        if !cache.sprites.contains_key(sprite_name) {
            let value = registry
                .get(sprite_name)
                .ok_or_else(|| GameUiError::missing(format!("missing sprite {sprite_name}")))
                .and_then(|sprite| resolver.decode_sprite(sprite))
                .map(Arc::new);
            cache.sprites.insert(sprite_name.to_string(), value);
        }
        cache
            .sprites
            .get(sprite_name)
            .expect("cache populated")
            .as_ref()
            .map(Arc::clone)
            .map_err(Clone::clone)
    }

    fn decoded_texture(&mut self, rel: &str) -> Result<Arc<RgbaImage>> {
        let resolver = self.resolver;
        let cache = self.cache.get_mut();
        if !cache.textures.contains_key(rel) {
            let value = resolver
                .decode_texture(rel)
                .map(|decoded| Arc::new(decoded.image));
            cache.textures.insert(rel.to_string(), value);
        }
        cache
            .textures
            .get(rel)
            .expect("cache populated")
            .as_ref()
            .map(Arc::clone)
            .map_err(Clone::clone)
    }

    fn font(&mut self, font_name: &str) -> Result<Arc<BitmapFont>> {
        let resolver = self.resolver;
        let cache_key = format!("unscoped:{font_name}");
        let cache = self.cache.get_mut();
        if !cache.fonts.contains_key(&cache_key) {
            let value = BitmapFont::load(resolver, font_name).map(Arc::new);
            cache.fonts.insert(cache_key.clone(), value);
        }
        cache
            .fonts
            .get(&cache_key)
            .expect("cache populated")
            .as_ref()
            .map(Arc::clone)
            .map_err(Clone::clone)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieChartSlice {
    pub weight: f64,
    pub color: [u8; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NormalizedPieSlice {
    segments: u32,
    color: [u8; 4],
}

fn normalize_pie_slices(rows: &[PieChartSlice]) -> Result<Vec<NormalizedPieSlice>> {
    if rows.is_empty()
        || rows
            .iter()
            .any(|row| !row.weight.is_finite() || row.weight < 0.0)
    {
        return Err(GameUiError::parse(
            "pie chart has invalid exact slice weights",
        ));
    }
    let total = rows.iter().map(|row| row.weight).sum::<f64>();
    if !total.is_finite() || total <= 0.0 {
        return Err(GameUiError::parse(
            "pie chart has no positive exact slice weight",
        ));
    }
    let mut prior = 0u32;
    Ok(rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let segments = if index + 1 == rows.len() {
                100u32.saturating_sub(prior)
            } else {
                ((row.weight / total) * 100.0 + 0.5) as u32
            };
            prior = prior.saturating_add(segments);
            NormalizedPieSlice {
                segments,
                color: row.color,
            }
        })
        .collect())
}

fn tint_sprite(source: &RgbaImage, color: [u8; 4]) -> RgbaImage {
    let mut tinted = source.clone();
    for pixel in tinted.pixels.chunks_exact_mut(4) {
        for channel in 0..4 {
            pixel[channel] =
                ((u16::from(pixel[channel]) * u16::from(color[channel]) + 127) / 255) as u8;
        }
    }
    tinted
}

fn draw_exact_pie(
    image: &mut RgbaImage,
    left: i32,
    top: i32,
    radius: i32,
    rows: &[NormalizedPieSlice],
) {
    let radius_f64 = f64::from(radius);
    let tau = std::f64::consts::TAU;
    let mut segment_colors = Vec::with_capacity(100);
    for row in rows {
        segment_colors.extend(std::iter::repeat(row.color).take(row.segments as usize));
    }
    segment_colors.truncate(100);
    for local_y in 0..radius.saturating_mul(2) {
        for local_x in 0..radius.saturating_mul(2) {
            let dx = f64::from(local_x) + 0.5 - radius_f64;
            let dy = f64::from(local_y) + 0.5 - radius_f64;
            let angle = (dx.atan2(dy) + tau) % tau;
            let segment = ((angle / tau) * 100.0).floor() as usize;
            let start = segment as f64 * tau / 100.0;
            let end = (segment + 1) as f64 * tau / 100.0;
            let a = [start.sin() * radius_f64, start.cos() * radius_f64];
            let b = [end.sin() * radius_f64, end.cos() * radius_f64];
            if !point_in_origin_triangle([dx, dy], a, b) {
                continue;
            }
            if let Some(color) = segment_colors.get(segment).copied() {
                blend_pixel_at(image, left + local_x, top + local_y, color);
            }
        }
    }
}

fn point_in_origin_triangle(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    let denominator = a[0] * b[1] - b[0] * a[1];
    if denominator.abs() <= f64::EPSILON {
        return false;
    }
    let u = (point[0] * b[1] - b[0] * point[1]) / denominator;
    let v = (a[0] * point[1] - point[0] * a[1]) / denominator;
    u >= 0.0 && v >= 0.0 && u + v <= 1.0
}

fn blend_pixel_at(image: &mut RgbaImage, x: i32, y: i32, source: [u8; 4]) {
    if x < 0 || y < 0 || x >= image.width as i32 || y >= image.height as i32 {
        return;
    }
    let offset = (y as usize * image.width + x as usize) * 4;
    let destination = [
        image.pixels[offset],
        image.pixels[offset + 1],
        image.pixels[offset + 2],
        image.pixels[offset + 3],
    ];
    image.pixels[offset..offset + 4].copy_from_slice(&alpha_blend(destination, source));
}

fn authenticated_options_are_static(options: &ComposeOptions) -> bool {
    let mut expected = ComposeOptions::new(options.viewport_width, options.viewport_height);
    expected.root_width = options.root_width;
    expected.root_height = options.root_height;
    options == &expected
}

fn validate_authenticated_sprite_executable_subset(
    sprite: &SpriteDefinition,
    sprite_role: &crate::authored_role_index::AuthoredSpriteRole,
    child_path: &[u32],
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    validate_authenticated_sprite_raw_fields(sprite, child_path)?;
    if !sprite.kind.eq_ignore_ascii_case("spriteType") {
        return Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
            child_path.to_vec(),
        ));
    }
    if sprite.no_of_frames == 0 || sprite.no_of_frames != sprite_role.frame_count {
        return Err(AuthenticatedComposeRejection::UnsupportedAnimatedVariant(
            child_path.to_vec(),
        ));
    }
    if sprite.border_size.is_some() {
        return Err(
            AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(
                child_path.to_vec(),
            ),
        );
    }
    let authored_color_was_explicit = sprite.raw.values_ci("color").next().is_some();
    if authored_color_was_explicit != sprite_role.authored_color_was_explicit
        || crate::gfx::a729_sprite_color_rgba_f32_bits(&sprite.raw).ok()
            != Some(sprite_role.authored_color_rgba_f32_bits)
    {
        return Err(
            AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(
                child_path.to_vec(),
            ),
        );
    }
    Ok(())
}

fn validate_authenticated_sprite_raw_fields(
    sprite: &SpriteDefinition,
    child_path: &[u32],
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    let reject = || AuthenticatedComposeRejection::SpriteDefinitionMismatch(child_path.to_vec());
    let exactly_one_scalar = |field: &str| {
        let mut values = sprite.raw.values_ci(field);
        values
            .next()
            .and_then(Value::as_scalar)
            .is_some_and(|value| !value.trim().is_empty())
            && values.next().is_none()
    };
    if !exactly_one_scalar("name") {
        return Err(reject());
    }
    let frame_values = sprite.raw.values_ci("noOfFrames").collect::<Vec<_>>();
    if frame_values.len() > 1
        || frame_values.first().is_some_and(|value| {
            scalar_u32(value).is_none_or(|frames| frames == 0 || frames != sprite.no_of_frames)
        })
    {
        return Err(reject());
    }
    let effect_values = sprite.raw.values_ci("effectFile").collect::<Vec<_>>();
    if effect_values.len() > 1
        || effect_values.first().is_some_and(|value| {
            value
                .as_scalar()
                .map_or(true, |value| value.trim().is_empty())
        })
    {
        return Err(reject());
    }
    let texture_values = sprite
        .raw
        .fields()
        .iter()
        .filter(|entry| {
            entry
                .key
                .as_deref()
                .is_some_and(|key| key.to_ascii_lowercase().starts_with("texturefile"))
        })
        .collect::<Vec<_>>();
    if texture_values.len() != 1
        || texture_values[0]
            .value
            .as_scalar()
            .map_or(true, |value| value.trim().is_empty())
    {
        return Err(reject());
    }
    for field in ["size", "borderSize"] {
        let values = sprite.raw.values_ci(field).collect::<Vec<_>>();
        if values.len() > 1 {
            return Err(reject());
        }
        let Some(value) = values.first() else {
            continue;
        };
        let horizontal = value
            .fields()
            .iter()
            .filter(|entry| {
                entry.key.as_deref().is_some_and(|key| {
                    key.eq_ignore_ascii_case("x") || key.eq_ignore_ascii_case("width")
                })
            })
            .count();
        let vertical = value
            .fields()
            .iter()
            .filter(|entry| {
                entry.key.as_deref().is_some_and(|key| {
                    key.eq_ignore_ascii_case("y") || key.eq_ignore_ascii_case("height")
                })
            })
            .count();
        if horizontal != 1 || vertical != 1 || crate::gfx::parse_size(value).is_none() {
            return Err(reject());
        }
    }
    if sprite.raw.values_ci("animation").next().is_some() {
        return Err(AuthenticatedComposeRejection::UnsupportedAnimatedVariant(
            child_path.to_vec(),
        ));
    }
    for entry in sprite.raw.fields() {
        let Some(key) = entry.key.as_deref() else {
            return Err(reject());
        };
        let normalized = key.to_ascii_lowercase();
        if normalized == "texturefile" {
            continue;
        }
        match normalized.as_str() {
            "name" | "effectfile" | "noofframes" | "size" | "bordersize" | "color" => {}
            "animation" => unreachable!("animation refused above"),
            _ => return Err(reject()),
        }
    }
    Ok(())
}

fn authenticated_sprite_cache_key(
    sprite: &SpriteDefinition,
    sprite_role: &crate::authored_role_index::AuthoredSpriteRole,
    admission: &ValidatedContentAdmission,
    index_identity_sha256: &str,
) -> Result<String> {
    authenticated_sprite_cache_key_material(
        admission.manifest_digest_sha256(),
        admission.product_modset_sha256(),
        index_identity_sha256,
        sprite,
        sprite_role,
    )
}

fn authenticated_sprite_cache_key_material(
    manifest_digest_sha256: &str,
    product_modset_sha256: &str,
    index_identity_sha256: &str,
    sprite: &SpriteDefinition,
    sprite_role: &crate::authored_role_index::AuthoredSpriteRole,
) -> Result<String> {
    let sealed_role = serde_json::to_string(&(
        manifest_digest_sha256,
        product_modset_sha256,
        index_identity_sha256,
        sprite.name.to_ascii_lowercase(),
        sprite.source_file.to_ascii_lowercase(),
        sprite_role,
    ))
    .map_err(|error| {
        GameUiError::parse(format!(
            "failed to seal authenticated sprite cache identity: {error}"
        ))
    })?;
    Ok(format!("authenticated:v2:{sealed_role}"))
}

fn validate_authenticated_dimensions(
    options: &ComposeOptions,
    framebuffer: Option<&RgbaImage>,
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    if options.viewport_width == 0
        || options.viewport_height == 0
        || options.root_width == 0
        || options.root_height == 0
        || options.viewport_width > i32::MAX as usize
        || options.viewport_height > i32::MAX as usize
        || options.root_width > i32::MAX as usize
        || options.root_height > i32::MAX as usize
        || framebuffer.is_some_and(|image| {
            image.width != options.viewport_width
                || image.height != options.viewport_height
                || image
                    .width
                    .checked_mul(image.height)
                    .and_then(|pixels| pixels.checked_mul(4))
                    != Some(image.pixels.len())
        })
    {
        return Err(AuthenticatedComposeRejection::RootIdentityMismatch);
    }
    Ok(())
}

fn validate_preflight(
    preflight: &AuthenticatedLayoutPreflight,
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
    panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
    panel_lease: &AuthenticatedReplayPanelLease<'_>,
    options: &ComposeOptions,
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    if !frame_authority.remains_staged_for(manager)
        || preflight.frame_generation != frame_authority.frame_generation()
        || preflight.index_identity_sha256 != index.index_identity_sha256
        || preflight.window_id != panel.window_id()
        || preflight.surface_id != panel.surface_id()
        || preflight.source_content != *panel.source_content()
        || !panel_lease.remains_current()
        || panel_lease.window_id() != panel.window_id()
        || preflight.runtime_instance_token != panel_lease.runtime_instance_token()
        || preflight.viewport_size() != [options.viewport_width, options.viewport_height]
        || preflight.root_size() != [options.root_width, options.root_height]
    {
        return Err(AuthenticatedComposeRejection::StaleRuntimeInstance(
            Vec::new(),
        ));
    }
    Ok(())
}

fn validate_semantic_census(
    semantics: &AuthenticatedPanelSemanticCensus,
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &crate::authored_role_index::AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &crate::authored_role_index::StrictAuthoredPanelAdmission,
    panel_lease: &AuthenticatedReplayPanelLease<'_>,
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    let surface = index
        .validate_panel_token(admission, panel)
        .map_err(|_| AuthenticatedComposeRejection::PanelAuthentication)?;
    if !frame_authority.remains_staged_for(manager)
        || semantics.frame_generation != frame_authority.frame_generation()
        || semantics.index_identity_sha256 != index.index_identity_sha256
        || semantics.window_id != panel.window_id()
        || semantics.surface_id != surface.surface_id
        || semantics.source_content != surface.source_content
        || !panel_lease.remains_current()
        || panel_lease.window_id() != panel.window_id()
        || semantics.runtime_instance_token != panel_lease.runtime_instance_token()
        || semantics.elements.len() != surface.elements.len()
        || surface
            .elements
            .iter()
            .any(|role| !semantics.elements.contains_key(&role.child_path))
        || semantics
            .elements
            .keys()
            .any(|path| !surface.elements.iter().any(|role| role.child_path == *path))
    {
        return Err(AuthenticatedComposeRejection::SemanticCensusIdentity);
    }
    Ok(())
}

fn validate_final_census_against_preflight(
    final_elements: &[AuthenticatedComposedElement],
    preflight: &AuthenticatedLayoutPreflight,
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    if final_elements.len() != preflight.elements.len() {
        return Err(AuthenticatedComposeRejection::RootIdentityMismatch);
    }
    for (final_element, preflight_element) in final_elements.iter().zip(&preflight.elements) {
        if final_element.child_path != preflight_element.child_path
            || final_element.element_id != preflight_element.element_id
            || final_element.authored_name != preflight_element.authored_name
            || final_element.element_type != preflight_element.element_type
            || final_element.visible
                && (final_element.screen_rect != preflight_element.screen_rect
                    || final_element.clip_rect != preflight_element.clip_rect)
            || final_element.paint_ordinal != preflight_element.paint_ordinal
        {
            return Err(AuthenticatedComposeRejection::RootIdentityMismatch);
        }
    }
    Ok(())
}

fn compose_rgba_transaction<T, E>(
    destination: &mut RgbaImage,
    compose: impl FnOnce(&mut RgbaImage) -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let mut staged = destination.clone();
    let result = compose(&mut staged)?;
    *destination = staged;
    Ok(result)
}

fn authenticated_element_rect(rect: Rect) -> AuthenticatedElementRect {
    AuthenticatedElementRect {
        x: rect.x,
        y: rect.y,
        width: rect.w,
        height: rect.h,
    }
}

fn rect_from_authenticated(rect: AuthenticatedElementRect) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y,
        w: rect.width,
        h: rect.height,
    }
}

fn record_hidden_authenticated_subtree(
    element: &GuiElement,
    child_path: &[u32],
    surface: &crate::authored_role_index::AuthoredSurfaceRole,
    elements: &mut Vec<AuthenticatedComposedElement>,
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    let role = surface
        .elements
        .iter()
        .find(|role| role.child_path == child_path)
        .ok_or(AuthenticatedComposeRejection::RootIdentityMismatch)?;
    elements.push(AuthenticatedComposedElement {
        child_path: child_path.to_vec(),
        element_id: role.element_id.clone(),
        authored_name: role.authored_name.clone(),
        element_type: role.element_type.clone(),
        visible: false,
        screen_rect: None,
        clip_rect: None,
        paint_ordinal: None,
    });
    for (ordinal, child) in element.children.iter().enumerate() {
        let ordinal = u32::try_from(ordinal)
            .map_err(|_| AuthenticatedComposeRejection::RootIdentityMismatch)?;
        let mut path = child_path.to_vec();
        path.push(ordinal);
        record_hidden_authenticated_subtree(child, &path, surface, elements)?;
    }
    Ok(())
}

fn validate_authenticated_tree(
    root: &GuiElement,
    surface: &crate::authored_role_index::AuthoredSurfaceRole,
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    fn walk(
        element: &GuiElement,
        child_path: &mut Vec<u32>,
        surface: &crate::authored_role_index::AuthoredSurfaceRole,
        visited: &mut BTreeSet<Vec<u32>>,
    ) -> std::result::Result<(), AuthenticatedComposeRejection> {
        let Some(role) = surface
            .elements
            .iter()
            .find(|role| role.child_path == *child_path)
        else {
            return Err(AuthenticatedComposeRejection::RootIdentityMismatch);
        };
        if role.authored_name != element.name
            || role.element_type != element.element_type
            || role.sprite_refs != element.sprite_refs
            || !visited.insert(child_path.clone())
        {
            return Err(AuthenticatedComposeRejection::RootIdentityMismatch);
        }
        for (ordinal, child) in element.children.iter().enumerate() {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| AuthenticatedComposeRejection::RootIdentityMismatch)?;
            child_path.push(ordinal);
            walk(child, child_path, surface, visited)?;
            child_path.pop();
        }
        Ok(())
    }

    let mut visited = BTreeSet::new();
    walk(root, &mut Vec::new(), surface, &mut visited)?;
    if visited.len() != surface.elements.len() {
        return Err(AuthenticatedComposeRejection::RootIdentityMismatch);
    }
    Ok(())
}

fn validate_authenticated_render_fields(
    element: &GuiElement,
    role: &crate::authored_role_index::AuthoredElementRole,
    child_path: &[u32],
) -> std::result::Result<(), AuthenticatedComposeRejection> {
    let reject = |field: &str| AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
        child_path: child_path.to_vec(),
        field: field.to_string(),
    };
    let family = WidgetFamily::from_element_type(&element.element_type).ok_or_else(|| {
        AuthenticatedComposeRejection::UnsupportedElementFamily(child_path.to_vec())
    })?;
    if role.element_type != element.element_type
        || role.authored_name != element.name
        || role.child_path != child_path
    {
        return Err(AuthenticatedComposeRejection::RootIdentityMismatch);
    }
    let text_family = matches!(
        family,
        WidgetFamily::InstantText | WidgetFamily::Text | WidgetFamily::EditBox
    );
    for entry in element.raw.fields() {
        let Some(key) = entry.key.as_deref() else {
            return Err(reject("<anonymous>"));
        };
        if WidgetFamily::from_element_type(key).is_some() {
            continue;
        }
        if raw_contains_variable_reference(&entry.value) {
            let font_alias = text_family
                && (key.eq_ignore_ascii_case("font") || key.eq_ignore_ascii_case("buttonFont"))
                && element.font.is_some()
                && role.font_ref == element.font;
            if !font_alias {
                return Err(AuthenticatedComposeRejection::UnsupportedVariableReference(
                    child_path.to_vec(),
                ));
            }
        }
    }
    for field in [
        "frame",
        "hide",
        "scale",
        "position",
        "size",
        "orientation",
        "clipping",
        "text",
        "font",
        "buttonFont",
        "textType",
        "format",
        "borderSize",
        "spacing",
        "layout",
        "scroll_wheel_factor",
        "smooth_scrolling",
        "slotsize",
        "max_slots_horizontal",
        "max_slots_vertical",
        "add_horizontal",
        "show_position",
        "hide_position",
        "show_animation_type",
        "hide_animation_type",
        "animation_time",
        "show_sound",
        "hide_sound",
        "margin",
        "verticalScrollbar",
        "vertical_scroll_step",
        "shortcut",
        "clicksound",
        "pdx_tooltip",
        "pdx_tooltip_delayed",
        "alwaystransparent",
    ] {
        if element.raw.values_ci(field).count() > 1 {
            return Err(reject(field));
        }
    }
    if let Some(frame) = element.raw.first_value_ci("frame") {
        if scalar_u32(frame) != Some(role.authored_one_based_frame)
            || role.authored_one_based_frame == 0
        {
            return Err(reject("frame"));
        }
    }
    let names = element.raw.values_ci("name").collect::<Vec<_>>();
    if names.len() != 1 || names[0].as_scalar() != Some(element.name.as_str()) {
        return Err(reject("name"));
    }
    let list_family = matches!(
        family,
        WidgetFamily::List | WidgetFamily::SmoothList | WidgetFamily::Grid | WidgetFamily::Overlap
    );
    let grid_family = matches!(family, WidgetFamily::Grid);
    let scroll_controller_family = list_family || matches!(family, WidgetFamily::Container);
    let structural_family = matches!(
        family,
        WidgetFamily::Window
            | WidgetFamily::Container
            | WidgetFamily::List
            | WidgetFamily::SmoothList
            | WidgetFamily::Grid
            | WidgetFamily::Overlap
            | WidgetFamily::LegacyScrollbar
            | WidgetFamily::ExtendedScrollbar
            | WidgetFamily::Browser
    );
    let sprite_family = matches!(
        family,
        WidgetFamily::Icon
            | WidgetFamily::Button
            | WidgetFamily::GuiButton
            | WidgetFamily::Checkbox
            | WidgetFamily::DropDown
            | WidgetFamily::ScrollbarPart
    );
    if !(text_family || structural_family || sprite_family) {
        return Err(AuthenticatedComposeRejection::UnsupportedElementFamily(
            child_path.to_vec(),
        ));
    }
    let has_text_fields = element.raw.first_value_ci("text").is_some()
        || element.raw.first_value_ci("font").is_some()
        || text_family && element.raw.first_value_ci("buttonFont").is_some()
        || element.raw.first_value_ci("textType").is_some()
        || text_family && element.raw.first_value_ci("format").is_some()
        || element.raw.first_value_ci("borderSize").is_some();
    if has_text_fields && !text_family {
        return Err(AuthenticatedComposeRejection::UnsupportedTextVariant(
            child_path.to_vec(),
        ));
    }
    if text_family
        && (element.font.is_none()
            || role.font_ref != element.font
            || role.font.is_none()
            || role.localisation_or_literal_ref != element.text)
    {
        return Err(AuthenticatedComposeRejection::UnsupportedTextVariant(
            child_path.to_vec(),
        ));
    }
    if let Some(format) = element.raw.first_value_ci("format") {
        let value = format.as_scalar().ok_or_else(|| reject("format"))?;
        let value = value.trim().to_ascii_lowercase();
        let valid = if text_family {
            matches!(value.as_str(), "left" | "center" | "centre" | "right")
        } else if grid_family {
            matches!(
                value.as_str(),
                "up" | "down" | "left" | "right" | "upper_left" | "lower_left" | "upper_right"
            )
        } else {
            false
        };
        if !valid {
            return Err(if grid_family {
                AuthenticatedComposeRejection::UnsupportedLayoutVariant(child_path.to_vec())
            } else {
                AuthenticatedComposeRejection::UnsupportedTextVariant(child_path.to_vec())
            });
        }
    }
    if let Some(slot_size) = element.raw.first_value_ci("slotsize") {
        let exact_pair = slot_size.as_object().is_some_and(|entries| {
            entries.len() == 2
                && entries.iter().all(|entry| {
                    entry.key.as_deref().is_some_and(|key| {
                        matches!(
                            key.to_ascii_lowercase().as_str(),
                            "x" | "y" | "width" | "height"
                        )
                    }) && entry.value.as_scalar().is_some()
                })
        });
        if !grid_family || !exact_pair || crate::gui::parse_measure_pair(slot_size).is_none() {
            return Err(reject("slotsize"));
        }
    }
    for field in ["max_slots_horizontal", "max_slots_vertical"] {
        if let Some(value) = element.raw.first_value_ci(field) {
            if !grid_family || scalar_u32(value).is_none_or(|value| value == 0) {
                return Err(reject(field));
            }
        }
    }
    if let Some(value) = element.raw.first_value_ci("add_horizontal") {
        let valid = value.as_scalar().is_some_and(|value| {
            value.eq_ignore_ascii_case("yes") || value.eq_ignore_ascii_case("no")
        });
        if !grid_family || !valid {
            return Err(reject("add_horizontal"));
        }
    }
    if element.raw.first_value_ci("borderSize").is_some() {
        authenticated_text_border(element, child_path)?;
    }
    for field in ["spacing", "layout"] {
        if element.raw.first_value_ci(field).is_some() && !list_family {
            return Err(AuthenticatedComposeRejection::UnsupportedLayoutVariant(
                child_path.to_vec(),
            ));
        }
    }
    if let Some(spacing) = element.raw.first_value_ci("spacing") {
        if scalar_i32(spacing).is_none() {
            return Err(reject("spacing"));
        }
    }
    if let Some(layout) = element.raw.first_value_ci("layout") {
        if layout
            .as_scalar()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(reject("layout"));
        }
    }
    if let Some(wheel) = element.raw.first_value_ci("scroll_wheel_factor") {
        if !scroll_controller_family || scalar_i32(wheel).is_none_or(|value| value <= 0) {
            return Err(reject("scroll_wheel_factor"));
        }
    }
    if let Some(smooth) = element.raw.first_value_ci("smooth_scrolling") {
        let valid = smooth.as_scalar().is_some_and(|value| {
            value.eq_ignore_ascii_case("yes") || value.eq_ignore_ascii_case("no")
        });
        if !scroll_controller_family || !valid {
            return Err(reject("smooth_scrolling"));
        }
    }
    for field in ["show_position", "hide_position"] {
        if let Some(value) = element.raw.first_value_ci(field) {
            if !child_path.is_empty()
                || !structural_family
                || crate::gui::parse_measure_pair(value).is_none()
            {
                return Err(reject(field));
            }
        }
    }
    for field in [
        "show_animation_type",
        "hide_animation_type",
        "show_sound",
        "hide_sound",
    ] {
        if let Some(value) = element.raw.first_value_ci(field) {
            if !child_path.is_empty()
                || !structural_family
                || value
                    .as_scalar()
                    .is_none_or(|value| value.trim().is_empty())
            {
                return Err(reject(field));
            }
        }
    }
    if let Some(value) = element.raw.first_value_ci("animation_time") {
        if !child_path.is_empty() || !structural_family || scalar_u32(value).is_none() {
            return Err(reject("animation_time"));
        }
    }
    if let Some(value) = element.raw.first_value_ci("margin") {
        if !scroll_controller_family || value.as_object().is_none() {
            return Err(reject("margin"));
        }
    }
    if let Some(value) = element.raw.first_value_ci("verticalScrollbar") {
        if !scroll_controller_family
            || value
                .as_scalar()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(reject("verticalScrollbar"));
        }
    }
    if let Some(value) = element.raw.first_value_ci("vertical_scroll_step") {
        if !scroll_controller_family || scalar_i32(value).is_none_or(|value| value <= 0) {
            return Err(reject("vertical_scroll_step"));
        }
    }
    for field in [
        "shortcut",
        "clicksound",
        "pdx_tooltip",
        "pdx_tooltip_delayed",
    ] {
        if let Some(value) = element.raw.first_value_ci(field) {
            if !sprite_family
                || value
                    .as_scalar()
                    .is_none_or(|value| value.trim().is_empty())
            {
                return Err(reject(field));
            }
        }
    }
    if let Some(value) = element.raw.first_value_ci("alwaystransparent") {
        let valid = value.as_scalar().is_some_and(|value| {
            value.eq_ignore_ascii_case("yes") || value.eq_ignore_ascii_case("no")
        });
        if !sprite_family || !valid {
            return Err(reject("alwaystransparent"));
        }
    }
    if raw_has_top_level_fragment(
        &element.raw,
        &[
            "color", "alpha", "opacity", "clip", "mask", "rotation", "rotate", "mirror", "flip",
        ],
    ) {
        return Err(
            AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(
                child_path.to_vec(),
            ),
        );
    }
    if raw_has_top_level_fragment(
        &element.raw,
        &[
            "minimumsize",
            "maximumsize",
            "padding",
            "maxwidth",
            "maxheight",
            "autoresize",
            "moveable",
            "resizeable",
        ],
    ) || element.raw.first_value_ci("horizontal").is_some()
        || element.raw.first_value_ci("vertical").is_some()
    {
        return Err(AuthenticatedComposeRejection::UnsupportedLayoutVariant(
            child_path.to_vec(),
        ));
    }
    if let Some(field) = first_unproved_authenticated_field(element, family) {
        return Err(
            AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                child_path: child_path.to_vec(),
                field,
            },
        );
    }
    for (field, parsed) in [
        ("position", element.position.is_some()),
        ("size", element.size.is_some()),
    ] {
        let Some(raw) = element.raw.first_value_ci(field) else {
            continue;
        };
        let Some(entries) = raw.as_object() else {
            return Err(reject(field));
        };
        if !parsed
            || entries.len() != 2
            || entries
                .iter()
                .any(|entry| entry.key.as_deref().is_none() || entry.value.as_scalar().is_none())
        {
            return Err(reject(field));
        }
        let horizontal = raw
            .fields()
            .iter()
            .filter(|entry| {
                entry.key.as_deref().is_some_and(|key| {
                    key.eq_ignore_ascii_case("x") || key.eq_ignore_ascii_case("width")
                })
            })
            .count();
        let vertical = raw
            .fields()
            .iter()
            .filter(|entry| {
                entry.key.as_deref().is_some_and(|key| {
                    key.eq_ignore_ascii_case("y") || key.eq_ignore_ascii_case("height")
                })
            })
            .count();
        if horizontal != 1 || vertical != 1 {
            return Err(reject(field));
        }
    }
    if let Some(value) = element.raw.first_value_ci("orientation") {
        if value.as_scalar().is_none() || element.orientation.is_none() {
            return Err(reject("orientation"));
        }
    }
    if let Some(value) = element.raw.first_value_ci("hide") {
        let Some(value) = value.as_scalar() else {
            return Err(reject("hide"));
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "no" | "false" => {}
            "yes" | "true" => {
                return Err(AuthenticatedComposeRejection::UnsupportedVisibilityVariant(
                    child_path.to_vec(),
                ))
            }
            _ => return Err(reject("hide")),
        }
    }
    if let Some(value) = element.raw.first_value_ci("scale") {
        if scalar_f32(value).is_none_or(|scale| !scale.is_finite() || scale <= 0.0) {
            return Err(reject("scale"));
        }
    }
    let sprite_fields = element
        .raw
        .fields()
        .iter()
        .filter(|entry| {
            entry.key.as_deref().is_some_and(|key| {
                let folded = key.to_ascii_lowercase();
                folded == "spritetype" || folded == "quadtexturesprite"
            })
        })
        .collect::<Vec<_>>();
    if sprite_fields.len() > 1
        || sprite_fields
            .iter()
            .any(|entry| entry.value.as_scalar().is_none())
    {
        return Err(reject("spriteType"));
    }
    if !sprite_fields.is_empty() && !sprite_family {
        return Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
            child_path.to_vec(),
        ));
    }
    authored_clipping_mode(element, child_path)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthoredClippingMode {
    No,
    YesOrUnspecified,
}

fn authored_clipping_mode(
    element: &GuiElement,
    child_path: &[u32],
) -> std::result::Result<AuthoredClippingMode, AuthenticatedComposeRejection> {
    let Some(value) = element.raw.first_value_ci("clipping") else {
        return Ok(AuthoredClippingMode::YesOrUnspecified);
    };
    let Some(value) = value.as_scalar() else {
        return Err(
            AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                child_path: child_path.to_vec(),
                field: "clipping".into(),
            },
        );
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "no" | "false" => Ok(AuthoredClippingMode::No),
        "yes" | "true" => Ok(AuthoredClippingMode::YesOrUnspecified),
        _ => Err(
            AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                child_path: child_path.to_vec(),
                field: "clipping".into(),
            },
        ),
    }
}

fn intersect_rect(left: Rect, right: Rect) -> Option<Rect> {
    if left.w <= 0 || left.h <= 0 || right.w <= 0 || right.h <= 0 {
        return None;
    }
    let left_right = i64::from(left.x) + i64::from(left.w);
    let left_bottom = i64::from(left.y) + i64::from(left.h);
    let right_right = i64::from(right.x) + i64::from(right.w);
    let right_bottom = i64::from(right.y) + i64::from(right.h);
    let x = i64::from(left.x).max(i64::from(right.x));
    let y = i64::from(left.y).max(i64::from(right.y));
    let end_x = left_right.min(right_right);
    let end_y = left_bottom.min(right_bottom);
    if end_x <= x || end_y <= y {
        return None;
    }
    Some(Rect {
        x: i32::try_from(x).ok()?,
        y: i32::try_from(y).ok()?,
        w: i32::try_from(end_x - x).ok()?,
        h: i32::try_from(end_y - y).ok()?,
    })
}

fn authenticated_content_clip(
    element: &GuiElement,
    clip: Rect,
    child_path: &[u32],
) -> std::result::Result<Rect, AuthenticatedComposeRejection> {
    let Some(value) = element.raw.first_value_ci("margin") else {
        return Ok(clip);
    };
    let entries = value.as_object().ok_or_else(|| {
        AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
            child_path: child_path.to_vec(),
            field: "margin".into(),
        }
    })?;
    let mut margins = BTreeMap::new();
    for entry in entries {
        let key = entry
            .key
            .as_deref()
            .map(str::to_ascii_lowercase)
            .filter(|key| matches!(key.as_str(), "left" | "right" | "top" | "bottom"))
            .ok_or_else(
                || AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                    child_path: child_path.to_vec(),
                    field: "margin".into(),
                },
            )?;
        let pixels = scalar_i32(&entry.value)
            .filter(|value| *value >= 0)
            .ok_or_else(
                || AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                    child_path: child_path.to_vec(),
                    field: "margin".into(),
                },
            )?;
        if margins.insert(key, pixels).is_some() {
            return Err(
                AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                    child_path: child_path.to_vec(),
                    field: "margin".into(),
                },
            );
        }
    }
    let left = margins.get("left").copied().unwrap_or(0);
    let right = margins.get("right").copied().unwrap_or(0);
    let top = margins.get("top").copied().unwrap_or(0);
    let bottom = margins.get("bottom").copied().unwrap_or(0);
    let x = clip.x.checked_add(left).ok_or_else(|| {
        AuthenticatedComposeRejection::UnsupportedLayoutVariant(child_path.to_vec())
    })?;
    let y = clip.y.checked_add(top).ok_or_else(|| {
        AuthenticatedComposeRejection::UnsupportedLayoutVariant(child_path.to_vec())
    })?;
    let w = clip
        .w
        .checked_sub(left)
        .and_then(|value| value.checked_sub(right))
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            AuthenticatedComposeRejection::UnsupportedLayoutVariant(child_path.to_vec())
        })?;
    let h = clip
        .h
        .checked_sub(top)
        .and_then(|value| value.checked_sub(bottom))
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            AuthenticatedComposeRejection::UnsupportedLayoutVariant(child_path.to_vec())
        })?;
    Ok(Rect { x, y, w, h })
}

fn rect_contains_geometry(bounds: Rect, x: i32, y: i32, width: usize, height: usize) -> bool {
    if bounds.w <= 0 || bounds.h <= 0 || width == 0 || height == 0 {
        return false;
    }
    let Ok(width) = i64::try_from(width) else {
        return false;
    };
    let Ok(height) = i64::try_from(height) else {
        return false;
    };
    let x = i64::from(x);
    let y = i64::from(y);
    x >= i64::from(bounds.x)
        && y >= i64::from(bounds.y)
        && x + width <= i64::from(bounds.x) + i64::from(bounds.w)
        && y + height <= i64::from(bounds.y) + i64::from(bounds.h)
}

fn resolve_authenticated_rect(
    element: &GuiElement,
    parent: Rect,
    natural: (usize, usize),
) -> std::result::Result<Rect, String> {
    resolve_authenticated_rect_with_position(element, parent, natural, element.position.as_ref())
}

fn resolve_authenticated_open_rect(
    element: &GuiElement,
    parent: Rect,
    natural: (usize, usize),
    root_open_state: bool,
) -> std::result::Result<Rect, String> {
    let show_position = if root_open_state {
        element
            .raw
            .first_value_ci("show_position")
            .map(|value| {
                crate::gui::parse_measure_pair(value).ok_or_else(|| "show_position".to_string())
            })
            .transpose()?
    } else {
        None
    };
    resolve_authenticated_rect_with_position(
        element,
        parent,
        natural,
        show_position.as_ref().or(element.position.as_ref()),
    )
}

fn resolve_authenticated_rect_with_position(
    element: &GuiElement,
    parent: Rect,
    natural: (usize, usize),
    position: Option<&MeasurePair>,
) -> std::result::Result<Rect, String> {
    let size = element
        .size
        .as_ref()
        .map(|size| {
            Ok::<_, String>((
                authenticated_size_measure_to_pixels(&size.x, parent.w)?,
                authenticated_size_measure_to_pixels(&size.y, parent.h)?,
            ))
        })
        .transpose()?;
    let (w, h) = match size {
        Some(size) => size,
        None => (
            i32::try_from(natural.0).map_err(|_| "natural-width-overflow".to_string())?,
            i32::try_from(natural.1).map_err(|_| "natural-height-overflow".to_string())?,
        ),
    };
    let (x_offset, y_offset) = position
        .map(|position| {
            Ok::<_, String>((
                authenticated_measure_to_pixels(&position.x, parent.w)?,
                authenticated_measure_to_pixels(&position.y, parent.h)?,
            ))
        })
        .transpose()?
        .unwrap_or((0, 0));
    let orientation = Orientation::parse(element.orientation.as_deref());
    let anchor_x = orientation
        .x_num()
        .checked_mul(parent.w)
        .ok_or_else(|| "orientation-x-overflow".to_string())?
        / 2;
    let anchor_y = orientation
        .y_num()
        .checked_mul(parent.h)
        .ok_or_else(|| "orientation-y-overflow".to_string())?
        / 2;
    let x = parent
        .x
        .checked_add(anchor_x)
        .and_then(|value| value.checked_add(x_offset))
        .ok_or_else(|| "position-x-overflow".to_string())?;
    let y = parent
        .y
        .checked_add(anchor_y)
        .and_then(|value| value.checked_add(y_offset))
        .ok_or_else(|| "position-y-overflow".to_string())?;
    Ok(Rect { x, y, w, h })
}

fn authenticated_measure_to_pixels(
    measure: &Measure,
    parent: i32,
) -> std::result::Result<i32, String> {
    let value = if measure.is_percent {
        let percent = measure
            .value
            .as_deref()
            .ok_or_else(|| "malformed-percent".to_string())?
            .parse::<f32>()
            .map_err(|_| "malformed-percent".to_string())?;
        (parent as f32) * percent * 0.01
    } else {
        measure
            .raw
            .trim_end_matches('%')
            .parse::<f32>()
            .map_err(|_| "malformed-measure".to_string())?
    };
    if !value.is_finite() || value < i32::MIN as f32 || value > i32::MAX as f32 {
        return Err("measure-overflow".into());
    }
    Ok(round_half_away_from_zero(value))
}

fn authenticated_size_measure_to_pixels(
    measure: &Measure,
    parent: i32,
) -> std::result::Result<i32, String> {
    if measure.is_percent {
        return authenticated_measure_to_pixels(measure, parent);
    }
    let value = measure
        .raw
        .trim_end_matches('%')
        .parse::<f32>()
        .map_err(|_| "malformed-size".to_string())?;
    if !value.is_finite() || value < i32::MIN as f32 || value > i32::MAX as f32 {
        return Err("size-overflow".into());
    }
    let value = round_half_away_from_zero(value);
    if value < 0 {
        parent
            .checked_add(value)
            .map(|value| value.max(0))
            .ok_or_else(|| "negative-size-overflow".to_string())
    } else {
        Ok(value)
    }
}

fn is_hidden(element: &GuiElement) -> bool {
    element
        .raw
        .first_scalar_ci("hide")
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "yes" | "true"))
}

fn primary_sprite_ref(element: &GuiElement) -> Option<&str> {
    for key in ["quadTextureSprite", "spriteType", "SpriteType"] {
        if let Some(value) = element.raw.first_scalar_ci(key) {
            return Some(value);
        }
    }
    element.sprite_refs.first().map(String::as_str)
}

fn is_progress_bar(sprite: &SpriteDefinition) -> bool {
    sprite.kind.eq_ignore_ascii_case("progressbartype")
        || sprite.kind.eq_ignore_ascii_case("progressBarType")
        || sprite.texture_files.len() >= 2
            && sprite
                .effect_file
                .as_deref()
                .is_some_and(|effect| effect.to_ascii_lowercase().contains("progress_startend"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    UpperLeft,
    UpperRight,
    LowerLeft,
    LowerRight,
    Center,
    CenterLeft,
    CenterRight,
    CenterUp,
    CenterDown,
}

impl Orientation {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw
            .map(str::trim)
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "upper_left" => Orientation::UpperLeft,
            "upper_right" => Orientation::UpperRight,
            "lower_left" => Orientation::LowerLeft,
            "lower_right" => Orientation::LowerRight,
            "center" => Orientation::Center,
            "center_left" => Orientation::CenterLeft,
            "center_right" => Orientation::CenterRight,
            "center_up" => Orientation::CenterUp,
            "center_down" => Orientation::CenterDown,
            _ => Orientation::UpperLeft,
        }
    }

    fn x_num(self) -> i32 {
        match self {
            Orientation::UpperLeft | Orientation::LowerLeft | Orientation::CenterLeft => 0,
            Orientation::Center | Orientation::CenterUp | Orientation::CenterDown => 1,
            Orientation::UpperRight | Orientation::LowerRight | Orientation::CenterRight => 2,
        }
    }

    fn y_num(self) -> i32 {
        match self {
            Orientation::UpperLeft | Orientation::UpperRight | Orientation::CenterUp => 0,
            Orientation::Center | Orientation::CenterLeft | Orientation::CenterRight => 1,
            Orientation::LowerLeft | Orientation::LowerRight | Orientation::CenterDown => 2,
        }
    }

    pub fn anchor(self, parent: Rect) -> (i32, i32) {
        (self.x_num() * parent.w / 2, self.y_num() * parent.h / 2)
    }
}

pub fn resolve_element_rect(
    element: &GuiElement,
    parent: Rect,
    natural: (usize, usize),
    options: &ComposeOptions,
) -> Rect {
    let size = element.size.as_ref().map(|size| {
        (
            size_measure_to_pixels(&size.x, parent.w),
            size_measure_to_pixels(&size.y, parent.h),
        )
    });
    let (w, h) = size.unwrap_or((natural.0 as i32, natural.1 as i32));
    let (x_offset, y_offset) = element
        .position
        .as_ref()
        .map(|pos| {
            (
                measure_to_pixels(&pos.x, parent.w),
                measure_to_pixels(&pos.y, parent.h),
            )
        })
        .unwrap_or((0, 0));
    let orientation = Orientation::parse(element.orientation.as_deref());
    let (anchor_x, anchor_y) = orientation.anchor(parent);

    let mut rect = Rect {
        x: parent.x + anchor_x + x_offset,
        y: parent.y + anchor_y + y_offset,
        w,
        h,
    };
    if element.name.eq_ignore_ascii_case("topbar") && element.size.is_none() {
        rect.w = options.viewport_width as i32;
        rect.h = options.viewport_height as i32;
    }
    rect
}

fn measure_to_pixels(measure: &Measure, parent: i32) -> i32 {
    if measure.is_percent {
        let value = measure
            .value
            .as_deref()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.0);
        round_half_away_from_zero((parent as f32) * value * 0.01)
    } else {
        measure
            .raw
            .trim_end_matches('%')
            .parse::<f32>()
            .map(round_half_away_from_zero)
            .unwrap_or(0)
    }
}

fn round_half_away_from_zero(value: f32) -> i32 {
    if value.is_sign_negative() {
        (value - 0.5).ceil() as i32
    } else {
        (value + 0.5).floor() as i32
    }
}

fn size_measure_to_pixels(measure: &Measure, parent: i32) -> i32 {
    if measure.is_percent {
        return measure_to_pixels(measure, parent);
    }
    let value = measure
        .raw
        .trim_end_matches('%')
        .parse::<f32>()
        .map(round_half_away_from_zero)
        .unwrap_or(0);
    if value < 0 {
        (parent + value).max(0)
    } else {
        value
    }
}

pub fn measure_to_pixels_for_parent(measure: &Measure, parent: i32) -> i32 {
    measure_to_pixels(measure, parent)
}

fn target_size(rect: Rect, natural_w: usize, natural_h: usize) -> (usize, usize) {
    (
        if rect.w > 0 {
            rect.w as usize
        } else {
            natural_w
        },
        if rect.h > 0 {
            rect.h as usize
        } else {
            natural_h
        },
    )
}

fn frame_index(element: &GuiElement, options: &ComposeOptions, frame_count: usize) -> usize {
    if frame_count == 0 {
        return 0;
    }
    let frame = options
        .frame_overrides
        .get(&element.name)
        .copied()
        .or_else(|| {
            scalar_u32_ci(&element.raw, "frame").map(|frame| frame.saturating_sub(1) as usize)
        })
        .unwrap_or(0);
    frame.min(frame_count - 1)
}

fn parse_sprite_size(size: &Size) -> Option<(usize, usize)> {
    let x = size.x.parse::<f32>().ok()?.round().max(0.0) as usize;
    let y = size.y.parse::<f32>().ok()?.round().max(0.0) as usize;
    Some((x, y))
}

fn parse_authenticated_sprite_size(size: &Size) -> std::result::Result<(usize, usize), String> {
    let parse = |raw: &str| -> std::result::Result<usize, String> {
        let value = raw
            .parse::<f32>()
            .map_err(|_| "malformed authenticated sprite size".to_string())?;
        if !value.is_finite() || value < 0.0 || value > usize::MAX as f32 {
            return Err("authenticated sprite size is outside range".to_string());
        }
        Ok(value.round() as usize)
    };
    Ok((parse(&size.x)?, parse(&size.y)?))
}

fn authenticated_text_border(
    element: &GuiElement,
    child_path: &[u32],
) -> std::result::Result<EngineTextBorder, AuthenticatedComposeRejection> {
    let values = element.raw.values_ci("borderSize").collect::<Vec<_>>();
    if values.len() > 1 {
        return Err(
            AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                child_path: child_path.to_vec(),
                field: "borderSize".to_string(),
            },
        );
    }
    let Some(value) = values.first() else {
        return Ok(EngineTextBorder::ExplicitlyAbsent);
    };
    let size = crate::gfx::parse_size(value).ok_or_else(|| {
        AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
            child_path: child_path.to_vec(),
            field: "borderSize".to_string(),
        }
    })?;
    let (x, y) = parse_authenticated_sprite_size(&size).map_err(|_| {
        AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
            child_path: child_path.to_vec(),
            field: "borderSize".to_string(),
        }
    })?;
    Ok(EngineTextBorder::Authored {
        x: i32::try_from(x).map_err(|_| {
            AuthenticatedComposeRejection::UnsupportedTextVariant(child_path.to_vec())
        })?,
        y: i32::try_from(y).map_err(|_| {
            AuthenticatedComposeRejection::UnsupportedTextVariant(child_path.to_vec())
        })?,
    })
}

fn authenticated_text_line_alignment(
    rect_width: i32,
    line_width: i32,
    border: EngineTextBorder,
    align: TextAlign,
) -> std::result::Result<i32, AuthenticatedComposeRejection> {
    let border_x = match border {
        EngineTextBorder::Authored { x, .. } => x,
        EngineTextBorder::ExplicitlyAbsent => 0,
    };
    let available = rect_width.saturating_sub(border_x.saturating_mul(2));
    let remainder = available.saturating_sub(line_width);
    Ok(match align {
        TextAlign::Left => 0,
        TextAlign::Center => remainder / 2,
        TextAlign::Right => remainder,
    })
}

fn authenticated_text_shader_rejection(
    child_path: &[u32],
    reason: TextShaderRejection,
) -> AuthenticatedComposeRejection {
    AuthenticatedComposeRejection::TextDraw {
        child_path: child_path.to_vec(),
        message: match reason {
            TextShaderRejection::PackedGlyphChannelUnproved => {
                "packed glyph channel resource swizzle is unproved".to_string()
            }
            TextShaderRejection::PageAbsent { page } => {
                format!("authenticated font atlas page {page} is absent")
            }
        },
    }
}

fn parse_engine_text_border(value: &Value) -> Option<(i32, i32)> {
    let x = value.first_value_ci("x").and_then(scalar_i32)?;
    let y = value.first_value_ci("y").and_then(scalar_i32)?;
    Some((x, y))
}

fn scalar_i32_ci(value: &Value, key: &str) -> Option<i32> {
    value.first_value_ci(key).and_then(scalar_i32)
}

fn scalar_u32_ci(value: &Value, key: &str) -> Option<u32> {
    value.first_value_ci(key).and_then(scalar_u32)
}

fn scalar_f32_ci(value: &Value, key: &str) -> Option<f32> {
    value.first_value_ci(key).and_then(scalar_f32)
}

fn raw_contains_mask_carrier(value: &Value) -> bool {
    value.fields().iter().any(|entry| {
        entry
            .key
            .as_deref()
            .is_some_and(|key| key.to_ascii_lowercase().contains("mask"))
            || raw_contains_mask_carrier(&entry.value)
    })
}

fn raw_contains_variable_reference(value: &Value) -> bool {
    if value
        .as_scalar()
        .is_some_and(|value| value.trim_start().starts_with('@'))
    {
        return true;
    }
    value.fields().iter().any(|entry| {
        entry
            .key
            .as_deref()
            .is_some_and(|key| key.trim_start().starts_with('@'))
            || raw_contains_variable_reference(&entry.value)
    })
}

fn first_unproved_authenticated_field(
    element: &GuiElement,
    family: WidgetFamily,
) -> Option<String> {
    element.raw.fields().iter().find_map(|entry| {
        let key = entry.key.as_deref()?;
        let folded = key.to_ascii_lowercase();
        let scalar_field = matches!(
            folded.as_str(),
            "name" | "position" | "size" | "orientation" | "clipping" | "hide" | "scale" | "frame"
        ) || folded == "spritetype"
            || folded == "quadtexturesprite"
            || matches!(
                folded.as_str(),
                "show_position"
                    | "hide_position"
                    | "show_animation_type"
                    | "hide_animation_type"
                    | "animation_time"
                    | "show_sound"
                    | "hide_sound"
            )
            || matches!(
                family,
                WidgetFamily::Button | WidgetFamily::GuiButton | WidgetFamily::Checkbox
            ) && matches!(
                folded.as_str(),
                "buttonfont"
                    | "shortcut"
                    | "clicksound"
                    | "pdx_tooltip"
                    | "pdx_tooltip_delayed"
                    | "alwaystransparent"
            )
            || matches!(
                family,
                WidgetFamily::InstantText | WidgetFamily::Text | WidgetFamily::EditBox
            ) && matches!(
                folded.as_str(),
                "text" | "font" | "buttonfont" | "texttype" | "format" | "bordersize"
            )
            || matches!(
                family,
                WidgetFamily::List
                    | WidgetFamily::SmoothList
                    | WidgetFamily::Grid
                    | WidgetFamily::Overlap
                    | WidgetFamily::Container
            ) && matches!(
                folded.as_str(),
                "spacing"
                    | "layout"
                    | "scroll_wheel_factor"
                    | "smooth_scrolling"
                    | "margin"
                    | "verticalscrollbar"
                    | "vertical_scroll_step"
            )
            || matches!(family, WidgetFamily::Grid)
                && matches!(
                    folded.as_str(),
                    "format"
                        | "slotsize"
                        | "max_slots_horizontal"
                        | "max_slots_vertical"
                        | "add_horizontal"
                );
        let parsed_child =
            entry.value.as_object().is_some() && WidgetFamily::from_element_type(key).is_some();
        (!scalar_field && !parsed_child).then(|| key.to_string())
    })
}

fn raw_has_top_level_fragment(value: &Value, fragments: &[&str]) -> bool {
    value.fields().iter().any(|entry| {
        entry.key.as_deref().is_some_and(|key| {
            let key = key.to_ascii_lowercase();
            if key == "clipping" {
                return false;
            }
            fragments.iter().any(|fragment| key.contains(fragment))
        })
    })
}

fn validate_xui23_animation_shape(value: &Value, owner: &str) -> Result<()> {
    const SCALAR_FIELDS: [&str; 8] = [
        "animationmaskfile",
        "animationtexturefile",
        "animationtime",
        "animationdelay",
        "animationrotation",
        "animationlooping",
        "animationtype",
        "animationblendmode",
    ];
    const PAIR_FIELDS: [&str; 2] = ["animationrotationoffset", "animationtexturescale"];

    let entries = value.as_object().ok_or_else(|| {
        GameUiError::parse(format!("{owner} animation is not an authored object"))
    })?;
    let mut seen = BTreeSet::new();
    for entry in entries {
        let key = entry.key.as_deref().ok_or_else(|| {
            GameUiError::parse(format!(
                "{owner} animation contains an unsupported positional member"
            ))
        })?;
        let folded = key.to_ascii_lowercase();
        if !SCALAR_FIELDS.contains(&folded.as_str())
            && !PAIR_FIELDS.contains(&folded.as_str())
            && folded != "animationframes"
        {
            return Err(GameUiError::parse(format!(
                "{owner} animation contains unsupported authored field {key}"
            )));
        }
        if !seen.insert(folded.clone()) {
            return Err(GameUiError::parse(format!(
                "{owner} animation repeats authored field {key}"
            )));
        }
        if SCALAR_FIELDS.contains(&folded.as_str()) {
            if entry.value.as_scalar().is_none() {
                return Err(GameUiError::parse(format!(
                    "{owner} animation {key} is not a scalar"
                )));
            }
        } else if PAIR_FIELDS.contains(&folded.as_str()) {
            validate_xui23_animation_pair_shape(&entry.value, owner, key)?;
        } else {
            validate_xui23_animation_frames_shape(&entry.value, owner)?;
        }
    }
    Ok(())
}

fn validate_xui23_animation_frames_shape(value: &Value, owner: &str) -> Result<()> {
    let entries = value.as_object().ok_or_else(|| {
        GameUiError::parse(format!(
            "{owner} animation animationframes is not an authored positional list"
        ))
    })?;
    for entry in entries {
        if entry.key.is_some() || entry.value.as_scalar().is_none() {
            return Err(GameUiError::parse(format!(
                "{owner} animation animationframes contains a non-positional integer member"
            )));
        }
        entry
            .value
            .as_scalar()
            .expect("scalar checked")
            .parse::<i32>()
            .map_err(|_| {
                GameUiError::parse(format!(
                    "{owner} animation animationframes contains a non-i32 frame"
                ))
            })?;
    }
    Ok(())
}

fn xui23_animation_frames_mask(value: &Value, owner: &str) -> Result<u32> {
    let Some(frames) = value.first_value_ci("animationframes") else {
        return Ok(u32::MAX);
    };
    let entries = frames.as_object().ok_or_else(|| {
        GameUiError::parse(format!(
            "{owner} animation animationframes is not an authored positional list"
        ))
    })?;
    if entries.is_empty() {
        return Ok(u32::MAX);
    }
    let mut mask = 0u32;
    for entry in entries {
        let frame = entry
            .value
            .as_scalar()
            .ok_or_else(|| {
                GameUiError::parse(format!(
                    "{owner} animation animationframes contains a non-scalar frame"
                ))
            })?
            .parse::<i32>()
            .map_err(|_| {
                GameUiError::parse(format!(
                    "{owner} animation animationframes contains a non-i32 frame"
                ))
            })?;
        mask |= 1u32.wrapping_shl(frame as u32);
    }
    Ok(mask)
}

fn xui23_animation_applies_to_frame(
    value: &Value,
    owner: &str,
    one_based_frame: usize,
) -> Result<bool> {
    let frame = u32::try_from(one_based_frame).map_err(|_| {
        GameUiError::parse(format!(
            "{owner} selected sprite frame exceeds exact animation mask width"
        ))
    })?;
    let mask = xui23_animation_frames_mask(value, owner)?;
    Ok(mask == u32::MAX || mask & 1u32.wrapping_shl(frame) != 0)
}

fn xui23_first_two_applicable_animations<'a>(
    animations: &[&'a Value],
    owner: &str,
    one_based_frame: usize,
) -> Result<Vec<&'a Value>> {
    let mut applicable = Vec::with_capacity(animations.len().min(2));
    for &animation in animations {
        if xui23_animation_applies_to_frame(animation, owner, one_based_frame)? {
            applicable.push(animation);
            if applicable.len() == 2 {
                break;
            }
        }
    }
    Ok(applicable)
}

fn validate_xui23_animation_pair_shape(value: &Value, owner: &str, field: &str) -> Result<()> {
    let entries = value.as_object().ok_or_else(|| {
        GameUiError::parse(format!("{owner} animation {field} is not an x/y object"))
    })?;
    let mut seen = BTreeSet::new();
    for entry in entries {
        let key = entry.key.as_deref().ok_or_else(|| {
            GameUiError::parse(format!(
                "{owner} animation {field} contains an unsupported positional member"
            ))
        })?;
        let folded = key.to_ascii_lowercase();
        if folded != "x" && folded != "y" {
            return Err(GameUiError::parse(format!(
                "{owner} animation {field} contains unsupported authored field {key}"
            )));
        }
        if !seen.insert(folded) {
            return Err(GameUiError::parse(format!(
                "{owner} animation {field} repeats authored field {key}"
            )));
        }
        if entry.value.as_scalar().is_none() {
            return Err(GameUiError::parse(format!(
                "{owner} animation {field}.{key} is not a scalar"
            )));
        }
    }
    Ok(())
}

fn required_scalar_ci<'a>(value: &'a Value, key: &str, owner: &str) -> Result<&'a str> {
    value
        .first_scalar_ci(key)
        .ok_or_else(|| GameUiError::parse(format!("{owner} animation lacks authored {key}")))
}

fn required_f32_ci(value: &Value, key: &str, owner: &str) -> Result<f32> {
    required_scalar_ci(value, key, owner)?
        .parse::<f32>()
        .map_err(|_| GameUiError::parse(format!("{owner} animation {key} is not f32")))
}

fn required_xy_ci(value: &Value, key: &str, owner: &str) -> Result<[f32; 2]> {
    let pair = value
        .first_value_ci(key)
        .ok_or_else(|| GameUiError::parse(format!("{owner} animation lacks authored {key}")))?;
    let x = pair
        .first_scalar_ci("x")
        .ok_or_else(|| GameUiError::parse(format!("{owner} animation {key}.x is absent")))?
        .parse::<f32>()
        .map_err(|_| GameUiError::parse(format!("{owner} animation {key}.x is not f32")))?;
    let y = pair
        .first_scalar_ci("y")
        .ok_or_else(|| GameUiError::parse(format!("{owner} animation {key}.y is absent")))?
        .parse::<f32>()
        .map_err(|_| GameUiError::parse(format!("{owner} animation {key}.y is not f32")))?;
    Ok([x, y])
}

fn required_yes_no_ci(value: &Value, key: &str, owner: &str) -> Result<bool> {
    match required_scalar_ci(value, key, owner)?
        .to_ascii_lowercase()
        .as_str()
    {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(GameUiError::parse(format!(
            "{owner} animation {key} is not yes/no"
        ))),
    }
}

fn validate_xui23_scrolling_add_animation(value: &Value, owner: &str) -> Result<()> {
    required_scalar_ci(value, "animationmaskfile", owner)?;
    required_scalar_ci(value, "animationtexturefile", owner)?;
    let animation_time = required_f32_ci(value, "animationtime", owner)?;
    let animation_delay = required_f32_ci(value, "animationdelay", owner)?;
    let rotation = required_f32_ci(value, "animationrotation", owner)?;
    let rotation_offset = required_xy_ci(value, "animationrotationoffset", owner)?;
    let texture_scale = required_xy_ci(value, "animationtexturescale", owner)?;
    let looping = required_yes_no_ci(value, "animationlooping", owner)?;
    let animation_type = required_scalar_ci(value, "animationtype", owner)?;
    let blend_mode = required_scalar_ci(value, "animationblendmode", owner)?;
    if !animation_time.is_finite()
        || animation_time <= 0.0
        || !animation_delay.is_finite()
        || animation_delay < 0.0
        || rotation != 0.0
        || rotation_offset != [0.0, 0.0]
        || texture_scale != [1.0, 1.0]
        || !looping
        || !animation_type.eq_ignore_ascii_case("scrolling")
        || !blend_mode.eq_ignore_ascii_case("add")
    {
        return Err(GameUiError::parse(format!(
            "animated sprite {owner} is outside the exact XUI23 scrolling/add law"
        )));
    }
    Ok(())
}

fn progress_bar_horizontal(sprite: &SpriteDefinition) -> Result<bool> {
    let values = sprite.raw.values_ci("horizontal").collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(true),
        [value] => match value.as_scalar().map(str::trim) {
            Some(value) if value.eq_ignore_ascii_case("yes") => Ok(true),
            Some(value) if value.eq_ignore_ascii_case("no") => Ok(false),
            _ => Err(GameUiError::parse(format!(
                "progress bar {} horizontal is not an authored yes/no scalar",
                sprite.name
            ))),
        },
        _ => Err(GameUiError::parse(format!(
            "progress bar {} repeats horizontal",
            sprite.name
        ))),
    }
}

fn image_pixel_f32(image: &RgbaImage, x: usize, y: usize) -> RgbaF32 {
    let offset = (y * image.width + x) * 4;
    RgbaF32(std::array::from_fn(|channel| {
        image.pixels[offset + channel] as f32 / 255.0
    }))
}

fn set_image_pixel_f32(image: &mut RgbaImage, x: usize, y: usize, pixel: RgbaF32) {
    let offset = (y * image.width + x) * 4;
    for channel in 0..4 {
        image.pixels[offset + channel] = (pixel.0[channel].clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

fn sample_linear(image: &RgbaImage, u: f32, v: f32, wrap: bool) -> RgbaF32 {
    if image.width == 0 || image.height == 0 {
        return RgbaF32([0.0; 4]);
    }
    let normalize = |coordinate: f32| {
        if wrap {
            coordinate.rem_euclid(1.0)
        } else {
            coordinate.clamp(0.0, 1.0)
        }
    };
    let x = normalize(u) * image.width as f32 - 0.5;
    let y = normalize(v) * image.height as f32 - 0.5;
    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let tx = x - x.floor();
    let ty = y - y.floor();
    let index = |raw: i64, len: usize| -> usize {
        if wrap {
            raw.rem_euclid(len as i64) as usize
        } else {
            raw.clamp(0, len as i64 - 1) as usize
        }
    };
    let p00 = image_pixel_f32(image, index(x0, image.width), index(y0, image.height));
    let p10 = image_pixel_f32(image, index(x0 + 1, image.width), index(y0, image.height));
    let p01 = image_pixel_f32(image, index(x0, image.width), index(y0 + 1, image.height));
    let p11 = image_pixel_f32(
        image,
        index(x0 + 1, image.width),
        index(y0 + 1, image.height),
    );
    RgbaF32(std::array::from_fn(|channel| {
        let top = p00.0[channel] + (p10.0[channel] - p00.0[channel]) * tx;
        let bottom = p01.0[channel] + (p11.0[channel] - p01.0[channel]) * tx;
        top + (bottom - top) * ty
    }))
}

pub fn draw_image(dst: &mut RgbaImage, src: &RgbaImage, x: i32, y: i32) {
    for sy in 0..src.height {
        let dy = y + sy as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        for sx in 0..src.width {
            let dx = x + sx as i32;
            if dx < 0 || dx >= dst.width as i32 {
                continue;
            }
            let src_offset = (sy * src.width + sx) * 4;
            blend_at(
                dst,
                dx as usize,
                dy as usize,
                src.pixels[src_offset..src_offset + 4]
                    .try_into()
                    .expect("slice length checked"),
            );
        }
    }
}

fn draw_image_clipped(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    x: i32,
    y: i32,
    clip: Option<[i32; 4]>,
) {
    let Some([clip_x, clip_y, clip_w, clip_h]) = clip else {
        draw_image(dst, src, x, y);
        return;
    };
    if clip_w <= 0 || clip_h <= 0 {
        return;
    }
    for sy in 0..src.height {
        let dy = y + sy as i32;
        if dy < 0 || dy >= dst.height as i32 || dy < clip_y || dy >= clip_y.saturating_add(clip_h) {
            continue;
        }
        for sx in 0..src.width {
            let dx = x + sx as i32;
            if dx < 0
                || dx >= dst.width as i32
                || dx < clip_x
                || dx >= clip_x.saturating_add(clip_w)
            {
                continue;
            }
            let src_offset = (sy * src.width + sx) * 4;
            blend_at(
                dst,
                dx as usize,
                dy as usize,
                src.pixels[src_offset..src_offset + 4]
                    .try_into()
                    .expect("slice length checked"),
            );
        }
    }
}

pub fn draw_rect_outline(dst: &mut RgbaImage, rect: Rect, rgba: [u8; 4]) {
    if rect.w <= 0 || rect.h <= 0 || dst.width == 0 || dst.height == 0 {
        return;
    }
    let x0 = rect.x.max(0);
    let y0 = rect.y.max(0);
    let x1 = (rect.x + rect.w - 1).min(dst.width as i32 - 1);
    let y1 = (rect.y + rect.h - 1).min(dst.height as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    for x in x0..=x1 {
        blend_at(dst, x as usize, y0 as usize, rgba);
        blend_at(dst, x as usize, y1 as usize, rgba);
    }
    for y in y0..=y1 {
        blend_at(dst, x0 as usize, y as usize, rgba);
        blend_at(dst, x1 as usize, y as usize, rgba);
    }
}

pub fn fill_rect(dst: &mut RgbaImage, rect: Rect, rgba: [u8; 4]) {
    if rect.w <= 0 || rect.h <= 0 || dst.width == 0 || dst.height == 0 {
        return;
    }
    let x0 = rect.x.max(0);
    let y0 = rect.y.max(0);
    let x1 = (rect.x + rect.w).min(dst.width as i32);
    let y1 = (rect.y + rect.h).min(dst.height as i32);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            blend_at(dst, x as usize, y as usize, rgba);
        }
    }
}

pub fn draw_scaled(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    x: i32,
    y: i32,
    target_w: usize,
    target_h: usize,
) {
    draw_scaled_crop(dst, src, x, y, target_w, target_h, target_w);
}

pub fn draw_scaled_crop(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    x: i32,
    y: i32,
    target_w: usize,
    target_h: usize,
    visible_w: usize,
) {
    if target_w == 0 || target_h == 0 || visible_w == 0 || src.width == 0 || src.height == 0 {
        return;
    }
    let visible_w = visible_w.min(target_w);
    for dy_local in 0..target_h {
        let dy = y + dy_local as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        let sy = dy_local * src.height / target_h;
        for dx_local in 0..visible_w {
            let dx = x + dx_local as i32;
            if dx < 0 || dx >= dst.width as i32 {
                continue;
            }
            let sx = dx_local * src.width / target_w;
            let src_offset = (sy * src.width + sx) * 4;
            blend_at(
                dst,
                dx as usize,
                dy as usize,
                src.pixels[src_offset..src_offset + 4]
                    .try_into()
                    .expect("slice length checked"),
            );
        }
    }
}

fn draw_progress_normal_axis(
    dst: &mut RgbaImage,
    front: &RgbaImage,
    back: &RgbaImage,
    x: i32,
    y: i32,
    target_w: usize,
    target_h: usize,
    current_state: f32,
    horizontal: bool,
) {
    if target_w == 0
        || target_h == 0
        || front.width == 0
        || front.height == 0
        || back.width == 0
        || back.height == 0
    {
        return;
    }
    for dy_local in 0..target_h {
        let dy = y + dy_local as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        for dx_local in 0..target_w {
            let dx = x + dx_local as i32;
            if dx < 0 || dx >= dst.width as i32 {
                continue;
            }
            let progress_coordinate = if horizontal {
                (dx_local as f32 + 0.5) / target_w as f32
            } else {
                (dy_local as f32 + 0.5) / target_h as f32
            };
            let src = if progress_coordinate <= current_state {
                front
            } else {
                back
            };
            let (sx, sy) = if horizontal {
                (
                    dx_local * src.width / target_w,
                    dy_local * src.height / target_h,
                )
            } else {
                (
                    dy_local * src.width / target_h,
                    dx_local * src.height / target_w,
                )
            };
            let src_offset = (sy * src.width + sx) * 4;
            blend_at(
                dst,
                dx as usize,
                dy as usize,
                src.pixels[src_offset..src_offset + 4]
                    .try_into()
                    .expect("slice length checked"),
            );
        }
    }
}

pub fn progress_minmax_uses_front(u: f32, current_state: f32) -> bool {
    (u < 0.5 && u >= current_state) || (u >= 0.5 && u <= current_state)
}

fn draw_progress_minmax_axis(
    dst: &mut RgbaImage,
    front: &RgbaImage,
    back: &RgbaImage,
    x: i32,
    y: i32,
    target_w: usize,
    target_h: usize,
    current_state: f32,
    horizontal: bool,
) {
    if target_w == 0
        || target_h == 0
        || front.width == 0
        || front.height == 0
        || back.width == 0
        || back.height == 0
    {
        return;
    }
    for dy_local in 0..target_h {
        let dy = y + dy_local as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        for dx_local in 0..target_w {
            let dx = x + dx_local as i32;
            if dx < 0 || dx >= dst.width as i32 {
                continue;
            }
            let progress_coordinate = if horizontal {
                (dx_local as f32 + 0.5) / target_w as f32
            } else {
                (dy_local as f32 + 0.5) / target_h as f32
            };
            let src = if progress_minmax_uses_front(progress_coordinate, current_state) {
                front
            } else {
                back
            };
            let (sx, sy) = if horizontal {
                (
                    dx_local * src.width / target_w,
                    dy_local * src.height / target_h,
                )
            } else {
                (
                    dy_local * src.width / target_h,
                    dx_local * src.height / target_w,
                )
            };
            let src_offset = (sy * src.width + sx) * 4;
            blend_at(
                dst,
                dx as usize,
                dy as usize,
                src.pixels[src_offset..src_offset + 4]
                    .try_into()
                    .expect("slice length checked"),
            );
        }
    }
}

pub fn draw_nine_slice(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    x: i32,
    y: i32,
    target_w: usize,
    target_h: usize,
    border_x: usize,
    border_y: usize,
) {
    if border_x == 0 && border_y == 0 {
        draw_scaled(dst, src, x, y, target_w, target_h);
        return;
    }
    let bx = border_x.min(src.width / 2).min(target_w / 2);
    let by = border_y.min(src.height / 2).min(target_h / 2);
    let src_cols = [
        (0, bx),
        (bx, src.width.saturating_sub(2 * bx)),
        (src.width - bx, bx),
    ];
    let src_rows = [
        (0, by),
        (by, src.height.saturating_sub(2 * by)),
        (src.height - by, by),
    ];
    let dst_cols = [
        (0, bx),
        (bx, target_w.saturating_sub(2 * bx)),
        (target_w - bx, bx),
    ];
    let dst_rows = [
        (0, by),
        (by, target_h.saturating_sub(2 * by)),
        (target_h - by, by),
    ];

    for row in 0..3 {
        for col in 0..3 {
            let (sx, sw) = src_cols[col];
            let (sy, sh) = src_rows[row];
            let (dx, dw) = dst_cols[col];
            let (dy, dh) = dst_rows[row];
            if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
                continue;
            }
            draw_region_scaled(
                dst,
                src,
                (sx, sy, sw, sh),
                x + dx as i32,
                y + dy as i32,
                dw,
                dh,
            );
        }
    }
}

fn draw_region_scaled(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    region: (usize, usize, usize, usize),
    x: i32,
    y: i32,
    target_w: usize,
    target_h: usize,
) {
    let (src_x, src_y, src_w, src_h) = region;
    for dy_local in 0..target_h {
        let dy = y + dy_local as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        let sy = src_y + dy_local * src_h / target_h;
        for dx_local in 0..target_w {
            let dx = x + dx_local as i32;
            if dx < 0 || dx >= dst.width as i32 {
                continue;
            }
            let sx = src_x + dx_local * src_w / target_w;
            let src_offset = (sy * src.width + sx) * 4;
            blend_at(
                dst,
                dx as usize,
                dy as usize,
                src.pixels[src_offset..src_offset + 4]
                    .try_into()
                    .expect("slice length checked"),
            );
        }
    }
}

fn blend_at(dst: &mut RgbaImage, x: usize, y: usize, src: [u8; 4]) {
    let dst_offset = (y * dst.width + x) * 4;
    let dst_px = [
        dst.pixels[dst_offset],
        dst.pixels[dst_offset + 1],
        dst.pixels[dst_offset + 2],
        dst.pixels[dst_offset + 3],
    ];
    let out = alpha_blend(dst_px, src);
    dst.pixels[dst_offset..dst_offset + 4].copy_from_slice(&out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradox_text::{Entry, Scalar};
    use portable_content_manifest::{ContentLayerKind, UsedContentKey};

    fn scalar_field(key: &str, value: &str) -> Entry {
        Entry {
            key: Some(key.into()),
            value: Value::Scalar(Scalar::Bare(value.into())),
            line: 1,
            column: 1,
        }
    }

    fn cache_content(path: &str, payload: char) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "b".repeat(64),
            payload_sha256: payload.to_string().repeat(64),
            payload_bytes: 1,
        }
    }

    fn authored_text_effect_row() -> UsedContentKey {
        let mut row = cache_content(
            &portable_content_manifest::normalize_game_relative_path(
                portable_content_manifest::authored_ui::AUTHORED_BITMAP_FONT_TEXT_EFFECT_PATH,
            )
            .expect("the authored text-effect path normalizes"),
            '9',
        );
        row.payload_sha256 = TEXT_SHADER_SHA256.to_string();
        row
    }

    /// Kills: comparing `normalized_game_relative_path` against the authored
    /// `gfx/FX/text.shader` spelling verbatim. Production only ever stores the
    /// lower-cased key, so the verbatim guard rejects every authenticated font
    /// role while a fixture written in the authored spelling still passes.
    #[test]
    fn the_text_effect_guard_accepts_the_spelling_production_stores() {
        let row = authored_text_effect_row();
        assert_ne!(
            row.normalized_game_relative_path,
            portable_content_manifest::authored_ui::AUTHORED_BITMAP_FONT_TEXT_EFFECT_PATH,
            "the authored constant must differ from its normalized key or this proves nothing"
        );
        assert!(text_effect_row_is_the_authored_shader(
            &row,
            TEXT_SHADER_BYTES as usize
        ));
    }

    /// Kills: accepting any spelling at all, or any payload. The row has to be
    /// the authored shader by key, digest and length.
    #[test]
    fn the_text_effect_guard_rejects_a_row_that_is_not_the_authored_shader() {
        let authored_spelling = {
            let mut row = authored_text_effect_row();
            row.normalized_game_relative_path =
                portable_content_manifest::authored_ui::AUTHORED_BITMAP_FONT_TEXT_EFFECT_PATH
                    .to_string();
            row
        };
        assert!(!text_effect_row_is_the_authored_shader(
            &authored_spelling,
            TEXT_SHADER_BYTES as usize
        ));
        let wrong_digest = {
            let mut row = authored_text_effect_row();
            row.payload_sha256 = "c".repeat(64);
            row
        };
        assert!(!text_effect_row_is_the_authored_shader(
            &wrong_digest,
            TEXT_SHADER_BYTES as usize
        ));
        assert!(!text_effect_row_is_the_authored_shader(
            &authored_text_effect_row(),
            TEXT_SHADER_BYTES as usize + 1
        ));
    }

    fn cache_sprite_and_role() -> (
        SpriteDefinition,
        crate::authored_role_index::AuthoredSpriteRole,
    ) {
        const CAPTURED_TEXTURE_PATH: &str = "gfx/interface/mapmode/mapmode_bottom_bg.dds";
        const CAPTURED_TEXTURE_SHA256: &str =
            "f1de475b5ea073936664855f59c0a52f81e8819a6af0062ff45e92385c49f384";
        let sprite = SpriteDefinition {
            kind: "spriteType".into(),
            name: "GFX_test".into(),
            texture_files: vec![crate::gfx::TextureSlot {
                key: "texturefile".into(),
                path: CAPTURED_TEXTURE_PATH.into(),
            }],
            no_of_frames: 1,
            effect_file: None,
            size: None,
            border_size: None,
            source_file: "interface/test.gfx".into(),
            raw: Value::Object(vec![
                scalar_field("name", "GFX_test"),
                scalar_field("textureFile", CAPTURED_TEXTURE_PATH),
                scalar_field("color", "0xffffffff"),
            ]),
        };
        let role = crate::authored_role_index::AuthoredSpriteRole {
            sprite_name: sprite.name.clone(),
            sprite_kind: sprite.kind.clone(),
            frame_count: 1,
            authored_effect_file: None,
            effective_shader_file: "gfx/FX/buttonstate.shader".into(),
            definition_content: cache_content("interface/test.gfx", 'c'),
            texture_slots: vec![crate::authored_role_index::AuthoredSpriteTextureRole {
                authored_slot_key: "texturefile".into(),
                authored_path: CAPTURED_TEXTURE_PATH.into(),
                content: UsedContentKey {
                    payload_sha256: CAPTURED_TEXTURE_SHA256.into(),
                    payload_bytes: 20_424,
                    ..cache_content(CAPTURED_TEXTURE_PATH, 'd')
                },
            }],
            effect_content: cache_content("gfx/FX/buttonstate.shader", 'e'),
            effect_include_content: vec![cache_content("gfx/FX/buttonstate.fxh", 'f')],
            authored_color_rgba_f32_bits: [1.0f32.to_bits(); 4],
            authored_color_was_explicit: true,
        };
        (sprite, role)
    }

    #[test]
    fn authenticated_sprite_subset_admits_exact_frames_and_color_and_refuses_mismatches() {
        let (sprite, role) = cache_sprite_and_role();
        assert_eq!(
            validate_authenticated_sprite_executable_subset(&sprite, &role, &[4]),
            Ok(())
        );

        let mut default_white = sprite.clone();
        default_white.raw = Value::Object(
            default_white
                .raw
                .fields()
                .iter()
                .filter(|entry| entry.key.as_deref() != Some("color"))
                .cloned()
                .collect(),
        );
        let mut default_white_role = role.clone();
        default_white_role.authored_color_was_explicit = false;
        assert_eq!(
            validate_authenticated_sprite_executable_subset(
                &default_white,
                &default_white_role,
                &[4]
            ),
            Ok(())
        );

        let mut frame_two = sprite.clone();
        frame_two.no_of_frames = 2;
        frame_two.raw = Value::Object(
            frame_two
                .raw
                .fields()
                .iter()
                .cloned()
                .chain(std::iter::once(scalar_field("noOfFrames", "2")))
                .collect(),
        );
        let mut frame_two_role = role.clone();
        frame_two_role.frame_count = 2;
        assert_eq!(
            validate_authenticated_sprite_executable_subset(&frame_two, &frame_two_role, &[4]),
            Ok(())
        );

        let mut progress_kind = sprite.clone();
        progress_kind.kind = "progressBarType".into();
        assert_eq!(
            validate_authenticated_sprite_executable_subset(&progress_kind, &role, &[4]),
            Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                vec![4]
            ))
        );

        let mut blendframes = role.clone();
        blendframes.effective_shader_file = "gfx/FX/buttonstate_blendframes.shader".into();
        assert_eq!(
            validate_authenticated_sprite_executable_subset(&sprite, &blendframes, &[4]),
            Ok(())
        );

        let mut bordered = sprite.clone();
        bordered.border_size = Some(Size {
            x: "1".into(),
            y: "1".into(),
        });
        assert_eq!(
            validate_authenticated_sprite_executable_subset(&bordered, &role, &[4]),
            Err(AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(vec![4]))
        );

        let mut unknown_pixel_field = sprite.clone();
        unknown_pixel_field.raw = Value::Object(
            unknown_pixel_field
                .raw
                .fields()
                .iter()
                .cloned()
                .chain(std::iter::once(scalar_field("frameDirectionY", "yes")))
                .collect(),
        );
        assert_eq!(
            validate_authenticated_sprite_executable_subset(&unknown_pixel_field, &role, &[4]),
            Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                vec![4]
            ))
        );

        for unproved_metadata in [
            "transparencecheck",
            "alwaystransparent",
            "legacy_lazy_load",
            "clicksound",
            "loadType",
        ] {
            let mut candidate = sprite.clone();
            candidate.raw = Value::Object(
                candidate
                    .raw
                    .fields()
                    .iter()
                    .cloned()
                    .chain(std::iter::once(scalar_field(unproved_metadata, "yes")))
                    .collect(),
            );
            assert_eq!(
                validate_authenticated_sprite_executable_subset(&candidate, &role, &[4]),
                Err(AuthenticatedComposeRejection::SpriteDefinitionMismatch(
                    vec![4]
                ))
            );
        }

        let mut colored = sprite.clone();
        colored.raw = Value::Object(
            colored
                .raw
                .fields()
                .iter()
                .map(|entry| {
                    if entry.key.as_deref() == Some("color") {
                        scalar_field("color", "0xff00ffff")
                    } else {
                        entry.clone()
                    }
                })
                .collect(),
        );
        let mut colored_role = role.clone();
        colored_role.authored_color_rgba_f32_bits =
            crate::gfx::a729_sprite_color_rgba_f32_bits(&colored.raw)
                .expect("exact authored color");
        assert_eq!(
            validate_authenticated_sprite_executable_subset(&colored, &colored_role, &[4]),
            Ok(())
        );
        assert_eq!(
            validate_authenticated_sprite_executable_subset(&colored, &role, &[4]),
            Err(AuthenticatedComposeRejection::UnsupportedColorOpacityOrClipVariant(vec![4]))
        );
    }

    #[test]
    fn authenticated_sprite_cache_cannot_reuse_legacy_or_changed_role_pixels() {
        let (sprite, role) = cache_sprite_and_role();
        let baseline = authenticated_sprite_cache_key_material(
            &"1".repeat(64),
            &"2".repeat(64),
            &"3".repeat(64),
            &sprite,
            &role,
        )
        .expect("sealed cache key");
        assert!(baseline.starts_with("authenticated:v2:"));
        assert_ne!(baseline, sprite.name.to_ascii_lowercase());

        let mut changed_effect = role.clone();
        changed_effect.effect_content.payload_sha256 = "0".repeat(64);
        let changed_effect_key = authenticated_sprite_cache_key_material(
            &"1".repeat(64),
            &"2".repeat(64),
            &"3".repeat(64),
            &sprite,
            &changed_effect,
        )
        .expect("changed effect key");
        assert_ne!(baseline, changed_effect_key);

        let mut changed_frame = role.clone();
        changed_frame.frame_count = 2;
        let changed_frame_key = authenticated_sprite_cache_key_material(
            &"1".repeat(64),
            &"2".repeat(64),
            &"3".repeat(64),
            &sprite,
            &changed_frame,
        )
        .expect("changed frame key");
        assert_ne!(baseline, changed_frame_key);

        let mut changed_color = role;
        changed_color.authored_color_rgba_f32_bits[0] = 0.5f32.to_bits();
        let changed_color_key = authenticated_sprite_cache_key_material(
            &"1".repeat(64),
            &"2".repeat(64),
            &"3".repeat(64),
            &sprite,
            &changed_color,
        )
        .expect("changed color key");
        assert_ne!(baseline, changed_color_key);

        let changed_index = authenticated_sprite_cache_key_material(
            &"1".repeat(64),
            &"2".repeat(64),
            &"4".repeat(64),
            &sprite,
            &changed_color,
        )
        .expect("changed index key");
        assert_ne!(changed_color_key, changed_index);
    }

    #[test]
    fn refused_authenticated_composition_never_publishes_partial_pixels() {
        let original = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let mut destination = original.clone();
        let result = compose_rgba_transaction(&mut destination, |staged| {
            staged.pixels[..4].copy_from_slice(&[100, 101, 102, 103]);
            Err::<(), _>(AuthenticatedComposeRejection::UnsupportedTextVariant(vec![
                1,
            ]))
        });
        assert_eq!(
            result,
            Err(AuthenticatedComposeRejection::UnsupportedTextVariant(vec![
                1
            ]))
        );
        assert_eq!(destination, original);
    }
    use crate::gui::{parse_measure, MeasurePair};
    use crate::paradox_text::parse_str;

    fn bare_element(name: &str, children: Vec<GuiElement>) -> GuiElement {
        GuiElement {
            element_type: "containerWindowType".to_string(),
            name: name.to_string(),
            position: None,
            size: None,
            orientation: None,
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children,
            raw: Value::Object(vec![scalar_field("name", name)]),
        }
    }

    fn element_role(
        element: &GuiElement,
        child_path: &[u32],
    ) -> crate::authored_role_index::AuthoredElementRole {
        let font =
            element
                .font
                .as_ref()
                .map(|font_id| crate::authored_role_index::AuthoredFontRole {
                    font_id: font_id.clone(),
                    selected_language: "l_english".into(),
                    selected_variant_index: 0,
                    variants: Vec::new(),
                    base_color: None,
                    base_border_color: None,
                    font_local_textcolors: None,
                    global_textcolor_tables: Vec::new(),
                    // Lower-case because that is the only spelling
                    // `normalize_game_relative_path` ever stores in this field.
                    text_effect_content: cache_content("gfx/fx/text.shader", '9'),
                });
        crate::authored_role_index::AuthoredElementRole {
            element_id: format!("test/{child_path:?}"),
            element_type: element.element_type.clone(),
            authored_name: element.name.clone(),
            child_path: child_path.to_vec(),
            authored_one_based_frame: 1,
            sprite_refs: element.sprite_refs.clone(),
            sprites: Vec::new(),
            direct_texture_refs: Vec::new(),
            direct_textures: Vec::new(),
            font_ref: element.font.clone(),
            font,
            localisation_or_literal_ref: element.text.clone(),
            localisation_content: None,
        }
    }

    #[test]
    fn authenticated_element_admits_only_the_indexed_authored_frame() {
        let mut element = bare_element("button", Vec::new());
        element.raw = Value::Object(vec![
            scalar_field("name", "button"),
            scalar_field("frame", "1"),
        ]);
        let role = element_role(&element, &[7]);
        assert_eq!(
            validate_authenticated_render_fields(&element, &role, &[7]),
            Ok(())
        );
        for raw_frame in ["0", "2"] {
            let mut changed = element.clone();
            changed.raw = Value::Object(vec![
                scalar_field("name", "button"),
                scalar_field("frame", raw_frame),
            ]);
            assert_eq!(
                validate_authenticated_render_fields(&changed, &role, &[7]),
                Err(
                    AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                        child_path: vec![7],
                        field: "frame".into(),
                    }
                )
            );
        }
    }

    #[test]
    fn authenticated_element_refuses_unproved_layout_and_variable_semantics() {
        for field in ["show_position", "hide_position", "margin"] {
            let mut element = bare_element("carrier", Vec::new());
            element.raw = Value::Object(vec![
                scalar_field("name", "carrier"),
                scalar_field(field, "1"),
            ]);
            assert_eq!(
                validate_authenticated_render_fields(&element, &element_role(&element, &[8]), &[8],),
                Err(
                    AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                        child_path: vec![8],
                        field: field.into(),
                    }
                )
            );
        }

        for field in [
            "minimumsize",
            "maximumsize",
            "padding",
            "spacing",
            "maxWidth",
            "horizontal",
            "vertical",
        ] {
            let mut element = bare_element("carrier", Vec::new());
            element.raw = Value::Object(vec![
                scalar_field("name", "carrier"),
                scalar_field(field, "1"),
            ]);
            assert_eq!(
                validate_authenticated_render_fields(&element, &element_role(&element, &[8]), &[8],),
                Err(AuthenticatedComposeRejection::UnsupportedLayoutVariant(
                    vec![8]
                ))
            );
        }

        let mut variable = bare_element("carrier", Vec::new());
        variable.raw = Value::Object(vec![
            scalar_field("name", "carrier"),
            scalar_field("scale", "@scale"),
        ]);
        assert_eq!(
            validate_authenticated_render_fields(&variable, &element_role(&variable, &[9]), &[9],),
            Err(AuthenticatedComposeRejection::UnsupportedVariableReference(
                vec![9]
            ))
        );

        let mut unknown = bare_element("carrier", Vec::new());
        unknown.raw = Value::Object(vec![
            scalar_field("name", "carrier"),
            scalar_field("mod_layout_policy", "left"),
        ]);
        assert_eq!(
            validate_authenticated_render_fields(&unknown, &element_role(&unknown, &[11]), &[11],),
            Err(
                AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                    child_path: vec![11],
                    field: "mod_layout_policy".into(),
                }
            )
        );
    }

    #[test]
    fn authenticated_element_admits_proven_structural_and_sprite_families_only() {
        for element_type in [
            "iconType",
            "listboxType",
            "smoothlistboxType",
            "gridboxType",
            "overlappingElementsBoxType",
            "scrollbarType",
            "extendedScrollbarType",
        ] {
            let mut element = bare_element("unsupported", Vec::new());
            element.element_type = element_type.into();
            assert_eq!(
                validate_authenticated_render_fields(
                    &element,
                    &element_role(&element, &[10]),
                    &[10],
                ),
                Ok(())
            );
        }
        let mut unknown = bare_element("unsupported", Vec::new());
        unknown.element_type = "modCustomType".into();
        assert_eq!(
            validate_authenticated_render_fields(&unknown, &element_role(&unknown, &[10]), &[10],),
            Err(AuthenticatedComposeRejection::UnsupportedElementFamily(
                vec![10]
            ))
        );
    }

    #[test]
    fn authenticated_text_and_list_geometry_use_only_authored_fields() {
        let mut text = bare_element("caption", Vec::new());
        text.element_type = "instantTextBoxType".into();
        text.font = Some("hoi_16mbs".into());
        text.text = Some("CAPTION_KEY".into());
        text.format = Some("center".into());
        text.raw = Value::Object(vec![
            scalar_field("name", "caption"),
            scalar_field("font", "hoi_16mbs"),
            scalar_field("text", "CAPTION_KEY"),
            scalar_field("format", "center"),
            Entry {
                key: Some("borderSize".into()),
                value: Value::Object(vec![scalar_field("x", "2"), scalar_field("y", "3")]),
                line: 1,
                column: 1,
            },
        ]);
        let text_role = element_role(&text, &[14]);
        assert_eq!(
            validate_authenticated_render_fields(&text, &text_role, &[14]),
            Ok(())
        );
        let border = authenticated_text_border(&text, &[14]).unwrap();
        assert_eq!(border, EngineTextBorder::Authored { x: 2, y: 3 });
        assert_eq!(
            authenticated_text_line_alignment(100, 20, border, TextAlign::Center),
            Ok(38)
        );

        let mut list = bare_element("rows", Vec::new());
        list.element_type = "listboxType".into();
        list.raw = Value::Object(vec![
            scalar_field("name", "rows"),
            scalar_field("spacing", "4"),
            scalar_field("layout", "vertical"),
            scalar_field("scroll_wheel_factor", "40"),
            scalar_field("smooth_scrolling", "yes"),
        ]);
        assert_eq!(
            validate_authenticated_render_fields(&list, &element_role(&list, &[15]), &[15],),
            Ok(())
        );

        for (field, value) in [
            ("scroll_wheel_factor", "0"),
            ("scroll_wheel_factor", "forty"),
            ("smooth_scrolling", "maybe"),
        ] {
            let mut malformed = bare_element("rows", Vec::new());
            malformed.element_type = "listboxType".into();
            malformed.raw = Value::Object(vec![
                scalar_field("name", "rows"),
                scalar_field(field, value),
            ]);
            assert_eq!(
                validate_authenticated_render_fields(
                    &malformed,
                    &element_role(&malformed, &[16]),
                    &[16],
                ),
                Err(
                    AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                        child_path: vec![16],
                        field: field.into(),
                    }
                )
            );
        }
    }

    #[test]
    fn authenticated_element_refuses_malformed_names_and_nested_geometry() {
        for names in [
            Vec::new(),
            vec![
                scalar_field("name", "carrier"),
                scalar_field("name", "other"),
            ],
            vec![Entry {
                key: Some("name".into()),
                value: Value::Object(Vec::new()),
                line: 1,
                column: 1,
            }],
        ] {
            let mut element = bare_element("carrier", Vec::new());
            element.raw = Value::Object(names);
            assert_eq!(
                validate_authenticated_render_fields(
                    &element,
                    &element_role(&element, &[12]),
                    &[12],
                ),
                Err(
                    AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                        child_path: vec![12],
                        field: "name".into(),
                    }
                )
            );
        }

        let mut element = bare_element("carrier", Vec::new());
        element.position = Some(MeasurePair {
            x: parse_measure("1"),
            y: parse_measure("2"),
        });
        element.raw = Value::Object(vec![
            scalar_field("name", "carrier"),
            Entry {
                key: Some("position".into()),
                value: Value::Object(vec![
                    scalar_field("x", "1"),
                    scalar_field("y", "2"),
                    scalar_field("mod_offset", "99"),
                ]),
                line: 1,
                column: 1,
            },
        ]);
        assert_eq!(
            validate_authenticated_render_fields(&element, &element_role(&element, &[13]), &[13],),
            Err(
                AuthenticatedComposeRejection::DuplicateOrMalformedAuthoredField {
                    child_path: vec![13],
                    field: "position".into(),
                }
            )
        );
    }

    #[test]
    fn composed_geometry_retains_authored_child_paths_when_names_repeat() {
        let root = bare_element(
            "root",
            vec![
                bare_element("same", Vec::new()),
                bare_element("same", vec![bare_element("leaf", Vec::new())]),
            ],
        );
        let resolver = AssetResolver::new(Vec::new());
        let registry = SpriteRegistry::new();
        let output =
            Compositor::new(&resolver, &registry).compose(&root, &ComposeOptions::new(64, 64));
        assert_eq!(
            output
                .debug_rects
                .iter()
                .map(|rect| (rect.name.as_str(), rect.child_path.as_slice()))
                .collect::<Vec<_>>(),
            vec![
                ("root", &[][..]),
                ("same", &[0][..]),
                ("same", &[1][..]),
                ("leaf", &[1, 0][..]),
            ]
        );
    }

    #[test]
    fn generic_shipping_text_route_uses_one_text1_plan_for_measure_and_draw() {
        let source = include_str!("compose.rs").replace("\r\n", "\n");
        let start = source.find("\n    fn draw_text(").expect("draw_text route");
        let end = source[start..]
            .find("\n    fn resolve_engine_inline_icons(")
            .map(|offset| start + offset)
            .expect("draw_text route end");
        let route = &source[start..end];
        assert!(route.contains("layout_engine_text_with_icons("));
        assert!(route.contains("draw_engine_layout("));
        assert!(!route.contains("wrap_text_greedy"));
        assert!(!route.contains("font.draw_text("));
        assert!(route.contains("MissingLocalisationProvenance"));
        for source in [
            "RuntimeTextSource::SetterCallerRva",
            "RuntimeTextSource::LocalisationKey",
            "RuntimeTextSource::Literal",
            "RuntimeTextSource::AuthoredToken",
        ] {
            assert!(
                route.contains(source),
                "missing exact source dispatch {source}"
            );
        }
        assert!(route.contains("element.text.as_deref().unwrap_or(\"\")"));
    }

    #[test]
    fn inline_sprite_definition_must_be_unique_case_insensitively() {
        fn definition(name: &str, source_file: &str) -> SpriteDefinition {
            SpriteDefinition {
                kind: "spriteType".to_string(),
                name: name.to_string(),
                texture_files: Vec::new(),
                no_of_frames: 1,
                effect_file: None,
                size: None,
                border_size: None,
                source_file: source_file.to_string(),
                raw: Value::Object(Vec::new()),
            }
        }

        let mut registry = SpriteRegistry::new();
        registry.insert(definition("GFX_test_inline", "interface/a.gfx"));
        assert_eq!(
            unique_inline_sprite_definition(&registry, "gfx_TEST_inline")
                .expect("one definition")
                .source_file,
            "interface/a.gfx"
        );

        registry.insert(definition("gfx_test_INLINE", "interface/b.gfx"));
        let error = unique_inline_sprite_definition(&registry, "GFX_test_inline")
            .expect_err("case-variant duplicate must be refused");
        assert!(error.message.contains("ambiguous"));
        assert!(error.message.contains("interface/a.gfx"));
        assert!(error.message.contains("interface/b.gfx"));
    }

    #[test]
    fn engine_text_border_parser_is_exact_and_preserves_negative_values() {
        let valid = parse_str("borderSize = { x = 7 y = -3 }");
        assert_eq!(
            parse_engine_text_border(valid.root.first_value_ci("borderSize").unwrap()),
            Some((7, -3))
        );

        let missing = parse_str("borderSize = { x = 7 }");
        assert_eq!(
            parse_engine_text_border(missing.root.first_value_ci("borderSize").unwrap()),
            None
        );

        let fractional = parse_str("borderSize = { x = 7.5 y = 3 }");
        assert_eq!(
            parse_engine_text_border(fractional.root.first_value_ci("borderSize").unwrap()),
            None
        );
    }

    #[test]
    fn date_template_records_unique_month_source_and_refuses_ambiguity() {
        let date = GuiElement {
            element_type: "instantTextBoxType".to_string(),
            name: "DateText".to_string(),
            position: None,
            size: None,
            orientation: None,
            sprite_refs: Vec::new(),
            font: None,
            text: Some("authored sample must not render".to_string()),
            format: None,
            children: Vec::new(),
            raw: Value::Object(Vec::new()),
        };
        let resolver = AssetResolver::new(Vec::new());
        let registry = SpriteRegistry::new();
        let source = "localisation/english/core_l_english.yml".to_string();

        let unique = Localisation {
            entries: BTreeMap::from([("September".to_string(), "September".to_string())]),
            source_by_key: BTreeMap::from([("September".to_string(), source.clone())]),
            provenance_complete: true,
            ..Localisation::default()
        };
        let mut options = ComposeOptions::new(10, 10).with_localisation(&unique);
        options.text_overrides.insert(
            "DateText".to_string(),
            "7:00 29, $September$ 1939".to_string(),
        );
        let output = Compositor::new(&resolver, &registry).compose(&date, &options);
        assert_eq!(
            output.report.localisation_sources_used.get("September"),
            Some(&source)
        );
        assert!(!output
            .report
            .unresolved_text_inputs
            .contains_key("DateText"));

        let ambiguous = Localisation {
            entries: BTreeMap::from([("September".to_string(), "September".to_string())]),
            ambiguous_sources_by_key: BTreeMap::from([(
                "September".to_string(),
                vec![
                    "localisation/english/a_l_english.yml".to_string(),
                    "localisation/english/b_l_english.yml".to_string(),
                ],
            )]),
            provenance_complete: true,
            ..Localisation::default()
        };
        let mut options = ComposeOptions::new(10, 10).with_localisation(&ambiguous);
        options.text_overrides.insert(
            "DateText".to_string(),
            "7:00 29, $September$ 1939".to_string(),
        );
        let output = Compositor::new(&resolver, &registry).compose(&date, &options);
        assert!(!output
            .report
            .localisation_sources_used
            .contains_key("September"));
        assert!(output.report.unresolved_text_inputs["DateText"]
            .iter()
            .any(|issue| {
                issue.kind == crate::text_law::TextIssueKind::AmbiguousLocalisationProvenance
            }));
    }

    #[test]
    fn percent_measure_uses_parent_dimension() {
        assert_eq!(measure_to_pixels(&parse_measure("50%%"), 200), 100);
    }

    #[test]
    fn percent_measure_rounds_half_away_from_zero() {
        assert_eq!(measure_to_pixels(&parse_measure("50%%"), 1), 1);
        assert_eq!(measure_to_pixels(&parse_measure("-50%%"), 1), -1);
    }

    #[test]
    fn upper_right_anchor_offsets_from_parent_right() {
        let element = GuiElement {
            element_type: "iconType".to_string(),
            name: "x".to_string(),
            position: Some(MeasurePair {
                x: parse_measure("-35"),
                y: parse_measure("7"),
            }),
            size: None,
            orientation: Some("UPPER_RIGHT".to_string()),
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: Vec::new(),
            raw: Value::Object(Vec::new()),
        };
        let rect = resolve_element_rect(
            &element,
            Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 108,
            },
            (30, 30),
            &ComposeOptions::new(1920, 108),
        );
        assert_eq!(rect.x, 1885);
        assert_eq!(rect.y, 7);
    }

    #[test]
    fn right_anchor_offsets_scale_with_viewport_widths() {
        let element = GuiElement {
            element_type: "buttonType".to_string(),
            name: "menu_button".to_string(),
            position: Some(MeasurePair {
                x: parse_measure("-35"),
                y: parse_measure("7"),
            }),
            size: None,
            orientation: Some("UPPER_RIGHT".to_string()),
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: Vec::new(),
            raw: Value::Object(Vec::new()),
        };
        for width in [1920, 2560] {
            let rect = resolve_element_rect(
                &element,
                Rect {
                    x: 0,
                    y: 0,
                    w: width,
                    h: 1080,
                },
                (30, 30),
                &ComposeOptions::new(width as usize, 1080),
            );
            assert_eq!(rect.x, width - 35);
            assert_eq!(rect.y, 7);
        }
    }

    #[test]
    fn orientation_parse_matches_engine_table() {
        assert_eq!(
            Orientation::parse(Some("UPPER_LEFT")),
            Orientation::UpperLeft
        );
        assert_eq!(
            Orientation::parse(Some("upper_right")),
            Orientation::UpperRight
        );
        assert_eq!(
            Orientation::parse(Some("LOWER_LEFT")),
            Orientation::LowerLeft
        );
        assert_eq!(
            Orientation::parse(Some("lower_RIGHT")),
            Orientation::LowerRight
        );
        assert_eq!(Orientation::parse(Some("CENTER")), Orientation::Center);
        assert_eq!(Orientation::parse(Some("center_up")), Orientation::CenterUp);
        assert_eq!(
            Orientation::parse(Some("center_down")),
            Orientation::CenterDown
        );
        assert_eq!(
            Orientation::parse(Some("center_left")),
            Orientation::CenterLeft
        );
        assert_eq!(
            Orientation::parse(Some("center_right")),
            Orientation::CenterRight
        );
        assert_eq!(Orientation::parse(Some("garbage")), Orientation::UpperLeft);
        assert_eq!(Orientation::parse(None), Orientation::UpperLeft);
    }

    #[test]
    fn anchor_fractions_follow_the_enum() {
        let parent = Rect::new(10, 20, 200, 100);
        assert_eq!(Orientation::UpperLeft.anchor(parent), (0, 0));
        assert_eq!(Orientation::UpperRight.anchor(parent), (200, 0));
        assert_eq!(Orientation::LowerLeft.anchor(parent), (0, 100));
        assert_eq!(Orientation::LowerRight.anchor(parent), (200, 100));
        assert_eq!(Orientation::Center.anchor(parent), (100, 50));
        assert_eq!(Orientation::CenterUp.anchor(parent), (100, 0));
        assert_eq!(Orientation::CenterDown.anchor(parent), (100, 100));
        assert_eq!(Orientation::CenterLeft.anchor(parent), (0, 50));
        assert_eq!(Orientation::CenterRight.anchor(parent), (200, 50));
    }

    #[test]
    fn center_anchor_halves_odd_parent_extents_with_integer_division() {
        let parent = Rect::new(0, 0, 101, 55);
        assert_eq!(Orientation::Center.anchor(parent), (50, 27));
        assert_eq!(Orientation::CenterRight.anchor(parent), (101, 27));
        assert_eq!(Orientation::CenterDown.anchor(parent), (50, 55));
    }

    #[test]
    fn center_up_anchors_x_center_y_top() {
        let element = GuiElement {
            element_type: "instantTextBoxType".to_string(),
            name: "cu".to_string(),
            position: Some(MeasurePair {
                x: parse_measure("0"),
                y: parse_measure("4"),
            }),
            size: None,
            orientation: Some("CENTER_UP".to_string()),
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: Vec::new(),
            raw: Value::Object(Vec::new()),
        };
        let rect = resolve_element_rect(
            &element,
            Rect::new(0, 0, 400, 100),
            (30, 30),
            &ComposeOptions::new(400, 100),
        );
        assert_eq!(rect.x, 200);
        assert_eq!(rect.y, 4);
    }

    #[test]
    fn negative_size_fills_parent_minus_offset() {
        let element = GuiElement {
            element_type: "containerWindowType".to_string(),
            name: "fill".to_string(),
            position: None,
            size: Some(MeasurePair {
                x: parse_measure("-20"),
                y: parse_measure("-5"),
            }),
            orientation: None,
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: Vec::new(),
            raw: Value::Object(Vec::new()),
        };
        let rect = resolve_element_rect(
            &element,
            Rect::new(0, 0, 550, 1000),
            (0, 0),
            &ComposeOptions::new(550, 1000),
        );
        assert_eq!(rect.w, 530);
        assert_eq!(rect.h, 995);
    }

    #[test]
    fn xui23_animation_sampling_wraps_authored_scrolling_axis() {
        let image = RgbaImage {
            width: 1,
            height: 2,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        assert_eq!(
            sample_linear(&image, 0.5, 0.25, true),
            sample_linear(&image, 0.5, 1.25, true)
        );
        assert_eq!(
            sample_linear(&image, 0.5, -0.75, true),
            sample_linear(&image, 0.5, 0.25, true)
        );
    }

    #[test]
    fn xui23_animation_parameters_fail_closed_when_authored_fields_are_absent() {
        let document = crate::paradox_text::parse_str(
            "animation = { animationtime = 1.0 animationlooping = yes }",
        );
        let animation = document
            .root
            .first_value_ci("animation")
            .expect("animation fixture");
        assert_eq!(
            required_f32_ci(animation, "animationtime", "fixture").expect("authored float"),
            1.0
        );
        assert!(required_xy_ci(animation, "animationtexturescale", "fixture").is_err());
        assert!(required_scalar_ci(animation, "animationblendmode", "fixture").is_err());
        assert!(validate_xui23_scrolling_add_animation(animation, "fixture").is_err());
    }

    #[test]
    fn xui23_animationframes_is_an_exact_one_based_positional_bitmask() {
        for (source, frame, applies) in [
            ("animation = {}", 31, true),
            ("animation = { animationframes = {} }", 31, true),
            ("animation = { animationframes = { 1 2 3 } }", 1, true),
            ("animation = { animationframes = { 1 2 3 } }", 4, false),
            ("animation = { animationframes = { 1 1 } }", 1, true),
            ("animation = { animationframes = { 33 } }", 1, true),
            ("animation = { animationframes = { -31 } }", 1, true),
        ] {
            let document = crate::paradox_text::parse_str(source);
            let animation = document
                .root
                .first_value_ci("animation")
                .expect("animation fixture");
            validate_xui23_animation_shape(animation, "fixture").expect("exact frames shape");
            assert_eq!(
                xui23_animation_applies_to_frame(animation, "fixture", frame)
                    .expect("exact frame mask"),
                applies,
                "unexpected frame selection for {source} at frame {frame}"
            );
        }
    }

    #[test]
    fn xui23_num_animations_2_selects_first_two_applicable_blocks_in_source_order() {
        let document = crate::paradox_text::parse_str(
            r#"animation = { id = first animationframes = { 2 } }
               animation = { id = second animationframes = { 1 } }
               animation = { id = third animationframes = { 1 } }
               animation = { id = fourth animationframes = { 1 } }"#,
        );
        let animations = document.root.values_ci("animation").collect::<Vec<_>>();
        let selected = xui23_first_two_applicable_animations(&animations, "fixture", 1).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|animation| animation.first_scalar_ci("id").unwrap())
                .collect::<Vec<_>>(),
            vec!["second", "third"]
        );
    }

    #[test]
    fn xui23_animation_shape_rejects_duplicate_and_malformed_members() {
        for (source, expected) in [
            (
                "animation = { animationframes = { x = 1 } }",
                "non-positional integer member",
            ),
            ("animation = { animationframes = { one } }", "non-i32 frame"),
            (
                "animation = { animationtime = 1 animationtime = 2 }",
                "repeats authored field animationtime",
            ),
            (
                "animation = { animationtexturescale = { x = 1 x = 2 y = 1 } }",
                "repeats authored field x",
            ),
            (
                "animation = { animationrotationoffset = { x = 0 z = 0 } }",
                "unsupported authored field z",
            ),
        ] {
            let document = crate::paradox_text::parse_str(source);
            let animation = document
                .root
                .first_value_ci("animation")
                .expect("animation fixture");
            let error = validate_xui23_animation_shape(animation, "fixture")
                .expect_err("unsupported animation shape must fail closed");
            assert!(
                error.to_string().contains(expected),
                "unexpected refusal for {source}: {error}"
            );
        }
    }

    #[test]
    fn xui23_compose_options_default_to_no_recorded_animation_or_disable_state() {
        let options = ComposeOptions::new(64, 64);
        assert_eq!(options.animation_elapsed_micros, 0);
        assert!(options.disabled_button_elements.is_empty());
        assert!(options.xui23_buttonstate_elements.is_empty());
        assert!(options.xui23_progress_elements.is_empty());
        assert!(!options.xui23_require_scoped_progress);
    }

    #[test]
    fn xui23_normal_progress_selects_one_texture_and_transposes_vertical_sampling() {
        let front = RgbaImage {
            width: 4,
            height: 2,
            pixels: vec![
                10, 0, 0, 128, 20, 0, 0, 128, 30, 0, 0, 128, 40, 0, 0, 128, 50, 0, 0, 128, 60, 0,
                0, 128, 70, 0, 0, 128, 80, 0, 0, 128,
            ],
        };
        let back = RgbaImage {
            width: 4,
            height: 2,
            pixels: vec![
                0, 110, 0, 128, 0, 120, 0, 128, 0, 130, 0, 128, 0, 140, 0, 128, 0, 150, 0, 128, 0,
                160, 0, 128, 0, 170, 0, 128, 0, 180, 0, 128,
            ],
        };
        let mut destination = RgbaImage {
            width: 2,
            height: 4,
            pixels: [7, 11, 13, 17].repeat(8),
        };
        draw_progress_normal_axis(&mut destination, &front, &back, 0, 0, 2, 4, 0.5, false);
        let expected_sources = [
            [10, 0, 0, 128],
            [50, 0, 0, 128],
            [20, 0, 0, 128],
            [60, 0, 0, 128],
            [0, 130, 0, 128],
            [0, 170, 0, 128],
            [0, 140, 0, 128],
            [0, 180, 0, 128],
        ];
        let expected = expected_sources
            .into_iter()
            .flat_map(|src| alpha_blend([7, 11, 13, 17], src))
            .collect::<Vec<_>>();
        assert_eq!(destination.pixels, expected);
    }

    #[test]
    fn xui23_vertical_progress_minmax_selects_and_transposes_texture_axes() {
        let front = RgbaImage {
            width: 4,
            height: 2,
            pixels: vec![
                10, 0, 0, 255, 20, 0, 0, 255, 30, 0, 0, 255, 40, 0, 0, 255, 50, 0, 0, 255, 60, 0,
                0, 255, 70, 0, 0, 255, 80, 0, 0, 255,
            ],
        };
        let back = RgbaImage {
            width: 4,
            height: 2,
            pixels: vec![
                110, 0, 0, 255, 120, 0, 0, 255, 130, 0, 0, 255, 140, 0, 0, 255, 150, 0, 0, 255,
                160, 0, 0, 255, 170, 0, 0, 255, 180, 0, 0, 255,
            ],
        };
        let mut destination = RgbaImage {
            width: 2,
            height: 4,
            pixels: vec![0; 32],
        };
        draw_progress_minmax_axis(&mut destination, &front, &back, 0, 0, 2, 4, 0.75, false);
        assert_eq!(
            destination.pixels,
            vec![
                110, 0, 0, 255, 150, 0, 0, 255, 120, 0, 0, 255, 160, 0, 0, 255, 30, 0, 0, 255, 70,
                0, 0, 255, 140, 0, 0, 255, 180, 0, 0, 255,
            ]
        );
    }

    #[test]
    fn xui23_buttonstate_path_is_scoped_and_shades_before_animation() {
        let source = include_str!("compose.rs").replace("\r\n", "\n");
        let start = source.find("    fn draw_plain_sprite(").unwrap();
        let end = source[start..]
            .find("    fn draw_progress_bar(")
            .map(|offset| start + offset)
            .unwrap();
        let path = &source[start..end];
        assert!(path.contains(
            "let xui23_authorized = options.xui23_buttonstate_elements.contains(&element.name);"
        ));
        assert!(path.contains("let frame = if xui23_authorized"));
        assert!(path.contains("self.shade_button_sprite("));
        assert!(!path.contains("&& (sprite.raw.first_value_ci(\"animation\")"));
        assert!(path.contains("xui23_first_two_applicable_animations("));
        assert!(path.contains("for animation in applicable"));
        assert!(path.contains("image_pixel_f32(base, x, y)"));
        assert!(!path.contains("contains(\"buttonstate\")"));
    }

    #[test]
    fn xui23_disabled_animation_quantizes_only_after_the_float_shader_chain() {
        let base = RgbaF32([50.0 / 255.0, 75.0 / 255.0, 100.0 / 255.0, 1.0]);
        let mask = RgbaF32([1.0; 4]);
        let animation_slot_0 = RgbaF32([0.0, 0.1, 0.0, 1.0 / 255.0]);
        let animation_slot_1 = RgbaF32([0.1, 0.0, 0.0, 1.0 / 255.0]);
        let pixel = shade_xui23_button_animation_pixel(
            base,
            true,
            [(mask, animation_slot_0), (mask, animation_slot_1)],
        );
        let mut output = RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![0; 4],
        };
        set_image_pixel_f32(&mut output, 0, 0, pixel);
        assert_eq!(output.pixels, vec![72, 72, 71, 255]);
    }
}

fn shade_xui23_button_animation_pixel(
    base: RgbaF32,
    disabled: bool,
    animation_samples: impl IntoIterator<Item = (RgbaF32, RgbaF32)>,
) -> RgbaF32 {
    let mut pixel = if disabled {
        shade_buttonstate(
            base,
            RgbaF32([1.0; 4]),
            ButtonVisualState::Disable,
            0.0,
            0.0,
        )
    } else {
        base
    };
    for (mask, animated) in animation_samples {
        let source = RgbaF32(std::array::from_fn(|channel| {
            animated.0[channel] * mask.0[channel]
        }));
        pixel = blend_sprite_animation(pixel, source, SpriteAnimationBlendMode::Add);
    }
    pixel
}
