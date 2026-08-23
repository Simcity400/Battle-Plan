//!    all-country product carriers do not acquire that dependency. A type that
//! content; recorder panel ownership, recorder cursor state and legacy

use crate::assets::AssetResolver;
use crate::binding::{
    apply_clock, clock_binding_from_frame, topbar_binding_from_frame,
    topbar_binding_from_observer_frame,
};
use crate::combat_detail_canvas::{
    active_texture_paths, admit_xui20_canvas_route, Xui20CanvasRoute, Xui20ViewerState,
};
use crate::compose::{resolve_element_rect, ComposeOptions, Compositor, Rect, UiAssetCache};
use crate::dds::RgbaImage;
use crate::decision_chrome::{
    category_icon_sprite, decision_icon_sprite, load_decision_authored_scene,
    parse_decision_layout, parse_decisions, DecisionSet, DECISION_GRID, DECISION_VIEW_GUI,
    ROOT_WINDOW,
};
use crate::decision_replay::DecisionStaticCatalog;
use crate::gfx::SpriteRegistry;
use crate::gui::GuiElement;
use crate::hud::{
    compose_prepared_tooltip_layer, prepare_composed_tooltip,
    ComposedTooltipPlacementInput, HudBinding, MinimapBinding, PreparedComposedTooltip,
    TooltipBinding,
};
use crate::loc::Localisation;
use crate::research_canvas::{admit_technology_canvas_render_route, TechnologyCanvasRenderRoute};
use crate::tooltip_compositor::{
    load_authored_tooltip_clamp_offsets, load_authored_tooltip_timing,
    resolve_authenticated_replay_tooltip_route, AdmittedTooltipSource,
    AuthenticatedReplayTooltipContext, AuthenticatedReplayTooltipRoute, AuthoredTooltipAnchor,
    HoverCandidate, PlacementEvidence, TooltipAnchor, TooltipFamilyFacts, TooltipLifecycle,
    TooltipOwner,
};
use crate::trade_state::{admit_trade_screen, AdmittedTradeScreen};
use crate::variant_admission::{
    DisplayMode, DisplayVariantKey, ExactKeyVariantContract, InputVariant, PixelExtent,
};
use crate::widget_layout::gridbox_layout;
use crate::{GameUiError, Result};
use replay_core::{
    Leaf, ValidatedContentAdmission, WorldFrame,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

pub const PRODUCTION_UI_WIDTH: usize = 1920;
pub const PRODUCTION_UI_HEIGHT: usize = 1080;
pub const PRODUCTION_UI_MIN_GUI: PixelExtent = PixelExtent {
    width: 1280,
    height: 720,
};
pub const PRODUCTION_UI_GUI_SCALE_BITS: u32 = 0x3f80_0000;
pub const PRODUCTION_UI_LANGUAGE: &str = "l_english";

fn require_xui20_active_texture_content(
    content_admission: &ValidatedContentAdmission,
    route: &Xui20CanvasRoute,
) -> std::result::Result<(), ProductionUiRejection> {
    let texture_paths = active_texture_paths(route);
    require_xui20_texture_paths(content_admission, &texture_paths)
}

fn require_xui20_texture_paths(
    content_admission: &ValidatedContentAdmission,
    texture_paths: &BTreeSet<String>,
) -> std::result::Result<(), ProductionUiRejection> {
    for texture_path in texture_paths {
        content_admission
            .require_used_content(texture_path)
            .map_err(ProductionUiRejection::Asset)?;
    }
    Ok(())
}

pub fn production_ui_variant_key() -> DisplayVariantKey {
    DisplayVariantKey {
        physical_viewport: PixelExtent {
            width: PRODUCTION_UI_WIDTH as u32,
            height: PRODUCTION_UI_HEIGHT as u32,
        },
        min_gui: PRODUCTION_UI_MIN_GUI,
        gui_scale_f32_bits: PRODUCTION_UI_GUI_SCALE_BITS,
        display_mode: DisplayMode::BorderlessFullscreen,
        localisation_language: PRODUCTION_UI_LANGUAGE.to_string(),
        input_variant: InputVariant::MouseKeyboard,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductHudSurface {
    pub consumer_token: &'static str,
    pub activation_kind: replay_core::HudSurfaceActivationKind,
    pub authored_root: &'static str,
    pub authored_gui: &'static str,
}

const PRODUCT_HUD_SURFACES: &[ProductHudSurface] = &[
    ProductHudSurface {
        consumer_token: "topbar",
        activation_kind: replay_core::HudSurfaceActivationKind::Topbar,
        authored_root: "topbar",
        authored_gui: crate::topbar::TOPBAR_GUI,
    },
    ProductHudSurface {
        consumer_token: "minimap",
        activation_kind: replay_core::HudSurfaceActivationKind::Minimap,
        authored_root: crate::hud::MINIMAP_WINDOW_NAME,
        authored_gui: crate::hud::MINIMAP_GUI,
    },
    ProductHudSurface {
        consumer_token: "mapmodes",
        activation_kind: replay_core::HudSurfaceActivationKind::Mapmodes,
        authored_root: crate::hud::MAPMODES_WINDOW_NAME,
        authored_gui: crate::hud::MAPMODES_GUI,
    },
    ProductHudSurface {
        consumer_token: "theatre_selector",
        activation_kind: replay_core::HudSurfaceActivationKind::TheatreSelector,
        authored_root: crate::hud::OUTLINER_WINDOW_NAME,
        authored_gui: crate::hud::OUTLINER_GUI,
    },
];

pub fn observer_authored_hud_sources() -> Vec<crate::hud::HudSourceWindow> {
    PRODUCT_HUD_SURFACES
        .iter()
        .map(|surface| crate::hud::HudSourceWindow {
            panel: surface.consumer_token,
            source_file: surface.authored_gui.to_string(),
            window_name: surface.authored_root.to_string(),
        })
        .collect()
}

pub fn hud_activation_kind_token(kind: replay_core::HudSurfaceActivationKind) -> &'static str {
    use replay_core::HudSurfaceActivationKind as Kind;
    match kind {
        Kind::Topbar => "topbar",
        Kind::Minimap => "minimap",
        Kind::Mapmodes => "mapmodes",
        Kind::TheatreSelector => "theatre_selector",
        Kind::DesignerCountryEquipment => "designer/country_equipment",
        Kind::DesignerTank => "designer/tank",
        Kind::DesignerPlane => "designer/plane",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedHudSurfaceActivation {
    routed_surfaces: Vec<String>,
    unrouted_active_surfaces: Vec<String>,
}

impl AdmittedHudSurfaceActivation {
    pub fn routed_surfaces(&self) -> &[String] {
        &self.routed_surfaces
    }

    pub fn unrouted_active_surfaces(&self) -> &[String] {
        &self.unrouted_active_surfaces
    }
}

pub fn admit_authenticated_hud_surface_activation(
    view: &replay_core::HudSurfaceActivationView,
    expected_hour: i64,
) -> std::result::Result<Option<AdmittedHudSurfaceActivation>, ProductionUiRejection> {
    match view.status {
        replay_core::ViewStatus::Present => {}
        replay_core::ViewStatus::DomainAbsent => return Ok(None),
        status => {
            return Err(ProductionUiRejection::IncompleteComposition(format!(
                "HUD surface activation carrier is {status:?}: the recording carries d79 and it \
                 was rejected, which is not the same as a recording that predates the domain"
            )))
        }
    }
    let activation = view.activation.as_ref().ok_or_else(|| {
        ProductionUiRejection::IncompleteComposition(
            "Present HUD surface activation carrier omitted its activation".into(),
        )
    })?;
    if activation.schema_version != HUD_SURFACE_ACTIVATION_SCHEMA_VERSION
        || activation.authority != HUD_SURFACE_ACTIVATION_AUTHORITY
    {
        return Err(ProductionUiRejection::IncompleteComposition(format!(
            "HUD surface activation carrier claims schema {} authority {:?}, not schema {} \
             authority {:?}",
            activation.schema_version,
            activation.authority,
            HUD_SURFACE_ACTIVATION_SCHEMA_VERSION,
            HUD_SURFACE_ACTIVATION_AUTHORITY
        )));
    }
    if activation.status_flags != 0 {
        return Err(ProductionUiRejection::IncompleteComposition(format!(
            "HUD surface activation frame is badged with status_flags {:#x}; the producer's own \
             promote_activation_frame rejects such a frame as not promotion-eligible",
            activation.status_flags
        )));
    }
    if u64::try_from(expected_hour).ok() != Some(activation.frame_hour) {
        return Err(ProductionUiRejection::IncompleteComposition(format!(
            "HUD surface activation frame hour {} is not the composed hour {expected_hour}",
            activation.frame_hour
        )));
    }
    let mut seen_kinds = BTreeSet::new();
    let mut seen_roots = BTreeSet::new();
    for row in &activation.active_surfaces {
        if !seen_kinds.insert(row.surface) {
            return Err(ProductionUiRejection::IncompleteComposition(format!(
                "HUD surface activation lists surface {} twice",
                hud_activation_kind_token(row.surface)
            )));
        }
        if !seen_roots.insert(row.runtime_root_name.to_ascii_lowercase()) {
            return Err(ProductionUiRejection::IncompleteComposition(format!(
                "HUD surface activation gives runtime root {:?} to more than one surface, under \
                 the ASCII-folded root identity the authored-root resolver uses",
                row.runtime_root_name
            )));
        }
    }
    let mut routed_surfaces = Vec::with_capacity(PRODUCT_HUD_SURFACES.len());
    for surface in PRODUCT_HUD_SURFACES {
        let active = activation
            .active_surfaces
            .iter()
            .find(|row| row.surface == surface.activation_kind)
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "HUD surface activation does not list drawn surface {}",
                    surface.consumer_token
                ))
            })?;
        if !active
            .runtime_root_name
            .eq_ignore_ascii_case(surface.authored_root)
        {
            return Err(ProductionUiRejection::IncompleteComposition(format!(
                "HUD surface {} was active on runtime root {:?}, but this compositor draws the \
                 authored root {:?}",
                surface.consumer_token, active.runtime_root_name, surface.authored_root
            )));
        }
        routed_surfaces.push(surface.consumer_token.to_string());
    }
    let unrouted_active_surfaces = activation
        .active_surfaces
        .iter()
        .filter(|row| {
            !PRODUCT_HUD_SURFACES
                .iter()
                .any(|surface| surface.activation_kind == row.surface)
        })
        .map(|row| hud_activation_kind_token(row.surface).to_string())
        .collect();
    Ok(Some(AdmittedHudSurfaceActivation {
        routed_surfaces,
        unrouted_active_surfaces,
    }))
}

pub const HUD_SURFACE_ACTIVATION_SCHEMA_VERSION: u32 = 1;
pub const HUD_SURFACE_ACTIVATION_AUTHORITY: &str = "exact_a729_hud_controller_roots";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedProductHudRoute {
    consumer_prefixes: Vec<String>,
}

impl AuthenticatedProductHudRoute {
    pub fn consumer_prefixes(&self) -> impl Iterator<Item = &str> {
        self.consumer_prefixes.iter().map(String::as_str)
    }
}

pub fn admit_authenticated_product_hud_route(
    admission: &replay_core::ValidatedContentAdmission,
) -> Result<Option<AuthenticatedProductHudRoute>> {
    let mut prefixes = Vec::new();
    let mut selected_paths = BTreeSet::new();
    let mut missing_surface = false;
    for ProductHudSurface {
        consumer_token: surface,
        authored_root: root,
        authored_gui: expected_gui_path,
        ..
    } in PRODUCT_HUD_SURFACES
    {
        let prefix = format!("hud_role/v1/{surface}/");
        let consumers = admission
            .required_consumers()
            .filter(|consumer| consumer.starts_with(&prefix))
            .collect::<Vec<_>>();
        if consumers.is_empty() {
            missing_surface = true;
            continue;
        }
        let any_gui_prefix = format!("{prefix}gui/");
        let gui_consumers = consumers
            .iter()
            .copied()
            .filter(|consumer| consumer.starts_with(&any_gui_prefix))
            .collect::<Vec<_>>();
        if gui_consumers.len() != 1 {
            return Err(GameUiError::parse(format!(
                "authenticated HUD surface {surface} has {} GUI-root consumers",
                gui_consumers.len()
            )));
        }
        let gui_prefix = format!("{prefix}gui/{root}/");
        if !gui_consumers[0].starts_with(&gui_prefix) {
            missing_surface = true;
            continue;
        }
        let gui_rows = admission
            .require_used_content_for_consumer(gui_consumers[0])
            .map_err(GameUiError::missing)?;
        if gui_rows.len() != 1
            || gui_consumers[0].strip_prefix(&gui_prefix)
                != Some(gui_rows[0].payload_sha256.as_str())
        {
            return Err(GameUiError::parse(format!(
                "authenticated HUD surface {surface} has a malformed GUI-root binding"
            )));
        }
        if gui_rows[0].normalized_game_relative_path != *expected_gui_path {
            missing_surface = true;
            continue;
        }
        for consumer in consumers {
            for content in admission
                .require_used_content_for_consumer(consumer)
                .map_err(GameUiError::missing)?
            {
                selected_paths.insert(content.normalized_game_relative_path.as_str());
            }
        }
        prefixes.push(prefix);
    }
    if missing_surface {
        return Ok(None);
    }
    for required in REQUIRED_AUTHORED_HUD_SOURCES {
        if !selected_paths.contains(required) {
            return Ok(None);
        }
    }
    Ok(Some(AuthenticatedProductHudRoute {
        consumer_prefixes: prefixes,
    }))
}

pub const REQUIRED_AUTHORED_HUD_SOURCES: &[&str] = &[
    crate::topbar::TOPBAR_GUI,
    crate::hud::MINIMAP_GUI,
    crate::hud::MAPMODES_GUI,
    crate::hud::OUTLINER_GUI,
];

pub fn admit_composed_xui05_tooltip_route(
    content_admission: &ValidatedContentAdmission,
    prepared: Option<PreparedComposedTooltip>,
) -> std::result::Result<Option<PreparedComposedTooltip>, ProductionUiRejection> {
    let Some(prepared) = prepared else {
        return Ok(None);
    };
    if !matches!(
        prepared.placement_evidence(),
        PlacementEvidence::ViewerCursor { .. }
            | PlacementEvidence::AuthoredAbsoluteAnchor { .. }
            | PlacementEvidence::AuthoredOwnerRelativeAnchor { .. }
            | PlacementEvidence::ViewerControllerRetained { .. }
    ) {
        return Err(ProductionUiRejection::IncompleteComposition(
            "XUI05 composed route lacks authenticated viewer/authored placement provenance".into(),
        ));
    }
    prepared
        .payload()
        .validate()
        .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
    if prepared.binding().owner_root_name != prepared.payload().tooltip_root_name {
        return Err(ProductionUiRejection::IncompleteComposition(
            "XUI05 composed binding root differs from its validated payload root".into(),
        ));
    }
    content_admission
        .require_used_content(crate::hud::CORE_GUI)
        .map_err(ProductionUiRejection::Asset)?;
    for row in &prepared.payload().rows {
        for source in &row.admitted_sources {
            match source {
                AdmittedTooltipSource::EffectiveGui {
                    game_relative_path, ..
                } => {
                    content_admission
                        .require_used_content(game_relative_path)
                        .map_err(ProductionUiRejection::Asset)?;
                }
                AdmittedTooltipSource::EffectiveLocalisation { source_files, .. } => {
                    if source_files.is_empty() {
                        return Err(ProductionUiRejection::IncompleteComposition(format!(
                            "XUI05 row {:?} names no admitted localisation source",
                            row.source_identity
                        )));
                    }
                    for source_file in source_files {
                        content_admission
                            .require_used_content(source_file)
                            .map_err(ProductionUiRejection::Asset)?;
                    }
                }
                AdmittedTooltipSource::EffectiveSprite {
                    definition_file,
                    texture_file,
                    ..
                } => {
                    content_admission
                        .require_used_content(definition_file)
                        .map_err(ProductionUiRejection::Asset)?;
                    content_admission
                        .require_used_content(texture_file)
                        .map_err(ProductionUiRejection::Asset)?;
                }
                AdmittedTooltipSource::EffectivePlacement {
                    graphics_defines_file,
                } => {
                    content_admission
                        .require_used_content(graphics_defines_file)
                        .map_err(ProductionUiRejection::Asset)?;
                }
                AdmittedTooltipSource::EffectiveAsset {
                    game_relative_path, ..
                } => {
                    content_admission
                        .require_used_content(game_relative_path)
                        .map_err(ProductionUiRejection::Asset)?;
                }
                AdmittedTooltipSource::RecordedState {
                    domain,
                    field,
                    identity,
                } => {
                    if domain.trim().is_empty()
                        || field.trim().is_empty()
                        || identity.trim().is_empty()
                    {
                        return Err(ProductionUiRejection::IncompleteComposition(format!(
                            "XUI05 row {:?} has an incomplete recorded-state trace",
                            row.source_identity
                        )));
                    }
                }
                AdmittedTooltipSource::PacketSemantic {
                    packet_id,
                    producer_identity,
                    field,
                    manifest_sha256,
                } => {
                    if packet_id.trim().is_empty()
                        || producer_identity.trim().is_empty()
                        || field.trim().is_empty()
                        || manifest_sha256.len() != 64
                        || !manifest_sha256
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    {
                        return Err(ProductionUiRejection::IncompleteComposition(format!(
                            "XUI05 row {:?} has an invalid packet-semantic trace",
                            row.source_identity
                        )));
                    }
                }
            }
        }
    }
    Ok(Some(prepared))
}

pub fn admit_xui05_tooltip_route(
    content_admission: &ValidatedContentAdmission,
    frame: &WorldFrame,
    pov_tag: Option<&str>,
    viewport_width: usize,
    viewport_height: usize,
) -> std::result::Result<Option<TooltipBinding>, ProductionUiRejection> {
    let Some(binding) = tooltip_binding_from_frame(frame, pov_tag, viewport_width, viewport_height)
    else {
        return Ok(None);
    };
    if binding.owner_root_name != crate::hud::TOOLTIP_WINDOW_NAME {
        return Err(ProductionUiRejection::IncompleteComposition(format!(
            "XUI05 carrier names tooltip owner root {:?}, which is not the authored root \
             {:?} this route resolves chrome from",
            binding.owner_root_name,
            crate::hud::TOOLTIP_WINDOW_NAME
        )));
    }
    content_admission
        .require_used_content(crate::hud::CORE_GUI)
        .map_err(ProductionUiRejection::Asset)?;
    Ok(Some(binding))
}

/// clipping, XUI05 tooltip ownership, and authenticated close/scroll/expansion
pub const PROMOTED_XUI_PACKETS: &[&str] = &[
    "XUI01", "XUI02", "XUI03", "XUI04", "XUI05", "XUI06", "XUI07", "XUI08", "XUI09", "XUI10",
    "XUI11", "XUI12", "XUI13", "XUI14", "XUI15", "XUI16", "XUI17", "XUI18", "XUI19", "XUI20",
    "XUI21", "XUI22", "XUI23", "XUI24", "XUI25", "XUI26",
];

pub const PROMOTED_EXTERNAL_DEPENDENCY_PACKETS: &[(&str, &str)] = &[
    (
        "XUI07",
        "EIR-NE-012 canonical exact-a729 THS2 v6 passive UI-open-independent recorder-POV-independent complete-D18/D98 all-country theatre controller hierarchy, priority, state, region, alert-frame, and active-visual-frame carrier for XUI07/XUI27",
    ),
    (
        "XUI08",
        "EIR-NE-001 canonical PCS3 exact-a729 passive UI-open-independent recorder-POV-independent global peace-conference participants, allocations, results, and action-identity carrier",
    ),
    (
        "XUI09",
        "canonical exact-a729 passive UI-open-independent recorder-POV-independent complete-D18 G4 completed-membership, X9S1 technology-state, and effective authored technology topology/catalog carrier",
    ),
    (
        "XUI10",
        "EIR-NE-002 canonical exact-a729 passive UI-open-independent recorder-POV-independent complete-D18 all-country evaluated national-focus scene, progress, layout-operation, and joint-viewing carrier",
    ),
    (
        "XUI11",
        "EIR-NE-003 canonical schema-7 exact-a729 passive UI-open-independent recorder-POV-independent complete-D18 all-country d14/d15 politics state and final-text carrier",
    ),
    (
        "XUI12",
        "EIR-NE-014 canonical exact-a729 passive UI-open-independent recorder-POV-independent complete-D18 all-country production-line row order, optional-row identity, controller semantics, and detached completeness carrier",
    ),
    (
        "XUI14",
        "EIR-NE-004 canonical schema-8 exact-a729 passive UI-open-independent recorder-POV-independent complete-D18 all-country intelligence state, presentation, reciprocal-join, and final-text carrier",
    ),
    (
        "XUI15",
        "EIR-NE-013 canonical exact-a729 passive UI-open-independent recorder-POV-independent complete-D18 all-country trade catalogue payload and detached authority carrier for the shipping XUI15 observer panel",
    ),
    (
        "XUI16",
        "EIR-NE-010 canonical exact-a729 passive UI-open-independent recorder-POV-independent complete D52/D66 occupation/garrison supplement, equipment identity/type/amount, and garrison-alert carrier",
    ),
    (
        "XUI17",
        "EIR-NE-005 canonical exact-a729 passive UI-open-independent recorder-POV-independent complete-D18 all-country D47/D28/D46 rosters and assignments plus G8 trait and G11 ace final-presentation carrier",
    ),
    (
        "XUI18",
        "EIR-NE-006 canonical exact-a729 passive UI-open-independent recorder-POV-independent complete-D18 all-country d74 state and final-presentation carrier",
    ),
    (
        "XUI19",
        "EIR-NE-007 canonical exact-a729 passive UI-open-independent recorder-POV-independent complete-D18 all-country decision evaluation, effective-cost, final-text, and ordered-presentation carrier",
    ),
    (
        "XUI20",
        "EIR-CW-002 canonical exact-a729 passive UI-open-independent recorder-POV-independent global five-panel combat final-presentation carrier",
    ),
    (
        "XUI21",
        "EIR-NE-008 canonical exact-a729 passive UI-open-independent recorder-POV-independent complete all-country extended-d75 event lifecycle, A25 final runtime-text contexts, leader/operative runtime bindings, and complete authenticated effective events/**/*.txt namespace",
    ),
    (
        "XUI23",
        "EIR-NE-009 canonical exact-a729 passive UI-open-independent recorder-POV-independent complete-D18 all-country command-ability lifecycle, availability, timer, cost, caster, target, and evaluator-context carrier",
    ),
    (
        "XUI26",
        "EIR-NE-015 complete same-hour XUI26 replay-payload family carriers with canonical record seal, source provenance, effective content, modset, language, typed domain payload, exact collection groups and final authored element semantics",
    ),
];

pub const PROMOTED_EXTENSION_PACKETS: &[&str] = &["XUI27"];

pub const OMITTED_PIXEL_CANVAS_ROUTES: &[(&str, &str)] = &[];

pub const OMITTED_EXTENSION_PACKETS: &[(&str, &str)] = &[];

