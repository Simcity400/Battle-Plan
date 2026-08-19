
use crate::authored_role_index::{AuthenticatedGuiRoleIndex, StrictAuthoredPanelAdmission};
use crate::compose::AuthenticatedComposeReport;
use crate::runtime_graphics::{select_one_based_frame, ButtonVisualState, FrameSelection};
use crate::window_manager_law::{
    AuthenticatedReplayFrameAuthority, AuthenticatedReplayPanelLease, ReplayOwnedWindowManager,
};
use replay_core::ValidatedContentAdmission;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetFamily {
    Window,
    Container,
    Icon,
    Button,
    GuiButton,
    Checkbox,
    DropDown,
    EditBox,
    Browser,
    InstantText,
    Text,
    List,
    SmoothList,
    Grid,
    Overlap,
    LegacyScrollbar,
    ExtendedScrollbar,
    ScrollbarPart,
}

impl WidgetFamily {
    pub fn from_element_type(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "windowtype" => Some(Self::Window),
            "containerwindowtype" => Some(Self::Container),
            "icontype" => Some(Self::Icon),
            "buttontype" => Some(Self::Button),
            "guibuttontype" => Some(Self::GuiButton),
            "checkboxtype" => Some(Self::Checkbox),
            "dropdownboxtype" => Some(Self::DropDown),
            "editboxtype" => Some(Self::EditBox),
            "browsertype" => Some(Self::Browser),
            "instanttextboxtype" => Some(Self::InstantText),
            "textboxtype" => Some(Self::Text),
            "listboxtype" => Some(Self::List),
            "smoothlistboxtype" => Some(Self::SmoothList),
            "gridboxtype" => Some(Self::Grid),
            "overlappingelementsboxtype" => Some(Self::Overlap),
            "scrollbartype" => Some(Self::LegacyScrollbar),
            "extendedscrollbartype" => Some(Self::ExtendedScrollbar),
            "decreasebutton" | "increasebutton" | "expandbutton" | "track" | "slider" => {
                Some(Self::ScrollbarPart)
            }
            _ => None,
        }
    }

    pub fn has_sprite_surface(self) -> bool {
        matches!(
            self,
            Self::Icon
                | Self::Button
                | Self::GuiButton
                | Self::Checkbox
                | Self::DropDown
                | Self::ScrollbarPart
        )
    }

    pub(crate) fn standard_button_carrier(self) -> bool {
        matches!(self, Self::Button | Self::GuiButton | Self::ScrollbarPart)
    }

    fn scrollbar(self) -> bool {
        matches!(self, Self::LegacyScrollbar | Self::ExtendedScrollbar)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOnlyPresentation {
    Passive,
    Navigation,
    InertSnapshot,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttentionCarrier {
    PoliticalGoalPulse { trigger_event: bool },
    ProducerResolved,
}

pub use crate::scrollbar_law::ScrollbarAxis;

#[derive(Clone, Debug, PartialEq)]
pub struct DragSnapshot {
    pub source_record_id: String,
    pub axis: ScrollbarAxis,
    pub value: f32,
    pub thumb_x: i32,
    pub thumb_y: i32,
    pub pointer_x: i32,
    pub pointer_y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressEffect {
    Linear,
    StartEnd,
    Reverse,
    MinMaxColor,
    MinMaxTexture,
    Radial,
    Circular,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressFill {
    pub effect: ProgressEffect,
    pub current_state: f32,
    pub circular_mix: Option<f32>,
}

impl ProgressFill {
    pub fn valid(self) -> bool {
        self.current_state.is_finite()
            && match self.effect {
                ProgressEffect::Circular => self.circular_mix.is_some_and(f32::is_finite),
                _ => self.circular_mix.is_none(),
            }
    }

    pub fn foreground_at(self, u: f32) -> Option<bool> {
        match self.effect {
            ProgressEffect::Linear | ProgressEffect::StartEnd => Some(u <= self.current_state),
            ProgressEffect::Reverse => Some(u >= 1.0 - self.current_state),
            ProgressEffect::MinMaxColor => {
                Some((u <= 0.5 && u <= self.current_state) || (u >= 0.5 && u >= self.current_state))
            }
            ProgressEffect::MinMaxTexture => {
                Some((u < 0.5 && u >= self.current_state) || (u >= 0.5 && u <= self.current_state))
            }
            ProgressEffect::Radial | ProgressEffect::Circular => None,
        }
    }

    pub fn texture_two_mix(self, u: f32, v: f32) -> Option<f32> {
        match self.effect {
            ProgressEffect::Radial => {
                let dx = 0.5_f32 - u;
                let dy = 0.5_f32 - v;
                let angle = dx.atan2(dy) + std::f32::consts::PI;
                Some(
                    ((angle - self.current_state * std::f32::consts::PI * 2.0) * 50.0)
                        .clamp(0.0, 1.0),
                )
            }
            ProgressEffect::Circular => self.circular_mix,
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetVisualRequest {
    pub source_record_id: String,
    pub family: WidgetFamily,
    pub presentation: ReadOnlyPresentation,
    pub interaction: Option<ButtonVisualState>,
    pub is_current_hover_target: bool,
    pub frame_count: usize,
    pub one_based_frame: Option<u32>,
    pub selected: bool,
    pub active: bool,
    pub toggle: Option<bool>,
    pub attention: Option<AttentionCarrier>,
    pub progress: Option<ProgressFill>,
    pub drag: Option<DragSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetVisualRejection {
    MissingProvenance,
    InvalidFrame,
    MissingButtonState,
    UnsupportedButtonState,
    MissingSemanticFrame,
    InvalidProgress,
    ProgressOnNonSprite,
    DragOnNonScrollbar,
    InvalidDragSnapshot,
    DisabledStateMismatch,
    UnindexedElement,
    ElementFamilyMismatch,
    SpriteFrameCountMismatch,
    UnsupportedViewerChromeState,
    InvalidViewerLifecycle,
    AuthoredGeometryMismatch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedWidgetVisual {
    frame: Option<FrameSelection>,
    pub interaction: Option<ButtonVisualState>,
    pub progress: Option<ProgressFill>,
    pub drag: Option<DragSnapshot>,
    pub attention: Option<AttentionCarrier>,
    pub allows_navigation_events: bool,
    pub allows_game_mutation: bool,
}

impl AdmittedWidgetVisual {
    pub fn frame(&self) -> Option<FrameSelection> {
        self.frame
    }
}

pub(crate) fn admit_widget_visual(
    request: &WidgetVisualRequest,
) -> Result<AdmittedWidgetVisual, WidgetVisualRejection> {
    if request.source_record_id.trim().is_empty() {
        return Err(WidgetVisualRejection::MissingProvenance);
    }

    let semantic_state = request.selected || request.active || request.toggle.is_some();
    if semantic_state && request.one_based_frame.is_none() {
        return Err(WidgetVisualRejection::MissingSemanticFrame);
    }

    if request.presentation == ReadOnlyPresentation::Disabled {
        if request.family.standard_button_carrier() {
            if request.interaction != Some(ButtonVisualState::Disable) {
                return Err(WidgetVisualRejection::DisabledStateMismatch);
            }
        } else if request.family.has_sprite_surface() && request.one_based_frame.is_none() {
            return Err(WidgetVisualRejection::DisabledStateMismatch);
        }
    }

    let effective_interaction = match request.interaction {
        Some(ButtonVisualState::Over) if !request.is_current_hover_target => {
            Some(ButtonVisualState::Up)
        }
        other => other,
    };
    let frame = if let Some(one_based) = request.one_based_frame {
        Some(
            select_one_based_frame(one_based, request.frame_count)
                .ok_or(WidgetVisualRejection::InvalidFrame)?,
        )
    } else if request.family.has_sprite_surface() {
        if request.family.standard_button_carrier() {
            request
                .interaction
                .ok_or(WidgetVisualRejection::MissingButtonState)?;
        }
        Some(
            select_one_based_frame(1, request.frame_count)
                .ok_or(WidgetVisualRejection::InvalidFrame)?,
        )
    } else {
        if request.interaction.is_some() {
            return Err(WidgetVisualRejection::UnsupportedButtonState);
        }
        None
    };

    if let Some(progress) = request.progress {
        if !request.family.has_sprite_surface() {
            return Err(WidgetVisualRejection::ProgressOnNonSprite);
        }
        if !progress.valid() {
            return Err(WidgetVisualRejection::InvalidProgress);
        }
    }

    if let Some(drag) = &request.drag {
        if !request.family.scrollbar() {
            return Err(WidgetVisualRejection::DragOnNonScrollbar);
        }
        if drag.source_record_id.trim().is_empty() || !drag.value.is_finite() {
            return Err(WidgetVisualRejection::InvalidDragSnapshot);
        }
    }

    Ok(AdmittedWidgetVisual {
        frame,
        interaction: effective_interaction,
        progress: request.progress,
        drag: request.drag.clone(),
        attention: request.attention,
        allows_navigation_events: request.presentation == ReadOnlyPresentation::Navigation,
        allows_game_mutation: false,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerOwnedChromeState {
    Passive,
    Navigation {
        interaction: ButtonVisualState,
        is_current_hover_target: bool,
    },
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthenticatedWidgetIdentity {
    index_identity_sha256: String,
    window_id: String,
    surface_id: String,
    element_id: String,
    authored_name: String,
    element_type: String,
    child_path: Vec<u32>,
}

impl AuthenticatedWidgetIdentity {
    pub fn window_id(&self) -> &str {
        &self.window_id
    }

    pub fn surface_id(&self) -> &str {
        &self.surface_id
    }

    pub fn element_id(&self) -> &str {
        &self.element_id
    }

    pub fn authored_name(&self) -> &str {
        &self.authored_name
    }

    pub fn element_type(&self) -> &str {
        &self.element_type
    }

    pub fn child_path(&self) -> &[u32] {
        &self.child_path
    }
}

pub fn admit_widget_identity(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &StrictAuthoredPanelAdmission,
    child_path: &[u32],
) -> Result<AuthenticatedWidgetIdentity, WidgetVisualRejection> {
    let surface = index
        .validate_panel_token(admission, panel)
        .map_err(|_| WidgetVisualRejection::UnindexedElement)?;
    let element = surface
        .elements
        .iter()
        .find(|element| element.child_path == child_path)
        .ok_or(WidgetVisualRejection::UnindexedElement)?;
    Ok(AuthenticatedWidgetIdentity {
        index_identity_sha256: index.index_identity_sha256.clone(),
        window_id: panel.window_id().to_string(),
        surface_id: surface.surface_id.clone(),
        element_id: element.element_id.clone(),
        authored_name: element.authored_name.clone(),
        element_type: element.element_type.clone(),
        child_path: element.child_path.clone(),
    })
}

/// One XUI07-sealed visible rectangle for the manager's current open lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedWidgetGeometry {
    identity: AuthenticatedWidgetIdentity,
    runtime_instance_token: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedViewerWidgetState {
    identity: AuthenticatedWidgetIdentity,
    runtime_instance_token: String,
    frame_generation: u64,
    lifecycle: ViewerWidgetLifecycle,
    chrome_state: Option<ViewerOwnedChromeState>,
    effect_entered_at_micros: u64,
    observed_at_micros: u64,
}

impl AuthenticatedViewerWidgetState {
    pub fn identity(&self) -> &AuthenticatedWidgetIdentity {
        &self.identity
    }

    pub(crate) fn runtime_instance_token(&self) -> &str {
        &self.runtime_instance_token
    }

    pub fn lifecycle(&self) -> ViewerWidgetLifecycle {
        self.lifecycle
    }

    pub(crate) fn frame_generation(&self) -> u64 {
        self.frame_generation
    }

    pub fn chrome_state(&self) -> Option<ViewerOwnedChromeState> {
        self.chrome_state
    }

    pub fn effect_entered_at_micros(&self) -> u64 {
        self.effect_entered_at_micros
    }

    pub fn observed_at_micros(&self) -> u64 {
        self.observed_at_micros
    }
}

pub(crate) fn advance_authenticated_viewer_widget_state(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &StrictAuthoredPanelAdmission,
    lease: &AuthenticatedReplayPanelLease<'_>,
    previous: Option<&AuthenticatedViewerWidgetState>,
    child_path: &[u32],
    lifecycle: ViewerWidgetLifecycle,
) -> Result<AuthenticatedViewerWidgetState, WidgetVisualRejection> {
    if !frame_authority.remains_staged_for(manager)
        || !lease.remains_current()
        || lease.window_id() != panel.window_id()
    {
        return Err(WidgetVisualRejection::InvalidViewerLifecycle);
    }
    let viewer_clock_micros = frame_authority.viewer_clock_micros();
    let identity = admit_widget_identity(index, admission, panel, child_path)?;
    if lease.window_id() != identity.window_id() {
        return Err(WidgetVisualRejection::InvalidViewerLifecycle);
    }
    advance_authenticated_viewer_widget_state_for_identity(
        previous,
        &identity,
        lease.runtime_instance_token(),
        frame_authority.frame_generation(),
        lifecycle,
        viewer_clock_micros,
    )
}

fn advance_authenticated_viewer_widget_state_for_identity(
    previous: Option<&AuthenticatedViewerWidgetState>,
    identity: &AuthenticatedWidgetIdentity,
    runtime_instance_token: &str,
    frame_generation: u64,
    lifecycle: ViewerWidgetLifecycle,
    viewer_clock_micros: u64,
) -> Result<AuthenticatedViewerWidgetState, WidgetVisualRejection> {
    if runtime_instance_token.trim().is_empty() {
        return Err(WidgetVisualRejection::InvalidViewerLifecycle);
    }
    if let Some(previous) = previous {
        if viewer_clock_micros < previous.observed_at_micros
            || previous.frame_generation >= frame_generation
        {
            return Err(WidgetVisualRejection::InvalidViewerLifecycle);
        }
    }
    let chrome_state = if lifecycle.visible {
        let family = WidgetFamily::from_element_type(identity.element_type())
            .ok_or(WidgetVisualRejection::ElementFamilyMismatch)?;
        Some(derive_viewer_owned_chrome_state(family, lifecycle)?)
    } else {
        if lifecycle.hovered || lifecycle.pressed || lifecycle.focused {
            return Err(WidgetVisualRejection::InvalidViewerLifecycle);
        }
        None
    };
    let effect_entered_at_micros = previous
        .filter(|previous| {
            previous.identity == *identity
                && previous.runtime_instance_token == runtime_instance_token
                && previous.chrome_state == chrome_state
        })
        .map_or(viewer_clock_micros, |previous| {
            previous.effect_entered_at_micros
        });
    Ok(AuthenticatedViewerWidgetState {
        identity: identity.clone(),
        runtime_instance_token: runtime_instance_token.to_string(),
        frame_generation,
        lifecycle,
        chrome_state,
        effect_entered_at_micros,
        observed_at_micros: viewer_clock_micros,
    })
}

impl AuthenticatedWidgetGeometry {
    pub(crate) fn identity(&self) -> &AuthenticatedWidgetIdentity {
        &self.identity
    }

    pub(crate) fn x(&self) -> i32 {
        self.x
    }

    pub(crate) fn y(&self) -> i32 {
        self.y
    }

    pub(crate) fn width(&self) -> i32 {
        self.width
    }

    pub(crate) fn height(&self) -> i32 {
        self.height
    }

    pub(crate) fn contains_half_open(
        &self,
        lease: &AuthenticatedReplayPanelLease<'_>,
        point: (i32, i32),
    ) -> bool {
        if !lease.remains_current()
            || lease.window_id() != self.identity.window_id()
            || lease.runtime_instance_token() != self.runtime_instance_token
        {
            return false;
        }
        let Some(right) = self.x.checked_add(self.width) else {
            return false;
        };
        let Some(bottom) = self.y.checked_add(self.height) else {
            return false;
        };
        self.width > 0
            && self.height > 0
            && point.0 >= self.x
            && point.1 >= self.y
            && point.0 < right
            && point.1 < bottom
    }
}

pub(crate) fn admit_composed_widget_geometry(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &StrictAuthoredPanelAdmission,
    panel_lease: &AuthenticatedReplayPanelLease<'_>,
    compose: &AuthenticatedComposeReport,
    child_path: &[u32],
) -> Result<AuthenticatedWidgetGeometry, WidgetVisualRejection> {
    if !panel_lease.remains_current()
        || panel_lease.window_id() != panel.window_id()
        || panel_lease.runtime_instance_token() != compose.runtime_instance_token()
        || compose.window_id() != panel.window_id()
        || compose.index_identity_sha256() != index.index_identity_sha256
        || compose.surface_id() != panel.surface_id()
        || compose.source_content() != panel.source_content()
    {
        return Err(WidgetVisualRejection::AuthoredGeometryMismatch);
    }
    let element = compose
        .elements()
        .iter()
        .find(|element| element.child_path() == child_path)
        .ok_or(WidgetVisualRejection::AuthoredGeometryMismatch)?;
    let rect = element
        .screen_rect()
        .filter(|_| element.visible())
        .ok_or(WidgetVisualRejection::AuthoredGeometryMismatch)?;
    if rect.width() < 0
        || rect.height() < 0
        || rect.x().checked_add(rect.width()).is_none()
        || rect.y().checked_add(rect.height()).is_none()
    {
        return Err(WidgetVisualRejection::AuthoredGeometryMismatch);
    }
    let identity = admit_widget_identity(index, admission, panel, element.child_path())?;
    if identity.element_id() != element.element_id()
        || identity.authored_name() != element.authored_name()
        || identity.element_type() != element.element_type()
    {
        return Err(WidgetVisualRejection::AuthoredGeometryMismatch);
    }
    Ok(AuthenticatedWidgetGeometry {
        identity,
        runtime_instance_token: panel_lease.runtime_instance_token().to_string(),
        x: rect.x(),
        y: rect.y(),
        width: rect.width(),
        height: rect.height(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerWidgetLifecycle {
    pub visible: bool,
    pub enabled: bool,
    pub hovered: bool,
    pub pressed: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedWidgetGraphicsState {
    identity: AuthenticatedWidgetIdentity,
    runtime_instance_token: String,
    frame_generation: u64,
    interaction: ButtonVisualState,
    frame: FrameSelection,
    effect_entered_at_micros: u64,
    observed_at_micros: u64,
}

impl AuthenticatedWidgetGraphicsState {
    pub fn identity(&self) -> &AuthenticatedWidgetIdentity {
        &self.identity
    }

    pub(crate) fn runtime_instance_token(&self) -> &str {
        &self.runtime_instance_token
    }

    pub fn interaction(&self) -> ButtonVisualState {
        self.interaction
    }

    pub(crate) fn frame_generation(&self) -> u64 {
        self.frame_generation
    }

    pub fn frame(&self) -> FrameSelection {
        self.frame
    }

    pub fn effect_entered_at_micros(&self) -> u64 {
        self.effect_entered_at_micros
    }

    pub fn observed_at_micros(&self) -> u64 {
        self.observed_at_micros
    }
}

pub(crate) fn derive_viewer_owned_chrome_state(
    family: WidgetFamily,
    lifecycle: ViewerWidgetLifecycle,
) -> Result<ViewerOwnedChromeState, WidgetVisualRejection> {
    if !lifecycle.visible && (lifecycle.hovered || lifecycle.pressed || lifecycle.focused)
        || !lifecycle.enabled && lifecycle.pressed
    {
        return Err(WidgetVisualRejection::InvalidViewerLifecycle);
    }
    if family.standard_button_carrier() {
        return Ok(if !lifecycle.enabled {
            ViewerOwnedChromeState::Disabled
        } else {
            ViewerOwnedChromeState::Navigation {
                interaction: if lifecycle.pressed && lifecycle.hovered {
                    ButtonVisualState::Down
                } else if lifecycle.hovered {
                    ButtonVisualState::Over
                } else {
                    ButtonVisualState::Up
                },
                is_current_hover_target: lifecycle.hovered,
            }
        });
    }
    if lifecycle.pressed || !lifecycle.enabled && family.has_sprite_surface() {
        return Err(WidgetVisualRejection::UnsupportedViewerChromeState);
    }
    Ok(ViewerOwnedChromeState::Passive)
}

/// release. Those values join the installed shader's declared effect order
pub(crate) fn admit_authenticated_widget_graphics_state(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &StrictAuthoredPanelAdmission,
    panel_lease: &AuthenticatedReplayPanelLease<'_>,
    state: &AuthenticatedViewerWidgetState,
) -> Result<AuthenticatedWidgetGraphicsState, WidgetVisualRejection> {
    let identity = admit_widget_identity(index, admission, panel, state.identity().child_path())?;
    if !frame_authority.remains_staged_for(manager)
        || state.frame_generation() != frame_authority.frame_generation()
        || !panel_lease.remains_current()
        || panel_lease.window_id() != panel.window_id()
        || panel_lease.window_id() != identity.window_id()
        || panel_lease.runtime_instance_token() != state.runtime_instance_token()
        || identity != *state.identity()
        || state.runtime_instance_token().trim().is_empty()
        || state.observed_at_micros() < state.effect_entered_at_micros()
        || !state.lifecycle().visible
    {
        return Err(WidgetVisualRejection::InvalidViewerLifecycle);
    }
    let family = WidgetFamily::from_element_type(identity.element_type())
        .ok_or(WidgetVisualRejection::ElementFamilyMismatch)?;
    let derived = derive_viewer_owned_chrome_state(family, state.lifecycle())?;
    if state.chrome_state() != Some(derived) {
        return Err(WidgetVisualRejection::InvalidViewerLifecycle);
    }
    let visual =
        admit_indexed_widget_visual(index, admission, panel, identity.element_id(), derived)?;
    let interaction = visual
        .interaction
        .ok_or(WidgetVisualRejection::UnsupportedViewerChromeState)?;
    let frame = visual
        .frame()
        .ok_or(WidgetVisualRejection::UnsupportedViewerChromeState)?;
    Ok(AuthenticatedWidgetGraphicsState {
        identity,
        runtime_instance_token: state.runtime_instance_token().to_string(),
        frame_generation: state.frame_generation(),
        interaction,
        frame,
        effect_entered_at_micros: state.effect_entered_at_micros(),
        observed_at_micros: state.observed_at_micros(),
    })
}

fn admit_indexed_widget_visual(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &StrictAuthoredPanelAdmission,
    element_id: &str,
    state: ViewerOwnedChromeState,
) -> Result<AdmittedWidgetVisual, WidgetVisualRejection> {
    let surface = index
        .validate_panel_token(admission, panel)
        .map_err(|_| WidgetVisualRejection::UnindexedElement)?;
    let element = surface
        .elements
        .iter()
        .find(|element| element.element_id == element_id)
        .ok_or(WidgetVisualRejection::UnindexedElement)?;
    let family = WidgetFamily::from_element_type(&element.element_type)
        .ok_or(WidgetVisualRejection::ElementFamilyMismatch)?;
    if state == ViewerOwnedChromeState::Passive && family.has_sprite_surface() {
        return Err(WidgetVisualRejection::UnsupportedViewerChromeState);
    }
    let frame_count = if family.has_sprite_surface() {
        let [sprite] = element.sprites.as_slice() else {
            return Err(WidgetVisualRejection::SpriteFrameCountMismatch);
        };
        usize::try_from(sprite.frame_count)
            .map_err(|_| WidgetVisualRejection::SpriteFrameCountMismatch)?
    } else {
        0
    };
    admit_widget_visual(&indexed_widget_visual_request(
        format!(
            "viewer-owned:{}:{}:{}",
            index.index_identity_sha256,
            panel.window_id(),
            element_id
        ),
        family,
        frame_count,
        element.authored_one_based_frame,
        state,
    )?)
}

fn indexed_widget_visual_request(
    source_record_id: String,
    family: WidgetFamily,
    frame_count: usize,
    authored_one_based_frame: u32,
    state: ViewerOwnedChromeState,
) -> Result<WidgetVisualRequest, WidgetVisualRejection> {
    let (presentation, interaction, is_current_hover_target) = match state {
        ViewerOwnedChromeState::Passive => (
            ReadOnlyPresentation::Passive,
            family
                .standard_button_carrier()
                .then_some(ButtonVisualState::Up),
            false,
        ),
        ViewerOwnedChromeState::Navigation {
            interaction,
            is_current_hover_target,
        } if family.standard_button_carrier() && interaction != ButtonVisualState::Disable => (
            ReadOnlyPresentation::Navigation,
            Some(interaction),
            is_current_hover_target,
        ),
        ViewerOwnedChromeState::Disabled if family.standard_button_carrier() => (
            ReadOnlyPresentation::Disabled,
            Some(ButtonVisualState::Disable),
            false,
        ),
        ViewerOwnedChromeState::Navigation { .. } | ViewerOwnedChromeState::Disabled => {
            return Err(WidgetVisualRejection::UnsupportedViewerChromeState);
        }
    };
    Ok(WidgetVisualRequest {
        source_record_id,
        family,
        presentation,
        interaction,
        is_current_hover_target,
        frame_count,
        one_based_frame: family
            .standard_button_carrier()
            .then_some(authored_one_based_frame),
        selected: false,
        active: false,
        toggle: None,
        attention: None,
        progress: None,
        drag: None,
    })
}

#[cfg(test)]
mod indexed_lifecycle_tests {
    use super::*;
    use crate::window_manager_law::{
        DormantWindowDefinition, DormantWindowManagerRegistry, ReplayOwnedWindowManager,
        ReplayPanelActivation,
    };
    use std::collections::BTreeSet;

    const TEST_WINDOW_ID: &str = "interface/test.gui#00001";

    fn identity(path: &[u32]) -> AuthenticatedWidgetIdentity {
        let path_id = path
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(".");
        AuthenticatedWidgetIdentity {
            index_identity_sha256: "a".repeat(64),
            window_id: TEST_WINDOW_ID.into(),
            surface_id: "test_root".into(),
            element_id: format!("button@{path_id}"),
            authored_name: "button".into(),
            element_type: "buttonType".into(),
            child_path: path.to_vec(),
        }
    }

    fn manager() -> ReplayOwnedWindowManager {
        let registry = DormantWindowManagerRegistry {
            windows: vec![DormantWindowDefinition {
                window_id: TEST_WINDOW_ID.into(),
                source_file: "interface/test.gui".into(),
                element_type: "windowType".into(),
                name: "test".into(),
                hierarchy_path: "00001".into(),
                downstream_runtime_dependencies: BTreeSet::new(),
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
            }],
        };
        ReplayOwnedWindowManager::new(&registry).unwrap()
    }

    fn open(manager: &mut ReplayOwnedWindowManager) {
        manager
            .open(ReplayPanelActivation {
                window_id: TEST_WINDOW_ID.into(),
                modal_owner: None,
                initial_focus: None,
                keyboard_traversal: Vec::new(),
                controller_traversal: Vec::new(),
                tooltip_targets: Vec::new(),
            })
            .unwrap();
    }

    #[test]
    fn half_open_geometry_never_double_hits_a_shared_edge() {
        let mut manager = manager();
        open(&mut manager);
        let lease = manager
            .authenticated_runtime_instance(TEST_WINDOW_ID)
            .unwrap();
        let runtime_instance_token = lease.runtime_instance_token().to_string();
        let left = AuthenticatedWidgetGeometry {
            identity: identity(&[0]),
            runtime_instance_token: runtime_instance_token.clone(),
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        };
        let right = AuthenticatedWidgetGeometry {
            identity: identity(&[1]),
            runtime_instance_token: runtime_instance_token.clone(),
            x: 40,
            y: 20,
            width: 30,
            height: 40,
        };
        assert!(!left.contains_half_open(&lease, (40, 30)));
        assert!(right.contains_half_open(&lease, (40, 30)));
        assert!(!left.contains_half_open(&lease, (39, 60)));
        let overflowing = AuthenticatedWidgetGeometry {
            identity: identity(&[2]),
            runtime_instance_token,
            x: i32::MAX,
            y: 0,
            width: 1,
            height: 1,
        };
        assert!(!overflowing.contains_half_open(&lease, (i32::MAX, 0)));
        let mut independent = self::manager();
        open(&mut independent);
        let independent_lease = independent
            .authenticated_runtime_instance(TEST_WINDOW_ID)
            .unwrap();
        assert!(
            !right.contains_half_open(&independent_lease, (40, 30)),
            "geometry sealed by one manager lifetime must not hit-test under another"
        );
    }

    #[test]
    fn effect_epoch_changes_only_with_effect_or_runtime_instance() {
        let first = identity(&[0]);
        let lifecycle = |hovered, pressed, focused| ViewerWidgetLifecycle {
            visible: true,
            enabled: true,
            hovered,
            pressed,
            focused,
        };
        let up = advance_authenticated_viewer_widget_state_for_identity(
            None,
            &first,
            "open:7",
            1,
            lifecycle(false, false, false),
            100,
        )
        .unwrap();
        let focused = advance_authenticated_viewer_widget_state_for_identity(
            Some(&up),
            &first,
            "open:7",
            2,
            lifecycle(false, false, true),
            110,
        )
        .unwrap();
        assert_eq!(focused.effect_entered_at_micros, 100);
        assert_eq!(
            advance_authenticated_viewer_widget_state_for_identity(
                Some(&focused),
                &first,
                "open:7",
                3,
                lifecycle(false, false, true),
                109,
            ),
            Err(WidgetVisualRejection::InvalidViewerLifecycle)
        );

        let over = advance_authenticated_viewer_widget_state_for_identity(
            Some(&focused),
            &first,
            "open:7",
            3,
            lifecycle(true, false, true),
            120,
        )
        .unwrap();
        assert_eq!(over.effect_entered_at_micros, 120);
        let down = advance_authenticated_viewer_widget_state_for_identity(
            Some(&over),
            &first,
            "open:7",
            4,
            lifecycle(true, true, true),
            130,
        )
        .unwrap();
        assert_eq!(down.effect_entered_at_micros, 130);
        let reopened = advance_authenticated_viewer_widget_state_for_identity(
            Some(&down),
            &first,
            "open:8",
            5,
            lifecycle(true, true, true),
            140,
        )
        .unwrap();
        assert_eq!(reopened.effect_entered_at_micros, 140);
    }

    #[test]
    fn authored_frame_two_survives_every_standard_button_effect() {
        for state in [
            ViewerOwnedChromeState::Navigation {
                interaction: ButtonVisualState::Up,
                is_current_hover_target: false,
            },
            ViewerOwnedChromeState::Navigation {
                interaction: ButtonVisualState::Down,
                is_current_hover_target: true,
            },
            ViewerOwnedChromeState::Navigation {
                interaction: ButtonVisualState::Over,
                is_current_hover_target: true,
            },
            ViewerOwnedChromeState::Disabled,
        ] {
            let request = indexed_widget_visual_request(
                "viewer-owned:test".into(),
                WidgetFamily::Button,
                4,
                2,
                state,
            )
            .unwrap();
            let admitted = admit_widget_visual(&request).unwrap();
            assert_eq!(admitted.frame().unwrap().one_based_frame, 2);
        }
    }
}
