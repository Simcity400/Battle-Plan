//! These laws and their sealed cross-packet payload ABI are non-promoting on

use crate::assets::AssetResolver;
use crate::gui::{GuiElement, LayoutSet};
use crate::loc::Localisation;
use crate::text_law::{
    resolve_localisation_preserving_inline_icons, RuntimeTextContext, TextIssue,
};
use crate::widget_visual_state::WidgetFamily;
use crate::{GameUiError, Result};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ScreenRect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains_half_open(self, point: (i32, i32)) -> bool {
        self.width > 0
            && self.height > 0
            && point.0 >= self.x
            && point.1 >= self.y
            && point.0 < self.x.saturating_add(self.width)
            && point.1 < self.y.saturating_add(self.height)
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self
            .x
            .saturating_add(self.width)
            .min(other.x.saturating_add(other.width));
        let y1 = self
            .y
            .saturating_add(self.height)
            .min(other.y.saturating_add(other.height));
        (x1 > x0 && y1 > y0).then(|| Self::new(x0, y0, x1 - x0, y1 - y0))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TooltipOwner {
    pub(crate) source_file: String,
    pub(crate) root_window: String,
    pub(crate) element_path: Vec<u32>,
    pub(crate) engine_instance_token: String,
}

impl TooltipOwner {
    pub fn source_file(&self) -> &str {
        &self.source_file
    }

    pub fn root_window(&self) -> &str {
        &self.root_window
    }

    pub fn element_path(&self) -> &[u32] {
        &self.element_path
    }

    pub fn engine_instance_token(&self) -> &str {
        &self.engine_instance_token
    }

    pub fn matches_authenticated_target(
        &self,
        source_file: &str,
        root_window: &str,
        element_path: &[u32],
        engine_instance_token: &str,
    ) -> bool {
        self.source_file == source_file
            && self.root_window == root_window
            && self.element_path == element_path
            && self.engine_instance_token == engine_instance_token
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitSurface {
    pub rect: ScreenRect,
    pub clip_rect: ScreenRect,
    pub draw_list_ordinal: u32,
    pub z_order: i32,
    pub item_ordinal: u32,
    pub owner: Option<TooltipOwner>,
}

pub fn frontmost_owner(surfaces: &[HitSurface], point: (i32, i32)) -> Option<&TooltipOwner> {
    surfaces
        .iter()
        .filter(|surface| {
            surface
                .rect
                .intersection(surface.clip_rect)
                .is_some_and(|visible| visible.contains_half_open(point))
        })
        .max_by_key(|surface| {
            (
                surface.draw_list_ordinal,
                surface.z_order,
                surface.item_ordinal,
            )
        })
        .and_then(|surface| surface.owner.as_ref())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooltipTimingLaw {
    pub show_delay_us: u64,
    pub hide_delay_us: u64,
    pub delayed_delay_us: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HoverCandidate {
    pub owner: TooltipOwner,
    pub route_identity: String,
    pub delayed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TooltipLifecycle {
    timing: TooltipTimingLaw,
    candidate: Option<HoverCandidate>,
    candidate_elapsed_us: u64,
    visible: Option<TooltipOwner>,
    leave_elapsed_us: u64,
}

impl TooltipLifecycle {
    pub(crate) fn new(timing: TooltipTimingLaw) -> Self {
        Self {
            timing,
            candidate: None,
            candidate_elapsed_us: 0,
            visible: None,
            leave_elapsed_us: 0,
        }
    }

    pub(crate) fn visible_owner(&self) -> Option<&TooltipOwner> {
        self.visible.as_ref()
    }

    pub(crate) fn delayed_content_visible(&self) -> bool {
        self.visible.is_some()
            && self
                .candidate
                .as_ref()
                .is_some_and(|candidate| candidate.delayed)
            && self.candidate_elapsed_us >= self.timing.delayed_delay_us
    }

    pub(crate) fn clear(&mut self) {
        self.candidate = None;
        self.candidate_elapsed_us = 0;
        self.visible = None;
        self.leave_elapsed_us = 0;
    }

    pub(crate) fn advance(&mut self, next: Option<HoverCandidate>, delta_us: u64) {
        if next != self.candidate {
            self.candidate = next;
            self.candidate_elapsed_us = 0;
            self.leave_elapsed_us = 0;
            if self.candidate.is_some() {
                self.visible = None;
            }
        } else if self.candidate.is_some() {
            self.candidate_elapsed_us = self.candidate_elapsed_us.saturating_add(delta_us);
        }
        if let Some(candidate) = &self.candidate {
            self.leave_elapsed_us = 0;
            if self.candidate_elapsed_us >= self.timing.show_delay_us {
                self.visible = Some(candidate.owner.clone());
            }
        } else if self.visible.is_some() {
            self.leave_elapsed_us = self.leave_elapsed_us.saturating_add(delta_us);
            if self.leave_elapsed_us >= self.timing.hide_delay_us {
                self.visible = None;
                self.leave_elapsed_us = 0;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlacementEvidence {
    ExtractedEngineRect {
        producer_record: String,
    },
    RenderDocOracle {
        capture_sha256: String,
    },
    ViewerCursor {
        raw_px: (i32, i32),
        ui_px: (i32, i32),
        gui_scale_f32_bits: (u32, u32),
    },
    AuthoredAbsoluteAnchor {
        packed_position: (i32, i32),
    },
    AuthoredOwnerRelativeAnchor {
        packed_position: (i32, i32),
        owner_screen_position: (i32, i32),
    },
    ViewerControllerRetained {
        position: (i32, i32),
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPlacement {
    pub screen_rect: ScreenRect,
    pub evidence: PlacementEvidence,
}

impl ResolvedPlacement {
    pub fn clipped_to(&self, viewport: ScreenRect) -> Result<(ScreenRect, ScreenRect)> {
        if self.screen_rect.width <= 0 || self.screen_rect.height <= 0 {
            return Err(GameUiError::parse("tooltip screen rectangle is empty"));
        }
        let clipped = self
            .screen_rect
            .intersection(viewport)
            .ok_or_else(|| GameUiError::missing("tooltip is fully outside viewport"))?;
        Ok((self.screen_rect, clipped))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowAvailability {
    Resolved,
    MeasuredEmpty,
    NotApplicable,
    DataBlocked {
        field: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TooltipLineKind {
    Header,
    Description,
    Value,
    Breakdown,
    Scripted,
    Spacer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipRow {
    pub source_ordinal: u32,
    pub line_kind: TooltipLineKind,
    pub source_identity: String,
    pub evaluation_context: String,
    pub availability: RowAvailability,
    pub text: String,
    pub admitted_sources: Vec<AdmittedTooltipSource>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmittedTooltipSource {
    EffectiveGui {
        game_relative_path: String,
        element_path: String,
    },
    EffectiveLocalisation {
        key: String,
        source_files: Vec<String>,
    },
    EffectiveSprite {
        sprite: String,
        definition_file: String,
        texture_file: String,
    },
    EffectivePlacement {
        graphics_defines_file: String,
    },
    EffectiveAsset {
        role: String,
        game_relative_path: String,
    },
    RecordedState {
        domain: String,
        field: String,
        identity: String,
    },
    PacketSemantic {
        packet_id: String,
        producer_identity: String,
        field: String,
        manifest_sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipPayload {
    pub(crate) owner: TooltipOwner,
    pub(crate) tooltip_root_name: String,
    pub(crate) rows: Vec<TooltipRow>,
}

impl TooltipPayload {
    pub fn owner(&self) -> &TooltipOwner {
        &self.owner
    }

    pub fn tooltip_root_name(&self) -> &str {
        &self.tooltip_root_name
    }

    pub fn rows(&self) -> &[TooltipRow] {
        &self.rows
    }

    pub fn validate(&self) -> Result<()> {
        if self.owner.source_file.trim().is_empty()
            || self.owner.root_window.trim().is_empty()
            || self.owner.engine_instance_token.trim().is_empty()
        {
            return Err(GameUiError::missing(
                "tooltip payload has no pointer-free owner identity",
            ));
        }
        if self.tooltip_root_name.trim().is_empty() {
            return Err(GameUiError::missing(
                "tooltip payload has no authored shell identity",
            ));
        }
        if self.rows.is_empty() {
            return Err(GameUiError::missing("tooltip payload has no producer rows"));
        }
        let mut prior = None;
        for row in &self.rows {
            if row.source_identity.trim().is_empty() || row.evaluation_context.trim().is_empty() {
                return Err(GameUiError::missing(
                    "tooltip row has no producer identity/evaluation context",
                ));
            }
            if prior.is_some_and(|value| row.source_ordinal <= value) {
                return Err(GameUiError::parse(
                    "tooltip row ordinals must be unique and strictly increasing",
                ));
            }
            prior = Some(row.source_ordinal);
            match &row.availability {
                RowAvailability::Resolved if row.text.is_empty() => {
                    return Err(GameUiError::parse("resolved tooltip row has empty text"))
                }
                RowAvailability::Resolved if row.admitted_sources.is_empty() => {
                    return Err(GameUiError::missing(format!(
                        "tooltip row {:?} has no admitted-state trace",
                        row.source_identity
                    )))
                }
                RowAvailability::MeasuredEmpty if !row.text.is_empty() => {
                    return Err(GameUiError::parse(
                        "measured-empty tooltip row carries non-empty text",
                    ))
                }
                RowAvailability::MeasuredEmpty if row.admitted_sources.is_empty() => {
                    return Err(GameUiError::missing(format!(
                        "measured-empty tooltip row {:?} has no admitted-state trace",
                        row.source_identity
                    )))
                }
                RowAvailability::NotApplicable if !row.text.is_empty() => {
                    return Err(GameUiError::parse(
                        "not-applicable tooltip row carries text",
                    ))
                }
                RowAvailability::DataBlocked { field } => {
                    return Err(GameUiError::missing(format!(
                        "tooltip upstream field is DATA_BLOCKED: {field}"
                    )))
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordedFact<T> {
    Absent,
    Null,
    Unknown { reason: String },
    Value(T),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TooltipRefusal {
    MissingLeaf {
        family: String,
        leaf: String,
    },
    FactAbsent {
        field: String,
    },
    FactNull {
        field: String,
    },
    FactUnknown {
        field: String,
        reason: String,
    },
    MissingLocalisationKey {
        key: String,
    },
    TextEvaluationIssues {
        key: String,
        issues: Vec<TextIssue>,
    },
    DamagedLocalisationCorpus {
        failures: Vec<String>,
    },
    InternalIdentityAsDisplayName {
        identity: String,
    },
    UntraceableRow {
        row: String,
    },
    InvalidPayload {
        reason: String,
    },
    MissingAuthoredQuantity {
        quantity: String,
    },
    InvalidAuthoredQuantity {
        quantity: String,
        value: String,
    },
    InvalidViewerQuantity {
        quantity: String,
        value: String,
    },
    RowRequiresWrap {
        row: String,
        measured_width: i32,
        interior_width: i32,
    },
    ArithmeticOverflow {
        operation: String,
    },
}

impl fmt::Display for TooltipRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for TooltipRefusal {}

fn require_fact<T>(field: &str, fact: RecordedFact<T>) -> std::result::Result<T, TooltipRefusal> {
    match fact {
        RecordedFact::Absent => Err(TooltipRefusal::FactAbsent {
            field: field.to_string(),
        }),
        RecordedFact::Null => Err(TooltipRefusal::FactNull {
            field: field.to_string(),
        }),
        RecordedFact::Unknown { reason } => Err(TooltipRefusal::FactUnknown {
            field: field.to_string(),
            reason,
        }),
        RecordedFact::Value(value) => Ok(value),
    }
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredUnscopedTooltipFacts {
    pub owner: TooltipOwner,
    pub tooltip_root_name: String,
    pub source_ordinal: u32,
    pub gui_source_path: String,
    pub element_path: String,
    pub localisation_key: RecordedFact<String>,
    pub localisation_property_name: String,
    pub delayed_localisation_key: Option<String>,
    pub delayed_property_name: Option<String>,
    pub authored_anchor: AuthoredTooltipAnchor,
    pub include_delayed: bool,
    pub has_scoped_evaluation: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthoredTooltipAnchor {
    Cursor,
    Packed {
        position: (i32, i32),
        relative_to_owner: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthoredTooltipTarget {
    pub gui_source_path: String,
    pub runtime_root_name: String,
    pub child_path: Vec<u32>,
    /// `child_path`; payload ownership uses this exact composed path.
    pub owner_child_path: Option<Vec<u32>>,
    pub element_type: String,
    pub element_name: String,
    pub engine_instance_token: String,
    pub paint_ordinal: u32,
    pub alternate_tooltip_selected: RecordedFact<bool>,
    pub include_delayed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedReplayTooltipContext {
    owner: crate::window_manager_law::AuthenticatedReplayTooltipOwner,
    hovered_surface: crate::authored_role_index::AuthenticatedPanelSurface,
    tooltip_surface: crate::authored_role_index::AuthenticatedPanelSurface,
    cursor: (i32, i32),
}

pub fn authenticate_replay_tooltip_context(
    owner: crate::window_manager_law::AuthenticatedReplayTooltipOwner,
    hovered_surface: crate::authored_role_index::AuthenticatedPanelSurface,
    tooltip_surface: crate::authored_role_index::AuthenticatedPanelSurface,
    cursor: (i32, i32),
) -> Result<AuthenticatedReplayTooltipContext> {
    if owner.frame_generation() == 0
        || owner.window_id() != hovered_surface.window_id()
        || owner.surface_id() != hovered_surface.surface_id()
        || owner.source_file()
            != hovered_surface
                .source_content()
                .normalized_game_relative_path
        || hovered_surface.manifest_digest_sha256() != tooltip_surface.manifest_digest_sha256()
        || hovered_surface.product_modset_sha256() != tooltip_surface.product_modset_sha256()
    {
        return Err(GameUiError::missing(
            "XUI05 hover/surface/shell identity join is incomplete",
        ));
    }

    let role = hovered_surface
        .element_role(owner.child_path())
        .ok_or_else(|| GameUiError::missing("XUI05 hovered element role is absent"))?;
    if role.element_id != owner.element_id()
        || role.element_type != owner.element_type()
        || role.authored_name != owner.authored_name()
    {
        return Err(GameUiError::missing(
            "XUI05 hovered element differs from its authenticated authored role",
        ));
    }
    if tooltip_surface.tooltip_shell().is_none() {
        return Err(GameUiError::missing(
            "XUI05 structural tooltip-shell capability is absent",
        ));
    }
    let screen = owner.screen_rect();
    let clip = owner.clip_rect();
    let visible = ScreenRect::new(screen.x(), screen.y(), screen.w(), screen.h())
        .intersection(ScreenRect::new(clip.x(), clip.y(), clip.w(), clip.h()));
    if !visible.is_some_and(|rect| rect.contains_half_open(cursor)) {
        return Err(GameUiError::missing(
            "XUI05 viewer cursor is outside the authenticated hovered geometry",
        ));
    }
    Ok(AuthenticatedReplayTooltipContext {
        owner,
        hovered_surface,
        tooltip_surface,
        cursor,
    })
}

impl AuthenticatedReplayTooltipContext {
    pub(crate) fn owner(&self) -> &crate::window_manager_law::AuthenticatedReplayTooltipOwner {
        &self.owner
    }

    pub(crate) fn hovered_surface(&self) -> &crate::authored_role_index::AuthenticatedPanelSurface {
        &self.hovered_surface
    }

    pub(crate) fn tooltip_surface(&self) -> &crate::authored_role_index::AuthenticatedPanelSurface {
        &self.tooltip_surface
    }

    pub(crate) fn cursor(&self) -> (i32, i32) {
        self.cursor
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AuthenticatedReplayTooltipRoute {
    Authored(TooltipFamilyFacts),
    NoDeclaration,
    SelectedDeclarationAbsent,
    Unsupported { mechanism: String },
}

const AUTHORED_TOOLTIP_PROPERTIES: [&str; 4] = [
    "pdx_tooltip",
    "pdx_tooltip_delayed",
    "pdx_disabled_tooltip",
    "pdx_disabled_tooltip_delayed",
];

const UNSUPPORTED_TOOLTIP_PROPERTIES: [&str; 5] = [
    "context_aware_tooltip",
    "bound_tooltip",
    "tooltip",
    "tooltiptext",
    "delayedtooltiptext",
];

fn has_authored_field(element: &GuiElement, key: &str) -> bool {
    element
        .raw
        .fields()
        .iter()
        .any(|entry| entry.key_matches(key))
}

fn classify_authenticated_tooltip_node(element: &GuiElement) -> Option<String> {
    for key in UNSUPPORTED_TOOLTIP_PROPERTIES {
        if has_authored_field(element, key) {
            return Some(key.to_string());
        }
    }
    for key in AUTHORED_TOOLTIP_PROPERTIES {
        if element
            .raw
            .fields()
            .iter()
            .filter(|entry| entry.key_matches(key))
            .any(|entry| entry.value.as_scalar().is_none())
        {
            return Some(format!("{key}:non-scalar"));
        }
    }
    let normal = has_authored_field(element, "pdx_tooltip");
    let normal_delayed = has_authored_field(element, "pdx_tooltip_delayed");
    let disabled = has_authored_field(element, "pdx_disabled_tooltip");
    let disabled_delayed = has_authored_field(element, "pdx_disabled_tooltip_delayed");
    if normal_delayed && !normal {
        return Some("pdx_tooltip_delayed:missing-primary".to_string());
    }
    if disabled_delayed && !disabled {
        return Some("pdx_disabled_tooltip_delayed:missing-primary".to_string());
    }
    None
}

fn node_has_any_tooltip_mechanism(element: &GuiElement) -> bool {
    AUTHORED_TOOLTIP_PROPERTIES
        .iter()
        .chain(UNSUPPORTED_TOOLTIP_PROPERTIES.iter())
        .any(|key| has_authored_field(element, key))
}

fn authenticated_alternate_selection_for_authored_node(
    element_type: &str,
    has_disabled_declaration: bool,
    pointer: crate::window_manager_law::ReplayElementPointerAuthority,
) -> crate::window_manager_law::ReplayTooltipAlternateSelection {
    use crate::window_manager_law::{
        ReplayElementPointerAuthority, ReplayTooltipAlternateSelection,
    };

    if !has_disabled_declaration {
        return ReplayTooltipAlternateSelection::AuthoredDefault;
    }
    if !WidgetFamily::from_element_type(element_type)
        .is_some_and(WidgetFamily::standard_button_carrier)
    {
        return ReplayTooltipAlternateSelection::UnavailableAuthoredAlternate;
    }
    match pointer {
        ReplayElementPointerAuthority::Interactive {
            visual_enabled: true,
        }
        | ReplayElementPointerAuthority::BlockOnly {
            visual_enabled: Some(true),
        } => ReplayTooltipAlternateSelection::AuthoredDefault,
        ReplayElementPointerAuthority::Interactive {
            visual_enabled: false,
        }
        | ReplayElementPointerAuthority::BlockOnly {
            visual_enabled: Some(false),
        } => ReplayTooltipAlternateSelection::AuthoredRuntimeAlternate,
        ReplayElementPointerAuthority::BlockOnly {
            visual_enabled: None,
        } => ReplayTooltipAlternateSelection::UnavailableAuthoredAlternate,
        ReplayElementPointerAuthority::PassThrough => {
            unreachable!("pass-through tooltip candidates were excluded above")
        }
    }
}

pub fn derive_authenticated_tooltip_bindings(
    surface: &crate::authored_role_index::AuthenticatedPanelSurface,
    pointer_bindings: &BTreeMap<String, crate::window_manager_law::ReplayElementPointerAuthority>,
) -> Result<(
    Vec<String>,
    BTreeMap<String, crate::window_manager_law::ReplayTooltipAlternateSelection>,
)> {
    use crate::window_manager_law::{
        ReplayElementPointerAuthority, ReplayTooltipAlternateSelection,
    };

    let mut targets = Vec::new();
    let mut selectors = BTreeMap::new();
    for role in surface.element_roles() {
        let Some(pointer) = pointer_bindings.get(&role.element_id).copied() else {
            continue;
        };
        if matches!(pointer, ReplayElementPointerAuthority::PassThrough) {
            continue;
        }
        let element = surface.authored_element(&role.child_path).ok_or_else(|| {
            GameUiError::missing(format!(
                "sealed XUI05 role {:?} has no exact authored element",
                role.element_id
            ))
        })?;
        if !node_has_any_tooltip_mechanism(element) {
            continue;
        }
        let selector = if UNSUPPORTED_TOOLTIP_PROPERTIES
            .iter()
            .any(|key| has_authored_field(element, key))
        {
            ReplayTooltipAlternateSelection::CustomProvider
        } else {
            let has_disabled = has_authored_field(element, "pdx_disabled_tooltip")
                || has_authored_field(element, "pdx_disabled_tooltip_delayed");
            authenticated_alternate_selection_for_authored_node(
                &element.element_type,
                has_disabled,
                pointer,
            )
        };
        targets.push(role.element_id.clone());
        if selectors
            .insert(role.element_id.clone(), selector)
            .is_some()
        {
            return Err(GameUiError::parse(format!(
                "sealed XUI05 surface repeats element id {:?}",
                role.element_id
            )));
        }
    }
    Ok((targets, selectors))
}

fn target_from_authenticated_owner(
    owner: &crate::window_manager_law::AuthenticatedReplayTooltipOwner,
    include_delayed: bool,
) -> AuthoredTooltipTarget {
    let alternate_tooltip_selected = match owner.alternate_selection() {
        crate::window_manager_law::ReplayTooltipAlternateSelection::AuthoredDefault => {
            RecordedFact::Value(false)
        }
        crate::window_manager_law::ReplayTooltipAlternateSelection::AuthoredRuntimeAlternate => {
            RecordedFact::Value(true)
        }
        crate::window_manager_law::ReplayTooltipAlternateSelection::CustomProvider => {
            RecordedFact::Unknown {
                reason: "custom provider has no authored ordinary/alternate token".into(),
            }
        }
        crate::window_manager_law::ReplayTooltipAlternateSelection::UnavailableAuthoredAlternate => {
            RecordedFact::Unknown {
                reason: "authenticated visual state does not select the authored ordinary/alternate tooltip".into(),
            }
        }
    };
    AuthoredTooltipTarget {
        gui_source_path: owner.source_file().to_string(),
        runtime_root_name: owner.surface_id().to_string(),
        child_path: owner.child_path().to_vec(),
        owner_child_path: None,
        element_type: owner.element_type().to_string(),
        element_name: owner.authored_name().to_string(),
        engine_instance_token: format!(
            "{}:{:016x}",
            owner.runtime_instance_token(),
            owner.engine_instance_token()
        ),
        paint_ordinal: owner.paint_ordinal(),
        alternate_tooltip_selected,
        include_delayed,
    }
}

pub(crate) fn resolve_authenticated_replay_tooltip_route(
    resolver: &AssetResolver,
    context: &AuthenticatedReplayTooltipContext,
    include_delayed: bool,
) -> Result<AuthenticatedReplayTooltipRoute> {
    let target = target_from_authenticated_owner(&context.owner, include_delayed);
    let authenticated = authenticate_authored_tooltip_target(resolver, &target)?;
    let current = authenticated
        .ancestry
        .last()
        .expect("authenticated GUI ancestry always contains its root");
    if matches!(
        context.owner.alternate_selection(),
        crate::window_manager_law::ReplayTooltipAlternateSelection::CustomProvider
    ) {
        let mechanism = UNSUPPORTED_TOOLTIP_PROPERTIES
            .iter()
            .find(|key| has_authored_field(current, key))
            .copied()
            .unwrap_or("custom-provider")
            .to_string();
        return Ok(AuthenticatedReplayTooltipRoute::Unsupported { mechanism });
    }
    if matches!(
        context.owner.alternate_selection(),
        crate::window_manager_law::ReplayTooltipAlternateSelection::UnavailableAuthoredAlternate
    ) {
        return Ok(AuthenticatedReplayTooltipRoute::Unsupported {
            mechanism: "authored-alternate-selector-unavailable".to_string(),
        });
    }
    if let Some(mechanism) = classify_authenticated_tooltip_node(current) {
        return Ok(AuthenticatedReplayTooltipRoute::Unsupported { mechanism });
    }
    if !AUTHORED_TOOLTIP_PROPERTIES
        .iter()
        .any(|key| has_authored_field(current, key))
    {
        return Ok(AuthenticatedReplayTooltipRoute::NoDeclaration);
    }
    Ok(
        match discover_authored_unscoped_tooltip(resolver, &target)? {
            Some(facts) => AuthenticatedReplayTooltipRoute::Authored(facts),
            None => AuthenticatedReplayTooltipRoute::SelectedDeclarationAbsent,
        },
    )
}

pub(crate) fn authenticated_tooltip_placement(
    resolver: &AssetResolver,
    context: &AuthenticatedReplayTooltipContext,
) -> Result<AuthoredTooltipPlacement> {
    discover_authored_tooltip_placement(
        resolver,
        &target_from_authenticated_owner(context.owner(), false),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthoredTooltipPlacement {
    pub owner: TooltipOwner,
    pub anchor: AuthoredTooltipAnchor,
}

struct AuthenticatedAuthoredTooltipTarget {
    source_file: String,
    ancestry: Vec<GuiElement>,
}

fn authenticate_authored_tooltip_target(
    resolver: &AssetResolver,
    target: &AuthoredTooltipTarget,
) -> Result<AuthenticatedAuthoredTooltipTarget> {
    if target.gui_source_path.trim().is_empty()
        || target.runtime_root_name.trim().is_empty()
        || target.element_type.trim().is_empty()
        || target.element_name.trim().is_empty()
        || target.engine_instance_token.trim().is_empty()
    {
        return Err(GameUiError::missing(
            "authenticated tooltip target has a blank effective-VFS/open-instance identity",
        ));
    }
    let resolved = resolver.resolve_file(&target.gui_source_path)?;
    let mut layouts = LayoutSet::default();
    layouts.parse_content(&resolver.read_file(&resolved)?, &resolved.rel_path)?;
    let document = layouts
        .documents
        .into_iter()
        .next()
        .ok_or_else(|| GameUiError::parse("effective GUI parse produced no document"))?;
    if document
        .decode_losses
        .as_ref()
        .is_some_and(|losses| losses.events != 0)
    {
        return Err(GameUiError::parse(format!(
            "{} destroyed bytes while decoding authored tooltip facts",
            resolved.rel_path
        )));
    }
    if !document.recoveries.is_empty() {
        return Err(GameUiError::parse(format!(
            "{} required GUI parse recovery while discovering authored tooltip facts",
            resolved.rel_path
        )));
    }

    let roots = document
        .roots
        .iter()
        .filter(|root| root.name.eq_ignore_ascii_case(&target.runtime_root_name))
        .collect::<Vec<_>>();
    let [root] = roots.as_slice() else {
        return Err(GameUiError::parse(format!(
            "effective GUI {} has {} roots named {:?}; authenticated tooltip discovery requires one",
            resolved.rel_path,
            roots.len(),
            target.runtime_root_name
        )));
    };

    let mut ancestry = vec![(**root).clone()];
    let mut current = *root;
    for (depth, raw_index) in target.child_path.iter().enumerate() {
        let index = usize::try_from(*raw_index)
            .map_err(|_| GameUiError::parse("authored GUI child index overflows usize"))?;
        current = current.children.get(index).ok_or_else(|| {
            GameUiError::parse(format!(
                "authenticated child path {:?} leaves {} at depth {depth}",
                target.child_path, resolved.rel_path
            ))
        })?;
        ancestry.push((*current).clone());
    }
    if !current
        .element_type
        .eq_ignore_ascii_case(&target.element_type)
        || !current.name.eq_ignore_ascii_case(&target.element_name)
    {
        return Err(GameUiError::parse(format!(
            "authenticated GUI target {:?}/{:?} disagrees with effective-VFS node {:?}/{:?}",
            target.element_type, target.element_name, current.element_type, current.name
        )));
    }
    Ok(AuthenticatedAuthoredTooltipTarget {
        source_file: resolved.rel_path,
        ancestry,
    })
}

pub(crate) fn discover_authored_tooltip_placement(
    resolver: &AssetResolver,
    target: &AuthoredTooltipTarget,
) -> Result<AuthoredTooltipPlacement> {
    let authenticated = authenticate_authored_tooltip_target(resolver, target)?;
    let current = authenticated
        .ancestry
        .last()
        .expect("authenticated GUI ancestry always contains its root");
    Ok(AuthoredTooltipPlacement {
        owner: TooltipOwner {
            source_file: authenticated.source_file.clone(),
            root_window: target.runtime_root_name.clone(),
            element_path: target.child_path.clone(),
            engine_instance_token: target.engine_instance_token.clone(),
        },
        anchor: parse_authored_tooltip_anchor(current, &authenticated.source_file)?,
    })
}

pub(crate) fn discover_authored_unscoped_tooltip(
    resolver: &AssetResolver,
    target: &AuthoredTooltipTarget,
) -> Result<Option<TooltipFamilyFacts>> {
    let authenticated = authenticate_authored_tooltip_target(resolver, target)?;
    let current = authenticated
        .ancestry
        .last()
        .expect("authenticated GUI ancestry always contains its root");
    let normal = one_scalar_declaration(current, "pdx_tooltip", &authenticated.source_file)?;
    let normal_delayed =
        one_scalar_declaration(current, "pdx_tooltip_delayed", &authenticated.source_file)?;
    let disabled =
        one_scalar_declaration(current, "pdx_disabled_tooltip", &authenticated.source_file)?;
    let disabled_delayed = one_scalar_declaration(
        current,
        "pdx_disabled_tooltip_delayed",
        &authenticated.source_file,
    )?;
    let alternate = if disabled.is_none() && disabled_delayed.is_none() {
        false
    } else {
        match &target.alternate_tooltip_selected {
            RecordedFact::Value(value) => *value,
            RecordedFact::Absent => {
                return Err(GameUiError::missing(
                    "a729 tooltip ordinary/alternate selector is absent",
                ))
            }
            RecordedFact::Null => {
                return Err(GameUiError::missing(
                    "a729 tooltip ordinary/alternate selector is null",
                ))
            }
            RecordedFact::Unknown { reason } => {
                return Err(GameUiError::missing(format!(
                    "a729 tooltip ordinary/alternate selector is unknown: {reason}"
                )))
            }
        }
    };
    let primary = if alternate {
        disabled
            .map(|key| ("pdx_disabled_tooltip", key))
            .or_else(|| normal.map(|key| ("pdx_tooltip", key)))
    } else {
        normal.map(|key| ("pdx_tooltip", key))
    };
    let delayed = if alternate {
        disabled_delayed
            .map(|key| ("pdx_disabled_tooltip_delayed", key))
            .or_else(|| normal_delayed.map(|key| ("pdx_tooltip_delayed", key)))
    } else {
        normal_delayed.map(|key| ("pdx_tooltip_delayed", key))
    };
    let Some((primary_property, localisation_key)) = primary else {
        return Ok(None);
    };

    let element_path = authenticated
        .ancestry
        .iter()
        .map(|element| element.name.as_str())
        .collect::<Vec<_>>()
        .join("/");
    let has_scoped_evaluation = localisation_key.contains('$')
        || localisation_key.contains('[')
        || localisation_key.contains(']')
        || delayed
            .as_ref()
            .is_some_and(|(_, key)| key.contains('$') || key.contains('[') || key.contains(']'));
    let (delayed_property_name, delayed_localisation_key) = delayed
        .map_or((None, None), |(property, key)| {
            (Some(property.to_string()), Some(key.to_string()))
        });
    let authored_anchor = parse_authored_tooltip_anchor(current, &authenticated.source_file)?;
    let tooltip_root_name = authored_tooltip_shell_name(resolver)?;

    Ok(Some(TooltipFamilyFacts::AuthoredUnscoped(
        AuthoredUnscopedTooltipFacts {
            owner: TooltipOwner {
                source_file: authenticated.source_file.clone(),
                root_window: target.runtime_root_name.clone(),
                element_path: target
                    .owner_child_path
                    .clone()
                    .unwrap_or_else(|| target.child_path.clone()),
                engine_instance_token: target.engine_instance_token.clone(),
            },
            tooltip_root_name,
            source_ordinal: target.paint_ordinal,
            gui_source_path: authenticated.source_file.clone(),
            element_path,
            localisation_key: RecordedFact::Value(localisation_key.to_string()),
            localisation_property_name: primary_property.to_string(),
            delayed_localisation_key,
            delayed_property_name,
            authored_anchor,
            include_delayed: target.include_delayed,
            has_scoped_evaluation,
        },
    )))
}

fn parse_authored_tooltip_anchor(
    element: &GuiElement,
    source_file: &str,
) -> Result<AuthoredTooltipAnchor> {
    let anchors = element
        .raw
        .values_ci("pdx_tooltip_anchor")
        .collect::<Vec<_>>();
    if anchors.is_empty() {
        return Ok(AuthoredTooltipAnchor::Cursor);
    }
    let [anchor] = anchors.as_slice() else {
        return Err(GameUiError::parse(format!(
            "{source_file} node {:?} declares pdx_tooltip_anchor {} times",
            element.name,
            anchors.len()
        )));
    };
    let x = anchor
        .first_value_ci("x")
        .and_then(crate::paradox_text::scalar_i32)
        .ok_or_else(|| {
            GameUiError::missing(format!(
                "{source_file} node {:?} tooltip anchor has no exact x",
                element.name
            ))
        })?;
    let y = anchor
        .first_value_ci("y")
        .and_then(crate::paradox_text::scalar_i32)
        .ok_or_else(|| {
            GameUiError::missing(format!(
                "{source_file} node {:?} tooltip anchor has no exact y",
                element.name
            ))
        })?;
    let relative_to_owner = match anchor
        .first_scalar_ci("relative")
        .map(|raw| raw.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("yes" | "true" | "1") => true,
        Some("no" | "false" | "0") => false,
        Some(raw) => {
            return Err(GameUiError::parse(format!(
                "{source_file} node {:?} has invalid tooltip anchor relative={raw:?}",
                element.name
            )))
        }
        None => {
            return Err(GameUiError::missing(format!(
                "{source_file} node {:?} tooltip anchor omits relative; replay will not guess the parser default",
                element.name
            )))
        }
    };
    Ok(AuthoredTooltipAnchor::Packed {
        position: (x, y),
        relative_to_owner,
    })
}

fn one_scalar_declaration<'a>(
    element: &'a GuiElement,
    key: &str,
    source_file: &str,
) -> Result<Option<&'a str>> {
    let values = scalar_declarations(element, key);
    if values.len() > 1 {
        return Err(GameUiError::parse(format!(
            "{source_file} node {:?} declares {key} {} times",
            element.name,
            values.len()
        )));
    }
    Ok(values.first().copied())
}

fn scalar_declarations<'a>(element: &'a GuiElement, key: &str) -> Vec<&'a str> {
    element
        .raw
        .fields()
        .iter()
        .filter(|entry| entry.key_matches(key))
        .filter_map(|entry| entry.value.as_scalar())
        .collect()
}

pub fn authored_tooltip_shell_name(resolver: &AssetResolver) -> Result<String> {
    let resolved = resolver.resolve_file("interface/core.gui")?;
    let mut layouts = LayoutSet::default();
    layouts.parse_content(&resolver.read_file(&resolved)?, &resolved.rel_path)?;
    let document = layouts
        .documents
        .into_iter()
        .next()
        .ok_or_else(|| GameUiError::parse("effective core.gui parse produced no document"))?;
    if document
        .decode_losses
        .as_ref()
        .is_some_and(|losses| losses.events != 0)
        || !document.recoveries.is_empty()
    {
        return Err(GameUiError::parse(
            "effective core.gui is damaged; tooltip shell identity is unauthenticated",
        ));
    }
    let candidates = document
        .roots
        .iter()
        .filter(|element| {
            element.element_type.eq_ignore_ascii_case("textBoxType")
                && element.raw.first_scalar_ci("textureFile").is_some()
                && element.raw.first_value_ci("borderSize").is_some()
                && element.raw.first_scalar_ci("font").is_some()
                && element.raw.first_scalar_ci("maxWidth").is_some()
                && element.raw.first_scalar_ci("maxHeight").is_some()
        })
        .collect::<Vec<_>>();
    let named_tooltips = candidates
        .iter()
        .copied()
        .filter(|candidate| {
            candidate
                .name
                .eq_ignore_ascii_case(crate::hud::TOOLTIP_WINDOW_NAME)
        })
        .collect::<Vec<_>>();
    let candidate = match (named_tooltips.as_slice(), candidates.as_slice()) {
        ([candidate], _) => *candidate,
        ([], [candidate]) => *candidate,
        _ => {
            return Err(GameUiError::parse(format!(
                "effective core.gui has {} fully-authored textBox candidates and {} named ToolTip; expected exactly one named ToolTip or one unambiguous renamed shell",
                candidates.len(),
                named_tooltips.len()
            )));
        }
    };
    Ok(candidate.name.clone())
}

pub fn load_authored_tooltip_clamp_offsets(
    resolver: &AssetResolver,
) -> Result<(AuthoredClampOffsets, Vec<String>)> {
    let (values, sources) = resolver.effective_define_numbers("NGraphics", &[
        "TOOLTIP_SCREEN_LEFT_OFFSET_X", "TOOLTIP_SCREEN_RIGHT_OFFSET_X", "TOOLTIP_SCREEN_TOP_OFFSET_Y", "TOOLTIP_SCREEN_BOTTOM_OFFSET_Y"
    ])?;
    let integer = |value: f64| -> Result<i32> {
        if !value.is_finite() || value.fract() != 0.0 || value < i32::MIN as f64 || value > i32::MAX as f64 { return Err(GameUiError::parse("invalid tooltip clamp integer")); }
        Ok(value as i32)
    };
    Ok((AuthoredClampOffsets { left: integer(values[0])?, right: integer(values[1])?, top: integer(values[2])?, bottom: integer(values[3])? }, sources))
}

pub fn load_authored_tooltip_timing(
    resolver: &AssetResolver,
) -> Result<(TooltipTimingLaw, Vec<String>)> {
    let (values, sources) = resolver.effective_define_numbers("NGraphics", &["TOOLTIP_SHOW_DELAY", "TOOLTIP_HIDE_DELAY", "TOOLTIP_DELAYED_DELAY"])?;
    let micros = |key: &str, seconds: f64| -> Result<u64> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(GameUiError::parse(format!("invalid tooltip delay {key}")));
        }
        let value = seconds * 1_000_000.0;
        if !value.is_finite() || value >= u64::MAX as f64 {
            return Err(GameUiError::parse(format!("{key} microseconds overflow u64")));
        }
        if value.fract() != 0.0 {
            return Err(GameUiError::parse(format!("{key} has sub-microsecond precision")));
        }
        Ok(value as u64)
    };
    Ok((TooltipTimingLaw {
        show_delay_us: micros("TOOLTIP_SHOW_DELAY", values[0])?,
        hide_delay_us: micros("TOOLTIP_HIDE_DELAY", values[1])?,
        delayed_delay_us: micros("TOOLTIP_DELAYED_DELAY", values[2])?,
    }, sources))
}





#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TooltipFamilyFacts {
    AuthoredUnscoped(AuthoredUnscopedTooltipFacts),
    ExactSemanticPayload(TooltipPayload),
    ProvinceSupplyRemaining,
    ProvinceSupplyAccumulatedHubFlow,
    ImpassableMapToken,
}

impl TooltipFamilyFacts {
    pub(crate) fn owner(&self) -> &TooltipOwner {
        match self {
            Self::AuthoredUnscoped(facts) => &facts.owner,
            Self::ExactSemanticPayload(payload) => payload.owner(),
            Self::ProvinceSupplyRemaining
            | Self::ProvinceSupplyAccumulatedHubFlow
            | Self::ImpassableMapToken => unreachable!(
                "unsupported tooltip families never cross the authenticated product route"
            ),
        }
    }

    pub(crate) fn has_delayed_branch(&self) -> bool {
        matches!(
            self,
            Self::AuthoredUnscoped(AuthoredUnscopedTooltipFacts {
                delayed_localisation_key: Some(_),
                ..
            })
        )
    }

    pub(crate) fn lifecycle_identity(&self) -> String {
        match self {
            Self::AuthoredUnscoped(facts) => format!(
                "authored:{}:{:?}:{}:{:?}:{:?}",
                facts.gui_source_path,
                facts.owner.element_path(),
                facts.localisation_property_name,
                facts.localisation_key,
                (
                    facts.delayed_property_name.as_deref(),
                    facts.delayed_localisation_key.as_deref()
                )
            ),
            Self::ExactSemanticPayload(payload) => format!(
                "semantic:{:?}:{}",
                payload.owner().element_path(),
                payload.rows().len()
            ),
            Self::ProvinceSupplyRemaining => "unsupported:province-supply-remaining".into(),
            Self::ProvinceSupplyAccumulatedHubFlow => {
                "unsupported:province-supply-accumulated-hub-flow".into()
            }
            Self::ImpassableMapToken => "unsupported:impassable-map-token".into(),
        }
    }

    pub(crate) fn authored_anchor(&self) -> AuthoredTooltipAnchor {
        match self {
            Self::AuthoredUnscoped(facts) => facts.authored_anchor,
            Self::ExactSemanticPayload(_) => AuthoredTooltipAnchor::Cursor,
            Self::ProvinceSupplyRemaining
            | Self::ProvinceSupplyAccumulatedHubFlow
            | Self::ImpassableMapToken => unreachable!(
                "unsupported tooltip families never cross the authenticated product route"
            ),
        }
    }
}

pub(crate) fn compose_payload_from_facts(
    facts: TooltipFamilyFacts,
    localisation: &Localisation,
) -> std::result::Result<TooltipPayload, TooltipRefusal> {
    match facts {
        TooltipFamilyFacts::ExactSemanticPayload(payload) => {
            validate_payload_trace(&payload)?;
            for row in payload.rows.iter().filter(|row| {
                matches!(
                    row.availability,
                    RowAvailability::Resolved | RowAvailability::MeasuredEmpty
                )
            }) {
                let packet_sources = row
                    .admitted_sources
                    .iter()
                    .filter_map(|source| match source {
                        AdmittedTooltipSource::PacketSemantic {
                            packet_id,
                            producer_identity,
                            field,
                            manifest_sha256,
                        } => Some((
                            packet_id,
                            producer_identity,
                            field,
                            manifest_sha256,
                        )),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if packet_sources.len() != 1 {
                    return Err(TooltipRefusal::UntraceableRow {
                        row: row.source_identity.clone(),
                    });
                }
                let (packet, producer, field, manifest) = packet_sources[0];
                if packet.trim().is_empty()
                    || producer.trim().is_empty()
                    || field.trim().is_empty()
                    || !is_lowercase_sha256(manifest)
                {
                    return Err(TooltipRefusal::InvalidPayload {
                        reason: format!(
                            "tooltip row {:?} has an invalid packet semantic identity",
                            row.source_identity
                        ),
                    });
                }
            }
            payload
                .validate()
                .map_err(|error| TooltipRefusal::InvalidPayload {
                    reason: error.to_string(),
                })?;
            Ok(payload)
        }
        TooltipFamilyFacts::AuthoredUnscoped(facts) => {
            let failures = localisation
                .parse_failures
                .iter()
                .chain(localisation.decode_lossy_files.iter())
                .cloned()
                .collect::<Vec<_>>();
            if !failures.is_empty() {
                return Err(TooltipRefusal::DamagedLocalisationCorpus { failures });
            }
            if facts.has_scoped_evaluation {
                return Err(TooltipRefusal::MissingLeaf {
                    family: "authored GUI tooltip".into(),
                    leaf: "scoped localisation evaluation context".into(),
                });
            }
            let primary_key =
                require_fact("authored tooltip localisation key", facts.localisation_key)?;
            if facts.localisation_property_name.trim().is_empty() {
                return Err(TooltipRefusal::InvalidPayload {
                    reason: "authored tooltip primary property name is blank".into(),
                });
            }
            if !matches!(
                facts.localisation_property_name.as_str(),
                "pdx_tooltip" | "pdx_disabled_tooltip"
            ) {
                return Err(TooltipRefusal::InvalidPayload {
                    reason: format!(
                        "unauthenticated authored tooltip property {:?}",
                        facts.localisation_property_name
                    ),
                });
            }
            if facts.delayed_localisation_key.is_some() != facts.delayed_property_name.is_some() {
                return Err(TooltipRefusal::InvalidPayload {
                    reason: "authored delayed tooltip property/key pairing is incomplete".into(),
                });
            }
            if let Some(property) = facts.delayed_property_name.as_deref() {
                let valid = property == "pdx_tooltip_delayed"
                    || (facts.localisation_property_name == "pdx_disabled_tooltip"
                        && property == "pdx_disabled_tooltip_delayed");
                if !valid {
                    return Err(TooltipRefusal::InvalidPayload {
                        reason: format!(
                            "delayed authored property {property:?} is inconsistent with primary {:?}",
                            facts.localisation_property_name
                        ),
                    });
                }
            }
            let mut keys = vec![(
                TooltipLineKind::Description,
                facts.localisation_property_name,
                primary_key,
            )];
            if facts.include_delayed {
                if let Some((property, delayed)) =
                    facts.delayed_property_name.zip(facts.delayed_localisation_key)
                {
                    keys.push((TooltipLineKind::Description, property, delayed));
                }
            }
            let mut rows = Vec::with_capacity(keys.len());
            for (row_index, (line_kind, property, key)) in keys.into_iter().enumerate() {
                if key.trim().is_empty() {
                    return Err(TooltipRefusal::InternalIdentityAsDisplayName { identity: key });
                }
                let resolved = resolve_localisation_preserving_inline_icons(
                    &localisation.entries,
                    &key,
                    &RuntimeTextContext::default(),
                );
                if !resolved.issues.is_empty() {
                    if resolved.issues.len() == 1
                        && matches!(
                            &resolved.issues[0].kind,
                            crate::text_law::TextIssueKind::MissingLocalisationKey
                        )
                    {
                        return Err(TooltipRefusal::MissingLocalisationKey {
                            key: resolved.issues[0].token.clone(),
                        });
                    }
                    return Err(TooltipRefusal::TextEvaluationIssues {
                        key,
                        issues: resolved.issues,
                    });
                }
                let text = resolved.text;
                if text.is_empty() {
                    return Err(TooltipRefusal::MissingLocalisationKey { key });
                }
                let localisation_sources = resolved
                    .source_keys
                    .into_iter()
                    .map(|key| {
                        if let Some(sources) = localisation.ambiguous_sources_for_key(&key) {
                            return Err(TooltipRefusal::UntraceableRow {
                                row: format!(
                                    "{} localisation key {key} has ambiguous definitions in {:?}",
                                    facts.element_path, sources
                                ),
                            });
                        }
                        let source = localisation.source_file_for_key(&key).ok_or_else(|| {
                            TooltipRefusal::UntraceableRow {
                                row: format!(
                                    "{} localisation key {key} has no unique admitted definition",
                                    facts.element_path
                                ),
                            }
                        })?;
                        Ok(AdmittedTooltipSource::EffectiveLocalisation {
                            key,
                            source_files: vec![source.to_string()],
                        })
                    })
                    .collect::<std::result::Result<Vec<_>, TooltipRefusal>>()?;
                rows.push(TooltipRow {
                    source_ordinal: facts
                        .source_ordinal
                        .checked_add(u32::try_from(row_index).map_err(|_| {
                            TooltipRefusal::ArithmeticOverflow {
                                operation: "authored tooltip row ordinal conversion".into(),
                            }
                        })?)
                        .ok_or_else(|| TooltipRefusal::ArithmeticOverflow {
                            operation: "authored tooltip row ordinal".into(),
                        })?,
                    line_kind,
                    source_identity: format!("{}/{}", facts.element_path, property),
                    evaluation_context: if row_index == 0 {
                        "unscoped authored tooltip".into()
                    } else {
                        "unscoped authored delayed tooltip".into()
                    },
                    availability: RowAvailability::Resolved,
                    text,
                    admitted_sources: std::iter::once(AdmittedTooltipSource::EffectiveGui {
                        game_relative_path: facts.gui_source_path.clone(),
                        element_path: facts.element_path.clone(),
                    })
                    .chain(localisation_sources)
                    .collect(),
                });
            }
            let payload = TooltipPayload {
                owner: facts.owner,
                tooltip_root_name: facts.tooltip_root_name,
                rows,
            };
            validate_payload_trace(&payload)?;
            payload
                .validate()
                .map_err(|error| TooltipRefusal::InvalidPayload {
                    reason: error.to_string(),
                })?;
            Ok(payload)
        }
        TooltipFamilyFacts::ProvinceSupplyRemaining => Err(TooltipRefusal::MissingLeaf {
            family: "province supply remaining".into(),
            leaf: "passively acquired remaining-supply scalar for every province and every viewer-selectable observer country".into(),
        }),
        TooltipFamilyFacts::ProvinceSupplyAccumulatedHubFlow => {
            Err(TooltipRefusal::MissingLeaf {
                family: "province supply accumulated hub-flow".into(),
                leaf: "passively acquired incoming hub-flow facts for every province and every viewer-selectable observer country; hover-gated recorder-POV observations do not close this port".into(),
            })
        }
        TooltipFamilyFacts::ImpassableMapToken => Err(TooltipRefusal::MissingLeaf {
            family: "impassable map token".into(),
            leaf: "effective localized display value for exact producer token IMPASSABLE".into(),
        }),
    }
}

pub fn validate_payload_trace(payload: &TooltipPayload) -> std::result::Result<(), TooltipRefusal> {
    for row in &payload.rows {
        if matches!(
            row.availability,
            RowAvailability::Resolved | RowAvailability::MeasuredEmpty
        ) && row.admitted_sources.is_empty()
        {
            return Err(TooltipRefusal::UntraceableRow {
                row: row.source_identity.clone(),
            });
        }
        if row.admitted_sources.iter().any(|source| {
            matches!(
                source,
                AdmittedTooltipSource::EffectiveLocalisation {
                    source_files,
                    ..
                } if source_files.is_empty()
            )
        }) {
            return Err(TooltipRefusal::UntraceableRow {
                row: row.source_identity.clone(),
            });
        }
    }
    Ok(())
}

pub fn refuse_rows_requiring_wrap(
    payload: &TooltipPayload,
    authored_max_width: usize,
    authored_border_x: usize,
    measure: impl Fn(&str) -> i32,
) -> std::result::Result<i32, TooltipRefusal> {
    let doubled_border =
        authored_border_x
            .checked_mul(2)
            .ok_or_else(|| TooltipRefusal::ArithmeticOverflow {
                operation: "2 * authored tooltip border_x".into(),
            })?;
    let interior = authored_max_width
        .checked_sub(doubled_border)
        .ok_or_else(|| TooltipRefusal::InvalidAuthoredQuantity {
            quantity: "maxWidth - 2*border_x".into(),
            value: format!("{authored_max_width} - 2*{authored_border_x}"),
        })?;
    let interior =
        i32::try_from(interior).map_err(|_| TooltipRefusal::InvalidAuthoredQuantity {
            quantity: "tooltip interior width".into(),
            value: interior.to_string(),
        })?;
    if interior <= 0 {
        return Err(TooltipRefusal::InvalidAuthoredQuantity {
            quantity: "tooltip interior width".into(),
            value: interior.to_string(),
        });
    }
    for row in &payload.rows {
        if !matches!(row.availability, RowAvailability::Resolved) {
            continue;
        }
        let measured_width = measure(&row.text);
        if measured_width > interior {
            return Err(TooltipRefusal::RowRequiresWrap {
                row: row.source_identity.clone(),
                measured_width,
                interior_width: interior,
            });
        }
    }
    Ok(interior)
}

pub fn refuse_rows_requiring_wrap_from_authored(
    payload: &TooltipPayload,
    authored_max_width: RecordedFact<usize>,
    authored_border_x: RecordedFact<usize>,
    measure: impl Fn(&str) -> i32,
) -> std::result::Result<i32, TooltipRefusal> {
    let max_width = require_fact("authored tooltip maxWidth", authored_max_width)?;
    let border_x = require_fact("authored tooltip borderSize.x", authored_border_x)?;
    refuse_rows_requiring_wrap(payload, max_width, border_x, measure)
}

const CURSOR_ANCHOR_OFFSET_X: i32 = 0x15;
const CURSOR_ANCHOR_OFFSET_Y: i32 = 0x18;
const OVERFLOW_FLIP_GAP: i32 = 0x14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthoredClampOffsets {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipAnchor {
    Cursor,
    AbsolutePacked {
        position: (i32, i32),
    },
    OwnerRelativePacked {
        packed: (i32, i32),
        owner_screen_position: (i32, i32),
    },
    StickyRetainedPosition {
        position: (i32, i32),
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerCursorPlacement {
    pub raw_cursor_px: (i32, i32),
    pub gui_scale_f32_bits: (u32, u32),
    pub anchor: TooltipAnchor,
    pub viewport: (i32, i32),
    pub tooltip_extent: (i32, i32),
    pub clamp: AuthoredClampOffsets,
}

pub fn place_under_viewer_cursor(
    input: ViewerCursorPlacement,
) -> std::result::Result<ResolvedPlacement, TooltipRefusal> {
    let scale_x = f32::from_bits(input.gui_scale_f32_bits.0);
    let scale_y = f32::from_bits(input.gui_scale_f32_bits.1);
    if !scale_x.is_finite() || !scale_y.is_finite() || scale_x <= 0.0 || scale_y <= 0.0 {
        return Err(TooltipRefusal::InvalidViewerQuantity {
            quantity: "viewer GUI scale".into(),
            value: format!("({scale_x}, {scale_y})"),
        });
    }
    let derived_ui = (
        transform_cursor_axis(input.raw_cursor_px.0, scale_x, "x")?,
        transform_cursor_axis(input.raw_cursor_px.1, scale_y, "y")?,
    );
    let (width, height) = input.tooltip_extent;
    let (screen_width, screen_height) = input.viewport;
    if width <= 0 || height <= 0 || screen_width <= 0 || screen_height <= 0 {
        return Err(TooltipRefusal::InvalidViewerQuantity {
            quantity: "tooltip/viewport extent".into(),
            value: format!("{:?}/{:?}", input.tooltip_extent, input.viewport),
        });
    }
    let evidence = match input.anchor {
        TooltipAnchor::Cursor => PlacementEvidence::ViewerCursor {
            raw_px: input.raw_cursor_px,
            ui_px: derived_ui,
            gui_scale_f32_bits: input.gui_scale_f32_bits,
        },
        TooltipAnchor::AbsolutePacked { position } => PlacementEvidence::AuthoredAbsoluteAnchor {
            packed_position: position,
        },
        TooltipAnchor::OwnerRelativePacked {
            packed,
            owner_screen_position,
        } => PlacementEvidence::AuthoredOwnerRelativeAnchor {
            packed_position: packed,
            owner_screen_position,
        },
        TooltipAnchor::StickyRetainedPosition { position } => {
            PlacementEvidence::ViewerControllerRetained { position }
        }
    };
    let (mut x, mut y, cursor_anchored) = match input.anchor {
        TooltipAnchor::Cursor => (
            derived_ui
                .0
                .checked_add(CURSOR_ANCHOR_OFFSET_X)
                .ok_or_else(|| TooltipRefusal::ArithmeticOverflow {
                    operation: "cursor x anchor".into(),
                })?,
            derived_ui
                .1
                .checked_add(CURSOR_ANCHOR_OFFSET_Y)
                .ok_or_else(|| TooltipRefusal::ArithmeticOverflow {
                    operation: "cursor y anchor".into(),
                })?,
            true,
        ),
        TooltipAnchor::AbsolutePacked { position } => (position.0, position.1, false),
        TooltipAnchor::StickyRetainedPosition { position } => (position.0, position.1, false),
        TooltipAnchor::OwnerRelativePacked {
            packed,
            owner_screen_position,
        } => (
            packed
                .0
                .checked_add(owner_screen_position.0)
                .ok_or_else(|| TooltipRefusal::ArithmeticOverflow {
                    operation: "owner-relative x anchor".into(),
                })?,
            packed
                .1
                .checked_add(owner_screen_position.1)
                .ok_or_else(|| TooltipRefusal::ArithmeticOverflow {
                    operation: "owner-relative y anchor".into(),
                })?,
            false,
        ),
    };
    if x < input.clamp.left {
        x = input.clamp.left;
    }
    let right_limit = screen_width
        .checked_sub(width)
        .and_then(|value| value.checked_sub(input.clamp.right))
        .ok_or_else(|| TooltipRefusal::ArithmeticOverflow {
            operation: "tooltip right clamp limit".into(),
        })?;
    if right_limit < x {
        x = if cursor_anchored {
            derived_ui
                .0
                .checked_sub(width)
                .and_then(|value| value.checked_sub(OVERFLOW_FLIP_GAP))
        } else {
            x.checked_sub(OVERFLOW_FLIP_GAP)
                .and_then(|value| value.checked_sub(width))
        }
        .ok_or_else(|| TooltipRefusal::ArithmeticOverflow {
            operation: "tooltip right-edge response".into(),
        })?;
    }
    if y < input.clamp.top {
        y = input.clamp.top;
    }
    let bottom_limit = screen_height
        .checked_sub(height)
        .and_then(|value| value.checked_sub(input.clamp.bottom))
        .ok_or_else(|| TooltipRefusal::ArithmeticOverflow {
            operation: "tooltip bottom clamp limit".into(),
        })?;
    if bottom_limit < y {
        y = if cursor_anchored {
            derived_ui
                .1
                .checked_sub(height)
                .and_then(|value| value.checked_sub(OVERFLOW_FLIP_GAP))
        } else {
            y.checked_sub(OVERFLOW_FLIP_GAP)
                .and_then(|value| value.checked_sub(height))
        }
        .ok_or_else(|| TooltipRefusal::ArithmeticOverflow {
            operation: "tooltip bottom-edge response".into(),
        })?;
    }
    x = x.max(0);
    y = y.max(0);
    Ok(ResolvedPlacement {
        screen_rect: ScreenRect::new(x, y, width, height),
        evidence,
    })
}

fn transform_cursor_axis(
    raw_px: i32,
    scale: f32,
    axis: &str,
) -> std::result::Result<i32, TooltipRefusal> {
    let product = (raw_px as f32) * scale;
    let exclusive_upper = -(i32::MIN as f32);
    if !product.is_finite() || product < i32::MIN as f32 || product >= exclusive_upper {
        return Err(TooltipRefusal::InvalidViewerQuantity {
            quantity: format!("viewer GUI transform {axis}"),
            value: format!("{raw_px} * {scale}"),
        });
    }
    Ok(product as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modvfs_tooltip_laws_evaluate_mod_scripts_and_report_every_source() {
        let base = unique_discovery_root("modvfs-tooltip-base");
        let mod_root = unique_discovery_root("modvfs-tooltip-mod");
        for root in [&base, &mod_root] {
            std::fs::create_dir_all(root.join("common/defines")).unwrap();
        }
        std::fs::write(base.join("common/defines/zz_base.lua"),
            "NDefines = { NGraphics = { TOOLTIP_SHOW_DELAY = 0.125, TOOLTIP_HIDE_DELAY = 0.2, TOOLTIP_DELAYED_DELAY = 0.5, TOOLTIP_SCREEN_LEFT_OFFSET_X = 1, TOOLTIP_SCREEN_RIGHT_OFFSET_X = 2, TOOLTIP_SCREEN_TOP_OFFSET_Y = 3, TOOLTIP_SCREEN_BOTTOM_OFFSET_Y = 4 } }").unwrap();
        std::fs::write(mod_root.join("common/defines/01_mod.lua"),
            "NDefines.NGraphics.TOOLTIP_SHOW_DELAY = NDefines.NGraphics.TOOLTIP_SHOW_DELAY * 3\nNDefines.NGraphics.TOOLTIP_SCREEN_LEFT_OFFSET_X = 11\n").unwrap();
        let resolver = AssetResolver::new(vec![mod_root.clone(), base.clone()]);
        let (timing, sources) = load_authored_tooltip_timing(&resolver).unwrap();
        assert_eq!(timing.show_delay_us, 375_000);
        let candidate = HoverCandidate { owner: owner("mod"), route_identity: "mod:tooltip".into(), delayed: false };
        let mut life = TooltipLifecycle::new(timing);
        life.advance(Some(candidate.clone()), 0);
        life.advance(Some(candidate.clone()), 374_999);
        assert_eq!(life.visible_owner(), None);
        life.advance(Some(candidate), 1);
        assert_eq!(life.visible_owner(), Some(&owner("mod")));
        assert_eq!(timing.hide_delay_us, 200_000);
        assert_eq!(timing.delayed_delay_us, 500_000);
        assert_eq!(sources, vec!["common/defines/zz_base.lua", "common/defines/01_mod.lua"]);
        let (clamp, clamp_sources) = load_authored_tooltip_clamp_offsets(&resolver).unwrap();
        assert_eq!((clamp.left, clamp.right, clamp.top, clamp.bottom), (11, 2, 3, 4));
        assert_eq!(clamp_sources, sources);
        portable_test_fs_retry::remove_dir_all(base).unwrap();
        portable_test_fs_retry::remove_dir_all(mod_root).unwrap();
    }
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_discovery_root(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "battle-plan-tooltip-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn owner(name: &str) -> TooltipOwner {
        TooltipOwner {
            source_file: "interface/test.gui".into(),
            root_window: "test_root".into(),
            element_path: name.bytes().map(u32::from).collect(),
            engine_instance_token: "test-window/1".into(),
        }
    }
    fn timing() -> TooltipTimingLaw {
        TooltipTimingLaw {
            show_delay_us: 50_000,
            hide_delay_us: 50_000,
            delayed_delay_us: 1_000_000,
        }
    }

    #[test]
    fn non_button_block_only_state_cannot_select_an_authored_alternate() {
        use crate::window_manager_law::{
            ReplayElementPointerAuthority, ReplayTooltipAlternateSelection,
        };

        for visual_enabled in [Some(true), Some(false)] {
            assert_eq!(
                authenticated_alternate_selection_for_authored_node(
                    "iconType",
                    true,
                    ReplayElementPointerAuthority::BlockOnly { visual_enabled },
                ),
                ReplayTooltipAlternateSelection::UnavailableAuthoredAlternate,
                "generic BlockOnly visual state is not the exact CButton selector"
            );
        }
        assert_eq!(
            authenticated_alternate_selection_for_authored_node(
                "buttonType",
                true,
                ReplayElementPointerAuthority::BlockOnly {
                    visual_enabled: Some(false),
                },
            ),
            ReplayTooltipAlternateSelection::AuthoredRuntimeAlternate,
            "the exact standard-button carrier retains its authenticated alternate join"
        );
        assert_eq!(
            authenticated_alternate_selection_for_authored_node(
                "iconType",
                false,
                ReplayElementPointerAuthority::BlockOnly {
                    visual_enabled: Some(false),
                },
            ),
            ReplayTooltipAlternateSelection::AuthoredDefault,
            "a node with no authored alternate does not require an alternate selector"
        );
    }

    fn trace(name: &str) -> Vec<AdmittedTooltipSource> {
        vec![AdmittedTooltipSource::RecordedState {
            domain: "test".into(),
            field: name.into(),
            identity: "fixture".into(),
        }]
    }

    #[test]
    fn authored_tooltip_shell_selects_named_engine_root_among_fully_authored_decoys() {
        let root = unique_discovery_root("named-shell");
        std::fs::create_dir_all(root.join("interface")).expect("create interface fixture");
        std::fs::write(
            root.join("interface/core.gui"),
            "guiTypes = { \
             textBoxType = { name = ChatMemberText textureFile = chat.dds font = chat \
             borderSize = { x = 1 y = 1 } maxWidth = 300 maxHeight = 22 } \
             textBoxType = { name = ToolTip textureFile = tip.dds font = tip \
             borderSize = { x = 18 y = 18 } maxWidth = 400 maxHeight = 256 } \
             textBoxType = { name = FPS_Counter textureFile = fps.dds font = fps \
             borderSize = { x = 32 y = 32 } maxWidth = 128 maxHeight = 64 } }",
        )
        .expect("write effective core GUI");
        let resolver = AssetResolver::new(vec![root.clone()]);
        assert_eq!(
            authored_tooltip_shell_name(&resolver).expect("select engine tooltip shell"),
            "ToolTip"
        );
        portable_test_fs_retry::remove_dir_all(root).expect("remove shell fixture");
    }

    #[test]
    fn effective_vfs_discovery_owns_authored_key_path_shell_and_clamps() {
        let base = unique_discovery_root("base");
        let winning_mod = unique_discovery_root("mod");
        for root in [&base, &winning_mod] {
            std::fs::create_dir_all(root.join("interface")).expect("create interface fixture");
            std::fs::create_dir_all(root.join("common/defines")).expect("create defines fixture");
        }
        std::fs::write(
            base.join("interface/hover.gui"),
            "guiTypes = { containerWindowType = { name = root \
             buttonType = { name = target pdx_tooltip = BASE_KEY } } }",
        )
        .expect("write shadowed base GUI");
        std::fs::write(
            winning_mod.join("interface/hover.gui"),
            "guiTypes = { containerWindowType = { name = root \
             buttonType = { name = target pdx_tooltip = MOD_KEY \
             pdx_tooltip_delayed = MOD_DELAY \
             pdx_disabled_tooltip = MOD_DISABLED \
             pdx_disabled_tooltip_delayed = MOD_DISABLED_DELAY \
             pdx_tooltip_anchor = { x = -310 y = 0 relative = yes } \
             iconType = { name = child } } } }",
        )
        .expect("write effective GUI winner");
        std::fs::write(
            winning_mod.join("interface/core.gui"),
            "guiTypes = { textBoxType = { name = RenamedTip \
             textureFile = \"gfx/interface/tip.dds\" font = \"tip_font\" \
             borderSize = { x = 7 y = 9 } maxWidth = 320 maxHeight = 180 } }",
        )
        .expect("write effective tooltip shell");
        std::fs::write(
            winning_mod.join("common/defines/00_graphics.lua"),
            "NDefines = { NGraphics = {} }\nNDefines.NGraphics.TOOLTIP_SCREEN_LEFT_OFFSET_X = 11\n\
             NDefines.NGraphics.TOOLTIP_SCREEN_RIGHT_OFFSET_X = 12\n\
             NDefines.NGraphics.TOOLTIP_SCREEN_TOP_OFFSET_Y = 13\n\
             NDefines.NGraphics.TOOLTIP_SCREEN_BOTTOM_OFFSET_Y = 14\n",
        )
        .expect("write effective graphics defines");

        let resolver = AssetResolver::new(vec![winning_mod.clone(), base.clone()]);
        let facts = discover_authored_unscoped_tooltip(
            &resolver,
            &AuthoredTooltipTarget {
                gui_source_path: "interface/hover.gui".into(),
                runtime_root_name: "root".into(),
                child_path: vec![0],
                owner_child_path: Some(vec![9, 8]),
                element_type: "buttonType".into(),
                element_name: "target".into(),
                engine_instance_token: "root/target/77".into(),
                paint_ordinal: 8,
                alternate_tooltip_selected: RecordedFact::Value(false),
                include_delayed: false,
            },
        )
        .expect("discover authored tooltip")
        .expect("target owns a tooltip");
        let TooltipFamilyFacts::AuthoredUnscoped(facts) = facts else {
            panic!("discovery returned a non-authored family");
        };
        assert_eq!(
            facts.localisation_key,
            RecordedFact::Value("MOD_KEY".into())
        );
        assert_eq!(facts.localisation_property_name, "pdx_tooltip");
        assert_eq!(facts.delayed_localisation_key.as_deref(), Some("MOD_DELAY"));
        assert_eq!(
            facts.delayed_property_name.as_deref(),
            Some("pdx_tooltip_delayed")
        );
        assert_eq!(
            facts.authored_anchor,
            AuthoredTooltipAnchor::Packed {
                position: (-310, 0),
                relative_to_owner: true,
            }
        );
        assert_eq!(facts.gui_source_path, "interface/hover.gui");
        assert_eq!(facts.element_path, "root/target");
        assert_eq!(facts.tooltip_root_name, "RenamedTip");
        assert_eq!(facts.owner.engine_instance_token, "root/target/77");
        assert_eq!(facts.owner.element_path, [9, 8]);
        assert_eq!(facts.source_ordinal, 8);
        let child_target = AuthoredTooltipTarget {
            gui_source_path: "interface/hover.gui".into(),
            runtime_root_name: "root".into(),
            child_path: vec![0, 1],
            owner_child_path: None,
            element_type: "iconType".into(),
            element_name: "child".into(),
            engine_instance_token: "root/target/child/77".into(),
            paint_ordinal: 9,
            alternate_tooltip_selected: RecordedFact::Unknown {
                reason: "child has no alternate declarations".into(),
            },
            include_delayed: false,
        };
        assert_eq!(
            discover_authored_unscoped_tooltip(&resolver, &child_target)
                .expect("authenticate exact child"),
            None,
        );
        assert_eq!(
            discover_authored_tooltip_placement(&resolver, &child_target)
                .expect("authenticate custom-provider placement")
                .anchor,
            AuthoredTooltipAnchor::Cursor,
            "placement is likewise owned by the exact target node"
        );
        let (clamps, source) =
            load_authored_tooltip_clamp_offsets(&resolver).expect("load authored clamps");
        assert_eq!(
            clamps,
            AuthoredClampOffsets {
                left: 11,
                right: 12,
                top: 13,
                bottom: 14,
            }
        );
        assert_eq!(source, vec!["common/defines/00_graphics.lua"]);

        let alternate = discover_authored_unscoped_tooltip(
            &resolver,
            &AuthoredTooltipTarget {
                gui_source_path: "interface/hover.gui".into(),
                runtime_root_name: "root".into(),
                child_path: vec![0],
                owner_child_path: None,
                element_type: "buttonType".into(),
                element_name: "target".into(),
                engine_instance_token: "root/target/78".into(),
                paint_ordinal: 9,
                alternate_tooltip_selected: RecordedFact::Value(true),
                include_delayed: true,
            },
        )
        .expect("discover alternate authored tooltip")
        .expect("alternate target owns a tooltip");
        let TooltipFamilyFacts::AuthoredUnscoped(alternate) = alternate else {
            panic!("alternate discovery returned a non-authored family");
        };
        assert_eq!(
            alternate.localisation_key,
            RecordedFact::Value("MOD_DISABLED".into())
        );
        assert_eq!(alternate.localisation_property_name, "pdx_disabled_tooltip");
        assert_eq!(
            alternate.delayed_localisation_key.as_deref(),
            Some("MOD_DISABLED_DELAY")
        );
        assert_eq!(
            alternate.delayed_property_name.as_deref(),
            Some("pdx_disabled_tooltip_delayed")
        );

        portable_test_fs_retry::remove_dir_all(&base).expect("remove base fixture");
        portable_test_fs_retry::remove_dir_all(&winning_mod).expect("remove mod fixture");
    }

    #[test]
    fn exact_timing_and_lifecycle_boundaries() {
        let root = unique_discovery_root("timing-lifecycle");
        std::fs::create_dir_all(root.join("common/defines")).unwrap();
        std::fs::write(root.join("common/defines/timing.lua"),
            "NDefines = { NGraphics = { TOOLTIP_DELAYED_DELAY = 2 / 2, TOOLTIP_SHOW_DELAY = 1 / 20, TOOLTIP_HIDE_DELAY = 2 / 40 } }").unwrap();
        let resolver = AssetResolver::new(vec![root.clone()]);
        let (authored, sources) = load_authored_tooltip_timing(&resolver).unwrap();
        assert_eq!(authored, timing());
        assert_eq!(sources, ["common/defines/timing.lua"]);
        let mut life = TooltipLifecycle::new(authored);
        let a = HoverCandidate {
            owner: owner("a"),
            route_identity: "ordinary:a".into(),
            delayed: false,
        };
        life.advance(Some(a.clone()), 9_000_000);
        life.advance(Some(a), 49_999);
        assert_eq!(life.visible_owner(), None);
        life.advance(
            Some(HoverCandidate {
                owner: owner("a"),
                route_identity: "ordinary:a".into(),
                delayed: false,
            }),
            1,
        );
        assert_eq!(life.visible_owner(), Some(&owner("a")));
        let b = HoverCandidate {
            owner: owner("b"),
            route_identity: "ordinary:b".into(),
            delayed: true,
        };
        life.advance(Some(b.clone()), 9_000_000);
        assert_eq!(life.visible_owner(), None);
        life.advance(Some(b.clone()), 49_999);
        assert_eq!(life.visible_owner(), None);
        life.advance(Some(b.clone()), 1);
        assert_eq!(life.visible_owner(), Some(&owner("b")));
        assert!(!life.delayed_content_visible());
        life.advance(Some(b), 950_000);
        assert!(life.delayed_content_visible());
        life.advance(None, 0);
        life.advance(None, 49_999);
        assert_eq!(life.visible_owner(), Some(&owner("b")));
        life.advance(None, 1);
        assert_eq!(life.visible_owner(), None);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn effective_tooltip_timing_refuses_invalid_ranges_and_submicrosecond_values() {
        let root = unique_discovery_root("invalid-timing");
        std::fs::create_dir_all(root.join("common/defines")).unwrap();
        for key in ["TOOLTIP_SHOW_DELAY", "TOOLTIP_HIDE_DELAY", "TOOLTIP_DELAYED_DELAY"] {
            for expression in ["nil", "true", "'slow'", "0 / 0", "1 / 0", "-1", "-0.0000001", "2 ^ 64", "18446744073709.551616", "0.0000001", "0.0000015", "0.1 * 3"] {
                let source = format!("NDefines = {{ NGraphics = {{ TOOLTIP_SHOW_DELAY = 0, TOOLTIP_HIDE_DELAY = 0, TOOLTIP_DELAYED_DELAY = 0 }} }}\nNDefines.NGraphics.{key} = {expression}");
                std::fs::write(root.join("common/defines/timing.lua"), source).unwrap();
                let resolver = AssetResolver::new(vec![root.clone()]);
                let error = load_authored_tooltip_timing(&resolver).unwrap_err();
                assert!(error.message.contains(key), "{key}={expression}: {error}");
                if ["0.0000001", "0.0000015", "0.1 * 3"].contains(&expression) {
                    assert_eq!(error.kind, crate::ErrorKind::Parse);
                    assert!(error.message.contains("sub-microsecond precision"), "{error}");
                }
            }
        }
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn effective_tooltip_timing_preserves_single_microsecond_boundaries() {
        let root = unique_discovery_root("microsecond-timing");
        std::fs::create_dir_all(root.join("common/defines")).unwrap();
        std::fs::write(root.join("common/defines/timing.lua"),
            "NDefines = { NGraphics = { TOOLTIP_SHOW_DELAY = 1 / 1000000, TOOLTIP_HIDE_DELAY = 2 / 1000000, TOOLTIP_DELAYED_DELAY = 3 / 1000000 } }").unwrap();
        let resolver = AssetResolver::new(vec![root.clone()]);
        let (authored, _) = load_authored_tooltip_timing(&resolver).unwrap();
        assert_eq!(authored, TooltipTimingLaw { show_delay_us: 1, hide_delay_us: 2, delayed_delay_us: 3 });
        let candidate = HoverCandidate { owner: owner("microsecond"), route_identity: "tiny:tooltip".into(), delayed: true };
        let mut life = TooltipLifecycle::new(authored);
        life.advance(Some(candidate.clone()), 0);
        assert_eq!(life.visible_owner(), None);
        life.advance(Some(candidate.clone()), 1);
        assert_eq!(life.visible_owner(), Some(&owner("microsecond")));
        life.advance(Some(candidate.clone()), 1);
        assert!(!life.delayed_content_visible());
        life.advance(Some(candidate), 1);
        assert!(life.delayed_content_visible());
        life.advance(None, 0);
        life.advance(None, 1);
        assert_eq!(life.visible_owner(), Some(&owner("microsecond")));
        life.advance(None, 1);
        assert_eq!(life.visible_owner(), None);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn authored_alternate_transition_restarts_dwell_for_the_same_owner() {
        let mut life = TooltipLifecycle::new(timing());
        let ordinary = HoverCandidate {
            owner: owner("same"),
            route_identity: "pdx_tooltip:ENABLED".into(),
            delayed: false,
        };
        life.advance(Some(ordinary.clone()), 0);
        life.advance(Some(ordinary), 50_000);
        assert_eq!(life.visible_owner(), Some(&owner("same")));

        let disabled = HoverCandidate {
            owner: owner("same"),
            route_identity: "pdx_disabled_tooltip:DISABLED".into(),
            delayed: false,
        };
        life.advance(Some(disabled.clone()), 0);
        assert_eq!(life.visible_owner(), None);
        life.advance(Some(disabled), 50_000);
        assert_eq!(life.visible_owner(), Some(&owner("same")));

        let ordinary_again = HoverCandidate {
            owner: owner("same"),
            route_identity: "pdx_tooltip:ENABLED".into(),
            delayed: false,
        };
        life.advance(Some(ordinary_again.clone()), 0);
        assert_eq!(life.visible_owner(), None);
        life.advance(Some(ordinary_again), 50_000);
        assert_eq!(life.visible_owner(), Some(&owner("same")));
    }

    #[test]
    fn collision_clipping_stacking_and_ownerless_occlusion() {
        let r = ScreenRect::new(0, 0, 100, 100);
        let mut surfaces = vec![
            HitSurface {
                rect: r,
                clip_rect: r,
                draw_list_ordinal: 0,
                z_order: 99,
                item_ordinal: 9,
                owner: Some(owner("old")),
            },
            HitSurface {
                rect: r,
                clip_rect: ScreenRect::new(0, 0, 50, 100),
                draw_list_ordinal: 1,
                z_order: 0,
                item_ordinal: 0,
                owner: Some(owner("clipped")),
            },
            HitSurface {
                rect: r,
                clip_rect: r,
                draw_list_ordinal: 1,
                z_order: 1,
                item_ordinal: 1,
                owner: Some(owner("front")),
            },
        ];
        assert_eq!(frontmost_owner(&surfaces, (99, 99)), Some(&owner("front")));
        assert_eq!(frontmost_owner(&surfaces, (100, 99)), None);
        surfaces.push(HitSurface {
            rect: r,
            clip_rect: r,
            draw_list_ordinal: 2,
            z_order: 0,
            item_ordinal: 0,
            owner: None,
        });
        assert_eq!(frontmost_owner(&surfaces, (10, 10)), None);
    }

    #[test]
    fn placement_clipping_preserves_engine_origin_and_extent() {
        let p = ResolvedPlacement {
            screen_rect: ScreenRect::new(1900, 1060, 100, 60),
            evidence: PlacementEvidence::ExtractedEngineRect {
                producer_record: "x".into(),
            },
        };
        assert_eq!(
            p.clipped_to(ScreenRect::new(0, 0, 1920, 1080)).unwrap().1,
            ScreenRect::new(1900, 1060, 20, 20)
        );
    }

    fn packet_trace(name: &str) -> Vec<AdmittedTooltipSource> {
        vec![AdmittedTooltipSource::PacketSemantic {
            packet_id: "XUI_TEST".into(),
            producer_identity: "test-producer".into(),
            field: name.into(),
            manifest_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .into(),
        }]
    }

    #[test]
    fn data_blocked_and_reordered_content_fail_closed() {
        let row = TooltipRow {
            source_ordinal: 1,
            line_kind: TooltipLineKind::Scripted,
            source_identity: "script".into(),
            evaluation_context: "country=GER".into(),
            availability: RowAvailability::DataBlocked {
                field: "resolved_scripted_text".into(),
            },
            text: String::new(),
            admitted_sources: Vec::new(),
        };
        let mut p = TooltipPayload {
            owner: owner("a"),
            tooltip_root_name: "ToolTip".into(),
            rows: vec![row],
        };
        assert!(p
            .validate()
            .unwrap_err()
            .to_string()
            .contains("DATA_BLOCKED"));
        p.rows = vec![
            TooltipRow {
                source_ordinal: 2,
                line_kind: TooltipLineKind::Header,
                source_identity: "b".into(),
                evaluation_context: "x".into(),
                availability: RowAvailability::Resolved,
                text: "b".into(),
                admitted_sources: trace("b"),
            },
            TooltipRow {
                source_ordinal: 1,
                line_kind: TooltipLineKind::Header,
                source_identity: "a".into(),
                evaluation_context: "x".into(),
                availability: RowAvailability::Resolved,
                text: "a".into(),
                admitted_sources: trace("a"),
            },
        ];
        assert!(p.validate().is_err());
    }

    fn authored_facts(key: RecordedFact<String>) -> TooltipFamilyFacts {
        TooltipFamilyFacts::AuthoredUnscoped(AuthoredUnscopedTooltipFacts {
            owner: owner("topbar/pause"),
            tooltip_root_name: "ToolTip".into(),
            source_ordinal: 7,
            gui_source_path: "interface/topbar.gui".into(),
            element_path: "topbar/pause".into(),
            localisation_key: key,
            localisation_property_name: "pdx_tooltip".into(),
            delayed_localisation_key: None,
            delayed_property_name: None,
            authored_anchor: AuthoredTooltipAnchor::Cursor,
            include_delayed: false,
            has_scoped_evaluation: false,
        })
    }

    fn localisation() -> Localisation {
        let source = "localisation/english/test_l_english.yml".to_string();
        Localisation {
            entries: BTreeMap::from([("PAUSE_TOOLTIP".into(), "Pause the game".into())]),
            source_by_key: BTreeMap::from([("PAUSE_TOOLTIP".into(), source.clone())]),
            provenance_complete: true,
            source_files: vec![source],
            ..Localisation::default()
        }
    }

    #[test]
    fn f1_missing_family_leaf_is_named_and_no_partial_payload_exists() {
        assert_eq!(
            compose_payload_from_facts(
                TooltipFamilyFacts::ProvinceSupplyAccumulatedHubFlow,
                &localisation()
            ),
            Err(TooltipRefusal::MissingLeaf {
                family: "province supply accumulated hub-flow".into(),
                leaf: "passively acquired incoming hub-flow facts for every province and every viewer-selectable observer country; hover-gated recorder-POV observations do not close this port"
                    .into(),
            })
        );
    }

    #[test]
    fn f2_absent_null_unknown_and_zero_are_four_distinct_typed_results() {
        assert_eq!(
            require_fact::<i64>("amount", RecordedFact::Absent),
            Err(TooltipRefusal::FactAbsent {
                field: "amount".into()
            })
        );
        assert_eq!(
            require_fact::<i64>("amount", RecordedFact::Null),
            Err(TooltipRefusal::FactNull {
                field: "amount".into()
            })
        );
        assert_eq!(
            require_fact::<i64>(
                "amount",
                RecordedFact::Unknown {
                    reason: "domain unavailable".into()
                }
            ),
            Err(TooltipRefusal::FactUnknown {
                field: "amount".into(),
                reason: "domain unavailable".into()
            })
        );
        assert_eq!(require_fact("amount", RecordedFact::Value(0_i64)), Ok(0));
    }

    #[test]
    fn f3_localisation_key_and_internal_identity_never_render_as_display_text() {
        assert_eq!(
            compose_payload_from_facts(
                authored_facts(RecordedFact::Value("MISSING_TOOLTIP_KEY".into())),
                &localisation()
            ),
            Err(TooltipRefusal::MissingLocalisationKey {
                key: "MISSING_TOOLTIP_KEY".into()
            })
        );
        let payload = compose_payload_from_facts(
            authored_facts(RecordedFact::Value("PAUSE_TOOLTIP".into())),
            &localisation(),
        )
        .expect("effective localisation exists");
        assert_eq!(payload.rows[0].text, "Pause the game");
        assert_ne!(payload.rows[0].text, payload.rows[0].source_identity);
        assert!(payload.rows[0].admitted_sources.iter().any(|source| {
            matches!(
                source,
                AdmittedTooltipSource::EffectiveLocalisation {
                    key,
                    source_files,
                } if key == "PAUSE_TOOLTIP"
                    && source_files == &["localisation/english/test_l_english.yml".to_string()]
            )
        }));
    }

    #[test]
    fn f4_completeness_refuses_a_drawn_row_without_an_admitted_trace() {
        let payload = TooltipPayload {
            owner: owner("x"),
            tooltip_root_name: "ToolTip".into(),
            rows: vec![TooltipRow {
                source_ordinal: 1,
                line_kind: TooltipLineKind::Description,
                source_identity: "untraced-row".into(),
                evaluation_context: "test".into(),
                availability: RowAvailability::Resolved,
                text: "visible".into(),
                admitted_sources: Vec::new(),
            }],
        };
        assert_eq!(
            validate_payload_trace(&payload),
            Err(TooltipRefusal::UntraceableRow {
                row: "untraced-row".into()
            })
        );
    }

    #[test]
    fn packet_owned_semantic_payload_cannot_self_admit_untraced_text() {
        let mut payload = TooltipPayload {
            owner: owner("custom"),
            tooltip_root_name: "ToolTip".into(),
            rows: vec![TooltipRow {
                source_ordinal: 1,
                line_kind: TooltipLineKind::Scripted,
                source_identity: "custom/provider/row0".into(),
                evaluation_context: "viewer_country=GER".into(),
                availability: RowAvailability::Resolved,
                text: "Exact semantic row".into(),
                admitted_sources: Vec::new(),
            }],
        };
        assert_eq!(
            compose_payload_from_facts(
                TooltipFamilyFacts::ExactSemanticPayload(payload.clone()),
                &Localisation::default(),
            ),
            Err(TooltipRefusal::UntraceableRow {
                row: "custom/provider/row0".into(),
            })
        );
        payload.rows[0].admitted_sources = packet_trace("custom/provider/row0");
        assert_eq!(
            compose_payload_from_facts(
                TooltipFamilyFacts::ExactSemanticPayload(payload.clone()),
                &Localisation::default(),
            ),
            Ok(payload.clone())
        );
        let mut unidentified = payload.clone();
        unidentified.rows[0].source_identity.clear();
        assert!(unidentified.validate().is_err());
        let mut context_free = payload;
        context_free.rows[0].evaluation_context.clear();
        assert!(context_free.validate().is_err());
    }

    #[test]
    fn measured_empty_packet_row_requires_the_same_semantic_authority_as_visible_text() {
        let mut payload = TooltipPayload {
            owner: owner("measured-empty"),
            tooltip_root_name: "ToolTip".into(),
            rows: vec![TooltipRow {
                source_ordinal: 1,
                line_kind: TooltipLineKind::Scripted,
                source_identity: "XUI08/allocation/empty-primary".into(),
                evaluation_context: "conference=77:1".into(),
                availability: RowAvailability::MeasuredEmpty,
                text: String::new(),
                admitted_sources: packet_trace("tooltip.primary.final_text"),
            }],
        };
        assert_eq!(
            compose_payload_from_facts(
                TooltipFamilyFacts::ExactSemanticPayload(payload.clone()),
                &Localisation::default(),
            ),
            Ok(payload.clone())
        );

        payload.rows[0].admitted_sources = vec![AdmittedTooltipSource::RecordedState {
            domain: "PCS3".into(),
            field: "tooltip.primary.final_text".into(),
            identity: "row:0".into(),
        }];
        assert_eq!(
            compose_payload_from_facts(
                TooltipFamilyFacts::ExactSemanticPayload(payload),
                &Localisation::default(),
            ),
            Err(TooltipRefusal::UntraceableRow {
                row: "XUI08/allocation/empty-primary".into(),
            })
        );
    }

    #[test]
    fn delayed_authored_row_keeps_the_selected_property_in_provenance() {
        let mut facts = match authored_facts(RecordedFact::Value("PAUSE_TOOLTIP".into())) {
            TooltipFamilyFacts::AuthoredUnscoped(facts) => facts,
            _ => unreachable!(),
        };
        facts.localisation_property_name = "pdx_disabled_tooltip".into();
        facts.delayed_property_name = Some("pdx_disabled_tooltip_delayed".into());
        facts.delayed_localisation_key = Some("DELAYED_TOOLTIP".into());
        facts.include_delayed = true;
        let mut loc = localisation();
        loc.entries
            .insert("DELAYED_TOOLTIP".into(), "Delayed exact text".into());
        loc.source_by_key.insert(
            "DELAYED_TOOLTIP".into(),
            "localisation/english/test_l_english.yml".into(),
        );
        let payload = compose_payload_from_facts(TooltipFamilyFacts::AuthoredUnscoped(facts), &loc)
            .expect("both authored rows resolve");
        assert_eq!(payload.rows.len(), 2);
        assert_eq!(
            payload.rows[1].source_identity,
            "topbar/pause/pdx_disabled_tooltip_delayed"
        );
        assert_eq!(payload.rows[1].text, "Delayed exact text");
    }

    #[test]
    fn f5_overwide_row_is_refused_by_name_until_text1_wrap_exists() {
        let payload = compose_payload_from_facts(
            authored_facts(RecordedFact::Value("PAUSE_TOOLTIP".into())),
            &localisation(),
        )
        .expect("payload");
        assert_eq!(
            refuse_rows_requiring_wrap(&payload, 40, 10, |_| 21),
            Err(TooltipRefusal::RowRequiresWrap {
                row: "topbar/pause/pdx_tooltip".into(),
                measured_width: 21,
                interior_width: 20,
            })
        );
    }

    #[test]
    fn f6_authored_border_and_width_have_no_default_path() {
        let payload = compose_payload_from_facts(
            authored_facts(RecordedFact::Value("PAUSE_TOOLTIP".into())),
            &localisation(),
        )
        .expect("payload");
        assert_eq!(
            refuse_rows_requiring_wrap_from_authored(
                &payload,
                RecordedFact::Value(400),
                RecordedFact::Absent,
                |_| 10,
            ),
            Err(TooltipRefusal::FactAbsent {
                field: "authored tooltip borderSize.x".into()
            })
        );
    }

    #[test]
    fn viewer_cursor_provenance_and_all_anchor_modes_follow_the_engine_law() {
        let base = ViewerCursorPlacement {
            raw_cursor_px: (100, 50),
            gui_scale_f32_bits: (1.5_f32.to_bits(), 2.0_f32.to_bits()),
            anchor: TooltipAnchor::Cursor,
            viewport: (500, 300),
            tooltip_extent: (80, 40),
            clamp: AuthoredClampOffsets {
                left: 0,
                right: 0,
                top: 0,
                bottom: 0,
            },
        };
        let cursor = place_under_viewer_cursor(base).expect("cursor placement");
        assert_eq!(cursor.screen_rect, ScreenRect::new(171, 124, 80, 40));
        assert_eq!(
            cursor.evidence,
            PlacementEvidence::ViewerCursor {
                raw_px: (100, 50),
                ui_px: (150, 100),
                gui_scale_f32_bits: (1.5_f32.to_bits(), 2.0_f32.to_bits()),
            }
        );
        let absolute = place_under_viewer_cursor(ViewerCursorPlacement {
            anchor: TooltipAnchor::AbsolutePacked { position: (7, 9) },
            ..base
        })
        .expect("absolute");
        assert_eq!(absolute.screen_rect, ScreenRect::new(7, 9, 80, 40));
        let relative = place_under_viewer_cursor(ViewerCursorPlacement {
            anchor: TooltipAnchor::OwnerRelativePacked {
                packed: (7, 9),
                owner_screen_position: (30, 40),
            },
            ..base
        })
        .expect("relative");
        assert_eq!(relative.screen_rect, ScreenRect::new(37, 49, 80, 40));
        let sticky = place_under_viewer_cursor(ViewerCursorPlacement {
            anchor: TooltipAnchor::StickyRetainedPosition {
                position: (111, 222),
            },
            ..base
        })
        .expect("viewer-owned retained position");
        assert_eq!(sticky.screen_rect, ScreenRect::new(111, 222, 80, 40));
    }
}