pub const OMITTED_AUTHORITY_BLOCKED_PACKETS: &[(&str, &str)] = &[];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProductionUiRejection {
    UnsupportedDisplayVariant { width: usize, height: usize },
    MissingDisplayVariantCarrier(&'static str),
    UnsupportedInputVariant(InputVariant),
    UnsupportedExactDisplayVariant(DisplayVariantKey),
    Asset(String),
    IncompleteComposition(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionUiReport {
    pub viewport: [usize; 2],
    pub pov_tag: Option<String>,
    pub promoted_packets: Vec<String>,
    pub promoted_extension_packets: Vec<String>,
    pub omitted_authority_blocked_packets: Vec<String>,
    pub omitted_pixel_canvas_packets: Vec<String>,
    pub omitted_extension_packets: Vec<String>,
    pub drawn_elements: usize,
    pub sprite_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionUiOutput {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionXui16Output {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
    pub authored_root: String,
    pub source_record_id: String,
    pub materialized_row_ids: Vec<String>,
    pub applied_scroll_row: usize,
    pub input_region: crate::occupation_compositor::OccupationInputRegion,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionXui17Output {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
    pub authored_roots: Vec<String>,
    pub rendered_stable_ids: Vec<String>,
    pub hit_targets: Vec<crate::character_ui::CharacterHitTarget>,
    pub tooltip_targets: Vec<crate::character_ui::CharacterTooltipHitTarget>,
    pub semantic_state: crate::character_ui::ObserverCharacterUi,
    pub authored_root: String,
    pub materialized_row_ids: Vec<String>,
    pub applied_scroll_row: usize,
    pub input_region: crate::character_ui::CharacterInputRegion,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionXui11Output {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
    pub presentation: crate::politics_state::PoliticsCountryPresentation,
    pub input_region: crate::politics_state::PoliticsInputRegion,
    pub source_record_identity_sha256: String,
    pub tooltip_targets: Vec<crate::politics_state::PoliticsTooltipHitTarget>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionXui08Output {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
    pub record_sha256: String,
    pub capture_sequence: u64,
    pub input_region: crate::peace_conference_ui::PeaceConferenceInputRegion,
    pub hit_targets: Vec<crate::peace_conference_ui::PeaceConferenceHitTarget>,
    pub tooltip_targets: Vec<crate::peace_conference_ui::PeaceConferenceTooltipHitTarget>,
    pub materialized_participant_ids: Vec<String>,
    pub materialized_allocation_ids: Vec<String>,
    pub materialized_result_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionXui09Output {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
    pub model: crate::research_observer::ObserverResearchModel,
    pub folder: String,
    pub node_count: usize,
    pub line_cell_count: usize,
    pub xor_cell_count: usize,
    pub input_region: crate::research_compositor::ResearchInputRegion,
    pub node_hit_targets: Vec<crate::research_compositor::ResearchNodeHitTarget>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionXui14Output {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
    pub authored_root: String,
    pub country_tag: String,
    pub source_record_identity_sha256: String,
    pub record_sha256: String,
    pub input_region: crate::intelligence_presentation::IntelligenceInputRegion,
    pub displayed_row_ids: Vec<String>,
    pub tooltip_targets: Vec<crate::intelligence_presentation::IntelligenceTooltipHitTarget>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionXui18Output {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
    pub authored_root: String,
    pub source_record_id: String,
    pub decision_record_ids: Vec<String>,
    pub panel_region: crate::power_balance_compositor::PowerBalanceInputRegion,
    pub close_region: crate::power_balance_compositor::PowerBalanceInputRegion,
    pub marker_top_left: [i32; 2],
    pub range_splitter_top_left: Vec<[i32; 2]>,
    pub tooltip_targets: Vec<crate::power_balance_compositor::PowerBalanceTooltipHitTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverDecisionCategoryInsertion {
    pub category_id: String,
    pub stable_namespace: String,
    pub expanded: bool,
    pub insertion_cursor_y: i32,
    pub parent_extent: [i32; 4],
    pub clip_rect: [i32; 4],
    pub scroll_transform_y: i32,
    pub z_order: u32,
    pub input_owner: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverDecisionCategoryExtent {
    pub category_id: String,
    pub expanded: bool,
    pub header_rect: [i32; 4],
    pub clipped_header_rect: [i32; 4],
    pub rect: [i32; 4],
    pub clipped_rect: [i32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverDecisionCategoryComposeRequest {
    pub category_id: String,
    pub category_source_file: String,
    pub provider_id: Option<String>,
    pub category_ordinal: usize,
    pub stable_namespace: String,
    pub expanded: bool,
    pub list_y: i32,
    pub parent_width: usize,
    pub parent_height: usize,
    pub clip_rect: [i32; 4],
    pub scroll_transform_y: i32,
    pub z_order: u32,
    pub input_owner: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObserverDecisionCategoryInteraction {
    pub root_enabled: bool,
    pub visual_enabled: BTreeMap<String, bool>,
    pub click_enabled: BTreeMap<String, bool>,
    pub right_click_enabled: BTreeMap<String, bool>,
    pub effect_enabled: BTreeMap<String, bool>,
}

pub struct ObserverDecisionCategoryDynamicChild {
    pub element: GuiElement,
    pub local_rect: [i32; 4],
    pub category_source_file: String,
    pub provider_id: String,
    pub definition_source_file: String,
    pub authored_root_source_file: String,
    pub window_name: String,
    pub manager_order_index: u32,
    pub interaction: ObserverDecisionCategoryInteraction,
    pub widgets: Vec<ObserverDecisionCategoryDynamicWidget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverDecisionCategoryDynamicWidget {
    pub paint_ordinal: u32,
    pub authored_child_path: Vec<u32>,
    pub authored_name: String,
    pub composed_name: String,
    pub element_type: String,
    pub has_authored_tooltip: bool,
    pub alternate_tooltip_selected: crate::tooltip_compositor::RecordedFact<bool>,
    pub visual_enabled: Option<bool>,
    pub click_enabled: Option<bool>,
    pub right_click_enabled: Option<bool>,
    pub effect_names: Vec<String>,
}

pub enum ObserverDecisionCategoryDynamicOutcome {
    NotBound,
    MeasuredInactive,
    Inserted(ObserverDecisionCategoryDynamicChild),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverDecisionCategoryProviderReceipt {
    pub category_id: String,
    pub provider_id: String,
    pub category_source_file: String,
    pub definition_source_file: String,
    pub authored_root_source_file: String,
    pub manager_order_index: u32,
    pub final_local_rect: [i32; 4],
    pub stable_namespace: String,
    pub interaction: ObserverDecisionCategoryInteraction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xui19CardHitTarget {
    pub source_record_id: String,
    pub decision_id: String,
    pub element_path: Vec<u32>,
    pub rect: [i32; 4],
    pub availability: replay_core::DecisionCardAvailabilityExact,
    pub interface_state: u32,
    pub tooltip_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xui19CustomIconHitTarget {
    pub source_record_id: String,
    pub category_id: String,
    pub authored_tag: String,
    pub element_path: Vec<u32>,
    pub rect: [i32; 4],
    pub tooltip_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xui22WidgetHitTarget {
    pub category_id: String,
    pub provider_id: String,
    pub definition_source_file: String,
    pub authored_root_source_file: String,
    pub window_name: String,
    pub manager_order_index: u32,
    pub stable_namespace: String,
    pub paint_ordinal: u32,
    pub authored_child_path: Vec<u32>,
    pub element_path: Vec<u32>,
    pub authored_name: String,
    pub element_type: String,
    pub rect: [i32; 4],
    pub has_authored_tooltip: bool,
    pub alternate_tooltip_selected: crate::tooltip_compositor::RecordedFact<bool>,
    pub visual_enabled: Option<bool>,
    pub click_enabled: Option<bool>,
    pub right_click_enabled: Option<bool>,
    pub effect_names: Vec<String>,
}

struct PendingXui22WidgetTarget {
    category_id: String,
    provider_id: String,
    definition_source_file: String,
    authored_root_source_file: String,
    window_name: String,
    manager_order_index: u32,
    stable_namespace: String,
    widget: ObserverDecisionCategoryDynamicWidget,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionXui19Output {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
    pub country_tag: String,
    pub source_sequence: u64,
    pub previous_record_sha256: Option<String>,
    pub record_sha256: String,
    pub content_manifest_sha256: String,
    pub panel_rect: [i32; 4],
    pub decision_grid_rect: [i32; 4],
    pub max_scroll_px: i32,
    pub effective_scroll_px: i32,
    pub category_insertions: Vec<ObserverDecisionCategoryInsertion>,
    pub category_extents: Vec<ObserverDecisionCategoryExtent>,
    pub provider_receipts: Vec<ObserverDecisionCategoryProviderReceipt>,
    pub xui22_widget_hit_targets: Vec<Xui22WidgetHitTarget>,
    pub card_hit_targets: Vec<Xui19CardHitTarget>,
    pub custom_icon_hit_targets: Vec<Xui19CustomIconHitTarget>,
}

fn find_xui19_element_mut<'a>(
    element: &'a mut GuiElement,
    name: &str,
) -> Option<&'a mut GuiElement> {
    if element.name.eq_ignore_ascii_case(name) {
        return Some(element);
    }
    element
        .children
        .iter_mut()
        .find_map(|child| find_xui19_element_mut(child, name))
}

fn find_xui19_element<'a>(element: &'a GuiElement, name: &str) -> Option<&'a GuiElement> {
    if element.name.eq_ignore_ascii_case(name) {
        return Some(element);
    }
    element
        .children
        .iter()
        .find_map(|child| find_xui19_element(child, name))
}

fn xui19_primary_sprite_ref(element: &GuiElement) -> Option<&str> {
    for key in ["quadTextureSprite", "spriteType", "SpriteType"] {
        if let Some(value) = element.raw.first_scalar_ci(key) {
            return Some(value);
        }
    }
    element.sprite_refs.first().map(String::as_str)
}

fn xui19_card_state_matches_template(
    template: &str,
    has_target: bool,
    timer_kind: Option<replay_core::DecisionTimerKindExact>,
) -> bool {
    let targeted_template = template == "targeted_decision_item";
    has_target == targeted_template
        && (template != "timed_decision_item" || timer_kind.is_some())
        && !timer_kind.is_some_and(|kind| {
            matches!(
                kind,
                replay_core::DecisionTimerKindExact::Targeted
                    | replay_core::DecisionTimerKindExact::TargetedTimed
            ) && !targeted_template
        })
}

fn xui19_interface_frame_valid(interface_state: u32, frame_count: u32) -> bool {
    frame_count > 0 && interface_state < frame_count
}

fn find_xui19_element_path(element: &GuiElement, name: &str) -> Option<Vec<u32>> {
    if element.name == name {
        return Some(Vec::new());
    }
    for (index, child) in element.children.iter().enumerate() {
        if let Some(mut suffix) = find_xui19_element_path(child, name) {
            let index = u32::try_from(index).ok()?;
            suffix.insert(0, index);
            return Some(suffix);
        }
    }
    None
}

fn rename_xui19_clone(element: &mut GuiElement, suffix: &str) {
    element.name = format!("{}__{suffix}", element.name);
    for child in &mut element.children {
        rename_xui19_clone(child, suffix);
    }
}

fn xui19_cloned_name(original: &str, suffix: &str) -> String {
    format!("{original}__{suffix}")
}

fn copy_xui19_rgba_rect(
    destination: &mut RgbaImage,
    source: &RgbaImage,
    rect: Rect,
) -> std::result::Result<(), ProductionUiRejection> {
    if destination.width != source.width
        || destination.height != source.height
        || destination.pixels.len() != source.pixels.len()
        || destination.pixels.len()
            != destination
                .width
                .checked_mul(destination.height)
                .and_then(|pixels| pixels.checked_mul(4))
                .unwrap_or(usize::MAX)
    {
        return Err(ProductionUiRejection::IncompleteComposition(
            "XUI19 clip source/destination image geometry disagrees".into(),
        ));
    }
    let left = usize::try_from(rect.x().max(0)).map_err(|_| {
        ProductionUiRejection::IncompleteComposition("XUI19 clip left is invalid".into())
    })?;
    let top = usize::try_from(rect.y().max(0)).map_err(|_| {
        ProductionUiRejection::IncompleteComposition("XUI19 clip top is invalid".into())
    })?;
    let right = usize::try_from(rect.x().saturating_add(rect.w()).max(0))
        .map_err(|_| {
            ProductionUiRejection::IncompleteComposition("XUI19 clip right is invalid".into())
        })?
        .min(destination.width);
    let bottom = usize::try_from(rect.y().saturating_add(rect.h()).max(0))
        .map_err(|_| {
            ProductionUiRejection::IncompleteComposition("XUI19 clip bottom is invalid".into())
        })?
        .min(destination.height);
    if left >= right || top >= bottom {
        return Err(ProductionUiRejection::IncompleteComposition(
            "XUI19 decision-grid clip is empty".into(),
        ));
    }
    let byte_left = left.checked_mul(4).ok_or_else(|| {
        ProductionUiRejection::IncompleteComposition("XUI19 clip byte offset overflowed".into())
    })?;
    let byte_right = right.checked_mul(4).ok_or_else(|| {
        ProductionUiRejection::IncompleteComposition("XUI19 clip byte extent overflowed".into())
    })?;
    for y in top..bottom {
        let row_bytes = y
            .checked_mul(destination.width)
            .and_then(|row| row.checked_mul(4))
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI19 clip row byte offset overflowed".into(),
                )
            })?;
        let start = row_bytes.checked_add(byte_left).ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition("XUI19 clip start overflowed".into())
        })?;
        let end = row_bytes.checked_add(byte_right).ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition("XUI19 clip end overflowed".into())
        })?;
        destination.pixels[start..end].copy_from_slice(&source.pixels[start..end]);
    }
    Ok(())
}

fn intersect_xui19_rect(rect: [i32; 4], clip: [i32; 4]) -> Option<[i32; 4]> {
    let left = rect[0].max(clip[0]);
    let top = rect[1].max(clip[1]);
    let right = rect[0]
        .saturating_add(rect[2])
        .min(clip[0].saturating_add(clip[2]));
    let bottom = rect[1]
        .saturating_add(rect[3])
        .min(clip[1].saturating_add(clip[3]));
    (right > left && bottom > top).then_some([left, top, right - left, bottom - top])
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionXui21Output {
    pub image: RgbaImage,
    pub event_sequence: i64,
    pub root_source: String,
    pub root_window: String,
    pub root_region: crate::event_chrome::EventPopupInputRegion,
    pub minimize_region: crate::event_chrome::EventPopupInputRegion,
    pub minimize_child_path: Vec<u32>,
    pub minimize_tooltip_key: Option<String>,
    pub option_targets: Vec<crate::event_chrome::EventOptionHitTarget>,
}

fn xui11_tooltip_instance_token(
    country_tag: &str,
    tag_db_id: u32,
    effective_root_identity_sha256: &str,
) -> String {
    // TooltipOwner is the lifetime identity of the instantiated authored
    format!("xui11:{effective_root_identity_sha256}:{country_tag}:{tag_db_id}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverViewerUiState {
    pub selected_country: Option<String>,
    pub peace_conference: crate::peace_conference_ui::PeaceConferenceViewerState,
    pub politics: crate::politics_state::PoliticsViewerState,
    pub power_balance: crate::power_balance_compositor::PowerBalanceViewerState,
    pub intelligence: crate::intelligence_presentation::IntelligenceViewerState,
    pub focus: crate::focus_observer::FocusViewerState,
    pub research: crate::research_compositor::ResearchViewerState,
    pub character_selection: crate::character_ui::CharacterViewerSelection,
    pub trade_panel_open: bool,
    pub production_lines: crate::production_lines::Xui12ViewerState,
    pub designer: Option<crate::design_catalog::ObserverDesignerViewState>,
    pub trade: crate::trade_layout::CountryTradeViewerState,
    pub decision_panel_open: bool,
    pub decision_scroll_px: i32,
    pub collapsed_decision_categories: BTreeSet<String>,
    pub mapmode: crate::hud::MapmodeBinding,
    pub theatre_mode: Option<ObserverTheatreMode>,
    pub theatre_settings_land_id: Option<u32>,
    pub occupation: crate::occupation_compositor::OccupationViewerState,
    pub abilities: crate::active_abilities::ObserverAbilityViewerState,
    pub combat: Xui20ViewerState,
    pub events: crate::event_chrome::EventPopupViewerState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionXui20Output {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
    pub route: Xui20CanvasRoute,
    pub input_region: crate::combat_detail_canvas::Xui20InputRegion,
    pub tooltip_targets: Vec<crate::combat_detail_canvas::Xui20TooltipHitTarget>,
}

impl Default for ObserverViewerUiState {
    fn default() -> Self {
        let mut production_lines = crate::production_lines::Xui12ViewerState::default();
        production_lines.panel_open = false;
        Self {
            selected_country: None,
            peace_conference: crate::peace_conference_ui::PeaceConferenceViewerState::default(),
            politics: crate::politics_state::PoliticsViewerState::default(),
            power_balance: crate::power_balance_compositor::PowerBalanceViewerState::default(),
            intelligence: crate::intelligence_presentation::IntelligenceViewerState::default(),
            focus: crate::focus_observer::FocusViewerState::default(),
            research: crate::research_compositor::ResearchViewerState::default(),
            character_selection: crate::character_ui::CharacterViewerSelection::default(),
            trade_panel_open: false,
            production_lines,
            designer: None,
            trade: crate::trade_layout::CountryTradeViewerState::default(),
            decision_panel_open: false,
            decision_scroll_px: 0,
            collapsed_decision_categories: BTreeSet::new(),
            mapmode: crate::hud::MapmodeBinding::default(),
            theatre_mode: None,
            theatre_settings_land_id: None,
            occupation: crate::occupation_compositor::OccupationViewerState::default(),
            abilities: crate::active_abilities::ObserverAbilityViewerState::default(),
            combat: Xui20ViewerState::default(),
            events: crate::event_chrome::EventPopupViewerState::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObserverTheatreMode {
    Land,
    Navy,
    Air,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverPlaybackUiProfile {
    pub variant_key: DisplayVariantKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverUiGap {
    pub surface: &'static str,
    pub field: &'static str,
    pub required_domain: &'static str,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverTheatreSurface {
    pub country_tag: String,
    pub owner_tag_db_id: u32,
    pub section: replay_core::TheatreSelectorCountrySection,
    /// Owner ruling option A (lane RE dossier section 2.4): the land combat-log
    /// leaf is retired from schema 6 and never carried. The icon is DERIVED
    /// from authoritative recorded state -- true iff this surface's observer
    /// country is one of the session's measured player countries. The ONLY
    /// accepted source is the exact D57 player-country carrier
    /// (`map_viewer::player_country_set_leaf_status` certifies it); an absent,
    /// failed or legacy-fallback-fed set refuses admission by name instead of
    /// silently hiding a player's icon. The engine's two remaining conjuncts
    /// (combat-log window input state, per-client flag byte) are panel-local
    /// presentation that no all-country carrier reconstructs; see
    /// `extractor/native/runs/re_theatre_read_law/DOSSIER.md` section 2.4.
    pub combat_log_icon_shown: bool,
}

fn reconcile_theatre_settings_land_id(
    requested: Option<u32>,
    mut is_current_land_theatre: impl FnMut(u32) -> bool,
) -> Option<u32> {
    requested.filter(|object_id| *object_id != 0 && is_current_land_theatre(*object_id))
}

fn theatre_gap(field: &'static str, reason: impl Into<String>) -> ObserverUiGap {
    ObserverUiGap {
        surface: "XUI07/theatre_selector",
        field,
        required_domain: "D98 all-country theatre-selector carrier + XUI24 content admission",
        reason: reason.into(),
    }
}

/// COMPLETE, and it takes the entries rather than the view so the completeness
/// decision is made once, at the call site that also needs the selected
/// country's identity.
fn require_exact_d18_d98_country_census(
    roster: &[replay_core::D18CountryRosterEntry],
    d98_census: &[u32],
) -> std::result::Result<(), ObserverUiGap> {
    let mut d18 = std::collections::BTreeSet::new();
    for entry in roster {
        if entry.tag_db_id == 0 || !d18.insert(entry.tag_db_id) {
            return Err(theatre_gap(
                "all_country_controller_authority",
                "complete D18 roster contains a zero or duplicate country identity",
            ));
        }
    }
    let d18 = d18.into_iter().collect::<Vec<_>>();
    if d18 != d98_census {
        return Err(theatre_gap(
            "all_country_controller_authority",
            "D98 country census does not exactly equal the complete D18 roster",
        ));
    }
    Ok(())
}

fn shipping_d98_predecessor_sequence(
    expected_hour: u64,
) -> std::result::Result<u64, ObserverUiGap> {
    expected_hour.checked_add(1).ok_or_else(|| {
        theatre_gap(
            "source_sequence",
            "replay frame hour cannot derive the required D98 source sequence",
        )
    })?;
    Ok(expected_hour)
}

/// Owner ruling option A (lane RE dossier section 2.4): the land combat-log
/// leaf is retired from schema 6 and never carried. The icon is DERIVED from
/// authoritative recorded state -- true iff this surface's observer country is
/// one of the session's measured player countries -- and the ONLY accepted
/// source is the exact D57 player-country carrier:
/// `map_viewer::player_country_set_leaf_status` is that carrier's own
/// three-way answer (absent sidecar, present-but-failed read, or known and
/// decoded into a measured leaf), so a set fed by `build_session_metadata`'s
/// two legacy fallback keys -- which carry no identity or producer-order proof
/// -- refuses here exactly as if the set were missing. An absent or failed
/// read refuses by name too: a player's icon must never be silently hidden
/// behind an unavailable input.
fn derive_combat_log_icon(
    frame: &WorldFrame,
    country_tag: &str,
) -> std::result::Result<bool, ObserverUiGap> {
    if map_viewer::player_country_set_leaf_status(frame) != replay_core::ViewStatus::Present {
        return Err(theatre_gap(
            "combat_log_icon_source",
            "the exact D57 player-country carrier is not admitted for this hour; \
             the retired combat-log icon cannot be derived from proven state",
        ));
    }
    Ok(matches!(
        &frame.session_metadata().player_countries,
        replay_core::Leaf::Measured(players)
            if players
                .iter()
                .any(|player| matches!(&player.tag, replay_core::Leaf::Measured(tag) if tag == country_tag))
    ))
}

fn admit_observer_theatre_surface(
    frame: &WorldFrame,
    content_admission: &ValidatedContentAdmission,
    country_tag: &str,
) -> std::result::Result<ObserverTheatreSurface, ObserverUiGap> {
    // COMPLETE, not partial: the selected country's identity is only half of
    // what this surface needs - `require_exact_d18_d98_country_census` below
    // asserts the D98 all-country census EQUALS this roster, which no partial
    // roster can back.
    let roster = frame.d18_country_roster();
    let roster_entries = match roster.complete() {
        Ok(entries) => entries,
        Err(refusal) => {
            return Err(theatre_gap(
                "selected_country_identity",
                format!("D18 country roster is not complete: {refusal}"),
            ))
        }
    };
    let entry = roster_entries
        .iter()
        .find(|entry| entry.tag == country_tag)
        .cloned()
        .ok_or_else(|| {
            theatre_gap(
                "selected_country_identity",
                format!("selected country {country_tag:?} is absent from the complete D18 roster"),
            )
        })?;
    let expectation =
        replay_core::TheatreSelectorContentExpectation::from_content_admission(content_admission)
            .map_err(|reason| theatre_gap("content_identity", reason))?;
    let expected_hour = u64::try_from(frame.hour_index()).map_err(|_| {
        theatre_gap(
            "frame_hour",
            "replay frame has a negative hour and cannot authorize D98",
        )
    })?;
    let predecessor_sequence = shipping_d98_predecessor_sequence(expected_hour)?;
    let theatre = frame
        .theatre_selector()
        .authorize(&expectation, expected_hour, Some(predecessor_sequence));
    if theatre.status != replay_core::ViewStatus::Present {
        return Err(theatre_gap(
            "all_country_controller_authority",
            theatre
                .diagnostic
                .unwrap_or_else(|| "D98 theatre-selector frame is unavailable".into()),
        ));
    }
    if !theatre.producer_complete {
        return Err(theatre_gap(
            "all_country_controller_authority",
            "D98 producer did not admit every country in its complete census",
        ));
    }
    require_exact_d18_d98_country_census(roster_entries, &theatre.census)?;
    let section = theatre.country(entry.tag_db_id);
    if section.status != replay_core::ViewStatus::Present {
        return Err(theatre_gap(
            "selected_country_section",
            section
                .rejection
                .clone()
                .unwrap_or_else(|| "selected country D98 section was rejected".into()),
        ));
    }
    // Owner ruling option A: derive the icon, but ONLY from proven state (see
    // `derive_combat_log_icon` for the fail-closed carrier law).
    let combat_log_icon_shown = derive_combat_log_icon(frame, country_tag)?;
    Ok(ObserverTheatreSurface {
        country_tag: country_tag.to_string(),
        owner_tag_db_id: entry.tag_db_id,
        section,
        combat_log_icon_shown,
    })
}


#[derive(Clone, Debug, PartialEq)]
pub struct ObserverDecisionSurface {
    pub country_tag: String,
    pub source_sequence: Option<u64>,
    pub record_sha256: Option<String>,
    pub authored_root: &'static str,
    pub authored_scene_source: String,
    pub authored_template_count: usize,
    pub viewer_scroll_px: i32,
    pub collapsed_categories: BTreeSet<String>,
    pub pixel_output: Option<ProductionXui19Output>,
    pub gaps: Vec<ObserverUiGap>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverCombatSurface {
    pub route: Xui20CanvasRoute,
    pub pixel_output: Option<ProductionUiOutput>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverTradeSurface {
    pub country_tag: Option<String>,
    pub screen: Option<AdmittedTradeScreen>,
    pub authored_root: &'static str,
    pub canvas: Option<crate::trade_compositor::StrictXui15PanelInput>,
    pub measured_empty: bool,
    pub gaps: Vec<ObserverUiGap>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverProductionLinesSurface {
    pub country_tag: Option<String>,
    pub route: Option<crate::production_lines::AdmittedXui12ProductionLineRoute>,
    pub canvas: Option<crate::production_lines::StrictXui12CanvasInput>,
    pub gaps: Vec<ObserverUiGap>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverDesignerRoute {
    pub country_tag: Option<String>,
    pub viewer: Option<crate::design_catalog::ObserverDesignerViewState>,
    pub measured_empty: bool,
    pub gaps: Vec<ObserverUiGap>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverCharacterSurface {
    pub country_tag: Option<String>,
    pub model: Option<crate::character_ui::ObserverCharacterUi>,
    pub gaps: Vec<ObserverUiGap>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverAbilityRoute {
    pub country_tag: Option<String>,
    pub source_record_id: Option<String>,
    pub admitted: Option<crate::active_abilities::AdmittedObserverAbilityFrame>,
    pub selected_context: Option<crate::active_abilities::ObserverAbilityContext>,
    pub measured_empty: bool,
    pub gaps: Vec<ObserverUiGap>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverFocusRoute {
    country_tag: Option<String>,
    semantic: Option<crate::national_focus::AdmittedNationalFocusPresentation>,
    scene: Option<crate::focus_observer::PreparedObserverFocusScene>,
    measured_empty: bool,
    gaps: Vec<ObserverUiGap>,
}

impl ObserverFocusRoute {
    pub fn country_tag(&self) -> Option<&str> {
        self.country_tag.as_deref()
    }

    pub fn semantic(&self) -> Option<&crate::national_focus::AdmittedNationalFocusPresentation> {
        self.semantic.as_ref()
    }

    pub fn scene(&self) -> Option<&crate::focus_observer::PreparedObserverFocusScene> {
        self.scene.as_ref()
    }

    pub fn measured_empty(&self) -> bool {
        self.measured_empty
    }

    pub fn gaps(&self) -> &[ObserverUiGap] {
        &self.gaps
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverEventRoute {
    pub country_tag: Option<String>,
    pub admitted_history: Option<Vec<crate::event_chrome::AdmittedFiredEvent>>,
    pub event_tick: Option<i64>,
    pub visible_sequences: Vec<i64>,
    pub focused_sequence: Option<i64>,
    pub measured_empty: bool,
    pub gaps: Vec<ObserverUiGap>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverUiSemanticFrame {
    pub hud_binding: HudBinding,
    pub playback_playing: bool,
    pub playback_speed_step: usize,
    pub theatre: Option<ObserverTheatreSurface>,
    pub theatre_mode: Option<ObserverTheatreMode>,
    pub theatre_settings_land_id: Option<u32>,
    pub trade: Option<ObserverTradeSurface>,
    pub production_lines: Option<ObserverProductionLinesSurface>,
    pub designer: Option<ObserverDesignerRoute>,
    pub characters: Option<ObserverCharacterSurface>,
    pub decisions: Option<ObserverDecisionSurface>,
    pub focus: ObserverFocusRoute,
    pub abilities: ObserverAbilityRoute,
    pub combat: Option<ObserverCombatSurface>,
    pub events: ObserverEventRoute,
    pub gaps: Vec<ObserverUiGap>,
}

pub fn tooltip_binding_from_frame(
    frame: &WorldFrame,
    pov_tag: Option<&str>,
    viewport_width: usize,
    viewport_height: usize,
) -> Option<TooltipBinding> {
    tooltip_binding_from_exact_view(
        frame.resolved_tooltip_exact(),
        pov_tag,
        viewport_width,
        viewport_height,
    )
}

pub fn tooltip_binding_from_exact_view(
    view: &replay_core::ResolvedTooltipExactView,
    pov_tag: Option<&str>,
    viewport_width: usize,
    viewport_height: usize,
) -> Option<TooltipBinding> {
    if view.status != replay_core::ViewStatus::Present {
        return None;
    }
    let exact = view.frame.as_ref()?;
    let [instance] = exact.instances.as_slice() else {
        return None;
    };
    let pov_matches = match pov_tag {
        Some(tag) => {
            instance.pov_country_or_global == tag || instance.pov_country_or_global == "global"
        }
        None => instance.pov_country_or_global == "global",
    };
    if !pov_matches
        || usize::try_from(instance.viewport_width_px).ok() != Some(viewport_width)
        || usize::try_from(instance.viewport_height_px).ok() != Some(viewport_height)
    {
        return None;
    }
    Some(TooltipBinding {
        x: instance.screen_rect.x,
        y: instance.screen_rect.y,
        width: usize::try_from(instance.screen_rect.width).ok()?,
        height: usize::try_from(instance.screen_rect.height).ok()?,
        text: instance.final_resolved_text.clone(),
        owner_root_name: instance.tooltip_owner_authored_name.clone(),
    })
}

pub struct ProductionUiRenderer {
    resolver: AssetResolver,
    registry: SpriteRegistry,
    localisation: Localisation,
    playback_localisation_language: String,
    character_portraits: std::result::Result<crate::character_ui::CharacterPortraitCatalog, String>,
    character_traits: std::result::Result<crate::character_ui::UnitLeaderTraitCatalog, String>,
    character_skill_costs: std::result::Result<crate::character_ui::LeaderSkillCostCatalog, String>,
    ability_catalog: std::result::Result<crate::active_abilities::AdmittedAbilityCatalog, String>,
    power_balance_catalog:
        std::result::Result<crate::power_balance::PowerBalanceDefinitionCatalog, String>,
    power_balance_decisions: std::result::Result<DecisionSet, String>,
    event_definitions: std::result::Result<BTreeMap<String, crate::event_chrome::EventDef>, String>,
    event_layout: std::result::Result<crate::gui::LayoutSet, String>,
    content_admission: ValidatedContentAdmission,
    playback_variant_key: Option<DisplayVariantKey>,
    asset_cache: Mutex<UiAssetCache>,
}

#[derive(Clone, Debug, Default)]
pub struct ProductionXui05Controller {
    lifecycle: Option<TooltipLifecycle>,
    last_viewer_clock_micros: Option<u64>,
    selected_country: Option<String>,
    retained: Option<PreparedComposedTooltip>,
    retained_authenticated_owner:
        Option<crate::window_manager_law::AuthenticatedReplayTooltipOwner>,
    retained_delayed_content: bool,
}

impl ProductionXui05Controller {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProductionXui05Disposition {
    NoHoveredOwner,
    HoveredWithoutAuthoredTooltip,
    DwellPending,
    Visible {
        image: RgbaImage,
        owner: TooltipOwner,
        authenticated_owner: crate::window_manager_law::AuthenticatedReplayTooltipOwner,
        row_count: usize,
        delayed_content_visible: bool,
    },
    SemanticVisible {
        image: RgbaImage,
        owner: TooltipOwner,
        row_count: usize,
        delayed_content_visible: bool,
    },
}

fn resolve_xui21_authored_tooltip_text(
    localisation: &Localisation,
    key: &str,
) -> std::result::Result<(String, Vec<AdmittedTooltipSource>), ProductionUiRejection> {
    let resolved = crate::text_law::resolve_localisation_preserving_inline_icons(
        &localisation.entries,
        key,
        &crate::text_law::RuntimeTextContext::default(),
    );
    if !resolved.admitted() || resolved.text.is_empty() {
        return Err(ProductionUiRejection::IncompleteComposition(format!(
            "XUI21 authored tooltip {key} did not resolve exactly: {:?}",
            resolved.issues
        )));
    }
    let mut sources = Vec::with_capacity(resolved.source_keys.len());
    for source_key in resolved.source_keys {
        if let Some(ambiguous) = localisation.ambiguous_sources_for_key(&source_key) {
            return Err(ProductionUiRejection::IncompleteComposition(format!(
                "XUI21 tooltip localisation key {source_key} is ambiguous in {ambiguous:?}"
            )));
        }
        let source = localisation
            .source_file_for_key(&source_key)
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI21 tooltip localisation key {source_key} has no unique admitted source"
                ))
            })?;
        sources.push(AdmittedTooltipSource::EffectiveLocalisation {
            key: source_key,
            source_files: vec![source.to_string()],
        });
    }
    Ok((resolved.text, sources))
}

impl ProductionUiRenderer {
    pub fn new(
        resolver: AssetResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
    ) -> Result<Self> {
        Self::new_with_playback_profile(resolver, content_admission, None)
    }

    pub fn new_for_playback(
        resolver: AssetResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        profile: &crate::variant_admission::PlaybackUiProfile,
    ) -> Result<Self> {
        Self::new_with_playback_profile(resolver, content_admission, Some(profile))
    }

    fn new_with_playback_profile(
        resolver: AssetResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        profile: Option<&crate::variant_admission::PlaybackUiProfile>,
    ) -> Result<Self> {
        if !resolver.is_admitted_for(content_admission) {
            return Err(GameUiError::missing(
                "production UI resolver is not restricted to the admitted replay content manifest",
            ));
        }
        for source in REQUIRED_AUTHORED_HUD_SOURCES {
            content_admission
                .require_used_content(source)
                .map_err(GameUiError::missing)?;
        }
        let mut registry = SpriteRegistry::new();
        let files = resolver.list_files_recursive("interface", Some("gfx"))?;
        if files.is_empty() {
            return Err(GameUiError::missing(
                "production UI has no effective interface/*.gfx files",
            ));
        }
        for file in files {
            registry.parse_file(&file.path, &file.rel_path)?;
        }
        if registry.is_empty() {
            return Err(GameUiError::missing(
                "production UI effective GFX registry is empty",
            ));
        }
        let localisation = match profile {
            Some(profile) => {
                Localisation::load_admitted_language(&resolver, profile.localisation_language())?
            }
            None => Localisation::load_english(&resolver)?,
        };
        let playback_localisation_language = profile
            .map(|profile| profile.localisation_language().to_string())
            .unwrap_or_else(|| PRODUCTION_UI_LANGUAGE.to_string());
        let character_portraits = crate::character_ui::CharacterPortraitCatalog::load(&resolver)
            .map_err(|error| error.to_string());
        let character_traits = crate::character_ui::UnitLeaderTraitCatalog::load(&resolver)
            .map_err(|error| error.to_string());
        let character_skill_costs = crate::character_ui::LeaderSkillCostCatalog::load(&resolver)
            .map_err(|error| error.to_string());
        let ability_catalog = crate::active_abilities::load_admitted_ability_catalog(
            &resolver,
            &registry,
            &localisation,
        )
        .map_err(|error| error.to_string());
        let power_balance_catalog =
            crate::power_balance::PowerBalanceDefinitionCatalog::from_effective_vfs(&resolver)
                .map_err(|error| error.to_string());
        let power_balance_decisions = parse_decisions(&resolver).map_err(|error| error.to_string());
        let event_definitions =
            crate::event_chrome::load_admitted_event_definitions(&resolver, content_admission);
        let event_layout =
            crate::event_chrome::load_event_layout(&resolver).map_err(|error| error.to_string());
        Ok(Self {
            resolver,
            registry,
            localisation,
            playback_localisation_language,
            character_portraits,
            character_traits,
            character_skill_costs,
            ability_catalog,
            power_balance_catalog,
            power_balance_decisions,
            event_definitions,
            event_layout,
            content_admission: content_admission.clone(),
            playback_variant_key: profile.map(|profile| profile.key().clone()),
            asset_cache: Mutex::new(UiAssetCache::default()),
        })
    }

    pub fn resolver(&self) -> &AssetResolver {
        &self.resolver
    }

    pub fn power_balance_catalog(
        &self,
    ) -> std::result::Result<&crate::power_balance::PowerBalanceDefinitionCatalog, &str> {
        self.power_balance_catalog.as_ref().map_err(String::as_str)
    }

    pub fn power_balance_decisions(&self) -> std::result::Result<&DecisionSet, &str> {
        self.power_balance_decisions
            .as_ref()
            .map_err(String::as_str)
    }

    pub fn event_definitions(
        &self,
    ) -> std::result::Result<&BTreeMap<String, crate::event_chrome::EventDef>, &str> {
        self.event_definitions.as_ref().map_err(String::as_str)
    }

    pub fn content_manifest_digest_sha256(&self) -> &str {
        self.content_admission.manifest_digest_sha256()
    }

    pub fn registry(&self) -> &SpriteRegistry {
        &self.registry
    }

    pub fn xui09_available_folders(
        &self,
        catalog: &crate::static_research::StaticResearchUiCatalog,
    ) -> std::result::Result<Vec<String>, ProductionUiRejection> {
        for source in [
            crate::research_compositor::XUI09_GUI_SOURCE,
            crate::research_compositor::XUI09_SLOT_GUI_SOURCE,
        ] {
            self.content_admission
                .require_used_content(source)
                .map_err(ProductionUiRejection::Asset)?;
        }
        crate::research_compositor::available_authored_research_folders(&self.resolver, catalog)
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))
    }

    pub fn compose_xui09_research_surface(
        &self,
        frame: &WorldFrame,
        selected_country: &str,
        catalog: &crate::static_research::StaticResearchUiCatalog,
        viewer: &crate::research_compositor::ResearchViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<ProductionXui09Output>, ProductionUiRejection> {
        if !viewer.panel_open {
            return Ok(None);
        }
        let selected_folder = viewer.selected_folder.as_deref().ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition(
                "XUI09 open panel has no viewer-selected authored folder".into(),
            )
        })?;
        self.xui09_available_folders(catalog)?;
        let model = crate::research_observer::observer_research_from_complete_frame(
            frame,
            selected_country,
            catalog,
        )
        .map_err(|error| {
            ProductionUiRejection::IncompleteComposition(format!(
                "XUI09 passive all-country admission rejected: {error}"
            ))
        })?;
        let mut cache = self
            .asset_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let composed = crate::research_compositor::compose_authored_research_window(
            &self.resolver,
            &self.registry,
            &self.localisation,
            &mut cache,
            &model,
            catalog,
            selected_folder,
            viewer.pan_pixels,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| {
            ProductionUiRejection::IncompleteComposition(format!(
                "XUI09 authored research composition rejected: {error}"
            ))
        })?;
        validate_complete_compose_report(&composed.output.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;
        let report = ProductionUiReport {
            viewport: [viewport_width, viewport_height],
            pov_tag: Some(selected_country.to_string()),
            promoted_packets: PROMOTED_XUI_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            promoted_extension_packets: PROMOTED_EXTENSION_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            omitted_authority_blocked_packets: OMITTED_AUTHORITY_BLOCKED_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_pixel_canvas_packets: OMITTED_PIXEL_CANVAS_ROUTES
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_extension_packets: OMITTED_EXTENSION_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            drawn_elements: composed.output.report.drawn_elements,
            sprite_count: self.registry.len(),
        };
        Ok(Some(ProductionXui09Output {
            image: composed.output.image,
            report,
            model,
            folder: composed.folder,
            node_count: composed.node_count,
            line_cell_count: composed.line_cell_count,
            xor_cell_count: composed.xor_cell_count,
            input_region: composed.input_region,
            node_hit_targets: composed.node_hit_targets,
        }))
    }

    pub fn compose_xui08_peace_conference_surface(
        &self,
        frame: &WorldFrame,
        viewer: &crate::peace_conference_ui::PeaceConferenceViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<ProductionXui08Output>, ProductionUiRejection> {
        if !viewer.open {
            return Ok(None);
        }
        for source in [
            crate::peace_conference_ui::PEACE_CONFERENCE_GUI,
            "interface/peaceconferencewindow.gfx",
        ] {
            self.content_admission
                .require_used_content(source)
                .map_err(ProductionUiRejection::Asset)?;
        }
        let exact = frame
            .peace_conference_presentation_exact()
            .presentation
            .as_ref()
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI08 passive peace presentation is unavailable".into(),
                )
            })?;
        if exact.content_manifest_sha256 != self.content_admission.manifest_digest_sha256() {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI08 carrier content manifest differs from playback admission".into(),
            ));
        }
        let snapshot = crate::readonly_design_peace::peace_conference_snapshot_from_world(frame)
            .map_err(|error| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI08 snapshot conversion rejected: {error:?}"
                ))
            })?;
        let admitted =
            crate::readonly_design_peace::admit_peace_conference(&snapshot).map_err(|error| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI08 semantic admission rejected: {error:?}"
                ))
            })?;
        let Some(composed) = crate::peace_conference_ui::render_peace_conference(
            &self.resolver,
            &self.registry,
            &self.localisation,
            exact,
            &admitted,
            viewer,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?
        else {
            return Ok(None);
        };
        validate_complete_compose_report(&composed.output.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;
        let report = ProductionUiReport {
            viewport: [viewport_width, viewport_height],
            pov_tag: None,
            promoted_packets: PROMOTED_XUI_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            promoted_extension_packets: PROMOTED_EXTENSION_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            omitted_authority_blocked_packets: OMITTED_AUTHORITY_BLOCKED_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_pixel_canvas_packets: OMITTED_PIXEL_CANVAS_ROUTES
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_extension_packets: OMITTED_EXTENSION_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            drawn_elements: composed.output.report.drawn_elements,
            sprite_count: self.registry.len(),
        };
        Ok(Some(ProductionXui08Output {
            image: composed.output.image,
            report,
            record_sha256: exact.record_sha256.clone(),
            capture_sequence: exact.capture_sequence,
            input_region: composed.input_region,
            hit_targets: composed.hit_targets,
            tooltip_targets: composed.tooltip_targets,
            materialized_participant_ids: composed.materialized_participant_ids,
            materialized_allocation_ids: composed.materialized_allocation_ids,
            materialized_result_ids: composed.materialized_result_ids,
        }))
    }

    pub fn compose_xui11_politics_surface(
        &self,
        frame: &WorldFrame,
        selected_country: &str,
        catalog: &crate::static_research::StaticResearchUiCatalog,
        power_balance_launcher: Option<&crate::power_balance::PowerBalancePresentation>,
        viewer: &crate::politics_state::PoliticsViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<ProductionXui11Output>, ProductionUiRejection> {
        if !viewer.panel_open {
            return Ok(None);
        }
        for source in [
            crate::politics_state::POLITICS_GUI_SOURCE,
            crate::politics_state::POLITICS_SPIRIT_TEMPLATE_SOURCE,
            crate::politics_state::POLITICS_SCROLLBAR_SOURCE,
        ] {
            self.content_admission
                .require_used_content(source)
                .map_err(ProductionUiRejection::Asset)?;
        }
        let presentation = crate::politics_state::admit_politics_presentation(
            frame,
            selected_country,
            catalog,
            self.content_admission.manifest_digest_sha256(),
        )
        .map_err(|error| {
            ProductionUiRejection::IncompleteComposition(format!(
                "XUI11 passive politics admission rejected: {error}"
            ))
        })?;
        let layout = crate::politics_state::PoliticsAuthoredLayout::load(
            &self.resolver,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let output = crate::politics_state::render_politics(
            &self.resolver,
            &self.registry,
            &self.localisation,
            &presentation,
            power_balance_launcher,
            viewer,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        validate_complete_compose_report(&output.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;
        let input_region = crate::politics_state::politics_input_region(
            &layout,
            &presentation,
            power_balance_launcher,
            &output,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let tooltip_targets = crate::politics_state::politics_tooltip_hit_targets(
            &output,
            &layout,
            &presentation,
            &xui11_tooltip_instance_token(
                &presentation.country_tag,
                presentation.tag_db_id,
                &presentation.effective_root_identity_sha256,
            ),
        )
        .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        let report = ProductionUiReport {
            viewport: [viewport_width, viewport_height],
            pov_tag: Some(selected_country.to_string()),
            promoted_packets: PROMOTED_XUI_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            promoted_extension_packets: PROMOTED_EXTENSION_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            omitted_authority_blocked_packets: OMITTED_AUTHORITY_BLOCKED_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_pixel_canvas_packets: OMITTED_PIXEL_CANVAS_ROUTES
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_extension_packets: OMITTED_EXTENSION_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            drawn_elements: output.report.drawn_elements,
            sprite_count: self.registry.len(),
        };
        Ok(Some(ProductionXui11Output {
            image: output.image,
            report,
            source_record_identity_sha256: presentation.source_record_identity.clone(),
            presentation,
            input_region,
            tooltip_targets,
        }))
    }

    pub fn compose_xui18_power_balance_surface(
        &self,
        selected_country: &str,
        panel: &crate::power_balance::PowerBalancePresentation,
        viewer: &crate::power_balance_compositor::PowerBalanceViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<ProductionXui18Output>, ProductionUiRejection> {
        if !viewer.panel_open {
            return Ok(None);
        }
        if panel.country_tag != selected_country || panel.display_ordinal != 0 {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI18 selected-country presentation identity disagrees".into(),
            ));
        }
        if panel.provenance.effective_content_manifest_sha256
            != self.content_admission.manifest_digest_sha256()
        {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI18 presentation content manifest differs from the renderer's admitted VFS"
                    .into(),
            ));
        }
        for source in [
            crate::power_balance_compositor::XUI18_GUI_SOURCE,
            crate::power_balance_compositor::XUI18_DECISION_GUI_SOURCE,
        ] {
            self.content_admission
                .require_used_content(source)
                .map_err(ProductionUiRejection::Asset)?;
        }
        self.content_admission
            .require_used_content(
                self.power_balance_catalog()
                    .map_err(|reason| ProductionUiRejection::Asset(reason.to_string()))?
                    .get(&panel.balance_id)
                    .ok_or_else(|| {
                        ProductionUiRejection::IncompleteComposition(format!(
                            "XUI18 effective definition {:?} is absent",
                            panel.balance_id
                        ))
                    })?
                    .source_file
                    .as_str(),
            )
            .map_err(ProductionUiRejection::Asset)?;
        let mut cache = self.asset_cache.lock().map_err(|_| {
            ProductionUiRejection::Asset("production UI asset cache is poisoned".into())
        })?;
        let composed = crate::power_balance_compositor::compose_power_balance_surface(
            &self.resolver,
            &self.registry,
            &self.localisation,
            &mut cache,
            panel,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        validate_complete_compose_report(&composed.output.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;
        let report = ProductionUiReport {
            viewport: [viewport_width, viewport_height],
            pov_tag: Some(selected_country.to_string()),
            promoted_packets: PROMOTED_XUI_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            promoted_extension_packets: PROMOTED_EXTENSION_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            omitted_authority_blocked_packets: OMITTED_AUTHORITY_BLOCKED_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_pixel_canvas_packets: OMITTED_PIXEL_CANVAS_ROUTES
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_extension_packets: OMITTED_EXTENSION_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            drawn_elements: composed.output.report.drawn_elements,
            sprite_count: self.registry.len(),
        };
        Ok(Some(ProductionXui18Output {
            image: composed.output.image,
            report,
            authored_root: composed.authored_root,
            source_record_id: composed.source_record_id,
            decision_record_ids: composed.decision_record_ids,
            panel_region: composed.panel_region,
            close_region: composed.close_region,
            marker_top_left: composed.marker_top_left,
            range_splitter_top_left: composed.range_splitter_top_left,
            tooltip_targets: composed.tooltip_targets,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn compose_xui19_decision_surface<F>(
        &self,
        frame: &WorldFrame,
        selected_country: &str,
        decision_catalog: &DecisionStaticCatalog,
        collapsed_categories: &BTreeSet<String>,
        scroll_px: i32,
        viewport_width: usize,
        viewport_height: usize,
        category_child: &mut F,
    ) -> std::result::Result<Option<ProductionXui19Output>, ProductionUiRejection>
    where
        F: FnMut(
            &ObserverDecisionCategoryComposeRequest,
            &mut ComposeOptions,
        ) -> std::result::Result<
            ObserverDecisionCategoryDynamicOutcome,
            ProductionUiRejection,
        >,
    {
        if selected_country.is_empty() || scroll_px < 0 {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI19 requires a selected country and non-negative viewer scroll".into(),
            ));
        }
        self.content_admission
            .require_used_content(DECISION_VIEW_GUI)
            .map_err(ProductionUiRejection::Asset)?;
        let view = frame.decision_evaluation_exact();
        if view.status != replay_core::ViewStatus::Present {
            return Ok(None);
        }
        let carrier = view.carrier.as_ref().ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition(
                "XUI19 Present view lost its admitted carrier".into(),
            )
        })?;
        if carrier.content_manifest_sha256 != self.content_admission.manifest_digest_sha256()
            || carrier.decision_catalog_sha256 != decision_catalog.source_sha256
        {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI19 carrier disagrees with admitted content or packaged decision catalog".into(),
            ));
        }
        let expected_language = self
            .playback_variant_key
            .as_ref()
            .map(|key| key.localisation_language.as_str())
            .unwrap_or(PRODUCTION_UI_LANGUAGE);
        if carrier.localisation_language != expected_language {
            return Err(ProductionUiRejection::IncompleteComposition(format!(
                "XUI19 carrier language {:?} differs from playback language {:?}",
                carrier.localisation_language, expected_language
            )));
        }
        let country = view.country(selected_country).ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition(format!(
                "XUI19 complete country census lacks selected country {selected_country}"
            ))
        })?;
        let set = parse_decisions(&self.resolver)
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        if set.recoveries != 0 || !set.orphan_category_blocks.is_empty() {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI19 effective decision definitions are structurally incomplete".into(),
            ));
        }
        let definitions = set
            .decisions
            .iter()
            .map(|definition| (definition.id.as_str(), definition))
            .collect::<BTreeMap<_, _>>();
        if collapsed_categories.iter().any(|category| {
            !country
                .categories
                .iter()
                .any(|row| &row.category_id == category)
        }) {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI19 viewer collapsed a category outside the exact visible set".into(),
            ));
        }

        let scene = load_decision_authored_scene(&self.resolver)
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let mut root = scene.root.clone();
        let mut options = ComposeOptions::new(viewport_width, viewport_height)
            .with_localisation(&self.localisation);
        let preliminary = {
            let mut cache = self.asset_cache.lock().map_err(|_| {
                ProductionUiRejection::Asset("production UI asset cache is poisoned".into())
            })?;
            Compositor::with_cache(&self.resolver, &self.registry, &mut cache)
                .compose(&root, &options)
        };
        validate_complete_compose_report(&preliminary.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;
        let grid_rect = preliminary
            .debug_rects
            .iter()
            .find(|rect| rect.name == DECISION_GRID)
            .map(|rect| Rect::new(rect.x, rect.y, rect.w, rect.h))
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI19 authored decision_grid has no resolved rectangle".into(),
                )
            })?;
        let panel_rect = preliminary
            .debug_rects
            .iter()
            .find(|rect| rect.name == ROOT_WINDOW)
            .map(|rect| [rect.x, rect.y, rect.w, rect.h])
            .filter(|rect| rect[2] > 0 && rect[3] > 0)
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI19 authored root has no positive resolved rectangle".into(),
                )
            })?;
        let grid_clip = [grid_rect.x(), grid_rect.y(), grid_rect.w(), grid_rect.h()];
        let grid_layout = gridbox_layout(
            &scene.decision_grid,
            Rect::new(0, 0, viewport_width as i32, viewport_height as i32),
            &options,
        )
        .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        let local_parent = Rect::new(0, 0, grid_rect.w(), grid_rect.h());

        let mut children = Vec::<GuiElement>::new();
        let mut child_sizes = Vec::<(i32, i32)>::new();
        let mut child_categories = Vec::<String>::new();
        let mut category_headers = Vec::<(String, bool, usize, String)>::new();
        let mut pending_provider =
            Vec::<(usize, [i32; 2], ObserverDecisionCategoryProviderReceipt)>::new();
        let mut pending_xui22_widgets = Vec::<PendingXui22WidgetTarget>::new();
        let mut pending_cards = Vec::<(
            String,
            String,
            String,
            replay_core::DecisionCardAvailabilityExact,
            u32,
            String,
        )>::new();
        let mut pending_custom_icons = Vec::<(String, String, String, String, String)>::new();
        let mut logical_cursor_y = 0i32;
        let mut card_ordinal = 0usize;

        for (category_ordinal, category) in country.categories.iter().enumerate() {
            let definition = set.categories.get(&category.category_id).ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI19 exact category {} is absent from effective definitions",
                    category.category_id
                ))
            })?;
            if category.custom_icon_definition_count as usize != definition.custom_icons.len() {
                return Err(ProductionUiRejection::IncompleteComposition(format!(
                    "XUI19 category {} custom-icon definition count disagrees with effective files",
                    category.category_id
                )));
            }
            let expanded = !collapsed_categories.contains(&category.category_id);
            let namespace = format!("xui19_category_{category_ordinal}");
            let mut header = scene.category_header.clone();
            let header_rect = resolve_element_rect(&header, local_parent, (0, 0), &options);
            if header_rect.w() <= 0 || header_rect.h() <= 0 {
                return Err(ProductionUiRejection::IncompleteComposition(
                    "XUI19 category header has non-positive authored geometry".into(),
                ));
            }
            rename_xui19_clone(&mut header, &namespace);
            options.text_overrides.insert(
                xui19_cloned_name("name_text", &namespace),
                category.name_text.clone(),
            );
            if let Some(sprite) = category_icon_sprite(definition) {
                if self.registry.get(&sprite).is_none() {
                    return Err(ProductionUiRejection::IncompleteComposition(format!(
                        "XUI19 category sprite {sprite} is missing"
                    )));
                }
                options
                    .sprite_name_overrides
                    .insert(xui19_cloned_name("icon", &namespace), sprite);
            }
            let header_index = children.len();
            children.push(header);
            child_sizes.push((header_rect.w(), header_rect.h()));
            child_categories.push(category.category_id.clone());
            logical_cursor_y = logical_cursor_y
                .checked_add(header_rect.h())
                .ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI19 category cursor overflowed".into(),
                    )
                })?;
            category_headers.push((
                category.category_id.clone(),
                expanded,
                header_index,
                namespace.clone(),
            ));
            if !expanded {
                continue;
            }

            let cursor_origin = grid_layout
                .explicit_coordinate_origin((0, logical_cursor_y))
                .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
            let input_owner = format!("XUI19/{selected_country}/{}", category.category_id);
            let request = ObserverDecisionCategoryComposeRequest {
                category_id: category.category_id.clone(),
                category_source_file: definition.source_file.clone(),
                provider_id: definition.scripted_gui.clone(),
                category_ordinal,
                stable_namespace: namespace.clone(),
                expanded: true,
                list_y: cursor_origin.1.saturating_sub(grid_layout.rect.y()),
                parent_width: usize::try_from(grid_rect.w()).map_err(|_| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI19 decision-grid width is not representable".into(),
                    )
                })?,
                parent_height: usize::try_from(grid_rect.h()).map_err(|_| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI19 decision-grid height is not representable".into(),
                    )
                })?,
                clip_rect: grid_clip,
                scroll_transform_y: -scroll_px,
                z_order: u32::try_from(category_ordinal).map_err(|_| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI19 category z-order overflowed".into(),
                    )
                })?,
                input_owner: input_owner.clone(),
            };
            let supplied = category_child(&request, &mut options)?;
            let supplied = match (definition.scripted_gui.as_ref(), supplied) {
                (None, ObserverDecisionCategoryDynamicOutcome::NotBound) => None,
                (Some(_), ObserverDecisionCategoryDynamicOutcome::MeasuredInactive) => None,
                (Some(_), ObserverDecisionCategoryDynamicOutcome::Inserted(child)) => Some(child),
                (Some(provider), ObserverDecisionCategoryDynamicOutcome::NotBound) => {
                    return Err(ProductionUiRejection::IncompleteComposition(format!(
                        "XUI19 category {} requires measured provider {provider}",
                        category.category_id
                    )))
                }
                (None, ObserverDecisionCategoryDynamicOutcome::MeasuredInactive)
                | (None, ObserverDecisionCategoryDynamicOutcome::Inserted(_)) => {
                    return Err(ProductionUiRejection::IncompleteComposition(format!(
                        "XUI19 category {} received a provider without an effective binding",
                        category.category_id
                    )))
                }
            };
            if let Some(child) = supplied {
                if child.provider_id != definition.scripted_gui.as_deref().unwrap_or_default()
                    || child.local_rect[2] <= 0
                    || child.local_rect[3] <= 0
                    || child.element.name.is_empty()
                {
                    return Err(ProductionUiRejection::IncompleteComposition(format!(
                        "XUI19 category {} provider identity or geometry rejected",
                        category.category_id
                    )));
                }
                let child_index = children.len();
                for widget in child.widgets {
                    pending_xui22_widgets.push(PendingXui22WidgetTarget {
                        category_id: category.category_id.clone(),
                        provider_id: child.provider_id.clone(),
                        definition_source_file: child.definition_source_file.clone(),
                        authored_root_source_file: child.authored_root_source_file.clone(),
                        window_name: child.window_name.clone(),
                        manager_order_index: child.manager_order_index,
                        stable_namespace: namespace.clone(),
                        widget,
                    });
                }
                child_sizes.push((child.local_rect[2], child.local_rect[3]));
                child_categories.push(category.category_id.clone());
                logical_cursor_y = logical_cursor_y
                    .checked_add(child.local_rect[3])
                    .ok_or_else(|| {
                        ProductionUiRejection::IncompleteComposition(
                            "XUI19 provider extent overflowed".into(),
                        )
                    })?;
                children.push(child.element);
                pending_provider.push((
                    child_index,
                    [child.local_rect[0], child.local_rect[1]],
                    ObserverDecisionCategoryProviderReceipt {
                        category_id: category.category_id.clone(),
                        provider_id: child.provider_id,
                        category_source_file: child.category_source_file,
                        definition_source_file: child.definition_source_file,
                        authored_root_source_file: child.authored_root_source_file,
                        manager_order_index: child.manager_order_index,
                        final_local_rect: [0, 0, child.local_rect[2], child.local_rect[3]],
                        stable_namespace: namespace.clone(),
                        interaction: child.interaction,
                    },
                ));
            } else if !category.description_text.is_empty() || !category.custom_icons.is_empty() {
                let desc_namespace = format!("xui19_category_desc_{category_ordinal}");
                let mut description = scene.category_description.clone();
                rename_xui19_clone(&mut description, &desc_namespace);
                let has_picture = definition.picture.is_some();
                let text_element = if has_picture {
                    options
                        .hidden_elements
                        .insert(xui19_cloned_name("full_text", &desc_namespace));
                    "short_text"
                } else {
                    options
                        .hidden_elements
                        .insert(xui19_cloned_name("short_text", &desc_namespace));
                    options
                        .hidden_elements
                        .insert(xui19_cloned_name("picture", &desc_namespace));
                    "full_text"
                };
                options.text_overrides.insert(
                    xui19_cloned_name(text_element, &desc_namespace),
                    category.description_text.clone(),
                );
                if has_picture {
                    let picture = category_icon_sprite(definition).ok_or_else(|| {
                        ProductionUiRejection::IncompleteComposition(format!(
                            "XUI19 category {} picture did not resolve",
                            category.category_id
                        ))
                    })?;
                    if self.registry.get(&picture).is_none() {
                        return Err(ProductionUiRejection::IncompleteComposition(format!(
                            "XUI19 category description sprite {picture} is missing"
                        )));
                    }
                    options
                        .sprite_name_overrides
                        .insert(xui19_cloned_name("picture", &desc_namespace), picture);
                }
                let custom_owner_name = xui19_cloned_name("custom_icons", &desc_namespace);
                let custom_icons_owner = find_xui19_element_mut(
                    &mut description,
                    &xui19_cloned_name("custom_icons", &desc_namespace),
                )
                .ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI19 category description lacks authored custom_icons owner".into(),
                    )
                })?;
                custom_icons_owner.children.clear();
                for icon in &category.custom_icons {
                    let authored = definition
                        .custom_icons
                        .get(icon.source_ordinal as usize)
                        .filter(|authored| {
                            authored.tag == icon.authored_tag
                                && authored.value_loc_key == icon.value_loc_key
                                && authored.desc_loc_key == icon.desc_loc_key
                        })
                        .ok_or_else(|| {
                            ProductionUiRejection::IncompleteComposition(format!(
                                "XUI19 category {} custom icon {} does not join its authored declaration",
                                category.category_id, icon.source_record_id
                            ))
                        })?;
                    let icon_namespace = format!(
                        "xui19_custom_icon_{}_{}",
                        category_ordinal, authored.source_ordinal
                    );
                    let mut item = scene.custom_icon_item.clone();
                    rename_xui19_clone(&mut item, &icon_namespace);
                    options.text_overrides.insert(
                        xui19_cloned_name("value", &icon_namespace),
                        icon.value_text.clone(),
                    );
                    let binding = topbar_binding_from_frame(frame, &icon.flag_tag);
                    let cosmetic = crate::binding::admitted_flag_cosmetic(
                        frame.country_identity(&icon.flag_tag),
                    )
                    .map_err(|detail| {
                        ProductionUiRejection::IncompleteComposition(format!(
                            "XUI19 custom icon flag for {}: {detail}",
                            icon.flag_tag
                        ))
                    })?;
                    let candidates = crate::topbar::flag_candidates_for_size(
                        binding.country_tag.as_deref(),
                        binding.country_ideology.as_deref(),
                        cosmetic,
                        binding.flag_texture.as_deref(),
                        crate::topbar::FlagSizeFolder::Small,
                    );
                    let mut decoded = None;
                    for candidate in candidates {
                        if self
                            .resolver
                            .resolve_optional_file(&candidate)
                            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?
                            .is_some()
                        {
                            decoded = Some(
                                self.resolver
                                    .decode_texture(&candidate)
                                    .map_err(|error| {
                                        ProductionUiRejection::Asset(error.to_string())
                                    })?
                                    .image,
                            );
                            break;
                        }
                    }
                    options.sprite_overrides.insert(
                        xui19_cloned_name("target_flag", &icon_namespace),
                        decoded.ok_or_else(|| {
                            ProductionUiRejection::IncompleteComposition(format!(
                                "XUI19 custom-icon flag {} is unavailable",
                                icon.flag_tag
                            ))
                        })?,
                    );
                    pending_custom_icons.push((
                        item.name.clone(),
                        icon.source_record_id.clone(),
                        category.category_id.clone(),
                        icon.authored_tag.clone(),
                        icon.tooltip_text.clone(),
                    ));
                    custom_icons_owner.children.push(item);
                }
                if category.custom_icons.is_empty() {
                    options.hidden_elements.insert(custom_owner_name);
                }
                let measured = {
                    let mut cache = self.asset_cache.lock().map_err(|_| {
                        ProductionUiRejection::Asset("production UI asset cache is poisoned".into())
                    })?;
                    Compositor::with_cache(&self.resolver, &self.registry, &mut cache)
                        .compose(&description, &options)
                };
                validate_complete_compose_report(&measured.report)
                    .map_err(ProductionUiRejection::IncompleteComposition)?;
                let root_rect = measured
                    .debug_rects
                    .iter()
                    .find(|rect| rect.name == description.name)
                    .ok_or_else(|| {
                        ProductionUiRejection::IncompleteComposition(
                            "XUI19 description root has no measured rectangle".into(),
                        )
                    })?;
                let marker = format!("__{desc_namespace}");
                let height = measured
                    .debug_rects
                    .iter()
                    .filter(|rect| rect.name.ends_with(&marker))
                    .map(|rect| rect.y.saturating_add(rect.h).saturating_sub(root_rect.y))
                    .max()
                    .unwrap_or(root_rect.h)
                    .max(root_rect.h);
                if root_rect.w <= 0 || height <= 0 {
                    return Err(ProductionUiRejection::IncompleteComposition(
                        "XUI19 description has non-positive authored extent".into(),
                    ));
                }
                child_sizes.push((root_rect.w, height));
                child_categories.push(category.category_id.clone());
                logical_cursor_y = logical_cursor_y.checked_add(height).ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI19 description extent overflowed".into(),
                    )
                })?;
                children.push(description);
            }

            for card in &category.cards {
                let effective = definitions.get(card.decision_id.as_str()).ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(format!(
                        "XUI19 exact decision {} is absent from effective files",
                        card.decision_id
                    ))
                })?;
                let packaged =
                    decision_catalog
                        .entries
                        .get(&card.decision_id)
                        .ok_or_else(|| {
                            ProductionUiRejection::IncompleteComposition(format!(
                                "XUI19 exact decision {} is absent from packaged catalog",
                                card.decision_id
                            ))
                        })?;
                if effective.category != category.category_id
                    || packaged.category_id != category.category_id
                    || packaged.template != effective.template
                    || packaged.source_file != effective.source_file
                {
                    return Err(ProductionUiRejection::IncompleteComposition(format!(
                        "XUI19 decision {} disagrees across carrier, packaged catalog, and effective files",
                        card.decision_id
                    )));
                }
                if !xui19_card_state_matches_template(
                    &effective.template,
                    card.target.is_some(),
                    card.timer.as_ref().map(|timer| timer.kind),
                ) {
                    return Err(ProductionUiRejection::IncompleteComposition(format!(
                        "XUI19 decision {} target/timer state disagrees with effective template {}",
                        card.decision_id, effective.template
                    )));
                }
                let mut element = scene
                    .card_templates
                    .get(&effective.template)
                    .cloned()
                    .ok_or_else(|| {
                        ProductionUiRejection::IncompleteComposition(format!(
                            "XUI19 authored card template {} is missing",
                            effective.template
                        ))
                    })?;
                let rect = resolve_element_rect(&element, local_parent, (0, 0), &options);
                if rect.w() <= 0 || rect.h() <= 0 {
                    return Err(ProductionUiRejection::IncompleteComposition(format!(
                        "XUI19 card template {} has non-positive authored geometry",
                        effective.template
                    )));
                }
                if effective.template != "on_map_decision_locator_item" {
                    let button = find_xui19_element(&element, "btn_select").ok_or_else(|| {
                        ProductionUiRejection::IncompleteComposition(format!(
                            "XUI19 card template {} lacks authored btn_select",
                            effective.template
                        ))
                    })?;
                    let sprite_name = xui19_primary_sprite_ref(button).ok_or_else(|| {
                        ProductionUiRejection::IncompleteComposition(format!(
                            "XUI19 card template {} btn_select has no authored sprite",
                            effective.template
                        ))
                    })?;
                    let frame_count = self
                        .registry
                        .get(sprite_name)
                        .map(|sprite| sprite.no_of_frames)
                        .filter(|count| *count > 0)
                        .ok_or_else(|| {
                            ProductionUiRejection::IncompleteComposition(format!(
                                "XUI19 card template {} btn_select sprite {} is missing or frame-empty",
                                effective.template, sprite_name
                            ))
                        })?;
                    if !xui19_interface_frame_valid(card.interface_state, frame_count) {
                        return Err(ProductionUiRejection::IncompleteComposition(format!(
                            "XUI19 decision {} interface state {} exceeds authored sprite {} frame count {}",
                            card.decision_id, card.interface_state, sprite_name, frame_count
                        )));
                    }
                }
                let namespace = format!("xui19_card_{card_ordinal}");
                card_ordinal = card_ordinal.saturating_add(1);
                rename_xui19_clone(&mut element, &namespace);
                options.text_overrides.insert(
                    xui19_cloned_name("name_text", &namespace),
                    card.name_text.clone(),
                );
                let exact_text = card
                    .timer
                    .as_ref()
                    .map(|timer| timer.formatted_text.as_str())
                    .unwrap_or(card.cost.formatted_text.as_str());
                options.text_overrides.insert(
                    xui19_cloned_name("cost_and_timer_text", &namespace),
                    exact_text.to_string(),
                );
                options.text_overrides.insert(
                    xui19_cloned_name("timer_text", &namespace),
                    exact_text.to_string(),
                );
                let icon = decision_icon_sprite(effective);
                if self.registry.get(&icon).is_none() {
                    return Err(ProductionUiRejection::IncompleteComposition(format!(
                        "XUI19 decision sprite {icon} is missing"
                    )));
                }
                let icon_element = if effective.template == "on_map_decision_locator_item" {
                    "decision_icon"
                } else {
                    "icon"
                };
                options
                    .sprite_name_overrides
                    .insert(xui19_cloned_name(icon_element, &namespace), icon);
                if effective.template != "on_map_decision_locator_item" {
                    options.frame_overrides.insert(
                        xui19_cloned_name("btn_select", &namespace),
                        card.interface_state as usize,
                    );
                }
                let good = xui19_cloned_name("btn_progress_good", &namespace);
                let bad = xui19_cloned_name("btn_progress_bad", &namespace);
                match card.timer.as_ref().and_then(|timer| {
                    timer
                        .progress_current_raw
                        .zip(timer.progress_max_raw)
                        .map(|(current, maximum)| (current as f32 / maximum as f32).clamp(0.0, 1.0))
                }) {
                    Some(progress) if packaged.is_good != Some(false) => {
                        options.progress_overrides.insert(good, progress);
                        options.hidden_elements.insert(bad);
                    }
                    Some(progress) => {
                        options.hidden_elements.insert(good);
                        options.progress_overrides.insert(bad, progress);
                    }
                    None => {
                        options.hidden_elements.insert(good);
                        options.hidden_elements.insert(bad);
                    }
                }
                match card.target.as_ref() {
                    Some(replay_core::DecisionTargetExactV1::Country { flag_tag, .. }) => {
                        let binding = topbar_binding_from_frame(frame, flag_tag);
                        let cosmetic =
                            crate::binding::admitted_flag_cosmetic(frame.country_identity(flag_tag))
                                .map_err(|detail| {
                                    ProductionUiRejection::IncompleteComposition(format!(
                                        "XUI19 decision target flag for {flag_tag}: {detail}"
                                    ))
                                })?;
                        let candidates = crate::topbar::flag_candidates_for_size(
                            binding.country_tag.as_deref(),
                            binding.country_ideology.as_deref(),
                            cosmetic,
                            binding.flag_texture.as_deref(),
                            crate::topbar::FlagSizeFolder::Small,
                        );
                        let mut decoded = None;
                        for candidate in candidates {
                            if self
                                .resolver
                                .resolve_optional_file(&candidate)
                                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?
                                .is_some()
                            {
                                decoded = Some(
                                    self.resolver
                                        .decode_texture(&candidate)
                                        .map_err(|error| {
                                            ProductionUiRejection::Asset(error.to_string())
                                        })?
                                        .image,
                                );
                                break;
                            }
                        }
                        options.sprite_overrides.insert(
                            xui19_cloned_name("target_flag", &namespace),
                            decoded.ok_or_else(|| {
                                ProductionUiRejection::IncompleteComposition(format!(
                                    "XUI19 target flag {flag_tag} is unavailable"
                                ))
                            })?,
                        );
                    }
                    _ => {
                        options
                            .hidden_elements
                            .insert(xui19_cloned_name("target_flag", &namespace));
                        options
                            .hidden_elements
                            .insert(xui19_cloned_name("target_flag_frame", &namespace));
                    }
                }
                pending_cards.push((
                    element.name.clone(),
                    card.source_record_id.clone(),
                    card.decision_id.clone(),
                    card.availability,
                    card.interface_state,
                    card.tooltip_text.clone(),
                ));
                child_sizes.push((rect.w(), rect.h()));
                child_categories.push(category.category_id.clone());
                logical_cursor_y = logical_cursor_y.checked_add(rect.h()).ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI19 card extent overflowed".into(),
                    )
                })?;
                children.push(element);
            }
        }

        let placements = grid_layout
            .production_variable_height_slot_rects(&child_sizes)
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        let provider_offsets = pending_provider
            .iter()
            .map(|(index, offset, _)| (*index, *offset))
            .collect::<BTreeMap<_, _>>();
        let content_height = placements
            .iter()
            .enumerate()
            .try_fold(0i32, |bottom, (index, placement)| {
                let offset = provider_offsets.get(&index).copied().unwrap_or([0, 0]);
                placement
                    .y()
                    .checked_sub(grid_layout.rect.y())
                    .and_then(|top| top.checked_add(offset[1]))
                    .and_then(|top| top.checked_add(placement.h()))
                    .map(|candidate| bottom.max(candidate))
                    .ok_or(())
            })
            .map_err(|_| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI19 resolved content extent overflowed".into(),
                )
            })?;
        let max_scroll_px = content_height.saturating_sub(grid_rect.h()).max(0);
        let effective_scroll_px = scroll_px.min(max_scroll_px);
        if effective_scroll_px != scroll_px && !pending_provider.is_empty() {
            return Err(ProductionUiRejection::IncompleteComposition(format!(
                "XUI19 provider request scroll {scroll_px} exceeds exact maximum {max_scroll_px}; caller must reconcile before XUI22 insertion"
            )));
        }
        for (index, (child, placement)) in children.iter().zip(&placements).enumerate() {
            let offset = provider_offsets.get(&index).copied().unwrap_or([0, 0]);
            let position = [
                placement
                    .x()
                    .saturating_sub(grid_layout.rect.x())
                    .saturating_add(offset[0]),
                placement
                    .y()
                    .saturating_sub(grid_layout.rect.y())
                    .saturating_sub(effective_scroll_px)
                    .saturating_add(offset[1]),
            ];
            if options
                .position_overrides
                .insert(child.name.clone(), position)
                .is_some()
            {
                return Err(ProductionUiRejection::IncompleteComposition(format!(
                    "XUI19 child {} collides with a provider position override",
                    child.name
                )));
            }
        }
        let mut provider_receipts = Vec::with_capacity(pending_provider.len());
        for (index, offset, mut receipt) in pending_provider {
            let placement = placements.get(index).ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI19 provider lost its grid placement".into(),
                )
            })?;
            receipt.final_local_rect = [
                placement
                    .x()
                    .saturating_sub(grid_layout.rect.x())
                    .saturating_add(offset[0]),
                placement
                    .y()
                    .saturating_sub(grid_layout.rect.y())
                    .saturating_sub(effective_scroll_px)
                    .saturating_add(offset[1]),
                placement.w(),
                placement.h(),
            ];
            provider_receipts.push(receipt);
        }
        let mut category_insertions = Vec::new();
        let mut category_extents = Vec::new();
        for (ordinal, (category_id, expanded, header_index, namespace)) in
            category_headers.into_iter().enumerate()
        {
            let placement = placements.get(header_index).ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI19 category header lost its grid placement".into(),
                )
            })?;
            let start = placement.y().saturating_sub(grid_layout.rect.y());
            let insertion = start.saturating_add(placement.h());
            let mut end = insertion;
            for index in header_index + 1..children.len() {
                if child_categories[index] != category_id {
                    break;
                }
                end = placements[index]
                    .y()
                    .saturating_sub(grid_layout.rect.y())
                    .saturating_add(placements[index].h());
            }
            let z_order = u32::try_from(ordinal).map_err(|_| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI19 category z-order overflowed".into(),
                )
            })?;
            category_insertions.push(ObserverDecisionCategoryInsertion {
                category_id: category_id.clone(),
                stable_namespace: namespace,
                expanded,
                insertion_cursor_y: insertion,
                parent_extent: [0, 0, grid_rect.w(), grid_rect.h()],
                clip_rect: grid_clip,
                scroll_transform_y: -effective_scroll_px,
                z_order,
                input_owner: format!("XUI19/{selected_country}/{category_id}"),
            });
            let header = [
                grid_rect.x(),
                grid_rect
                    .y()
                    .saturating_add(start)
                    .saturating_sub(effective_scroll_px),
                grid_rect.w(),
                placement.h(),
            ];
            let extent = [header[0], header[1], header[2], end.saturating_sub(start)];
            category_extents.push(ObserverDecisionCategoryExtent {
                category_id,
                expanded,
                header_rect: header,
                clipped_header_rect: intersect_xui19_rect(header, grid_clip)
                    .unwrap_or([header[0], header[1], 0, 0]),
                rect: extent,
                clipped_rect: intersect_xui19_rect(extent, grid_clip)
                    .unwrap_or([extent[0], extent[1], 0, 0]),
            });
        }
        let grid = find_xui19_element_mut(&mut root, DECISION_GRID).ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition(
                "XUI19 root does not own the authored decision_grid".into(),
            )
        })?;
        *grid = scene.decision_grid;
        grid.children = children;
        let composed = {
            let mut cache = self.asset_cache.lock().map_err(|_| {
                ProductionUiRejection::Asset("production UI asset cache is poisoned".into())
            })?;
            Compositor::with_cache(&self.resolver, &self.registry, &mut cache)
                .compose(&root, &options)
        };
        validate_complete_compose_report(&composed.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;
        let mut card_hit_targets = Vec::new();
        for (name, source_record_id, decision_id, availability, interface_state, tooltip_text) in
            pending_cards
        {
            let element_path = find_xui19_element_path(&root, &name).ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI19 card {source_record_id} lost its composed owner path"
                ))
            })?;
            let rect = composed
                .debug_rects
                .iter()
                .find(|rect| rect.name == name)
                .map(|rect| [rect.x, rect.y, rect.w, rect.h])
                .and_then(|rect| intersect_xui19_rect(rect, grid_clip));
            if let Some(rect) = rect {
                card_hit_targets.push(Xui19CardHitTarget {
                    source_record_id,
                    decision_id,
                    element_path,
                    rect,
                    availability,
                    interface_state,
                    tooltip_text,
                });
            }
        }
        let mut custom_icon_hit_targets = Vec::new();
        for (name, source_record_id, category_id, authored_tag, tooltip_text) in
            pending_custom_icons
        {
            let element_path = find_xui19_element_path(&root, &name).ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI19 custom icon {source_record_id} lost its composed owner path"
                ))
            })?;
            if let Some(rect) = composed
                .debug_rects
                .iter()
                .find(|rect| rect.name == name)
                .map(|rect| [rect.x, rect.y, rect.w, rect.h])
                .and_then(|rect| intersect_xui19_rect(rect, grid_clip))
            {
                custom_icon_hit_targets.push(Xui19CustomIconHitTarget {
                    source_record_id,
                    category_id,
                    authored_tag,
                    element_path,
                    rect,
                    tooltip_text,
                });
            }
        }
        let mut xui22_widget_hit_targets = Vec::new();
        for pending in pending_xui22_widgets {
            let element_path = find_xui19_element_path(&root, &pending.widget.composed_name)
                .ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(format!(
                        "XUI22 widget {:?} lost its composed owner path",
                        pending.widget.authored_name
                    ))
                })?;
            if let Some(rect) = composed
                .debug_rects
                .iter()
                .find(|rect| rect.name == pending.widget.composed_name)
                .map(|rect| [rect.x, rect.y, rect.w, rect.h])
                .and_then(|rect| intersect_xui19_rect(rect, grid_clip))
            {
                xui22_widget_hit_targets.push(Xui22WidgetHitTarget {
                    category_id: pending.category_id,
                    provider_id: pending.provider_id,
                    definition_source_file: pending.definition_source_file,
                    authored_root_source_file: pending.authored_root_source_file,
                    window_name: pending.window_name,
                    manager_order_index: pending.manager_order_index,
                    stable_namespace: pending.stable_namespace,
                    paint_ordinal: pending.widget.paint_ordinal,
                    authored_child_path: pending.widget.authored_child_path,
                    element_path,
                    authored_name: pending.widget.authored_name,
                    element_type: pending.widget.element_type,
                    rect,
                    has_authored_tooltip: pending.widget.has_authored_tooltip,
                    alternate_tooltip_selected: pending.widget.alternate_tooltip_selected,
                    visual_enabled: pending.widget.visual_enabled,
                    click_enabled: pending.widget.click_enabled,
                    right_click_enabled: pending.widget.right_click_enabled,
                    effect_names: pending.widget.effect_names,
                });
            }
        }
        let mut image = preliminary.image;
        copy_xui19_rgba_rect(&mut image, &composed.image, grid_rect)?;
        Ok(Some(ProductionXui19Output {
            image,
            report: ProductionUiReport {
                viewport: [viewport_width, viewport_height],
                pov_tag: Some(selected_country.to_string()),
                promoted_packets: PROMOTED_XUI_PACKETS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                promoted_extension_packets: PROMOTED_EXTENSION_PACKETS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                omitted_authority_blocked_packets: OMITTED_AUTHORITY_BLOCKED_PACKETS
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                omitted_pixel_canvas_packets: OMITTED_PIXEL_CANVAS_ROUTES
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                omitted_extension_packets: OMITTED_EXTENSION_PACKETS
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                drawn_elements: composed.report.drawn_elements,
                sprite_count: self.registry.len(),
            },
            country_tag: selected_country.to_string(),
            source_sequence: carrier.source_sequence,
            previous_record_sha256: carrier.previous_record_sha256.clone(),
            record_sha256: carrier.record_sha256.clone(),
            content_manifest_sha256: carrier.content_manifest_sha256.clone(),
            panel_rect,
            decision_grid_rect: grid_clip,
            max_scroll_px,
            effective_scroll_px,
            category_insertions,
            category_extents,
            provider_receipts,
            xui22_widget_hit_targets,
            card_hit_targets,
            custom_icon_hit_targets,
        }))
    }

    pub fn compose_xui20_combat_surface(
        &self,
        frame: &WorldFrame,
        viewer: &Xui20ViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<ProductionXui20Output>, ProductionUiRejection> {
        let Some(route) = admit_xui20_canvas_route(
            &self.resolver,
            frame,
            viewer,
            self.content_admission.manifest_digest_sha256(),
        )
        .map_err(|error| {
            ProductionUiRejection::IncompleteComposition(format!(
                "XUI20 passive combat admission rejected: {error}"
            ))
        })?
        else {
            return Ok(None);
        };
        let catalog =
            crate::combat_detail_canvas::load_xui20_authored_canvas_catalog(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        self.content_admission
            .require_used_content(&route.root.source_file)
            .map_err(ProductionUiRejection::Asset)?;
        for row in &route.presentation.dynamic_rows {
            let template = catalog.template(&row.template_name).ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI20 effective template {:?} disappeared",
                    row.template_name
                ))
            })?;
            self.content_admission
                .require_used_content(&template.source_file)
                .map_err(ProductionUiRejection::Asset)?;
        }
        require_xui20_active_texture_content(&self.content_admission, &route)?;
        let rendered = crate::combat_detail_canvas::render_xui20_surface(
            &self.resolver,
            &self.registry,
            &route,
            viewer,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        validate_complete_compose_report(&rendered.output.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;
        let report = ProductionUiReport {
            viewport: [viewport_width, viewport_height],
            pov_tag: route.presentation.owner_country_tag.clone(),
            promoted_packets: PROMOTED_XUI_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            promoted_extension_packets: PROMOTED_EXTENSION_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            omitted_authority_blocked_packets: OMITTED_AUTHORITY_BLOCKED_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_pixel_canvas_packets: OMITTED_PIXEL_CANVAS_ROUTES
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_extension_packets: OMITTED_EXTENSION_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            drawn_elements: rendered.output.report.drawn_elements,
            sprite_count: self.registry.len(),
        };
        Ok(Some(ProductionXui20Output {
            image: rendered.output.image,
            report,
            route,
            input_region: rendered.input_region,
            tooltip_targets: rendered.tooltip_targets,
        }))
    }

    pub fn compose_xui14_intelligence_surface(
        &self,
        frame: &WorldFrame,
        selected_country: &str,
        viewer: &crate::intelligence_presentation::IntelligenceViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<ProductionXui14Output>, ProductionUiRejection> {
        if !viewer.panel_open {
            return Ok(None);
        }
        for source in [
            crate::intelligence_presentation::AGENCY_GUI,
            crate::intelligence_presentation::OPERATIVE_GUI,
            crate::intelligence_presentation::LEDGER_GUI,
            crate::intelligence_presentation::OPERATION_GUI,
        ] {
            self.content_admission
                .require_used_content(source)
                .map_err(ProductionUiRejection::Asset)?;
        }
        let (carrier, presentation) =
            crate::intelligence_presentation::admit_intelligence_presentation(
                frame,
                selected_country,
                viewer,
                self.content_admission.manifest_digest_sha256(),
            )
            .map_err(|error| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI14 passive intelligence admission rejected: {error:?}"
                ))
            })?;
        let composed = crate::intelligence_presentation::render_intelligence(
            &self.resolver,
            &self.registry,
            presentation,
            &carrier.producer_manifest_sha256,
            viewer,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        validate_complete_compose_report(&composed.output.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;
        let report = ProductionUiReport {
            viewport: [viewport_width, viewport_height],
            pov_tag: Some(selected_country.to_string()),
            promoted_packets: PROMOTED_XUI_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            promoted_extension_packets: PROMOTED_EXTENSION_PACKETS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            omitted_authority_blocked_packets: OMITTED_AUTHORITY_BLOCKED_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_pixel_canvas_packets: OMITTED_PIXEL_CANVAS_ROUTES
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            omitted_extension_packets: OMITTED_EXTENSION_PACKETS
                .iter()
                .map(|(packet, _)| (*packet).to_string())
                .collect(),
            drawn_elements: composed.output.report.drawn_elements,
            sprite_count: self.registry.len(),
        };
        Ok(Some(ProductionXui14Output {
            image: composed.output.image,
            report,
            authored_root: composed.authored_root,
            country_tag: presentation.country_tag.clone(),
            source_record_identity_sha256: presentation.source_record_identity_sha256.clone(),
            record_sha256: carrier.record_sha256.clone(),
            input_region: composed.input_region,
            displayed_row_ids: composed.displayed_row_ids,
            tooltip_targets: composed.tooltip_targets,
        }))
    }

    pub fn xui11_idea_at(
        &self,
        viewer: &crate::politics_state::PoliticsViewerState,
        presentation: &crate::politics_state::PoliticsCountryPresentation,
        point: (i32, i32),
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<String>, ProductionUiRejection> {
        let layout = crate::politics_state::PoliticsAuthoredLayout::load(
            &self.resolver,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        layout
            .hit_test_idea(viewer, point, presentation)
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))
    }

    pub fn xui11_tooltip_payloads(
        &self,
        presentation: &crate::politics_state::PoliticsCountryPresentation,
        viewer: &crate::politics_state::PoliticsViewerState,
        owner: &TooltipOwner,
        producer_manifest_sha256: &str,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<
        Option<crate::politics_state::PoliticsTooltipPayloads>,
        ProductionUiRejection,
    > {
        let layout = crate::politics_state::PoliticsAuthoredLayout::load(
            &self.resolver,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let scene = crate::politics_state::build_politics_compose_scene(
            &layout,
            &self.registry,
            &self.localisation,
            presentation,
            None,
            viewer,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        crate::politics_state::politics_tooltip_payload_for_owner(
            &scene,
            presentation,
            producer_manifest_sha256,
            owner,
        )
        .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))
    }

    pub fn compose_xui22_scripted_gui_parent(
        &self,
        catalog: &crate::definition_admission::GuiDefinitionCatalog,
        panel: &crate::observer_panels::ScriptedGuiObserverPanel,
        parent_width: usize,
        parent_height: usize,
    ) -> std::result::Result<
        Vec<crate::scripted_gui_compositor::ComposedScriptedGuiRoot>,
        ProductionUiRejection,
    > {
        let mut cache = self.asset_cache.lock().map_err(|_| {
            ProductionUiRejection::Asset("production UI asset cache is poisoned".into())
        })?;
        crate::scripted_gui_compositor::compose_scripted_gui_panel(
            &self.resolver,
            &self.registry,
            &self.localisation,
            &self.content_admission,
            catalog,
            panel,
            parent_width,
            parent_height,
            &mut cache,
        )
        .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))
    }

    pub fn compose_xui15_trade_surface(
        &self,
        frame: &WorldFrame,
        panel: &crate::trade_compositor::StrictXui15PanelInput,
        viewer: &crate::trade_layout::CountryTradeViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<crate::compose::ComposeOutput, ProductionUiRejection> {
        panel
            .validate_for_frame_and_viewer(frame, viewer)
            .map_err(|error| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI15 strict producer seal changed before composition: {error:?}"
                ))
            })?;
        let output = crate::trade_compositor::compose_observer_trade_panel(
            &self.resolver,
            &self.registry,
            &self.localisation,
            crate::trade_compositor::AdmittedObserverTradePanel {
                frame_hour: u64::try_from(frame.hour_index()).map_err(|_| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI15 replay hour is negative".into(),
                    )
                })?,
                country_tag: panel.country_tag(),
                presentation: panel.presentation(),
                catalogue_rows: panel.catalogue_rows(),
                country_filters: panel.country_filters(),
            },
            viewer,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        validate_complete_compose_report(&output.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;
        Ok(output)
    }

    pub fn advance_xui15_trade_smooth_scroll(
        &self,
        viewer: &mut crate::trade_layout::CountryTradeViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<(), ProductionUiRejection> {
        let layout = crate::trade_layout::TradeAuthoredLayout::load_with_registry(
            &self.resolver,
            &self.registry,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let mut advanced = viewer.clone();
        layout
            .advance_smooth_scroll(&mut advanced)
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        *viewer = advanced;
        Ok(())
    }

    pub fn xui15_trade_pointer_action(
        &self,
        panel: &crate::trade_compositor::StrictXui15PanelInput,
        viewer: &mut crate::trade_layout::CountryTradeViewerState,
        point: (i32, i32),
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<crate::trade_layout::TradePointerAction, ProductionUiRejection> {
        panel.validate_for_viewer(viewer).map_err(|error| {
            ProductionUiRejection::IncompleteComposition(format!(
                "XUI15 interaction seal is stale: {error:?}"
            ))
        })?;
        let layout = crate::trade_layout::TradeAuthoredLayout::load_with_registry(
            &self.resolver,
            &self.registry,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let filter_ids = panel
            .country_filters()
            .iter()
            .map(|row| row.token_id)
            .collect::<Vec<_>>();
        let action = layout
            .hit_test(
                viewer,
                point,
                &filter_ids,
                crate::trade_state::TRADE_RESOURCE_ORDER.len(),
                panel.country_filter_count(),
                panel.catalogue_row_count(),
            )
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        layout
            .apply_pointer_action(
                viewer,
                action,
                crate::trade_state::TRADE_RESOURCE_ORDER.len(),
                panel.country_filter_count(),
                panel.catalogue_row_count(),
            )
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        Ok(action)
    }

    pub fn xui15_trade_drag_scrollbar(
        &self,
        panel: &crate::trade_compositor::StrictXui15PanelInput,
        viewer: &mut crate::trade_layout::CountryTradeViewerState,
        pointer_y: i32,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<bool, ProductionUiRejection> {
        let layout = crate::trade_layout::TradeAuthoredLayout::load_with_registry(
            &self.resolver,
            &self.registry,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        layout
            .drag_active_scrollbar(
                viewer,
                pointer_y,
                crate::trade_state::TRADE_RESOURCE_ORDER.len(),
                panel.country_filter_count(),
                panel.catalogue_row_count(),
            )
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))
    }

    pub fn xui15_trade_release_scrollbar(
        &self,
        viewer: &mut crate::trade_layout::CountryTradeViewerState,
    ) -> bool {
        viewer.active_scrollbar_drag.take().is_some()
    }

    pub fn xui15_trade_wheel_action(
        &self,
        panel: &crate::trade_compositor::StrictXui15PanelInput,
        viewer: &mut crate::trade_layout::CountryTradeViewerState,
        point: (i32, i32),
        emitted_notches: i32,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<crate::trade_layout::TradePointerAction, ProductionUiRejection> {
        panel.validate_for_viewer(viewer).map_err(|error| {
            ProductionUiRejection::IncompleteComposition(format!(
                "XUI15 interaction seal is stale: {error:?}"
            ))
        })?;
        let layout = crate::trade_layout::TradeAuthoredLayout::load_with_registry(
            &self.resolver,
            &self.registry,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let action = layout
            .wheel_action(
                viewer,
                point,
                emitted_notches,
                crate::trade_state::TRADE_RESOURCE_ORDER.len(),
                panel.country_filter_count(),
                panel.catalogue_row_count(),
            )
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        Ok(action)
    }

    pub fn xui15_trade_blocks_lower_input(
        &self,
        point: (i32, i32),
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<bool, ProductionUiRejection> {
        let layout = crate::trade_layout::TradeAuthoredLayout::load_with_registry(
            &self.resolver,
            &self.registry,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        Ok(point.0 >= layout.root_rect.x()
            && point.1 >= layout.root_rect.y()
            && point.0 < layout.root_rect.x().saturating_add(layout.root_rect.w())
            && point.1 < layout.root_rect.y().saturating_add(layout.root_rect.h()))
    }

    pub fn compose_xui13_designer_surface(
        &self,
        frame: &WorldFrame,
        selected_country: &str,
        viewer: &crate::design_catalog::ObserverDesignerViewState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<crate::designer_observer::ObserverDesignerSurface, ProductionUiRejection>
    {
        let chrome = crate::designer_observer::ObserverDesignerChromeCatalog::load(&self.resolver)
            .map_err(|error| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI13 effective authored chrome rejected: {error}"
                ))
            })?;
        let mut asset_cache = self
            .asset_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let surface = chrome
            .render(
                &self.resolver,
                &self.registry,
                &self.localisation,
                frame,
                selected_country,
                viewer,
                &mut asset_cache,
                viewport_width,
                viewport_height,
            )
            .map_err(|error| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI13 strict designer compositor rejected: {error}"
                ))
            })?;
        drop(asset_cache);
        for source in &surface.source_files {
            self.content_admission
                .require_used_content(source)
                .map_err(ProductionUiRejection::Asset)?;
        }
        crate::designer_observer::validate_complete_designer_report(&surface.output.report)
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        Ok(surface)
    }

    pub fn xui13_effective_membership(
        &self,
        frame: &WorldFrame,
        selected_country: &str,
        viewer: &crate::design_catalog::ObserverDesignerViewState,
    ) -> std::result::Result<
        crate::designer_observer::ObserverDesignerEffectiveMembership,
        ProductionUiRejection,
    > {
        let chrome = crate::designer_observer::ObserverDesignerChromeCatalog::load(&self.resolver)
            .map_err(|error| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI13 effective authored membership rejected: {error}"
                ))
            })?;
        chrome
            .effective_membership(frame, selected_country, viewer)
            .map_err(|error| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI13 effective membership rejected: {error}"
                ))
            })
    }

    pub fn admit_xui12_production_line_route(
        &self,
        frame: &WorldFrame,
        selected_country: &str,
        viewer: &crate::production_lines::Xui12ViewerState,
    ) -> std::result::Result<
        Option<crate::production_lines::AdmittedXui12ProductionLineRoute>,
        ProductionUiRejection,
    > {
        crate::production_lines::admit_xui12_production_line_route(
            &self.resolver,
            &self.registry,
            &self.localisation,
            &self.content_admission,
            frame,
            selected_country,
            viewer,
        )
        .map_err(|rejection| {
            ProductionUiRejection::IncompleteComposition(format!(
                "XUI12 production-line route rejected: {rejection:?}"
            ))
        })
    }

    pub fn admit_xui09_selected_present_route(
        &self,
        frame: &WorldFrame,
        pass: &replay_core::AuthenticatedUiPass,
    ) -> std::result::Result<Option<TechnologyCanvasRenderRoute>, ProductionUiRejection> {
        if frame.technology_canvas_exact().status != replay_core::ViewStatus::Present {
            return Ok(None);
        }
        let expected_hour = u64::try_from(frame.hour_index()).map_err(|_| {
            ProductionUiRejection::IncompleteComposition(
                "XUI09 cannot admit a negative replay hour".into(),
            )
        })?;
        admit_technology_canvas_render_route(frame.technology_canvas_exact(), pass, expected_hour)
            .map(Some)
            .map_err(|rejection| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI09 selected-Present route rejected [{}]: {}",
                    rejection.code.regime_id(),
                    rejection.reason
                ))
            })
    }

    pub fn admit_xui05_tooltip_route(
        &self,
        frame: &WorldFrame,
        pov_tag: Option<&str>,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<TooltipBinding>, ProductionUiRejection> {
        admit_xui05_tooltip_route(
            &self.content_admission,
            frame,
            pov_tag,
            viewport_width,
            viewport_height,
        )
    }

    pub fn compose_authenticated_xui05_tooltip(
        &self,
        controller: &mut ProductionXui05Controller,
        context: Option<&AuthenticatedReplayTooltipContext>,
        selected_country: Option<&str>,
        viewer_clock_micros: u64,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<ProductionXui05Disposition, ProductionUiRejection> {
        let key = self.playback_variant_key.as_ref().ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition(
                "XUI05 product composition requires a playback-admitted display profile".into(),
            )
        })?;
        if usize::try_from(key.physical_viewport.width).ok() != Some(viewport_width)
            || usize::try_from(key.physical_viewport.height).ok() != Some(viewport_height)
        {
            return Err(ProductionUiRejection::UnsupportedDisplayVariant {
                width: viewport_width,
                height: viewport_height,
            });
        }

        let selected_country = selected_country.map(str::to_owned);
        let clock_rolled_back = controller
            .last_viewer_clock_micros
            .is_some_and(|last| viewer_clock_micros < last);
        if controller.selected_country != selected_country || clock_rolled_back {
            controller.reset();
            controller.selected_country = selected_country.clone();
        }
        let delta_us = controller
            .last_viewer_clock_micros
            .map_or(0, |last| viewer_clock_micros.saturating_sub(last));
        controller.last_viewer_clock_micros = Some(viewer_clock_micros);
        if controller.lifecycle.is_none() {
            let (timing, timing_source) = load_authored_tooltip_timing(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
            self.content_admission
                .require_used_content(&timing_source)
                .map_err(ProductionUiRejection::Asset)?;
            controller.lifecycle = Some(TooltipLifecycle::new(timing));
        }

        let lifecycle = controller
            .lifecycle
            .as_mut()
            .expect("XUI05 lifecycle initialized above");
        let Some(context) = context else {
            if controller.retained.is_some() && controller.retained_authenticated_owner.is_none() {
                lifecycle.clear();
                controller.retained = None;
                controller.retained_delayed_content = false;
                return Ok(ProductionXui05Disposition::NoHoveredOwner);
            }
            lifecycle.advance(None, delta_us);
            if lifecycle.visible_owner().is_none() {
                controller.retained = None;
                controller.retained_delayed_content = false;
                return Ok(ProductionXui05Disposition::NoHoveredOwner);
            }
            let prepared = controller.retained.as_ref().ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI05 hide-delay owner has no retained authenticated composition".into(),
                )
            })?;
            let image = compose_prepared_tooltip_layer(
                &self.resolver,
                prepared,
                viewport_width,
                viewport_height,
            )
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
            return Ok(ProductionXui05Disposition::Visible {
                image,
                owner: prepared.payload().owner().clone(),
                authenticated_owner: controller.retained_authenticated_owner.clone().ok_or_else(
                    || {
                        ProductionUiRejection::IncompleteComposition(
                            "XUI05 hide-delay composition has no retained authenticated owner"
                                .into(),
                        )
                    },
                )?,
                row_count: prepared.payload().rows().len(),
                delayed_content_visible: controller.retained_delayed_content,
            });
        };

        let route = resolve_authenticated_replay_tooltip_route(&self.resolver, context, false)
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        let facts = match route {
            AuthenticatedReplayTooltipRoute::Unsupported { mechanism } => {
                lifecycle.clear();
                controller.retained = None;
                controller.retained_authenticated_owner = None;
                return Err(ProductionUiRejection::IncompleteComposition(
                    format!(
                        "XUI05_UNSUPPORTED_TOOLTIP_MECHANISM_REQUIRES_FEATURE_LOCAL_SEMANTIC_PRODUCER:{mechanism}"
                    ),
                ));
            }
            AuthenticatedReplayTooltipRoute::NoDeclaration
            | AuthenticatedReplayTooltipRoute::SelectedDeclarationAbsent => {
                lifecycle.clear();
                controller.retained = None;
                controller.retained_authenticated_owner = None;
                return Ok(ProductionXui05Disposition::HoveredWithoutAuthoredTooltip);
            }
            AuthenticatedReplayTooltipRoute::Authored(facts) => facts,
        };
        let candidate = HoverCandidate {
            owner: facts.owner().clone(),
            route_identity: facts.lifecycle_identity(),
            delayed: facts.has_delayed_branch(),
        };
        lifecycle.advance(Some(candidate), delta_us);
        if lifecycle.visible_owner() != Some(facts.owner()) {
            controller.retained = None;
            controller.retained_authenticated_owner = None;
            return Ok(ProductionXui05Disposition::DwellPending);
        }

        let delayed_content_visible = lifecycle.delayed_content_visible();
        let facts = match resolve_authenticated_replay_tooltip_route(
            &self.resolver,
            context,
            delayed_content_visible,
        )
        .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?
        {
            AuthenticatedReplayTooltipRoute::Authored(facts) => facts,
            _ => {
                return Err(ProductionUiRejection::IncompleteComposition(
                    "XUI05 authenticated route changed while composing one viewer frame".into(),
                ))
            }
        };
        let anchor = match facts.authored_anchor() {
            AuthoredTooltipAnchor::Cursor => TooltipAnchor::Cursor,
            AuthoredTooltipAnchor::Packed {
                position,
                relative_to_owner: false,
            } => TooltipAnchor::AbsolutePacked { position },
            AuthoredTooltipAnchor::Packed {
                position,
                relative_to_owner: true,
            } => {
                let rect = context.owner().screen_rect();
                TooltipAnchor::OwnerRelativePacked {
                    packed: position,
                    owner_screen_position: (rect.x(), rect.y()),
                }
            }
        };
        let (clamp, clamp_source_file) = load_authored_tooltip_clamp_offsets(&self.resolver)
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let viewport = (
            i32::try_from(viewport_width).map_err(|_| {
                ProductionUiRejection::IncompleteComposition("XUI05 viewport width overflow".into())
            })?,
            i32::try_from(viewport_height).map_err(|_| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI05 viewport height overflow".into(),
                )
            })?,
        );
        let prepared = prepare_composed_tooltip(
            &self.resolver,
            &self.registry,
            &self.localisation,
            context.tooltip_surface(),
            facts,
            ComposedTooltipPlacementInput {
                raw_cursor_px: context.cursor(),
                gui_scale_f32_bits: (key.gui_scale_f32_bits, key.gui_scale_f32_bits),
                anchor,
                viewport,
                clamp,
                clamp_source_file,
            },
        )
        .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        let prepared = admit_composed_xui05_tooltip_route(&self.content_admission, Some(prepared))?
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI05 composed route unexpectedly omitted a prepared tooltip".into(),
                )
            })?;
        let image = compose_prepared_tooltip_layer(
            &self.resolver,
            &prepared,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let output = ProductionXui05Disposition::Visible {
            image,
            owner: prepared.payload().owner().clone(),
            authenticated_owner: context.owner().clone(),
            row_count: prepared.payload().rows().len(),
            delayed_content_visible,
        };
        controller.retained = Some(prepared);
        controller.retained_authenticated_owner = Some(context.owner().clone());
        controller.retained_delayed_content = delayed_content_visible;
        Ok(output)
    }

    pub fn compose_packet_xui05_tooltip(
        &self,
        controller: &mut ProductionXui05Controller,
        shell: &crate::authored_role_index::AuthenticatedPanelSurface,
        payloads: Option<&crate::politics_state::PoliticsTooltipPayloads>,
        packet_id: &str,
        selected_country: Option<&str>,
        cursor: (i32, i32),
        viewer_clock_micros: u64,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<ProductionXui05Disposition, ProductionUiRejection> {
        let key = self.playback_variant_key.as_ref().ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition(
                "XUI05 product composition requires a playback-admitted display profile".into(),
            )
        })?;
        if usize::try_from(key.physical_viewport.width).ok() != Some(viewport_width)
            || usize::try_from(key.physical_viewport.height).ok() != Some(viewport_height)
        {
            return Err(ProductionUiRejection::UnsupportedDisplayVariant {
                width: viewport_width,
                height: viewport_height,
            });
        }
        let selected_country = selected_country.map(str::to_owned);
        let clock_rolled_back = controller
            .last_viewer_clock_micros
            .is_some_and(|last| viewer_clock_micros < last);
        if controller.selected_country != selected_country || clock_rolled_back {
            controller.reset();
            controller.selected_country = selected_country.clone();
        }
        let delta_us = controller
            .last_viewer_clock_micros
            .map_or(0, |last| viewer_clock_micros.saturating_sub(last));
        controller.last_viewer_clock_micros = Some(viewer_clock_micros);
        if controller.lifecycle.is_none() {
            let (timing, timing_source) = load_authored_tooltip_timing(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
            self.content_admission
                .require_used_content(&timing_source)
                .map_err(ProductionUiRejection::Asset)?;
            controller.lifecycle = Some(TooltipLifecycle::new(timing));
        }
        let lifecycle = controller
            .lifecycle
            .as_mut()
            .expect("XUI05 lifecycle initialized above");
        let Some(payloads) = payloads else {
            if controller.retained_authenticated_owner.is_some() {
                lifecycle.clear();
                controller.retained = None;
                controller.retained_authenticated_owner = None;
                controller.retained_delayed_content = false;
                return Ok(ProductionXui05Disposition::NoHoveredOwner);
            }
            lifecycle.advance(None, delta_us);
            if lifecycle.visible_owner().is_none() {
                controller.retained = None;
                controller.retained_delayed_content = false;
                return Ok(ProductionXui05Disposition::NoHoveredOwner);
            }
            let prepared = controller.retained.as_ref().ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI05 semantic hide-delay owner has no retained composition".into(),
                )
            })?;
            let image = compose_prepared_tooltip_layer(
                &self.resolver,
                prepared,
                viewport_width,
                viewport_height,
            )
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
            return Ok(ProductionXui05Disposition::SemanticVisible {
                image,
                owner: prepared.payload().owner().clone(),
                row_count: prepared.payload().rows().len(),
                delayed_content_visible: controller.retained_delayed_content,
            });
        };
        if packet_id != "XUI08"
            && packet_id != "XUI09"
            && packet_id != "XUI11"
            && packet_id != "XUI14"
            && packet_id != "XUI17"
            && packet_id != "XUI19"
            && packet_id != "XUI20"
            && packet_id != "XUI21"
        {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI05 semantic tooltip route has an unsupported packet identity".into(),
            ));
        }
        let candidate = HoverCandidate {
            owner: payloads.primary.owner().clone(),
            route_identity: format!(
                "{}:{:?}:{}",
                packet_id.to_ascii_lowercase(),
                payloads.primary.owner().element_path(),
                payloads.primary.rows().len()
            ),
            delayed: payloads.delayed.is_some(),
        };
        lifecycle.advance(Some(candidate), delta_us);
        if lifecycle.visible_owner() != Some(payloads.primary.owner()) {
            controller.retained = None;
            controller.retained_authenticated_owner = None;
            return Ok(ProductionXui05Disposition::DwellPending);
        }
        let delayed_content_visible = lifecycle.delayed_content_visible();
        let payload = if delayed_content_visible {
            payloads.delayed.as_ref().unwrap_or(&payloads.primary)
        } else {
            &payloads.primary
        };
        let (clamp, clamp_source_file) = load_authored_tooltip_clamp_offsets(&self.resolver)
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let prepared = prepare_composed_tooltip(
            &self.resolver,
            &self.registry,
            &self.localisation,
            shell,
            TooltipFamilyFacts::ExactSemanticPayload(payload.clone()),
            ComposedTooltipPlacementInput {
                raw_cursor_px: cursor,
                gui_scale_f32_bits: (key.gui_scale_f32_bits, key.gui_scale_f32_bits),
                anchor: TooltipAnchor::Cursor,
                viewport: (
                    i32::try_from(viewport_width).map_err(|_| {
                        ProductionUiRejection::IncompleteComposition(
                            "XUI05 viewport width overflow".into(),
                        )
                    })?,
                    i32::try_from(viewport_height).map_err(|_| {
                        ProductionUiRejection::IncompleteComposition(
                            "XUI05 viewport height overflow".into(),
                        )
                    })?,
                ),
                clamp,
                clamp_source_file,
            },
        )
        .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        let prepared = admit_composed_xui05_tooltip_route(&self.content_admission, Some(prepared))?
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI05 semantic route unexpectedly omitted its prepared tooltip".into(),
                )
            })?;
        let image = compose_prepared_tooltip_layer(
            &self.resolver,
            &prepared,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let output = ProductionXui05Disposition::SemanticVisible {
            image,
            owner: prepared.payload().owner().clone(),
            row_count: prepared.payload().rows().len(),
            delayed_content_visible,
        };
        controller.retained = Some(prepared);
        controller.retained_authenticated_owner = None;
        controller.retained_delayed_content = delayed_content_visible;
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn compose_xui19_xui05_tooltip(
        &self,
        controller: &mut ProductionXui05Controller,
        shell: &crate::authored_role_index::AuthenticatedPanelSurface,
        payloads: Option<&crate::politics_state::PoliticsTooltipPayloads>,
        selected_country: Option<&str>,
        cursor: (i32, i32),
        viewer_clock_micros: u64,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<ProductionXui05Disposition, ProductionUiRejection> {
        if payloads.is_none() {
            controller.reset();
            controller.selected_country = selected_country.map(str::to_owned);
            controller.last_viewer_clock_micros = Some(viewer_clock_micros);
            return Ok(ProductionXui05Disposition::HoveredWithoutAuthoredTooltip);
        }
        self.compose_packet_xui05_tooltip(
            controller,
            shell,
            payloads,
            "XUI19",
            selected_country,
            cursor,
            viewer_clock_micros,
            viewport_width,
            viewport_height,
        )
    }

    pub fn compose_xui20_xui05_tooltip(
        &self,
        controller: &mut ProductionXui05Controller,
        shell: &crate::authored_role_index::AuthenticatedPanelSurface,
        payload: Option<&crate::tooltip_compositor::TooltipPayload>,
        selected_country: Option<&str>,
        cursor: (i32, i32),
        viewer_clock_micros: u64,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<ProductionXui05Disposition, ProductionUiRejection> {
        let adapted = payload.map(|complete| {
            let mut primary = complete.clone();
            let delayed = (complete.rows.len() > 1).then(|| complete.clone());
            primary.rows.truncate(1);
            crate::politics_state::PoliticsTooltipPayloads { primary, delayed }
        });
        self.compose_packet_xui05_tooltip(
            controller,
            shell,
            adapted.as_ref(),
            "XUI20",
            selected_country,
            cursor,
            viewer_clock_micros,
            viewport_width,
            viewport_height,
        )
    }

    pub fn xui09_tooltip_payloads(
        &self,
        model: &crate::research_observer::ObserverResearchModel,
        target: &crate::research_compositor::ResearchNodeHitTarget,
    ) -> std::result::Result<crate::politics_state::PoliticsTooltipPayloads, ProductionUiRejection>
    {
        use crate::tooltip_compositor::{
            authored_tooltip_shell_name, AdmittedTooltipSource, RowAvailability, TooltipLineKind,
            TooltipOwner, TooltipPayload, TooltipRow,
        };
        let node = model
            .nodes
            .iter()
            .find(|node| node.id == target.technology_id)
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI09 tooltip target is absent from the admitted classifier model".into(),
                )
            })?;
        let manifest = self.content_admission.manifest_digest_sha256();
        let owner = TooltipOwner {
            source_file: crate::research_compositor::XUI09_GUI_SOURCE.into(),
            root_window: "countrytechtreeview".into(),
            element_path: target.child_path.clone(),
            engine_instance_token: format!(
                "xui09:{manifest}:{}:{}",
                model.selected_country, node.id
            ),
        };
        let payload = TooltipPayload {
            owner,
            tooltip_root_name: authored_tooltip_shell_name(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?,
            rows: vec![TooltipRow {
                source_ordinal: 0,
                line_kind: TooltipLineKind::Header,
                source_identity: format!("technology/{}/localized_name", node.id),
                evaluation_context: format!(
                    "country={}:world_hour={}",
                    model.selected_country, model.world_hour
                ),
                availability: RowAvailability::Resolved,
                text: node.display_name.clone(),
                admitted_sources: vec![AdmittedTooltipSource::PacketSemantic {
                    packet_id: "XUI09".into(),
                    producer_identity: format!(
                        "{}:{}:{}:{}",
                        model.authority, node.source_layer, node.source_file, node.id
                    ),
                    field: "effective_localized_technology_name".into(),
                    manifest_sha256: manifest.into(),
                }],
            }],
        };
        payload
            .validate()
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        Ok(crate::politics_state::PoliticsTooltipPayloads {
            primary: payload,
            delayed: None,
        })
    }

    pub fn xui19_card_tooltip_payloads(
        &self,
        country_tag: &str,
        producer_manifest_sha256: &str,
        target: &Xui19CardHitTarget,
    ) -> std::result::Result<
        Option<crate::politics_state::PoliticsTooltipPayloads>,
        ProductionUiRejection,
    > {
        self.xui19_tooltip_payloads(
            country_tag,
            producer_manifest_sha256,
            &target.source_record_id,
            &format!("decision/{}", target.decision_id),
            &target.element_path,
            &target.tooltip_text,
        )
    }

    pub fn xui19_custom_icon_tooltip_payloads(
        &self,
        country_tag: &str,
        producer_manifest_sha256: &str,
        target: &Xui19CustomIconHitTarget,
    ) -> std::result::Result<
        Option<crate::politics_state::PoliticsTooltipPayloads>,
        ProductionUiRejection,
    > {
        self.xui19_tooltip_payloads(
            country_tag,
            producer_manifest_sha256,
            &target.source_record_id,
            &format!(
                "category/{}/custom_icon/{}",
                target.category_id, target.authored_tag
            ),
            &target.element_path,
            &target.tooltip_text,
        )
    }

    pub fn xui22_widget_tooltip_payloads(
        &self,
        country_tag: &str,
        producer_manifest_sha256: &str,
        target: &Xui22WidgetHitTarget,
    ) -> std::result::Result<
        Option<crate::politics_state::PoliticsTooltipPayloads>,
        ProductionUiRejection,
    > {
        use crate::tooltip_compositor::{
            compose_payload_from_facts, discover_authored_unscoped_tooltip, AuthoredTooltipTarget,
        };
        if !target.has_authored_tooltip {
            return Ok(None);
        }
        if producer_manifest_sha256 != self.content_admission.manifest_digest_sha256()
            || country_tag.is_empty()
            || target.provider_id.is_empty()
            || target.window_name.is_empty()
            || target.element_path.is_empty()
        {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI22 tooltip target lacks its exact Presented/content identity".into(),
            ));
        }
        for source in [
            target.definition_source_file.as_str(),
            target.authored_root_source_file.as_str(),
        ] {
            self.content_admission
                .require_used_content(source)
                .map_err(ProductionUiRejection::Asset)?;
        }
        let make_target = |include_delayed| AuthoredTooltipTarget {
            gui_source_path: target.authored_root_source_file.clone(),
            runtime_root_name: target.window_name.clone(),
            child_path: target.authored_child_path.clone(),
            owner_child_path: Some(target.element_path.clone()),
            element_type: target.element_type.clone(),
            element_name: target.authored_name.clone(),
            engine_instance_token: format!(
                "xui22:{producer_manifest_sha256}:{country_tag}:{}:{}:{}:{}",
                target.category_id,
                target.provider_id,
                target.stable_namespace,
                target.paint_ordinal,
            ),
            paint_ordinal: target.paint_ordinal,
            alternate_tooltip_selected: target.alternate_tooltip_selected.clone(),
            include_delayed,
        };
        let primary_facts = discover_authored_unscoped_tooltip(&self.resolver, &make_target(false))
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI22 Presented tooltip target lost its authenticated declaration".into(),
                )
            })?;
        let has_delayed_branch = primary_facts.has_delayed_branch();
        let primary = compose_payload_from_facts(primary_facts, &self.localisation)
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        if !has_delayed_branch {
            return Ok(Some(crate::politics_state::PoliticsTooltipPayloads {
                primary,
                delayed: None,
            }));
        }
        let delayed_facts = discover_authored_unscoped_tooltip(&self.resolver, &make_target(true))
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI22 delayed tooltip target lost its authenticated declaration".into(),
                )
            })?;
        let delayed = compose_payload_from_facts(delayed_facts, &self.localisation)
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        Ok(Some(crate::politics_state::PoliticsTooltipPayloads {
            delayed: Some(delayed),
            primary,
        }))
    }

    fn xui19_tooltip_payloads(
        &self,
        country_tag: &str,
        producer_manifest_sha256: &str,
        producer_identity: &str,
        semantic_identity: &str,
        element_path: &[u32],
        tooltip_text: &str,
    ) -> std::result::Result<
        Option<crate::politics_state::PoliticsTooltipPayloads>,
        ProductionUiRejection,
    > {
        use crate::tooltip_compositor::{
            authored_tooltip_shell_name, AdmittedTooltipSource, RowAvailability, TooltipLineKind,
            TooltipOwner, TooltipPayload, TooltipRow,
        };
        if producer_manifest_sha256 != self.content_admission.manifest_digest_sha256() {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI19 tooltip producer manifest differs from admitted effective content".into(),
            ));
        }
        if tooltip_text.is_empty() {
            return Ok(None);
        }
        if country_tag.is_empty() || producer_identity.is_empty() || element_path.is_empty() {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI19 tooltip target lacks an authenticated Presented owner".into(),
            ));
        }
        let payload = TooltipPayload {
            owner: TooltipOwner {
                source_file: DECISION_VIEW_GUI.into(),
                root_window: ROOT_WINDOW.into(),
                element_path: element_path.to_vec(),
                engine_instance_token: format!(
                    "xui19:{producer_manifest_sha256}:{country_tag}:{semantic_identity}"
                ),
            },
            tooltip_root_name: authored_tooltip_shell_name(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?,
            rows: vec![TooltipRow {
                source_ordinal: 0,
                line_kind: TooltipLineKind::Description,
                source_identity: format!("{semantic_identity}/tooltip_text"),
                evaluation_context: format!("country={country_tag}"),
                availability: RowAvailability::Resolved,
                text: tooltip_text.to_string(),
                admitted_sources: vec![AdmittedTooltipSource::PacketSemantic {
                    packet_id: "XUI19".into(),
                    producer_identity: producer_identity.into(),
                    field: "final_tooltip_text".into(),
                    manifest_sha256: producer_manifest_sha256.into(),
                }],
            }],
        };
        payload
            .validate()
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        Ok(Some(crate::politics_state::PoliticsTooltipPayloads {
            primary: payload,
            delayed: None,
        }))
    }

    pub fn compose_xui16_frame(
        &self,
        frame: &WorldFrame,
        surface: &crate::occupation_binding::AdmittedOccupationSurface,
        viewer: &crate::occupation_compositor::OccupationViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<ProductionXui16Output>, ProductionUiRejection> {
        if viewer.panel == crate::occupation_compositor::OccupationPanel::Closed {
            return Ok(None);
        }
        if viewport_width != PRODUCTION_UI_WIDTH || viewport_height != PRODUCTION_UI_HEIGHT {
            return Err(ProductionUiRejection::UnsupportedDisplayVariant {
                width: viewport_width,
                height: viewport_height,
            });
        }
        for source in crate::occupation_compositor::XUI16_GUI_SOURCES {
            self.content_admission
                .require_used_content(source)
                .map_err(ProductionUiRejection::Asset)?;
        }
        let compositor =
            crate::occupation_compositor::OccupationPixelCompositor::load(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let mut asset_cache = self
            .asset_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let composed = compositor
            .compose(
                &self.resolver,
                &self.registry,
                &self.localisation,
                &mut asset_cache,
                frame,
                surface,
                viewer,
                viewport_width,
                viewport_height,
            )
            .map_err(|rejection| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI16 pixel compositor rejected: {rejection}"
                ))
            })?;
        drop(asset_cache);
        Ok(composed.map(|composed| ProductionXui16Output {
            image: composed.image,
            report: ProductionUiReport {
                viewport: [viewport_width, viewport_height],
                pov_tag: Some(viewer.selected_country.clone()),
                promoted_packets: PROMOTED_XUI_PACKETS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                promoted_extension_packets: PROMOTED_EXTENSION_PACKETS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                omitted_authority_blocked_packets: OMITTED_AUTHORITY_BLOCKED_PACKETS
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                omitted_pixel_canvas_packets: OMITTED_PIXEL_CANVAS_ROUTES
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                omitted_extension_packets: OMITTED_EXTENSION_PACKETS
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                drawn_elements: composed.report.drawn_elements,
                sprite_count: self.registry.len(),
            },
            authored_root: composed.authored_root,
            source_record_id: composed.source_record_id,
            materialized_row_ids: composed.materialized_row_ids,
            applied_scroll_row: composed.applied_scroll_row,
            input_region: composed.input_region,
        }))
    }

    pub fn compose_xui17_character_surface(
        &self,
        model: &crate::character_ui::ObserverCharacterUi,
        viewer: &crate::character_ui::CharacterViewerSelection,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<ProductionXui17Output>, ProductionUiRejection> {
        if viewer.panel == crate::character_ui::CharacterPanel::Closed {
            return Ok(None);
        }
        if viewport_width != PRODUCTION_UI_WIDTH || viewport_height != PRODUCTION_UI_HEIGHT {
            return Err(ProductionUiRejection::UnsupportedDisplayVariant {
                width: viewport_width,
                height: viewport_height,
            });
        }
        let expected_content_manifest = self.content_admission.manifest_digest_sha256();
        let carrier_content_manifest = match viewer.panel {
            crate::character_ui::CharacterPanel::TraitTree { .. } => {
                model.trait_content_manifest_sha256.as_deref()
            }
            crate::character_ui::CharacterPanel::Aces => {
                model.ace_content_manifest_sha256.as_deref()
            }
            crate::character_ui::CharacterPanel::Closed
            | crate::character_ui::CharacterPanel::ArmyRoster
            | crate::character_ui::CharacterPanel::NavyRoster => None,
        };
        if matches!(
            viewer.panel,
            crate::character_ui::CharacterPanel::TraitTree { .. }
                | crate::character_ui::CharacterPanel::Aces
        ) && carrier_content_manifest != Some(expected_content_manifest)
        {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI17 G8/G11 carrier effective-content manifest differs from the admitted renderer content"
                    .into(),
            ));
        }
        for source in [
            crate::character_ui::ARMY_LEADER_GUI,
            crate::character_ui::NAVY_LEADER_GUI,
            crate::character_ui::ACES_GUI,
            crate::character_ui::ARMY_TRAIT_TREE_GUI,
            crate::character_ui::NAVY_TRAIT_TREE_GUI,
            crate::character_ui::TRAIT_TREE_COMMON_GUI,
        ] {
            self.content_admission
                .require_used_content(source)
                .map_err(ProductionUiRejection::Asset)?;
        }
        let layouts = crate::character_ui::CharacterPanelLayouts::load(&self.resolver)
            .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        let mut asset_cache = self
            .asset_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let composed = crate::character_ui::compose_observer_character_panel(
            &self.resolver,
            &self.registry,
            &self.localisation,
            &mut asset_cache,
            &layouts,
            model,
            expected_content_manifest,
            viewer,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| {
            ProductionUiRejection::IncompleteComposition(format!(
                "XUI17 character compositor rejected: {error}"
            ))
        })?;
        drop(asset_cache);
        validate_complete_compose_report(&composed.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;
        let authored_root = composed.authored_roots.first().cloned().ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition(
                "XUI17 open compositor returned no authored root".into(),
            )
        })?;
        Ok(Some(ProductionXui17Output {
            image: composed.image,
            report: ProductionUiReport {
                viewport: [viewport_width, viewport_height],
                pov_tag: Some(model.selected_country.clone()),
                promoted_packets: PROMOTED_XUI_PACKETS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                promoted_extension_packets: PROMOTED_EXTENSION_PACKETS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                omitted_authority_blocked_packets: OMITTED_AUTHORITY_BLOCKED_PACKETS
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                omitted_pixel_canvas_packets: OMITTED_PIXEL_CANVAS_ROUTES
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                omitted_extension_packets: OMITTED_EXTENSION_PACKETS
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                drawn_elements: composed.report.drawn_elements,
                sprite_count: self.registry.len(),
            },
            authored_root,
            materialized_row_ids: composed.rendered_stable_ids.clone(),
            applied_scroll_row: composed.applied_scroll_row,
            input_region: composed.input_region,
            authored_roots: composed.authored_roots,
            rendered_stable_ids: composed.rendered_stable_ids,
            hit_targets: composed.hit_targets,
            tooltip_targets: composed.tooltip_targets,
            semantic_state: composed.semantic_state,
        }))
    }

    pub fn xui17_tooltip_payloads(
        &self,
        target: &crate::character_ui::CharacterTooltipHitTarget,
    ) -> std::result::Result<
        Option<crate::politics_state::PoliticsTooltipPayloads>,
        ProductionUiRejection,
    > {
        use crate::tooltip_compositor::{
            authored_tooltip_shell_name, AdmittedTooltipSource, RowAvailability, TooltipLineKind,
            TooltipOwner, TooltipPayload, TooltipRow,
        };
        if target.blocked_reason.is_empty()
            && target.progress.is_none()
            && target.leader_experience.is_none()
        {
            return Ok(None);
        }
        if target.content_manifest_sha256 != self.content_admission.manifest_digest_sha256() {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI17 tooltip content manifest differs from admitted effective content".into(),
            ));
        }
        if target.producer_identity.is_empty()
            || target.source_file.is_empty()
            || target.root_window.is_empty()
            || target.element_path.is_empty()
            || (target.trait_token.is_empty() && target.leader_experience.is_none())
        {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI17 tooltip target lacks an authenticated owner or semantic identity".into(),
            ));
        }
        let mut rows = Vec::new();
        if let Some(experience) = &target.leader_experience {
            let skill = target.leader_skill.ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI17 leader XP tooltip has no exact current skill".into(),
                )
            })?;
            let kind = target.leader_skill_kind.ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI17 leader XP tooltip has no exact engine skill subtype".into(),
                )
            })?;
            let effect_text = target.leader_skill_effect_text.as_deref().ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI17 leader XP tooltip lacks the engine-formatted current-skill effects"
                        .into(),
                )
            })?;
            let effect_language = target
                .leader_skill_effect_text_language
                .as_deref()
                .ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI17 leader XP tooltip lacks its effect-text language".into(),
                    )
                })?;
            validate_xui17_leader_skill_effect_language(
                effect_language,
                &self.playback_localisation_language,
            )?;
            let (heading_key, heading_text) =
                format_xui17_leader_skill_heading(&self.localisation, skill, experience)?;
            let heading_source = self
                .localisation
                .source_file_for_key(heading_key)
                .ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(format!(
                        "XUI17 {heading_key} has no sole effective source file"
                    ))
                })?;
            rows.push(TooltipRow {
                source_ordinal: 0,
                line_kind: TooltipLineKind::Description,
                source_identity: format!("d47:{}/skill-heading", target.unit_leader_id),
                evaluation_context: format!("unit_leader_id={}", target.unit_leader_id),
                availability: RowAvailability::Resolved,
                text: heading_text,
                admitted_sources: vec![
                    AdmittedTooltipSource::PacketSemantic {
                        packet_id: "XUI17".into(),
                        producer_identity: target.producer_identity.clone(),
                        field: target.progress_field.into(),
                        manifest_sha256: replay_core::XUI17_PRODUCER_CONTRACT_MANIFEST_SHA256
                            .into(),
                    },
                    AdmittedTooltipSource::EffectiveAsset {
                        role: "leader_skill_next_level_definition".into(),
                        game_relative_path: crate::character_ui::LEADER_SKILLS_FILE.into(),
                    },
                    AdmittedTooltipSource::EffectiveLocalisation {
                        key: heading_key.into(),
                        source_files: vec![heading_source.into()],
                    },
                ],
            });
            let description_key = match kind {
                crate::character_ui::LeaderSkillKind::FieldMarshal
                | crate::character_ui::LeaderSkillKind::CorpsCommander => {
                    "UNIT_LEADER_SKILL_LAND_DESC"
                }
                crate::character_ui::LeaderSkillKind::Navy => "UNIT_LEADER_SKILL_NAVY_DESC",
            };
            let description_text = self.localisation.get(description_key).ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI17 effective {description_key} localisation is unavailable"
                ))
            })?;
            let description_source = self
                .localisation
                .source_file_for_key(description_key)
                .ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(format!(
                        "XUI17 {description_key} has no sole effective source file"
                    ))
                })?;
            let description_row = || TooltipRow {
                source_ordinal: 0,
                line_kind: TooltipLineKind::Description,
                source_identity: format!("d47:{}/skill-description", target.unit_leader_id),
                evaluation_context: format!("unit_leader_id={}", target.unit_leader_id),
                availability: RowAvailability::Resolved,
                text: description_text.to_string(),
                admitted_sources: vec![AdmittedTooltipSource::EffectiveLocalisation {
                    key: description_key.into(),
                    source_files: vec![description_source.into()],
                }],
            };
            let effect_row = || TooltipRow {
                source_ordinal: 0,
                line_kind: TooltipLineKind::Description,
                source_identity: format!("d47:{}/skill-effects", target.unit_leader_id),
                evaluation_context: format!("unit_leader_id={}", target.unit_leader_id),
                availability: RowAvailability::Resolved,
                text: effect_text.to_string(),
                admitted_sources: vec![AdmittedTooltipSource::PacketSemantic {
                    packet_id: "XUI17".into(),
                    producer_identity: target.producer_identity.clone(),
                    field: "leaders[].skill_effect_text".into(),
                    manifest_sha256: replay_core::XUI17_PRODUCER_CONTRACT_MANIFEST_SHA256.into(),
                }],
            };
            for effect_first in xui17_leader_skill_body_order(kind, effect_text) {
                rows.push(if effect_first {
                    effect_row()
                } else {
                    description_row()
                });
            }
            for (ordinal, row) in rows.iter_mut().enumerate() {
                row.source_ordinal = u32::try_from(ordinal).map_err(|_| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI17 leader tooltip row ordinal overflows u32".into(),
                    )
                })?;
            }
        }
        if let Some((numerator, scale, maximum)) = target.progress {
            if scale != 100_000 {
                return Err(ProductionUiRejection::IncompleteComposition(
                    "XUI17 tooltip progress scale is not the exact a729 100000 scale".into(),
                ));
            }
            if maximum <= 0 {
                return Err(ProductionUiRejection::IncompleteComposition(
                    "XUI17 tooltip progress maximum is not a positive effective authored cost"
                        .into(),
                ));
            }
            let progress_template =
                self.localisation
                    .get("UNIT_LEADER_TRAIT_XP")
                    .ok_or_else(|| {
                        ProductionUiRejection::IncompleteComposition(
                            "XUI17 effective UNIT_LEADER_TRAIT_XP localisation is unavailable"
                                .into(),
                        )
                    })?;
            let progress_localisation_source = self
                .localisation
                .source_file_for_key("UNIT_LEADER_TRAIT_XP")
                .ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI17 UNIT_LEADER_TRAIT_XP has no sole effective source file".into(),
                    )
                })?;
            let progress_cost_source = target
                .progress_max_source_file
                .as_deref()
                .filter(|source| !source.trim().is_empty())
                .ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI17 trait progress maximum has no effective definition source".into(),
                    )
                })?;
            let progress_current = Self::format_xui17_fixed_raw(numerator, scale)?;
            let progress_text = progress_template
                .replace("$CUR|.0$", &progress_current)
                .replace("$MAX|.0$", &maximum.to_string());
            if progress_text.contains("$CUR") || progress_text.contains("$MAX") {
                return Err(ProductionUiRejection::IncompleteComposition(
                    "XUI17 effective UNIT_LEADER_TRAIT_XP localisation has unsupported CUR/MAX tokens"
                        .into(),
                ));
            }
            rows.push(TooltipRow {
                source_ordinal: 0,
                line_kind: TooltipLineKind::Description,
                source_identity: format!(
                    "g8:{}/{}/{}:progress",
                    target.unit_leader_id, target.tree_root_offset, target.trait_token
                ),
                evaluation_context: format!(
                    "definition_census_ordinal={}",
                    target.definition_census_ordinal
                ),
                availability: RowAvailability::Resolved,
                text: progress_text,
                admitted_sources: vec![
                    AdmittedTooltipSource::PacketSemantic {
                        packet_id: "XUI17".into(),
                        producer_identity: target.producer_identity.clone(),
                        field: target.progress_field.into(),
                        manifest_sha256: replay_core::XUI17_PRODUCER_CONTRACT_MANIFEST_SHA256
                            .into(),
                    },
                    AdmittedTooltipSource::EffectiveAsset {
                        role: "unit_leader_trait_progress_cost".into(),
                        game_relative_path: progress_cost_source.into(),
                    },
                    AdmittedTooltipSource::EffectiveLocalisation {
                        key: "UNIT_LEADER_TRAIT_XP".into(),
                        source_files: vec![progress_localisation_source.into()],
                    },
                ],
            });
        }
        if !target.blocked_reason.is_empty() {
            rows.push(TooltipRow {
                source_ordinal: u32::try_from(rows.len()).map_err(|_| {
                    ProductionUiRejection::IncompleteComposition(
                        "XUI17 tooltip row ordinal overflow".into(),
                    )
                })?,
                line_kind: TooltipLineKind::Description,
                source_identity: format!(
                    "g8:{}/{}/{}",
                    target.unit_leader_id, target.tree_root_offset, target.trait_token
                ),
                evaluation_context: format!(
                    "definition_census_ordinal={}",
                    target.definition_census_ordinal
                ),
                availability: RowAvailability::Resolved,
                text: target.blocked_reason.clone(),
                admitted_sources: vec![AdmittedTooltipSource::PacketSemantic {
                    packet_id: "XUI17".into(),
                    producer_identity: target.producer_identity.clone(),
                    field: "blocked_reason".into(),
                    manifest_sha256: replay_core::XUI17_PRODUCER_CONTRACT_MANIFEST_SHA256.into(),
                }],
            });
        }
        let payload = TooltipPayload {
            owner: TooltipOwner {
                source_file: target.source_file.clone(),
                root_window: target.root_window.clone(),
                element_path: target.element_path.clone(),
                engine_instance_token: format!(
                    "xui17:{}:{}:{}:{}",
                    target.producer_identity,
                    target.unit_leader_id,
                    target.tree_root_offset,
                    target.definition_census_ordinal
                ),
            },
            tooltip_root_name: authored_tooltip_shell_name(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?,
            rows,
        };
        payload
            .validate()
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        Ok(Some(crate::politics_state::PoliticsTooltipPayloads {
            primary: payload,
            delayed: None,
        }))
    }

    fn format_xui17_fixed_raw(
        raw: i64,
        scale: i64,
    ) -> std::result::Result<String, ProductionUiRejection> {
        if scale <= 0 {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI17 fixed-point scale must be positive".into(),
            ));
        }
        let sign = if raw < 0 { "-" } else { "" };
        let magnitude = i128::from(raw).abs();
        let scale = i128::from(scale);
        let whole = magnitude / scale;
        let remainder = magnitude % scale;
        if remainder == 0 {
            return Ok(format!("{sign}{whole}"));
        }
        let width = scale.to_string().len().saturating_sub(1);
        let mut fraction = format!("{remainder:0width$}");
        while fraction.ends_with('0') {
            fraction.pop();
        }
        Ok(format!("{sign}{whole}.{fraction}"))
    }

    fn admit_xui17_character_model(
        &self,
        frame: &WorldFrame,
        country_tag: &str,
        selection: &crate::character_ui::CharacterViewerSelection,
    ) -> std::result::Result<crate::character_ui::ObserverCharacterUi, String> {
        // PARTIAL: all this needs is the SELECTED country's own tag_db_id, a
        // presence claim about one country. The character UI it builds is
        // per-country and says nothing about any other.
        let d18 = frame.d18_country_roster();
        let partial = d18
            .partial()
            .map_err(|refusal| format!("D18 country identity roster is unusable: {refusal}"))?;
        let owner_tag_db_id = partial
            .resolved
            .iter()
            .find(|entry| entry.tag == country_tag)
            .map(|entry| entry.tag_db_id)
            .ok_or_else(|| {
                let unnamed = if partial.excluded.is_empty() {
                    String::new()
                } else {
                    format!(
                        " ({} emitted country key(s) carry no D18 identity this hour: {})",
                        partial.excluded.len(),
                        partial.excluded.join(",")
                    )
                };
                format!(
                    "selected country {country_tag:?} is absent from the D18 roster{unnamed}"
                )
            })?;
        let roster = frame
            .character_roster_exact(country_tag)
            .ok_or_else(|| format!("no D47 exact character roster for {country_tag}"))?;
        let portraits = self
            .character_portraits
            .as_ref()
            .map_err(|error| format!("effective portrait catalog rejected: {error}"))?;
        let traits = self
            .character_traits
            .as_ref()
            .map_err(|error| format!("effective unit-leader trait catalog rejected: {error}"))?;
        let skill_costs = self.character_skill_costs.as_ref().map_err(String::as_str);
        crate::character_ui::admit_observer_character_ui(
            country_tag,
            owner_tag_db_id,
            roster,
            frame.ace_roster_exact(country_tag),
            portraits,
            traits,
            skill_costs,
            frame.character_presentation_g8_exact(),
            &self.registry,
            selection,
        )
        .map_err(|error| error.to_string())
    }

    pub fn prepare_xui23_ability_route(
        &self,
        frame: &WorldFrame,
        viewer: &ObserverViewerUiState,
    ) -> ObserverAbilityRoute {
        let mut route = ObserverAbilityRoute {
            country_tag: viewer.selected_country.clone(),
            source_record_id: None,
            admitted: None,
            selected_context: viewer.abilities.selected_context.clone(),
            measured_empty: false,
            gaps: Vec::new(),
        };
        let Some(country_tag) = viewer.selected_country.as_deref() else {
            route.gaps.push(ObserverUiGap {
                surface: "XUI23/command_abilities",
                field: "selected_country",
                required_domain: "viewer-selected country identity",
                reason: "XUI23 carrier is present but no observer country is selected".into(),
            });
            return route;
        };
        let catalog = match &self.ability_catalog {
            Ok(catalog) => catalog,
            Err(error) => {
                route.gaps.push(ObserverUiGap {
                    surface: "XUI23/command_abilities",
                    field: "effective_definitions_gfx_localisation",
                    required_domain:
                        "admitted effective common/abilities + interface GFX + playback localisation",
                    reason: format!("XUI23 effective FILE catalog rejected: {error}"),
                });
                return route;
            }
        };
        let admitted = match crate::active_abilities::admit_observer_ability_frame(
            frame,
            self.content_admission.manifest_digest_sha256(),
            catalog,
            country_tag,
        ) {
            Ok(admitted) => admitted,
            Err(error) => {
                route.gaps.push(ObserverUiGap {
                    surface: "XUI23/command_abilities",
                    field: "authenticated_semantic_ingress",
                    required_domain:
                        "same-frame exact-a729 passive all-country carrier and receipt",
                    reason: error.0,
                });
                return route;
            }
        };
        route.source_record_id = Some(admitted.source_record_id.clone());
        if viewer.abilities.open {
            if let Err(error) = self
                .content_admission
                .require_used_content(crate::active_abilities::ABILITY_LIST_GUI)
            {
                route.gaps.push(ObserverUiGap {
                    surface: "XUI23/command_abilities",
                    field: "effective_abilitylist_root",
                    required_domain: "admitted effective interface/abilitylist.gui",
                    reason: format!("XUI23 ability-list root rejected: {error}"),
                });
            }
            match viewer.abilities.selected_context.as_ref() {
                None => route.gaps.push(ObserverUiGap {
                    surface: "XUI23/command_abilities",
                    field: "viewer_selected_context",
                    required_domain: "viewer-owned context from the complete carrier roster",
                    reason: "XUI23 is open without a viewer-selected evaluator context".into(),
                }),
                Some(context) if !admitted.snapshot.contexts.contains(context) => {
                    route.gaps.push(ObserverUiGap {
                        surface: "XUI23/command_abilities",
                        field: "viewer_selected_context",
                        required_domain: "viewer-owned context from the complete carrier roster",
                        reason: "XUI23 viewer context is stale or absent from this frame".into(),
                    });
                }
                Some(context) => {
                    route.measured_empty = !admitted.rows.iter().any(|row| {
                        row.caster == context.caster && row.group_target == context.group_target
                    });
                }
            }
        }
        route.admitted = Some(admitted);
        route
    }

    pub fn prepare_xui10_focus_route(
        &self,
        frame: &WorldFrame,
        viewer: &ObserverViewerUiState,
        content: &crate::static_focus_defs::AdmittedStaticFocusUiContent,
    ) -> ObserverFocusRoute {
        let mut route = ObserverFocusRoute {
            country_tag: viewer.selected_country.clone(),
            semantic: None,
            scene: None,
            measured_empty: false,
            gaps: Vec::new(),
        };
        if content.content_manifest_digest_sha256() != self.content_manifest_digest_sha256() {
            route.gaps.push(ObserverUiGap {
                surface: "XUI10/national_focus",
                field: "static_content_identity",
                required_domain: "one manifest-sealed focus definitions/catalog bundle",
                reason: "focus_defs_static identity differs from the active renderer manifest"
                    .into(),
            });
            return route;
        }
        let Some(country_tag) = viewer.selected_country.as_deref() else {
            route.gaps.push(ObserverUiGap {
                surface: "XUI10/national_focus",
                field: "selected_country",
                required_domain: "viewer-selected country identity",
                reason: "XUI10 has no observer country selected".into(),
            });
            return route;
        };
        let Some(focus) = frame.focus(country_tag) else {
            route.gaps.push(ObserverUiGap {
                surface: "XUI10/national_focus",
                field: "d12_focus_state",
                required_domain: "complete current-hour selected-country D12 focus state",
                reason: format!("D12 has no focus row for {country_tag}"),
            });
            return route;
        };
        let (tree_id, visible_order) = match (&focus.focus_tree, &focus.visible_order) {
            (replay_core::Leaf::Measured(tree_id), replay_core::Leaf::Measured(visible_order)) => {
                (tree_id, visible_order)
            }
            _ => {
                route.gaps.push(ObserverUiGap {
                    surface: "XUI10/national_focus",
                    field: "d12_tree_and_visible_order",
                    required_domain: "measured D12 focus tree and engine-visible order",
                    reason: "D12 focus tree or visible order is unavailable".into(),
                });
                return route;
            }
        };
        let topology = match content.topology_for_runtime_visible(tree_id, visible_order) {
            Ok(topology) => topology,
            Err(error) => {
                route.gaps.push(ObserverUiGap {
                    surface: "XUI10/national_focus",
                    field: "effective_focus_topology",
                    required_domain: "authenticated focus_defs_static effective topology",
                    reason: error.to_string(),
                });
                return route;
            }
        };
        Self::prepare_xui10_focus_route_from_admitted_topology(
            frame,
            viewer,
            focus,
            &topology,
            content.catalog(),
            self.content_manifest_digest_sha256(),
        )
    }

    fn prepare_xui10_focus_route_from_admitted_topology(
        frame: &WorldFrame,
        viewer: &ObserverViewerUiState,
        focus: &replay_core::FocusView,
        topology: &crate::national_focus::FocusTreeTopology,
        catalog: &crate::national_focus::FocusCatalog,
        content_manifest_digest_sha256: &str,
    ) -> ObserverFocusRoute {
        let semantic = match crate::national_focus::admit_observer_national_focus_presentation(
            focus, topology, catalog,
        ) {
            Ok(semantic) => semantic,
            Err(error) => {
                return ObserverFocusRoute {
                    country_tag: viewer.selected_country.clone(),
                    semantic: None,
                    scene: None,
                    measured_empty: false,
                    gaps: vec![ObserverUiGap {
                    surface: "XUI10/national_focus",
                    field: "semantic_admission",
                    required_domain: "D12 state joined to authenticated effective definitions",
                    reason: error.reason,
                    }],
                };
            }
        };
        let mut route = ObserverFocusRoute {
            country_tag: viewer.selected_country.clone(),
            measured_empty: semantic.nodes.is_empty(),
            semantic: None,
            scene: None,
            gaps: Vec::new(),
        };
        let Some(country_tag) = viewer.selected_country.as_deref() else {
            route.gaps.push(ObserverUiGap {
                surface: "XUI10/national_focus",
                field: "selected_country",
                required_domain: "viewer-selected country identity",
                reason: "XUI10 admitted semantics without an observer country".into(),
            });
            return route;
        };
        let carrier = match frame.national_focus_observer_exact().presentation.as_ref() {
            Some(carrier) => carrier,
            None => {
                route.semantic = Some(semantic);
                route.gaps.push(ObserverUiGap {
                    surface: "XUI10/national_focus",
                    field: "evaluated_all_country_focus_scene",
                    required_domain:
                        "replay-derived observer focus scene; EIR-NE-002's all-country carrier \
                         cannot be produced passively",
                    reason: format!(
                        "passive evaluated focus carrier is {:?}; the replay-side derivation \
                         (game_ui/src/focus_observer_derivation.rs::derive_observer_focus_scene) \
                         cannot produce these families for any content: {}. Of those, the engine \
                         law is already ported for: {}",
                        frame.national_focus_observer_exact_status(),
                        crate::focus_observer_derivation::statically_refused_families().join("; "),
                        crate::focus_observer_derivation::law_ported_families().join("; "),
                    ),
                });
                return route;
            }
        };
        if carrier.content_manifest_sha256 != content_manifest_digest_sha256 {
            route.semantic = Some(semantic);
            route.gaps.push(ObserverUiGap {
                surface: "XUI10/national_focus",
                field: "evaluated_all_country_focus_scene",
                required_domain: "same-content EIR-NE-002 carrier",
                reason: "passive evaluated focus carrier content identity differs from the renderer"
                    .into(),
            });
            return route;
        }
        let Some(row) = carrier.rows.iter().find(|row| row.country_tag == country_tag) else {
            route.semantic = Some(semantic);
            route.gaps.push(ObserverUiGap {
                surface: "XUI10/national_focus",
                field: "evaluated_all_country_focus_scene",
                required_domain: "complete-D18 all-country EIR-NE-002 row",
                reason: format!("passive evaluated focus carrier has no row for {country_tag}"),
            });
            return route;
        };
        let evaluated = crate::focus_observer::EvaluatedFocusScene {
            frame_hour: carrier.frame_hour,
            country_tag: row.country_tag.clone(),
            tree_id: row.tree_id.clone(),
            effective_vfs_manifest_sha256: carrier.content_manifest_sha256.clone(),
            authored_gui_sha256: carrier.authored_gui_sha256.clone(),
            producer_complete: carrier.producer_complete,
            expected_node_count: row.expected_node_count,
            expected_link_tile_count: row.expected_link_tile_count,
            progress: row.progress.as_ref().map(|progress| {
                crate::focus_observer::EvaluatedFocusProgress {
                    progress_raw: progress.progress_raw,
                    effective_required_progress_raw: progress.effective_required_progress_raw,
                    progress_scale_raw: progress.progress_scale_raw,
                    display_progress_percent: progress.display_progress_percent,
                }
            }),
            nodes: row
                .nodes
                .iter()
                .map(|node| crate::focus_observer::EvaluatedFocusNode {
                    focus_id: node.focus_id.clone(),
                    source_ordinal: node.source_ordinal,
                    canvas_position_pixels: node.canvas_position_pixels,
                    presentation_state_code: node.presentation_state_code,
                    display_progress_percent: node.display_progress_percent,
                    continuous_glow_visible: node.continuous_glow_visible,
                    highlight_glow_visible: node.highlight_glow_visible,
                    historical_marker_visible: node.historical_marker_visible,
                    joint_viewing_country_tag: node.joint_viewing_country_tag.clone(),
                    joint_viewing_flag_sprite: node.joint_viewing_flag_sprite.clone(),
                })
                .collect(),
            link_tiles: row
                .link_tiles
                .iter()
                .map(|link| crate::focus_observer::EvaluatedFocusLinkTile {
                    source_ordinal: link.source_ordinal,
                    from_focus_id: link.from_focus_id.clone(),
                    to_focus_id: link.to_focus_id.clone(),
                    mutually_exclusive: link.mutually_exclusive,
                    container_width_pixels: link.container_width_pixels,
                    canvas_position_pixels: link.canvas_position_pixels,
                    child_operations: link
                        .child_operations
                        .iter()
                        .map(|child| crate::focus_observer::EvaluatedFocusLinkChildOperation {
                            child_name: child.child_name.clone(),
                            visible: child.visible,
                            sprite_name: child.sprite_name.clone(),
                            frame_index: child.frame_index,
                            local_position_pixels: child.local_position_pixels,
                        })
                        .collect(),
                })
                .collect(),
        };
        let scene = match crate::focus_observer::prepare_observer_focus_scene(
            &semantic,
            &evaluated,
            u64::try_from(frame.hour_index()).unwrap_or(u64::MAX),
            country_tag,
            content_manifest_digest_sha256,
            &carrier.authored_gui_sha256,
        ) {
            Ok(scene) => scene,
            Err(error) => {
                route.semantic = Some(semantic);
                route.gaps.push(ObserverUiGap {
                    surface: "XUI10/national_focus",
                    field: "evaluated_all_country_focus_scene",
                    required_domain: "strict prepared EIR-NE-002 focus scene",
                    reason: error.to_string(),
                });
                return route;
            }
        };
        route.semantic = Some(semantic);
        route.scene = Some(scene);
        route
    }

    pub fn compose_xui10_focus_surface(
        &self,
        route: &ObserverFocusRoute,
        viewer: &crate::focus_observer::FocusViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<
        Option<crate::focus_observer::ComposedObserverFocusSurface>,
        ProductionUiRejection,
    > {
        execute_xui10_compositor_route(route, viewer.open, |scene| {
            let layout = crate::focus_observer::load_authored_focus_layout(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
            crate::focus_observer::render_observer_focus(
                &self.resolver,
                &self.registry,
                &layout,
                scene,
                viewer,
                &self.localisation,
                viewport_width,
                viewport_height,
            )
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))
        })
    }

    pub fn compose_xui23_ability_surface(
        &self,
        route: &ObserverAbilityRoute,
        viewer: &crate::active_abilities::ObserverAbilityViewerState,
        animation_elapsed_micros: u64,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<
        Option<crate::active_abilities::ObserverAbilitySurface>,
        ProductionUiRejection,
    > {
        if !viewer.open {
            return Ok(None);
        }
        if !route.gaps.is_empty() {
            return Err(ProductionUiRejection::IncompleteComposition(format!(
                "XUI23 semantic route is unavailable: {:?}",
                route.gaps
            )));
        }
        let admitted = route.admitted.as_ref().ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition(
                "XUI23 open route has no authenticated admitted frame".into(),
            )
        })?;
        let context = route.selected_context.as_ref().ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition(
                "XUI23 open route has no viewer-selected context".into(),
            )
        })?;
        let mut cache = self.asset_cache.lock().map_err(|_| {
            ProductionUiRejection::IncompleteComposition(
                "XUI23 shared UI asset cache is poisoned".into(),
            )
        })?;
        let composed = crate::active_abilities::compose_observer_ability_surface(
            &self.resolver,
            &self.registry,
            &mut cache,
            &admitted.country_tag,
            context,
            &admitted.rows,
            animation_elapsed_micros,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        if let Some(surface) = composed.as_ref() {
            validate_complete_compose_report(&surface.pixels.report)
                .map_err(ProductionUiRejection::IncompleteComposition)?;
        }
        Ok(composed)
    }

    fn prepare_xui21_event_route(
        &self,
        frame: &WorldFrame,
        viewer: &ObserverViewerUiState,
    ) -> ObserverEventRoute {
        let mut route = ObserverEventRoute {
            country_tag: viewer.selected_country.clone(),
            admitted_history: None,
            event_tick: None,
            visible_sequences: Vec::new(),
            focused_sequence: None,
            measured_empty: false,
            gaps: Vec::new(),
        };
        let Some(country_tag) = viewer.selected_country.as_deref() else {
            return route;
        };
        let definitions = match self.event_definitions.as_ref() {
            Ok(value) => value,
            Err(error) => {
                route.gaps.push(ObserverUiGap {
                    surface: "XUI21/events",
                    field: "effective_event_definitions",
                    required_domain: "lossless effective events/*.txt registry",
                    reason: error.clone(),
                });
                return route;
            }
        };
        let (contexts, runtime_text_unavailable) = match frame.runtime_text() {
            Ok(Leaf::Measured(catalog)) if catalog.contexts_complete => (
                catalog
                    .contexts
                    .iter()
                    .map(|(id, row)| (id.clone(), crate::text_law::RuntimeTextContext::from(row)))
                    .collect::<BTreeMap<_, _>>(),
                None,
            ),
            Ok(Leaf::Measured(_)) => (
                BTreeMap::new(),
                Some("A25 runtime-text catalog is explicitly incomplete".to_string()),
            ),
            Ok(Leaf::Absent) => (
                BTreeMap::new(),
                Some("A25 runtime-text catalog is absent".to_string()),
            ),
            Ok(Leaf::Unobtained) => (
                BTreeMap::new(),
                Some("A25 runtime-text catalog was explicitly unobtained".to_string()),
            ),
            Err(error) => (
                BTreeMap::new(),
                Some(format!("A25 runtime-text catalog rejected: {error}")),
            ),
        };
        let source_record_id = format!("world_events.fired_events:d75:hour={}", frame.hour_index());
        let admitted = match crate::event_chrome::admit_event_history(
            crate::event_chrome::EventHistoryRequest {
                source_record_id: &source_record_id,
                history: frame.fired_events(),
                definitions,
                localisation: &self.localisation,
                sprites: &self.registry,
                runtime_text: &contexts,
            },
        ) {
            Ok(value) => value,
            Err(error) => {
                route.gaps.push(ObserverUiGap {
                    surface: "XUI21/events",
                    field: "passive_all_country_event_history",
                    required_domain:
                        "complete passive UI-open-independent all-country d75 fired-event history",
                    reason: format!("d75 event history rejected: {error:?}"),
                });
                return route;
            }
        };
        let tick = match crate::event_chrome::current_event_tick(frame.hour_index(), &admitted) {
            Ok(value) => value,
            Err(error) => {
                route.gaps.push(ObserverUiGap {
                    surface: "XUI21/events",
                    field: "event_tick_domain",
                    required_domain: "one consistent d75 fire_tick-fire_hour offset",
                    reason: format!("d75 event tick domain rejected: {error:?}"),
                });
                return route;
            }
        };
        let unavailable_open = admitted
            .iter()
            .filter_map(|event| {
                event
                    .popup_suppression
                    .filter(|reason| !reason.is_measured_no_popup())
                    .filter(|_| event.lifecycle_open_at(country_tag, tick))
                    .map(|reason| format!("{}:{reason:?}", event.event_sequence))
            })
            .collect::<Vec<_>>();
        if !unavailable_open.is_empty() {
            route.gaps.push(ObserverUiGap {
                surface: "XUI21/events",
                field: "popup_materialisation",
                required_domain:
                    "complete d75 subtype/options/description/picture plus conditional A25 text authority",
                reason: match runtime_text_unavailable {
                    Some(a25) => format!(
                        "open popup rows are unavailable ({}) and {a25}",
                        unavailable_open.join(",")
                    ),
                    None => format!(
                        "open popup rows are unavailable ({})",
                        unavailable_open.join(",")
                    ),
                },
            });
            return route;
        }
        let (visible, focused) = crate::event_chrome::visible_event_popup_stack(
            &admitted,
            country_tag,
            tick,
            &viewer.events,
        );
        route.visible_sequences = visible.iter().map(|event| event.event_sequence).collect();
        route.focused_sequence = focused.map(|event| event.event_sequence);
        route.measured_empty = route.visible_sequences.is_empty();
        route.event_tick = Some(tick);
        route.admitted_history = Some(admitted);
        route
    }

    pub fn xui21_minimize_tooltip_payloads(
        &self,
        country_tag: &str,
        event_sequence: i64,
        root_source: &str,
        root_window: &str,
        minimize_child_path: &[u32],
        key: &str,
    ) -> std::result::Result<crate::politics_state::PoliticsTooltipPayloads, ProductionUiRejection>
    {
        use crate::tooltip_compositor::{
            authored_tooltip_shell_name, AdmittedTooltipSource, RowAvailability, TooltipLineKind,
            TooltipOwner, TooltipPayload, TooltipRow,
        };
        let (text, localisation_sources) =
            resolve_xui21_authored_tooltip_text(&self.localisation, key)?;
        self.content_admission
            .require_used_content(root_source)
            .map_err(ProductionUiRejection::Asset)?;
        for source in &localisation_sources {
            let AdmittedTooltipSource::EffectiveLocalisation { source_files, .. } = source else {
                return Err(ProductionUiRejection::IncompleteComposition(
                    "XUI21 tooltip resolver returned a non-localisation source".into(),
                ));
            };
            for source_file in source_files {
                self.content_admission
                    .require_used_content(source_file)
                    .map_err(ProductionUiRejection::Asset)?;
            }
        }
        let manifest = self.content_admission.manifest_digest_sha256();
        let payload = TooltipPayload {
            owner: TooltipOwner {
                source_file: root_source.into(),
                root_window: root_window.into(),
                element_path: minimize_child_path.to_vec(),
                engine_instance_token: format!(
                    "xui21:{manifest}:{country_tag}:{event_sequence}:btn_minimize"
                ),
            },
            tooltip_root_name: authored_tooltip_shell_name(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?,
            rows: vec![TooltipRow {
                source_ordinal: 0,
                line_kind: TooltipLineKind::Description,
                source_identity: format!("{root_source}:btn_minimize/pdx_tooltip:{key}"),
                evaluation_context: format!(
                    "country={country_tag}:event_sequence={event_sequence}"
                ),
                availability: RowAvailability::Resolved,
                text,
                admitted_sources: std::iter::once(AdmittedTooltipSource::EffectiveGui {
                    game_relative_path: root_source.into(),
                    element_path: "btn_minimize/pdx_tooltip".into(),
                })
                .chain(localisation_sources)
                .collect(),
            }],
        };
        payload
            .validate()
            .map_err(|error| ProductionUiRejection::IncompleteComposition(error.to_string()))?;
        Ok(crate::politics_state::PoliticsTooltipPayloads {
            primary: payload,
            delayed: None,
        })
    }

    pub fn compose_xui21_event_stack(
        &self,
        route: &ObserverEventRoute,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Vec<ProductionXui21Output>, ProductionUiRejection> {
        let Some(country_tag) = route.country_tag.as_deref() else {
            return Ok(Vec::new());
        };
        let Some(history) = route.admitted_history.as_ref() else {
            return Ok(Vec::new());
        };
        let tick = route.event_tick.ok_or_else(|| {
            ProductionUiRejection::IncompleteComposition(
                "XUI21 admitted history has no event tick".into(),
            )
        })?;
        let layouts = self.event_layout.as_ref().map_err(|error| {
            ProductionUiRejection::IncompleteComposition(format!(
                "XUI21 effective event layout unavailable: {error}"
            ))
        })?;
        let mut ordered = route.visible_sequences.clone();
        if let Some(focused) = route.focused_sequence {
            ordered.retain(|sequence| *sequence != focused);
            ordered.push(focused);
        }
        let mut cache = self
            .asset_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut outputs = Vec::with_capacity(ordered.len());
        for sequence in ordered {
            let event = history
                .iter()
                .find(|event| event.event_sequence == sequence)
                .ok_or_else(|| {
                    ProductionUiRejection::IncompleteComposition(format!(
                        "XUI21 visible sequence {sequence} is absent from admitted history"
                    ))
                })?;
            let presentation =
                crate::event_chrome::event_popup_presentation(event, layouts, country_tag, tick)
                    .map_err(|error| {
                        ProductionUiRejection::IncompleteComposition(format!(
                            "XUI21 popup {sequence} presentation rejected: {error:?}"
                        ))
                    })?;
            let composed = crate::event_chrome::compose_event_popup_surface(
                &self.resolver,
                &self.registry,
                &self.localisation,
                &mut cache,
                &presentation,
                layouts,
                viewport_width,
                viewport_height,
            )
            .map_err(|error| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI21 popup {sequence} composition rejected: {error:?}"
                ))
            })?;
            for source in &composed.source_files {
                self.content_admission
                    .require_used_content(source)
                    .map_err(ProductionUiRejection::Asset)?;
            }
            validate_complete_compose_report(&composed.output.report)
                .map_err(ProductionUiRejection::IncompleteComposition)?;
            outputs.push(ProductionXui21Output {
                image: composed.output.image,
                event_sequence: composed.event_sequence,
                root_source: composed.root_source,
                root_window: composed.root_window,
                root_region: composed.root_region,
                minimize_region: composed.minimize_region,
                minimize_child_path: composed.minimize_child_path,
                minimize_tooltip_key: composed.minimize_tooltip_key,
                option_targets: composed.option_targets,
            });
        }
        Ok(outputs)
    }

    pub fn prepare_observer_semantic_frame(
        &self,
        frame: &WorldFrame,
        decision_catalog: Option<&DecisionStaticCatalog>,
        static_focus_content: &crate::static_focus_defs::AdmittedStaticFocusUiContent,
        viewer: &ObserverViewerUiState,
        playback: &ObserverPlaybackUiProfile,
        playback_speed_step: usize,
        playback_playing: bool,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<ObserverUiSemanticFrame, ProductionUiRejection> {
        self.prepare_observer_semantic_frame_with_xui12_producer(
            frame,
            decision_catalog,
            static_focus_content,
            viewer,
            playback,
            playback_speed_step,
            playback_playing,
            viewport_width,
            viewport_height,
            Ok(None),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_observer_semantic_frame_with_xui12_producer(
        &self,
        frame: &WorldFrame,
        decision_catalog: Option<&DecisionStaticCatalog>,
        static_focus_content: &crate::static_focus_defs::AdmittedStaticFocusUiContent,
        viewer: &ObserverViewerUiState,
        playback: &ObserverPlaybackUiProfile,
        playback_speed_step: usize,
        playback_playing: bool,
        viewport_width: usize,
        viewport_height: usize,
        xui12_producer: std::result::Result<
            Option<&crate::production_lines::Xui12ProducerFramePayload>,
            crate::production_lines::Xui12CanvasLayoutRejection,
        >,
    ) -> std::result::Result<ObserverUiSemanticFrame, ProductionUiRejection> {
        self.prepare_observer_semantic_frame_with_xui12_and_xui22_producers(
            frame,
            decision_catalog,
            static_focus_content,
            viewer,
            playback,
            playback_speed_step,
            playback_playing,
            viewport_width,
            viewport_height,
            xui12_producer,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_observer_semantic_frame_with_xui12_and_xui22_producers(
        &self,
        frame: &WorldFrame,
        decision_catalog: Option<&DecisionStaticCatalog>,
        static_focus_content: &crate::static_focus_defs::AdmittedStaticFocusUiContent,
        viewer: &ObserverViewerUiState,
        playback: &ObserverPlaybackUiProfile,
        playback_speed_step: usize,
        playback_playing: bool,
        viewport_width: usize,
        viewport_height: usize,
        xui12_producer: std::result::Result<
            Option<&crate::production_lines::Xui12ProducerFramePayload>,
            crate::production_lines::Xui12CanvasLayoutRejection,
        >,
        xui22_catalog: Option<&crate::definition_admission::GuiDefinitionCatalog>,
        xui22_panel: Option<&crate::observer_panels::ScriptedGuiObserverPanel>,
    ) -> std::result::Result<ObserverUiSemanticFrame, ProductionUiRejection> {
        if viewport_width
            != usize::try_from(playback.variant_key.physical_viewport.width).unwrap_or(usize::MAX)
            || viewport_height
                != usize::try_from(playback.variant_key.physical_viewport.height)
                    .unwrap_or(usize::MAX)
        {
            return Err(ProductionUiRejection::UnsupportedDisplayVariant {
                width: viewport_width,
                height: viewport_height,
            });
        }
        let expected_key = self.playback_variant_key.as_ref().ok_or(
            ProductionUiRejection::MissingDisplayVariantCarrier(
                "playback-owned observer UI profile",
            ),
        )?;
        if expected_key != &playback.variant_key {
            return Err(ProductionUiRejection::UnsupportedExactDisplayVariant(
                playback.variant_key.clone(),
            ));
        }
        ExactKeyVariantContract::admit(production_ui_variant_key())
            .map_err(|_| {
                ProductionUiRejection::MissingDisplayVariantCarrier(
                    "invalid product exact-key contract",
                )
            })?
            .select(&playback.variant_key)
            .map_err(|_| {
                ProductionUiRejection::UnsupportedExactDisplayVariant(playback.variant_key.clone())
            })?;
        if viewer.mapmode.selected_button > 11
            || (viewer.mapmode.selected_button != 0
                && viewer.mapmode.use_four_button_layout.is_none())
        {
            return Err(ProductionUiRejection::IncompleteComposition(
                "viewer mapmode state has no admitted button/layout identity".into(),
            ));
        }
        if playback_speed_step > 4 {
            return Err(ProductionUiRejection::IncompleteComposition(
                "viewer transport speed step is outside the five authored controls".into(),
            ));
        }

        let mut gaps = Vec::new();
        let mut hud_binding = HudBinding {
            minimap: MinimapBinding::default(),
            mapmode: viewer.mapmode,
            ..HudBinding::default()
        };
        let mut theatre = None;
        if let Some(country_tag) = viewer.selected_country.as_deref() {
            let observer_topbar = topbar_binding_from_observer_frame(frame, country_tag);
            hud_binding.topbar = observer_topbar.binding;
            gaps.extend(observer_topbar.gaps.into_iter().map(|gap| ObserverUiGap {
                surface: "XUI27/topbar",
                field: gap.field,
                required_domain: gap.required_domain,
                reason: gap.reason.to_string(),
            }));
            match admit_observer_theatre_surface(frame, &self.content_admission, country_tag) {
                Ok(surface) => theatre = Some(surface),
                Err(gap) => gaps.push(gap),
            }
        }
        apply_clock(
            &mut hud_binding.topbar,
            &clock_binding_from_frame(frame, playback_speed_step, &self.localisation),
        );
        hud_binding.tooltip = None;

        let abilities = self.prepare_xui23_ability_route(frame, viewer);
        gaps.extend(abilities.gaps.iter().cloned());
        let focus = self.prepare_xui10_focus_route(frame, viewer, static_focus_content);
        gaps.extend(focus.gaps.iter().cloned());
        let events = self.prepare_xui21_event_route(frame, viewer);
        gaps.extend(events.gaps.iter().cloned());

        let trade = if viewer.trade_panel_open {
            let mut surface_gaps = Vec::new();
            let mut screen = None;
            let mut canvas = None;
            let mut measured_empty = false;
            match viewer.selected_country.as_deref() {
                None => surface_gaps.push(ObserverUiGap {
                    surface: "XUI15/trade",
                    field: "selected_country",
                    required_domain: "viewer-selected country identity",
                    reason: "the viewer opened trade without selecting a country".into(),
                }),
                Some(country_tag) => {
                    if let Err(error) = self
                        .content_admission
                        .require_used_content("interface/countrytradeview.gui")
                    {
                        surface_gaps.push(ObserverUiGap {
                            surface: "XUI15/trade",
                            field: "effective_authored_root",
                            required_domain: "admitted effective interface/countrytradeview.gui",
                            reason: format!("XUI15 effective authored root rejected: {error}"),
                        });
                    } else {
                        let admitted = admit_trade_screen(
                            frame.trade_status(),
                            country_tag,
                            frame.trade(country_tag),
                        );
                        if let AdmittedTradeScreen::Visible(presentation) = &admitted {
                            match crate::trade_compositor::xui15_producer_payload_from_frame(frame) {
                                Ok(Some(payload)) => {
                                    match crate::trade_compositor::seal_xui15_producer_panel_for_frame_and_viewer(
                                        payload,
                                        frame,
                                        &self.resolver,
                                        &self.localisation,
                                        &self.content_admission,
                                        country_tag,
                                        presentation,
                                        &viewer.trade,
                                    ) {
                                        Ok(sealed) => {
                                            measured_empty = sealed.measured_empty();
                                            canvas = Some(sealed);
                                        }
                                        Err(error) => surface_gaps.push(ObserverUiGap {
                                            surface: "XUI15/trade",
                                            field: "strict_catalogue_authority",
                                            required_domain: "same-frame authenticated all-country trade catalogue + exact viewer state",
                                            reason: format!("XUI15 strict catalogue rejected: {error:?}"),
                                        }),
                                    }
                                }
                                Ok(None) => surface_gaps.push(ObserverUiGap {
                                    surface: "XUI15/trade",
                                    field: "country_trade_catalogue_authority",
                                    required_domain: crate::trade_compositor::XUI15_PRODUCER_AUTHORITY_SCHEMA,
                                    reason: "the complete UI-open-independent country matrix and filter-token catalogue are unavailable for this frame".into(),
                                }),
                                Err(error) => surface_gaps.push(ObserverUiGap {
                                    surface: "XUI15/trade",
                                    field: "strict_catalogue_authority",
                                    required_domain: crate::trade_compositor::XUI15_PRODUCER_AUTHORITY_SCHEMA,
                                    reason: format!("XUI15 frame-resident producer carrier rejected: {error:?}"),
                                }),
                            }
                        } else if let AdmittedTradeScreen::Unavailable(reason) = &admitted {
                            surface_gaps.push(ObserverUiGap {
                                surface: "XUI15/trade",
                                field: "selected_country_trade_state",
                                required_domain: "trade",
                                reason: format!("{reason:?}"),
                            });
                        }
                        screen = Some(admitted);
                    }
                }
            }
            gaps.extend(surface_gaps.iter().cloned());
            Some(ObserverTradeSurface {
                country_tag: viewer.selected_country.clone(),
                screen,
                authored_root: "countrytradeview",
                canvas,
                measured_empty,
                gaps: surface_gaps,
            })
        } else {
            None
        };

        let decisions = if viewer.decision_panel_open {
            let country_tag = viewer.selected_country.as_deref().ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "viewer opened XUI19 without selecting a country".into(),
                )
            })?;
            if viewer.decision_scroll_px < 0 {
                return Err(ProductionUiRejection::IncompleteComposition(
                    "XUI19 viewer scroll offset is negative".into(),
                ));
            }
            self.content_admission
                .require_used_content(DECISION_VIEW_GUI)
                .map_err(ProductionUiRejection::Asset)?;
            let layout = parse_decision_layout(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
            if !layout.is_complete() || layout.recoveries != 0 {
                return Err(ProductionUiRejection::IncompleteComposition(
                    "XUI19 effective decision chrome is incomplete or required parser recovery"
                        .into(),
                ));
            }
            let authored_scene = load_decision_authored_scene(&self.resolver)
                .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
            if authored_scene.root.name != ROOT_WINDOW
                || authored_scene.card_templates.len()
                    != crate::decision_chrome::CARD_TEMPLATES.len()
            {
                return Err(ProductionUiRejection::IncompleteComposition(
                    "XUI19 authored compositor scene lost its exact root/template set".into(),
                ));
            }
            let mut surface_gaps = Vec::new();
            let admitted = frame.decision_evaluation_exact();
            let source_sequence = admitted.carrier.as_ref().map(|row| row.source_sequence);
            let record_sha256 = admitted
                .carrier
                .as_ref()
                .map(|row| row.record_sha256.clone());
            let pixel_output = match decision_catalog {
                None => {
                    surface_gaps.push(ObserverUiGap {
                        surface: "XUI19",
                        field: "authored_decision_catalog",
                        required_domain: "decisions_static_definitions",
                        reason: "the recorder-packaged effective decision catalog is unavailable"
                            .into(),
                    });
                    None
                }
                Some(catalog) => {
                    let mut xui22_category_provider =
                        |request: &ObserverDecisionCategoryComposeRequest,
                         options: &mut ComposeOptions| {
                            match (xui22_catalog, xui22_panel) {
                                (Some(gui_catalog), Some(panel)) => {
                                    crate::scripted_gui_compositor::supply_xui19_decision_category_child(
                                        &self.content_admission,
                                        gui_catalog,
                                        panel,
                                        request,
                                        options,
                                    )
                                    .map_err(|error| {
                                        ProductionUiRejection::IncompleteComposition(
                                            error.to_string(),
                                        )
                                    })
                                }
                                _ => Ok(ObserverDecisionCategoryDynamicOutcome::NotBound),
                            }
                        };
                    match self.compose_xui19_decision_surface(
                        frame,
                        country_tag,
                        catalog,
                        &viewer.collapsed_decision_categories,
                        viewer.decision_scroll_px,
                        viewport_width,
                        viewport_height,
                        &mut xui22_category_provider,
                    ) {
                        Ok(Some(output)) => Some(output),
                        Ok(None) => {
                            surface_gaps.push(ObserverUiGap {
                                surface: "XUI19",
                                field: "passive_all_country_decision_evaluation",
                                required_domain: replay_core::XUI19_SCHEMA,
                                reason: admitted.diagnostic.clone().unwrap_or_else(|| {
                                    "the strict XUI19 carrier is unavailable".into()
                                }),
                            });
                            None
                        }
                        Err(error) => {
                            surface_gaps.push(ObserverUiGap {
                                surface: "XUI19",
                                field: "exact_compositor_or_category_provider",
                                required_domain: "strict XUI19 carrier + effective FILE definitions + XUI22 provider for bound categories",
                                reason: format!("{error:?}"),
                            });
                            None
                        }
                    }
                }
            };
            gaps.extend(surface_gaps.iter().cloned());
            Some(ObserverDecisionSurface {
                country_tag: country_tag.to_string(),
                source_sequence,
                record_sha256,
                authored_root: ROOT_WINDOW,
                authored_scene_source: authored_scene.source_file,
                authored_template_count: authored_scene.card_templates.len(),
                viewer_scroll_px: viewer.decision_scroll_px,
                collapsed_categories: viewer.collapsed_decision_categories.clone(),
                pixel_output,
                gaps: surface_gaps,
            })
        } else {
            None
        };

        let production_lines = if viewer.production_lines.panel_open {
            let mut surface_gaps = Vec::new();
            let mut canvas = None;
            let route = match viewer.selected_country.as_deref() {
                None => {
                    surface_gaps.push(ObserverUiGap {
                        surface: "XUI12/production_lines",
                        field: "selected_country",
                        required_domain: "viewer-selected country identity",
                        reason: "the viewer opened production lines without selecting a country"
                            .into(),
                    });
                    None
                }
                Some(country_tag) => match xui12_producer.clone() {
                    Ok(Some(payload)) => {
                        match crate::production_lines::seal_xui12_producer_canvas_for_frame_and_viewer(
                            payload.clone(),
                            frame,
                            &self.content_admission,
                            country_tag,
                            &viewer.production_lines,
                        ) {
                            Ok(sealed) => canvas = Some(sealed),
                            Err(error) => surface_gaps.push(ObserverUiGap {
                                surface: "XUI12/production_lines",
                                field: "strict_canvas_authority",
                                required_domain: crate::production_lines::XUI12_PRODUCER_AUTHORITY_SCHEMA,
                                reason: format!("XUI12 strict canvas rejected: {error:?}"),
                            }),
                        }
                        None
                    }
                    Ok(None) => {
                        surface_gaps.push(ObserverUiGap {
                            surface: "XUI12/production_lines",
                            field: "passive_all_country_production_lines",
                            required_domain: crate::production_lines::XUI12_PRODUCER_AUTHORITY_SCHEMA,
                            reason: "the complete UI-open-independent all-country XUI12 payload and detached authority are unavailable for this frame".into(),
                        });
                        None
                    }
                    Err(error) => {
                        surface_gaps.push(ObserverUiGap {
                            surface: "XUI12/production_lines",
                            field: "strict_canvas_authority",
                            required_domain:
                                crate::production_lines::XUI12_PRODUCER_AUTHORITY_SCHEMA,
                            reason: format!(
                                "XUI12 frame-resident producer carrier rejected: {error:?}"
                            ),
                        });
                        None
                    }
                },
            };
            gaps.extend(surface_gaps.iter().cloned());
            Some(ObserverProductionLinesSurface {
                country_tag: viewer.selected_country.clone(),
                route,
                canvas,
                gaps: surface_gaps,
            })
        } else {
            None
        };

        let designer = if let Some(requested) = viewer.designer.as_ref() {
            let mut surface_gaps = Vec::new();
            let mut resolved_viewer = None;
            let mut measured_empty = false;
            match viewer.selected_country.as_deref() {
                None => surface_gaps.push(ObserverUiGap {
                    surface: "XUI13/designer",
                    field: "selected_country",
                    required_domain: "viewer-selected country identity",
                    reason: "the viewer opened the designer without selecting a country".into(),
                }),
                Some(country_tag) => {
                    match crate::design_catalog::resolve_observer_designer_view_state(
                        frame,
                        country_tag,
                        requested,
                    ) {
                        Ok((resolved, admitted)) => {
                            measured_empty = admitted.ordered().is_empty();
                            resolved_viewer = Some(resolved);
                        }
                        Err(error) => surface_gaps.push(ObserverUiGap {
                            surface: "XUI13/designer",
                            field: "designer_candidate_authority",
                            required_domain:
                                "extractor.a27.live_heap_catalog.a729.v3 complete all-country designer candidate carrier",
                            reason: format!(
                                "XUI13 strict designer authority is unavailable: {error:?}"
                            ),
                        }),
                    }
                }
            }
            gaps.extend(surface_gaps.iter().cloned());
            Some(ObserverDesignerRoute {
                country_tag: viewer.selected_country.clone(),
                viewer: resolved_viewer,
                measured_empty,
                gaps: surface_gaps,
            })
        } else {
            None
        };

        let characters = if viewer.selected_country.is_some()
            || viewer.character_selection.panel != crate::character_ui::CharacterPanel::Closed
        {
            let mut surface_gaps = Vec::new();
            let mut model = None;
            match viewer.selected_country.as_deref() {
                None => surface_gaps.push(ObserverUiGap {
                    surface: "XUI17/characters",
                    field: "selected_country",
                    required_domain: "viewer-selected country identity",
                    reason: "the viewer opened characters without selecting a country".into(),
                }),
                Some(country_tag) => match self.admit_xui17_character_model(
                    frame,
                    country_tag,
                    &viewer.character_selection,
                ) {
                    Err(error) => surface_gaps.push(ObserverUiGap {
                        surface: "XUI17/characters",
                        field: "selected_country_character_state",
                        required_domain:
                            "D18/D47/D28/D46 plus G8/G11 and effective character/trait files",
                        reason: format!("XUI17 semantic admission rejected: {error}"),
                    }),
                    Ok(admitted) => {
                        surface_gaps.extend(
                            crate::character_ui::character_panel_blockers(
                                &admitted,
                                &viewer.character_selection.panel,
                            )
                            .into_iter()
                            .map(|reason| ObserverUiGap {
                                surface: "XUI17/characters",
                                field: "active_panel_exact_inputs",
                                required_domain:
                                    "complete exact inputs for the viewer-selected XUI17 panel",
                                reason,
                            }),
                        );
                        model = Some(admitted);
                    }
                },
            }
            gaps.extend(surface_gaps.iter().cloned());
            Some(ObserverCharacterSurface {
                country_tag: viewer.selected_country.clone(),
                model,
                gaps: surface_gaps,
            })
        } else {
            None
        };

        if let Some(panel_country) = viewer
            .combat
            .panel
            .as_ref()
            .and_then(|panel| panel.country_tag())
        {
            if viewer.selected_country.as_deref() != Some(panel_country) {
                return Err(ProductionUiRejection::IncompleteComposition(format!(
                    "XUI20 panel country {panel_country} does not match the viewer-selected country {:?}",
                    viewer.selected_country
                )));
            }
        }
        let combat = admit_xui20_canvas_route(
            &self.resolver,
            frame,
            &viewer.combat,
            self.content_admission.manifest_digest_sha256(),
        )
        .map_err(ProductionUiRejection::IncompleteComposition)?
        .map(|route| {
            if !route.root.unsupported.is_empty() {
                gaps.push(ObserverUiGap {
                    surface: "XUI20",
                    field: "authored_constructs",
                    required_domain: "exact authored GUI compositor support",
                    reason: format!(
                        "authored root {} contains unsupported constructs {:?}",
                        route.root.name, route.root.unsupported
                    ),
                });
            }
            ObserverCombatSurface {
                route,
                pixel_output: None,
            }
        });

        let theatre_settings_land_id =
            reconcile_theatre_settings_land_id(viewer.theatre_settings_land_id, |object_id| {
                theatre.as_ref().is_some_and(|surface| {
                    surface
                        .section
                        .theatres_of_kind(replay_core::TheatreSelectorObjectKind::LandTheatre)
                        .any(|row| row.object_id == object_id)
                })
            });
        Ok(ObserverUiSemanticFrame {
            hud_binding,
            playback_playing,
            playback_speed_step,
            theatre,
            theatre_mode: viewer.theatre_mode,
            theatre_settings_land_id,
            trade,
            production_lines,
            designer,
            characters,
            decisions,
            focus,
            abilities,
            combat,
            events,
            gaps,
        })
    }

}

fn require_xui10_composable_scene(
    route: &ObserverFocusRoute,
    viewer_open: bool,
) -> std::result::Result<
    Option<&crate::focus_observer::PreparedObserverFocusScene>,
    ProductionUiRejection,
> {
    if !viewer_open {
        return Ok(None);
    }
    if !route.gaps.is_empty() {
        return Err(ProductionUiRejection::IncompleteComposition(format!(
            "XUI10 semantic route is unavailable: {:?}",
            route.gaps
        )));
    }
    route.scene.as_ref().map(Some).ok_or_else(|| {
        ProductionUiRejection::IncompleteComposition(
            "XUI10 open route has no authenticated evaluated scene".into(),
        )
    })
}

fn execute_xui10_compositor_route<T>(
    route: &ObserverFocusRoute,
    viewer_open: bool,
    compose: impl FnOnce(
        &crate::focus_observer::PreparedObserverFocusScene,
    ) -> std::result::Result<Option<T>, ProductionUiRejection>,
) -> std::result::Result<Option<T>, ProductionUiRejection> {
    let Some(scene) = require_xui10_composable_scene(route, viewer_open)? else {
        return Ok(None);
    };
    compose(scene)
}

fn validate_xui17_leader_skill_effect_language(
    effect_language: &str,
    playback_language: &str,
) -> std::result::Result<(), ProductionUiRejection> {
    if effect_language != playback_language {
        return Err(ProductionUiRejection::IncompleteComposition(format!(
            "XUI17 leader skill effects use language {effect_language:?}, expected {playback_language:?}"
        )));
    }
    Ok(())
}

fn xui17_leader_skill_body_order(
    kind: crate::character_ui::LeaderSkillKind,
    effect_text: &str,
) -> Vec<bool> {
    let has_effects = !effect_text.is_empty();
    match kind {
        crate::character_ui::LeaderSkillKind::FieldMarshal
        | crate::character_ui::LeaderSkillKind::CorpsCommander => {
            if has_effects {
                vec![false, true]
            } else {
                vec![false]
            }
        }
        crate::character_ui::LeaderSkillKind::Navy => {
            if has_effects {
                vec![true, false]
            } else {
                vec![false]
            }
        }
    }
}

fn format_xui17_leader_skill_heading(
    localisation: &crate::loc::Localisation,
    skill: i64,
    experience: &crate::character_ui::LeaderExperienceExact,
) -> std::result::Result<(&'static str, String), ProductionUiRejection> {
    if skill < 0 {
        return Err(ProductionUiRejection::IncompleteComposition(
            "XUI17 leader XP tooltip has a negative current skill".into(),
        ));
    }
    if experience.fixed_scale != crate::character_ui::LEADER_EXPERIENCE_SCALE {
        return Err(ProductionUiRejection::IncompleteComposition(
            "XUI17 leader XP scale is not the exact a729 100000 scale".into(),
        ));
    }
    let (key, percent) = match (experience.next_level_cost, experience.next_level_cost_raw) {
        (None, None) if experience.progress_raw == 0 => ("LEADER_SKILL_AT_CAP", None),
        (Some(cost), Some(cost_raw)) if cost > 0 => {
            let expected_cost_raw = cost.checked_mul(experience.fixed_scale).ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI17 leader XP next-level denominator overflowed".into(),
                )
            })?;
            if cost_raw != expected_cost_raw
                || experience.experience_raw < 0
                || experience.experience_raw > cost_raw
            {
                return Err(ProductionUiRejection::IncompleteComposition(
                    "XUI17 leader XP contradicts its exact effective next-level cost".into(),
                ));
            }
            let expected_progress = i64::try_from(
                i128::from(experience.experience_raw) * i128::from(experience.fixed_scale)
                    / i128::from(cost_raw),
            )
            .map_err(|_| {
                ProductionUiRejection::IncompleteComposition(
                    "XUI17 normalized leader XP overflows i64".into(),
                )
            })?;
            if experience.progress_raw != expected_progress {
                return Err(ProductionUiRejection::IncompleteComposition(
                ));
            }
            let percent = ((experience.progress_raw as f64 / experience.fixed_scale as f64) * 100.0)
                .round() as i64;
            ("LEADER_SKILL_WITH_PROGRESS", Some(percent))
        }
        _ => {
            return Err(ProductionUiRejection::IncompleteComposition(
                "XUI17 leader XP has a partial next-level/cap state".into(),
            ))
        }
    };
    let template = localisation.get(key).ok_or_else(|| {
        ProductionUiRejection::IncompleteComposition(format!(
            "XUI17 effective {key} localisation is unavailable"
        ))
    })?;
    let mut text = template.replace("$SKILL|Y$", &skill.to_string());
    if let Some(percent) = percent {
        text = text.replace("$PROGRESS|%Y0$", &format!("{percent}%"));
    }
    if text.contains("$SKILL") || text.contains("$PROGRESS") {
        return Err(ProductionUiRejection::IncompleteComposition(format!(
            "XUI17 effective {key} has unsupported SKILL/PROGRESS tokens"
        )));
    }
    Ok((key, text))
}

pub(crate) fn validate_complete_compose_report(
    report: &crate::compose::ComposeReport,
) -> std::result::Result<(), String> {
    if !report.missing_sprites.is_empty()
        || !report.undecodable_sprites.is_empty()
        || !report.missing_fonts.is_empty()
        || !report.missing_glyphs.is_empty()
        || !report.unresolved_loc_keys.is_empty()
        || !report.unresolved_text_inputs.is_empty()
    {
        return Err(format!(
            "production UI composition is incomplete: missing_sprites={:?}; undecodable_sprites={:?}; missing_fonts={:?}; missing_glyphs={:?}; unresolved_loc={:?}; unresolved_runtime_text={:?}",
            report.missing_sprites,
            report.undecodable_sprites,
            report.missing_fonts,
            report.missing_glyphs,
            report.unresolved_loc_keys,
            report.unresolved_text_inputs,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xui17_leader_skill_heading_uses_next_level_integer_progress_and_cap_branch() {
        let mut localisation = crate::loc::Localisation::default();
        localisation.entries.insert(
            "LEADER_SKILL_WITH_PROGRESS".into(),
            "Skill: $SKILL|Y$ ($PROGRESS|%Y0$)".into(),
        );
        localisation.entries.insert(
            "LEADER_SKILL_AT_CAP".into(),
            "Skill: $SKILL|Y$ (Max)".into(),
        );
        let experience = crate::character_ui::LeaderExperienceExact {
            experience_raw: 49_500_000,
            fixed_scale: 100_000,
            next_level_cost: Some(1_000),
            next_level_cost_raw: Some(100_000_000),
            progress_raw: 49_500,
        };
        assert_eq!(
            format_xui17_leader_skill_heading(&localisation, 4, &experience).unwrap(),
            ("LEADER_SKILL_WITH_PROGRESS", "Skill: 4 (50%)".into())
        );

        let mut contradictory = experience;
        contradictory.next_level_cost_raw = Some(99_999_999);
        assert!(format_xui17_leader_skill_heading(&localisation, 4, &contradictory).is_err());

        let at_cap = crate::character_ui::LeaderExperienceExact {
            experience_raw: 0,
            fixed_scale: 100_000,
            next_level_cost: None,
            next_level_cost_raw: None,
            progress_raw: 0,
        };
        assert_eq!(
            format_xui17_leader_skill_heading(&localisation, 9, &at_cap).unwrap(),
            ("LEADER_SKILL_AT_CAP", "Skill: 9 (Max)".into())
        );
    }

    #[test]
    fn xui17_leader_skill_tooltip_body_matches_exact_army_and_navy_order() {
        use crate::character_ui::LeaderSkillKind;

        assert_eq!(
            xui17_leader_skill_body_order(LeaderSkillKind::CorpsCommander, "+5% Attack"),
            vec![false, true]
        );
        assert_eq!(
            xui17_leader_skill_body_order(LeaderSkillKind::FieldMarshal, "+5% Planning"),
            vec![false, true]
        );
        assert_eq!(
            xui17_leader_skill_body_order(LeaderSkillKind::Navy, "+5% Coordination"),
            vec![true, false]
        );
        assert_eq!(
            xui17_leader_skill_body_order(LeaderSkillKind::Navy, ""),
            vec![false]
        );
        assert!(validate_xui17_leader_skill_effect_language("l_english", "l_english").is_ok());
        assert!(validate_xui17_leader_skill_effect_language("l_german", "l_english").is_err());
    }

    #[test]
    fn xui19_shipping_route_is_strict_composed_and_xui22_extensible() {
        assert!(PROMOTED_XUI_PACKETS.contains(&"XUI19"));
        assert!(OMITTED_PIXEL_CANVAS_ROUTES.is_empty());
        let source = include_str!("production_renderer.rs").replace("\r\n", "\n");
        let route = source
            .split_once("    pub fn compose_xui19_decision_surface")
            .expect("shipping XUI19 compositor")
            .1
            .split_once("    pub fn compose_xui20_combat_surface")
            .expect("XUI19 compositor boundary")
            .0;
        for required in [
            "frame.decision_evaluation_exact()",
            "parse_decisions(&self.resolver)",
            "load_decision_authored_scene(&self.resolver)",
            "production_variable_height_slot_rects",
            "ObserverDecisionCategoryComposeRequest",
            "category_child(&request",
            "pending_xui22_widgets",
            "xui22_widget_hit_targets",
            "card.tooltip_text.clone()",
            "card.interface_state",
            "custom_icon_definition_count",
            "crate::binding::admitted_flag_cosmetic(",
        ] {
            assert!(
                route.contains(required),
                "missing XUI19 invariant {required}"
            );
        }
        assert_eq!(
            route.matches("crate::binding::admitted_flag_cosmetic(").count(),
            2,
            "both XUI19 flag sites (custom icon, decision target) must admit the cosmetic identity"
        );
        for forbidden in [
            "decision_panel_live",
            "selected_present",
            "d87",
            "fudge",
            "binding.cosmetic_tag",
        ] {
            assert!(
                !route.to_ascii_lowercase().contains(forbidden),
                "XUI19 shipping compositor contains forbidden route {forbidden}"
            );
        }
    }

    #[test]
    fn xui19_rejects_contradictory_template_and_interface_state() {
        use replay_core::DecisionTimerKindExact::{OrdinaryActive, TargetedTimed};

        assert!(xui19_card_state_matches_template(
            "decision_item",
            false,
            None
        ));
        assert!(xui19_card_state_matches_template(
            "timed_decision_item",
            false,
            Some(OrdinaryActive),
        ));
        assert!(xui19_card_state_matches_template(
            "targeted_decision_item",
            true,
            Some(TargetedTimed),
        ));
        assert!(!xui19_card_state_matches_template(
            "targeted_decision_item",
            false,
            None,
        ));
        assert!(!xui19_card_state_matches_template(
            "decision_item",
            true,
            None,
        ));
        assert!(!xui19_card_state_matches_template(
            "decision_item",
            false,
            Some(TargetedTimed),
        ));
        assert!(!xui19_card_state_matches_template(
            "timed_decision_item",
            false,
            None,
        ));
        assert!(xui19_interface_frame_valid(0, 1));
        assert!(xui19_interface_frame_valid(3, 4));
        assert!(!xui19_interface_frame_valid(4, 4));
        assert!(!xui19_interface_frame_valid(0, 0));
    }

    fn content_admission(paths: &[&str]) -> ValidatedContentAdmission {
        use replay_core::{
            ContentLayerKind, ContentManifest, LayerStorageKind, PlaybackContentInventory,
            PlaybackContentLayer, RecordedContentLayer, RequiredContent, RequiredContentKind,
            ResolvedPlaybackContent, SourceGameIdentity, SourceProfileIdentity, UsedContentKey,
        };
        let source_game = SourceGameIdentity {
            version: "1.19.2".into(),
            build: "a729".into(),
            executable_sha256: "8".repeat(64),
            build_profile: "release".into(),
        };
        let mut source_profile = SourceProfileIdentity {
            profile_id: "xui20-test-profile".into(),
            playset_id: "xui20-test-playset".into(),
            selection_sha256: "0".repeat(64),
        };
        let vfs = "e".repeat(64);
        let ordered_layer = RecordedContentLayer {
            ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            display_name: "Base".into(),
            layer_id: "base".into(),
            descriptor_sha256: None,
            source_container_sha256: None,
            storage_kind: LayerStorageKind::Directory,
            replace_paths: Vec::new(),
            required_for_playback: true,
        };
        let required_content = paths
            .iter()
            .map(|path| RequiredContent {
                content_key: (*path).into(),
                kind: RequiredContentKind::Texture,
                observation_count: 1,
                consumers: vec!["xui20/test/direct-texture".into()],
                content: UsedContentKey {
                    effective_vfs_manifest_sha256: vfs.clone(),
                    normalized_game_relative_path: (*path).into(),
                    winning_layer_ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    source_sha256: "a".repeat(64),
                    payload_sha256: "a".repeat(64),
                    payload_bytes: 4,
                },
            })
            .collect::<Vec<_>>();
        let mut manifest = ContentManifest {
            schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
            product_modset_sha256: "9".repeat(64),
            source_game: source_game.clone(),
            source_profile: source_profile.clone(),
            effective_vfs_manifest_sha256: vfs,
            ordered_layers: vec![ordered_layer.clone()],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: std::collections::BTreeMap::new(),
            recorder_local_settings: BTreeMap::new(),
            required_content: required_content.clone(),
            manifest_digest_sha256: None,
        };
        source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest.source_profile = source_profile.clone();
        manifest = manifest.with_computed_digest().unwrap();
        let inventory = PlaybackContentInventory {
            source_game,
            source_profile,
            layers: vec![PlaybackContentLayer {
                recorded_ordinal: 0,
                layer_kind: ordered_layer.layer_kind,
                layer_id: ordered_layer.layer_id,
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
            }],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            settings: std::collections::BTreeMap::new(),
            resolved_content: required_content
                .into_iter()
                .map(|row| ResolvedPlaybackContent {
                    normalized_game_relative_path: row.content.normalized_game_relative_path,
                    winning_layer_ordinal: row.content.winning_layer_ordinal,
                    layer_kind: row.content.layer_kind,
                    layer_id: row.content.layer_id,
                    source_sha256: row.content.source_sha256,
                    payload_sha256: row.content.payload_sha256,
                    payload_bytes: row.content.payload_bytes,
                })
                .collect(),
        };
        manifest.admit_playback(&inventory).unwrap()
    }

    #[test]
    fn xui20_direct_textures_are_admitted_before_the_production_compositor() {
        let path = "gfx/flags/eng_democratic.tga";
        let paths = BTreeSet::from([path.to_string()]);
        let admitted = content_admission(&[path]);
        require_xui20_texture_paths(&admitted, &paths)
            .expect("recorded direct texture is eligible for composition");
        let unrecorded = content_admission(&[]);
        let refusal = require_xui20_texture_paths(&unrecorded, &paths).unwrap_err();
        let ProductionUiRejection::Asset(reason) = refusal else {
            panic!("expected content refusal, got {refusal:?}");
        };
        assert!(reason.contains(path), "{reason}");

        let source = include_str!("production_renderer.rs").replace("\r\n", "\n");
        let route = source
            .split_once("    pub fn compose_xui20_combat_surface(")
            .expect("shipping XUI20 route")
            .1
            .split_once("    pub fn compose_xui20_xui05_tooltip(")
            .expect("shipping XUI20 route boundary")
            .0;
        let admission = route
            .find("require_xui20_active_texture_content(&self.content_admission, &route)?;")
            .expect("active direct textures are admitted");
        let compose = route
            .find("render_xui20_surface(")
            .expect("XUI20 compositor call");
        assert!(admission < compose);
    }

    #[test]
    fn xui11_tooltip_instance_identity_is_stable_across_replay_records() {
        let first = xui11_tooltip_instance_token("GER", 17, &"ab".repeat(32));
        let next_hour = xui11_tooltip_instance_token("GER", 17, &"ab".repeat(32));
        assert_eq!(first, next_hour);
        assert_ne!(
            first,
            xui11_tooltip_instance_token("FRA", 18, &"ab".repeat(32))
        );
        assert_ne!(
            first,
            xui11_tooltip_instance_token("GER", 17, &"cd".repeat(32))
        );

        let source = include_str!("production_renderer.rs").replace("\r\n", "\n");
        let compose = source
            .split_once("    pub fn compose_xui11_politics_surface(")
            .expect("XUI11 compositor")
            .1
            .split_once("    pub fn xui11_idea_at(")
            .expect("XUI11 compositor boundary")
            .0;
        assert!(!compose.contains("frame.hour_index(),"));
        assert!(!compose.contains("presentation.source_record_identity,"));
        assert!(!compose.contains("presentation.effective_controller_identity_sha256,"));
    }

    #[test]
    fn vanished_theatre_settings_selection_closes_instead_of_aborting_the_frame() {
        let mut current_land_theatres = std::collections::BTreeSet::from([41_u32]);
        assert_eq!(
            reconcile_theatre_settings_land_id(Some(41), |object_id| {
                current_land_theatres.contains(&object_id)
            }),
            Some(41)
        );

        current_land_theatres.clear();
        assert_eq!(
            reconcile_theatre_settings_land_id(Some(41), |object_id| {
                current_land_theatres.contains(&object_id)
            }),
            None,
            "a vanished selected theatre must close its overlay without refusing unrelated UI"
        );
        assert_eq!(
            reconcile_theatre_settings_land_id(Some(0), |_| true),
            None,
            "zero is never a valid theatre identity"
        );
    }

    #[test]
    fn observer_semantic_preparation_has_no_pixel_or_legacy_geometry_dependency() {
        let source = include_str!("production_renderer.rs").replace("\r\n", "\n");
        let start = source
            .find("    pub fn prepare_observer_semantic_frame(\n")
            .expect("pixel-free observer semantic boundary");
        let end = source[start..]
            .find("\nfn require_xui10_composable_scene(")
            .map(|offset| start + offset)
            .expect("post-observer private helper boundary");
        let semantic = &source[start..end];
        for forbidden in [
            "HudAuthoredGeometryRoot",
            "HudSourceWindow",
            "ComposeOutput",
            "RgbaImage",
        ] {
            assert!(
                !semantic.contains(forbidden),
                "pixel-free observer semantics unexpectedly depend on {forbidden}"
            );
        }
    }

    #[test]
    fn shipping_observer_pipeline_uses_only_playback_local_variant_authority() {
        let source = include_str!("production_renderer.rs").replace("\r\n", "\n");
        let start = source
            .find("    pub fn prepare_observer_semantic_frame(\n")
            .expect("shipping observer semantic boundary");
        let end = source[start..]
            .find("\nfn require_xui10_composable_scene(")
            .map(|offset| start + offset)
            .expect("post-observer private helper boundary");
        let observer = source[start..end]
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        for forbidden in [
            "session_metadata",
            "production_render",
            "recorded_input_variant",
            "AuthenticatedUiPass",
            "selected_present",
            "selected-Present",
            "d72",
            "decision_panel_exact",
            "_r2_decision_panel_live",
            "frame.decisions_live()",
        ] {
            assert!(
                !observer.contains(forbidden),
                "shipping observer compositor reads recorder presentation authority: {forbidden}"
            );
        }
        assert!(observer.contains("self.playback_variant_key.as_ref()"));
        assert!(observer.contains(".select(&playback.variant_key)"));
        assert!(observer.contains("viewer.selected_country.as_deref()"));
        assert!(observer.contains("viewer.decision_panel_open"));
        assert!(observer.contains("match decision_catalog"));
        assert!(observer.contains("frame.decision_evaluation_exact()"));
        assert!(observer.contains("pixel_output: None"));
    }

    #[test]
    fn observer_theatre_semantics_use_all_country_admission_without_flat_outliner_fallbacks() {
        let source = include_str!("production_renderer.rs").replace("\r\n", "\n");
        assert!(source.contains("fn admit_observer_theatre_surface("));
        let admission = source
            .split("fn admit_observer_theatre_surface(")
            .nth(1)
            .expect("theatre admission body")
            .split("/// Shipping XUI15 route")
            .next()
            .expect("theatre admission end");
        let selector = admission
            .find(".theatre_selector()")
            .expect("typed D98 selector view");
        let authorization = admission
            .find(".authorize(&expectation")
            .expect("content/hour authorization");
        assert!(selector < authorization);
        assert!(source.contains("TheatreSelectorContentExpectation::from_content_admission("));
        assert!(source.contains("frame.d18_country_roster()"));
        assert!(
            !source
                .split("fn admit_observer_theatre_surface(")
                .nth(1)
                .expect("theatre admission body")
                .split("/// Shipping XUI15 route")
                .next()
                .expect("theatre admission end")
                .contains("outliner_binding_from_frame"),
            "observer theatre admission must not flatten D3 rows"
        );
    }

    #[test]
    fn theatre_gap_names_the_exact_combined_authority() {
        let gap = theatre_gap("content_identity", "wrong winning localization bytes");
        assert_eq!(gap.surface, "XUI07/theatre_selector");
        assert_eq!(gap.field, "content_identity");
        assert_eq!(
            gap.required_domain,
            "D98 all-country theatre-selector carrier + XUI24 content admission"
        );
        assert!(gap.reason.contains("wrong winning localization bytes"));
    }

    #[test]
    fn xui10_shipping_prepare_and_compositor_execute_complete_and_fail_closed() {
        use replay_core::{
            world::{
                NationalFocusObserverCountryExact, NationalFocusObserverExact,
                NationalFocusObserverNodeExact, NationalFocusObserverProgressExact,
            },
            RawFrame, ReplaySource, SourceError, WorldTimeline,
        };

        struct FocusSource {
            hours: [i64; 1],
            rest: serde_json::Map<String, serde_json::Value>,
        }
        impl ReplaySource for FocusSource {
            fn game_id(&self) -> &str {
                "xui10-shipping-route"
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
        let decoded = |rest| {
            WorldTimeline::open(Box::new(FocusSource {
                hours: [900],
                rest,
            }))
            .expect("open in-memory focus timeline")
            .state_at(900)
            .expect("decode focus frame")
        };

        let authored_root = std::env::temp_dir().join(format!(
            "xui10_shipping_compositor_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(authored_root.join("interface"))
            .expect("create authored XUI10 fixture root");
        std::fs::create_dir_all(authored_root.join("gfx/interface"))
            .expect("create authored XUI10 texture root");
        std::fs::write(
            authored_root.join("interface/nationalfocusview.gui"),
            br#"guiTypes = {
                containerWindowType = {
                    name = "nationalfocusview"
                    position = { x = -3 y = 78 }
                    size = { width = 100%% height = 100%% }
                    containerWindowType = {
                        name = "tree"
                        size = { width = 100%% height = 100%% }
                        containerWindowType = {
                            name = "grid_window"
                            size = { width = 100%% height = 100%% }
                            gridBoxType = { name = "grid" position = { x = 50 y = 50 } }
                        }
                    }
                }
                containerWindowType = {
                    name = "national_focus_item"
                    position = { x = 0 y = 0 }
                    size = { width = 165 height = 128 }
                    iconType = { name = "continuous_glow" spriteType = "GFX_focus_test" }
                    iconType = { name = "highlight_glow" spriteType = "GFX_focus_test" }
                    iconType = { name = "bg" spriteType = "GFX_focus_test" }
                    iconType = { name = "symbol" spriteType = "GFX_focus_test" }
                    iconType = { name = "overlay" spriteType = "GFX_focus_test" }
                    iconType = { name = "historical" spriteType = "GFX_focus_test" }
                    instantTextBoxType = { name = "name" text = "" }
                    iconType = { name = "viewing_flag" }
                    iconType = { name = "viewing_flag_border" }
                }
                containerWindowType = {
                    name = "national_focus_link"
                    size = { width = 16 height = 16 }
                    iconType = { name = "link" }
                }
                containerWindowType = {
                    name = "national_focus_exclusive_item"
                    size = { width = 1 height = 16 }
                    iconType = { name = "link1" }
                    iconType = { name = "link2" }
                    iconType = { name = "left" }
                    iconType = { name = "right" }
                    iconType = { name = "mid" }
                }
                positionType = { name = "focus_spacing" position = { x = 96 y = 130 } }
                positionType = { name = "national_focus_center" position = { x = 130 y = 32 } }
                positionType = { name = "link_spacing" position = { x = 16 y = 16 } }
                positionType = { name = "link_offsets" position = { x = 0 y = 32 } }
                positionType = { name = "link_begin" position = { x = 80 y = 64 } }
                positionType = { name = "link_end" position = { x = 80 y = 0 } }
            }"#,
        )
        .expect("write authored XUI10 fixture");
        let mut tga = vec![0_u8; 18];
        tga[2] = 2;
        tga[12..14].copy_from_slice(&2_u16.to_le_bytes());
        tga[14..16].copy_from_slice(&2_u16.to_le_bytes());
        tga[16] = 32;
        tga[17] = 0x28;
        for _ in 0..4 {
            tga.extend_from_slice(&[80, 120, 180, 255]);
        }
        std::fs::write(authored_root.join("gfx/interface/focus_test.tga"), tga)
            .expect("write authored XUI10 test texture");
        std::fs::write(
            authored_root.join("interface/focus_test.gfx"),
            br#"spriteTypes = {
                spriteType = { name = "GFX_focus_test" texturefile = "gfx/interface/focus_test.tga" }
                spriteType = { name = "GFX_focus_current" texturefile = "gfx/interface/focus_test.tga" }
                spriteType = { name = "GFX_goal_generic" texturefile = "gfx/interface/focus_test.tga" }
            }"#,
        )
        .expect("write authored XUI10 test sprites");
        let resolver = crate::assets::AssetResolver::new(vec![authored_root.clone()]);
        let layout = crate::focus_observer::load_authored_focus_layout(&resolver)
            .expect("load shipping authored XUI10 layout");

        let content_admission = content_admission(&[]);
        let manifest_sha256 = content_admission.manifest_digest_sha256().to_string();
        let mut registry = crate::gfx::SpriteRegistry::new();
        registry
            .parse_file(
                &authored_root.join("interface/focus_test.gfx"),
                "interface/focus_test.gfx",
            )
            .expect("parse authored XUI10 test sprites");
        let renderer = ProductionUiRenderer {
            resolver,
            registry,
            localisation: Localisation::default(),
            playback_localisation_language: PRODUCTION_UI_LANGUAGE.into(),
            character_portraits: Err("unused test feature".into()),
            character_traits: Err("unused test feature".into()),
            character_skill_costs: Err("unused test feature".into()),
            ability_catalog: Err("unused test feature".into()),
            power_balance_catalog: Err("unused test feature".into()),
            power_balance_decisions: Err("unused test feature".into()),
            event_definitions: Ok(BTreeMap::new()),
            event_layout: Ok(crate::gui::LayoutSet::default()),
            content_admission,
            playback_variant_key: None,
            asset_cache: Mutex::new(UiAssetCache::default()),
        };
        let authored_gui_sha256 = layout.source_payload_sha256.clone();
        let mut carrier = NationalFocusObserverExact {
            schema_version: 1,
            authority: "exact_a729_passive_all_country_national_focus_scene_v1".into(),
            executable_sha256:
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
                    .into(),
            producer_contract: "EIR-NE-002".into(),
            content_manifest_sha256: manifest_sha256.clone(),
            authored_gui_sha256: authored_gui_sha256.clone(),
            frame_hour: 900,
            capture_sequence: 7,
            passive_all_country: true,
            ui_open_independent: true,
            recorder_pov_independent: true,
            producer_complete: true,
            expected_country_count: 1,
            previous_record_sha256: None,
            record_sha256: String::new(),
            rows: vec![NationalFocusObserverCountryExact {
                source_ordinal: 0,
                country_tag: "GER".into(),
                tag_db_id: 47,
                tree_id: "german_focus".into(),
                expected_node_count: 1,
                expected_link_tile_count: 0,
                progress: Some(NationalFocusObserverProgressExact {
                    progress_raw: 5,
                    effective_required_progress_raw: 10,
                    progress_scale_raw: 1,
                    display_progress_percent: 50,
                }),
                nodes: vec![NationalFocusObserverNodeExact {
                    focus_id: "ger_focus".into(),
                    source_ordinal: 0,
                    canvas_position_pixels: [100, 200],
                    presentation_state_code: 3,
                    display_progress_percent: 50,
                    continuous_glow_visible: true,
                    highlight_glow_visible: false,
                    historical_marker_visible: false,
                    joint_viewing_country_tag: None,
                    joint_viewing_flag_sprite: None,
                }],
                link_tiles: Vec::new(),
            }],
        };
        carrier.record_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&carrier).expect("encode EIR-NE-002 carrier"),
        );
        let mut rest = serde_json::Map::new();
        rest.insert(
            "countries".into(),
            serde_json::json!({"GER": {
                "_r2_tag_db_id": 47,
                "focus": {
                    "focus_tree": "german_focus",
                    "current": "ger_focus",
                    "current_set": ["ger_focus"],
                    "current_continuous_set": [],
                    "completed_count": 0,
                    "progress": 0.5,
                    "completed": [],
                    "available": [],
                    "visible_order": ["ger_focus"],
                    "paused": false
                }
            }}),
        );
        rest.insert(
            "_r2_national_focus_observer_exact".into(),
            serde_json::to_value(&carrier).expect("encode carrier value"),
        );
        let frame = decoded(rest.clone());
        let mut viewer = ObserverViewerUiState::default();
        viewer.selected_country = Some("GER".into());
        let topology = crate::national_focus::FocusTreeTopology {
            tree_id: "german_focus".into(),
            effective_node_ids: std::collections::BTreeSet::from(["ger_focus".into()]),
        };
        let catalog = crate::national_focus::FocusCatalog {
            nodes: std::collections::BTreeMap::from([(
                "ger_focus".into(),
                crate::national_focus::FocusNodeDefinition {
                    id: "ger_focus".into(),
                    kind: crate::national_focus::FocusDefinitionKind::Local,
                    icon_sprite: Some("GFX_goal_generic".into()),
                    overlay_sprite: None,
                    title_loc_key: "ger_focus".into(),
                    joint_state_backgrounds: None,
                },
            )]),
        };
        let static_content = crate::static_focus_defs::AdmittedStaticFocusUiContent::fixture(
            manifest_sha256.clone(),
            topology,
            catalog,
        );
        let route = renderer.prepare_xui10_focus_route(
            frame.as_ref(),
            &viewer,
            &static_content,
        );
        assert!(route.gaps.is_empty(), "{:?}", route.gaps);
        let mut focus_viewer = crate::focus_observer::FocusViewerState::default();
        focus_viewer.open = true;
        let composed = renderer
            .compose_xui10_focus_surface(&route, &focus_viewer, 1920, 1080)
        .expect("complete EIR-NE-002 route executes compositor")
        .expect("open complete route composes");
        assert_eq!(composed.node_regions.len(), 1);
        assert!(composed.output.report.missing_sprites.is_empty());
        assert!(composed.output.report.undecodable_sprites.is_empty());
        assert!(composed.output.report.missing_fonts.is_empty());

        let assert_refused = |label: &str, refused_rest: serde_json::Map<String, serde_json::Value>| {
            let refused_frame = decoded(refused_rest);
            let refused = renderer.prepare_xui10_focus_route(
                refused_frame.as_ref(),
                &viewer,
                &static_content,
            );
            let result = renderer.compose_xui10_focus_surface(
                &refused,
                &focus_viewer,
                1920,
                1080,
            );
            assert!(
                matches!(result, Err(ProductionUiRejection::IncompleteComposition(_))),
                "{label}: {result:?}"
            );
        };

        let mut missing = rest.clone();
        missing.remove("_r2_national_focus_observer_exact");
        assert_refused("missing", missing);

        let mut malformed = rest.clone();
        malformed.insert(
            "_r2_national_focus_observer_exact".into(),
            serde_json::json!({"authority": "wrong"}),
        );
        assert_refused("malformed", malformed);

        let mut partial_carrier = carrier.clone();
        partial_carrier.producer_complete = false;
        partial_carrier.record_sha256.clear();
        partial_carrier.record_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&partial_carrier).expect("encode partial carrier"),
        );
        let mut partial = rest.clone();
        partial.insert(
            "_r2_national_focus_observer_exact".into(),
            serde_json::to_value(partial_carrier).expect("encode partial carrier value"),
        );
        assert_refused("partial", partial);

        let mut cross_hour_carrier = carrier;
        cross_hour_carrier.frame_hour = 899;
        cross_hour_carrier.record_sha256.clear();
        cross_hour_carrier.record_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&cross_hour_carrier).expect("encode cross-hour carrier"),
        );
        let mut cross_hour = rest;
        cross_hour.insert(
            "_r2_national_focus_observer_exact".into(),
            serde_json::to_value(cross_hour_carrier).expect("encode cross-hour carrier value"),
        );
        assert_refused("cross-hour", cross_hour);

        portable_test_fs_retry::remove_dir_all(authored_root).expect("remove authored XUI10 fixture root");
    }

    #[test]
    fn neutral_d98_section_may_join_shipping_only_with_the_exact_complete_d18_roster() {
        assert_eq!(shipping_d98_predecessor_sequence(900), Ok(900));
        let sequence_gap = shipping_d98_predecessor_sequence(u64::MAX)
            .expect_err("an underivable shipping sequence must refuse");
        assert_eq!(sequence_gap.field, "source_sequence");

        let roster = vec![
            replay_core::D18CountryRosterEntry {
                tag: "EMPTY".into(),
                tag_db_id: 91,
            },
            replay_core::D18CountryRosterEntry {
                tag: "GER".into(),
                tag_db_id: 23,
            },
        ];
        assert_eq!(
            require_exact_d18_d98_country_census(&roster, &[23, 91]),
            Ok(())
        );
        for bad in [vec![23], vec![23, 77, 91], vec![91, 23]] {
            let gap = require_exact_d18_d98_country_census(&roster, &bad)
                .expect_err("partial, extra, or noncanonical D98 census must refuse");
            assert_eq!(gap.field, "all_country_controller_authority");
            assert!(gap.reason.contains("exactly equal"));
        }

        let duplicate = vec![
            replay_core::D18CountryRosterEntry {
                tag: "GER".into(),
                tag_db_id: 23,
            },
            replay_core::D18CountryRosterEntry {
                tag: "GER_ALIAS".into(),
                tag_db_id: 23,
            },
        ];
        assert!(require_exact_d18_d98_country_census(&duplicate, &[23]).is_err());

        // kills: `admit_observer_theatre_surface` taking the RESOLVED half of a
        // roster that could not name one of the frame's countries. The surface
        // asserts D98's all-country census EQUALS the roster, so a roster short
        // by an unnameable country would make a D98 census that is equally
        // short look exact.
        //
        // Driven through the PRODUCTION FUNCTION, not through
        // `D18CountryRosterView::complete` directly: a test that calls
        // `complete()` itself only proves the view refuses, which
        // `replay_core`'s own tests already prove, and would stay green if this
        // call site were switched to `partial()?.resolved`.
        let unnameable_frame = {
            let mut rest = serde_json::Map::new();
            rest.insert(
                "countries".into(),
                serde_json::json!({"GER": {"_r2_tag_db_id": 23}, "367": {}}),
            );
            observer_test_frame(rest)
        };
        let gap = admit_observer_theatre_surface(&unnameable_frame, &content_admission(&[]), "GER")
            .expect_err("an incomplete D18 roster must refuse this surface");
        assert_eq!(gap.field, "selected_country_identity");
        assert!(
            gap.reason.contains("367") && gap.reason.contains("not complete"),
            "the refusal must name the country the recording could not name: {}",
            gap.reason
        );

        // ... and the refusal is the ROSTER's, not something this frame would
        // have refused anyway: drop the unnameable key and the same call gets
        // past the roster and fails on a later authority instead. Without this
        // arm the assertion above would also hold for a frame that never
        // reached the roster check.
        let nameable_frame = {
            let mut rest = serde_json::Map::new();
            rest.insert(
                "countries".into(),
                serde_json::json!({"GER": {"_r2_tag_db_id": 23}}),
            );
            observer_test_frame(rest)
        };
        let later = admit_observer_theatre_surface(&nameable_frame, &content_admission(&[]), "GER")
            .expect_err("this fixture carries no D98 selector, so it still refuses");
        assert_ne!(
            later.field, "selected_country_identity",
            "a complete roster must not refuse at the roster: {later:?}"
        );
    }

    #[test]
    fn the_retired_combat_log_icon_derives_only_from_the_exact_d57_carrier() {
        let d57_frame = |rows: serde_json::Value| {
            let mut rest = serde_json::Map::new();
            rest.insert(
                "_r2_session_multiplayer_metadata_live".into(),
                serde_json::json!({
                    "players_by_country": {"state": "known", "rows": rows}
                }),
            );
            observer_test_frame(rest)
        };
        let row = |tag: &str| {
            serde_json::json!({
                "country_tag": tag,
                "is_human_controlled": true,
                "tag_db_id": 23,
                "human_name": {"state": "not_sourced"}
            })
        };

        // The certified carrier: a measured player-country set. Membership
        // derives TRUE for a player country...
        let ger = d57_frame(serde_json::json!([row("GER")]));
        assert_eq!(derive_combat_log_icon(&ger, "GER"), Ok(true));
        // ...and FALSE for an observed non-player country -- a measured
        // negative, not a silent default.
        assert_eq!(derive_combat_log_icon(&ger, "SOV"), Ok(false));

        // kills: deriving off `build_session_metadata`'s LEGACY fallback keys.
        // `{"session":{"player_countries":["GER"]}}` decodes into a perfectly
        // measured one-row leaf with no engine identity and no producer order;
        // the carrier check must refuse it exactly like a missing set instead
        // of letting unproven data draw the icon.
        let mut legacy_only = serde_json::Map::new();
        legacy_only.insert(
            "session".into(),
            serde_json::json!({"player_countries": ["GER"]}),
        );
        let legacy_frame = observer_test_frame(legacy_only);
        let gap = derive_combat_log_icon(&legacy_frame, "GER")
            .expect_err("a legacy-fed player set has no identity or order proof to derive from");
        assert_eq!(gap.field, "combat_log_icon_source");
        assert!(gap.reason.contains("D57"), "{gap:?}");

        // A sidecar whose read FAILED is not an empty lobby either: refuse by
        // name instead of silently hiding a player's icon.
        let mut failed = serde_json::Map::new();
        failed.insert(
            "_r2_session_multiplayer_metadata_live".into(),
            serde_json::json!({"players_by_country": {"state": "unavailable"}}),
        );
        let failed_frame = observer_test_frame(failed);
        let gap = derive_combat_log_icon(&failed_frame, "GER")
            .expect_err("a failed D57 read must refuse, not render as no players");
        assert_eq!(gap.field, "combat_log_icon_source");
    }

    #[test]
    fn xui03_shared_collection_law_has_real_shipping_owners() {
        assert!(PROMOTED_XUI_PACKETS.contains(&"XUI03"));
        assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .any(|(packet, _)| *packet == "XUI03"));

        let designer = include_str!("designer_observer.rs");
        assert!(designer.contains("plan_observer_collection("));
        assert!(designer.contains("Xui13AcquiredVariantsStandardAutoAppend"));
        assert!(designer.contains("Xui13VariantCountryFilterStandardAutoAppend"));

        let trade = include_str!("trade_compositor.rs");
        assert!(trade.contains("plan_observer_collection("));
        assert!(trade.contains("Xui15CountryTradeStandardAutoAppend"));

        let occupation = include_str!("occupation_compositor.rs");
        assert!(occupation.contains("plan_authenticated_smooth_list_rects("));
        assert!(occupation.contains("\"equipments_list\""));

        let manager = include_str!("window_manager_law.rs");
        assert!(manager.contains("plan_authenticated_overlap_rects("));
        assert!(manager.contains("build_exact_a729_theatre_materialization_plan"));
    }

    #[test]
    fn packet_partition_has_no_overlap_and_keeps_all_authority_blocked_surfaces_omitted() {
        let promoted = PROMOTED_XUI_PACKETS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let omitted = OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(promoted.is_disjoint(&omitted));
        let omitted_canvas = OMITTED_PIXEL_CANVAS_ROUTES
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(promoted.is_disjoint(&omitted_canvas));
        assert!(omitted.is_disjoint(&omitted_canvas));
        assert!(omitted_canvas.is_empty());
        assert_eq!(
            promoted,
            [
                "XUI01", "XUI02", "XUI03", "XUI04", "XUI05", "XUI06", "XUI07", "XUI08", "XUI09",
                "XUI10", "XUI11", "XUI12", "XUI13", "XUI14", "XUI15", "XUI16", "XUI17", "XUI18",
                "XUI19", "XUI20", "XUI21", "XUI22", "XUI23", "XUI24", "XUI25", "XUI26",
            ]
            .into_iter()
            .collect()
        );
        for packet in [
            "XUI02", "XUI03", "XUI04", "XUI07", "XUI11", "XUI13", "XUI18", "XUI24", "XUI26",
        ] {
            assert!(promoted.contains(packet));
            assert!(!omitted.contains(packet));
            assert!(!omitted_canvas.contains(packet));
        }
        // including Presented descendant ownership and XUI05 tooltips. XUI12,
        assert!(omitted.is_empty());
        assert!(promoted.contains("XUI05"));
        assert!(promoted.contains("XUI23"));
        assert!(promoted.contains("XUI09"));
        assert!(promoted.contains("XUI10"));
        assert!(!omitted.contains("XUI10"));
        assert!(!omitted.contains("XUI23"));
        assert_eq!(PROMOTED_EXTERNAL_DEPENDENCY_PACKETS.len(), 16);
        assert_eq!(
            PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
                .iter()
                .map(|(packet, _)| *packet)
                .collect::<std::collections::BTreeSet<_>>(),
            [
                "XUI07", "XUI08", "XUI09", "XUI10", "XUI11", "XUI12", "XUI14", "XUI15", "XUI16",
                "XUI17", "XUI18", "XUI19", "XUI20", "XUI21", "XUI23", "XUI26",
            ]
            .into_iter()
            .collect()
        );
        for moved in [
            "XUI09", "XUI12", "XUI15", "XUI16", "XUI17", "XUI18", "XUI19",
        ] {
            assert!(
                !omitted.contains(moved),
                "{moved} was re-filed out of blocked"
            );
        }
        assert!(promoted.contains("XUI14"));
        assert!(promoted.contains("XUI08"));
        assert!(!omitted.contains("XUI08"));
        assert!(!omitted.contains("XUI14"));
        assert!(promoted.contains("XUI20"));
        assert!(!omitted.contains("XUI20"));
        assert!(promoted.contains("XUI22"));
        assert!(!omitted.contains("XUI22"));
        assert!(!omitted_canvas.contains("XUI22"));
        assert_eq!(PROMOTED_EXTENSION_PACKETS, ["XUI27"]);
        assert!(OMITTED_EXTENSION_PACKETS.is_empty());
        assert!(OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .all(|(_, dependency)| !dependency.trim().is_empty()));
    }

    #[test]
    fn xui15_content_and_producer_rejections_remain_feature_local() {
        let source = include_str!("production_renderer.rs");
        let start = source
            .find("let trade = if viewer.trade_panel_open")
            .expect("shipping XUI15 semantic route");
        let end = source[start..]
            .find("let decisions = if viewer.decision_panel_open")
            .map(|offset| start + offset)
            .expect("XUI15 route boundary");
        let route = &source[start..end];
        assert!(route.contains("field: \"effective_authored_root\""));
        assert!(route.contains("field: \"strict_catalogue_authority\""));
        assert!(route.contains("field: \"country_trade_catalogue_authority\""));
        assert!(route.contains("surface_gaps.push"));
        assert!(route.contains("gaps.extend(surface_gaps.iter().cloned())"));
        assert!(
            !route.contains("map_err(ProductionUiRejection::Asset)?"),
            "XUI15 content admission must not suppress unrelated observer UI"
        );
    }

    fn tooltip_instance(
        pov: &str,
        root: &str,
        text: &str,
    ) -> replay_core::ResolvedTooltipInstanceExact {
        replay_core::ResolvedTooltipInstanceExact {
            frame_hour: 1,
            pov_country_or_global: pov.into(),
            hovered_element_authored_name: "technology_slot".into(),
            tooltip_owner_authored_name: root.into(),
            repeat_index: 0,
            screen_rect: replay_core::ResolvedTooltipRectExact {
                x: 666,
                y: 871,
                width: 400,
                height: 120,
            },
            viewport_width_px: 1920,
            viewport_height_px: 1080,
            producer_record_identity: "controller:1".into(),
            final_resolved_text: text.into(),
        }
    }

    fn tooltip_view(
        instances: Vec<replay_core::ResolvedTooltipInstanceExact>,
        status: replay_core::ViewStatus,
    ) -> replay_core::ResolvedTooltipExactView {
        replay_core::ResolvedTooltipExactView {
            status,
            frame: Some(replay_core::ResolvedTooltipExactFrame {
                frame_hour: 1,
                selected_present_candidate_id: 9,
                selected_present_ordinal: 0,
                selected_present_qpc_ticks: 100,
                selected_present_receipt_sha256: "b7".repeat(32),
                producer_manifest_sha256: "a6".repeat(32),
                completeness_record_identity: "a729.tooltip.instances[1]".into(),
                instances,
            }),
        }
    }

    #[test]
    fn xui05_binding_carries_the_recorded_owner_root_and_the_blob_verbatim() {
        let blob = "Supply Area\nProjected supply is stable";
        let view = tooltip_view(
            vec![tooltip_instance("GER", "ToolTip", blob)],
            replay_core::ViewStatus::Present,
        );
        let binding = tooltip_binding_from_exact_view(&view, Some("GER"), 1920, 1080)
            .expect("an authenticated instance for this POV and viewport binds");
        assert_eq!((binding.x, binding.y), (666, 871));
        assert_eq!((binding.width, binding.height), (400, 120));
        assert_eq!(binding.text, blob);
        assert_eq!(binding.owner_root_name, "ToolTip");
    }

    #[test]
    fn xui05_omits_rather_than_guesses_on_every_absence() {
        let present = tooltip_view(
            vec![tooltip_instance("GER", "ToolTip", "text")],
            replay_core::ViewStatus::Present,
        );
        assert!(tooltip_binding_from_exact_view(&present, Some("FRA"), 1920, 1080).is_none());
        assert!(tooltip_binding_from_exact_view(&present, Some("GER"), 2560, 1440).is_none());
        assert!(tooltip_binding_from_exact_view(&present, None, 1920, 1080).is_none());

        let empty = tooltip_view(Vec::new(), replay_core::ViewStatus::Present);
        assert!(tooltip_binding_from_exact_view(&empty, Some("GER"), 1920, 1080).is_none());

        for status in [
            replay_core::ViewStatus::DomainAbsent,
            replay_core::ViewStatus::DomainUnavailable,
        ] {
            let view = tooltip_view(vec![tooltip_instance("GER", "ToolTip", "text")], status);
            assert!(tooltip_binding_from_exact_view(&view, Some("GER"), 1920, 1080).is_none());
        }
    }

    #[test]
    fn xui05_is_promoted_and_selected_present_remains_only_oracle_history() {
        assert!(PROMOTED_XUI_PACKETS.contains(&"XUI05"));
        assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .any(|(packet, _)| *packet == "XUI05"));
        assert!(!OMITTED_PIXEL_CANVAS_ROUTES
            .iter()
            .any(|(packet, _)| *packet == "XUI05"));
        let route = include_str!("production_renderer.rs");
        let start = route
            .find("pub fn compose_authenticated_xui05_tooltip(")
            .expect("shipping authenticated XUI05 route");
        let body = &route[start..];
        assert!(body.contains("resolve_authenticated_replay_tooltip_route("));
        assert!(!body[..body.find("pub fn compose_xui16_frame(").unwrap()]
            .contains("resolved_tooltip_exact"));
    }

    #[test]
    fn promoted_external_dependencies_name_the_exact_passive_authority() {
        let xui19 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
            .iter()
            .find(|(packet, _)| *packet == "XUI19")
            .expect("XUI19 is promoted with an explicit producer dependency");
        assert!(xui19.1.contains("exact-a729"));
        assert!(xui19.1.contains("passive"));
        assert!(xui19.1.contains("UI-open-independent"));
        assert!(xui19.1.contains("recorder-POV-independent"));
        assert!(xui19
            .1
            .contains("complete-D18 all-country decision evaluation"));
        assert!(PROMOTED_XUI_PACKETS.contains(&"XUI19"));
        assert!(!OMITTED_PIXEL_CANVAS_ROUTES
            .iter()
            .any(|(packet, _)| *packet == "XUI19"));
        assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .any(|(packet, _)| *packet == "XUI19"));
        let route = include_str!("production_renderer.rs");
        assert!(route.contains("pub fn compose_xui19_decision_surface"));

        let xui21 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
            .iter()
            .find(|(packet, _)| *packet == "XUI21")
            .expect("XUI21 is promoted with an explicit producer dependency");
        assert!(xui21.1.contains("exact-a729"));
        assert!(xui21.1.contains("passive"));
        assert!(xui21.1.contains("UI-open-independent"));
        assert!(xui21.1.contains("recorder-POV-independent"));
        assert!(xui21
            .1
            .contains("complete all-country extended-d75 event lifecycle"));
        assert!(xui21.1.contains("leader/operative runtime bindings"));
        assert!(xui21.1.contains("A25 final runtime-text contexts"));
        assert!(xui21
            .1
            .contains("complete authenticated effective events/**/*.txt namespace"));
        assert!(PROMOTED_XUI_PACKETS.contains(&"XUI21"));
        assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .any(|(packet, _)| *packet == "XUI21"));

        assert!(PROMOTED_XUI_PACKETS.contains(&"XUI14"));
        assert!(PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
            .iter()
            .any(|(packet, dependency)| *packet == "XUI14"
                && dependency.contains("passive")
                && dependency.contains("all-country")));
        assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .any(|(packet, _)| *packet == "XUI14"));
        let route = include_str!("production_renderer.rs");
        let start = route
            .find("pub fn compose_xui14_intelligence_surface(")
            .expect("shipping XUI14 compositor route");
        let body = &route[start..];
        assert!(body.contains("admit_intelligence_presentation("));
        assert!(body.contains("render_intelligence("));
        assert!(body.contains("require_used_content(source)"));

        assert!(PROMOTED_XUI_PACKETS.contains(&"XUI22"));
        assert!(OMITTED_PIXEL_CANVAS_ROUTES.is_empty());
        assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .any(|(packet, _)| *packet == "XUI22"));

        assert!(PROMOTED_XUI_PACKETS.contains(&"XUI20"));
        let xui20 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
            .iter()
            .find(|(packet, _)| *packet == "XUI20")
            .expect("XUI20 external producer dependency");
        for required in ["EIR-CW-002", "passive", "global", "five-panel combat"] {
            assert!(
                xui20.1.contains(required),
                "XUI20 dependency omits {required}"
            );
        }
        assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .any(|(packet, _)| *packet == "XUI20"));
    }

    fn activation_root(
        surface: replay_core::HudSurfaceActivationKind,
        runtime_root_name: &str,
    ) -> replay_core::HudSurfaceActivationRoot {
        replay_core::HudSurfaceActivationRoot {
            surface,
            runtime_root_name: runtime_root_name.into(),
        }
    }

    const ACTIVATION_HOUR: i64 = 7;

    fn activation_view(
        status: replay_core::ViewStatus,
        active_surfaces: Vec<replay_core::HudSurfaceActivationRoot>,
    ) -> replay_core::HudSurfaceActivationView {
        replay_core::HudSurfaceActivationView {
            status,
            activation: Some(replay_core::HudSurfaceActivation {
                schema_version: HUD_SURFACE_ACTIVATION_SCHEMA_VERSION,
                authority: HUD_SURFACE_ACTIVATION_AUTHORITY.into(),
                frame_hour: ACTIVATION_HOUR as u64,
                status_flags: 0,
                observation_sequence: 3,
                active_surfaces,
            }),
        }
    }

    fn admit_activation(
        view: &replay_core::HudSurfaceActivationView,
    ) -> std::result::Result<Option<AdmittedHudSurfaceActivation>, ProductionUiRejection> {
        admit_authenticated_hud_surface_activation(view, ACTIVATION_HOUR)
    }

    fn activation_refusal(view: &replay_core::HudSurfaceActivationView) -> String {
        match admit_activation(view) {
            Err(ProductionUiRejection::IncompleteComposition(reason)) => reason,
            other => panic!("expected a fail-closed refusal, got {other:?}"),
        }
    }

    fn four_core_roots() -> Vec<replay_core::HudSurfaceActivationRoot> {
        PRODUCT_HUD_SURFACES
            .iter()
            .map(|surface| activation_root(surface.activation_kind, surface.authored_root))
            .collect()
    }

    #[test]
    fn the_surface_table_is_the_single_extension_point_for_the_always_on_shape() {
        let tokens = PRODUCT_HUD_SURFACES
            .iter()
            .map(|surface| surface.consumer_token)
            .collect::<BTreeSet<_>>();
        let kinds = PRODUCT_HUD_SURFACES
            .iter()
            .map(|surface| surface.activation_kind)
            .collect::<BTreeSet<_>>();
        let roots = PRODUCT_HUD_SURFACES
            .iter()
            .map(|surface| surface.authored_root)
            .collect::<BTreeSet<_>>();
        assert_eq!(tokens.len(), PRODUCT_HUD_SURFACES.len());
        assert_eq!(kinds.len(), PRODUCT_HUD_SURFACES.len());
        assert_eq!(roots.len(), PRODUCT_HUD_SURFACES.len());
        assert_eq!(
            PRODUCT_HUD_SURFACES
                .iter()
                .map(|surface| surface.authored_gui)
                .collect::<BTreeSet<_>>(),
            REQUIRED_AUTHORED_HUD_SOURCES
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
        );
        for surface in PRODUCT_HUD_SURFACES {
            assert_eq!(
                surface.consumer_token,
                hud_activation_kind_token(surface.activation_kind)
            );
        }
        assert_eq!(PRODUCT_HUD_SURFACES.len(), 4);
    }

    #[test]
    fn an_absent_activation_carrier_binds_nothing_and_a_rejected_one_refuses() {
        let absent = activation_view(replay_core::ViewStatus::DomainAbsent, four_core_roots());
        assert_eq!(admit_activation(&absent), Ok(None));

        let rejected = activation_view(
            replay_core::ViewStatus::DomainUnavailable,
            four_core_roots(),
        );
        let reason = activation_refusal(&rejected);
        assert!(reason.contains("DomainUnavailable"), "{reason}");
        assert!(reason.contains("predates the domain"), "{reason}");
    }

    #[test]
    fn a_badged_activation_frame_refuses_even_though_replay_core_admits_it() {
        let mut view = activation_view(replay_core::ViewStatus::Present, four_core_roots());
        view.activation
            .as_mut()
            .expect("fixture carries one")
            .status_flags = 0x1;
        let reason = activation_refusal(&view);
        assert!(reason.contains("badged"), "{reason}");
        assert!(reason.contains("status_flags"), "{reason}");
        view.activation
            .as_mut()
            .expect("fixture carries one")
            .status_flags = 0;
        assert!(admit_activation(&view)
            .expect("clean frame admits")
            .is_some());
    }

    #[test]
    fn a_foreign_authority_schema_or_hour_refuses_at_the_route_boundary() {
        let mut view = activation_view(replay_core::ViewStatus::Present, four_core_roots());
        view.activation.as_mut().unwrap().authority = "exact_a729_something_else".into();
        assert!(activation_refusal(&view).contains("authority"));

        let mut view = activation_view(replay_core::ViewStatus::Present, four_core_roots());
        view.activation.as_mut().unwrap().schema_version = 2;
        assert!(activation_refusal(&view).contains("schema"));

        let mut view = activation_view(replay_core::ViewStatus::Present, four_core_roots());
        view.activation.as_mut().unwrap().frame_hour = ACTIVATION_HOUR as u64 + 1;
        let reason = activation_refusal(&view);
        assert!(reason.contains("not the composed hour"), "{reason}");
    }

    #[test]
    fn a_duplicated_surface_or_a_shared_runtime_root_refuses() {
        let mut roots = four_core_roots();
        roots.push(activation_root(
            replay_core::HudSurfaceActivationKind::Minimap,
            "a_second_minimap",
        ));
        assert!(
            activation_refusal(&activation_view(replay_core::ViewStatus::Present, roots))
                .contains("twice")
        );

        let mut roots = four_core_roots();
        roots.push(activation_root(
            replay_core::HudSurfaceActivationKind::DesignerTank,
            "topbar",
        ));
        let reason = activation_refusal(&activation_view(replay_core::ViewStatus::Present, roots));
        assert!(reason.contains("more than one surface"), "{reason}");

        let mut roots = four_core_roots();
        roots.push(activation_root(
            replay_core::HudSurfaceActivationKind::DesignerTank,
            "TOPBAR",
        ));
        let reason = activation_refusal(&activation_view(replay_core::ViewStatus::Present, roots));
        assert!(reason.contains("more than one surface"), "{reason}");
        assert!(reason.contains("ASCII-folded"), "{reason}");
    }

    #[test]
    fn a_case_differing_runtime_root_binds_rather_than_refusing() {
        let mut roots = four_core_roots();
        let minimap = roots
            .iter_mut()
            .find(|root| root.surface == replay_core::HudSurfaceActivationKind::Minimap)
            .expect("the table has a minimap row");
        minimap.runtime_root_name = minimap.runtime_root_name.to_ascii_uppercase();
        let admitted = admit_activation(&activation_view(replay_core::ViewStatus::Present, roots))
            .expect("a case-differing root is the same root")
            .expect("a Present carrier binds");
        assert_eq!(admitted.routed_surfaces().len(), PRODUCT_HUD_SURFACES.len());
    }

    #[test]
    fn a_present_activation_carrier_binds_every_drawn_surface_by_its_runtime_root() {
        let view = activation_view(replay_core::ViewStatus::Present, four_core_roots());
        let admitted = admit_activation(&view)
            .expect("a carrier naming exactly the drawn roots is admitted")
            .expect("a Present carrier binds");
        assert_eq!(
            admitted.routed_surfaces(),
            PRODUCT_HUD_SURFACES
                .iter()
                .map(|surface| surface.consumer_token.to_string())
                .collect::<Vec<_>>()
        );
        assert!(admitted.unrouted_active_surfaces().is_empty());
    }

    #[test]
    fn a_renamed_runtime_root_refuses_instead_of_drawing_the_vanilla_one() {
        let mut roots = four_core_roots();
        let minimap = roots
            .iter_mut()
            .find(|root| root.surface == replay_core::HudSurfaceActivationKind::Minimap)
            .expect("the table has a minimap row");
        minimap.runtime_root_name = "modded_minimap_container".into();
        let reason = activation_refusal(&activation_view(replay_core::ViewStatus::Present, roots));
        assert!(reason.contains("minimap"), "{reason}");
        assert!(reason.contains("modded_minimap_container"), "{reason}");
        assert!(reason.contains(crate::hud::MINIMAP_WINDOW_NAME), "{reason}");
    }

    #[test]
    fn a_drawn_surface_the_carrier_omits_refuses() {
        let roots = four_core_roots()
            .into_iter()
            .filter(|root| root.surface != replay_core::HudSurfaceActivationKind::Mapmodes)
            .collect();
        let reason = activation_refusal(&activation_view(replay_core::ViewStatus::Present, roots));
        assert!(reason.contains("mapmodes"), "{reason}");
    }

    #[test]
    fn retired_carrier_unrouted_row_is_reported_by_legacy_admission() {
        let mut roots = four_core_roots();
        roots.push(activation_root(
            replay_core::HudSurfaceActivationKind::DesignerTank,
            "mod_tank_designer",
        ));
        let view = activation_view(replay_core::ViewStatus::Present, roots);
        let admitted = admit_activation(&view)
            .expect("an active designer is not a refusal")
            .expect("a Present carrier binds");
        assert_eq!(admitted.routed_surfaces().len(), 4);
        assert_eq!(admitted.unrouted_active_surfaces(), ["designer/tank"]);
        assert!(!PRODUCT_HUD_SURFACES
            .iter()
            .any(|surface| surface.activation_kind
                == replay_core::HudSurfaceActivationKind::DesignerTank));
    }

    #[test]
    fn a_present_carrier_without_its_activation_refuses() {
        let view = replay_core::HudSurfaceActivationView {
            status: replay_core::ViewStatus::Present,
            activation: None,
        };
        assert!(activation_refusal(&view).contains("omitted its activation"));
    }

    #[test]
    fn every_activation_kind_the_carrier_can_name_has_a_report_token() {
        use replay_core::HudSurfaceActivationKind as Kind;
        let all = [
            Kind::Topbar,
            Kind::Minimap,
            Kind::Mapmodes,
            Kind::TheatreSelector,
            Kind::DesignerCountryEquipment,
            Kind::DesignerTank,
            Kind::DesignerPlane,
        ];
        let tokens = all
            .into_iter()
            .map(hud_activation_kind_token)
            .collect::<BTreeSet<_>>();
        assert_eq!(tokens.len(), 7);
        assert!(tokens.contains("designer/country_equipment"));
    }

    #[test]
    fn legacy_tooltip_oracle_adapter_remains_reviewable_without_the_hud_renderer() {
        let source = include_str!("production_renderer.rs").replace("\r\n", "\n");
        let free_fn = source
            .find("pub fn admit_xui05_tooltip_route(\n    content_admission: &ValidatedContentAdmission,")
            .expect("XUI05 admission is a free function over the content admission");
        let method = source
            .find("    pub fn admit_xui05_tooltip_route(\n        &self,")
            .expect("the renderer method still exists");
        assert!(free_fn < method);
        let after_signature = method
            + source[method..]
                .find("> {\n")
                .expect("the method signature ends in a return type and a brace")
            + "> {\n".len();
        let method_body = &source[after_signature..];
        let method_body = &method_body[..method_body
            .find("\n    }\n")
            .expect("the method has a body")];
        assert!(
            !method_body.contains("pub fn"),
            "the slice overran the method"
        );
        assert!(method_body.contains("admit_xui05_tooltip_route("));
        assert!(!method_body.contains("require_used_content"));
        assert!(!method_body.contains("TOOLTIP_WINDOW_NAME"));
    }

    #[test]
    fn only_the_frozen_xui25_viewport_is_admitted() {
        assert_eq!([PRODUCTION_UI_WIDTH, PRODUCTION_UI_HEIGHT], [1920, 1080]);
        let rejection = ProductionUiRejection::UnsupportedDisplayVariant {
            width: 1280,
            height: 720,
        };
        assert!(matches!(
            rejection,
            ProductionUiRejection::UnsupportedDisplayVariant {
                width: 1280,
                height: 720
            }
        ));
    }

    #[test]
    fn xui21_minimize_tooltip_recursively_resolves_text_icons_and_provenance() {
        let mut localisation = Localisation::default();
        localisation.entries.insert(
            "XUI21_CLOSE".into(),
            "Close $XUI21_WINDOW$ £political_power|2".into(),
        );
        localisation
            .entries
            .insert("XUI21_WINDOW".into(), "event window".into());
        localisation.source_by_key.insert(
            "XUI21_CLOSE".into(),
            "localisation/english/event_l_english.yml".into(),
        );
        localisation.source_by_key.insert(
            "XUI21_WINDOW".into(),
            "localisation/english/common_l_english.yml".into(),
        );
        localisation.provenance_complete = true;

        let (text, sources) = resolve_xui21_authored_tooltip_text(&localisation, "XUI21_CLOSE")
            .expect("recursive authored tooltip is exact");
        assert_eq!(text, "Close event window £political_power|2");
        assert_eq!(sources.len(), 2);
        assert!(sources.iter().any(|source| matches!(
            source,
            AdmittedTooltipSource::EffectiveLocalisation { key, source_files }
                if key == "XUI21_CLOSE"
                    && source_files == &["localisation/english/event_l_english.yml"]
        )));
        assert!(sources.iter().any(|source| matches!(
            source,
            AdmittedTooltipSource::EffectiveLocalisation { key, source_files }
                if key == "XUI21_WINDOW"
                    && source_files == &["localisation/english/common_l_english.yml"]
        )));

        localisation
            .entries
            .insert("XUI21_BROKEN".into(), "Close $XUI21_MISSING$".into());
        localisation.source_by_key.insert(
            "XUI21_BROKEN".into(),
            "localisation/english/event_l_english.yml".into(),
        );
        assert!(resolve_xui21_authored_tooltip_text(&localisation, "XUI21_BROKEN").is_err());

        localisation.ambiguous_sources_by_key.insert(
            "XUI21_WINDOW".into(),
            vec![
                "localisation/english/common_l_english.yml".into(),
                "localisation/english/mod_l_english.yml".into(),
            ],
        );
        localisation.source_by_key.remove("XUI21_WINDOW");
        assert!(resolve_xui21_authored_tooltip_text(&localisation, "XUI21_CLOSE").is_err());
    }

    #[test]
    fn xui21_production_compositor_returns_no_surface_for_measured_empty_history() {
        let renderer = ProductionUiRenderer {
            resolver: AssetResolver::new(Vec::new()),
            registry: SpriteRegistry::new(),
            localisation: Localisation::default(),
            playback_localisation_language: PRODUCTION_UI_LANGUAGE.into(),
            character_portraits: Err("unused test feature".into()),
            character_traits: Err("unused test feature".into()),
            character_skill_costs: Err("unused test feature".into()),
            ability_catalog: Err("unused test feature".into()),
            power_balance_catalog: Err("unused test feature".into()),
            power_balance_decisions: Err("unused test feature".into()),
            event_definitions: Ok(BTreeMap::new()),
            event_layout: Ok(crate::gui::LayoutSet::default()),
            content_admission: content_admission(&[]),
            playback_variant_key: None,
            asset_cache: Mutex::new(UiAssetCache::default()),
        };
        let route = ObserverEventRoute {
            country_tag: Some("ENG".into()),
            admitted_history: Some(Vec::new()),
            event_tick: Some(900),
            visible_sequences: Vec::new(),
            focused_sequence: None,
            measured_empty: true,
            gaps: Vec::new(),
        };
        assert!(renderer
            .compose_xui21_event_stack(&route, PRODUCTION_UI_WIDTH, PRODUCTION_UI_HEIGHT)
            .expect("measured-empty history is a successful zero-surface composition")
            .is_empty());
    }

    #[test]
    fn xui21_event_registry_refuses_a_manifest_without_the_complete_namespace_capability() {
        let error = crate::event_chrome::load_admitted_event_definitions(
            &AssetResolver::new(Vec::new()),
            &content_admission(&[]),
        )
        .expect_err("an incidental or absent event corpus must not be admitted");
        assert!(
            error.contains(crate::event_chrome::EVENT_DEFINITION_NAMESPACE_CONSUMER),
            "{error}"
        );
    }


    fn production_source() -> String {
        let source = include_str!("production_renderer.rs").replace("\r\n", "\n");
        let marker = ["#[cfg(", "test)]"].concat();
        let cut = source
            .rfind(&marker)
            .expect("this module has a test module to cut off");
        source[..cut].to_string()
    }

    #[test]
    fn the_production_renderer_source_slice_actually_excludes_the_tests() {
        let production = production_source();
        assert!(production.len() < include_str!("production_renderer.rs").len());
        assert!(!production.contains('\r'));
        assert!(
            !production.contains(&["mod ", "tests {"].concat()),
            "the cut landed after the test module opens"
        );
        let test_only = [
            "the_production_renderer_source_slice_",
            "actually_excludes_the_tests",
        ]
        .concat();
        assert!(!production.contains(&test_only));
        assert!(production.contains("pub fn prepare_observer_semantic_frame("));
    }

    #[test]
    fn the_production_renderer_never_reaches_for_the_probe_hud_compositor() {
        let production = production_source();
        let hud = include_str!("hud.rs").replace("\r\n", "\n");
        let topbar = include_str!("topbar.rs").replace("\r\n", "\n");
        for (probe_only, declaration, haystack) in [
            ("compose_hud", "pub fn compose_hud(", &hud),
            (
                "compose_minimap_panel",
                "pub fn compose_minimap_panel(",
                &hud,
            ),
            (
                "compose_mapmode_panel",
                "pub fn compose_mapmode_panel(",
                &hud,
            ),
            (
                "compose_outliner_panel",
                "pub fn compose_outliner_panel(",
                &hud,
            ),
            ("HudBinding::sample", "    pub fn sample() -> Self {", &hud),
            (
                "topbar_debug_overlay",
                "pub fn topbar_debug_overlay(",
                &topbar,
            ),
        ] {
            assert!(
                !production.contains(probe_only),
                "production composition must not reach the probe-only item {probe_only}"
            );
            assert!(
                haystack.contains(declaration),
                "the forbidden needle {probe_only} no longer names a declared item; \
                 the prohibition above has stopped guarding anything"
            );
        }
    }

    #[test]
    fn only_the_playback_constructor_supplies_a_variant_key() {
        let production = production_source();
        let slice_between = |open: &str, close: &str| {
            let start = production.find(open).expect("constructor opening");
            let end = production[start..]
                .find(close)
                .map(|offset| start + offset)
                .expect("constructor closing");
            production[start..end].to_string()
        };
        assert!(
            slice_between("    pub fn new(\n", "    pub fn new_for_playback(\n")
                .contains("Self::new_with_playback_profile(resolver, content_admission, None)"),
            "the non-playback constructor must forward an absent profile verbatim"
        );
        assert!(
            slice_between(
                "    pub fn new_for_playback(\n",
                "    fn new_with_playback_profile(\n"
            )
            .contains("Self::new_with_playback_profile(resolver, content_admission, Some(profile))"),
            "the playback constructor must forward the profile it was handed verbatim"
        );

        assert!(
            !production.contains(".playback_variant_key ="),
            "the stored key must never be re-assigned after construction"
        );

        let initialiser = production
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("playback_variant_key:"))
            .collect::<Vec<_>>();
        assert_eq!(
            initialiser,
            [
                "playback_variant_key: Option<DisplayVariantKey>,",
                "playback_variant_key: profile.map(|profile| profile.key().clone()),",
            ],
            "the stored key must be exactly the supplied profile's key, with no \
             fallback and no second assignment anywhere in the production half"
        );
    }

    fn observer_only_renderer(
        playback_variant_key: Option<DisplayVariantKey>,
    ) -> ProductionUiRenderer {
        ProductionUiRenderer {
            resolver: AssetResolver::new(Vec::new()),
            registry: crate::gfx::SpriteRegistry::new(),
            localisation: Localisation::default(),
            playback_localisation_language: PRODUCTION_UI_LANGUAGE.into(),
            character_portraits: Err("unused test feature".into()),
            character_traits: Err("unused test feature".into()),
            character_skill_costs: Err("unused test feature".into()),
            ability_catalog: Err("unused test feature".into()),
            power_balance_catalog: Err("unused test feature".into()),
            power_balance_decisions: Err("unused test feature".into()),
            event_definitions: Ok(BTreeMap::new()),
            event_layout: Ok(crate::gui::LayoutSet::default()),
            content_admission: content_admission(&[]),
            playback_variant_key,
            asset_cache: Mutex::new(UiAssetCache::default()),
        }
    }

    fn observer_test_frame(
        rest: serde_json::Map<String, serde_json::Value>,
    ) -> std::sync::Arc<WorldFrame> {
        use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};

        struct OneFrame {
            rest: serde_json::Map<String, serde_json::Value>,
        }
        impl ReplaySource for OneFrame {
            fn game_id(&self) -> &str {
                "kh-observer-preparation"
            }
            fn hours(&self) -> &[i64] {
                &[1_000]
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

        WorldTimeline::open(Box::new(OneFrame { rest }))
            .expect("open in-memory observer timeline")
            .state_at(1_000)
            .expect("decode observer frame")
    }

    fn observer_static_focus_content() -> crate::static_focus_defs::AdmittedStaticFocusUiContent {
        crate::static_focus_defs::AdmittedStaticFocusUiContent::fixture(
            content_admission(&[]).manifest_digest_sha256().to_string(),
            crate::national_focus::FocusTreeTopology {
                tree_id: "kh_observer_tree".into(),
                effective_node_ids: BTreeSet::new(),
            },
            crate::national_focus::FocusCatalog {
                nodes: BTreeMap::new(),
            },
        )
    }

    fn observer_country_frame() -> std::sync::Arc<WorldFrame> {
        let mut rest = serde_json::Map::new();
        rest.insert(
            "countries".into(),
            serde_json::json!({"GER": {"_r2_tag_db_id": 73}}),
        );
        observer_test_frame(rest)
    }

    fn prepare_observer(
        renderer: &ProductionUiRenderer,
        frame: &WorldFrame,
        static_content: &crate::static_focus_defs::AdmittedStaticFocusUiContent,
        viewer: &ObserverViewerUiState,
        playback: &ObserverPlaybackUiProfile,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<ObserverUiSemanticFrame, ProductionUiRejection> {
        renderer.prepare_observer_semantic_frame_with_xui12_and_xui22_producers(
            frame,
            None,
            static_content,
            viewer,
            playback,
            0,
            false,
            viewport_width,
            viewport_height,
            Ok(None),
            None,
            None,
        )
    }

    #[test]
    fn an_unsupported_physical_viewport_is_refused_before_the_playback_carrier_is_consulted() {
        let frozen = production_ui_variant_key();
        let renderer = observer_only_renderer(Some(frozen.clone()));
        let frame = observer_country_frame();
        let static_content = observer_static_focus_content();
        let viewer = ObserverViewerUiState::default();
        let playback = ObserverPlaybackUiProfile {
            variant_key: frozen.clone(),
        };

        assert_eq!(
            prepare_observer(
                &renderer,
                frame.as_ref(),
                &static_content,
                &viewer,
                &playback,
                1280,
                720,
            ),
            Err(ProductionUiRejection::UnsupportedDisplayVariant {
                width: 1280,
                height: 720,
            }),
            "a viewport the playback profile did not admit must be refused"
        );

        assert_eq!(
            prepare_observer(
                &observer_only_renderer(None),
                frame.as_ref(),
                &static_content,
                &viewer,
                &playback,
                1280,
                720,
            ),
            Err(ProductionUiRejection::UnsupportedDisplayVariant {
                width: 1280,
                height: 720,
            }),
            "the viewport must be settled before the playback carrier is consulted"
        );

        assert!(prepare_observer(
            &renderer,
            frame.as_ref(),
            &static_content,
            &viewer,
            &playback,
            PRODUCTION_UI_WIDTH,
            PRODUCTION_UI_HEIGHT,
        )
        .is_ok());

        let unfrozen = DisplayVariantKey {
            physical_viewport: PixelExtent {
                width: 1280,
                height: 720,
            },
            ..frozen
        };
        let unfrozen_renderer = observer_only_renderer(Some(unfrozen.clone()));
        assert_eq!(
            prepare_observer(
                &unfrozen_renderer,
                frame.as_ref(),
                &static_content,
                &viewer,
                &ObserverPlaybackUiProfile {
                    variant_key: unfrozen.clone(),
                },
                1280,
                720,
            ),
            Err(ProductionUiRejection::UnsupportedExactDisplayVariant(
                unfrozen
            )),
            "an internally consistent but non-frozen variant must be refused by the \
             exact-key contract, naming the key, not by the viewport arithmetic"
        );
    }

    #[test]
    fn observer_preparation_requires_the_playback_owned_variant_key_and_will_not_infer_one() {
        let frozen = production_ui_variant_key();
        let frame = observer_country_frame();
        let static_content = observer_static_focus_content();
        let viewer = ObserverViewerUiState::default();
        let playback = ObserverPlaybackUiProfile {
            variant_key: frozen.clone(),
        };

        assert_eq!(
            prepare_observer(
                &observer_only_renderer(None),
                frame.as_ref(),
                &static_content,
                &viewer,
                &playback,
                PRODUCTION_UI_WIDTH,
                PRODUCTION_UI_HEIGHT,
            ),
            Err(ProductionUiRejection::MissingDisplayVariantCarrier(
                "playback-owned observer UI profile"
            )),
        );

        let foreign = DisplayVariantKey {
            localisation_language: "l_french".to_string(),
            ..frozen.clone()
        };
        assert_eq!(
            prepare_observer(
                &observer_only_renderer(Some(foreign)),
                frame.as_ref(),
                &static_content,
                &viewer,
                &playback,
                PRODUCTION_UI_WIDTH,
                PRODUCTION_UI_HEIGHT,
            ),
            Err(ProductionUiRejection::UnsupportedExactDisplayVariant(
                frozen
            )),
        );
    }

    #[test]
    fn observer_preparation_is_decode_stable_and_country_scope_is_viewer_owned() {
        let frozen = production_ui_variant_key();
        let renderer = observer_only_renderer(Some(frozen.clone()));
        let static_content = observer_static_focus_content();
        let playback = ObserverPlaybackUiProfile {
            variant_key: frozen,
        };
        let mut viewer = ObserverViewerUiState::default();
        viewer.selected_country = Some("GER".into());

        let prepare = |frame: &WorldFrame, viewer: &ObserverViewerUiState| {
            prepare_observer(
                &renderer,
                frame,
                &static_content,
                viewer,
                &playback,
                PRODUCTION_UI_WIDTH,
                PRODUCTION_UI_HEIGHT,
            )
            .expect("the shipping observer entry point prepares a semantic frame")
        };

        let first = prepare(observer_country_frame().as_ref(), &viewer);
        let second = prepare(observer_country_frame().as_ref(), &viewer);
        assert_eq!(
            first, second,
            "two independent decodes of the same recorded frame content must prepare \
             to the same semantic frame"
        );

        let mut other_country = ObserverViewerUiState::default();
        other_country.selected_country = Some("FRA".into());
        let elsewhere = prepare(observer_country_frame().as_ref(), &other_country);
        assert_ne!(
            first.hud_binding.topbar, elsewhere.hud_binding.topbar,
            "the topbar must bind the country the viewer selected, not the one the \
             recording happens to contain"
        );
        assert_ne!(
            first, elsewhere,
            "the country the viewer selected must decide what is prepared"
        );

        let omniscient = prepare(
            observer_country_frame().as_ref(),
            &ObserverViewerUiState::default(),
        );
        assert_ne!(
            first, omniscient,
            "the viewer's country selection must change what is prepared"
        );
        assert!(
            first
                .gaps
                .iter()
                .any(|gap| gap.surface == "XUI07/theatre_selector"),
            "a selected country runs the country-scoped theatre admission: {:?}",
            first.gaps
        );
        assert!(
            !omniscient
                .gaps
                .iter()
                .any(|gap| gap.surface == "XUI07/theatre_selector"),
            "with no country selected the country-scoped admission must not run at \
             all, so the recording cannot supply a POV of its own: {:?}",
            omniscient.gaps
        );
        assert!(
            omniscient.theatre.is_none(),
            "no country selected must leave every country-scoped surface absent"
        );
    }
}
