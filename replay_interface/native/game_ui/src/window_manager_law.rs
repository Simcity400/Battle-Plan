
use crate::authored_role_index::{
    AuthenticatedGuiRoleIndex, AuthenticatedPanelSurface, StrictAuthoredPanelAdmission,
};
use crate::compose::{
    authenticate_persistent_hud_surfaces, authenticated_dynamic_sprite_natural_size,
    authenticated_template_child_rect, authenticated_template_grid_slot_rect,
    authenticated_template_root_extent, compose_authenticated_surface_batch,
    seal_authenticated_materialization_plan, seal_authenticated_panel_semantics,
    AuthenticatedCollectionScrollRegionPlan, AuthenticatedComposeReport, AuthenticatedElementRect,
    AuthenticatedElementSemantic, AuthenticatedExtendedScrollbarPlan, AuthenticatedLayoutPreflight,
    AuthenticatedMaterializationPlan, AuthenticatedMaterializedInstancePlan,
    AuthenticatedPanelBatchInput, AuthenticatedPanelComposeRequest, AuthenticatedScrollbarPartPlan,
    ComposeOptions, Compositor, Rect,
};
use crate::dds::RgbaImage;
use crate::definition_admission::{GuiDefinitionCatalog, FROZEN_FILE_COUNT, FROZEN_WINDOW_COUNT};
use crate::gfx::SpriteRegistry;
use crate::observer_collection::{
    plan_observer_collection, AuthoredCollectionSource, AuthoredRowBinding,
    ObserverSemanticCollection, ObserverSemanticRow, StandardGridAuthority, ViewerCollectionState,
};
use crate::scrollbar_law::{
    extended_thumb_pixel, ExtendedScrollbarCarrier, ScrollbarAxis, ScrollbarCarrierAuthority,
};
use crate::widget_visual_state::{
    advance_authenticated_viewer_widget_state, AuthenticatedViewerWidgetState, ProgressFill,
    ViewerWidgetLifecycle, WidgetFamily, WidgetVisualRejection,
};
use replay_core::ValidatedContentAdmission;
use serde::Deserialize;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

static NEXT_REPLAY_MANAGER_ORDINAL: AtomicU64 = AtomicU64::new(0);
static NEXT_REPLAY_ENGINE_INSTANCE_TOKEN: AtomicU64 = AtomicU64::new(0);

pub const XUI02_ACTIVE_GUI_FILES: usize = FROZEN_FILE_COUNT;
pub const XUI02_WINDOW_DEFINITIONS: usize = FROZEN_WINDOW_COUNT;
pub const XUI02_ADMITTED_DEFINITIONS: usize = 9_155;
pub const XUI02_REJECTED_DEFINITIONS: usize = 0;
pub const XUI02_RUNTIME_ACTIVATIONS: usize = 0;
pub const XUI02_DEPENDENCY_WINDOWS: usize = 190;
pub const XUI02_DEPENDENCY_FILES: usize = 77;
const ACTIVATION_INVENTORY_JSON: &str = include_str!("xui02_activation_inventory.json");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeWindowManagerState {
    pub window_id: String,
    pub producer_packet_id: String,
    pub producer_manifest_sha256: String,
    pub producer_record: String,
    pub open: bool,
    pub visible: bool,
    pub composed: bool,
    pub modal_owner: Option<String>,
    pub stack_ordinal: Option<u32>,
    pub focus_owner: Option<String>,
    pub restore_focus_to: Option<String>,
    pub tooltip_owner: Option<String>,
    pub tooltip_instance_record: Option<String>,
    pub variant_envelope_record: Option<String>,
    pub keyboard_traversal: Vec<String>,
    pub controller_traversal: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DormantWindowDefinition {
    pub window_id: String,
    pub source_file: String,
    pub element_type: String,
    pub name: String,
    pub hierarchy_path: String,
    pub downstream_runtime_dependencies: BTreeSet<String>,
    pub open: bool,
    pub visible: bool,
    pub composed: bool,
    pub modal_owner: Option<String>,
    pub stack_ordinal: Option<u32>,
    pub focus_owner: Option<String>,
    pub restore_focus_to: Option<String>,
    pub tooltip_owner: Option<String>,
    pub keyboard_traversal: Vec<String>,
    pub controller_traversal: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DormantWindowManagerRegistry {
    pub windows: Vec<DormantWindowDefinition>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResolvableActivationClass {
    ExactControllerWindow,
    TypedFileControllerRoot,
    ExplicitStateSeamRoot,
    ScriptedGuiPredicateRoot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeActivationSource {
    pub window_id: String,
    pub class: ResolvableActivationClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedRuntimeWindowState {
    pub window_id: String,
    pub activation_source_window_id: String,
    pub inherited_from_source: bool,
    pub producer_packet_id: String,
    pub producer_manifest_sha256: String,
    pub producer_record: String,
    pub open: bool,
    pub visible: bool,
    pub composed: bool,
    pub modal_owner: Option<String>,
    pub stack_ordinal: Option<u32>,
    pub focus_owner: Option<String>,
    pub restore_focus_to: Option<String>,
    pub tooltip_owner: Option<String>,
    pub tooltip_instance_record: Option<String>,
    pub variant_envelope_record: Option<String>,
    pub keyboard_traversal: Vec<String>,
    pub controller_traversal: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeActivationDependency {
    AwaitingResolvableControllerFrame(String),
    AwaitingDataProducer {
        activation_class: String,
        dossiers: Vec<String>,
    },
    DownstreamOwners(Vec<String>),
    StaticHelperHasNoRuntimeProducer,
    NoRuntimeProducer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedRuntimeWindowState {
    pub window_id: String,
    pub effective_activation_source_window_id: String,
    pub dependency: RuntimeActivationDependency,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveWindowManagerFrame {
    pub resolved: Vec<ResolvedRuntimeWindowState>,
    pub blocked: Vec<BlockedRuntimeWindowState>,
}

#[derive(Deserialize)]
struct FrozenActivationInventory {
    windows: Vec<FrozenActivationWindow>,
}

#[derive(Deserialize)]
struct FrozenActivationWindow {
    window_id: String,
    effective_activation_source_window_id: String,
    effective_activation_class: String,
    downstream_runtime_dependency_owners: Vec<String>,
    runtime_data_dependencies: Vec<String>,
}

impl DormantWindowManagerRegistry {
    pub fn dependency_window_count(&self) -> usize {
        self.windows
            .iter()
            .filter(|row| !row.downstream_runtime_dependencies.is_empty())
            .count()
    }

    pub fn dependency_file_count(&self) -> usize {
        self.windows
            .iter()
            .filter(|row| !row.downstream_runtime_dependencies.is_empty())
            .map(|row| row.source_file.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowManagerRejection {
    NamespaceDrift,
    DefinitionRejectionsPresent,
    DuplicateWindowIdentity(String),
    DependencyClassificationDrift,
    UnknownManagerStateWindow(String),
    RuntimeStateRequiresTypedOwnerAdmission(String),
    ActivationInventoryDrift,
    DuplicateRuntimeState(String),
    UnresolvableActivationSource(String),
    InvalidRuntimeState(String),
    InvalidModalOwner(String),
}

impl fmt::Display for WindowManagerRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NamespaceDrift => write!(formatter, "XUI02 GUI definition namespace drift"),
            Self::DefinitionRejectionsPresent => {
                write!(formatter, "XUI02 requires zero rejected GUI definitions")
            }
            Self::DuplicateWindowIdentity(window_id) => {
                write!(formatter, "XUI02 duplicate window identity: {window_id}")
            }
            Self::DependencyClassificationDrift => {
                write!(
                    formatter,
                    "XUI02 downstream dependency classification drift"
                )
            }
            Self::UnknownManagerStateWindow(window_id) => {
                write!(
                    formatter,
                    "XUI02 manager state names unknown window: {window_id}"
                )
            }
            Self::RuntimeStateRequiresTypedOwnerAdmission(window_id) => write!(
                formatter,
                "XUI02 runtime state requires typed owner admission: {window_id}"
            ),
            Self::ActivationInventoryDrift => {
                write!(formatter, "XUI02 activation inventory drift")
            }
            Self::DuplicateRuntimeState(window_id) => {
                write!(formatter, "XUI02 duplicate runtime state: {window_id}")
            }
            Self::UnresolvableActivationSource(window_id) => {
                write!(formatter, "XUI02 unresolved activation source: {window_id}")
            }
            Self::InvalidRuntimeState(window_id) => {
                write!(formatter, "XUI02 invalid runtime state: {window_id}")
            }
            Self::InvalidModalOwner(window_id) => {
                write!(formatter, "XUI02 invalid modal owner for: {window_id}")
            }
        }
    }
}

impl Error for WindowManagerRejection {}

pub fn register_dormant_window_manager(
    catalog: &GuiDefinitionCatalog,
    runtime_states: &[RuntimeWindowManagerState],
) -> Result<DormantWindowManagerRegistry, WindowManagerRejection> {
    if catalog.rejected_file_count() != 0
        || catalog.rejected_windows.len() != XUI02_REJECTED_DEFINITIONS
    {
        return Err(WindowManagerRejection::DefinitionRejectionsPresent);
    }
    if catalog.files.len() != XUI02_ACTIVE_GUI_FILES
        || catalog.admitted_file_count() != XUI02_ACTIVE_GUI_FILES
        || catalog.total_window_count() != XUI02_WINDOW_DEFINITIONS
        || catalog.admitted_windows.len() != XUI02_ADMITTED_DEFINITIONS
    {
        return Err(WindowManagerRejection::NamespaceDrift);
    }
    let registry = register_effective_dormant_window_manager(catalog, runtime_states)?;
    if registry.windows.len() != XUI02_WINDOW_DEFINITIONS
        || registry.dependency_window_count() != XUI02_DEPENDENCY_WINDOWS
        || registry.dependency_file_count() != XUI02_DEPENDENCY_FILES
    {
        return Err(WindowManagerRejection::DependencyClassificationDrift);
    }
    Ok(registry)
}

pub fn register_effective_dormant_window_manager(
    catalog: &GuiDefinitionCatalog,
    runtime_states: &[RuntimeWindowManagerState],
) -> Result<DormantWindowManagerRegistry, WindowManagerRejection> {
    if catalog.files.is_empty()
        || catalog.admitted_windows.is_empty()
        || catalog.rejected_file_count() != 0
        || catalog.admitted_file_count() != catalog.files.len()
    {
        return Err(WindowManagerRejection::DefinitionRejectionsPresent);
    }
    let mut file_rows = BTreeMap::new();
    for file in &catalog.files {
        if file_rows
            .insert(
                file.source_file.as_str(),
                (file.admitted_windows, file.rejected_windows),
            )
            .is_some()
        {
            return Err(WindowManagerRejection::DefinitionRejectionsPresent);
        }
    }
    let mut actual_admitted_by_file = file_rows
        .keys()
        .map(|source| (*source, 0usize))
        .collect::<BTreeMap<_, _>>();
    for row in &catalog.admitted_windows {
        let Some(count) = actual_admitted_by_file.get_mut(row.source_file.as_str()) else {
            return Err(WindowManagerRejection::NamespaceDrift);
        };
        *count += 1;
    }
    let mut actual_rejected_by_file = file_rows
        .keys()
        .map(|source| (*source, 0usize))
        .collect::<BTreeMap<_, _>>();
    for row in &catalog.rejected_windows {
        let Some(count) = actual_rejected_by_file.get_mut(row.source_file.as_str()) else {
            return Err(WindowManagerRejection::NamespaceDrift);
        };
        *count += 1;
    }
    if file_rows.iter().any(|(source, (admitted, rejected))| {
        actual_admitted_by_file.get(*source) != Some(admitted)
            || actual_rejected_by_file.get(*source) != Some(rejected)
    }) {
        return Err(WindowManagerRejection::NamespaceDrift);
    }
    let mut ids = BTreeSet::new();
    for row in &catalog.admitted_windows {
        if !ids.insert(row.window_id.as_str()) {
            return Err(WindowManagerRejection::DuplicateWindowIdentity(
                row.window_id.clone(),
            ));
        }
    }
    if let Some(collision) = catalog
        .rejected_windows
        .iter()
        .find(|row| ids.contains(row.window_id.as_str()))
    {
        return Err(WindowManagerRejection::DuplicateWindowIdentity(
            collision.window_id.clone(),
        ));
    }
    for state in runtime_states {
        if !ids.contains(state.window_id.as_str()) {
            return Err(WindowManagerRejection::UnknownManagerStateWindow(
                state.window_id.clone(),
            ));
        }
        return Err(
            WindowManagerRejection::RuntimeStateRequiresTypedOwnerAdmission(
                state.window_id.clone(),
            ),
        );
    }

    Ok(DormantWindowManagerRegistry {
        windows: catalog
            .admitted_windows
            .iter()
            .map(|row| DormantWindowDefinition {
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
            })
            .collect(),
    })
}

fn activation_class(value: &str) -> Option<ResolvableActivationClass> {
    match value {
        "EXACT_CONTROLLER_WINDOW" => Some(ResolvableActivationClass::ExactControllerWindow),
        "TYPED_FILE_CONTROLLER_ROOT" => Some(ResolvableActivationClass::TypedFileControllerRoot),
        "EXPLICIT_STATE_SEAM_ROOT" => Some(ResolvableActivationClass::ExplicitStateSeamRoot),
        "SCRIPTED_GUI_PREDICATE_ROOT" => Some(ResolvableActivationClass::ScriptedGuiPredicateRoot),
        _ => None,
    }
}

fn parsed_activation_inventory(
) -> Result<&'static FrozenActivationInventory, WindowManagerRejection> {
    static PARSED: OnceLock<Result<FrozenActivationInventory, WindowManagerRejection>> =
        OnceLock::new();
    PARSED
        .get_or_init(|| {
            serde_json::from_str(ACTIVATION_INVENTORY_JSON)
                .map_err(|_| WindowManagerRejection::ActivationInventoryDrift)
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn activation_inventory(
    registry: &DormantWindowManagerRegistry,
) -> Result<&'static FrozenActivationInventory, WindowManagerRejection> {
    let inventory = parsed_activation_inventory()?;
    let registry_ids = registry
        .windows
        .iter()
        .map(|row| row.window_id.as_str())
        .collect::<BTreeSet<_>>();
    let inventory_ids = inventory
        .windows
        .iter()
        .map(|row| row.window_id.as_str())
        .collect::<BTreeSet<_>>();
    if registry_ids.len() != XUI02_WINDOW_DEFINITIONS
        || inventory.windows.len() != XUI02_WINDOW_DEFINITIONS
        || inventory_ids != registry_ids
    {
        return Err(WindowManagerRejection::ActivationInventoryDrift);
    }
    Ok(inventory)
}

pub fn resolvable_activation_sources(
    registry: &DormantWindowManagerRegistry,
) -> Result<Vec<RuntimeActivationSource>, WindowManagerRejection> {
    let inventory = activation_inventory(registry)?;
    Ok(resolvable_sources_from(inventory))
}

fn resolvable_sources_from(inventory: &FrozenActivationInventory) -> Vec<RuntimeActivationSource> {
    let mut sources = BTreeMap::new();
    for row in &inventory.windows {
        if let Some(class) = activation_class(&row.effective_activation_class) {
            sources
                .entry(row.effective_activation_source_window_id.clone())
                .and_modify(|current| {
                    if *current != class {
                        *current = class;
                    }
                })
                .or_insert(class);
        }
    }
    sources
        .into_iter()
        .map(|(window_id, class)| RuntimeActivationSource { window_id, class })
        .collect()
}

fn nonblank(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(|text| !text.trim().is_empty())
}

fn valid_traversal(values: &[String]) -> bool {
    values.iter().all(|value| !value.trim().is_empty())
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn validate_runtime_state(state: &RuntimeWindowManagerState) -> bool {
    let has_variant_state = state.focus_owner.is_some()
        || state.restore_focus_to.is_some()
        || !state.keyboard_traversal.is_empty()
        || !state.controller_traversal.is_empty();
    if state.producer_packet_id.trim().is_empty()
        || !crate::is_lowercase_sha256(&state.producer_manifest_sha256)
        || state.producer_record.trim().is_empty()
        || !nonblank(&state.modal_owner)
        || !nonblank(&state.focus_owner)
        || !nonblank(&state.restore_focus_to)
        || !nonblank(&state.tooltip_owner)
        || !nonblank(&state.tooltip_instance_record)
        || !nonblank(&state.variant_envelope_record)
        || !valid_traversal(&state.keyboard_traversal)
        || !valid_traversal(&state.controller_traversal)
        || state.tooltip_owner.is_some() != state.tooltip_instance_record.is_some()
        || has_variant_state != state.variant_envelope_record.is_some()
    {
        return false;
    }
    if !state.open {
        return !state.visible
            && !state.composed
            && state.modal_owner.is_none()
            && state.stack_ordinal.is_none()
            && state.focus_owner.is_none()
            && state.restore_focus_to.is_none()
            && state.tooltip_owner.is_none()
            && state.tooltip_instance_record.is_none()
            && state.variant_envelope_record.is_none()
            && state.keyboard_traversal.is_empty()
            && state.controller_traversal.is_empty();
    }
    if state.visible && !state.composed {
        return false;
    }
    if state.composed != state.stack_ordinal.is_some() {
        return false;
    }
    true
}

pub fn apply_runtime_activation_frame(
    registry: &DormantWindowManagerRegistry,
    runtime_states: &[RuntimeWindowManagerState],
) -> Result<ActiveWindowManagerFrame, WindowManagerRejection> {
    let inventory = activation_inventory(registry)?;
    let sources = resolvable_sources_from(inventory)
        .into_iter()
        .map(|source| (source.window_id, source.class))
        .collect::<BTreeMap<_, _>>();
    let mut states = BTreeMap::new();
    for state in runtime_states {
        if !sources.contains_key(&state.window_id) {
            return Err(WindowManagerRejection::UnresolvableActivationSource(
                state.window_id.clone(),
            ));
        }
        if !validate_runtime_state(state) {
            return Err(WindowManagerRejection::InvalidRuntimeState(
                state.window_id.clone(),
            ));
        }
        if states.insert(state.window_id.clone(), state).is_some() {
            return Err(WindowManagerRejection::DuplicateRuntimeState(
                state.window_id.clone(),
            ));
        }
    }
    for state in states.values() {
        if let Some(owner) = &state.modal_owner {
            let Some(owner_state) = states.get(owner) else {
                return Err(WindowManagerRejection::InvalidModalOwner(
                    state.window_id.clone(),
                ));
            };
            if owner == &state.window_id || !owner_state.open || !owner_state.composed {
                return Err(WindowManagerRejection::InvalidModalOwner(
                    state.window_id.clone(),
                ));
            }
        }
    }

    let by_id = inventory
        .windows
        .iter()
        .map(|row| (row.window_id.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let mut resolved = Vec::new();
    let mut blocked = Vec::new();
    for row in &inventory.windows {
        if let Some(state) = states.get(&row.effective_activation_source_window_id) {
            let inherited = row.window_id != row.effective_activation_source_window_id;
            resolved.push(ResolvedRuntimeWindowState {
                window_id: row.window_id.clone(),
                activation_source_window_id: row.effective_activation_source_window_id.clone(),
                inherited_from_source: inherited,
                producer_packet_id: state.producer_packet_id.clone(),
                producer_manifest_sha256: state.producer_manifest_sha256.clone(),
                producer_record: state.producer_record.clone(),
                open: state.open,
                visible: state.visible,
                composed: state.composed,
                modal_owner: (!inherited).then(|| state.modal_owner.clone()).flatten(),
                stack_ordinal: (!inherited).then_some(state.stack_ordinal).flatten(),
                focus_owner: (!inherited).then(|| state.focus_owner.clone()).flatten(),
                restore_focus_to: (!inherited)
                    .then(|| state.restore_focus_to.clone())
                    .flatten(),
                tooltip_owner: (!inherited).then(|| state.tooltip_owner.clone()).flatten(),
                tooltip_instance_record: (!inherited)
                    .then(|| state.tooltip_instance_record.clone())
                    .flatten(),
                variant_envelope_record: (!inherited)
                    .then(|| state.variant_envelope_record.clone())
                    .flatten(),
                keyboard_traversal: if inherited {
                    Vec::new()
                } else {
                    state.keyboard_traversal.clone()
                },
                controller_traversal: if inherited {
                    Vec::new()
                } else {
                    state.controller_traversal.clone()
                },
            });
            continue;
        }
        let source = by_id
            .get(row.effective_activation_source_window_id.as_str())
            .ok_or(WindowManagerRejection::ActivationInventoryDrift)?;
        let dependency = if activation_class(&row.effective_activation_class).is_some() {
            if source.runtime_data_dependencies.is_empty() {
                RuntimeActivationDependency::AwaitingResolvableControllerFrame(
                    row.effective_activation_class.clone(),
                )
            } else {
                RuntimeActivationDependency::AwaitingDataProducer {
                    activation_class: row.effective_activation_class.clone(),
                    dossiers: source.runtime_data_dependencies.clone(),
                }
            }
        } else {
            match row.effective_activation_class.as_str() {
                "DOWNSTREAM_OWNER_ROOT" => RuntimeActivationDependency::DownstreamOwners(
                    source.downstream_runtime_dependency_owners.clone(),
                ),
                "STATIC_HELPER_ROOT_NO_RUNTIME_PRODUCER" => {
                    RuntimeActivationDependency::StaticHelperHasNoRuntimeProducer
                }
                "UNRESOLVED_ROOT_NO_RUNTIME_PRODUCER" => {
                    RuntimeActivationDependency::NoRuntimeProducer
                }
                _ => return Err(WindowManagerRejection::ActivationInventoryDrift),
            }
        };
        blocked.push(BlockedRuntimeWindowState {
            window_id: row.window_id.clone(),
            effective_activation_source_window_id: row
                .effective_activation_source_window_id
                .clone(),
            dependency,
        });
    }
    if resolved.len() + blocked.len() != XUI02_WINDOW_DEFINITIONS {
        return Err(WindowManagerRejection::ActivationInventoryDrift);
    }
    Ok(ActiveWindowManagerFrame { resolved, blocked })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayTraversalDevice {
    Keyboard,
    Controller,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayTraversalDirection {
    Forward,
    Backward,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayFocusTarget {
    pub window_id: String,
    pub widget_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayTooltipOwnership {
    pub window_id: String,
    pub widget_id: String,
    /// presentation lifetime; tooltip content remains validated separately.
    pub tooltip_instance_record: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedReplayTooltipOwner {
    window_id: String,
    runtime_instance_token: String,
    engine_instance_token: u64,
    source_file: String,
    surface_id: String,
    element_id: String,
    authored_name: String,
    child_path: Vec<u32>,
    element_type: String,
    screen_rect: Rect,
    clip_rect: Rect,
    stack_ordinal: u32,
    paint_ordinal: u32,
    frame_generation: u64,
    alternate_selection: ReplayTooltipAlternateSelection,
}

impl AuthenticatedReplayTooltipOwner {
    pub fn window_id(&self) -> &str {
        &self.window_id
    }

    pub fn runtime_instance_token(&self) -> &str {
        &self.runtime_instance_token
    }

    pub fn engine_instance_token(&self) -> u64 {
        self.engine_instance_token
    }

    pub fn element_id(&self) -> &str {
        &self.element_id
    }

    pub fn source_file(&self) -> &str {
        &self.source_file
    }

    pub fn surface_id(&self) -> &str {
        &self.surface_id
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

    pub fn screen_rect(&self) -> Rect {
        self.screen_rect
    }

    pub fn clip_rect(&self) -> Rect {
        self.clip_rect
    }

    pub fn frame_generation(&self) -> u64 {
        self.frame_generation
    }

    pub fn stack_ordinal(&self) -> u32 {
        self.stack_ordinal
    }

    pub fn paint_ordinal(&self) -> u32 {
        self.paint_ordinal
    }

    pub fn alternate_selection(&self) -> ReplayTooltipAlternateSelection {
        self.alternate_selection
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayPanelActivation {
    pub window_id: String,
    pub modal_owner: Option<String>,
    pub initial_focus: Option<String>,
    pub keyboard_traversal: Vec<String>,
    pub controller_traversal: Vec<String>,
    /// payload. Empty means the panel admits no tooltip ownership.
    pub tooltip_targets: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayPanelState {
    pub window_id: String,
    /// Viewer-owned, pointer-free identity for this particular open lifetime.
    runtime_instance_token: Option<String>,
    pub open: bool,
    pub visible: bool,
    pub composed: bool,
    pub stack_ordinal: Option<u32>,
    pub modal_owner: Option<String>,
    pub focused_widget: Option<String>,
    pub keyboard_traversal: Vec<String>,
    pub controller_traversal: Vec<String>,
    pub tooltip_targets: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayPointerDisposition {
    Interactive,
    BlockOnly,
    PassThrough,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayElementPointerAuthority {
    PassThrough,
    BlockOnly { visual_enabled: Option<bool> },
    Interactive { visual_enabled: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayTooltipAlternateSelection {
    AuthoredDefault,
    AuthoredRuntimeAlternate,
    CustomProvider,
    UnavailableAuthoredAlternate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoliticsGameplayMutation {
    CreateFaction,
    DismantleFaction,
    CancelNationalFocus,
    CancelContinuousFocus,
    DecreaseAutonomy,
    IncreaseAutonomy,
    AssignOrReplaceIdea,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoliticsViewerAction {
    ClosePolitics,
    OpenFocusTree,
    OpenFactionOverview,
    OpenCurrentFaction,
    OpenPowerBalance,
    OpenAutonomyCountry,
    OpenOccupiedTerritories,
    OpenSubjects,
    OpenExilesAndCollaborations,
    OpenIdeaDetail,
    OpenIdeaRoster,
    RefuseGameplayMutation(PoliticsGameplayMutation),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportViewerAction {
    SetPlaybackSpeedStep(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapmodeViewerAction {
    SelectMapmodeButton(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TheatreViewerAction {
    OpenLandSettings(u32),
    CloseLandSettings(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProductionLinesViewerAction {
    Close,
    ToggleRow(crate::production_lines::Xui12RowIdentity),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayViewerAction {
    Politics(PoliticsViewerAction),
    Transport(TransportViewerAction),
    Mapmode(MapmodeViewerAction),
    Theatre(TheatreViewerAction),
    ProductionLines(ProductionLinesViewerAction),
}

impl ReplayViewerAction {
    pub(crate) fn digest_record(&self) -> Vec<u8> {
        match self {
            Self::Politics(action) => {
                let token = match action {
                    PoliticsViewerAction::ClosePolitics => "close",
                    PoliticsViewerAction::OpenFocusTree => "open-focus-tree",
                    PoliticsViewerAction::OpenFactionOverview => "open-faction-overview",
                    PoliticsViewerAction::OpenCurrentFaction => "open-current-faction",
                    PoliticsViewerAction::OpenPowerBalance => "open-power-balance",
                    PoliticsViewerAction::OpenAutonomyCountry => "open-autonomy-country",
                    PoliticsViewerAction::OpenOccupiedTerritories => "open-occupied-territories",
                    PoliticsViewerAction::OpenSubjects => "open-subjects",
                    PoliticsViewerAction::OpenExilesAndCollaborations => {
                        "open-exiles-and-collaborations"
                    }
                    PoliticsViewerAction::OpenIdeaDetail => "open-idea-detail",
                    PoliticsViewerAction::OpenIdeaRoster => "open-idea-roster",
                    PoliticsViewerAction::RefuseGameplayMutation(mutation) => match mutation {
                        PoliticsGameplayMutation::CreateFaction => "refuse-create-faction",
                        PoliticsGameplayMutation::DismantleFaction => "refuse-dismantle-faction",
                        PoliticsGameplayMutation::CancelNationalFocus => {
                            "refuse-cancel-national-focus"
                        }
                        PoliticsGameplayMutation::CancelContinuousFocus => {
                            "refuse-cancel-continuous-focus"
                        }
                        PoliticsGameplayMutation::DecreaseAutonomy => "refuse-decrease-autonomy",
                        PoliticsGameplayMutation::IncreaseAutonomy => "refuse-increase-autonomy",
                        PoliticsGameplayMutation::AssignOrReplaceIdea => {
                            "refuse-assign-or-replace-idea"
                        }
                    },
                };
                format!("politics:{token}").into_bytes()
            }
            Self::Transport(TransportViewerAction::SetPlaybackSpeedStep(step)) => {
                format!("transport:speed:{step}").into_bytes()
            }
            Self::Mapmode(MapmodeViewerAction::SelectMapmodeButton(ordinal)) => {
                format!("mapmode:select:{ordinal}").into_bytes()
            }
            Self::Theatre(TheatreViewerAction::OpenLandSettings(object_id)) => {
                format!("theatre:open-land-settings:{object_id}").into_bytes()
            }
            Self::Theatre(TheatreViewerAction::CloseLandSettings(object_id)) => {
                format!("theatre:close-land-settings:{object_id}").into_bytes()
            }
            Self::ProductionLines(ProductionLinesViewerAction::Close) => {
                b"production-lines:close".to_vec()
            }
            Self::ProductionLines(ProductionLinesViewerAction::ToggleRow(identity)) => {
                format!("production-lines:toggle-row:{identity:?}").into_bytes()
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayViewerActionBinding {
    authored_tree_identity_sha256: String,
    authored_template_child_path: Vec<u32>,
    rendered_child_path: Vec<u32>,
    action: ReplayViewerAction,
}

impl ReplayViewerActionBinding {
    pub(crate) fn from_authenticated_owner(
        panel: &StrictAuthoredPanelAdmission,
        authored_template_child_path: Vec<u32>,
        rendered_child_path: Vec<u32>,
        action: ReplayViewerAction,
    ) -> Self {
        Self {
            authored_tree_identity_sha256: panel.authored_tree_identity_sha256().to_string(),
            authored_template_child_path,
            rendered_child_path,
            action,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedReplayViewerAction {
    action: ReplayViewerAction,
    window_id: String,
    runtime_instance_token: String,
    frame_generation: u64,
    input_transition_generation: u64,
    rendered_child_path: Vec<u32>,
}

impl AuthenticatedReplayViewerAction {
    pub fn action(&self) -> &ReplayViewerAction {
        &self.action
    }

    pub fn window_id(&self) -> &str {
        &self.window_id
    }

    pub fn runtime_instance_token(&self) -> &str {
        &self.runtime_instance_token
    }

    pub fn frame_generation(&self) -> u64 {
        self.frame_generation
    }

    pub fn input_transition_generation(&self) -> u64 {
        self.input_transition_generation
    }

    pub fn rendered_child_path(&self) -> &[u32] {
        &self.rendered_child_path
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayAuthenticatedScrollBinding {
    pub element_id: String,
    pub axis: ReplayScrollAxis,
    pub min_offset_pixels: i32,
    pub max_offset_pixels: i32,
    pub offset_pixels: i32,
    pub producer_record: String,
}

/// staged the current frame and panel lifetime.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayAuthenticatedElementSemantic {
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

#[derive(Clone, Debug, PartialEq)]
pub struct ReplayAuthenticatedPersistentHudBinding {
    index_identity_sha256: String,
    manifest_digest_sha256: String,
    product_modset_sha256: String,
    window_id: String,
    surface_id: String,
    viewport_width: usize,
    viewport_height: usize,
    semantics: BTreeMap<String, ReplayAuthenticatedElementSemantic>,
    element_pointer_bindings: BTreeMap<String, ReplayElementPointerAuthority>,
    viewer_actions: Vec<ReplayViewerActionBinding>,
    identity_sha256: String,
}

impl ReplayAuthenticatedPersistentHudBinding {
    pub fn element_pointer_bindings(&self) -> &BTreeMap<String, ReplayElementPointerAuthority> {
        &self.element_pointer_bindings
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayAuthenticatedTheatreBinding {
    index_identity_sha256: String,
    manifest_digest_sha256: String,
    product_modset_sha256: String,
    owner_window_id: String,
    viewport_size: [usize; 2],
    templates: crate::authenticated_semantic_provider::AuthenticatedTheatreTemplateSet,
    surface: crate::production_renderer::ObserverTheatreSurface,
    mode: Option<crate::production_renderer::ObserverTheatreMode>,
    settings_land_id: Option<u32>,
    scroll_offset_pixels: i32,
    identity_sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
struct ReplayAuthenticatedProductionRowBinding {
    input: crate::production_lines::StrictXui12CanvasRow,
    template: StrictAuthoredPanelAdmission,
    expand_child_path: Option<Vec<u32>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplayAuthenticatedScrollbarPartBinding {
    sprite_name: String,
    position: [i32; 2],
    natural_size: [i32; 2],
}

#[derive(Clone, Debug, PartialEq)]
struct ReplayAuthenticatedExtendedScrollbarBinding {
    style_root: crate::gui::GuiElement,
    source_content: replay_core::UsedContentKey,
    position: [i32; 2],
    width: i32,
    background: ReplayAuthenticatedScrollbarPartBinding,
    track: ReplayAuthenticatedScrollbarPartBinding,
    decrease: ReplayAuthenticatedScrollbarPartBinding,
    increase: ReplayAuthenticatedScrollbarPartBinding,
    slider: ReplayAuthenticatedScrollbarPartBinding,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReplayAuthenticatedProductionLinesBinding {
    index_identity_sha256: String,
    manifest_digest_sha256: String,
    product_modset_sha256: String,
    owner_window_id: String,
    viewport_size: [usize; 2],
    country_tag: String,
    completeness_record: String,
    list_element_id: String,
    list_child_path: Vec<u32>,
    grid_child_path: Vec<u32>,
    scrollbar: ReplayAuthenticatedExtendedScrollbarBinding,
    rows: Vec<ReplayAuthenticatedProductionRowBinding>,
    scroll_offset_pixels: i32,
    owner_semantics: BTreeMap<String, ReplayAuthenticatedElementSemantic>,
    owner_pointer_bindings: BTreeMap<String, ReplayElementPointerAuthority>,
    owner_viewer_actions: Vec<ReplayViewerActionBinding>,
    identity_sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
struct ReplayAuthenticatedFrontendCollectionRowBinding {
    input: replay_core::Xui26ReplayCollectionRow,
    template: StrictAuthoredPanelAdmission,
    /// The row's resolved semantics, by child path: the template's authored
    /// derivation with the one proven runtime text store overwritten.
    semantics: BTreeMap<Vec<u32>, ReplayAuthenticatedElementSemantic>,
}

#[derive(Clone, Debug, PartialEq)]
struct ReplayAuthenticatedFrontendCollectionBinding {
    input: replay_core::Xui26ReplayCollection,
    /// DERIVED from the group's authored host law, not carried.
    collection_element_id: String,
    collection_child_path: Vec<u32>,
    empty_template: StrictAuthoredPanelAdmission,
    rows: Vec<ReplayAuthenticatedFrontendCollectionRowBinding>,
    scroll_offset_pixels: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReplayAuthenticatedFrontendCollectionsBinding {
    index_identity_sha256: String,
    manifest_digest_sha256: String,
    product_modset_sha256: String,
    owner_window_id: String,
    family_id: String,
    source_record_id: String,
    frame_hour: i64,
    viewport_size: [usize; 2],
    collections: Vec<ReplayAuthenticatedFrontendCollectionBinding>,
    identity_sha256: String,
}

fn xui26_frontend_collections_binding_digest(
    binding: &ReplayAuthenticatedFrontendCollectionsBinding,
) -> String {
    snapshot::sha256::to_hex(&Sha256::digest(
        format!(
            "XUI26/frontend-collections-v1/{:?}",
            (
                &binding.index_identity_sha256,
                &binding.manifest_digest_sha256,
                &binding.product_modset_sha256,
                &binding.owner_window_id,
                &binding.family_id,
                &binding.source_record_id,
                binding.frame_hour,
                binding.viewport_size,
                &binding.collections,
            )
        )
        .as_bytes(),
    ))
}

impl ReplayAuthenticatedFrontendCollectionsBinding {
    pub fn family_id(&self) -> &str {
        &self.family_id
    }

    pub fn set_scroll_offset_pixels(
        &mut self,
        element_id: &str,
        offset_pixels: i32,
    ) -> Result<(), String> {
        let collection = self
            .collections
            .iter_mut()
            .find(|collection| collection.collection_element_id == element_id)
            .ok_or_else(|| format!("XUI26 scroll outcome names foreign region {element_id:?}"))?;
        if offset_pixels < 0 {
            return Err("XUI26 scroll offset is negative".into());
        }
        collection.scroll_offset_pixels = offset_pixels;
        self.identity_sha256 = xui26_frontend_collections_binding_digest(self);
        Ok(())
    }

    pub fn apply_effective_scroll_offsets(
        &mut self,
        offsets: &BTreeMap<String, i32>,
    ) -> Result<(), String> {
        // A group the carrier measured EMPTY has no scroll region and so no
        // composed offset: it holds no rows to move past its host. Every other
        // group must have exactly one, and no offset may name a group that is
        // not there - so a lost region is still caught, per group, by count.
        let scrollable = self
            .collections
            .iter()
            .filter(|collection| !collection.rows.is_empty())
            .count();
        if offsets.len() != scrollable
            || self.collections.iter().any(|collection| {
                offsets.contains_key(&collection.collection_element_id)
                    == collection.rows.is_empty()
            })
        {
            return Err("XUI26 composed scroll-offset census differs from carrier groups".into());
        }
        for collection in &mut self.collections {
            let Some(offset) = offsets.get(&collection.collection_element_id).copied() else {
                collection.scroll_offset_pixels = 0;
                continue;
            };
            if offset < 0 {
                return Err("XUI26 composed scroll offset is negative".into());
            }
            collection.scroll_offset_pixels = offset;
        }
        self.identity_sha256 = xui26_frontend_collections_binding_digest(self);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MaterializedPointerActionIdentity {
    window_id: String,
    runtime_instance_token: String,
    action: ReplayViewerAction,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MaterializedPointerLifecycle {
    hovered: Option<MaterializedPointerActionIdentity>,
    pressed: Option<MaterializedPointerActionIdentity>,
}

impl ReplayAuthenticatedTheatreBinding {
    pub fn owner_window_id(&self) -> &str {
        &self.owner_window_id
    }

    pub fn scroll_offset_pixels(&self) -> i32 {
        self.scroll_offset_pixels
    }
}

fn theatre_materialization_rejection(window_id: &str) -> ReplayWindowManagerRejection {
    ReplayWindowManagerRejection::InvalidComposedGeometry(format!(
        "{window_id}:theatre-materialization"
    ))
}

fn theatre_read_only_button_interaction(
    authored_name: &str,
) -> crate::runtime_graphics::ButtonVisualState {
    let normalized = authored_name.to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "assign_button"
            | "unmoveable_assign_button"
            | "btn_delete"
            | "split_off_btn"
            | "priority_button"
            | "priority_button_1"
            | "priority_button_2"
            | "priority_button_3"
            | "priority_1"
            | "priority_2"
            | "priority_3"
    ) {
        crate::runtime_graphics::ButtonVisualState::Disable
    } else {
        crate::runtime_graphics::ButtonVisualState::Up
    }
}

fn apply_materialized_pointer_lifecycle(
    instances: &mut [AuthenticatedMaterializedInstancePlan],
    owner_window_id: &str,
    owner_runtime_instance_token: &str,
    lifecycle: &MaterializedPointerLifecycle,
) -> Result<(), ReplayWindowManagerRejection> {
    for instance in instances {
        let actions = instance
            .viewer_actions()
            .iter()
            .map(|(path, action)| (path.clone(), action.clone()))
            .collect::<Vec<_>>();
        for (path, action) in actions {
            let interaction = materialized_button_interaction(
                owner_window_id,
                owner_runtime_instance_token,
                &action,
                lifecycle,
            );
            instance
                .set_read_only_button_interaction(&path, interaction)
                .map_err(|_| theatre_materialization_rejection(owner_window_id))?;
        }
    }
    Ok(())
}

fn materialized_button_interaction(
    owner_window_id: &str,
    owner_runtime_instance_token: &str,
    action: &ReplayViewerAction,
    lifecycle: &MaterializedPointerLifecycle,
) -> crate::runtime_graphics::ButtonVisualState {
    let matches = |identity: Option<&MaterializedPointerActionIdentity>| {
        identity.is_some_and(|identity| {
            identity.window_id == owner_window_id
                && identity.runtime_instance_token == owner_runtime_instance_token
                && identity.action == *action
        })
    };
    if matches(lifecycle.pressed.as_ref()) {
        crate::runtime_graphics::ButtonVisualState::Down
    } else if matches(lifecycle.hovered.as_ref()) {
        crate::runtime_graphics::ButtonVisualState::Over
    } else {
        crate::runtime_graphics::ButtonVisualState::Up
    }
}

fn materialized_pointer_disposition(
    semantic: Option<&AuthenticatedElementSemantic>,
    interactive: bool,
) -> (ReplayPointerDisposition, Option<bool>) {
    if interactive {
        return (ReplayPointerDisposition::Interactive, Some(true));
    }
    match semantic {
        Some(AuthenticatedElementSemantic::ReadOnlyButton { interaction, .. }) => (
            ReplayPointerDisposition::BlockOnly,
            Some(*interaction != crate::runtime_graphics::ButtonVisualState::Disable),
        ),
        _ => (ReplayPointerDisposition::PassThrough, None),
    }
}

fn theatre_template_semantics(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    owner_surface: &AuthenticatedPanelSurface,
    panel: &StrictAuthoredPanelAdmission,
    overrides: &[(&str, ReplayAuthenticatedElementSemantic)],
) -> Result<BTreeMap<Vec<u32>, AuthenticatedElementSemantic>, ReplayWindowManagerRejection> {
    let mut semantics =
        crate::authenticated_semantic_provider::authenticated_theatre_template_static_semantics(
            index,
            admission,
            owner_surface,
            panel,
        )
        .map_err(|_| theatre_materialization_rejection(panel.window_id()))?
        .into_iter()
        .map(|(path, semantic)| (path, authenticated_element_semantic_from_replay(&semantic)))
        .collect::<BTreeMap<_, _>>();
    let mut overridden = BTreeSet::new();
    for (authored_name, semantic) in overrides {
        let matches = panel
            .element_roles()
            .iter()
            .filter(|role| role.authored_name.eq_ignore_ascii_case(authored_name))
            .collect::<Vec<_>>();
        let [role] = matches.as_slice() else {
            return Err(theatre_materialization_rejection(panel.window_id()));
        };
        if !overridden.insert(role.child_path.clone()) {
            return Err(theatre_materialization_rejection(panel.window_id()));
        }
        let mut sealed = authenticated_element_semantic_from_replay(semantic);
        if let AuthenticatedElementSemantic::PassiveSprite { one_based_frame } = sealed {
            let authored = panel
                .authored_element(&role.child_path)
                .ok_or_else(|| theatre_materialization_rejection(panel.window_id()))?;
            if WidgetFamily::from_element_type(&authored.element_type)
                .is_some_and(WidgetFamily::standard_button_carrier)
            {
                sealed = AuthenticatedElementSemantic::ReadOnlyButton {
                    one_based_frame,
                    interaction: theatre_read_only_button_interaction(&role.authored_name),
                };
            }
        }
        semantics.insert(role.child_path.clone(), sealed);
    }
    Ok(semantics)
}

fn theatre_hidden_template_semantics(
    panel: &StrictAuthoredPanelAdmission,
) -> BTreeMap<Vec<u32>, AuthenticatedElementSemantic> {
    panel
        .element_roles()
        .iter()
        .map(|role| {
            (
                role.child_path.clone(),
                AuthenticatedElementSemantic::Hidden,
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn append_theatre_instance(
    instances: &mut Vec<AuthenticatedMaterializedInstancePlan>,
    binding: &ReplayAuthenticatedTheatreBinding,
    kind: crate::authenticated_semantic_provider::AuthenticatedTheatreTemplateKind,
    instance_key: String,
    host_instance_ordinal: Option<u32>,
    host_child_path: Vec<u32>,
    owner_child_insertion_ordinal: u32,
    parent_rect: AuthenticatedElementRect,
    required_clip: AuthenticatedElementRect,
    semantics: BTreeMap<Vec<u32>, AuthenticatedElementSemantic>,
) -> Result<u32, ReplayWindowManagerRejection> {
    let template = binding
        .templates
        .template(kind)
        .ok_or_else(|| theatre_materialization_rejection(&binding.owner_window_id))?
        .clone();
    let instance_ordinal = u32::try_from(instances.len())
        .map_err(|_| theatre_materialization_rejection(&binding.owner_window_id))?;
    instances.push(AuthenticatedMaterializedInstancePlan::new(
        instance_key,
        instance_ordinal,
        template,
        false,
        host_instance_ordinal,
        host_child_path,
        owner_child_insertion_ordinal,
        parent_rect,
        required_clip,
        semantics,
    ));
    Ok(instance_ordinal)
}

#[allow(clippy::too_many_arguments)]
fn append_theatre_global_root_instance(
    instances: &mut Vec<AuthenticatedMaterializedInstancePlan>,
    binding: &ReplayAuthenticatedTheatreBinding,
    kind: crate::authenticated_semantic_provider::AuthenticatedTheatreTemplateKind,
    instance_key: String,
    parent_rect: AuthenticatedElementRect,
    required_clip: AuthenticatedElementRect,
    semantics: BTreeMap<Vec<u32>, AuthenticatedElementSemantic>,
) -> Result<u32, ReplayWindowManagerRejection> {
    let template = binding
        .templates
        .template(kind)
        .ok_or_else(|| theatre_materialization_rejection(&binding.owner_window_id))?
        .clone();
    let instance_ordinal = u32::try_from(instances.len())
        .map_err(|_| theatre_materialization_rejection(&binding.owner_window_id))?;
    instances.push(AuthenticatedMaterializedInstancePlan::new(
        instance_key,
        instance_ordinal,
        template,
        true,
        None,
        Vec::new(),
        0,
        parent_rect,
        required_clip,
        semantics,
    ));
    Ok(instance_ordinal)
}

fn bind_theatre_instance_viewer_action(
    instances: &mut [AuthenticatedMaterializedInstancePlan],
    instance_ordinal: u32,
    template: &StrictAuthoredPanelAdmission,
    authored_name: &str,
    action: ReplayViewerAction,
) -> Result<(), ReplayWindowManagerRejection> {
    let path = theatre_unique_child_path(template, authored_name)?;
    let instance = instances
        .get_mut(
            usize::try_from(instance_ordinal)
                .map_err(|_| theatre_materialization_rejection(template.window_id()))?,
        )
        .filter(|instance| instance.instance_ordinal() == instance_ordinal)
        .ok_or_else(|| theatre_materialization_rejection(template.window_id()))?;
    instance
        .bind_viewer_action(path, action)
        .map_err(|_| theatre_materialization_rejection(template.window_id()))
}

fn checked_theatre_rect(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    window_id: &str,
) -> Result<AuthenticatedElementRect, ReplayWindowManagerRejection> {
    if width <= 0 || height <= 0 {
        return Err(theatre_materialization_rejection(window_id));
    }
    Ok(AuthenticatedElementRect::new(x, y, width, height))
}

fn checked_add_theatre_pixels(
    left: i32,
    right: i32,
    window_id: &str,
) -> Result<i32, ReplayWindowManagerRejection> {
    left.checked_add(right)
        .ok_or_else(|| theatre_materialization_rejection(window_id))
}

fn exact_a729_theatre_content_height(
    templates: &crate::authenticated_semantic_provider::AuthenticatedTheatreTemplateSet,
    surface: &crate::production_renderer::ObserverTheatreSurface,
    mode: Option<crate::production_renderer::ObserverTheatreMode>,
    list_width: i32,
    list_height: i32,
    owner_window_id: &str,
) -> Result<i32, ReplayWindowManagerRejection> {
    use crate::authenticated_semantic_provider::AuthenticatedTheatreTemplateKind as Template;
    use crate::production_renderer::ObserverTheatreMode;
    use replay_core::{
        TheatreSelectorController as TheatreControllerState,
        TheatreSelectorObjectKind as ObjectKind,
    };

    let template_height = |kind: Template| {
        let panel = templates
            .template(kind)
            .ok_or_else(|| theatre_materialization_rejection(owner_window_id))?;
        authenticated_template_root_extent(panel, list_width, list_height)
            .map(|(_, height)| height)
            .map_err(|_| theatre_materialization_rejection(owner_window_id))
    };
    let count_height = |count: usize, height: i32| {
        i32::try_from(count)
            .map_err(|_| theatre_materialization_rejection(owner_window_id))?
            .checked_mul(height)
            .ok_or_else(|| theatre_materialization_rejection(owner_window_id))
    };
    let mut content_height = 0_i32;
    let mut add = |height: i32| {
        content_height = checked_add_theatre_pixels(content_height, height, owner_window_id)?;
        Ok::<(), ReplayWindowManagerRejection>(())
    };

    match mode {
        None => {}
        Some(ObserverTheatreMode::Land) => {
            let theatre_height = template_height(Template::LandTheatre)?;
            let group_height = template_height(Template::ArmyGroupBox)?;
            for theatre in surface.section.theatres_of_kind(ObjectKind::LandTheatre) {
                if !matches!(theatre.controller, TheatreControllerState::Land { .. }) {
                    return Err(theatre_materialization_rejection(owner_window_id));
                }
                let group_count = theatre
                    .members
                    .iter()
                    .filter(|member| {
                        member.parent_kind == ObjectKind::LandTheatre
                            && member.parent_id == theatre.object_id
                            && member.kind == ObjectKind::ArmyGroup
                    })
                    .count();
                add(theatre_height)?;
                add(count_height(group_count, group_height)?)?;
            }
        }
        Some(ObserverTheatreMode::Navy) => {
            let theatre_height = template_height(Template::NavyTheatre)?;
            let row_height = template_height(Template::FleetRowBox)?;
            for theatre in surface.section.theatres_of_kind(ObjectKind::NavyTheatre) {
                if !matches!(theatre.controller, TheatreControllerState::Navy { .. }) {
                    return Err(theatre_materialization_rejection(owner_window_id));
                }
                let normal_count = theatre
                    .members
                    .iter()
                    .filter(|member| {
                        member.kind == ObjectKind::Fleet
                            && member.parent_kind == ObjectKind::NavyTheatre
                            && member.parent_id == theatre.object_id
                            && !member.reserve_navy
                    })
                    .count();
                let reserve_count = theatre
                    .members
                    .iter()
                    .filter(|member| {
                        member.kind == ObjectKind::ReserveFleetAggregate
                            && member.parent_kind == ObjectKind::NavyTheatre
                            && member.parent_id == theatre.object_id
                            && member.reserve_navy
                    })
                    .count();
                if reserve_count > 1 {
                    return Err(theatre_materialization_rejection(owner_window_id));
                }
                add(theatre_height)?;
                add(count_height(normal_count + reserve_count, row_height)?)?;
            }
        }
        Some(ObserverTheatreMode::Air) => {
            if !surface.section.free_air_wings.is_empty() {
                add(template_height(Template::FreeAirWings)?)?;
            }
            let theatre_height = template_height(Template::AirTheatre)?;
            let row_height = template_height(Template::AirTheatreGroupRow)?;
            for theatre in surface.section.theatres_of_kind(ObjectKind::AirTheatre) {
                if !matches!(theatre.controller, TheatreControllerState::Air { .. }) {
                    return Err(theatre_materialization_rejection(owner_window_id));
                }
                let group_count = theatre
                    .members
                    .iter()
                    .filter(|member| {
                        member.kind == ObjectKind::AirGroup
                            && member.parent_kind == ObjectKind::AirTheatre
                            && member.parent_id == theatre.object_id
                    })
                    .count();
                add(theatre_height)?;
                add(count_height(group_count, row_height)?)?;
            }
        }
    }

    let region_height = template_height(Template::StrategicRegion)?;
    add(count_height(
        surface.section.strategic_regions.len(),
        region_height,
    )?)?;
    Ok(content_height)
}

fn clamp_exact_theatre_scroll(
    retained_offset_pixels: i32,
    content_height: i32,
    viewport_height: i32,
) -> Result<i32, ReplayWindowManagerRejection> {
    if retained_offset_pixels < 0 || content_height < 0 || viewport_height <= 0 {
        return Err(theatre_materialization_rejection(
            "theatre_selection_window",
        ));
    }
    Ok(retained_offset_pixels.min(content_height.saturating_sub(viewport_height).max(0)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExactTheatreScrollExtent {
    effective_offset_pixels: i32,
    max_offset_pixels: i32,
    identity_sha256: String,
}

fn exact_theatre_scroll_extent(
    retained_offset_pixels: i32,
    content_height: i32,
    list_rect: AuthenticatedElementRect,
    clipped_viewport_rect: AuthenticatedElementRect,
) -> Result<ExactTheatreScrollExtent, ReplayWindowManagerRejection> {
    let list_right = list_rect
        .x()
        .checked_add(list_rect.width())
        .ok_or_else(|| theatre_materialization_rejection("theatre_selection_window"))?;
    let list_bottom = list_rect
        .y()
        .checked_add(list_rect.height())
        .ok_or_else(|| theatre_materialization_rejection("theatre_selection_window"))?;
    let clip_right = clipped_viewport_rect
        .x()
        .checked_add(clipped_viewport_rect.width())
        .ok_or_else(|| theatre_materialization_rejection("theatre_selection_window"))?;
    let clip_bottom = clipped_viewport_rect
        .y()
        .checked_add(clipped_viewport_rect.height())
        .ok_or_else(|| theatre_materialization_rejection("theatre_selection_window"))?;
    if list_rect.width() <= 0
        || list_rect.height() <= 0
        || clipped_viewport_rect.width() <= 0
        || clipped_viewport_rect.height() <= 0
        || clipped_viewport_rect.x() < list_rect.x()
        || clipped_viewport_rect.y() < list_rect.y()
        || clip_right > list_right
        || clip_bottom > list_bottom
    {
        return Err(theatre_materialization_rejection(
            "theatre_selection_window",
        ));
    }
    let effective_offset_pixels = clamp_exact_theatre_scroll(
        retained_offset_pixels,
        content_height,
        clipped_viewport_rect.height(),
    )?;
    let max_offset_pixels = content_height
        .saturating_sub(clipped_viewport_rect.height())
        .max(0);
    let identity_sha256 = snapshot::sha256::to_hex(&Sha256::digest(
        format!(
            "{:?}",
            (
                "exact-a729-theatre-scroll-extent-v1",
                list_rect,
                clipped_viewport_rect,
                content_height,
                max_offset_pixels,
                effective_offset_pixels,
            )
        )
        .as_bytes(),
    ));
    Ok(ExactTheatreScrollExtent {
        effective_offset_pixels,
        max_offset_pixels,
        identity_sha256,
    })
}

fn theatre_unique_child_path(
    panel: &StrictAuthoredPanelAdmission,
    authored_name: &str,
) -> Result<Vec<u32>, ReplayWindowManagerRejection> {
    let matches = panel
        .element_roles()
        .iter()
        .filter(|role| role.authored_name.eq_ignore_ascii_case(authored_name))
        .map(|role| role.child_path.clone())
        .collect::<Vec<_>>();
    let [path] = matches.as_slice() else {
        return Err(theatre_materialization_rejection(panel.window_id()));
    };
    Ok(path.clone())
}

fn theatre_overlap_rects(
    panel: &StrictAuthoredPanelAdmission,
    host_child_path: &[u32],
    root_rect: AuthenticatedElementRect,
    item_extent: (i32, i32),
    count: usize,
) -> Result<Vec<AuthenticatedElementRect>, ReplayWindowManagerRejection> {
    let host_rect = authenticated_template_child_rect(panel, host_child_path, root_rect)
        .map_err(|_| theatre_materialization_rejection(panel.window_id()))?;
    let host = panel
        .authored_element(host_child_path)
        .ok_or_else(|| theatre_materialization_rejection(panel.window_id()))?;
    crate::observer_collection::plan_authenticated_overlap_rects(
        host,
        Rect::new(
            host_rect.x(),
            host_rect.y(),
            host_rect.width(),
            host_rect.height(),
        ),
        &vec![item_extent; count],
    )
    .map_err(|_| theatre_materialization_rejection(panel.window_id()))?
    .into_iter()
    .map(|rect| checked_theatre_rect(rect.x(), rect.y(), rect.w(), rect.h(), panel.window_id()))
    .collect()
}

#[allow(clippy::too_many_arguments)]
fn build_exact_a729_theatre_materialization_plan(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    owner_surface: &AuthenticatedPanelSurface,
    owner_panel: &StrictAuthoredPanelAdmission,
    owner_lease: &AuthenticatedReplayPanelLease<'_>,
    preflight: &AuthenticatedLayoutPreflight,
    binding: &ReplayAuthenticatedTheatreBinding,
    pointer_lifecycle: &MaterializedPointerLifecycle,
) -> Result<AuthenticatedMaterializationPlan, ReplayWindowManagerRejection> {
    use crate::authenticated_semantic_provider::AuthenticatedTheatreTemplateKind as Template;
    use crate::production_renderer::ObserverTheatreMode;
    use replay_core::{
        TheatreSelectorController as TheatreControllerState,
        TheatreSelectorFreeAirWing as FreeAirWingRow, TheatreSelectorMember as MemberRow,
        TheatreSelectorMemberController as MemberControllerState,
        TheatreSelectorObjectKind as ObjectKind,
    };

    if binding.owner_window_id != owner_panel.window_id()
        || binding.index_identity_sha256 != index.index_identity_sha256
        || binding.manifest_digest_sha256 != admission.manifest_digest_sha256()
        || binding.product_modset_sha256 != admission.product_modset_sha256()
        || binding.viewport_size != preflight.viewport_size()
        || binding.identity_sha256.trim().is_empty()
    {
        return Err(theatre_materialization_rejection(owner_panel.window_id()));
    }
    let list = preflight
        .elements()
        .iter()
        .filter(|element| {
            element.visible()
                && element
                    .authored_name()
                    .eq_ignore_ascii_case("elements_list")
        })
        .collect::<Vec<_>>();
    let [list] = list.as_slice() else {
        return Err(theatre_materialization_rejection(owner_panel.window_id()));
    };
    let list_rect = list
        .screen_rect()
        .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
    let required_clip = list
        .clip_rect()
        .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
    let expected_content_height = exact_a729_theatre_content_height(
        &binding.templates,
        &binding.surface,
        binding.mode,
        list_rect.width(),
        list_rect.height(),
        owner_panel.window_id(),
    )?;
    let scroll_extent = exact_theatre_scroll_extent(
        binding.scroll_offset_pixels,
        expected_content_height,
        list_rect,
        required_clip,
    )?;
    let host_path = list.child_path().to_vec();
    let host_insertion = owner_panel
        .authored_element(&host_path)
        .and_then(|element| u32::try_from(element.children.len()).ok())
        .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
    let viewport = preflight.viewport_size();
    let viewport_rect = checked_theatre_rect(
        0,
        0,
        i32::try_from(viewport[0])
            .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?,
        i32::try_from(viewport[1])
            .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?,
        owner_panel.window_id(),
    )?;
    let mut instances = Vec::new();
    let mut content_height = 0_i32;
    let content_x = list_rect.x();
    let visible_y = list_rect
        .y()
        .checked_sub(scroll_extent.effective_offset_pixels)
        .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;

    let template_extent = |kind: Template| {
        let panel = binding
            .templates
            .template(kind)
            .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
        authenticated_template_root_extent(panel, list_rect.width(), list_rect.height())
            .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))
    };

    match binding.mode {
        None => {}
        Some(ObserverTheatreMode::Land) => {
            for theatre in binding
                .surface
                .section
                .theatres_of_kind(ObjectKind::LandTheatre)
            {
                let panel = binding
                    .templates
                    .template(Template::LandTheatre)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let (width, height) = template_extent(Template::LandTheatre)?;
                let (priority_ordinal, _delete_eligible) = match &theatre.controller {
                    TheatreControllerState::Land {
                        priority_ordinal,
                        delete_eligible,
                    } => (*priority_ordinal, *delete_eligible),
                    _ => return Err(theatre_materialization_rejection(owner_panel.window_id())),
                };
                let mut overrides = vec![
                    (
                        "background",
                        ReplayAuthenticatedElementSemantic::PassiveSprite { one_based_frame: 2 },
                    ),
                    (
                        "selected_theatre_frame",
                        ReplayAuthenticatedElementSemantic::Hidden,
                    ),
                    (
                        "assign_button",
                        ReplayAuthenticatedElementSemantic::PassiveSprite { one_based_frame: 1 },
                    ),
                    (
                        "settings_button",
                        ReplayAuthenticatedElementSemantic::PassiveSprite { one_based_frame: 1 },
                    ),
                    (
                        "name_box",
                        ReplayAuthenticatedElementSemantic::Text {
                            resolved_text: theatre.display_name.clone(),
                        },
                    ),
                    (
                        "theater_supply_priority",
                        ReplayAuthenticatedElementSemantic::PassiveSprite {
                            one_based_frame: priority_ordinal,
                        },
                    ),
                    (
                        "armies_count_box",
                        ReplayAuthenticatedElementSemantic::Text {
                            resolved_text: theatre
                                .members
                                .iter()
                                .filter(|member| member.kind == ObjectKind::Army)
                                .count()
                                .to_string(),
                        },
                    ),
                    (
                        "attack_combat_count",
                        ReplayAuthenticatedElementSemantic::Hidden,
                    ),
                    (
                        "defence_combat_count",
                        ReplayAuthenticatedElementSemantic::Hidden,
                    ),
                ];
                // Owner ruling option A (lane RE dossier section 2.4): schema 6
                // never carries the land combat-log leaf. The icon is derived at
                // admission time from authoritative state -- the observer
                // country's membership in the session's measured player set --
                // and rides on the surface (`combat_log_icon_shown`).
                if binding.surface.combat_log_icon_shown {
                    overrides.push((
                        "combat_logs_btn",
                        ReplayAuthenticatedElementSemantic::PassiveSprite { one_based_frame: 1 },
                    ));
                }
                let row_y =
                    checked_add_theatre_pixels(visible_y, content_height, owner_panel.window_id())?;
                let row_rect =
                    checked_theatre_rect(content_x, row_y, width, height, owner_panel.window_id())?;
                let settings = binding
                    .templates
                    .template(Template::SettingsView)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let (settings_width, settings_height) = template_extent(Template::SettingsView)?;
                let settings_rect = checked_theatre_rect(
                    row_rect.x().checked_sub(settings_width).ok_or_else(|| {
                        theatre_materialization_rejection(owner_panel.window_id())
                    })?,
                    row_rect.y(),
                    settings_width,
                    settings_height,
                    owner_panel.window_id(),
                )?;
                let settings_semantics = if binding.settings_land_id == Some(theatre.object_id) {
                    let mut settings_overrides = vec![
                        (
                            "close_button",
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: 1,
                            },
                        ),
                        (
                            "btn_delete",
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: 1,
                            },
                        ),
                        (
                            "name",
                            ReplayAuthenticatedElementSemantic::Text {
                                resolved_text: theatre.display_name.clone(),
                            },
                        ),
                    ];
                    for ordinal in 1..=3_u32 {
                        settings_overrides.push((
                            match ordinal {
                                1 => "priority_button_1",
                                2 => "priority_button_2",
                                _ => "priority_button_3",
                            },
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: if ordinal == priority_ordinal { 2 } else { 1 },
                            },
                        ));
                    }
                    theatre_template_semantics(
                        index,
                        admission,
                        owner_surface,
                        settings,
                        &settings_overrides,
                    )?
                } else {
                    theatre_hidden_template_semantics(settings)
                };
                let settings_ordinal = append_theatre_global_root_instance(
                    &mut instances,
                    binding,
                    Template::SettingsView,
                    format!("land-settings/{}", theatre.object_id),
                    settings_rect,
                    viewport_rect,
                    settings_semantics,
                )?;
                if binding.settings_land_id == Some(theatre.object_id) {
                    bind_theatre_instance_viewer_action(
                        &mut instances,
                        settings_ordinal,
                        settings,
                        "close_button",
                        ReplayViewerAction::Theatre(TheatreViewerAction::CloseLandSettings(
                            theatre.object_id,
                        )),
                    )?;
                }
                let land_ordinal = append_theatre_instance(
                    &mut instances,
                    binding,
                    Template::LandTheatre,
                    format!("land-theatre/{}", theatre.object_id),
                    None,
                    host_path.clone(),
                    host_insertion,
                    row_rect,
                    required_clip,
                    theatre_template_semantics(index, admission, owner_surface, panel, &overrides)?,
                )?;
                bind_theatre_instance_viewer_action(
                    &mut instances,
                    land_ordinal,
                    panel,
                    "settings_button",
                    ReplayViewerAction::Theatre(TheatreViewerAction::OpenLandSettings(
                        theatre.object_id,
                    )),
                )?;
                content_height =
                    checked_add_theatre_pixels(content_height, height, owner_panel.window_id())?;

                let orders_template = binding
                    .templates
                    .template(Template::OrdersGroup)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let orders_extent = authenticated_template_root_extent(
                    orders_template,
                    row_rect.width(),
                    row_rect.height(),
                )
                .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                let groups = theatre
                    .members
                    .iter()
                    .filter(|member| {
                        member.parent_kind == ObjectKind::LandTheatre
                            && member.parent_id == theatre.object_id
                            && member.kind == ObjectKind::ArmyGroup
                    })
                    .collect::<Vec<_>>();
                let standalones = theatre
                    .members
                    .iter()
                    .filter(|member| {
                        member.parent_kind == ObjectKind::LandTheatre
                            && member.parent_id == theatre.object_id
                            && member.kind == ObjectKind::Army
                    })
                    .collect::<Vec<_>>();
                let direct_members = groups
                    .iter()
                    .copied()
                    .chain(standalones.iter().copied())
                    .collect::<Vec<_>>();
                let orders_path = theatre_unique_child_path(panel, "orders_group_box")?;
                let direct_rects = theatre_overlap_rects(
                    panel,
                    &orders_path,
                    row_rect,
                    orders_extent,
                    direct_members.len(),
                )?;
                for (member, member_rect) in direct_members.iter().zip(direct_rects) {
                    let (training_visible, stop_visible, conflict_visible) = match &member
                        .controller
                    {
                        MemberControllerState::Army {
                            training_visible,
                            stop_at_max_visible,
                            border_conflict_visible,
                        }
                        | MemberControllerState::ArmyGroup {
                            training_visible,
                            stop_at_max_visible,
                            border_conflict_visible,
                        } => (
                            *training_visible,
                            *stop_at_max_visible,
                            *border_conflict_visible,
                        ),
                        _ => {
                            return Err(theatre_materialization_rejection(owner_panel.window_id()))
                        }
                    };
                    let member_overrides = vec![
                        (
                            "army_item_bg",
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: 1,
                            },
                        ),
                        (
                            "army_item_frame",
                            ReplayAuthenticatedElementSemantic::Hidden,
                        ),
                        (
                            "orders_group_item_training",
                            if training_visible {
                                ReplayAuthenticatedElementSemantic::PassiveSprite {
                                    one_based_frame: 1,
                                }
                            } else {
                                ReplayAuthenticatedElementSemantic::Hidden
                            },
                        ),
                        (
                            "icn_exercise_stop_at_max",
                            if stop_visible {
                                ReplayAuthenticatedElementSemantic::PassiveSprite {
                                    one_based_frame: 1,
                                }
                            } else {
                                ReplayAuthenticatedElementSemantic::Hidden
                            },
                        ),
                        (
                            "orders_group_item_border_conflict",
                            if conflict_visible {
                                ReplayAuthenticatedElementSemantic::PassiveSprite {
                                    one_based_frame: 1,
                                }
                            } else {
                                ReplayAuthenticatedElementSemantic::Hidden
                            },
                        ),
                        (
                            "armies_count_box",
                            ReplayAuthenticatedElementSemantic::Hidden,
                        ),
                        (
                            "orders_group_no_order",
                            ReplayAuthenticatedElementSemantic::Hidden,
                        ),
                    ];
                    append_theatre_instance(
                        &mut instances,
                        binding,
                        Template::OrdersGroup,
                        format!("land-member/{}/{}", theatre.object_id, member.object_id),
                        Some(land_ordinal),
                        orders_path.clone(),
                        0,
                        member_rect,
                        required_clip,
                        theatre_template_semantics(
                            index,
                            admission,
                            owner_surface,
                            orders_template,
                            &member_overrides,
                        )?,
                    )?;
                }

                let group_box_template = binding
                    .templates
                    .template(Template::ArmyGroupBox)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let group_box_extent = authenticated_template_root_extent(
                    group_box_template,
                    row_rect.width(),
                    row_rect.height(),
                )
                .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                let root_insertion = u32::try_from(panel.authored_root().children.len())
                    .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                for (group_index, group) in groups.iter().enumerate() {
                    let group_y = checked_add_theatre_pixels(
                        row_rect.y(),
                        checked_add_theatre_pixels(
                            height,
                            i32::try_from(group_index)
                                .map_err(|_| {
                                    theatre_materialization_rejection(owner_panel.window_id())
                                })?
                                .checked_mul(group_box_extent.1)
                                .ok_or_else(|| {
                                    theatre_materialization_rejection(owner_panel.window_id())
                                })?,
                            owner_panel.window_id(),
                        )?,
                        owner_panel.window_id(),
                    )?;
                    let group_rect = checked_theatre_rect(
                        row_rect.x(),
                        group_y,
                        group_box_extent.0,
                        group_box_extent.1,
                        owner_panel.window_id(),
                    )?;
                    let group_ordinal = append_theatre_instance(
                        &mut instances,
                        binding,
                        Template::ArmyGroupBox,
                        format!("army-group-box/{}/{}", theatre.object_id, group.object_id),
                        Some(land_ordinal),
                        Vec::new(),
                        root_insertion,
                        group_rect,
                        required_clip,
                        theatre_template_semantics(
                            index,
                            admission,
                            owner_surface,
                            group_box_template,
                            &[
                                (
                                    "army_group_box_middle_bg",
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    },
                                ),
                                (
                                    "army_group_box_bottom_bg",
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    },
                                ),
                            ],
                        )?,
                    )?;
                    let armies_path = theatre_unique_child_path(group_box_template, "armies_box")?;
                    let grouped_members = std::iter::once(*group)
                        .chain(theatre.members.iter().filter(|member| {
                            member.kind == ObjectKind::Army
                                && member.parent_kind == ObjectKind::ArmyGroup
                                && member.parent_id == group.object_id
                        }))
                        .collect::<Vec<_>>();
                    let grouped_rects = theatre_overlap_rects(
                        group_box_template,
                        &armies_path,
                        group_rect,
                        orders_extent,
                        grouped_members.len(),
                    )?;
                    for (member, member_rect) in grouped_members.iter().zip(grouped_rects) {
                        let (training_visible, stop_visible, conflict_visible) =
                            match &member.controller {
                                MemberControllerState::Army {
                                    training_visible,
                                    stop_at_max_visible,
                                    border_conflict_visible,
                                }
                                | MemberControllerState::ArmyGroup {
                                    training_visible,
                                    stop_at_max_visible,
                                    border_conflict_visible,
                                } => (
                                    *training_visible,
                                    *stop_at_max_visible,
                                    *border_conflict_visible,
                                ),
                                _ => {
                                    return Err(theatre_materialization_rejection(
                                        owner_panel.window_id(),
                                    ))
                                }
                            };
                        let member_overrides = vec![
                            (
                                "army_item_bg",
                                ReplayAuthenticatedElementSemantic::PassiveSprite {
                                    one_based_frame: 1,
                                },
                            ),
                            (
                                "army_item_frame",
                                ReplayAuthenticatedElementSemantic::Hidden,
                            ),
                            (
                                "orders_group_item_training",
                                if training_visible {
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    }
                                } else {
                                    ReplayAuthenticatedElementSemantic::Hidden
                                },
                            ),
                            (
                                "icn_exercise_stop_at_max",
                                if stop_visible {
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    }
                                } else {
                                    ReplayAuthenticatedElementSemantic::Hidden
                                },
                            ),
                            (
                                "orders_group_item_border_conflict",
                                if conflict_visible {
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    }
                                } else {
                                    ReplayAuthenticatedElementSemantic::Hidden
                                },
                            ),
                            (
                                "armies_count_box",
                                ReplayAuthenticatedElementSemantic::Hidden,
                            ),
                            (
                                "orders_group_no_order",
                                ReplayAuthenticatedElementSemantic::Hidden,
                            ),
                        ];
                        append_theatre_instance(
                            &mut instances,
                            binding,
                            Template::OrdersGroup,
                            format!(
                                "army-group-member/{}/{}/{}",
                                theatre.object_id, group.object_id, member.object_id
                            ),
                            Some(group_ordinal),
                            armies_path.clone(),
                            0,
                            member_rect,
                            required_clip,
                            theatre_template_semantics(
                                index,
                                admission,
                                owner_surface,
                                orders_template,
                                &member_overrides,
                            )?,
                        )?;
                    }
                }
                content_height = checked_add_theatre_pixels(
                    content_height,
                    i32::try_from(groups.len())
                        .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?
                        .checked_mul(group_box_extent.1)
                        .ok_or_else(|| {
                            theatre_materialization_rejection(owner_panel.window_id())
                        })?,
                    owner_panel.window_id(),
                )?;
            }
        }
        Some(ObserverTheatreMode::Navy) => {
            for theatre in binding
                .surface
                .section
                .theatres_of_kind(ObjectKind::NavyTheatre)
            {
                let panel = binding
                    .templates
                    .template(Template::NavyTheatre)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let priority_ordinal = match &theatre.controller {
                    TheatreControllerState::Navy { priority_ordinal } => *priority_ordinal,
                    _ => return Err(theatre_materialization_rejection(owner_panel.window_id())),
                };
                let (width, height) = template_extent(Template::NavyTheatre)?;
                let row_rect = checked_theatre_rect(
                    content_x,
                    checked_add_theatre_pixels(visible_y, content_height, owner_panel.window_id())?,
                    width,
                    height,
                    owner_panel.window_id(),
                )?;
                let navy_ordinal = append_theatre_instance(
                    &mut instances,
                    binding,
                    Template::NavyTheatre,
                    format!("navy-theatre/{}", theatre.object_id),
                    None,
                    host_path.clone(),
                    host_insertion,
                    row_rect,
                    required_clip,
                    theatre_template_semantics(
                        index,
                        admission,
                        owner_surface,
                        panel,
                        &[
                            (
                                "background",
                                ReplayAuthenticatedElementSemantic::PassiveSprite {
                                    one_based_frame: 3,
                                },
                            ),
                            (
                                "selected_theatre_frame",
                                ReplayAuthenticatedElementSemantic::Hidden,
                            ),
                            (
                                "assign_button",
                                ReplayAuthenticatedElementSemantic::PassiveSprite {
                                    one_based_frame: 3,
                                },
                            ),
                            (
                                "theater_name_box",
                                ReplayAuthenticatedElementSemantic::Text {
                                    resolved_text: theatre.display_name.clone(),
                                },
                            ),
                            ("ship_count_box", ReplayAuthenticatedElementSemantic::Hidden),
                            (
                                "priority_button",
                                ReplayAuthenticatedElementSemantic::PassiveSprite {
                                    one_based_frame: priority_ordinal,
                                },
                            ),
                        ],
                    )?,
                )?;
                let fleet_row_template = binding
                    .templates
                    .template(Template::FleetRowBox)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let reserve_row_template = binding
                    .templates
                    .template(Template::ReserveFleetRowBox)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let fleet_icon_template = binding
                    .templates
                    .template(Template::NavyFleet)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let row_path = theatre_unique_child_path(panel, "fleets_row_box")?;
                let fleet_icon_extent = authenticated_template_root_extent(
                    fleet_icon_template,
                    row_rect.width(),
                    row_rect.height(),
                )
                .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                let fleet_row_extent = authenticated_template_root_extent(
                    fleet_row_template,
                    row_rect.width(),
                    row_rect.height(),
                )
                .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                let normal_fleets = theatre
                    .members
                    .iter()
                    .filter(|member| {
                        member.kind == ObjectKind::Fleet
                            && member.parent_kind == ObjectKind::NavyTheatre
                            && member.parent_id == theatre.object_id
                            && !member.reserve_navy
                    })
                    .collect::<Vec<_>>();
                for (row_index, fleet) in normal_fleets.iter().enumerate() {
                    let slot_rect = authenticated_template_grid_slot_rect(
                        panel, &row_path, row_rect, row_index,
                    )
                    .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                    let (split_off_count, split_off_visible) = match &fleet.controller {
                        MemberControllerState::Fleet {
                            split_off_count,
                            split_off_visible,
                        } => (*split_off_count, *split_off_visible),
                        _ => {
                            return Err(theatre_materialization_rejection(owner_panel.window_id()))
                        }
                    };
                    let mut row_overrides = vec![
                        (
                            "army_group_box_middle_bg",
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: 1,
                            },
                        ),
                        (
                            "army_group_box_bottom_bg",
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: 1,
                            },
                        ),
                    ];
                    if split_off_visible {
                        row_overrides.push((
                            "split_off_btn",
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: 2,
                            },
                        ));
                        row_overrides.push((
                            "split_off_count",
                            ReplayAuthenticatedElementSemantic::Text {
                                resolved_text: split_off_count.to_string(),
                            },
                        ));
                    } else {
                        row_overrides.push((
                            "split_off_count",
                            ReplayAuthenticatedElementSemantic::Hidden,
                        ));
                    }
                    let fleet_row_ordinal = append_theatre_instance(
                        &mut instances,
                        binding,
                        Template::FleetRowBox,
                        format!("fleet-row/{}/{}", theatre.object_id, fleet.object_id),
                        Some(navy_ordinal),
                        row_path.clone(),
                        0,
                        slot_rect,
                        required_clip,
                        theatre_template_semantics(
                            index,
                            admission,
                            owner_surface,
                            fleet_row_template,
                            &row_overrides,
                        )?,
                    )?;
                    let navy_box_path = theatre_unique_child_path(fleet_row_template, "navy_box")?;
                    let row_members = std::iter::once(*fleet)
                        .chain(theatre.members.iter().filter(|member| {
                            member.kind == ObjectKind::TaskForce
                                && member.parent_kind == ObjectKind::Fleet
                                && member.parent_id == fleet.object_id
                                && !member.reserve_navy
                        }))
                        .collect::<Vec<_>>();
                    let icon_rects = theatre_overlap_rects(
                        fleet_row_template,
                        &navy_box_path,
                        slot_rect,
                        fleet_icon_extent,
                        row_members.len(),
                    )?;
                    for (member, icon_rect) in row_members.iter().zip(icon_rects) {
                        let (spotting_visible, training_visible, stop_at_max_visible) =
                            match &member.controller {
                                MemberControllerState::TaskForce {
                                    spotting_visible,
                                    training_visible,
                                    stop_at_max_visible,
                                } => (*spotting_visible, *training_visible, *stop_at_max_visible),
                                MemberControllerState::Fleet { .. } => (false, false, false),
                                _ => {
                                    return Err(theatre_materialization_rejection(
                                        owner_panel.window_id(),
                                    ))
                                }
                            };
                        let icon_overrides = vec![
                            (
                                "navy_item_bg",
                                ReplayAuthenticatedElementSemantic::PassiveSprite {
                                    one_based_frame: 1,
                                },
                            ),
                            (
                                "navy_item_frame",
                                ReplayAuthenticatedElementSemantic::Hidden,
                            ),
                            (
                                "training_icon",
                                if training_visible {
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    }
                                } else {
                                    ReplayAuthenticatedElementSemantic::Hidden
                                },
                            ),
                            (
                                "spotting_icon",
                                if spotting_visible {
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    }
                                } else {
                                    ReplayAuthenticatedElementSemantic::Hidden
                                },
                            ),
                            ("in_combat_icon", ReplayAuthenticatedElementSemantic::Hidden),
                            (
                                "moving_to_combat_icon",
                                ReplayAuthenticatedElementSemantic::Hidden,
                            ),
                            (
                                "generic_mission_icon",
                                ReplayAuthenticatedElementSemantic::Hidden,
                            ),
                            (
                                "training_will_stop_at_max_icon",
                                if stop_at_max_visible {
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    }
                                } else {
                                    ReplayAuthenticatedElementSemantic::Hidden
                                },
                            ),
                            ("ship_count_box", ReplayAuthenticatedElementSemantic::Hidden),
                        ];
                        append_theatre_instance(
                            &mut instances,
                            binding,
                            Template::NavyFleet,
                            format!(
                                "fleet-row-member/{}/{}/{}",
                                theatre.object_id, fleet.object_id, member.object_id
                            ),
                            Some(fleet_row_ordinal),
                            navy_box_path.clone(),
                            0,
                            icon_rect,
                            required_clip,
                            theatre_template_semantics(
                                index,
                                admission,
                                owner_surface,
                                fleet_icon_template,
                                &icon_overrides,
                            )?,
                        )?;
                    }
                }
                let reserve = theatre.members.iter().find(|member| {
                    member.kind == ObjectKind::ReserveFleetAggregate
                        && member.parent_kind == ObjectKind::NavyTheatre
                        && member.parent_id == theatre.object_id
                        && member.reserve_navy
                });
                if theatre
                    .members
                    .iter()
                    .filter(|member| {
                        member.kind == ObjectKind::ReserveFleetAggregate
                            && member.parent_kind == ObjectKind::NavyTheatre
                            && member.parent_id == theatre.object_id
                            && member.reserve_navy
                    })
                    .count()
                    > 1
                {
                    return Err(theatre_materialization_rejection(owner_panel.window_id()));
                }
                if let Some(reserve) = reserve {
                    let row_index = normal_fleets.len();
                    let slot_rect = authenticated_template_grid_slot_rect(
                        panel, &row_path, row_rect, row_index,
                    )
                    .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                    let (split_off_count, split_off_visible) = match &reserve.controller {
                        MemberControllerState::ReserveFleetAggregate {
                            split_off_count,
                            split_off_visible,
                        } => (*split_off_count, *split_off_visible),
                        _ => {
                            return Err(theatre_materialization_rejection(owner_panel.window_id()))
                        }
                    };
                    let mut row_overrides = vec![
                        (
                            "army_group_box_middle_bg",
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: 1,
                            },
                        ),
                        (
                            "army_group_box_bottom_bg",
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: 1,
                            },
                        ),
                        (
                            "reserve_fleet_btn",
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: 1,
                            },
                        ),
                    ];
                    if split_off_visible {
                        row_overrides.push((
                            "split_off_btn",
                            ReplayAuthenticatedElementSemantic::PassiveSprite {
                                one_based_frame: 2,
                            },
                        ));
                        row_overrides.push((
                            "split_off_count",
                            ReplayAuthenticatedElementSemantic::Text {
                                resolved_text: split_off_count.to_string(),
                            },
                        ));
                    } else {
                        row_overrides.push((
                            "split_off_count",
                            ReplayAuthenticatedElementSemantic::Hidden,
                        ));
                    }
                    let reserve_row_ordinal = append_theatre_instance(
                        &mut instances,
                        binding,
                        Template::ReserveFleetRowBox,
                        format!(
                            "reserve-fleet-row/{}/{}",
                            theatre.object_id, reserve.object_id
                        ),
                        Some(navy_ordinal),
                        row_path.clone(),
                        0,
                        slot_rect,
                        required_clip,
                        theatre_template_semantics(
                            index,
                            admission,
                            owner_surface,
                            reserve_row_template,
                            &row_overrides,
                        )?,
                    )?;
                    let navy_box_path =
                        theatre_unique_child_path(reserve_row_template, "navy_box")?;
                    let reserve_members = theatre
                        .members
                        .iter()
                        .filter(|member| {
                            member.kind == ObjectKind::TaskForce
                                && member.parent_kind == ObjectKind::ReserveFleetAggregate
                                && member.parent_id == reserve.object_id
                                && member.reserve_navy
                        })
                        .collect::<Vec<_>>();
                    let icon_rects = theatre_overlap_rects(
                        reserve_row_template,
                        &navy_box_path,
                        slot_rect,
                        fleet_icon_extent,
                        reserve_members.len(),
                    )?;
                    for (member, icon_rect) in reserve_members.iter().zip(icon_rects) {
                        let MemberControllerState::TaskForce {
                            spotting_visible,
                            training_visible,
                            stop_at_max_visible,
                        } = &member.controller
                        else {
                            return Err(theatre_materialization_rejection(owner_panel.window_id()));
                        };
                        let icon_overrides = vec![
                            (
                                "navy_item_bg",
                                ReplayAuthenticatedElementSemantic::PassiveSprite {
                                    one_based_frame: 1,
                                },
                            ),
                            (
                                "navy_item_frame",
                                ReplayAuthenticatedElementSemantic::Hidden,
                            ),
                            (
                                "training_icon",
                                if *training_visible {
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    }
                                } else {
                                    ReplayAuthenticatedElementSemantic::Hidden
                                },
                            ),
                            (
                                "spotting_icon",
                                if *spotting_visible {
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    }
                                } else {
                                    ReplayAuthenticatedElementSemantic::Hidden
                                },
                            ),
                            ("in_combat_icon", ReplayAuthenticatedElementSemantic::Hidden),
                            (
                                "moving_to_combat_icon",
                                ReplayAuthenticatedElementSemantic::Hidden,
                            ),
                            (
                                "generic_mission_icon",
                                ReplayAuthenticatedElementSemantic::Hidden,
                            ),
                            (
                                "training_will_stop_at_max_icon",
                                if *stop_at_max_visible {
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    }
                                } else {
                                    ReplayAuthenticatedElementSemantic::Hidden
                                },
                            ),
                            ("ship_count_box", ReplayAuthenticatedElementSemantic::Hidden),
                        ];
                        append_theatre_instance(
                            &mut instances,
                            binding,
                            Template::NavyFleet,
                            format!(
                                "reserve-fleet-member/{}/{}/{}",
                                theatre.object_id, reserve.object_id, member.object_id
                            ),
                            Some(reserve_row_ordinal),
                            navy_box_path.clone(),
                            0,
                            icon_rect,
                            required_clip,
                            theatre_template_semantics(
                                index,
                                admission,
                                owner_surface,
                                fleet_icon_template,
                                &icon_overrides,
                            )?,
                        )?;
                    }
                }
                content_height = checked_add_theatre_pixels(
                    content_height,
                    checked_add_theatre_pixels(
                        height,
                        i32::try_from(normal_fleets.len() + usize::from(reserve.is_some()))
                            .map_err(|_| {
                                theatre_materialization_rejection(owner_panel.window_id())
                            })?
                            .checked_mul(fleet_row_extent.1)
                            .ok_or_else(|| {
                                theatre_materialization_rejection(owner_panel.window_id())
                            })?,
                        owner_panel.window_id(),
                    )?,
                    owner_panel.window_id(),
                )?;
            }
        }
        Some(ObserverTheatreMode::Air) => {
            if !binding.surface.section.free_air_wings.is_empty() {
                let panel = binding
                    .templates
                    .template(Template::FreeAirWings)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let (width, height) = template_extent(Template::FreeAirWings)?;
                let row_rect = checked_theatre_rect(
                    content_x,
                    checked_add_theatre_pixels(visible_y, content_height, owner_panel.window_id())?,
                    width,
                    height,
                    owner_panel.window_id(),
                )?;
                let mut overrides = Vec::new();
                for ordinal in 1..=3_u32 {
                    overrides.push((
                        match ordinal {
                            1 => "priority_1",
                            2 => "priority_2",
                            _ => "priority_3",
                        },
                        ReplayAuthenticatedElementSemantic::PassiveSprite {
                            one_based_frame: if binding.surface.section.free_air_priority_ordinal
                                == Some(ordinal)
                            {
                                2
                            } else {
                                1
                            },
                        },
                    ));
                }
                let free_air_ordinal = append_theatre_instance(
                    &mut instances,
                    binding,
                    Template::FreeAirWings,
                    format!("free-air/{}", binding.surface.owner_tag_db_id),
                    None,
                    host_path.clone(),
                    host_insertion,
                    row_rect,
                    required_clip,
                    theatre_template_semantics(index, admission, owner_surface, panel, &overrides)?,
                )?;
                let wing_template = binding
                    .templates
                    .template(Template::AirWing)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let wing_extent = authenticated_template_root_extent(
                    wing_template,
                    row_rect.width(),
                    row_rect.height(),
                )
                .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                let air_box_path = theatre_unique_child_path(panel, "air_groups_box")?;
                let mut buckets = BTreeMap::<i32, Vec<&FreeAirWingRow>>::new();
                for wing in &binding.surface.section.free_air_wings {
                    buckets.entry(wing.bucket_key).or_default().push(wing);
                }
                let bucket_rects = theatre_overlap_rects(
                    panel,
                    &air_box_path,
                    row_rect,
                    wing_extent,
                    buckets.len(),
                )?;
                for ((bucket_key, wings), wing_rect) in buckets.into_iter().zip(bucket_rects) {
                    let active_frame = wings
                        .first()
                        .map(|wing| wing.active_visual_frame)
                        .ok_or_else(|| {
                            theatre_materialization_rejection(owner_panel.window_id())
                        })?;
                    if wings
                        .iter()
                        .any(|wing| wing.active_visual_frame != active_frame)
                    {
                        return Err(theatre_materialization_rejection(owner_panel.window_id()));
                    }
                    append_theatre_instance(
                        &mut instances,
                        binding,
                        Template::AirWing,
                        format!("free-air-bucket/{bucket_key}"),
                        Some(free_air_ordinal),
                        air_box_path.clone(),
                        0,
                        wing_rect,
                        required_clip,
                        theatre_template_semantics(
                            index,
                            admission,
                            owner_surface,
                            wing_template,
                            &[
                                (
                                    "background",
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: active_frame,
                                    },
                                ),
                                (
                                    "air_wing_count_box",
                                    ReplayAuthenticatedElementSemantic::Text {
                                        resolved_text: wings.len().to_string(),
                                    },
                                ),
                            ],
                        )?,
                    )?;
                }
                content_height =
                    checked_add_theatre_pixels(content_height, height, owner_panel.window_id())?;
            }
            for theatre in binding
                .surface
                .section
                .theatres_of_kind(ObjectKind::AirTheatre)
            {
                let panel = binding
                    .templates
                    .template(Template::AirTheatre)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let selected_priority_ordinal = match &theatre.controller {
                    TheatreControllerState::Air {
                        selected_priority_ordinal,
                        ..
                    } => *selected_priority_ordinal,
                    _ => return Err(theatre_materialization_rejection(owner_panel.window_id())),
                };
                let (width, height) = template_extent(Template::AirTheatre)?;
                let row_rect = checked_theatre_rect(
                    content_x,
                    checked_add_theatre_pixels(visible_y, content_height, owner_panel.window_id())?,
                    width,
                    height,
                    owner_panel.window_id(),
                )?;
                let mut overrides = vec![
                    (
                        "background",
                        ReplayAuthenticatedElementSemantic::PassiveSprite { one_based_frame: 5 },
                    ),
                    (
                        "assign_button",
                        ReplayAuthenticatedElementSemantic::PassiveSprite { one_based_frame: 4 },
                    ),
                    (
                        "theatre_name_box",
                        ReplayAuthenticatedElementSemantic::Text {
                            resolved_text: theatre.display_name.clone(),
                        },
                    ),
                    (
                        "selected_theatre_frame",
                        ReplayAuthenticatedElementSemantic::Hidden,
                    ),
                    (
                        "air_wing_count_box",
                        ReplayAuthenticatedElementSemantic::Text {
                            resolved_text: theatre
                                .members
                                .iter()
                                .filter(|member| member.kind == ObjectKind::AirWing)
                                .count()
                                .to_string(),
                        },
                    ),
                ];
                for ordinal in 1..=3_u32 {
                    overrides.push((
                        match ordinal {
                            1 => "priority_1",
                            2 => "priority_2",
                            _ => "priority_3",
                        },
                        ReplayAuthenticatedElementSemantic::PassiveSprite {
                            one_based_frame: if selected_priority_ordinal == Some(ordinal) {
                                2
                            } else {
                                1
                            },
                        },
                    ));
                }
                let air_ordinal = append_theatre_instance(
                    &mut instances,
                    binding,
                    Template::AirTheatre,
                    format!("air-theatre/{}", theatre.object_id),
                    None,
                    host_path.clone(),
                    host_insertion,
                    row_rect,
                    required_clip,
                    theatre_template_semantics(index, admission, owner_surface, panel, &overrides)?,
                )?;
                let group_template = binding
                    .templates
                    .template(Template::AirTheatreGroup)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let group_row_template = binding
                    .templates
                    .template(Template::AirTheatreGroupRow)
                    .ok_or_else(|| {
                    theatre_materialization_rejection(owner_panel.window_id())
                })?;
                let wing_template = binding
                    .templates
                    .template(Template::AirWing)
                    .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
                let group_extent = authenticated_template_root_extent(
                    group_template,
                    row_rect.width(),
                    row_rect.height(),
                )
                .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                let row_extent = authenticated_template_root_extent(
                    group_row_template,
                    row_rect.width(),
                    row_rect.height(),
                )
                .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                let wing_extent = authenticated_template_root_extent(
                    wing_template,
                    row_rect.width(),
                    row_rect.height(),
                )
                .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                let groups = theatre
                    .members
                    .iter()
                    .filter(|member| {
                        member.kind == ObjectKind::AirGroup
                            && member.parent_kind == ObjectKind::AirTheatre
                            && member.parent_id == theatre.object_id
                    })
                    .collect::<Vec<_>>();
                let group_box_path = theatre_unique_child_path(panel, "air_groups_box")?;
                let group_rects = theatre_overlap_rects(
                    panel,
                    &group_box_path,
                    row_rect,
                    group_extent,
                    groups.len(),
                )?;
                for (group, group_rect) in groups.iter().zip(group_rects) {
                    if !matches!(&group.controller, MemberControllerState::AirGroup) {
                        return Err(theatre_materialization_rejection(owner_panel.window_id()));
                    }
                    let wing_count = theatre
                        .members
                        .iter()
                        .filter(|member| {
                            member.kind == ObjectKind::AirWing
                                && member.parent_kind == ObjectKind::AirGroup
                                && member.parent_id == group.object_id
                        })
                        .count();
                    append_theatre_instance(
                        &mut instances,
                        binding,
                        Template::AirTheatreGroup,
                        format!("air-group-icon/{}/{}", theatre.object_id, group.object_id),
                        Some(air_ordinal),
                        group_box_path.clone(),
                        0,
                        group_rect,
                        required_clip,
                        theatre_template_semantics(
                            index,
                            admission,
                            owner_surface,
                            group_template,
                            &[
                                (
                                    "background",
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    },
                                ),
                                (
                                    "selected_group_frame",
                                    ReplayAuthenticatedElementSemantic::Hidden,
                                ),
                                (
                                    "air_wing_count_box",
                                    ReplayAuthenticatedElementSemantic::Text {
                                        resolved_text: wing_count.to_string(),
                                    },
                                ),
                            ],
                        )?,
                    )?;
                }
                let row_box_path = theatre_unique_child_path(panel, "air_groups_row_box")?;
                for (row_index, group) in groups.iter().enumerate() {
                    let row_slot = authenticated_template_grid_slot_rect(
                        panel,
                        &row_box_path,
                        row_rect,
                        row_index,
                    )
                    .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
                    let row_ordinal = append_theatre_instance(
                        &mut instances,
                        binding,
                        Template::AirTheatreGroupRow,
                        format!("air-group-row/{}/{}", theatre.object_id, group.object_id),
                        Some(air_ordinal),
                        row_box_path.clone(),
                        0,
                        row_slot,
                        required_clip,
                        theatre_template_semantics(
                            index,
                            admission,
                            owner_surface,
                            group_row_template,
                            &[
                                (
                                    "army_group_box_middle_bg",
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    },
                                ),
                                (
                                    "army_group_box_bottom_bg",
                                    ReplayAuthenticatedElementSemantic::PassiveSprite {
                                        one_based_frame: 1,
                                    },
                                ),
                            ],
                        )?,
                    )?;
                    let row_air_box = theatre_unique_child_path(group_row_template, "air_box")?;
                    let mut buckets = BTreeMap::<i32, Vec<&MemberRow>>::new();
                    for wing in theatre.members.iter().filter(|member| {
                        member.kind == ObjectKind::AirWing
                            && member.parent_kind == ObjectKind::AirGroup
                            && member.parent_id == group.object_id
                    }) {
                        let MemberControllerState::AirWing { bucket_key, .. } = &wing.controller
                        else {
                            return Err(theatre_materialization_rejection(owner_panel.window_id()));
                        };
                        buckets.entry(*bucket_key).or_default().push(wing);
                    }
                    let wing_rects = theatre_overlap_rects(
                        group_row_template,
                        &row_air_box,
                        row_slot,
                        wing_extent,
                        buckets.len(),
                    )?;
                    for ((bucket_key, wings), wing_rect) in buckets.into_iter().zip(wing_rects) {
                        let active_frame = match &wings[0].controller {
                            MemberControllerState::AirWing {
                                active_visual_frame,
                                ..
                            } => *active_visual_frame,
                            _ => unreachable!("bucket was constructed only from air wings"),
                        };
                        if wings.iter().any(|wing| {
                            !matches!(
                                &wing.controller,
                                MemberControllerState::AirWing {
                                    active_visual_frame: frame,
                                    ..
                                } if *frame == active_frame
                            )
                        }) {
                            return Err(theatre_materialization_rejection(owner_panel.window_id()));
                        }
                        append_theatre_instance(
                            &mut instances,
                            binding,
                            Template::AirWing,
                            format!(
                                "air-group-bucket/{}/{}/{}",
                                theatre.object_id, group.object_id, bucket_key
                            ),
                            Some(row_ordinal),
                            row_air_box.clone(),
                            0,
                            wing_rect,
                            required_clip,
                            theatre_template_semantics(
                                index,
                                admission,
                                owner_surface,
                                wing_template,
                                &[
                                    (
                                        "background",
                                        ReplayAuthenticatedElementSemantic::PassiveSprite {
                                            one_based_frame: active_frame,
                                        },
                                    ),
                                    (
                                        "air_wing_count_box",
                                        ReplayAuthenticatedElementSemantic::Text {
                                            resolved_text: wings.len().to_string(),
                                        },
                                    ),
                                ],
                            )?,
                        )?;
                    }
                }
                content_height = checked_add_theatre_pixels(
                    content_height,
                    checked_add_theatre_pixels(
                        height,
                        i32::try_from(groups.len())
                            .map_err(|_| {
                                theatre_materialization_rejection(owner_panel.window_id())
                            })?
                            .checked_mul(row_extent.1)
                            .ok_or_else(|| {
                                theatre_materialization_rejection(owner_panel.window_id())
                            })?,
                        owner_panel.window_id(),
                    )?,
                    owner_panel.window_id(),
                )?;
            }
        }
    }

    for region in &binding.surface.section.strategic_regions {
        let panel = binding
            .templates
            .template(Template::StrategicRegion)
            .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
        let (width, height) = template_extent(Template::StrategicRegion)?;
        let mut overrides = vec![
            (
                "background",
                ReplayAuthenticatedElementSemantic::PassiveSprite {
                    one_based_frame: region.background_frame,
                },
            ),
            (
                "pinned",
                ReplayAuthenticatedElementSemantic::PassiveSprite {
                    one_based_frame: region.pinned_frame,
                },
            ),
            (
                "name",
                ReplayAuthenticatedElementSemantic::Text {
                    resolved_text: region.display_name.clone(),
                },
            ),
        ];
        if let Some(frame) = region.naval_access.visible_frame {
            overrides.push((
                "naval_access_rule",
                ReplayAuthenticatedElementSemantic::PassiveSprite {
                    one_based_frame: frame,
                },
            ));
        }
        let region_rect = checked_theatre_rect(
            content_x,
            checked_add_theatre_pixels(visible_y, content_height, owner_panel.window_id())?,
            width,
            height,
            owner_panel.window_id(),
        )?;
        let region_ordinal = append_theatre_instance(
            &mut instances,
            binding,
            Template::StrategicRegion,
            format!("strategic-region/{}", region.engine_region_index),
            None,
            host_path.clone(),
            host_insertion,
            region_rect,
            required_clip,
            theatre_template_semantics(index, admission, owner_surface, panel, &overrides)?,
        )?;
        let alert_template = binding
            .templates
            .template(Template::StrategicRegionAlert)
            .ok_or_else(|| theatre_materialization_rejection(owner_panel.window_id()))?;
        let alert_extent = authenticated_template_root_extent(
            alert_template,
            list_rect.width(),
            list_rect.height(),
        )
        .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))?;
        let alerts_path = theatre_unique_child_path(panel, "alerts")?;
        let alert_rects = theatre_overlap_rects(
            panel,
            &alerts_path,
            region_rect,
            alert_extent,
            region.alerts.len(),
        )?;
        for (alert, alert_rect) in region.alerts.iter().zip(alert_rects) {
            let mut alert_overrides = vec![(
                "icon",
                ReplayAuthenticatedElementSemantic::PassiveSprite {
                    one_based_frame: alert.atlas_frame,
                },
            )];
            for glow_name in [
                "theatre_alert_red_glow",
                "theatre_alert_green_glow",
                "theatre_alert_yellow_glow",
            ] {
                alert_overrides.push((glow_name, ReplayAuthenticatedElementSemantic::Hidden));
            }
            append_theatre_instance(
                &mut instances,
                binding,
                Template::StrategicRegionAlert,
                format!(
                    "strategic-alert/{}/{}",
                    region.engine_region_index, alert.alert_id
                ),
                Some(region_ordinal),
                alerts_path.clone(),
                0,
                alert_rect,
                required_clip,
                theatre_template_semantics(
                    index,
                    admission,
                    owner_surface,
                    alert_template,
                    &alert_overrides,
                )?,
            )?;
        }
        content_height =
            checked_add_theatre_pixels(content_height, height, owner_panel.window_id())?;
    }

    if expected_content_height != content_height {
        return Err(theatre_materialization_rejection(owner_panel.window_id()));
    }
    if scroll_extent.max_offset_pixels
        != content_height.saturating_sub(required_clip.height()).max(0)
        || scroll_extent.effective_offset_pixels > scroll_extent.max_offset_pixels
    {
        return Err(theatre_materialization_rejection(owner_panel.window_id()));
    }
    apply_materialized_pointer_lifecycle(
        &mut instances,
        owner_panel.window_id(),
        owner_lease.runtime_instance_token(),
        pointer_lifecycle,
    )?;
    let content_rect = checked_theatre_rect(
        content_x,
        visible_y,
        list_rect.width(),
        content_height.max(1),
        owner_panel.window_id(),
    )?;
    seal_authenticated_materialization_plan(
        manager,
        frame_authority,
        index,
        admission,
        owner_panel,
        owner_lease,
        viewport_rect,
        content_rect,
        scroll_extent.effective_offset_pixels,
        scroll_extent.identity_sha256,
        instances,
        None,
        Vec::new(),
    )
    .map_err(|_| theatre_materialization_rejection(owner_panel.window_id()))
}

#[allow(clippy::too_many_arguments)]
pub fn derive_exact_a729_theatre_binding(
    persistent_surfaces: &[AuthenticatedPanelSurface],
    index: &AuthenticatedGuiRoleIndex,
    catalog: &GuiDefinitionCatalog,
    admission: &ValidatedContentAdmission,
    viewport_width: usize,
    viewport_height: usize,
    surface: crate::production_renderer::ObserverTheatreSurface,
    mode: Option<crate::production_renderer::ObserverTheatreMode>,
    settings_land_id: Option<u32>,
    scroll_offset_pixels: i32,
) -> Result<ReplayAuthenticatedTheatreBinding, String> {
    let panels = authenticate_persistent_hud_surfaces(persistent_surfaces)
        .map_err(|error| format!("theatre persistent-root authentication failed: {error:?}"))?;
    let owner = panels
        .panels()
        .iter()
        .find(|panel| {
            panel
                .surface_id()
                .eq_ignore_ascii_case("theatre_selection_window")
        })
        .ok_or_else(|| {
            "authenticated persistent HUD omitted theatre_selection_window".to_string()
        })?;
    let templates = crate::authenticated_semantic_provider::seal_exact_a729_theatre_templates(
        &panels, index, catalog, admission,
    )
    .map_err(|error| error.to_string())?;
    if viewport_width == 0
        || viewport_height == 0
        || i32::try_from(viewport_width).is_err()
        || i32::try_from(viewport_height).is_err()
    {
        return Err("theatre viewport exceeds the authenticated layout domain".into());
    }
    if scroll_offset_pixels < 0 {
        return Err("selected-country theatre scroll offset is negative".into());
    }
    if surface.owner_tag_db_id == 0
        || surface.section.owner_tag_db_id != surface.owner_tag_db_id
        || settings_land_id.is_some_and(|land_id| {
            !surface
                .section
                .theatres_of_kind(replay_core::TheatreSelectorObjectKind::LandTheatre)
                .any(|theatre| theatre.object_id == land_id)
        })
    {
        return Err("selected-country theatre binding identity is invalid".into());
    }
    let mut binding = ReplayAuthenticatedTheatreBinding {
        index_identity_sha256: owner.index_identity_sha256().to_string(),
        manifest_digest_sha256: admission.manifest_digest_sha256().to_string(),
        product_modset_sha256: admission.product_modset_sha256().to_string(),
        owner_window_id: owner.window_id().to_string(),
        viewport_size: [viewport_width, viewport_height],
        templates,
        surface,
        mode,
        settings_land_id,
        scroll_offset_pixels,
        identity_sha256: String::new(),
    };
    binding.identity_sha256 = snapshot::sha256::to_hex(&Sha256::digest(
        format!(
            "{:?}",
            (
                &binding.index_identity_sha256,
                &binding.manifest_digest_sha256,
                &binding.product_modset_sha256,
                &binding.owner_window_id,
                binding.viewport_size,
                binding.templates.identity_sha256(),
                &binding.surface,
                binding.mode,
                binding.settings_land_id,
                binding.scroll_offset_pixels,
            )
        )
        .as_bytes(),
    ));
    Ok(binding)
}

impl ReplayAuthenticatedProductionLinesBinding {
    fn refresh_identity(&mut self) {
        self.identity_sha256.clear();
        self.identity_sha256 =
            snapshot::sha256::to_hex(&Sha256::digest(format!("{self:?}").as_bytes()));
    }

    pub fn owner_window_id(&self) -> &str {
        &self.owner_window_id
    }

    pub fn viewport_size(&self) -> [usize; 2] {
        self.viewport_size
    }

    pub fn country_tag(&self) -> &str {
        &self.country_tag
    }

    pub fn scroll_offset_pixels(&self) -> i32 {
        self.scroll_offset_pixels
    }

    pub fn list_element_id(&self) -> &str {
        &self.list_element_id
    }

    pub fn set_scroll_offset_pixels(&mut self, offset_pixels: i32) -> Result<(), String> {
        if offset_pixels < 0 {
            return Err("XUI12 production-line scroll offset is negative".into());
        }
        self.scroll_offset_pixels = offset_pixels;
        self.refresh_identity();
        Ok(())
    }

    pub fn owner_semantics(&self) -> &BTreeMap<String, ReplayAuthenticatedElementSemantic> {
        &self.owner_semantics
    }

    pub fn owner_pointer_bindings(&self) -> &BTreeMap<String, ReplayElementPointerAuthority> {
        &self.owner_pointer_bindings
    }

    pub fn owner_viewer_actions(&self) -> &[ReplayViewerActionBinding] {
        &self.owner_viewer_actions
    }
}

fn xui12_absolute_pair(value: &crate::paradox_text::Value) -> Result<[i32; 2], String> {
    let pair = crate::gui::parse_measure_pair(value)
        .ok_or_else(|| "XUI12 extended-scrollbar measure pair is malformed".to_string())?;
    if pair.x.is_percent || pair.y.is_percent {
        return Err("XUI12 extended-scrollbar percentage geometry is unsupported".into());
    }
    Ok([
        crate::compose::measure_to_pixels_for_parent(&pair.x, 1),
        crate::compose::measure_to_pixels_for_parent(&pair.y, 1),
    ])
}

fn xui12_scrollbar_part(
    owner: &AuthenticatedPanelSurface,
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    style: &crate::gui::GuiElement,
    field: &str,
) -> Result<ReplayAuthenticatedScrollbarPartBinding, String> {
    let raw = style
        .raw
        .first_value_ci(field)
        .ok_or_else(|| format!("XUI12 extended-scrollbar style lacks {field}"))?;
    let sprite_name = raw
        .first_scalar_ci("quadTextureSprite")
        .map(|value| value.trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("XUI12 extended-scrollbar {field} lacks quadTextureSprite"))?;
    let position = raw
        .first_value_ci("position")
        .map(xui12_absolute_pair)
        .transpose()?
        .unwrap_or([0, 0]);
    let natural_size =
        authenticated_dynamic_sprite_natural_size(owner, index, admission, &sprite_name)?;
    if natural_size[0] <= 0 || natural_size[1] <= 0 {
        return Err(format!(
            "XUI12 extended-scrollbar {field} has empty authenticated graphics"
        ));
    }
    Ok(ReplayAuthenticatedScrollbarPartBinding {
        sprite_name,
        position,
        natural_size,
    })
}

/// The row template for a group, DERIVED: the one ADMITTED surface whose id
/// the group's law names.
///
/// The template used to be a path, a digest and a surface id the producer
/// supplied and this code checked against the same law. There is no producer of
/// those: a recorder cannot know the replay's role-index identities, and the
/// file identity it could know is already authenticated by the admission this
/// lookup goes through. The search is over the whole admitted index rather than
/// the owner panel's own source file because a shipped family authors its row
/// template in a DIFFERENT file from its window - `ingame_lobby_window` is in
/// `interface/ingamelobby.gui` and its `ingame_multiplayer_entry` is in
/// `interface/playerlobby.gui`. Exactly one candidate must exist: a group whose
/// law names three alternative row shapes and finds all three admitted leaves
/// the choice undecidable, and choosing is what a substitute value is.
fn xui26_row_template(
    index: &AuthenticatedGuiRoleIndex,
    catalog: &GuiDefinitionCatalog,
    admission: &ValidatedContentAdmission,
    law: &Xui26CollectionLaw,
) -> Result<StrictAuthoredPanelAdmission, String> {
    let mut candidates = index
        .surfaces
        .iter()
        .filter(|surface| {
            law.template_surface_ids
                .iter()
                .any(|id| surface.surface_id.eq_ignore_ascii_case(id))
        })
        .collect::<Vec<_>>();
    let [candidate] = candidates.as_slice() else {
        return Err(if candidates.is_empty() {
            "XUI26 collection group has no admitted authored row template".to_string()
        } else {
            format!(
                "XUI26 collection group has {} admitted authored row templates and cannot choose",
                candidates.len()
            )
        });
    };
    let admitted = index.admit_surface(
        catalog,
        admission,
        &candidate.source_content,
        &candidate.surface_id,
    )?;
    candidates.clear();
    Ok(admitted)
}

/// Whether this row element can be drawn TRUTHFULLY.
///
/// A row is instantiated once per recorded item, so its content is by
/// construction the engine's per-item output; the owner panel's chrome is the
/// same authored chrome the static-only families already present. An element
/// the authored file does not fully decide, and that the group's payload does
/// not supply, would be drawn at its authored default - the "FAILED" literal
/// `ai_trace_entry` authors on every row, or the unchecked frame of a
/// two-frame checkbox - which is a substituted value, not a late one.
///
/// So exactly two things pass: an element the proven store SUPPLIES, and an
/// element the authored derivation PROVABLY draws nothing for. `Hidden` is not
/// that proof: `static_authored_semantic` returns it both for a standard button
/// it deliberately draws nothing for AND as its fallback for a presentation it
/// could not determine - a direct-texture icon, a multi-sprite element, a
/// font-bearing box with no authored text, an unresolved token. Admitting the
/// fallback would draw those rows INVISIBLE without anything having recorded
/// that they are invisible, so only the two arms the provider reaches
/// deliberately are taken here: its `Structural` answer, and the standard-button
/// arm, whose condition is read off the compositor's own `WidgetFamily`.
fn xui26_row_element_is_determined(
    role: &crate::authored_role_index::AuthoredElementRole,
    authored: &crate::gui::GuiElement,
    semantic: &ReplayAuthenticatedElementSemantic,
    store_texts: Option<&BTreeSet<&str>>,
) -> bool {
    let Some(family) = WidgetFamily::from_element_type(&authored.element_type) else {
        // A type the compositor does not know cannot be judged at all. The
        // effective definition catalog refuses such a window before this, so
        // this is defence in depth rather than a reachable arm.
        return false;
    };
    if let Some(texts) = store_texts {
        // The store is probed with THE STRINGS THIS GROUP MEASURED, not a
        // stand-in: a literal probe proves only that some string would compose,
        // and the row binds the measured one. An empty measured string binds
        // `Hidden` instead and is not probed, because an empty string draws
        // nothing whatever the element is. Judged by the COMPOSITOR'S own rule
        // rather than a second copy of it.
        return texts.iter().all(|text| {
            crate::compose::validate_element_semantic(
                role,
                authored,
                &AuthenticatedElementSemantic::Text {
                    resolved_text: (*text).to_string(),
                },
            )
            .is_ok()
        });
    }
    if family.standard_button_carrier() {
        // The provider's deliberate draws-nothing arm, and its exact condition.
        return role.sprites.len() == 1;
    }
    // Its other deliberate arm. The compositor is asked whether it would accept
    // that answer for this element rather than a second copy of its rules being
    // kept here: `Structural` is refused for a text-typed element, and only the
    // compositor knows that.
    matches!(semantic, ReplayAuthenticatedElementSemantic::Structural)
        && crate::compose::validate_element_semantic(
            role,
            authored,
            &AuthenticatedElementSemantic::Structural,
        )
        .is_ok()
}

#[derive(Clone, Copy)]
struct Xui26CollectionLaw {
    family_id: &'static str,
    group_id: &'static str,
    host_authored_names: &'static [&'static str],
    template_surface_ids: &'static [&'static str],
    /// The authored child of the row template that the engine writes the row's
    /// one runtime string into, where that store is proven in the admitted
    /// build. `None` means no such law is proven for this group, and a row
    /// carrying text is then REFUSED rather than having an element picked for
    /// it by shape.
    row_text_authored_name: Option<&'static str>,
}

/// Exact semantic joins retained from the installed authored roots. Mods may
/// replace the effective bytes at these paths and roots, but a renamed host or
/// template is a different unproven semantic contract and remains unavailable
/// instead of inheriting a group by position.
///
/// `row_text_authored_name` for `hotjoin_requests` is a729's own store:
/// literal `"name"` (`lStack_48 = 0x656d616e`), resolves that child through the
/// owner's text-element getter at vtable `+0x78`, and writes the item's string
/// at `+0x100` into it. No other group's row store is read, so no other group
/// names one.
const XUI26_COLLECTION_LAWS: &[Xui26CollectionLaw] = &[
    Xui26CollectionLaw {
        family_id: "frontend_game_setup",
        group_id: "bookmarks",
        host_authored_names: &["bookmarks_grid"],
        template_surface_ids: &["bookmark_entry"],
        row_text_authored_name: None,
    },
    Xui26CollectionLaw {
        family_id: "frontend_game_setup",
        group_id: "countries",
        host_authored_names: &["countries_grid", "countries_mini_expanded"],
        template_surface_ids: &[
            "country_entry",
            "country_entry_medium",
            "country_entry_mini",
        ],
        row_text_authored_name: None,
    },
    Xui26CollectionLaw {
        family_id: "frontend_game_setup",
        group_id: "game_rules",
        host_authored_names: &["game_rule_grid"],
        template_surface_ids: &["game_rule_window"],
        row_text_authored_name: None,
    },
    Xui26CollectionLaw {
        family_id: "message_filters",
        group_id: "message_filters",
        host_authored_names: &["countries_listbox"],
        template_surface_ids: &["message_filters_entry"],
        row_text_authored_name: None,
    },
    Xui26CollectionLaw {
        family_id: "saved_game_rules",
        group_id: "saved_game_rules",
        host_authored_names: &["savedgamerules_grid"],
        template_surface_ids: &["savedgamerules_item"],
        row_text_authored_name: None,
    },
    Xui26CollectionLaw {
        family_id: "ingame_lobby",
        group_id: "players",
        host_authored_names: &["multiplayer_list"],
        template_surface_ids: &["ingame_multiplayer_entry"],
        row_text_authored_name: None,
    },
    Xui26CollectionLaw {
        family_id: "player_lobby",
        group_id: "players",
        host_authored_names: &["multiplayer_list"],
        template_surface_ids: &["ingame_multiplayer_entry"],
        row_text_authored_name: None,
    },
    Xui26CollectionLaw {
        family_id: "hotjoin",
        group_id: "hotjoin_requests",
        host_authored_names: &["requests_grid"],
        template_surface_ids: &["hotjoin_request_entry"],
        row_text_authored_name: Some("name"),
    },
    Xui26CollectionLaw {
        family_id: "matchmaking_chat",
        group_id: "chat_messages",
        host_authored_names: &["chat"],
        template_surface_ids: &["matchmaking_chat_item"],
        row_text_authored_name: None,
    },
    Xui26CollectionLaw {
        family_id: "ai_trace",
        group_id: "trace_entries",
        host_authored_names: &["entries"],
        template_surface_ids: &["ai_trace_entry"],
        row_text_authored_name: None,
    },
];

fn xui26_collection_law(family_id: &str, group_id: &str) -> Option<&'static Xui26CollectionLaw> {
    XUI26_COLLECTION_LAWS
        .iter()
        .find(|law| law.family_id == family_id && law.group_id == group_id)
}

pub fn derive_xui26_frontend_collections_binding(
    owner: &AuthenticatedPanelSurface,
    index: &AuthenticatedGuiRoleIndex,
    catalog: &GuiDefinitionCatalog,
    admission: &ValidatedContentAdmission,
    viewport_width: usize,
    viewport_height: usize,
) -> Result<Option<ReplayAuthenticatedFrontendCollectionsBinding>, String> {
    let family = owner
        .frontend_composition()
        .family()
        .ok_or("XUI26 collection owner has no classified family")?;
    let [binding] = family.admitted_bindings() else {
        return if family.admitted_bindings().is_empty() {
            Ok(None)
        } else {
            Err("XUI26 collection owner has multiple dynamic bindings".into())
        };
    };
    if crate::frontend_meta::frontend_family_scope(family.family_id())
        .map_err(|error| format!("XUI26 family scope rejected: {error:?}"))?
        != crate::frontend_meta::FrontendFamilyScope::ReplayPayloadShipping
        || binding.frame_hour.is_none()
        || owner.manifest_digest_sha256() != admission.manifest_digest_sha256()
        || owner.product_modset_sha256() != admission.product_modset_sha256()
        || viewport_width == 0
        || viewport_height == 0
    {
        return Err("XUI26 collection owner/frame/content identity is invalid".into());
    }
    if binding.collections.is_empty() {
        return Ok(None);
    }
    let mut collections = Vec::with_capacity(binding.collections.len());
    for collection in &binding.collections {
        let law = xui26_collection_law(family.family_id(), &collection.group_id)
            .ok_or("XUI26 collection group has no exact family-owned host/template law")?;
        // THE HOST, DERIVED: the one authored collection widget of the owner
        // panel whose name the group's law states. It used to be an element id
        // the producer named and this code checked against the same law.
        let host_roles = owner
            .element_roles()
            .iter()
            .filter(|role| {
                owner.authored_element(&role.child_path).is_some_and(|host| {
                    law.host_authored_names
                        .iter()
                        .any(|name| host.name.eq_ignore_ascii_case(name))
                })
            })
            .collect::<Vec<_>>();
        let [host_role] = host_roles.as_slice() else {
            return Err(
                "XUI26 collection group has no single authored host in the owner panel".into(),
            );
        };
        let host = owner
            .authored_element(&host_role.child_path)
            .ok_or("XUI26 collection host tree is absent")?;
        if !matches!(
            host.element_type.to_ascii_lowercase().as_str(),
            "listboxtype" | "smoothlistboxtype" | "gridboxtype" | "overlappingelementsboxtype"
        ) {
            return Err("XUI26 collection host is not an exact authored collection widget".into());
        }
        let template = xui26_row_template(index, catalog, admission, law)?;
        // A MEASURED-EMPTY group instantiates no row, so nothing below it can
        // be drawn and nothing below it is required: judging a template that
        // will render nothing would turn a measured emptiness into an
        // unavailability. Everything from here to the row loop is a ROW
        // requirement and runs only when there are rows.
        let (template_semantics, row_text_path) = if collection.rows.is_empty() {
            (BTreeMap::new(), None)
        } else {
            // The row's own semantics come from the SAME authored derivation
            // the owner panel's do, keyed by child path so a materialized
            // instance can be addressed without an element-id join.
            let template_semantics =
            crate::authenticated_semantic_provider::authenticated_theatre_template_static_semantics(
                index, admission, owner, &template,
            )
            .map_err(|error| format!("XUI26 row template semantics rejected: {error}"))?;
            // THE STORE, by the authored NAME the engine's own refresh loads,
            // and the SHAPE that refresh resolves: the text-element getter at
            // vtable +0x78 returns a font-bearing text element, so a modded
            // file that authors an icon or a button under the same name does
            // not qualify. What that child can then hold is probed with the
            // group's own measured strings through the compositor's validator.
            let row_text_path = match law.row_text_authored_name {
                Some(name) => {
                    let mut paths = template
                        .element_roles()
                        .iter()
                        .filter(|role| {
                            template.authored_element(&role.child_path).is_some_and(|element| {
                                element.name.eq_ignore_ascii_case(name)
                                    && element.element_type.to_ascii_lowercase().contains("text")
                                    && element.font.is_some()
                            })
                        })
                        .map(|role| role.child_path.clone())
                        .collect::<Vec<_>>();
                    if paths.len() != 1 {
                        return Err(
                            "XUI26 row template has no single authored text child for its proven store"
                                .into(),
                        );
                    }
                    Some(paths.remove(0))
                }
                None => None,
            };
            // A row's runtime text is carried EXACTLY when its group proves a
            // store for it: text with no store could only be placed by shape,
            // and a store with no text would leave the engine's own slot
            // holding whatever the authored file left there.
            if collection
                .rows
                .iter()
                .any(|row| row.row_text.is_some() != row_text_path.is_some())
            {
                return Err(
                    "XUI26 collection row text does not match its group's proven authored store"
                        .into(),
                );
            }
            // EVERY element of the row, or none of them. An element the
            // authored file does not decide and the payload does not supply
            // would be drawn at its authored default.
            // ALL of them, not the first: a template with three undecided
            // elements is three separate authoring problems, and naming one
            // sends the next reader back three times.
            let mut undecided = Vec::new();
            for role in template.element_roles() {
                let authored = template
                    .authored_element(&role.child_path)
                    .ok_or("XUI26 row template element tree is absent")?;
                let semantic = template_semantics
                    .get(&role.child_path)
                    .ok_or("XUI26 row template semantic census is short")?;
                let store_texts = (row_text_path.as_deref() == Some(role.child_path.as_slice()))
                    .then(|| {
                        collection
                            .rows
                            .iter()
                            .filter_map(|row| row.row_text.as_deref())
                            .filter(|text| !text.is_empty())
                            .collect::<BTreeSet<_>>()
                    });
                if !xui26_row_element_is_determined(role, authored, semantic, store_texts.as_ref()) {
                    undecided.push(role.authored_name.clone());
                }
            }
            if !undecided.is_empty() {
                return Err(format!(
                    "XUI26 row template elements {undecided:?} are neither decided by the authored file nor supplied by the payload"
                ));
            }
            (template_semantics, row_text_path)
        };
        let mut rows = Vec::with_capacity(collection.rows.len());
        for row in &collection.rows {
            let mut semantics = template_semantics.clone();
            if let (Some(text), Some(path)) = (&row.row_text, &row_text_path) {
                // The engine writes whatever string it holds, and an EMPTY one
                // draws as nothing. `Hidden` is that, exactly and losslessly;
                // it is not an absence, because the row carried a measured
                // string and this is what that string looks like.
                semantics.insert(
                    path.clone(),
                    if text.is_empty() {
                        ReplayAuthenticatedElementSemantic::Hidden
                    } else {
                        ReplayAuthenticatedElementSemantic::Text {
                            resolved_text: text.clone(),
                        }
                    },
                );
            }
            rows.push(ReplayAuthenticatedFrontendCollectionRowBinding {
                input: row.clone(),
                template: template.clone(),
                semantics,
            });
        }
        collections.push(ReplayAuthenticatedFrontendCollectionBinding {
            input: collection.clone(),
            collection_element_id: host_role.element_id.clone(),
            collection_child_path: host_role.child_path.clone(),
            empty_template: template,
            rows,
            scroll_offset_pixels: 0,
        });
    }
    let mut result = ReplayAuthenticatedFrontendCollectionsBinding {
        index_identity_sha256: index.index_identity_sha256.clone(),
        manifest_digest_sha256: admission.manifest_digest_sha256().to_string(),
        product_modset_sha256: admission.product_modset_sha256().to_string(),
        owner_window_id: owner.window_id().to_string(),
        family_id: family.family_id().to_string(),
        source_record_id: binding.source_record_id.clone(),
        frame_hour: binding.frame_hour.expect("checked above"),
        viewport_size: [viewport_width, viewport_height],
        collections,
        identity_sha256: String::new(),
    };
    result.identity_sha256 = xui26_frontend_collections_binding_digest(&result);
    Ok(Some(result))
}

#[allow(clippy::too_many_arguments)]
fn build_xui26_frontend_materialization_plan(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    owner_panel: &StrictAuthoredPanelAdmission,
    owner_lease: &AuthenticatedReplayPanelLease<'_>,
    preflight: &AuthenticatedLayoutPreflight,
    binding: &ReplayAuthenticatedFrontendCollectionsBinding,
    pointer_lifecycle: &MaterializedPointerLifecycle,
) -> Result<AuthenticatedMaterializationPlan, ReplayWindowManagerRejection> {
    let reject = || {
        ReplayWindowManagerRejection::InvalidComposedGeometry(format!(
            "{}:XUI26-materialization",
            owner_panel.window_id()
        ))
    };
    if binding.index_identity_sha256 != index.index_identity_sha256
        || binding.manifest_digest_sha256 != admission.manifest_digest_sha256()
        || binding.product_modset_sha256 != admission.product_modset_sha256()
        || binding.owner_window_id != owner_panel.window_id()
        || binding.viewport_size != preflight.viewport_size()
        || binding.collections.is_empty()
    {
        return Err(reject());
    }
    let options = ComposeOptions::new(binding.viewport_size[0], binding.viewport_size[1])
        .with_root_size(preflight.root_size()[0], preflight.root_size()[1]);
    let mut instances = Vec::new();
    let mut collection_scroll_regions = Vec::new();
    let mut content_rect: Option<Rect> = None;
    for collection in &binding.collections {
        let host_preflight = preflight
            .elements()
            .iter()
            .find(|element| element.child_path() == collection.collection_child_path)
            .ok_or_else(reject)?;
        let host_rect = host_preflight.screen_rect().ok_or_else(reject)?;
        let host_clip = host_preflight.clip_rect().unwrap_or(host_rect);
        let mut parent_path = collection.collection_child_path.clone();
        parent_path.pop();
        let parent_preflight = preflight
            .elements()
            .iter()
            .find(|element| element.child_path() == parent_path)
            .ok_or_else(reject)?;
        let parent_rect = parent_preflight.screen_rect().ok_or_else(reject)?;
        let host = owner_panel
            .authored_element(&collection.collection_child_path)
            .ok_or_else(reject)?;
        let semantic = ObserverSemanticCollection {
            collection_instance_token: format!(
                "XUI26:{}:{}:{}",
                binding.source_record_id, binding.family_id, collection.collection_element_id
            ),
            frame_hour: u64::try_from(binding.frame_hour).map_err(|_| reject())?,
            context_country: None,
            complete: collection.input.complete,
            rows: collection
                .rows
                .iter()
                .enumerate()
                .map(|(ordinal, row)| {
                    Ok(ObserverSemanticRow {
                        stable_item_id: row.input.stable_item_id.clone(),
                        source_ordinal: u32::try_from(ordinal).map_err(|_| reject())?,
                        subtype_id: row.template.surface_id().to_string(),
                    })
                })
                .collect::<Result<Vec<_>, ReplayWindowManagerRejection>>()?,
        };
        let row_bindings = collection
            .rows
            .iter()
            .map(|row| AuthoredRowBinding {
                stable_item_id: row.input.stable_item_id.as_str(),
                subtype_id: row.template.surface_id(),
                element_source_identity: row.template.surface_id(),
                instance_token: row.input.stable_item_id.as_str(),
                template: row
                    .template
                    .authored_element(&[])
                    .expect("strict admitted template always has a root"),
            })
            .collect::<Vec<_>>();
        let empty_template_root = collection
            .empty_template
            .authored_element(&[])
            .ok_or_else(reject)?;
        let owner_source_identity = format!(
            "{}#{}",
            owner_panel.source_content().normalized_game_relative_path,
            owner_panel.surface_id()
        );
        let grid_authority = host
            .element_type
            .eq_ignore_ascii_case("gridBoxType")
            .then_some(StandardGridAuthority::Xui26FrontendExactCarrierOrder);
        let plan_at = |offset_pixels: i32| {
            plan_observer_collection(
                u64::try_from(binding.frame_hour).map_err(|_| reject())?,
                &semantic,
                AuthoredCollectionSource {
                    owning_window_id: &owner_source_identity,
                    root_source_identity: &owner_source_identity,
                    collection_element_id: &collection.collection_element_id,
                    collection_element: host,
                    effective_row_template_identity: Some(collection.empty_template.surface_id()),
                    effective_row_template: Some(empty_template_root),
                    parent_rect: Rect::new(
                        parent_rect.x(),
                        parent_rect.y(),
                        parent_rect.width(),
                        parent_rect.height(),
                    ),
                    grid_authority,
                },
                &row_bindings,
                ViewerCollectionState {
                    scroll_offset_pixels: (0, offset_pixels),
                    window_stack_ordinal: 0,
                    root_paint_ordinal: host_preflight.paint_ordinal().unwrap_or(0),
                    visible_clip: Rect::new(
                        host_clip.x(),
                        host_clip.y(),
                        host_clip.width(),
                        host_clip.height(),
                    ),
                },
                &options,
            )
            .map_err(|error| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(format!(
                    "{}:XUI26-collection-plan:{error}",
                    owner_panel.window_id()
                ))
            })
        };
        let unscrolled = plan_at(0)?;
        let extent = unscrolled.content_extent_pixels();
        let max_offset_pixels = extent.1.saturating_sub(host_clip.height()).max(0);
        let effective_offset_pixels = collection.scroll_offset_pixels.min(max_offset_pixels);
        let plan = unscrolled;
        if plan.collection_rect()
            != Rect::new(
                host_rect.x(),
                host_rect.y(),
                host_rect.width(),
                host_rect.height(),
            )
        {
            return Err(reject());
        }
        let host_insertion_ordinal = u32::try_from(host.children.len()).map_err(|_| reject())?;
        for planned in plan.rows() {
            let row = collection
                .rows
                .iter()
                .find(|row| row.input.stable_item_id == planned.stable_item_id())
                .ok_or_else(reject)?;
            let semantics = row
                .semantics
                .iter()
                .map(|(path, semantic)| {
                    (
                        path.clone(),
                        authenticated_element_semantic_from_replay(semantic),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let rect = planned.final_rect();
            let scrolled_y = rect
                .y()
                .checked_sub(effective_offset_pixels)
                .ok_or_else(reject)?;
            instances.push(AuthenticatedMaterializedInstancePlan::new(
                format!(
                    "{}:{}:{}",
                    binding.family_id,
                    collection.collection_element_id,
                    row.input.stable_item_id
                ),
                u32::try_from(instances.len()).map_err(|_| reject())?,
                row.template.clone(),
                false,
                None,
                collection.collection_child_path.clone(),
                host_insertion_ordinal,
                AuthenticatedElementRect::new(rect.x(), scrolled_y, rect.w(), rect.h()),
                host_clip,
                semantics,
            ));
        }
        let authored_wheel = host
            .raw
            .first_scalar_ci("scroll_wheel_factor")
            .or_else(|| host.raw.first_scalar_ci("vertical_scroll_step"));
        let wheel_step_pixels = match authored_wheel {
            Some(value) => Some(
                value
                    .trim()
                    .parse::<i32>()
                    .ok()
                    .filter(|step| *step > 0)
                    .ok_or_else(reject)?,
            ),
            None => None,
        }
        .or_else(|| {
            authenticated_template_root_extent(
                &collection.empty_template,
                host_rect.width(),
                host_rect.height(),
            )
            .ok()
            .map(|(_, height)| height)
            .filter(|height| *height > 0)
        })
        .unwrap_or(0);
        let smooth_scrolling = match host.raw.first_scalar_ci("smooth_scrolling").map(str::trim) {
            None => false,
            Some(value) if value.eq_ignore_ascii_case("yes") => true,
            Some(value) if value.eq_ignore_ascii_case("no") => false,
            _ => return Err(reject()),
        };
        // A MEASURED-EMPTY group has nothing to scroll, so it gets no scroll
        // region: `max_offset_pixels` is zero, the wheel step is never applied,
        // and demanding a positive one - or a positive extent from a template
        // no row will instantiate - would turn a measured emptiness into a
        // refusal at the materialization seal.
        if !collection.rows.is_empty() {
            if wheel_step_pixels <= 0 {
                return Err(reject());
            }
            collection_scroll_regions.push(AuthenticatedCollectionScrollRegionPlan {
                element_id: collection.collection_element_id.clone(),
                child_path: collection.collection_child_path.clone(),
                viewport_rect: host_clip,
                max_offset_pixels,
                offset_pixels: effective_offset_pixels,
                wheel_step_pixels,
                smooth_scrolling,
                geometry_source_record: format!(
                    "{}/family={}/group={}/host={}/carrier={}",
                    binding.source_record_id,
                    binding.family_id,
                    collection.input.group_id,
                    collection.collection_element_id,
                    binding.identity_sha256,
                ),
            });
        }
        let current = Rect::new(
            host_rect.x(),
            host_rect.y(),
            extent.0.max(host_rect.width()).max(1),
            extent.1.max(host_rect.height()).max(1),
        );
        content_rect = Some(match content_rect {
            None => current,
            Some(prior) => {
                let left = prior.x().min(current.x());
                let top = prior.y().min(current.y());
                let right = prior
                    .x()
                    .checked_add(prior.w())
                    .ok_or_else(reject)?
                    .max(current.x().checked_add(current.w()).ok_or_else(reject)?);
                let bottom = prior
                    .y()
                    .checked_add(prior.h())
                    .ok_or_else(reject)?
                    .max(current.y().checked_add(current.h()).ok_or_else(reject)?);
                Rect::new(left, top, right - left, bottom - top)
            }
        });
    }
    apply_materialized_pointer_lifecycle(
        &mut instances,
        owner_panel.window_id(),
        owner_lease.runtime_instance_token(),
        pointer_lifecycle,
    )?;
    let viewport_width = i32::try_from(binding.viewport_size[0]).map_err(|_| reject())?;
    let viewport_height = i32::try_from(binding.viewport_size[1]).map_err(|_| reject())?;
    let content = content_rect.ok_or_else(reject)?;
    let scroll_identity = snapshot::sha256::to_hex(&Sha256::digest(
        format!("XUI26/{}/no-viewer-scroll", binding.identity_sha256).as_bytes(),
    ));
    seal_authenticated_materialization_plan(
        manager,
        frame_authority,
        index,
        admission,
        owner_panel,
        owner_lease,
        AuthenticatedElementRect::new(0, 0, viewport_width, viewport_height),
        AuthenticatedElementRect::new(content.x(), content.y(), content.w(), content.h()),
        0,
        scroll_identity,
        instances,
        None,
        collection_scroll_regions,
    )
    .map_err(|error| {
        ReplayWindowManagerRejection::InvalidComposedGeometry(format!(
            "{}:XUI26-materialization-seal:{error:?}",
            owner_panel.window_id()
        ))
    })
}

fn derive_xui12_extended_scrollbar_binding(
    owner: &AuthenticatedPanelSurface,
    index: &AuthenticatedGuiRoleIndex,
    catalog: &GuiDefinitionCatalog,
    admission: &ValidatedContentAdmission,
    list: &crate::authored_role_index::AuthoredElementRole,
) -> Result<ReplayAuthenticatedExtendedScrollbarBinding, String> {
    let authored_list = owner
        .authored_element(&list.child_path)
        .ok_or_else(|| "XUI12 production_lines authored list disappeared".to_string())?;
    let style_name = authored_list
        .raw
        .first_scalar_ci("verticalScrollbar")
        .map(|value| value.trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "XUI12 production_lines lacks verticalScrollbar".to_string())?;
    let effective = catalog
        .layouts
        .documents
        .iter()
        .flat_map(|document| {
            document
                .roots
                .iter()
                .map(move |root| (document.source_file.as_str(), root))
        })
        .filter(|(_, root)| root.name.eq_ignore_ascii_case(&style_name))
        .last()
        .ok_or_else(|| format!("XUI12 extended-scrollbar style {style_name:?} is missing"))?;
    let (source_file, style_root) = effective;
    if !style_root
        .element_type
        .eq_ignore_ascii_case("extendedScrollbarType")
    {
        return Err(format!(
            "XUI12 effective style {style_name:?} is not an extendedScrollbarType"
        ));
    }
    if !style_root
        .orientation
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("upper_right"))
        || !style_root
            .raw
            .first_scalar_ci("origo")
            .is_some_and(|value| value.trim_matches('"').eq_ignore_ascii_case("upper_right"))
        || style_root
            .raw
            .first_scalar_ci("horizontal")
            .is_some_and(|value| !value.eq_ignore_ascii_case("no"))
    {
        return Err(
            "XUI12 requires the exact vertical upper-right extended-scrollbar family".into(),
        );
    }
    let position = style_root
        .position
        .as_ref()
        .map(|pair| {
            if pair.x.is_percent || pair.y.is_percent {
                return Err::<[i32; 2], String>(
                    "XUI12 extended-scrollbar percentage root position is unsupported".into(),
                );
            }
            Ok([
                crate::compose::measure_to_pixels_for_parent(&pair.x, 1),
                crate::compose::measure_to_pixels_for_parent(&pair.y, 1),
            ])
        })
        .transpose()?
        .unwrap_or([0, 0]);
    let size = style_root
        .size
        .as_ref()
        .ok_or_else(|| "XUI12 extended-scrollbar root has no authored size".to_string())?;
    if size.x.is_percent || size.y.is_percent {
        return Err("XUI12 extended-scrollbar percentage root size is unsupported".into());
    }
    let width = crate::compose::measure_to_pixels_for_parent(&size.x, 1);
    if width <= 0 {
        return Err("XUI12 extended-scrollbar width is not positive".into());
    }
    let source_content = admission
        .require_used_content(source_file)
        .map_err(|error| error.to_string())?
        .clone();
    Ok(ReplayAuthenticatedExtendedScrollbarBinding {
        style_root: style_root.clone(),
        source_content,
        position,
        width,
        background: xui12_scrollbar_part(owner, index, admission, style_root, "background")?,
        track: xui12_scrollbar_part(owner, index, admission, style_root, "track")?,
        decrease: xui12_scrollbar_part(owner, index, admission, style_root, "decreaseButton")?,
        increase: xui12_scrollbar_part(owner, index, admission, style_root, "increaseButton")?,
        slider: xui12_scrollbar_part(owner, index, admission, style_root, "slider")?,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_exact_a729_production_lines_materialization_plan(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    owner_panel: &StrictAuthoredPanelAdmission,
    owner_lease: &AuthenticatedReplayPanelLease<'_>,
    preflight: &AuthenticatedLayoutPreflight,
    binding: &ReplayAuthenticatedProductionLinesBinding,
    pointer_lifecycle: &MaterializedPointerLifecycle,
) -> Result<AuthenticatedMaterializationPlan, ReplayWindowManagerRejection> {
    let reject = || {
        ReplayWindowManagerRejection::InvalidComposedGeometry(format!(
            "{}:XUI12-materialization",
            owner_panel.window_id()
        ))
    };
    if binding.index_identity_sha256 != index.index_identity_sha256
        || binding.manifest_digest_sha256 != admission.manifest_digest_sha256()
        || binding.product_modset_sha256 != admission.product_modset_sha256()
        || binding.owner_window_id != owner_panel.window_id()
        || binding.viewport_size != preflight.viewport_size()
        || binding.scroll_offset_pixels < 0
    {
        return Err(reject());
    }
    let list = preflight
        .elements()
        .iter()
        .find(|element| element.child_path() == binding.list_child_path)
        .ok_or_else(reject)?;
    let grid = preflight
        .elements()
        .iter()
        .find(|element| element.child_path() == binding.grid_child_path)
        .ok_or_else(reject)?;
    let list_clip = list.clip_rect().ok_or_else(reject)?;
    let grid_rect = grid.screen_rect().ok_or_else(reject)?;
    let authored_grid = owner_panel
        .authored_element(&binding.grid_child_path)
        .filter(|element| element.element_type.eq_ignore_ascii_case("gridBoxType"))
        .ok_or_else(reject)?;
    if !authored_grid
        .raw
        .first_scalar_ci("format")
        .is_some_and(|value| value.trim_matches('"').eq_ignore_ascii_case("UPPER_LEFT"))
        || authored_grid
            .raw
            .first_scalar_ci("max_slots_horizontal")
            .and_then(|value| value.trim().parse::<u32>().ok())
            != Some(1)
        || !authored_grid
            .raw
            .first_scalar_ci("add_horizontal")
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("no"))
    {
        return Err(reject());
    }
    let slot = authored_grid
        .raw
        .first_value_ci("slotsize")
        .and_then(crate::gui::parse_measure_pair)
        .ok_or_else(reject)?;
    let slot_width = crate::compose::measure_to_pixels_for_parent(&slot.x, grid_rect.width());
    let slot_height = crate::compose::measure_to_pixels_for_parent(&slot.y, grid_rect.height());
    if slot_width <= 0 || slot_height <= 0 {
        return Err(reject());
    }
    let layout = crate::widget_layout::GridBoxLayout {
        rect: Rect::new(
            grid_rect.x(),
            grid_rect.y(),
            grid_rect.width(),
            grid_rect.height(),
        ),
        format: crate::widget_layout::GridBoxFormat::UpperLeft,
        slot_size: (slot_width, slot_height),
        max_slots_x: Some(1),
        max_slots_y: None,
        add_horizontal: Some(false),
    };
    let host_insertion_ordinal =
        u32::try_from(authored_grid.children.len()).map_err(|_| reject())?;
    let canvas_rows = binding
        .rows
        .iter()
        .map(|row| crate::production_lines::AuthenticatedXui12CanvasRow {
            global_source_ordinal: row.input.global_source_ordinal,
            resolved_width_px: row.input.resolved_width_px,
            resolved_height_px: row.input.resolved_height_px,
        })
        .collect::<Vec<_>>();
    let content_height = binding
        .rows
        .iter()
        .try_fold(0i32, |height, row| {
            height.checked_add(row.input.resolved_height_px)
        })
        .ok_or_else(reject)?;
    let max_offset = content_height.saturating_sub(list_clip.height()).max(0);
    let effective_scroll_offset = binding.scroll_offset_pixels.min(max_offset);
    let rects = crate::production_lines::plan_xui12_canvas_row_rects(
        &layout,
        &canvas_rows,
        effective_scroll_offset,
    )
    .map_err(|_| reject())?;
    let mut instances = Vec::with_capacity(binding.rows.len());
    for (row, planned) in binding.rows.iter().zip(rects) {
        let mut semantics = BTreeMap::new();
        for role in row.template.element_roles() {
            let semantic = row
                .input
                .semantics
                .get(&role.element_id)
                .ok_or_else(reject)?;
            semantics.insert(
                role.child_path.clone(),
                authenticated_element_semantic_from_replay(semantic),
            );
        }
        let ordinal = u32::try_from(instances.len()).map_err(|_| reject())?;
        let mut instance = AuthenticatedMaterializedInstancePlan::new(
            format!("{:?}", row.input.row_identity),
            ordinal,
            row.template.clone(),
            false,
            None,
            binding.grid_child_path.clone(),
            host_insertion_ordinal,
            AuthenticatedElementRect::new(
                planned.rect.x(),
                planned.rect.y(),
                planned.rect.w(),
                planned.rect.h(),
            ),
            list_clip,
            semantics,
        );
        if let Some(path) = row.expand_child_path.clone() {
            instance
                .bind_viewer_action(
                    path,
                    ReplayViewerAction::ProductionLines(ProductionLinesViewerAction::ToggleRow(
                        row.input.row_identity.clone(),
                    )),
                )
                .map_err(|_| reject())?;
        }
        instances.push(instance);
    }
    apply_materialized_pointer_lifecycle(
        &mut instances,
        owner_panel.window_id(),
        owner_lease.runtime_instance_token(),
        pointer_lifecycle,
    )?;
    let viewport_width = i32::try_from(binding.viewport_size[0]).map_err(|_| reject())?;
    let viewport_height = i32::try_from(binding.viewport_size[1]).map_err(|_| reject())?;
    let visible_content_y = grid_rect
        .y()
        .checked_sub(effective_scroll_offset)
        .ok_or_else(reject)?;
    let scroll_identity = snapshot::sha256::to_hex(&Sha256::digest(
        format!(
            "XUI12/{}/{}/{}/{max_offset}",
            binding.identity_sha256,
            list_clip.width(),
            list_clip.height()
        )
        .as_bytes(),
    ));
    let style = &binding.scrollbar;
    let scrollbar_x = list_clip
        .x()
        .checked_add(list_clip.width())
        .and_then(|value| value.checked_add(style.position[0]))
        .and_then(|value| value.checked_sub(style.width))
        .ok_or_else(reject)?;
    let scrollbar_y = list_clip
        .y()
        .checked_add(style.position[1])
        .ok_or_else(reject)?;
    let decrease_x = scrollbar_x
        .checked_add(style.decrease.position[0])
        .ok_or_else(reject)?;
    let decrease_y = scrollbar_y
        .checked_add(style.decrease.position[1])
        .ok_or_else(reject)?;
    let increase_x = scrollbar_x
        .checked_add(style.width)
        .and_then(|value| value.checked_add(style.increase.position[0]))
        .ok_or_else(reject)?;
    let increase_y = scrollbar_y
        .checked_add(list_clip.height())
        .and_then(|value| value.checked_add(style.increase.position[1]))
        .ok_or_else(reject)?;
    let thumb_base_y = decrease_y
        .checked_add(style.decrease.natural_size[1])
        .ok_or_else(reject)?;
    let track_height = increase_y.checked_sub(thumb_base_y).ok_or_else(reject)?;
    let thumb_travel = track_height
        .checked_sub(style.slider.natural_size[1])
        .ok_or_else(reject)?;
    if track_height <= 0 || thumb_travel < 0 {
        return Err(reject());
    }
    let max_fixed = i64::from(max_offset)
        .checked_mul(100_000)
        .ok_or_else(reject)?;
    let value_fixed = i64::from(effective_scroll_offset)
        .checked_mul(100_000)
        .ok_or_else(reject)?;
    let (_, mapped_thumb_y) = extended_thumb_pixel(ExtendedScrollbarCarrier {
        authority: ScrollbarCarrierAuthority::ExactA729Runtime,
        axis: ScrollbarAxis::Vertical,
        min_fixed: 0,
        max_fixed,
        value_fixed,
        base_pixel: (scrollbar_x, thumb_base_y),
        travel_pixels: (0, thumb_travel),
    })
    .map_err(|_| reject())?;
    let slider_x = scrollbar_x
        .checked_add(style.slider.position[0])
        .ok_or_else(reject)?;
    let slider_y = mapped_thumb_y
        .checked_add(style.slider.position[1])
        .ok_or_else(reject)?;
    let part = |binding: &ReplayAuthenticatedScrollbarPartBinding,
                x: i32,
                y: i32,
                width: i32,
                height: i32| AuthenticatedScrollbarPartPlan {
        sprite_name: binding.sprite_name.clone(),
        rect: AuthenticatedElementRect::new(x, y, width, height),
    };
    let scrollbar = AuthenticatedExtendedScrollbarPlan {
        style_root: style.style_root.clone(),
        source_content: style.source_content.clone(),
        clip_rect: AuthenticatedElementRect::new(0, 0, viewport_width, viewport_height),
        background: part(
            &style.background,
            scrollbar_x,
            scrollbar_y,
            style.width,
            list_clip.height(),
        ),
        track: part(
            &style.track,
            scrollbar_x
                .checked_add(style.track.position[0])
                .ok_or_else(reject)?,
            thumb_base_y,
            style.track.natural_size[0],
            track_height,
        ),
        decrease: part(
            &style.decrease,
            decrease_x,
            decrease_y,
            style.decrease.natural_size[0],
            style.decrease.natural_size[1],
        ),
        increase: part(
            &style.increase,
            increase_x,
            increase_y,
            style.increase.natural_size[0],
            style.increase.natural_size[1],
        ),
        slider: part(
            &style.slider,
            slider_x,
            slider_y,
            style.slider.natural_size[0],
            style.slider.natural_size[1],
        ),
        thumb_base_pixels: thumb_base_y,
        thumb_travel_pixels: thumb_travel,
        thumb_authored_offset_pixels: style.slider.position[1],
    };
    seal_authenticated_materialization_plan(
        manager,
        frame_authority,
        index,
        admission,
        owner_panel,
        owner_lease,
        AuthenticatedElementRect::new(0, 0, viewport_width, viewport_height),
        AuthenticatedElementRect::new(
            grid_rect.x(),
            visible_content_y,
            grid_rect.width().max(1),
            content_height.max(1),
        ),
        effective_scroll_offset,
        scroll_identity,
        instances,
        Some(scrollbar),
        Vec::new(),
    )
    .map_err(|_| reject())
}

pub fn derive_exact_a729_production_lines_binding(
    owner: &AuthenticatedPanelSurface,
    index: &AuthenticatedGuiRoleIndex,
    catalog: &GuiDefinitionCatalog,
    admission: &ValidatedContentAdmission,
    viewport_width: usize,
    viewport_height: usize,
    input: &crate::production_lines::StrictXui12CanvasInput,
) -> Result<ReplayAuthenticatedProductionLinesBinding, String> {
    input
        .validate_shape()
        .map_err(|error| format!("XUI12 typed adapter rejected: {error:?}"))?;
    if !owner
        .surface_id()
        .eq_ignore_ascii_case(crate::production_lines::COUNTRY_PRODUCTION_LINE_ROOT)
        || owner.manifest_digest_sha256() != admission.manifest_digest_sha256()
        || owner.product_modset_sha256() != admission.product_modset_sha256()
        || viewport_width == 0
        || viewport_height == 0
    {
        return Err("XUI12 owner/content/viewport identity is invalid".into());
    }
    let exact_roles = |name: &str| {
        owner
            .element_roles()
            .iter()
            .filter(|role| role.authored_name.eq_ignore_ascii_case(name))
            .collect::<Vec<_>>()
    };
    let list_roles = exact_roles("production_lines");
    let [list] = list_roles.as_slice() else {
        return Err("XUI12 exact production_lines viewport is missing or ambiguous".into());
    };
    let grid_roles = exact_roles("item_grid");
    let [grid] = grid_roles.as_slice() else {
        return Err("XUI12 exact item_grid is missing or ambiguous".into());
    };
    let scrollbar =
        derive_xui12_extended_scrollbar_binding(owner, index, catalog, admission, list)?;
    let owner_ids = owner
        .element_roles()
        .iter()
        .map(|role| role.element_id.as_str())
        .collect::<BTreeSet<_>>();
    if owner_ids.len() != input.owner_semantics.len()
        || !input
            .owner_semantics
            .keys()
            .all(|element_id| owner_ids.contains(element_id.as_str()))
    {
        return Err("XUI12 owner semantic census is incomplete or foreign".into());
    }
    let mut rows = Vec::with_capacity(input.rows.len());
    for row in &input.rows {
        let candidates = index
            .surfaces
            .iter()
            .filter(|surface| {
                surface.surface_id == row.template_name
                    && surface.source_content == *owner.source_content()
            })
            .collect::<Vec<_>>();
        let [template_role] = candidates.as_slice() else {
            return Err(format!(
                "XUI12 template {:?} is missing or ambiguous in the exact owner source",
                row.template_name
            ));
        };
        let template = index
            .admit_panel(catalog, admission, &template_role.root_window_id)
            .map_err(|error| format!("XUI12 template admission rejected: {error:?}"))?;
        let ids = template
            .element_roles()
            .iter()
            .map(|role| role.element_id.as_str())
            .collect::<BTreeSet<_>>();
        if ids.len() != row.semantics.len()
            || !row
                .semantics
                .keys()
                .all(|element_id| ids.contains(element_id.as_str()))
        {
            return Err(format!(
                "XUI12 template {:?} semantic census is incomplete or foreign",
                row.template_name
            ));
        }
        let expand = template
            .element_roles()
            .iter()
            .filter(|role| role.authored_name.eq_ignore_ascii_case("expand"))
            .collect::<Vec<_>>();
        let expand_child_path = match expand.as_slice() {
            [] if matches!(
                &row.row_identity,
                crate::production_lines::Xui12RowIdentity::OptionalNewLine(_)
            ) =>
            {
                None
            }
            [role] => {
                if !matches!(
                    row.semantics.get(&role.element_id),
                    Some(ReplayAuthenticatedElementSemantic::ReadOnlyButton {
                        interaction: crate::runtime_graphics::ButtonVisualState::Up
                            | crate::runtime_graphics::ButtonVisualState::Down
                            | crate::runtime_graphics::ButtonVisualState::Over,
                        ..
                    })
                ) {
                    return Err(
                        "XUI12 expand action lacks exact active read-only button semantics".into(),
                    );
                }
                Some(role.child_path.clone())
            }
            _ => return Err("XUI12 expand action is missing or ambiguous".into()),
        };
        rows.push(ReplayAuthenticatedProductionRowBinding {
            input: row.clone(),
            template,
            expand_child_path,
        });
    }
    let close_roles = exact_roles("close_button");
    let [close] = close_roles.as_slice() else {
        return Err("XUI12 exact close action is missing or ambiguous".into());
    };
    if !matches!(
        input.owner_semantics.get(&close.element_id),
        Some(ReplayAuthenticatedElementSemantic::StandardButton)
    ) {
        return Err("XUI12 close action lacks exact active button semantics".into());
    }
    let owner_pointer_bindings = owner
        .element_roles()
        .iter()
        .map(|role| {
            let authority = if role.element_id == close.element_id {
                ReplayElementPointerAuthority::Interactive {
                    visual_enabled: true,
                }
            } else {
                ReplayElementPointerAuthority::PassThrough
            };
            (role.element_id.clone(), authority)
        })
        .collect::<BTreeMap<_, _>>();
    let owner_viewer_actions = vec![ReplayViewerActionBinding::from_authenticated_owner(
        &owner.panel,
        close.child_path.clone(),
        close.child_path.clone(),
        ReplayViewerAction::ProductionLines(ProductionLinesViewerAction::Close),
    )];
    let mut binding = ReplayAuthenticatedProductionLinesBinding {
        index_identity_sha256: index.index_identity_sha256.clone(),
        manifest_digest_sha256: admission.manifest_digest_sha256().to_string(),
        product_modset_sha256: admission.product_modset_sha256().to_string(),
        owner_window_id: owner.window_id().to_string(),
        viewport_size: [viewport_width, viewport_height],
        country_tag: input.country_tag.clone(),
        completeness_record: input.completeness_record.clone(),
        list_element_id: list.element_id.clone(),
        list_child_path: list.child_path.clone(),
        grid_child_path: grid.child_path.clone(),
        scrollbar,
        rows,
        scroll_offset_pixels: input.scroll_offset_px,
        owner_semantics: input.owner_semantics.clone(),
        owner_pointer_bindings,
        owner_viewer_actions,
        identity_sha256: String::new(),
    };
    binding.refresh_identity();
    Ok(binding)
}

pub fn derive_exact_a729_persistent_hud_bindings(
    surfaces: &[AuthenticatedPanelSurface],
    hud: &crate::hud::HudBinding,
    viewport_width: usize,
    viewport_height: usize,
    theatre_available: bool,
) -> Result<Vec<ReplayAuthenticatedPersistentHudBinding>, String> {
    if viewport_width == 0 || viewport_height == 0 || surfaces.is_empty() {
        return Err("persistent HUD binding viewport or surface set is empty".into());
    }
    let derived =
        crate::authenticated_semantic_provider::derive_exact_a729_persistent_hud_bindings(
            surfaces,
            hud,
            theatre_available,
        )
        .map_err(|error| error.to_string())?;
    if derived.len() != surfaces.len() {
        return Err("persistent HUD semantic provider returned an incomplete census".into());
    }
    surfaces
        .iter()
        .zip(derived)
        .map(|(surface, derived)| {
            if derived.window_id != surface.window_id() {
                return Err("persistent HUD semantic provider changed surface order".into());
            }
            let mut binding = ReplayAuthenticatedPersistentHudBinding {
                index_identity_sha256: surface.panel.index_identity_sha256().to_string(),
                manifest_digest_sha256: surface.manifest_digest_sha256().to_string(),
                product_modset_sha256: surface.product_modset_sha256().to_string(),
                window_id: surface.window_id().to_string(),
                surface_id: surface.surface_id().to_string(),
                viewport_width,
                viewport_height,
                semantics: derived.semantics,
                element_pointer_bindings: derived.pointer_bindings,
                viewer_actions: derived.viewer_actions,
                identity_sha256: String::new(),
            };
            binding.identity_sha256 = persistent_hud_binding_digest(&binding);
            Ok(binding)
        })
        .collect()
}

fn persistent_hud_binding_digest(binding: &ReplayAuthenticatedPersistentHudBinding) -> String {
    snapshot::sha256::to_hex(&Sha256::digest(
        format!(
            "{:?}",
            (
                &binding.index_identity_sha256,
                &binding.manifest_digest_sha256,
                &binding.product_modset_sha256,
                &binding.window_id,
                &binding.surface_id,
                binding.viewport_width,
                binding.viewport_height,
                &binding.semantics,
                &binding.element_pointer_bindings,
                &binding.viewer_actions,
            )
        )
        .as_bytes(),
    ))
}

pub struct ReplayAuthenticatedPanelFrameInput<'a> {
    surface: &'a AuthenticatedPanelSurface,
    panel: &'a StrictAuthoredPanelAdmission,
    viewport_width: usize,
    viewport_height: usize,
    semantics: &'a BTreeMap<String, ReplayAuthenticatedElementSemantic>,
    element_pointer_bindings: &'a BTreeMap<String, ReplayElementPointerAuthority>,
    scroll_bindings: &'a [ReplayAuthenticatedScrollBinding],
    viewer_actions: &'a [ReplayViewerActionBinding],
    tooltip_targets: &'a [String],
    tooltip_selectors: &'a BTreeMap<String, ReplayTooltipAlternateSelection>,
    activation: Option<&'a ReplayPanelActivation>,
    layer: ReplayAuthenticatedPanelLayer,
    persistent_capability: Option<&'a ReplayAuthenticatedPersistentHudBinding>,
    theatre_capability: Option<&'a ReplayAuthenticatedTheatreBinding>,
    production_lines_capability: Option<&'a ReplayAuthenticatedProductionLinesBinding>,
    frontend_collections_capability: Option<&'a ReplayAuthenticatedFrontendCollectionsBinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplayAuthenticatedPanelLayer {
    PersistentHud,
    ViewerPanel,
}

impl<'a> ReplayAuthenticatedPanelFrameInput<'a> {
    pub fn new(
        surface: &'a AuthenticatedPanelSurface,
        viewport_width: usize,
        viewport_height: usize,
        semantics: &'a BTreeMap<String, ReplayAuthenticatedElementSemantic>,
        element_pointer_bindings: &'a BTreeMap<String, ReplayElementPointerAuthority>,
        scroll_bindings: &'a [ReplayAuthenticatedScrollBinding],
        viewer_actions: &'a [ReplayViewerActionBinding],
        tooltip_selectors: &'a BTreeMap<String, ReplayTooltipAlternateSelection>,
        activation: &'a ReplayPanelActivation,
    ) -> Self {
        Self {
            surface,
            panel: &surface.panel,
            viewport_width,
            viewport_height,
            semantics,
            element_pointer_bindings,
            scroll_bindings,
            viewer_actions,
            tooltip_targets: &activation.tooltip_targets,
            tooltip_selectors,
            activation: Some(activation),
            layer: ReplayAuthenticatedPanelLayer::ViewerPanel,
            persistent_capability: None,
            theatre_capability: None,
            production_lines_capability: None,
            frontend_collections_capability: None,
        }
    }

    pub fn persistent_hud(
        surface: &'a AuthenticatedPanelSurface,
        binding: &'a ReplayAuthenticatedPersistentHudBinding,
        scroll_bindings: &'a [ReplayAuthenticatedScrollBinding],
        tooltip_targets: &'a [String],
        tooltip_selectors: &'a BTreeMap<String, ReplayTooltipAlternateSelection>,
        theatre_capability: Option<&'a ReplayAuthenticatedTheatreBinding>,
    ) -> Self {
        Self {
            surface,
            panel: &surface.panel,
            viewport_width: binding.viewport_width,
            viewport_height: binding.viewport_height,
            semantics: &binding.semantics,
            element_pointer_bindings: &binding.element_pointer_bindings,
            scroll_bindings,
            viewer_actions: &binding.viewer_actions,
            tooltip_targets,
            tooltip_selectors,
            activation: None,
            layer: ReplayAuthenticatedPanelLayer::PersistentHud,
            persistent_capability: Some(binding),
            theatre_capability,
            production_lines_capability: None,
            frontend_collections_capability: None,
        }
    }

    pub fn production_lines(
        surface: &'a AuthenticatedPanelSurface,
        tooltip_selectors: &'a BTreeMap<String, ReplayTooltipAlternateSelection>,
        activation: &'a ReplayPanelActivation,
        capability: &'a ReplayAuthenticatedProductionLinesBinding,
    ) -> Self {
        Self {
            surface,
            panel: &surface.panel,
            viewport_width: capability.viewport_size[0],
            viewport_height: capability.viewport_size[1],
            semantics: &capability.owner_semantics,
            element_pointer_bindings: &capability.owner_pointer_bindings,
            scroll_bindings: &[],
            viewer_actions: &capability.owner_viewer_actions,
            tooltip_targets: &activation.tooltip_targets,
            tooltip_selectors,
            activation: Some(activation),
            layer: ReplayAuthenticatedPanelLayer::ViewerPanel,
            persistent_capability: None,
            theatre_capability: None,
            production_lines_capability: Some(capability),
            frontend_collections_capability: None,
        }
    }

    pub fn frontend_collections(
        surface: &'a AuthenticatedPanelSurface,
        viewport_width: usize,
        viewport_height: usize,
        semantics: &'a BTreeMap<String, ReplayAuthenticatedElementSemantic>,
        element_pointer_bindings: &'a BTreeMap<String, ReplayElementPointerAuthority>,
        tooltip_selectors: &'a BTreeMap<String, ReplayTooltipAlternateSelection>,
        activation: &'a ReplayPanelActivation,
        capability: &'a ReplayAuthenticatedFrontendCollectionsBinding,
    ) -> Self {
        Self {
            surface,
            panel: &surface.panel,
            viewport_width,
            viewport_height,
            semantics,
            element_pointer_bindings,
            scroll_bindings: &[],
            viewer_actions: &[],
            tooltip_targets: &activation.tooltip_targets,
            tooltip_selectors,
            activation: Some(activation),
            layer: ReplayAuthenticatedPanelLayer::ViewerPanel,
            persistent_capability: None,
            theatre_capability: None,
            production_lines_capability: None,
            frontend_collections_capability: Some(capability),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayHitTarget {
    pub window_id: String,
    pub runtime_instance_token: String,
    pub source_file: String,
    pub surface_id: String,
    pub element_id: String,
    pub authored_name: String,
    pub child_path: Vec<u32>,
    pub global_root_sibling: bool,
    pub element_type: String,
    pub screen_rect: Rect,
    pub clip_rect: Rect,
    pub paint_ordinal: u32,
    pub disposition: ReplayPointerDisposition,
    pub visual_enabled: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayScrollAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayScrollDeltaPixels(i32);

impl ReplayScrollDeltaPixels {
    pub(crate) fn from_authenticated_family(delta_pixels: i32) -> Self {
        Self(delta_pixels)
    }

    pub fn value(self) -> i32 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayExtendedScrollbarGeometry {
    pub background_rect: Rect,
    pub track_rect: Rect,
    pub decrease_rect: Rect,
    pub increase_rect: Rect,
    pub slider_rect: Rect,
    pub thumb_base_pixels: i32,
    pub thumb_travel_pixels: i32,
    pub thumb_authored_offset_pixels: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayScrollRegion {
    pub window_id: String,
    pub runtime_instance_token: String,
    pub element_id: String,
    pub child_path: Vec<u32>,
    pub viewport_rect: Rect,
    pub axis: ReplayScrollAxis,
    pub min_offset_pixels: i32,
    pub max_offset_pixels: i32,
    pub offset_pixels: i32,
    pub wheel_step_pixels: Option<i32>,
    pub smooth_scrolling: bool,
    pub extended_scrollbar: Option<ReplayExtendedScrollbarGeometry>,
    pub geometry_source_record: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayComposedPanelGeometry {
    pub window_id: String,
    pub runtime_instance_token: String,
    pub source_file: String,
    pub surface_id: String,
    pub screen_rect: Rect,
    pub clip_rect: Rect,
    pub geometry_source_record: String,
    pub element_states: Vec<ReplayComposedElementState>,
    pub hit_targets: Vec<ReplayHitTarget>,
    pub scroll_regions: Vec<ReplayScrollRegion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayComposedElementState {
    pub window_id: String,
    pub runtime_instance_token: String,
    pub source_file: String,
    pub surface_id: String,
    pub element_id: String,
    pub authored_name: String,
    pub child_path: Vec<u32>,
    pub global_root_sibling: bool,
    pub element_type: String,
    pub paint_ordinal: Option<u32>,
    pub screen_rect: Option<Rect>,
    pub clip_rect: Option<Rect>,
    pub disposition: ReplayPointerDisposition,
    pub visible: bool,
    pub visual_enabled: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayPointerTarget {
    pub window_id: String,
    pub runtime_instance_token: String,
    pub source_file: String,
    pub surface_id: String,
    pub element_id: String,
    pub authored_name: String,
    pub child_path: Vec<u32>,
    pub global_root_sibling: bool,
    pub element_type: String,
    pub screen_rect: Rect,
    pub clip_rect: Rect,
    pub stack_ordinal: u32,
    pub paint_ordinal: u32,
    pub disposition: ReplayPointerDisposition,
    pub visual_enabled: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayPointerRelease {
    pub pointer_x: i32,
    pub pointer_y: i32,
    pub pressed: Option<ReplayPointerTarget>,
    pub released_over: Option<ReplayPointerTarget>,
    activated: bool,
    activated_action: Option<AuthenticatedReplayViewerAction>,
}

impl ReplayPointerRelease {
    pub fn unavailable(pointer_x: i32, pointer_y: i32) -> Self {
        Self {
            pointer_x,
            pointer_y,
            pressed: None,
            released_over: None,
            activated: false,
            activated_action: None,
        }
    }

    pub fn activated(&self) -> bool {
        self.activated
    }

    /// Exact manager-authenticated identity consumed by the one-shot release.
    pub fn activated_target(&self) -> Option<&ReplayPointerTarget> {
        self.activated
            .then_some(self.released_over.as_ref())
            .flatten()
    }

    pub fn activated_action(&self) -> Option<&AuthenticatedReplayViewerAction> {
        self.activated
            .then_some(self.activated_action.as_ref())
            .flatten()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayAuthenticatedComposedPanel {
    window_id: String,
    runtime_instance_token: String,
    frame_generation: u64,
    drawn_elements: usize,
    effective_scroll_offsets: BTreeMap<String, i32>,
}

impl ReplayAuthenticatedComposedPanel {
    pub fn window_id(&self) -> &str {
        &self.window_id
    }

    pub fn runtime_instance_token(&self) -> &str {
        &self.runtime_instance_token
    }

    pub fn frame_generation(&self) -> u64 {
        self.frame_generation
    }

    pub fn drawn_elements(&self) -> usize {
        self.drawn_elements
    }

    pub fn effective_scroll_offsets(&self) -> &BTreeMap<String, i32> {
        &self.effective_scroll_offsets
    }
}

/// Manager-sealed identity for one currently open panel lifetime.
#[derive(Debug)]
pub struct AuthenticatedReplayPanelLease<'a> {
    manager: &'a ReplayOwnedWindowManager,
    window_id: String,
    runtime_instance_token: String,
}

impl AuthenticatedReplayPanelLease<'_> {
    pub fn window_id(&self) -> &str {
        &self.window_id
    }

    pub(crate) fn runtime_instance_token(&self) -> &str {
        &self.runtime_instance_token
    }

    pub(crate) fn remains_current(&self) -> bool {
        self.manager
            .current_runtime_instance_token(&self.window_id)
            .is_some_and(|token| token == self.runtime_instance_token.as_str())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedReplayFrameAuthority {
    manager_namespace_ordinal: u64,
    frame_generation: u64,
    viewer_clock_micros: u64,
}

#[derive(Debug)]
pub(crate) struct StagedReplayCanonicalStates {
    frame_authority: AuthenticatedReplayFrameAuthority,
    states: BTreeMap<(String, Vec<u32>), AuthenticatedViewerWidgetState>,
    viewer_actions: BTreeMap<(String, Vec<u32>), AuthenticatedReplayViewerAction>,
}

impl StagedReplayCanonicalStates {
    pub(crate) fn frame_authority(&self) -> &AuthenticatedReplayFrameAuthority {
        &self.frame_authority
    }

    pub(crate) fn states(&self) -> &BTreeMap<(String, Vec<u32>), AuthenticatedViewerWidgetState> {
        &self.states
    }
}

impl AuthenticatedReplayFrameAuthority {
    pub(crate) fn frame_generation(&self) -> u64 {
        self.frame_generation
    }

    pub(crate) fn viewer_clock_micros(&self) -> u64 {
        self.viewer_clock_micros
    }

    pub(crate) fn remains_staged_for(&self, manager: &ReplayOwnedWindowManager) -> bool {
        self.manager_namespace_ordinal == manager.manager_namespace_ordinal
            && manager
                .staged_frame_generation
                .as_ref()
                .is_some_and(|staged| {
                    staged.generation == self.frame_generation
                        && staged.viewer_clock_micros == self.viewer_clock_micros
                })
    }
}

/// edge. The manager refuses a second authority until the matching release has
#[derive(Debug)]
pub(crate) struct AuthenticatedReplayPressAuthority {
    manager_namespace_ordinal: u64,
    input_transition_generation: u64,
    window_id: String,
    runtime_instance_token: String,
    child_path: Vec<u32>,
}

impl AuthenticatedReplayPressAuthority {
    pub(crate) fn input_transition_generation(&self) -> u64 {
        self.input_transition_generation
    }

    pub(crate) fn window_id(&self) -> &str {
        &self.window_id
    }

    pub(crate) fn runtime_instance_token(&self) -> &str {
        &self.runtime_instance_token
    }

    pub(crate) fn child_path(&self) -> &[u32] {
        &self.child_path
    }

    pub(crate) fn remains_active_for(&self, manager: &ReplayOwnedWindowManager) -> bool {
        manager
            .active_input_transition
            .as_ref()
            .is_some_and(|active| {
                self.manager_namespace_ordinal == manager.manager_namespace_ordinal
                    && self.input_transition_generation == active.generation
                    && self.window_id == active.window_id
                    && self.runtime_instance_token == active.runtime_instance_token
                    && self.child_path == active.child_path
            })
    }
}

/// One-shot authority for the release that consumed a canonical press
/// generation. XUI04 must take this value by ownership; it is intentionally
#[derive(Debug)]
pub(crate) struct AuthenticatedReplayReleaseAuthority {
    manager_namespace_ordinal: u64,
    input_transition_generation: u64,
    window_id: String,
    runtime_instance_token: String,
    child_path: Vec<u32>,
}

impl AuthenticatedReplayReleaseAuthority {
    pub(crate) fn input_transition_generation(&self) -> u64 {
        self.input_transition_generation
    }

    pub(crate) fn window_id(&self) -> &str {
        &self.window_id
    }

    pub(crate) fn runtime_instance_token(&self) -> &str {
        &self.runtime_instance_token
    }

    pub(crate) fn child_path(&self) -> &[u32] {
        &self.child_path
    }

    pub(crate) fn matches_consumed_generation(&self, manager: &ReplayOwnedWindowManager) -> bool {
        self.manager_namespace_ordinal == manager.manager_namespace_ordinal
            && self.input_transition_generation == manager.consumed_input_transition_generation
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveReplayInputTransition {
    generation: u64,
    window_id: String,
    runtime_instance_token: String,
    child_path: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StagedReplayFrameGeneration {
    generation: u64,
    viewer_clock_micros: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayScrollOutcome {
    pub window_id: String,
    pub runtime_instance_token: String,
    pub element_id: String,
    pub child_path: Vec<u32>,
    pub axis: ReplayScrollAxis,
    pub prior_offset_pixels: i32,
    pub offset_pixels: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayComposedWindowSnapshot {
    pub window_id: String,
    pub runtime_instance_token: String,
    pub source_file: String,
    pub surface_id: String,
    pub stack_ordinal: u32,
    pub modal_owner: Option<String>,
    pub owns_input: bool,
    pub focused_widget: Option<String>,
    pub screen_rect: Rect,
    pub clip_rect: Rect,
    pub geometry_source_record: String,
    pub hit_targets: Vec<ReplayHitTarget>,
    pub element_interactions: Vec<ReplayElementInteractionSnapshot>,
    pub scroll_regions: Vec<ReplayScrollRegion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayElementInteractionSnapshot {
    pub window_id: String,
    pub runtime_instance_token: String,
    pub source_file: String,
    pub surface_id: String,
    pub element_id: String,
    pub authored_name: String,
    pub child_path: Vec<u32>,
    pub global_root_sibling: bool,
    pub element_type: String,
    pub stack_ordinal: u32,
    pub paint_ordinal: Option<u32>,
    pub screen_rect: Option<Rect>,
    pub clip_rect: Option<Rect>,
    pub disposition: ReplayPointerDisposition,
    pub visible: bool,
    pub visual_enabled: Option<bool>,
    pub hovered: bool,
    pub pressed: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayWindowManagerSnapshot {
    pub transition_ordinal: u64,
    pub committed_frame_generation: u64,
    pub viewer_clock_micros: u64,
    pub dispatched_input_transition_generation: u64,
    pub pointer_position: Option<[i32; 2]>,
    pub hovered: Option<ReplayPointerTarget>,
    pub pressed: Option<ReplayPointerTarget>,
    pub tooltip: Option<ReplayTooltipOwnership>,
    pub stack: Vec<ReplayComposedWindowSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayWindowManagerRejection {
    UnknownWindow(String),
    AlreadyOpen(String),
    AlreadyClosed(String),
    InvalidModalOwner(String),
    ModalInputBlocked(String),
    InvalidTraversal(String),
    InvalidFocusTarget(String),
    InvalidTooltipOwner(String),
    MissingComposedGeometry(String),
    StaleRuntimeInstance(String),
    InvalidComposedGeometry(String),
    InvalidPointerState(String),
    InvalidScrollRegion(String),
    ScrollTargetUnavailable,
    PixelCoordinateOverflow,
    StackOrdinalOverflow,
    ManagerNamespaceOrdinalOverflow,
    RuntimeInstanceOrdinalOverflow,
    FrameGenerationOverflow,
    InputTransitionGenerationOverflow,
}

impl fmt::Display for ReplayWindowManagerRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownWindow(id) => write!(formatter, "XUI02 unknown replay window: {id}"),
            Self::AlreadyOpen(id) => write!(formatter, "XUI02 replay window already open: {id}"),
            Self::AlreadyClosed(id) => {
                write!(formatter, "XUI02 replay window already closed: {id}")
            }
            Self::InvalidModalOwner(id) => {
                write!(formatter, "XUI02 invalid replay modal owner: {id}")
            }
            Self::ModalInputBlocked(id) => write!(
                formatter,
                "XUI02 replay input blocked by active modal: {id}"
            ),
            Self::InvalidTraversal(id) => {
                write!(formatter, "XUI02 invalid replay traversal for: {id}")
            }
            Self::InvalidFocusTarget(id) => {
                write!(formatter, "XUI02 invalid replay focus target: {id}")
            }
            Self::InvalidTooltipOwner(id) => {
                write!(formatter, "XUI02 invalid replay tooltip owner: {id}")
            }
            Self::MissingComposedGeometry(id) => {
                write!(
                    formatter,
                    "XUI02 composed replay window lacks exact geometry: {id}"
                )
            }
            Self::StaleRuntimeInstance(id) => {
                write!(
                    formatter,
                    "XUI02 geometry names a stale replay window instance: {id}"
                )
            }
            Self::InvalidComposedGeometry(id) => {
                write!(
                    formatter,
                    "XUI02 invalid composed geometry for replay window: {id}"
                )
            }
            Self::InvalidPointerState(id) => {
                write!(formatter, "XUI02 invalid replay pointer state for: {id}")
            }
            Self::InvalidScrollRegion(id) => {
                write!(formatter, "XUI02 invalid replay scroll region for: {id}")
            }
            Self::ScrollTargetUnavailable => {
                write!(
                    formatter,
                    "XUI02 pointer is not over an admitted scroll region"
                )
            }
            Self::PixelCoordinateOverflow => {
                write!(formatter, "XUI02 pixel coordinate arithmetic overflow")
            }
            Self::StackOrdinalOverflow => write!(formatter, "XUI02 replay stack ordinal overflow"),
            Self::ManagerNamespaceOrdinalOverflow => {
                write!(formatter, "XUI02 replay manager namespace ordinal overflow")
            }
            Self::RuntimeInstanceOrdinalOverflow => {
                write!(formatter, "XUI02 replay runtime-instance ordinal overflow")
            }
            Self::FrameGenerationOverflow => {
                write!(formatter, "XUI02 replay frame generation overflow")
            }
            Self::InputTransitionGenerationOverflow => {
                write!(
                    formatter,
                    "XUI02 replay input-transition generation overflow"
                )
            }
        }
    }
}

impl Error for ReplayWindowManagerRejection {}

/// input, focus restores when that modal closes, and tooltip ownership cannot
#[derive(Debug, PartialEq, Eq)]
pub struct ReplayOwnedWindowManager {
    panels: BTreeMap<String, ReplayPanelState>,
    stack: Vec<String>,
    activation_contracts: BTreeMap<String, ReplayPanelActivation>,
    focus: Option<ReplayFocusTarget>,
    focus_restore: BTreeMap<String, Option<ReplayFocusTarget>>,
    tooltip: Option<ReplayTooltipOwnership>,
    persistent_instances: BTreeMap<String, String>,
    engine_instance_tokens: BTreeMap<String, u64>,
    persistent_tooltip_targets: BTreeMap<String, Vec<String>>,
    tooltip_selectors: BTreeMap<(String, String), ReplayTooltipAlternateSelection>,
    persistent_geometry: Vec<ReplayComposedPanelGeometry>,
    geometry: BTreeMap<String, ReplayComposedPanelGeometry>,
    pointer_position: Option<[i32; 2]>,
    hovered: Option<ReplayPointerTarget>,
    pressed: Option<ReplayPointerTarget>,
    transition_ordinal: u64,
    manager_namespace_ordinal: u64,
    next_runtime_instance_ordinal: u64,
    last_issued_frame_generation: u64,
    committed_frame_generation: u64,
    staged_frame_generation: Option<StagedReplayFrameGeneration>,
    canonical_widget_state_generation: u64,
    last_viewer_clock_micros: Option<u64>,
    input_transition_generation: u64,
    consumed_input_transition_generation: u64,
    dispatched_input_transition_generation: u64,
    active_input_transition: Option<ActiveReplayInputTransition>,
    canonical_widget_states: BTreeMap<(String, Vec<u32>), AuthenticatedViewerWidgetState>,
    canonical_viewer_actions: BTreeMap<(String, Vec<u32>), AuthenticatedReplayViewerAction>,
    presented_checkpoint: Option<Box<ReplayManagerAtomicCheckpoint>>,
    pending_presentation: Option<Box<ReplayManagerAtomicCheckpoint>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplayManagerAtomicCheckpoint {
    panels: BTreeMap<String, ReplayPanelState>,
    stack: Vec<String>,
    activation_contracts: BTreeMap<String, ReplayPanelActivation>,
    focus: Option<ReplayFocusTarget>,
    focus_restore: BTreeMap<String, Option<ReplayFocusTarget>>,
    tooltip: Option<ReplayTooltipOwnership>,
    persistent_instances: BTreeMap<String, String>,
    engine_instance_tokens: BTreeMap<String, u64>,
    persistent_tooltip_targets: BTreeMap<String, Vec<String>>,
    tooltip_selectors: BTreeMap<(String, String), ReplayTooltipAlternateSelection>,
    persistent_geometry: Vec<ReplayComposedPanelGeometry>,
    geometry: BTreeMap<String, ReplayComposedPanelGeometry>,
    pointer_position: Option<[i32; 2]>,
    hovered: Option<ReplayPointerTarget>,
    pressed: Option<ReplayPointerTarget>,
    transition_ordinal: u64,
    committed_frame_generation: u64,
    staged_frame_generation: Option<StagedReplayFrameGeneration>,
    canonical_widget_state_generation: u64,
    last_viewer_clock_micros: Option<u64>,
    input_transition_generation: u64,
    consumed_input_transition_generation: u64,
    dispatched_input_transition_generation: u64,
    active_input_transition: Option<ActiveReplayInputTransition>,
    canonical_widget_states: BTreeMap<(String, Vec<u32>), AuthenticatedViewerWidgetState>,
    canonical_viewer_actions: BTreeMap<(String, Vec<u32>), AuthenticatedReplayViewerAction>,
}

#[derive(Clone, Debug)]
pub struct ReplayWindowManagerInputCheckpoint {
    state: ReplayManagerAtomicCheckpoint,
}

impl ReplayWindowManagerInputCheckpoint {
    pub fn apply_capture_loss(&mut self) {
        let changed_pressed = self.state.pressed.take().is_some();
        let changed_transition = if let Some(active) = self.state.active_input_transition.take() {
            self.state.consumed_input_transition_generation = active.generation;
            true
        } else {
            false
        };
        if changed_pressed || changed_transition {
            self.state.transition_ordinal = self.state.transition_ordinal.saturating_add(1);
        }
    }
}

impl ReplayOwnedWindowManager {
    pub fn new(
        registry: &DormantWindowManagerRegistry,
    ) -> Result<Self, ReplayWindowManagerRejection> {
        let mut panels = BTreeMap::new();
        for row in &registry.windows {
            let state = ReplayPanelState {
                window_id: row.window_id.clone(),
                runtime_instance_token: None,
                open: false,
                visible: false,
                composed: false,
                stack_ordinal: None,
                modal_owner: None,
                focused_widget: None,
                keyboard_traversal: Vec::new(),
                controller_traversal: Vec::new(),
                tooltip_targets: Vec::new(),
            };
            if panels.insert(row.window_id.clone(), state).is_some() {
                return Err(ReplayWindowManagerRejection::UnknownWindow(
                    row.window_id.clone(),
                ));
            }
        }
        let manager_namespace_ordinal = NEXT_REPLAY_MANAGER_ORDINAL
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |ordinal| {
                ordinal.checked_add(1)
            })
            .map(|ordinal| ordinal + 1)
            .map_err(|_| ReplayWindowManagerRejection::ManagerNamespaceOrdinalOverflow)?;
        let mut manager = Self {
            panels,
            stack: Vec::new(),
            activation_contracts: BTreeMap::new(),
            focus: None,
            focus_restore: BTreeMap::new(),
            tooltip: None,
            persistent_instances: BTreeMap::new(),
            engine_instance_tokens: BTreeMap::new(),
            persistent_tooltip_targets: BTreeMap::new(),
            tooltip_selectors: BTreeMap::new(),
            persistent_geometry: Vec::new(),
            geometry: BTreeMap::new(),
            pointer_position: None,
            hovered: None,
            pressed: None,
            transition_ordinal: 0,
            manager_namespace_ordinal,
            next_runtime_instance_ordinal: 0,
            last_issued_frame_generation: 0,
            committed_frame_generation: 0,
            staged_frame_generation: None,
            canonical_widget_state_generation: 0,
            last_viewer_clock_micros: None,
            input_transition_generation: 0,
            consumed_input_transition_generation: 0,
            dispatched_input_transition_generation: 0,
            active_input_transition: None,
            canonical_widget_states: BTreeMap::new(),
            canonical_viewer_actions: BTreeMap::new(),
            presented_checkpoint: None,
            pending_presentation: None,
        };
        manager.presented_checkpoint = Some(Box::new(manager.atomic_checkpoint()));
        Ok(manager)
    }

    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }

    pub fn input_checkpoint(
        &self,
    ) -> Result<ReplayWindowManagerInputCheckpoint, ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        Ok(ReplayWindowManagerInputCheckpoint {
            state: self.atomic_checkpoint(),
        })
    }

    pub fn restore_input_checkpoint(
        &mut self,
        checkpoint: ReplayWindowManagerInputCheckpoint,
    ) -> Result<(), ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        self.restore_atomic_checkpoint(checkpoint.state);
        Ok(())
    }

    /// target invalidation. Open lifetimes and viewer-owned bindings survive,
    pub fn invalidate_composed_frame(&mut self) {
        if let Some(checkpoint) = self.pending_presentation.take() {
            self.restore_atomic_checkpoint(*checkpoint);
        }
        self.persistent_geometry.clear();
        self.geometry.clear();
        self.hovered = None;
        self.pressed = None;
        self.invalidate_active_input_transition();
        self.committed_frame_generation = 0;
        self.canonical_widget_state_generation = 0;
        self.canonical_widget_states.clear();
        self.canonical_viewer_actions.clear();
        self.tooltip_selectors.clear();
        self.bump_transition();
        self.presented_checkpoint = Some(Box::new(self.atomic_checkpoint()));
    }

    pub fn transition_ordinal(&self) -> u64 {
        self.transition_ordinal
    }

    fn materialized_pointer_action_identity(
        &self,
        target: Option<&ReplayPointerTarget>,
    ) -> Option<MaterializedPointerActionIdentity> {
        let target = target.filter(|target| target.child_path.first() == Some(&u32::MAX))?;
        let action = self
            .canonical_viewer_actions
            .get(&(target.window_id.clone(), target.child_path.clone()))
            .filter(|action| {
                action.window_id == target.window_id
                    && action.runtime_instance_token == target.runtime_instance_token
                    && action.frame_generation == self.committed_frame_generation
                    && action.rendered_child_path == target.child_path
            })?;
        Some(MaterializedPointerActionIdentity {
            window_id: target.window_id.clone(),
            runtime_instance_token: target.runtime_instance_token.clone(),
            action: action.action.clone(),
        })
    }

    fn materialized_pointer_lifecycle(&self) -> MaterializedPointerLifecycle {
        MaterializedPointerLifecycle {
            hovered: self.materialized_pointer_action_identity(self.hovered.as_ref()),
            pressed: self.materialized_pointer_action_identity(self.pressed.as_ref()),
        }
    }

    fn atomic_checkpoint(&self) -> ReplayManagerAtomicCheckpoint {
        ReplayManagerAtomicCheckpoint {
            panels: self.panels.clone(),
            stack: self.stack.clone(),
            activation_contracts: self.activation_contracts.clone(),
            focus: self.focus.clone(),
            focus_restore: self.focus_restore.clone(),
            tooltip: self.tooltip.clone(),
            persistent_instances: self.persistent_instances.clone(),
            engine_instance_tokens: self.engine_instance_tokens.clone(),
            persistent_tooltip_targets: self.persistent_tooltip_targets.clone(),
            tooltip_selectors: self.tooltip_selectors.clone(),
            persistent_geometry: self.persistent_geometry.clone(),
            geometry: self.geometry.clone(),
            pointer_position: self.pointer_position,
            hovered: self.hovered.clone(),
            pressed: self.pressed.clone(),
            transition_ordinal: self.transition_ordinal,
            committed_frame_generation: self.committed_frame_generation,
            staged_frame_generation: self.staged_frame_generation.clone(),
            canonical_widget_state_generation: self.canonical_widget_state_generation,
            last_viewer_clock_micros: self.last_viewer_clock_micros,
            input_transition_generation: self.input_transition_generation,
            consumed_input_transition_generation: self.consumed_input_transition_generation,
            dispatched_input_transition_generation: self.dispatched_input_transition_generation,
            active_input_transition: self.active_input_transition.clone(),
            canonical_widget_states: self.canonical_widget_states.clone(),
            canonical_viewer_actions: self.canonical_viewer_actions.clone(),
        }
    }

    fn restore_atomic_checkpoint(&mut self, checkpoint: ReplayManagerAtomicCheckpoint) {
        self.panels = checkpoint.panels;
        self.stack = checkpoint.stack;
        self.activation_contracts = checkpoint.activation_contracts;
        self.focus = checkpoint.focus;
        self.focus_restore = checkpoint.focus_restore;
        self.tooltip = checkpoint.tooltip;
        self.persistent_instances = checkpoint.persistent_instances;
        self.engine_instance_tokens = checkpoint.engine_instance_tokens;
        self.persistent_tooltip_targets = checkpoint.persistent_tooltip_targets;
        self.tooltip_selectors = checkpoint.tooltip_selectors;
        self.persistent_geometry = checkpoint.persistent_geometry;
        self.geometry = checkpoint.geometry;
        self.pointer_position = checkpoint.pointer_position;
        self.hovered = checkpoint.hovered;
        self.pressed = checkpoint.pressed;
        self.transition_ordinal = checkpoint.transition_ordinal;
        self.committed_frame_generation = checkpoint.committed_frame_generation;
        self.staged_frame_generation = checkpoint.staged_frame_generation;
        self.canonical_widget_state_generation = checkpoint.canonical_widget_state_generation;
        self.last_viewer_clock_micros = checkpoint.last_viewer_clock_micros;
        self.input_transition_generation = checkpoint.input_transition_generation;
        self.consumed_input_transition_generation = checkpoint.consumed_input_transition_generation;
        self.dispatched_input_transition_generation =
            checkpoint.dispatched_input_transition_generation;
        self.active_input_transition = checkpoint.active_input_transition;
        self.canonical_widget_states = checkpoint.canonical_widget_states;
        self.canonical_viewer_actions = checkpoint.canonical_viewer_actions;
    }

    fn stage_frame_authority(
        &mut self,
        viewer_clock_micros: u64,
    ) -> Result<AuthenticatedReplayFrameAuthority, ReplayWindowManagerRejection> {
        if self.staged_frame_generation.is_some() {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "frame-generation-already-staged".to_string(),
            ));
        }
        let frame_generation = self
            .last_issued_frame_generation
            .checked_add(1)
            .ok_or(ReplayWindowManagerRejection::FrameGenerationOverflow)?;
        if self
            .last_viewer_clock_micros
            .is_some_and(|last| viewer_clock_micros < last)
        {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "viewer-clock-rollback".to_string(),
            ));
        }
        self.staged_frame_generation = Some(StagedReplayFrameGeneration {
            generation: frame_generation,
            viewer_clock_micros,
        });
        self.last_issued_frame_generation = frame_generation;
        Ok(AuthenticatedReplayFrameAuthority {
            manager_namespace_ordinal: self.manager_namespace_ordinal,
            frame_generation,
            viewer_clock_micros,
        })
    }

    fn commit_staged_canonical_states(
        &mut self,
        staged: StagedReplayCanonicalStates,
    ) -> Result<(), ReplayWindowManagerRejection> {
        let StagedReplayCanonicalStates {
            frame_authority: authority,
            states: canonical_widget_states,
            viewer_actions: canonical_viewer_actions,
        } = staged;
        if !authority.remains_staged_for(self) {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "stale-frame-generation-authority".to_string(),
            ));
        }
        if canonical_widget_states
            .values()
            .any(|state| state.frame_generation() != authority.frame_generation)
            || canonical_viewer_actions
                .iter()
                .any(|((window_id, child_path), action)| {
                    action.frame_generation != authority.frame_generation
                        || action.input_transition_generation != 0
                        || action.window_id != *window_id
                        || action.rendered_child_path != *child_path
                })
        {
            self.staged_frame_generation = None;
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "canonical-frame-generation-mismatch".to_string(),
            ));
        }
        self.committed_frame_generation = authority.frame_generation;
        self.staged_frame_generation = None;
        self.canonical_widget_states = canonical_widget_states;
        self.canonical_viewer_actions = canonical_viewer_actions;
        self.canonical_widget_state_generation = authority.frame_generation;
        self.last_viewer_clock_micros = Some(authority.viewer_clock_micros);
        self.transition_ordinal = self.transition_ordinal.saturating_add(1);
        Ok(())
    }

    fn abort_frame_authority(
        &mut self,
        authority: AuthenticatedReplayFrameAuthority,
    ) -> Result<(), ReplayWindowManagerRejection> {
        if !authority.remains_staged_for(self) {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "stale-frame-generation-authority".to_string(),
            ));
        }
        self.staged_frame_generation = None;
        Ok(())
    }

    pub fn compose_authenticated_panel_frame(
        &mut self,
        final_framebuffer: &mut RgbaImage,
        index: &AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        inputs: &[ReplayAuthenticatedPanelFrameInput<'_>],
        viewer_clock_micros: u64,
    ) -> Result<Vec<ReplayAuthenticatedComposedPanel>, ReplayWindowManagerRejection> {
        if self.pending_presentation.is_some() {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "authenticated-frame-awaits-presentation".to_string(),
            ));
        }
        if final_framebuffer.width == 0 || final_framebuffer.height == 0 {
            return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                "empty-authenticated-framebuffer".to_string(),
            ));
        }
        let checkpoint = self.atomic_checkpoint();
        let viewport_width = final_framebuffer.width;
        let viewport_height = final_framebuffer.height;
        let result = self.compose_authenticated_panel_frame_inner(
            index,
            admission,
            inputs,
            viewer_clock_micros,
            viewport_width,
            viewport_height,
        );
        match result {
            Ok((composed, staged_framebuffer)) => {
                *final_framebuffer = staged_framebuffer;
                self.pending_presentation = self.presented_checkpoint.clone();
                Ok(composed)
            }
            Err(error) => {
                self.restore_atomic_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub fn confirm_presented_frame(&mut self) -> Result<(), ReplayWindowManagerRejection> {
        if self.pending_presentation.take().is_none() {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "no-authenticated-frame-awaits-presentation".to_string(),
            ));
        }
        self.presented_checkpoint = Some(Box::new(self.atomic_checkpoint()));
        Ok(())
    }

    pub fn reject_unpresented_frame(&mut self) -> Result<(), ReplayWindowManagerRejection> {
        let checkpoint = self.pending_presentation.take().ok_or_else(|| {
            ReplayWindowManagerRejection::InvalidPointerState(
                "no-authenticated-frame-awaits-presentation".to_string(),
            )
        })?;
        self.restore_atomic_checkpoint(*checkpoint);
        Ok(())
    }

    fn compose_authenticated_panel_frame_inner(
        &mut self,
        index: &AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        inputs: &[ReplayAuthenticatedPanelFrameInput<'_>],
        viewer_clock_micros: u64,
        viewport_width: usize,
        viewport_height: usize,
    ) -> Result<(Vec<ReplayAuthenticatedComposedPanel>, RgbaImage), ReplayWindowManagerRejection>
    {
        if self.staged_frame_generation.is_some() {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "frame-generation-already-staged".to_string(),
            ));
        }
        let materialized_pointer_lifecycle = self.materialized_pointer_lifecycle();
        let desired_persistent_order = inputs
            .iter()
            .filter(|input| input.layer == ReplayAuthenticatedPanelLayer::PersistentHud)
            .map(|input| input.panel.window_id().to_string())
            .collect::<Vec<_>>();
        let desired_persistent_ids = desired_persistent_order
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let desired_viewer_ids = inputs
            .iter()
            .filter(|input| input.layer == ReplayAuthenticatedPanelLayer::ViewerPanel)
            .map(|input| input.panel.window_id().to_string())
            .collect::<BTreeSet<_>>();
        if desired_persistent_ids.len() != desired_persistent_order.len() {
            return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                "duplicate-persistent-panel-input".to_string(),
            ));
        }
        let mut by_window = BTreeMap::new();
        for input in inputs {
            let surface = index
                .validate_panel_token(admission, input.panel)
                .map_err(|_| {
                    ReplayWindowManagerRejection::InvalidComposedGeometry(
                        input.panel.window_id().to_string(),
                    )
                })?;
            if input.surface.index.index_identity_sha256 != index.index_identity_sha256
                || &input.surface.admission != admission
                || surface.root_window_id != input.panel.window_id()
                || input.viewport_width != viewport_width
                || input.viewport_height != viewport_height
                || by_window
                    .insert(input.panel.window_id().to_string(), input)
                    .is_some()
            {
                return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                    input.panel.window_id().to_string(),
                ));
            }
            validate_authenticated_frame_bindings(surface, input)?;
        }

        let closing = self
            .stack
            .iter()
            .rev()
            .filter(|window_id| !desired_viewer_ids.contains(*window_id))
            .cloned()
            .collect::<Vec<_>>();
        for window_id in closing {
            self.close(&window_id)?;
        }

        for input in inputs {
            let window_id = input.panel.window_id();
            match input.layer {
                ReplayAuthenticatedPanelLayer::PersistentHud => {
                    if input.activation.is_some()
                        || self.panels.get(window_id).is_some_and(|panel| panel.open)
                    {
                        return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                            window_id.to_string(),
                        ));
                    }
                    if !self.persistent_instances.contains_key(window_id) {
                        let token = self.issue_runtime_instance_token(window_id)?;
                        self.persistent_instances
                            .insert(window_id.to_string(), token);
                    }
                }
                ReplayAuthenticatedPanelLayer::ViewerPanel => {
                    let activation = input.activation.ok_or_else(|| {
                        ReplayWindowManagerRejection::InvalidComposedGeometry(window_id.to_string())
                    })?;
                    if activation.window_id != window_id
                        || self.persistent_instances.contains_key(window_id)
                    {
                        return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                            window_id.to_string(),
                        ));
                    }
                    if self.panels.get(window_id).is_some_and(|panel| !panel.open) {
                        self.open(activation.clone())?;
                    } else if self.activation_contracts.get(window_id) != Some(activation) {
                        return Err(ReplayWindowManagerRejection::InvalidTraversal(
                            window_id.to_string(),
                        ));
                    }
                }
            }
        }

        let expected_ids = desired_persistent_ids
            .iter()
            .cloned()
            .chain(self.stack.iter().cloned())
            .collect::<BTreeSet<_>>();
        if expected_ids.len() != by_window.len()
            || expected_ids
                .iter()
                .any(|window_id| !by_window.contains_key(window_id))
        {
            return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                "incomplete-authenticated-panel-batch".to_string(),
            ));
        }
        if desired_persistent_ids.iter().any(|window_id| {
            by_window
                .get(window_id.as_str())
                .is_none_or(|input| input.layer != ReplayAuthenticatedPanelLayer::PersistentHud)
        }) || self.stack.iter().any(|window_id| {
            by_window
                .get(window_id.as_str())
                .is_none_or(|input| input.layer != ReplayAuthenticatedPanelLayer::ViewerPanel)
        }) {
            return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                "authenticated-panel-layer-mismatch".to_string(),
            ));
        }

        let frame_authority = self.stage_frame_authority(viewer_clock_micros)?;
        if !desired_persistent_order.is_empty() {
            let persistent_surfaces = desired_persistent_order
                .iter()
                .map(|window_id| {
                    by_window
                        .get(window_id.as_str())
                        .expect("persistent input census checked above")
                        .surface
                        .clone()
                })
                .collect::<Vec<_>>();
            let persistent_panels = authenticate_persistent_hud_surfaces(&persistent_surfaces)
                .map_err(|_| {
                    ReplayWindowManagerRejection::InvalidComposedGeometry(
                        "persistent-hud-controller-surface-set".to_string(),
                    )
                })?;
            let slots = crate::authenticated_semantic_provider::seal_exact_a729_hud_semantic_slots(
                &persistent_panels,
            )
            .map_err(|_| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(
                    "persistent-hud-controller-slot-index".to_string(),
                )
            })?;
            let topbar_lease = self
                .authenticated_runtime_instance(persistent_panels.panels()[0].window_id())
                .ok_or_else(|| {
                    ReplayWindowManagerRejection::StaleRuntimeInstance(
                        persistent_panels.panels()[0].window_id().to_string(),
                    )
                })?;
            let mapmodes_lease = self
                .authenticated_runtime_instance(persistent_panels.panels()[2].window_id())
                .ok_or_else(|| {
                    ReplayWindowManagerRejection::StaleRuntimeInstance(
                        persistent_panels.panels()[2].window_id().to_string(),
                    )
                })?;
            let static_actions =
                crate::authenticated_semantic_provider::seal_exact_a729_static_hud_actions(
                    self,
                    &frame_authority,
                    &persistent_panels,
                    &slots,
                    &topbar_lease,
                    &mapmodes_lease,
                )
                .map_err(|_| {
                    ReplayWindowManagerRejection::InvalidPointerState(
                        "persistent-hud-static-action-census".to_string(),
                    )
                })?;
            let supplied_action_count = desired_persistent_order
                .iter()
                .map(|window_id| {
                    by_window
                        .get(window_id.as_str())
                        .expect("persistent input census checked above")
                        .viewer_actions
                        .len()
                })
                .sum::<usize>();
            if supplied_action_count == 0
                || supplied_action_count > static_actions.len()
                || static_actions.identity_sha256().trim().is_empty()
            {
                return Err(ReplayWindowManagerRejection::InvalidPointerState(
                    "persistent-hud-static-action-binding-drift".to_string(),
                ));
            }
        }
        let mut preflights = BTreeMap::new();
        let mut compose_requests = BTreeMap::new();
        let mut semantic_censuses = BTreeMap::new();
        for window_id in &expected_ids {
            let input = by_window
                .get(window_id.as_str())
                .expect("complete input census checked above");
            let lease = self
                .authenticated_runtime_instance(window_id)
                .ok_or_else(|| {
                    ReplayWindowManagerRejection::StaleRuntimeInstance((*window_id).to_string())
                })?;
            let request = AuthenticatedPanelComposeRequest::seal(
                self,
                &frame_authority,
                input.panel,
                &lease,
                viewport_width,
                viewport_height,
            )
            .map_err(|error| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(format!(
                    "{window_id}:compose-request:{error:?}"
                ))
            })?;
            let semantic_elements = authenticated_semantic_elements(input)?;
            let semantics = seal_authenticated_panel_semantics(
                self,
                &frame_authority,
                index,
                admission,
                input.panel,
                &lease,
                semantic_elements,
            )
            .map_err(|error| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(format!(
                    "{window_id}:semantic-census:{error:?}"
                ))
            })?;
            let empty_legacy_registry = SpriteRegistry::new();
            let mut exact_compositor =
                Compositor::new(&input.surface.assets, &empty_legacy_registry);
            let preflight = exact_compositor
                .preflight_authenticated_layout(
                    self,
                    &frame_authority,
                    &request,
                    index,
                    admission,
                    input.panel,
                    &lease,
                    &semantics,
                )
                .map_err(|error| {
                    ReplayWindowManagerRejection::InvalidComposedGeometry(format!(
                        "{window_id}:preflight:{error:?}"
                    ))
                })?;
            let geometry = geometry_from_authenticated_preflight(
                input,
                &lease,
                &preflight,
                frame_authority.frame_generation(),
            )?;
            validate_geometry(&geometry)?;
            compose_requests.insert(window_id.clone(), request);
            semantic_censuses.insert(window_id.clone(), semantics);
            preflights.insert(window_id.clone(), (preflight, geometry));
        }

        let mut staged_geometry = BTreeMap::new();
        let mut staged_persistent = Vec::with_capacity(desired_persistent_order.len());
        for window_id in &desired_persistent_order {
            let (_, geometry) = preflights.get(window_id).ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(window_id.clone())
            })?;
            staged_persistent.push(geometry.clone());
        }
        for window_id in &self.stack {
            if desired_persistent_ids.contains(window_id) {
                continue;
            }
            let (_, geometry) = preflights.get(window_id).ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(window_id.clone())
            })?;
            staged_geometry.insert(window_id.clone(), geometry.clone());
        }
        let retired_persistent_tokens = self
            .persistent_instances
            .iter()
            .filter(|(window_id, _)| !desired_persistent_ids.contains(*window_id))
            .map(|(_, token)| token.clone())
            .collect::<Vec<_>>();
        self.persistent_instances
            .retain(|window_id, _| desired_persistent_ids.contains(window_id));
        for token in retired_persistent_tokens {
            self.engine_instance_tokens.remove(&token);
        }
        self.persistent_tooltip_targets = desired_persistent_order
            .iter()
            .map(|window_id| {
                let input = by_window
                    .get(window_id.as_str())
                    .expect("desired persistent root belongs to complete input census");
                (window_id.clone(), input.tooltip_targets.to_vec())
            })
            .collect();
        self.tooltip_selectors = inputs
            .iter()
            .flat_map(|input| {
                input
                    .tooltip_selectors
                    .iter()
                    .map(|(element_id, selector)| {
                        (
                            (input.panel.window_id().to_string(), element_id.clone()),
                            *selector,
                        )
                    })
            })
            .collect();
        self.persistent_geometry = staged_persistent;
        self.geometry = staged_geometry;
        self.reconcile_current_activation_state()?;
        self.reconcile_pointer_capture_with_current_geometry()?;
        self.hovered = self
            .pointer_position
            .map(|[x, y]| self.hit_test(x, y))
            .transpose()?
            .flatten();

        let panels = by_window
            .iter()
            .map(|(window_id, input)| (window_id.clone(), (*input.panel).clone()))
            .collect::<BTreeMap<_, _>>();
        let staged_viewer_actions = authenticate_viewer_actions(
            self,
            index,
            admission,
            &by_window,
            &preflights,
            &frame_authority,
        )?;
        let mut staged_states = self
            .stage_current_widget_states(
                index,
                admission,
                &panels,
                staged_viewer_actions,
                frame_authority,
            )
            .map_err(|_| {
                ReplayWindowManagerRejection::InvalidPointerState(
                    "canonical-widget-state-admission".to_string(),
                )
            })?;

        let compose_order = desired_persistent_order
            .into_iter()
            .chain(self.stack.iter().cloned())
            .collect::<Vec<_>>();
        struct PreparedBatchPanel<'a> {
            window_id: String,
            lease: AuthenticatedReplayPanelLease<'a>,
            widget_states: BTreeMap<Vec<u32>, AuthenticatedViewerWidgetState>,
        }
        let prepared = compose_order
            .iter()
            .map(|window_id| {
                let lease = self
                    .authenticated_runtime_instance(window_id)
                    .ok_or_else(|| {
                        ReplayWindowManagerRejection::StaleRuntimeInstance(window_id.to_string())
                    })?;
                let widget_states = staged_states
                    .states()
                    .iter()
                    .filter(|((owner, _), _)| owner == window_id)
                    .map(|((_, child_path), state)| (child_path.clone(), state.clone()))
                    .collect::<BTreeMap<_, _>>();
                Ok(PreparedBatchPanel {
                    window_id: window_id.clone(),
                    lease,
                    widget_states,
                })
            })
            .collect::<Result<Vec<_>, ReplayWindowManagerRejection>>()?;
        let mut materialization_plans = BTreeMap::new();
        for prepared_panel in &prepared {
            let input = by_window
                .get(prepared_panel.window_id.as_str())
                .expect("complete input census checked above");
            let (preflight, _) = preflights
                .get(prepared_panel.window_id.as_str())
                .expect("complete preflight census checked above");
            let plan = if let Some(theatre) = input.theatre_capability {
                if theatre.owner_window_id() != prepared_panel.window_id {
                    return Err(theatre_materialization_rejection(&prepared_panel.window_id));
                }
                build_exact_a729_theatre_materialization_plan(
                    self,
                    staged_states.frame_authority(),
                    index,
                    admission,
                    input.surface,
                    input.panel,
                    &prepared_panel.lease,
                    preflight,
                    theatre,
                    &materialized_pointer_lifecycle,
                )?
            } else if let Some(production_lines) = input.production_lines_capability {
                build_exact_a729_production_lines_materialization_plan(
                    self,
                    staged_states.frame_authority(),
                    index,
                    admission,
                    input.panel,
                    &prepared_panel.lease,
                    preflight,
                    production_lines,
                    &materialized_pointer_lifecycle,
                )?
            } else if let Some(frontend) = input.frontend_collections_capability {
                build_xui26_frontend_materialization_plan(
                    self,
                    staged_states.frame_authority(),
                    index,
                    admission,
                    input.panel,
                    &prepared_panel.lease,
                    preflight,
                    frontend,
                    &materialized_pointer_lifecycle,
                )?
            } else {
                continue;
            };
            materialization_plans.insert(prepared_panel.window_id.clone(), plan);
        }
        let batch_inputs = prepared
            .iter()
            .map(|prepared| {
                let input = by_window
                    .get(prepared.window_id.as_str())
                    .expect("complete input census checked above");
                let (preflight, _) = preflights
                    .get(prepared.window_id.as_str())
                    .expect("complete preflight census checked above");
                let request = compose_requests
                    .get(prepared.window_id.as_str())
                    .expect("complete compose-request census checked above");
                let semantics = semantic_censuses
                    .get(prepared.window_id.as_str())
                    .expect("complete semantic census checked above");
                AuthenticatedPanelBatchInput::new(
                    request,
                    input.surface,
                    &prepared.lease,
                    preflight,
                    &prepared.widget_states,
                    semantics,
                    materialization_plans.get(prepared.window_id.as_str()),
                )
            })
            .collect::<Vec<_>>();
        let batch = compose_authenticated_surface_batch(
            self,
            staged_states.frame_authority(),
            viewport_width,
            viewport_height,
            &batch_inputs,
        )
        .map_err(|error| {
            ReplayWindowManagerRejection::InvalidComposedGeometry(format!(
                "authenticated-ui-batch:{error:?}"
            ))
        })?;
        let (staged_framebuffer, reports) = batch.into_parts();
        if reports.len() != prepared.len() {
            return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                "authenticated-ui-batch-report-census".to_string(),
            ));
        }
        let mut final_geometry = BTreeMap::new();
        let mut materialized_actions = BTreeMap::new();
        for (report, prepared_panel) in reports.iter().zip(&prepared) {
            let input = by_window
                .get(prepared_panel.window_id.as_str())
                .expect("complete input census checked above");
            let plan = materialization_plans.get(prepared_panel.window_id.as_str());
            let actions = match plan {
                Some(plan) => seal_materialized_viewer_actions(
                    &prepared_panel.window_id,
                    &prepared_panel.lease,
                    report,
                    plan,
                    staged_states.frame_authority().frame_generation(),
                )?,
                None => BTreeMap::new(),
            };
            let geometry = geometry_from_authenticated_compose_report(
                input,
                &prepared_panel.lease,
                report,
                plan,
                &actions,
            )?;
            validate_geometry(&geometry)?;
            if final_geometry
                .insert(prepared_panel.window_id.clone(), geometry)
                .is_some()
            {
                return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                    prepared_panel.window_id.clone(),
                ));
            }
            for (key, action) in actions {
                if staged_states.viewer_actions.contains_key(&key)
                    || materialized_actions.insert(key, action).is_some()
                {
                    return Err(ReplayWindowManagerRejection::InvalidPointerState(
                        prepared_panel.window_id.clone(),
                    ));
                }
            }
        }
        staged_states.viewer_actions.extend(materialized_actions);
        let mut final_persistent_geometry = Vec::new();
        let mut final_viewer_geometry = BTreeMap::new();
        for window_id in &compose_order {
            let geometry = final_geometry.remove(window_id).ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(window_id.clone())
            })?;
            let input = by_window
                .get(window_id.as_str())
                .expect("complete input census checked above");
            match input.layer {
                ReplayAuthenticatedPanelLayer::PersistentHud => {
                    final_persistent_geometry.push(geometry);
                }
                ReplayAuthenticatedPanelLayer::ViewerPanel => {
                    final_viewer_geometry.insert(window_id.clone(), geometry);
                }
            }
        }
        if !final_geometry.is_empty() {
            return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                "authenticated-ui-final-geometry-census".to_string(),
            ));
        }
        let mut effective_scroll_offsets = final_persistent_geometry
            .iter()
            .chain(final_viewer_geometry.values())
            .map(|geometry| {
                (
                    geometry.window_id.clone(),
                    geometry
                        .scroll_regions
                        .iter()
                        .map(|region| (region.element_id.clone(), region.offset_pixels))
                        .collect::<BTreeMap<_, _>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let composed = reports
            .into_iter()
            .zip(&prepared)
            .map(|(report, prepared)| {
                if report.window_id() != prepared.window_id.as_str()
                    || report.runtime_instance_token() != prepared.lease.runtime_instance_token()
                    || report.frame_generation()
                        != staged_states.frame_authority().frame_generation()
                    || report.report().drawn_elements == 0
                {
                    return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                        prepared.window_id.clone(),
                    ));
                }
                Ok(ReplayAuthenticatedComposedPanel {
                    window_id: prepared.window_id.clone(),
                    runtime_instance_token: prepared.lease.runtime_instance_token().to_string(),
                    frame_generation: report.frame_generation(),
                    drawn_elements: report.report().drawn_elements,
                    effective_scroll_offsets: effective_scroll_offsets
                        .remove(&prepared.window_id)
                        .ok_or_else(|| {
                            ReplayWindowManagerRejection::InvalidScrollRegion(
                                prepared.window_id.clone(),
                            )
                        })?,
                })
            })
            .collect::<Result<Vec<_>, ReplayWindowManagerRejection>>()?;
        if !effective_scroll_offsets.is_empty() {
            return Err(ReplayWindowManagerRejection::InvalidScrollRegion(
                "authenticated-ui-scroll-offset-census".to_string(),
            ));
        }
        drop(batch_inputs);
        drop(prepared);
        self.persistent_geometry = final_persistent_geometry;
        self.geometry = final_viewer_geometry;
        self.reconcile_current_activation_state()?;
        self.reconcile_pointer_capture_with_current_geometry()?;
        self.hovered = self
            .pointer_position
            .map(|[x, y]| self.hit_test(x, y))
            .transpose()?
            .flatten();
        self.commit_staged_canonical_states(staged_states)?;
        Ok((composed, staged_framebuffer))
    }

    fn issue_press_authority_for_current_edge(
        &mut self,
    ) -> Result<AuthenticatedReplayPressAuthority, ReplayWindowManagerRejection> {
        if self.active_input_transition.is_some() {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "input-transition-already-active".to_string(),
            ));
        }
        let pressed = self.pressed.as_ref().ok_or_else(|| {
            ReplayWindowManagerRejection::InvalidPointerState(
                "canonical-press-has-no-interactive-target".to_string(),
            )
        })?;
        if self.current_runtime_instance_token(&pressed.window_id)
            != Some(pressed.runtime_instance_token.as_str())
        {
            return Err(ReplayWindowManagerRejection::StaleRuntimeInstance(
                pressed.window_id.clone(),
            ));
        }
        let generation = self
            .input_transition_generation
            .checked_add(1)
            .ok_or(ReplayWindowManagerRejection::InputTransitionGenerationOverflow)?;
        let active = ActiveReplayInputTransition {
            generation,
            window_id: pressed.window_id.clone(),
            runtime_instance_token: pressed.runtime_instance_token.clone(),
            child_path: pressed.child_path.clone(),
        };
        let authority = AuthenticatedReplayPressAuthority {
            manager_namespace_ordinal: self.manager_namespace_ordinal,
            input_transition_generation: generation,
            window_id: active.window_id.clone(),
            runtime_instance_token: active.runtime_instance_token.clone(),
            child_path: active.child_path.clone(),
        };
        self.input_transition_generation = generation;
        self.active_input_transition = Some(active);
        Ok(authority)
    }

    fn consume_release_authority(
        &mut self,
        released_over: Option<&ReplayPointerTarget>,
    ) -> Result<Option<AuthenticatedReplayReleaseAuthority>, ReplayWindowManagerRejection> {
        let Some(active) = self.active_input_transition.take() else {
            return Ok(None);
        };
        self.consumed_input_transition_generation = active.generation;
        let Some(released_over) = released_over else {
            return Ok(None);
        };
        if active.window_id != released_over.window_id
            || active.runtime_instance_token != released_over.runtime_instance_token
            || active.child_path != released_over.child_path
        {
            return Ok(None);
        }
        Ok(Some(AuthenticatedReplayReleaseAuthority {
            manager_namespace_ordinal: self.manager_namespace_ordinal,
            input_transition_generation: active.generation,
            window_id: active.window_id,
            runtime_instance_token: active.runtime_instance_token,
            child_path: active.child_path,
        }))
    }

    pub fn panel(&self, window_id: &str) -> Option<&ReplayPanelState> {
        self.panels.get(window_id)
    }

    pub fn composed_stack(&self) -> &[String] {
        &self.stack
    }

    pub fn focus(&self) -> Option<&ReplayFocusTarget> {
        self.focus.as_ref()
    }

    pub fn tooltip(&self) -> Option<&ReplayTooltipOwnership> {
        self.tooltip.as_ref()
    }

    pub fn pointer_position(&self) -> Option<(i32, i32)> {
        self.pointer_position.map(|point| (point[0], point[1]))
    }

    pub fn authenticated_tooltip_owner_at_cursor(
        &self,
    ) -> Result<Option<AuthenticatedReplayTooltipOwner>, ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        self.authenticated_tooltip_owner_from_current_state()
    }

    pub fn authenticated_pending_tooltip_owner_at_cursor(
        &self,
    ) -> Result<Option<AuthenticatedReplayTooltipOwner>, ReplayWindowManagerRejection> {
        if self.pending_presentation.is_none() {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "no-authenticated-frame-awaits-tooltip-composition".to_string(),
            ));
        }
        self.authenticated_tooltip_owner_from_current_state()
    }

    fn authenticated_tooltip_owner_from_current_state(
        &self,
    ) -> Result<Option<AuthenticatedReplayTooltipOwner>, ReplayWindowManagerRejection> {
        let Some(target) = self.hovered.as_ref() else {
            return Ok(None);
        };
        if self.committed_frame_generation == 0
            || self.current_runtime_instance_token(&target.window_id)
                != Some(target.runtime_instance_token.as_str())
            || !(self
                .panels
                .get(&target.window_id)
                .is_some_and(|panel| panel.tooltip_targets.contains(&target.element_id))
                || self
                    .persistent_tooltip_targets
                    .get(&target.window_id)
                    .is_some_and(|targets| targets.contains(&target.element_id)))
        {
            return Ok(None);
        }
        let engine_instance_token = self
            .engine_instance_tokens
            .get(&target.runtime_instance_token)
            .copied()
            .filter(|token| *token != 0)
            .ok_or_else(|| {
                ReplayWindowManagerRejection::StaleRuntimeInstance(target.window_id.clone())
            })?;
        let alternate_selection = self
            .tooltip_selectors
            .get(&(target.window_id.clone(), target.element_id.clone()))
            .copied()
            .ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidTooltipOwner(target.window_id.clone())
            })?;
        Ok(Some(AuthenticatedReplayTooltipOwner {
            window_id: target.window_id.clone(),
            runtime_instance_token: target.runtime_instance_token.clone(),
            engine_instance_token,
            source_file: target.source_file.clone(),
            surface_id: target.surface_id.clone(),
            element_id: target.element_id.clone(),
            authored_name: target.authored_name.clone(),
            child_path: target.child_path.clone(),
            element_type: target.element_type.clone(),
            screen_rect: target.screen_rect,
            clip_rect: target.clip_rect,
            stack_ordinal: target.stack_ordinal,
            paint_ordinal: target.paint_ordinal,
            frame_generation: self.committed_frame_generation,
            alternate_selection,
        }))
    }

    pub fn input_window_id(&self) -> Option<&str> {
        self.stack
            .iter()
            .rev()
            .find(|id| {
                self.panels
                    .get(*id)
                    .is_some_and(|row| row.modal_owner.is_some())
            })
            .or_else(|| self.stack.last())
            .map(String::as_str)
    }

    fn issue_runtime_instance_token(
        &mut self,
        window_id: &str,
    ) -> Result<String, ReplayWindowManagerRejection> {
        if !self.panels.contains_key(window_id) {
            return Err(ReplayWindowManagerRejection::UnknownWindow(
                window_id.to_string(),
            ));
        }
        self.next_runtime_instance_ordinal = self
            .next_runtime_instance_ordinal
            .checked_add(1)
            .ok_or(ReplayWindowManagerRejection::RuntimeInstanceOrdinalOverflow)?;
        let runtime_instance_token = format!(
            "replay-window/{:016x}/{:016x}/{}",
            self.manager_namespace_ordinal, self.next_runtime_instance_ordinal, window_id
        );
        let engine_instance_token = NEXT_REPLAY_ENGINE_INSTANCE_TOKEN
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |ordinal| {
                ordinal.checked_add(1)
            })
            .map(|ordinal| ordinal + 1)
            .map_err(|_| ReplayWindowManagerRejection::RuntimeInstanceOrdinalOverflow)?;
        self.engine_instance_tokens
            .insert(runtime_instance_token.clone(), engine_instance_token);
        Ok(runtime_instance_token)
    }

    pub fn open(
        &mut self,
        activation: ReplayPanelActivation,
    ) -> Result<(), ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        let activation_contract = activation.clone();
        let Some(existing) = self.panels.get(&activation.window_id) else {
            return Err(ReplayWindowManagerRejection::UnknownWindow(
                activation.window_id,
            ));
        };
        if existing.open {
            return Err(ReplayWindowManagerRejection::AlreadyOpen(
                activation.window_id,
            ));
        }
        if self
            .persistent_instances
            .contains_key(&activation.window_id)
        {
            return Err(ReplayWindowManagerRejection::AlreadyOpen(
                activation.window_id,
            ));
        }
        u32::try_from(self.stack.len())
            .map_err(|_| ReplayWindowManagerRejection::StackOrdinalOverflow)?;
        if !valid_traversal(&activation.keyboard_traversal)
            || !valid_traversal(&activation.controller_traversal)
            || !valid_traversal(&activation.tooltip_targets)
        {
            return Err(ReplayWindowManagerRejection::InvalidTraversal(
                activation.window_id,
            ));
        }
        let focusable = activation
            .keyboard_traversal
            .iter()
            .chain(&activation.controller_traversal)
            .collect::<BTreeSet<_>>();
        if activation
            .initial_focus
            .as_ref()
            .is_some_and(|target| !focusable.contains(target))
        {
            return Err(ReplayWindowManagerRejection::InvalidFocusTarget(
                activation.window_id,
            ));
        }
        if let Some(owner) = &activation.modal_owner {
            if owner == &activation.window_id
                || self.input_window_id() != Some(owner.as_str())
                || !self
                    .panels
                    .get(owner)
                    .is_some_and(|row| row.open && row.composed)
            {
                return Err(ReplayWindowManagerRejection::InvalidModalOwner(
                    activation.window_id,
                ));
            }
        } else if let Some(active) = self.input_window_id() {
            if self
                .panels
                .get(active)
                .is_some_and(|row| row.modal_owner.is_some())
            {
                return Err(ReplayWindowManagerRejection::ModalInputBlocked(
                    activation.window_id,
                ));
            }
        }

        let prior_focus = self.focus.clone();
        let initial_focus = activation
            .initial_focus
            .clone()
            .or_else(|| activation.keyboard_traversal.first().cloned())
            .or_else(|| activation.controller_traversal.first().cloned());
        let runtime_instance_token = self.issue_runtime_instance_token(&activation.window_id)?;
        let state = self
            .panels
            .get_mut(&activation.window_id)
            .expect("window existence checked before mutation");
        state.open = true;
        state.visible = true;
        state.composed = true;
        state.runtime_instance_token = Some(runtime_instance_token);
        state.modal_owner = activation.modal_owner;
        state.focused_widget = initial_focus.clone();
        state.keyboard_traversal = activation.keyboard_traversal;
        state.controller_traversal = activation.controller_traversal;
        state.tooltip_targets = activation.tooltip_targets;
        self.focus_restore
            .insert(activation.window_id.clone(), prior_focus);
        self.activation_contracts
            .insert(activation.window_id.clone(), activation_contract);
        self.stack.push(activation.window_id.clone());
        self.focus = initial_focus.map(|widget_id| ReplayFocusTarget {
            window_id: activation.window_id,
            widget_id,
        });
        self.tooltip = None;
        self.hovered = None;
        self.pressed = None;
        self.invalidate_active_input_transition();
        self.finish_transition();
        Ok(())
    }

    pub fn close(&mut self, window_id: &str) -> Result<(), ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        let Some(state) = self.panels.get(window_id) else {
            return Err(ReplayWindowManagerRejection::UnknownWindow(
                window_id.to_string(),
            ));
        };
        if !state.open {
            return Err(ReplayWindowManagerRejection::AlreadyClosed(
                window_id.to_string(),
            ));
        }
        if self
            .panels
            .values()
            .any(|row| row.open && row.modal_owner.as_deref() == Some(window_id))
        {
            return Err(ReplayWindowManagerRejection::ModalInputBlocked(
                window_id.to_string(),
            ));
        }
        let was_input_owner = self.input_window_id() == Some(window_id);
        self.stack.retain(|id| id != window_id);
        self.activation_contracts.remove(window_id);
        let state = self
            .panels
            .get_mut(window_id)
            .expect("window existence checked before mutation");
        if let Some(runtime_instance_token) = state.runtime_instance_token.as_ref() {
            self.engine_instance_tokens.remove(runtime_instance_token);
        }
        state.open = false;
        state.visible = false;
        state.composed = false;
        state.runtime_instance_token = None;
        state.stack_ordinal = None;
        state.modal_owner = None;
        state.focused_widget = None;
        state.keyboard_traversal.clear();
        state.controller_traversal.clear();
        state.tooltip_targets.clear();
        self.geometry.remove(window_id);
        self.canonical_widget_states
            .retain(|(owner, _), _| owner != window_id);
        if self
            .tooltip
            .as_ref()
            .is_some_and(|row| row.window_id == window_id)
        {
            self.tooltip = None;
        }
        if self
            .hovered
            .as_ref()
            .is_some_and(|row| row.window_id == window_id)
        {
            self.hovered = None;
        }
        if self
            .pressed
            .as_ref()
            .is_some_and(|row| row.window_id == window_id)
        {
            self.pressed = None;
            self.invalidate_active_input_transition();
        }
        let restore = self.focus_restore.remove(window_id).flatten();
        if was_input_owner
            || self
                .focus
                .as_ref()
                .is_some_and(|row| row.window_id == window_id)
        {
            self.focus = restore.filter(|target| self.valid_focus_target(target));
            if self.focus.is_none() {
                self.focus = self.default_focus_for_input_owner();
            }
        }
        self.sync_focused_widgets();
        self.finish_transition();
        Ok(())
    }

    pub fn bring_to_front(&mut self, window_id: &str) -> Result<(), ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        let Some(state) = self.panels.get(window_id) else {
            return Err(ReplayWindowManagerRejection::UnknownWindow(
                window_id.to_string(),
            ));
        };
        if !state.open {
            return Err(ReplayWindowManagerRejection::AlreadyClosed(
                window_id.to_string(),
            ));
        }
        if let Some(active) = self.input_window_id() {
            if self
                .panels
                .get(active)
                .is_some_and(|row| row.modal_owner.is_some())
                && active != window_id
            {
                return Err(ReplayWindowManagerRejection::ModalInputBlocked(
                    window_id.to_string(),
                ));
            }
        }
        self.stack.retain(|id| id != window_id);
        self.stack.push(window_id.to_string());
        self.focus = self.default_focus_for(window_id);
        self.tooltip = None;
        self.hovered = None;
        self.pressed = None;
        self.invalidate_active_input_transition();
        self.sync_focused_widgets();
        self.finish_transition();
        Ok(())
    }

    pub fn focus_widget(
        &mut self,
        window_id: &str,
        widget_id: &str,
    ) -> Result<(), ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        if self.input_window_id() != Some(window_id) {
            return Err(ReplayWindowManagerRejection::ModalInputBlocked(
                window_id.to_string(),
            ));
        }
        let target = ReplayFocusTarget {
            window_id: window_id.to_string(),
            widget_id: widget_id.to_string(),
        };
        if !self.valid_focus_target(&target) {
            return Err(ReplayWindowManagerRejection::InvalidFocusTarget(
                window_id.to_string(),
            ));
        }
        self.focus = Some(target);
        self.tooltip = None;
        self.sync_focused_widgets();
        self.bump_transition();
        Ok(())
    }

    pub fn traverse(
        &mut self,
        device: ReplayTraversalDevice,
        direction: ReplayTraversalDirection,
    ) -> Result<&ReplayFocusTarget, ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        let window_id = self
            .input_window_id()
            .ok_or_else(|| ReplayWindowManagerRejection::InvalidFocusTarget("<none>".into()))?
            .to_string();
        let state = self
            .panels
            .get(&window_id)
            .expect("input owner belongs to panel map");
        let traversal = match device {
            ReplayTraversalDevice::Keyboard => &state.keyboard_traversal,
            ReplayTraversalDevice::Controller => &state.controller_traversal,
        };
        if traversal.is_empty() {
            return Err(ReplayWindowManagerRejection::InvalidTraversal(window_id));
        }
        let current = self
            .focus
            .as_ref()
            .filter(|row| row.window_id == window_id)
            .and_then(|row| traversal.iter().position(|id| id == &row.widget_id));
        let next = match (current, direction) {
            (Some(index), ReplayTraversalDirection::Forward) => (index + 1) % traversal.len(),
            (Some(0), ReplayTraversalDirection::Backward) => traversal.len() - 1,
            (Some(index), ReplayTraversalDirection::Backward) => index - 1,
            (None, ReplayTraversalDirection::Forward) => 0,
            (None, ReplayTraversalDirection::Backward) => traversal.len() - 1,
        };
        self.focus = Some(ReplayFocusTarget {
            window_id,
            widget_id: traversal[next].clone(),
        });
        self.tooltip = None;
        self.sync_focused_widgets();
        self.bump_transition();
        Ok(self.focus.as_ref().expect("focus assigned above"))
    }

    pub fn set_authenticated_tooltip_owner(
        &mut self,
        owner: &AuthenticatedReplayTooltipOwner,
        tooltip_instance_record: &str,
    ) -> Result<(), ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        let current = self.authenticated_tooltip_owner_at_cursor()?;
        if tooltip_instance_record.trim().is_empty() || current.as_ref() != Some(owner) {
            return Err(ReplayWindowManagerRejection::InvalidTooltipOwner(
                owner.window_id.clone(),
            ));
        }
        self.tooltip = Some(ReplayTooltipOwnership {
            window_id: owner.window_id.clone(),
            widget_id: owner.element_id.clone(),
            tooltip_instance_record: tooltip_instance_record.to_string(),
        });
        self.bump_transition();
        Ok(())
    }

    pub fn clear_tooltip(&mut self) -> Result<(), ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        if self.tooltip.take().is_some() {
            self.bump_transition();
        }
        Ok(())
    }

    fn runtime_instance_token(&self, window_id: &str) -> Option<&str> {
        self.panels
            .get(window_id)
            .filter(|panel| panel.open)
            .and_then(|panel| panel.runtime_instance_token.as_deref())
    }

    pub fn authenticated_runtime_instance(
        &self,
        window_id: &str,
    ) -> Option<AuthenticatedReplayPanelLease<'_>> {
        if let Some(runtime_instance_token) = self.persistent_instances.get(window_id) {
            return Some(AuthenticatedReplayPanelLease {
                manager: self,
                window_id: window_id.to_string(),
                runtime_instance_token: runtime_instance_token.clone(),
            });
        }
        let panel = self.panels.get(window_id)?;
        if !panel.open {
            return None;
        }
        Some(AuthenticatedReplayPanelLease {
            manager: self,
            window_id: panel.window_id.clone(),
            runtime_instance_token: panel.runtime_instance_token.clone()?,
        })
    }

    fn current_runtime_instance_token(&self, window_id: &str) -> Option<&str> {
        self.persistent_instances
            .get(window_id)
            .map(String::as_str)
            .or_else(|| {
                self.panels
                    .get(window_id)
                    .filter(|panel| panel.open)
                    .and_then(|panel| panel.runtime_instance_token.as_deref())
            })
    }

    /// resulting rectangles and binds them to the current open lifetime, so a
    fn install_composed_geometry(
        &mut self,
        geometry: ReplayComposedPanelGeometry,
    ) -> Result<(), ReplayWindowManagerRejection> {
        let Some(panel) = self.panels.get(&geometry.window_id) else {
            return Err(ReplayWindowManagerRejection::UnknownWindow(
                geometry.window_id,
            ));
        };
        if !panel.open || !panel.composed {
            return Err(ReplayWindowManagerRejection::AlreadyClosed(
                geometry.window_id,
            ));
        }
        if panel.runtime_instance_token.as_deref() != Some(&geometry.runtime_instance_token) {
            return Err(ReplayWindowManagerRejection::StaleRuntimeInstance(
                geometry.window_id,
            ));
        }
        validate_geometry(&geometry)?;
        let checkpoint = self.atomic_checkpoint();
        self.geometry.insert(geometry.window_id.clone(), geometry);
        let hovered = self
            .pointer_position
            .map(|[x, y]| self.hit_test(x, y))
            .transpose();
        let hovered = match hovered {
            Ok(hovered) => hovered.flatten(),
            Err(error) => {
                self.restore_atomic_checkpoint(checkpoint);
                return Err(error);
            }
        };
        self.hovered = hovered;
        self.bump_transition();
        Ok(())
    }

    pub fn composed_snapshot(
        &self,
    ) -> Result<ReplayWindowManagerSnapshot, ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        if self.persistent_instances.len() != self.persistent_geometry.len() {
            return Err(ReplayWindowManagerRejection::MissingComposedGeometry(
                "persistent-hud-instance-census".to_string(),
            ));
        }
        let input_window = self.input_window_id();
        let mut stack = Vec::with_capacity(
            self.persistent_geometry
                .len()
                .saturating_add(self.stack.len()),
        );
        for (ordinal, geometry) in self.persistent_geometry.iter().enumerate() {
            if self
                .persistent_instances
                .get(&geometry.window_id)
                .map(String::as_str)
                != Some(geometry.runtime_instance_token.as_str())
            {
                return Err(ReplayWindowManagerRejection::StaleRuntimeInstance(
                    geometry.window_id.clone(),
                ));
            }
            let stack_ordinal = u32::try_from(ordinal)
                .map_err(|_| ReplayWindowManagerRejection::StackOrdinalOverflow)?;
            stack.push(snapshot_geometry(
                geometry,
                stack_ordinal,
                None,
                false,
                None,
                self.hovered.as_ref(),
                self.pressed.as_ref(),
            ));
        }
        for (ordinal, window_id) in self.stack.iter().enumerate() {
            let stack_index = self
                .persistent_geometry
                .len()
                .checked_add(ordinal)
                .ok_or(ReplayWindowManagerRejection::StackOrdinalOverflow)?;
            let stack_ordinal = u32::try_from(stack_index)
                .map_err(|_| ReplayWindowManagerRejection::StackOrdinalOverflow)?;
            let panel = self
                .panels
                .get(window_id)
                .expect("stack entries belong to panel map");
            let geometry = self.geometry.get(window_id).ok_or_else(|| {
                ReplayWindowManagerRejection::MissingComposedGeometry(window_id.clone())
            })?;
            if panel.runtime_instance_token.as_deref()
                != Some(geometry.runtime_instance_token.as_str())
            {
                return Err(ReplayWindowManagerRejection::StaleRuntimeInstance(
                    window_id.clone(),
                ));
            }
            stack.push(snapshot_geometry(
                geometry,
                stack_ordinal,
                panel.modal_owner.clone(),
                input_window == Some(window_id.as_str()),
                panel.focused_widget.as_deref(),
                self.hovered.as_ref(),
                self.pressed.as_ref(),
            ));
        }
        Ok(ReplayWindowManagerSnapshot {
            transition_ordinal: self.transition_ordinal,
            committed_frame_generation: self.committed_frame_generation,
            viewer_clock_micros: self.last_viewer_clock_micros.unwrap_or(0),
            dispatched_input_transition_generation: self.dispatched_input_transition_generation,
            pointer_position: self.pointer_position,
            hovered: self.hovered.clone(),
            pressed: self.pressed.clone(),
            tooltip: self.tooltip.clone(),
            stack,
        })
    }

    fn stage_current_widget_states(
        &mut self,
        index: &AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        panels: &BTreeMap<String, StrictAuthoredPanelAdmission>,
        viewer_actions: BTreeMap<(String, Vec<u32>), AuthenticatedReplayViewerAction>,
        frame_authority: AuthenticatedReplayFrameAuthority,
    ) -> Result<StagedReplayCanonicalStates, WidgetVisualRejection> {
        let states =
            match self.build_current_widget_states(index, admission, panels, &frame_authority) {
                Ok(states) => states,
                Err(error) => {
                    if frame_authority.remains_staged_for(self) {
                        self.staged_frame_generation = None;
                    }
                    return Err(error);
                }
            };
        Ok(StagedReplayCanonicalStates {
            frame_authority,
            states,
            viewer_actions,
        })
    }

    fn build_current_widget_states(
        &self,
        index: &AuthenticatedGuiRoleIndex,
        admission: &ValidatedContentAdmission,
        panels: &BTreeMap<String, StrictAuthoredPanelAdmission>,
        frame_authority: &AuthenticatedReplayFrameAuthority,
    ) -> Result<BTreeMap<(String, Vec<u32>), AuthenticatedViewerWidgetState>, WidgetVisualRejection>
    {
        if !frame_authority.remains_staged_for(self) {
            return Err(WidgetVisualRejection::InvalidViewerLifecycle);
        }
        let snapshot = self
            .composed_snapshot()
            .map_err(|_| WidgetVisualRejection::InvalidViewerLifecycle)?;
        let mut sealed = BTreeMap::new();
        for window in snapshot.stack {
            let panel = panels
                .get(&window.window_id)
                .ok_or(WidgetVisualRejection::UnindexedElement)?;
            let instance = self
                .authenticated_runtime_instance(&window.window_id)
                .ok_or(WidgetVisualRejection::InvalidViewerLifecycle)?;
            for element in window.element_interactions {
                let Some(visual_enabled) = element.visual_enabled else {
                    continue;
                };
                let key = (window.window_id.clone(), element.child_path.clone());
                let state = advance_authenticated_viewer_widget_state(
                    self,
                    frame_authority,
                    index,
                    admission,
                    panel,
                    &instance,
                    self.canonical_widget_states.get(&key),
                    &element.child_path,
                    ViewerWidgetLifecycle {
                        visible: element.visible,
                        enabled: visual_enabled,
                        hovered: element.hovered
                            && element.disposition == ReplayPointerDisposition::Interactive,
                        pressed: element.pressed
                            && element.disposition == ReplayPointerDisposition::Interactive,
                        focused: element.focused
                            && element.disposition == ReplayPointerDisposition::Interactive,
                    },
                )?;
                sealed.insert(key, state);
            }
        }
        Ok(sealed)
    }

    fn require_current_interactive_state(
        &self,
        target: &ReplayPointerTarget,
    ) -> Result<(), ReplayWindowManagerRejection> {
        if target.element_type == "extendedScrollbarSlider" {
            let current = self.geometry.get(&target.window_id).and_then(|geometry| {
                geometry.hit_targets.iter().find(|candidate| {
                    candidate.element_id == target.element_id
                        && candidate.runtime_instance_token == target.runtime_instance_token
                        && candidate.child_path == target.child_path
                })
            });
            if self.committed_frame_generation != 0
                && current.is_some_and(|candidate| {
                    candidate.disposition == ReplayPointerDisposition::Interactive
                        && candidate.visual_enabled == Some(true)
                })
            {
                return Ok(());
            }
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                target.element_id.clone(),
            ));
        }
        if let Some(action) = self
            .canonical_viewer_actions
            .get(&(target.window_id.clone(), target.child_path.clone()))
            .filter(|_| target.child_path.first() == Some(&u32::MAX))
        {
            if self.committed_frame_generation != 0
                && action.frame_generation == self.committed_frame_generation
                && action.window_id == target.window_id
                && action.runtime_instance_token == target.runtime_instance_token
                && action.rendered_child_path == target.child_path
                && target.disposition == ReplayPointerDisposition::Interactive
                && target.visual_enabled == Some(true)
            {
                return Ok(());
            }
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                target.element_id.clone(),
            ));
        }
        let state = self
            .canonical_widget_states
            .get(&(target.window_id.clone(), target.child_path.clone()))
            .ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidPointerState(
                    "pointer-target-absent-from-canonical-frame".to_string(),
                )
            })?;
        let lifecycle = state.lifecycle();
        if self.committed_frame_generation == 0
            || self.canonical_widget_state_generation != self.committed_frame_generation
            || state.frame_generation() != self.committed_frame_generation
            || state.identity().window_id() != target.window_id.as_str()
            || state.identity().child_path() != target.child_path.as_slice()
            || state.runtime_instance_token() != target.runtime_instance_token.as_str()
            || target.disposition != ReplayPointerDisposition::Interactive
            || target.visual_enabled != Some(true)
            || !lifecycle.visible
            || !lifecycle.enabled
        {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                target.element_id.clone(),
            ));
        }
        Ok(())
    }

    fn reconcile_pointer_capture_with_current_geometry(
        &mut self,
    ) -> Result<(), ReplayWindowManagerRejection> {
        let Some(previous) = self.pressed.clone() else {
            return Ok(());
        };
        let mut replacement = None;
        for (ordinal, geometry) in self.persistent_geometry.iter().enumerate() {
            if let Some(target) = geometry.hit_targets.iter().find(|target| {
                same_pointer_identity(&previous, target)
                    && target.disposition == ReplayPointerDisposition::Interactive
                    && target.visual_enabled == Some(true)
            }) {
                replacement = Some(pointer_target(
                    target,
                    u32::try_from(ordinal)
                        .map_err(|_| ReplayWindowManagerRejection::StackOrdinalOverflow)?,
                ));
                break;
            }
        }
        if replacement.is_none() {
            for (ordinal, window_id) in self.stack.iter().enumerate() {
                let Some(geometry) = self.geometry.get(window_id) else {
                    continue;
                };
                if let Some(target) = geometry.hit_targets.iter().find(|target| {
                    same_pointer_identity(&previous, target)
                        && target.disposition == ReplayPointerDisposition::Interactive
                        && target.visual_enabled == Some(true)
                }) {
                    let stack_ordinal = self
                        .persistent_geometry
                        .len()
                        .checked_add(ordinal)
                        .ok_or(ReplayWindowManagerRejection::StackOrdinalOverflow)?;
                    replacement = Some(pointer_target(
                        target,
                        u32::try_from(stack_ordinal)
                            .map_err(|_| ReplayWindowManagerRejection::StackOrdinalOverflow)?,
                    ));
                    break;
                }
            }
        }
        self.pressed = replacement;
        if self.pressed.is_none() {
            self.invalidate_active_input_transition();
        }
        Ok(())
    }

    fn reconcile_current_activation_state(&mut self) -> Result<(), ReplayWindowManagerRejection> {
        for window_id in &self.stack {
            let contract = self
                .activation_contracts
                .get(window_id)
                .ok_or_else(|| ReplayWindowManagerRejection::InvalidTraversal(window_id.clone()))?;
            let geometry = self.geometry.get(window_id).ok_or_else(|| {
                ReplayWindowManagerRejection::MissingComposedGeometry(window_id.clone())
            })?;
            let enabled = geometry
                .element_states
                .iter()
                .filter(|element| {
                    element.visible
                        && element.disposition == ReplayPointerDisposition::Interactive
                        && element.visual_enabled == Some(true)
                })
                .map(|element| element.element_id.as_str())
                .collect::<BTreeSet<_>>();
            let tooltip_visible = geometry
                .element_states
                .iter()
                .filter(|element| {
                    element.visible && element.disposition != ReplayPointerDisposition::PassThrough
                })
                .map(|element| element.element_id.as_str())
                .collect::<BTreeSet<_>>();
            let panel = self
                .panels
                .get_mut(window_id)
                .ok_or_else(|| ReplayWindowManagerRejection::UnknownWindow(window_id.clone()))?;
            panel.keyboard_traversal = contract
                .keyboard_traversal
                .iter()
                .filter(|element_id| enabled.contains(element_id.as_str()))
                .cloned()
                .collect();
            panel.controller_traversal = contract
                .controller_traversal
                .iter()
                .filter(|element_id| enabled.contains(element_id.as_str()))
                .cloned()
                .collect();
            panel.tooltip_targets = contract
                .tooltip_targets
                .iter()
                .filter(|element_id| tooltip_visible.contains(element_id.as_str()))
                .cloned()
                .collect();
        }
        if self.focus.as_ref().is_some_and(|focus| {
            !self.panels.get(&focus.window_id).is_some_and(|panel| {
                panel.keyboard_traversal.contains(&focus.widget_id)
                    || panel.controller_traversal.contains(&focus.widget_id)
            })
        }) {
            self.focus = self.default_focus_for_input_owner();
        }
        if self.tooltip.as_ref().is_some_and(|tooltip| {
            !(self
                .panels
                .get(&tooltip.window_id)
                .is_some_and(|panel| panel.tooltip_targets.contains(&tooltip.widget_id))
                || self
                    .persistent_tooltip_targets
                    .get(&tooltip.window_id)
                    .is_some_and(|targets| targets.contains(&tooltip.widget_id)))
        }) {
            self.tooltip = None;
        }
        self.sync_focused_widgets();
        Ok(())
    }

    pub fn pointer_target_at(
        &self,
        x: i32,
        y: i32,
    ) -> Result<Option<ReplayPointerTarget>, ReplayWindowManagerRejection> {
        self.hit_test(x, y)
    }

    pub fn blocks_lower_pointer_input_at(
        &self,
        x: i32,
        y: i32,
    ) -> Result<bool, ReplayWindowManagerRejection> {
        self.ensure_complete_geometry()?;
        if self.stack.iter().rev().any(|window_id| {
            self.panels
                .get(window_id)
                .is_some_and(|panel| panel.modal_owner.is_some())
        }) {
            return Ok(true);
        }
        Ok(self.hit_test(x, y)?.is_some())
    }

    pub fn pointer_move(
        &mut self,
        x: i32,
        y: i32,
    ) -> Result<Option<&ReplayPointerTarget>, ReplayWindowManagerRejection> {
        self.ensure_complete_geometry()?;
        if let (Some(previous), Some(pressed)) = (self.pointer_position, self.pressed.clone()) {
            if pressed.element_type == "extendedScrollbarSlider" {
                self.drag_extended_scrollbar(&pressed, previous[1], y)?;
            }
        }
        self.pointer_position = Some([x, y]);
        self.hovered = self.hit_test(x, y)?;
        self.bump_transition();
        Ok(self.hovered.as_ref())
    }

    pub fn record_pending_pointer_position(
        &mut self,
        x: i32,
        y: i32,
    ) -> Result<(), ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        self.pointer_position = Some([x, y]);
        self.hovered = None;
        self.pressed = None;
        self.invalidate_active_input_transition();
        self.bump_transition();
        Ok(())
    }

    pub(crate) fn canonical_pointer_press(
        &mut self,
        x: i32,
        y: i32,
    ) -> Result<
        (
            Option<ReplayPointerTarget>,
            Option<AuthenticatedReplayPressAuthority>,
        ),
        ReplayWindowManagerRejection,
    > {
        let checkpoint = self.atomic_checkpoint();
        let result = (|| {
            if self.pressed.is_some() || self.active_input_transition.is_some() {
                return Err(ReplayWindowManagerRejection::InvalidPointerState(
                    "primary-pointer-capture-already-active".to_string(),
                ));
            }
            self.pointer_move(x, y)?;
            self.pressed = self.hovered.as_ref().and_then(|target| {
                (target.disposition == ReplayPointerDisposition::Interactive
                    && target.visual_enabled == Some(true))
                .then_some(target.clone())
            });
            if let Some(target) = self.pressed.as_ref() {
                let focus = ReplayFocusTarget {
                    window_id: target.window_id.clone(),
                    widget_id: target.element_id.clone(),
                };
                if self.valid_focus_target(&focus) {
                    self.focus = Some(focus);
                    self.sync_focused_widgets();
                }
            }
            self.bump_transition();
            let target = self.pressed.clone();
            let authority = target
                .as_ref()
                .map(|_| self.issue_press_authority_for_current_edge())
                .transpose()?;
            Ok((target, authority))
        })();
        if result.is_err() {
            self.restore_atomic_checkpoint(checkpoint);
        }
        result
    }

    pub fn pointer_press(
        &mut self,
        x: i32,
        y: i32,
    ) -> Result<Option<ReplayPointerTarget>, ReplayWindowManagerRejection> {
        let prospective = self.hit_test(x, y)?;
        if let Some(target) = prospective
            .as_ref()
            .filter(|target| target.disposition == ReplayPointerDisposition::Interactive)
        {
            self.require_current_interactive_state(target)?;
        }
        self.canonical_pointer_press(x, y)
            .map(|(target, _authority)| target)
    }

    pub(crate) fn canonical_pointer_release(
        &mut self,
        x: i32,
        y: i32,
    ) -> Result<
        (
            ReplayPointerRelease,
            Option<AuthenticatedReplayReleaseAuthority>,
        ),
        ReplayWindowManagerRejection,
    > {
        self.pointer_move(x, y)?;
        let pressed = self.pressed.take();
        let released_over = self.hovered.clone();
        let authority = self.consume_release_authority(released_over.as_ref())?;
        self.bump_transition();
        Ok((
            ReplayPointerRelease {
                pointer_x: x,
                pointer_y: y,
                pressed,
                released_over,
                // the one-shot release authority returned alongside this
                activated: false,
                activated_action: None,
            },
            authority,
        ))
    }

    pub fn pointer_release(
        &mut self,
        x: i32,
        y: i32,
    ) -> Result<ReplayPointerRelease, ReplayWindowManagerRejection> {
        let checkpoint = self.atomic_checkpoint();
        let result = (|| {
            let (mut release, authority) = self.canonical_pointer_release(x, y)?;
            release.activated = match (authority.as_ref(), release.released_over.as_ref()) {
                (Some(authority), Some(target))
                    if release.pressed.as_ref().map_or(true, |pressed| {
                        pressed.element_type != "extendedScrollbarSlider"
                    }) && authority.matches_consumed_generation(self)
                        && authority.window_id() == target.window_id.as_str()
                        && authority.runtime_instance_token()
                            == target.runtime_instance_token.as_str()
                        && authority.child_path() == target.child_path.as_slice() =>
                {
                    self.require_current_interactive_state(target).is_ok()
                }
                _ => false,
            };
            if release.activated {
                let target = release
                    .released_over
                    .as_ref()
                    .expect("activated release has an exact target");
                let mut action = self
                    .canonical_viewer_actions
                    .get(&(target.window_id.clone(), target.child_path.clone()))
                    .filter(|action| {
                        action.runtime_instance_token == target.runtime_instance_token
                            && action.frame_generation == self.committed_frame_generation
                    })
                    .cloned()
                    .ok_or_else(|| {
                        ReplayWindowManagerRejection::InvalidPointerState(
                            "activated-target-lacks-canonical-viewer-action".to_string(),
                        )
                    })?;
                action.input_transition_generation = authority
                    .as_ref()
                    .expect("activated release consumed an authority")
                    .input_transition_generation();
                release.activated_action = Some(action);
            }
            Ok(release)
        })();
        if result.is_err() {
            self.restore_atomic_checkpoint(checkpoint);
        }
        result
    }

    /// Consume a release-sealed viewer action exactly once.
    /// The semantic action is returned only while its panel lifetime, last
    /// remain current. Retaining or cloning the opaque release value cannot
    pub fn consume_authenticated_viewer_action(
        &mut self,
        action: &AuthenticatedReplayViewerAction,
    ) -> Result<ReplayViewerAction, ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        let key = (action.window_id.clone(), action.rendered_child_path.clone());
        let canonical = self.canonical_viewer_actions.get(&key).ok_or_else(|| {
            ReplayWindowManagerRejection::InvalidPointerState(
                "viewer-action-is-not-canonical".to_string(),
            )
        })?;
        if action.input_transition_generation == 0
            || action.input_transition_generation != self.consumed_input_transition_generation
            || action.input_transition_generation <= self.dispatched_input_transition_generation
            || action.input_transition_generation > self.input_transition_generation
            || action.window_id != canonical.window_id
            || action.runtime_instance_token != canonical.runtime_instance_token
            || action.frame_generation != canonical.frame_generation
            || action.rendered_child_path != canonical.rendered_child_path
            || action.action != canonical.action
            || action.frame_generation != self.committed_frame_generation
            || self.current_runtime_instance_token(&action.window_id)
                != Some(action.runtime_instance_token.as_str())
        {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "stale-or-replayed-viewer-action".to_string(),
            ));
        }
        self.dispatched_input_transition_generation = action.input_transition_generation;
        self.bump_transition();
        Ok(action.action.clone())
    }

    pub fn activate_focused_viewer_action(
        &mut self,
    ) -> Result<Option<AuthenticatedReplayViewerAction>, ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        self.ensure_complete_geometry()?;
        let Some(focus) = self.focus.clone() else {
            return Ok(None);
        };
        if self.input_window_id() != Some(focus.window_id.as_str()) {
            return Err(ReplayWindowManagerRejection::InvalidFocusTarget(
                focus.window_id,
            ));
        }
        let geometry = self.geometry.get(&focus.window_id).ok_or_else(|| {
            ReplayWindowManagerRejection::MissingComposedGeometry(focus.window_id.clone())
        })?;
        let target = geometry
            .hit_targets
            .iter()
            .find(|target| {
                target.element_id == focus.widget_id
                    && target.disposition == ReplayPointerDisposition::Interactive
                    && target.visual_enabled == Some(true)
            })
            .ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidFocusTarget(focus.window_id.clone())
            })?;
        let pointer_target = pointer_target(
            target,
            u32::try_from(
                self.persistent_geometry
                    .len()
                    .checked_add(
                        self.stack
                            .iter()
                            .position(|window_id| window_id == &focus.window_id)
                            .ok_or_else(|| {
                                ReplayWindowManagerRejection::InvalidFocusTarget(
                                    focus.window_id.clone(),
                                )
                            })?,
                    )
                    .ok_or(ReplayWindowManagerRejection::StackOrdinalOverflow)?,
            )
            .map_err(|_| ReplayWindowManagerRejection::StackOrdinalOverflow)?,
        );
        self.require_current_interactive_state(&pointer_target)?;
        if self.active_input_transition.is_some() || self.pressed.is_some() {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "keyboard-activation-during-pointer-capture".to_string(),
            ));
        }
        let generation = self
            .input_transition_generation
            .checked_add(1)
            .ok_or(ReplayWindowManagerRejection::InputTransitionGenerationOverflow)?;
        let mut action = self
            .canonical_viewer_actions
            .get(&(focus.window_id.clone(), target.child_path.clone()))
            .filter(|action| {
                action.runtime_instance_token == target.runtime_instance_token
                    && action.frame_generation == self.committed_frame_generation
            })
            .cloned()
            .ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidPointerState(
                    "focused-target-lacks-canonical-viewer-action".to_string(),
                )
            })?;
        self.input_transition_generation = generation;
        self.consumed_input_transition_generation = generation;
        action.input_transition_generation = generation;
        self.bump_transition();
        Ok(Some(action))
    }

    pub fn cancel_pointer_capture(&mut self) -> Result<(), ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        if self.pressed.take().is_some() | self.invalidate_active_input_transition() {
            self.bump_transition();
        }
        Ok(())
    }

    fn invalidate_active_input_transition(&mut self) -> bool {
        let Some(active) = self.active_input_transition.take() else {
            return false;
        };
        self.consumed_input_transition_generation = active.generation;
        true
    }

    pub fn scroll_at(
        &mut self,
        x: i32,
        y: i32,
        delta_pixels: ReplayScrollDeltaPixels,
    ) -> Result<ReplayScrollOutcome, ReplayWindowManagerRejection> {
        self.ensure_complete_geometry()?;
        let target = self
            .topmost_scroll_region(x, y)?
            .ok_or(ReplayWindowManagerRejection::ScrollTargetUnavailable)?;
        self.apply_scroll_delta(target, delta_pixels.value())
    }

    pub fn scroll_wheel_notches_at(
        &mut self,
        x: i32,
        y: i32,
        emitted_notches: i32,
    ) -> Result<Option<ReplayScrollOutcome>, ReplayWindowManagerRejection> {
        self.ensure_complete_geometry()?;
        if emitted_notches == 0 {
            return Ok(None);
        }
        let Some(target) = self.topmost_scroll_region(x, y)? else {
            return Ok(None);
        };
        let step = self
            .geometry
            .get(&target.window_id)
            .or_else(|| {
                self.persistent_geometry.iter().find(|geometry| {
                    geometry.window_id == target.window_id
                        && geometry.runtime_instance_token == target.runtime_instance_token
                })
            })
            .and_then(|geometry| {
                geometry.scroll_regions.iter().find(|region| {
                    region.runtime_instance_token == target.runtime_instance_token
                        && region.element_id == target.element_id
                        && region.child_path == target.child_path
                })
            })
            .and_then(|region| region.wheel_step_pixels);
        let Some(step) = step else {
            return Ok(None);
        };
        let delta_pixels = emitted_notches
            .checked_mul(step)
            .and_then(i32::checked_neg)
            .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
        self.apply_scroll_delta(target, delta_pixels).map(Some)
    }

    fn apply_scroll_delta(
        &mut self,
        target: ReplayScrollRegion,
        delta_pixels: i32,
    ) -> Result<ReplayScrollOutcome, ReplayWindowManagerRejection> {
        let geometry = if let Some(geometry) = self.geometry.get_mut(&target.window_id) {
            geometry
        } else {
            self.persistent_geometry
                .iter_mut()
                .find(|geometry| {
                    geometry.window_id == target.window_id
                        && geometry.runtime_instance_token == target.runtime_instance_token
                })
                .ok_or_else(|| {
                    ReplayWindowManagerRejection::InvalidScrollRegion(target.element_id.clone())
                })?
        };
        let region_index = geometry
            .scroll_regions
            .iter()
            .position(|region| {
                region.runtime_instance_token == target.runtime_instance_token
                    && region.element_id == target.element_id
                    && region.child_path == target.child_path
            })
            .expect("scroll target was cloned from this geometry");
        let next_offset = geometry.scroll_regions[region_index]
            .offset_pixels
            .checked_add(delta_pixels)
            .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
        let outcome = set_scroll_region_offset(geometry, region_index, next_offset)?;
        self.bump_transition();
        Ok(outcome)
    }

    fn drag_extended_scrollbar(
        &mut self,
        pressed: &ReplayPointerTarget,
        previous_pointer_y: i32,
        pointer_y: i32,
    ) -> Result<(), ReplayWindowManagerRejection> {
        let geometry = self.geometry.get_mut(&pressed.window_id).ok_or_else(|| {
            ReplayWindowManagerRejection::InvalidScrollRegion(pressed.element_id.clone())
        })?;
        let region_index = geometry
            .scroll_regions
            .iter()
            .position(|region| {
                format!("{}.extended-scrollbar-slider", region.element_id) == pressed.element_id
                    && region.runtime_instance_token == pressed.runtime_instance_token
            })
            .ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidScrollRegion(pressed.element_id.clone())
            })?;
        let region = &geometry.scroll_regions[region_index];
        let scrollbar = region.extended_scrollbar.as_ref().ok_or_else(|| {
            ReplayWindowManagerRejection::InvalidScrollRegion(pressed.element_id.clone())
        })?;
        let grab_offset = previous_pointer_y
            .checked_sub(scrollbar.slider_rect.y())
            .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
        let desired_slider_y = pointer_y
            .checked_sub(grab_offset)
            .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
        let desired_mapped_y = desired_slider_y
            .checked_sub(scrollbar.thumb_authored_offset_pixels)
            .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
        let next_offset = exact_extended_scrollbar_offset_for_pixel(
            region.max_offset_pixels,
            scrollbar.thumb_base_pixels,
            scrollbar.thumb_travel_pixels,
            desired_mapped_y,
        )?;
        set_scroll_region_offset(geometry, region_index, next_offset)?;
        if let Some(current) = geometry
            .hit_targets
            .iter()
            .find(|target| target.element_id == pressed.element_id)
        {
            self.pressed = Some(ReplayPointerTarget {
                screen_rect: current.screen_rect,
                clip_rect: current.clip_rect,
                ..pressed.clone()
            });
        }
        Ok(())
    }

    pub fn scroll_region_offset(
        &self,
        window_id: &str,
        element_id: &str,
    ) -> Result<Option<i32>, ReplayWindowManagerRejection> {
        self.ensure_complete_geometry()?;
        Ok(self
            .geometry
            .get(window_id)
            .and_then(|geometry| {
                geometry
                    .scroll_regions
                    .iter()
                    .find(|region| region.element_id == element_id)
            })
            .map(|region| region.offset_pixels))
    }

    fn ensure_complete_geometry(&self) -> Result<(), ReplayWindowManagerRejection> {
        self.require_no_pending_presentation()?;
        for window_id in &self.stack {
            if !self.geometry.contains_key(window_id) {
                return Err(ReplayWindowManagerRejection::MissingComposedGeometry(
                    window_id.clone(),
                ));
            }
        }
        Ok(())
    }

    fn require_no_pending_presentation(&self) -> Result<(), ReplayWindowManagerRejection> {
        if self.pending_presentation.is_some() {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                "authenticated-frame-awaits-presentation".to_string(),
            ));
        }
        Ok(())
    }

    fn hit_test(
        &self,
        x: i32,
        y: i32,
    ) -> Result<Option<ReplayPointerTarget>, ReplayWindowManagerRejection> {
        self.ensure_complete_geometry()?;
        let global_stack_ordinal = u32::try_from(
            self.persistent_geometry
                .len()
                .checked_add(self.stack.len())
                .ok_or(ReplayWindowManagerRejection::StackOrdinalOverflow)?,
        )
        .map_err(|_| ReplayWindowManagerRejection::StackOrdinalOverflow)?;
        for geometry in self
            .stack
            .iter()
            .rev()
            .filter_map(|window_id| self.geometry.get(window_id))
        {
            if let Some(target) = geometry.hit_targets.iter().rev().find(|target| {
                target.global_root_sibling
                    && target.disposition != ReplayPointerDisposition::PassThrough
                    && rect_contains(target.screen_rect, x, y).unwrap_or(false)
                    && rect_contains(target.clip_rect, x, y).unwrap_or(false)
            }) {
                return Ok(Some(pointer_target(target, global_stack_ordinal)));
            }
        }
        for geometry in self.persistent_geometry.iter().rev() {
            if let Some(target) = geometry.hit_targets.iter().rev().find(|target| {
                target.global_root_sibling
                    && target.disposition != ReplayPointerDisposition::PassThrough
                    && rect_contains(target.screen_rect, x, y).unwrap_or(false)
                    && rect_contains(target.clip_rect, x, y).unwrap_or(false)
            }) {
                return Ok(Some(pointer_target(target, global_stack_ordinal)));
            }
        }
        let active_modal = self.stack.iter().rev().find(|window_id| {
            self.panels
                .get(*window_id)
                .is_some_and(|panel| panel.modal_owner.is_some())
        });
        for (stack_index, window_id) in self.stack.iter().enumerate().rev() {
            if active_modal.is_some_and(|modal| modal != window_id) {
                continue;
            }
            let stack_index = self
                .persistent_geometry
                .len()
                .checked_add(stack_index)
                .ok_or(ReplayWindowManagerRejection::StackOrdinalOverflow)?;
            let stack_ordinal = u32::try_from(stack_index)
                .map_err(|_| ReplayWindowManagerRejection::StackOrdinalOverflow)?;
            let geometry = self
                .geometry
                .get(window_id)
                .expect("complete geometry checked above");
            if !rect_contains(geometry.clip_rect, x, y)? {
                continue;
            }
            if let Some(target) = geometry.hit_targets.iter().rev().find(|target| {
                target.disposition != ReplayPointerDisposition::PassThrough
                    && rect_contains(target.screen_rect, x, y).unwrap_or(false)
                    && rect_contains(target.clip_rect, x, y).unwrap_or(false)
            }) {
                return Ok(Some(pointer_target(target, stack_ordinal)));
            }
        }
        if active_modal.is_some() {
            return Ok(None);
        }
        for (stack_index, geometry) in self.persistent_geometry.iter().enumerate().rev() {
            let stack_ordinal = u32::try_from(stack_index)
                .map_err(|_| ReplayWindowManagerRejection::StackOrdinalOverflow)?;
            if !rect_contains(geometry.clip_rect, x, y)? {
                continue;
            }
            if let Some(target) = geometry.hit_targets.iter().rev().find(|target| {
                target.disposition != ReplayPointerDisposition::PassThrough
                    && rect_contains(target.screen_rect, x, y).unwrap_or(false)
                    && rect_contains(target.clip_rect, x, y).unwrap_or(false)
            }) {
                return Ok(Some(pointer_target(target, stack_ordinal)));
            }
        }
        Ok(None)
    }

    fn topmost_scroll_region(
        &self,
        x: i32,
        y: i32,
    ) -> Result<Option<ReplayScrollRegion>, ReplayWindowManagerRejection> {
        if self
            .geometry
            .values()
            .chain(self.persistent_geometry.iter())
            .flat_map(|geometry| geometry.hit_targets.iter())
            .any(|target| {
                target.global_root_sibling
                    && target.disposition != ReplayPointerDisposition::PassThrough
                    && rect_contains(target.screen_rect, x, y).unwrap_or(false)
                    && rect_contains(target.clip_rect, x, y).unwrap_or(false)
            })
        {
            return Ok(None);
        }
        let active_modal = self.stack.iter().rev().find(|window_id| {
            self.panels
                .get(*window_id)
                .is_some_and(|panel| panel.modal_owner.is_some())
        });
        for window_id in self.stack.iter().rev() {
            if active_modal.is_some_and(|modal| modal != window_id) {
                continue;
            }
            let geometry = self.geometry.get(window_id).ok_or_else(|| {
                ReplayWindowManagerRejection::MissingComposedGeometry(window_id.clone())
            })?;
            if !rect_contains(geometry.clip_rect, x, y)? {
                continue;
            }
            if let Some(region) = geometry
                .scroll_regions
                .iter()
                .rev()
                .find(|region| rect_contains(region.viewport_rect, x, y).unwrap_or(false))
            {
                return Ok(Some(region.clone()));
            }
            if geometry.hit_targets.iter().any(|target| {
                target.disposition != ReplayPointerDisposition::PassThrough
                    && rect_contains(target.screen_rect, x, y).unwrap_or(false)
                    && rect_contains(target.clip_rect, x, y).unwrap_or(false)
            }) {
                return Ok(None);
            }
        }
        if active_modal.is_some() {
            return Ok(None);
        }
        for geometry in self.persistent_geometry.iter().rev() {
            if !rect_contains(geometry.clip_rect, x, y)? {
                continue;
            }
            if let Some(region) = geometry
                .scroll_regions
                .iter()
                .rev()
                .find(|region| rect_contains(region.viewport_rect, x, y).unwrap_or(false))
            {
                return Ok(Some(region.clone()));
            }
            if geometry.hit_targets.iter().any(|target| {
                target.disposition != ReplayPointerDisposition::PassThrough
                    && rect_contains(target.screen_rect, x, y).unwrap_or(false)
                    && rect_contains(target.clip_rect, x, y).unwrap_or(false)
            }) {
                return Ok(None);
            }
        }
        Ok(None)
    }

    fn valid_focus_target(&self, target: &ReplayFocusTarget) -> bool {
        self.panels.get(&target.window_id).is_some_and(|state| {
            state.open
                && (state.keyboard_traversal.contains(&target.widget_id)
                    || state.controller_traversal.contains(&target.widget_id))
        })
    }

    fn default_focus_for(&self, window_id: &str) -> Option<ReplayFocusTarget> {
        let state = self.panels.get(window_id)?;
        state
            .keyboard_traversal
            .first()
            .or_else(|| state.controller_traversal.first())
            .map(|widget_id| ReplayFocusTarget {
                window_id: window_id.to_string(),
                widget_id: widget_id.clone(),
            })
    }

    fn default_focus_for_input_owner(&self) -> Option<ReplayFocusTarget> {
        self.input_window_id()
            .and_then(|window_id| self.default_focus_for(window_id))
    }

    fn sync_focused_widgets(&mut self) {
        for state in self.panels.values_mut() {
            state.focused_widget = self
                .focus
                .as_ref()
                .filter(|target| target.window_id == state.window_id)
                .map(|target| target.widget_id.clone());
        }
    }

    fn finish_transition(&mut self) {
        for state in self.panels.values_mut() {
            state.stack_ordinal = None;
        }
        for (ordinal, window_id) in self.stack.iter().enumerate() {
            let ordinal = u32::try_from(ordinal)
                .expect("stack growth validates the prospective ordinal before mutation");
            let state = self
                .panels
                .get_mut(window_id)
                .expect("stack entries belong to panel map");
            state.stack_ordinal = Some(ordinal);
            state.open = true;
            state.visible = true;
            state.composed = true;
        }
        self.bump_transition();
    }

    fn bump_transition(&mut self) {
        self.staged_frame_generation = None;
        self.transition_ordinal = self.transition_ordinal.saturating_add(1);
    }
}

#[cfg(test)]
mod replay_runtime_instance_tests {
    use super::*;

    #[test]
    fn exact_theatre_scroll_clamps_before_binding_identity() {
        assert_eq!(clamp_exact_theatre_scroll(120, 240, 100).unwrap(), 120);
        assert_eq!(clamp_exact_theatre_scroll(120, 80, 100).unwrap(), 0);
        assert_eq!(clamp_exact_theatre_scroll(120, 110, 100).unwrap(), 10);
        assert!(clamp_exact_theatre_scroll(-1, 240, 100).is_err());
    }

    #[test]
    fn exact_theatre_scroll_extent_binds_final_clipped_viewport() {
        let list = AuthenticatedElementRect::new(10, 20, 300, 100);
        let full = exact_theatre_scroll_extent(
            200,
            240,
            list,
            AuthenticatedElementRect::new(10, 20, 300, 100),
        )
        .unwrap();
        let clipped = exact_theatre_scroll_extent(
            200,
            240,
            list,
            AuthenticatedElementRect::new(10, 40, 300, 60),
        )
        .unwrap();
        assert_eq!(full.max_offset_pixels, 140);
        assert_eq!(full.effective_offset_pixels, 140);
        assert_eq!(clipped.max_offset_pixels, 180);
        assert_eq!(clipped.effective_offset_pixels, 180);
        assert_ne!(full.identity_sha256, clipped.identity_sha256);
        assert!(exact_theatre_scroll_extent(
            0,
            240,
            list,
            AuthenticatedElementRect::new(10, 10, 300, 100),
        )
        .is_err());
    }

    #[test]
    fn materialized_read_only_buttons_block_without_becoming_actions() {
        use crate::runtime_graphics::ButtonVisualState;

        assert_eq!(
            materialized_pointer_disposition(
                Some(&AuthenticatedElementSemantic::ReadOnlyButton {
                    one_based_frame: 1,
                    interaction: ButtonVisualState::Disable,
                }),
                false,
            ),
            (ReplayPointerDisposition::BlockOnly, Some(false))
        );
        assert_eq!(
            materialized_pointer_disposition(
                Some(&AuthenticatedElementSemantic::ReadOnlyButton {
                    one_based_frame: 1,
                    interaction: ButtonVisualState::Up,
                }),
                false,
            ),
            (ReplayPointerDisposition::BlockOnly, Some(true))
        );
        assert_eq!(
            materialized_pointer_disposition(
                Some(&AuthenticatedElementSemantic::PassiveSprite { one_based_frame: 1 }),
                false,
            ),
            (ReplayPointerDisposition::PassThrough, None)
        );
    }

    #[test]
    fn materialized_viewer_actions_follow_presented_hover_and_press_lifecycle() {
        use crate::runtime_graphics::ButtonVisualState;

        let action = ReplayViewerAction::Theatre(TheatreViewerAction::OpenLandSettings(41));
        let identity = MaterializedPointerActionIdentity {
            window_id: "theatre_selection_window".into(),
            runtime_instance_token: "instance-7".into(),
            action: action.clone(),
        };
        let mut lifecycle = MaterializedPointerLifecycle {
            hovered: Some(identity.clone()),
            pressed: None,
        };
        assert_eq!(
            materialized_button_interaction(
                "theatre_selection_window",
                "instance-7",
                &action,
                &lifecycle,
            ),
            ButtonVisualState::Over
        );
        lifecycle.pressed = Some(identity);
        assert_eq!(
            materialized_button_interaction(
                "theatre_selection_window",
                "instance-7",
                &action,
                &lifecycle,
            ),
            ButtonVisualState::Down
        );
        assert_eq!(
            materialized_button_interaction(
                "theatre_selection_window",
                "stale-instance",
                &action,
                &lifecycle,
            ),
            ButtonVisualState::Up
        );
    }

    #[test]
    fn viewer_action_digest_tokens_are_stable_and_typed() {
        assert_eq!(
            ReplayViewerAction::Politics(PoliticsViewerAction::RefuseGameplayMutation(
                PoliticsGameplayMutation::AssignOrReplaceIdea,
            ))
            .digest_record(),
            b"politics:refuse-assign-or-replace-idea"
        );
        assert_eq!(
            ReplayViewerAction::Theatre(TheatreViewerAction::OpenLandSettings(41)).digest_record(),
            b"theatre:open-land-settings:41"
        );
    }

    #[test]
    fn observer_input_checkpoint_restores_event_state_without_resurrecting_capture_loss() {
        let mut manager = ReplayOwnedWindowManager::new(&registry()).unwrap();
        let target = ReplayPointerTarget {
            window_id: "interface/test.gui#00000".into(),
            runtime_instance_token: "instance-1".into(),
            source_file: "interface/test.gui".into(),
            surface_id: "test".into(),
            element_id: "slider".into(),
            authored_name: "slider".into(),
            child_path: vec![0],
            global_root_sibling: false,
            element_type: "extendedScrollbarSlider".into(),
            screen_rect: Rect::new(0, 0, 20, 20),
            clip_rect: Rect::new(0, 0, 20, 20),
            stack_ordinal: 0,
            paint_ordinal: 0,
            disposition: ReplayPointerDisposition::Interactive,
            visual_enabled: Some(true),
        };
        manager.pointer_position = Some([4, 5]);
        manager.hovered = Some(target.clone());
        manager.pressed = Some(target.clone());
        manager.focus = Some(ReplayFocusTarget {
            window_id: target.window_id.clone(),
            widget_id: "slider".into(),
        });
        manager.input_transition_generation = 7;
        manager.active_input_transition = Some(ActiveReplayInputTransition {
            generation: 7,
            window_id: target.window_id.clone(),
            runtime_instance_token: target.runtime_instance_token.clone(),
            child_path: target.child_path.clone(),
        });
        let mut checkpoint = manager.input_checkpoint().unwrap();

        manager.pointer_position = Some([90, 91]);
        manager.hovered = None;
        manager.focus = None;
        checkpoint.apply_capture_loss();
        manager.cancel_pointer_capture().unwrap();
        manager.restore_input_checkpoint(checkpoint).unwrap();

        assert_eq!(manager.pointer_position, Some([4, 5]));
        assert_eq!(manager.hovered, Some(target));
        assert_eq!(
            manager.focus,
            Some(ReplayFocusTarget {
                window_id: "interface/test.gui#00000".into(),
                widget_id: "slider".into(),
            })
        );
        assert!(manager.pressed.is_none());
        assert!(manager.active_input_transition.is_none());
        assert_eq!(manager.consumed_input_transition_generation, 7);
    }

    #[test]
    fn post_resize_checkpoint_cannot_resurrect_old_presented_input_geometry() {
        let mut manager = ReplayOwnedWindowManager::new(&registry()).unwrap();
        let target = ReplayPointerTarget {
            window_id: "interface/test.gui#00000".into(),
            runtime_instance_token: "old-size-instance".into(),
            source_file: "interface/test.gui".into(),
            surface_id: "test".into(),
            element_id: "slider".into(),
            authored_name: "slider".into(),
            child_path: vec![0],
            global_root_sibling: false,
            element_type: "extendedScrollbarSlider".into(),
            screen_rect: Rect::new(0, 0, 20, 20),
            clip_rect: Rect::new(0, 0, 20, 20),
            stack_ordinal: 0,
            paint_ordinal: 0,
            disposition: ReplayPointerDisposition::Interactive,
            visual_enabled: Some(true),
        };
        manager.pointer_position = Some([4, 5]);
        manager.hovered = Some(target.clone());
        manager.pressed = Some(target);
        let pre_resize = manager.input_checkpoint().unwrap();

        manager.invalidate_composed_frame();
        let post_resize = manager.input_checkpoint().unwrap();
        assert!(manager.hovered.is_none());
        assert!(manager.pressed.is_none());

        manager.restore_input_checkpoint(pre_resize).unwrap();
        assert!(manager.hovered.is_some());
        manager.restore_input_checkpoint(post_resize).unwrap();
        assert!(manager.hovered.is_none());
        assert!(manager.pressed.is_none());
        assert!(manager.geometry.is_empty());
        assert!(manager.persistent_geometry.is_empty());
        assert_eq!(manager.committed_frame_generation, 0);
    }

    fn registry() -> DormantWindowManagerRegistry {
        DormantWindowManagerRegistry {
            windows: vec![DormantWindowDefinition {
                window_id: "interface/test.gui#00000".to_string(),
                source_file: "interface/test.gui".to_string(),
                element_type: "windowType".to_string(),
                name: "test".to_string(),
                hierarchy_path: "test".to_string(),
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
        }
    }

    fn activation(window_id: &str) -> ReplayPanelActivation {
        ReplayPanelActivation {
            window_id: window_id.to_string(),
            modal_owner: None,
            initial_focus: None,
            keyboard_traversal: Vec::new(),
            controller_traversal: Vec::new(),
            tooltip_targets: Vec::new(),
        }
    }

    fn two_window_registry() -> DormantWindowManagerRegistry {
        let mut registry = registry();
        let mut second = registry.windows[0].clone();
        second.window_id = "interface/test.gui#00001".to_string();
        second.name = "test_modal".to_string();
        second.hierarchy_path = "test_modal".to_string();
        registry.windows.push(second);
        registry
    }

    #[test]
    fn authenticated_viewer_action_consumes_one_input_edge_once() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&id)).unwrap();
        let runtime_instance_token = manager.runtime_instance_token(&id).unwrap().to_string();
        manager.committed_frame_generation = 9;
        manager.input_transition_generation = 3;
        manager.consumed_input_transition_generation = 3;
        let canonical = AuthenticatedReplayViewerAction {
            action: ReplayViewerAction::Politics(PoliticsViewerAction::ClosePolitics),
            window_id: id.clone(),
            runtime_instance_token,
            frame_generation: 9,
            input_transition_generation: 0,
            rendered_child_path: vec![0],
        };
        manager
            .canonical_viewer_actions
            .insert((id, vec![0]), canonical.clone());
        let mut released = canonical;
        released.input_transition_generation = 3;

        assert_eq!(
            manager
                .consume_authenticated_viewer_action(&released)
                .unwrap(),
            ReplayViewerAction::Politics(PoliticsViewerAction::ClosePolitics)
        );
        assert!(matches!(
            manager.consume_authenticated_viewer_action(&released),
            Err(ReplayWindowManagerRejection::InvalidPointerState(_))
        ));
    }

    fn install_button_geometry(manager: &mut ReplayOwnedWindowManager, id: &str) {
        install_button_geometry_at(manager, id, Rect::new(0, 0, 40, 20), false);
    }

    fn install_button_geometry_at(
        manager: &mut ReplayOwnedWindowManager,
        id: &str,
        rect: Rect,
        include_scroll: bool,
    ) {
        let token = manager.runtime_instance_token(id).unwrap().to_string();
        manager
            .install_composed_geometry(ReplayComposedPanelGeometry {
                window_id: id.to_string(),
                runtime_instance_token: token.clone(),
                source_file: "interface/test.gui".to_string(),
                surface_id: "test".to_string(),
                screen_rect: rect,
                clip_rect: rect,
                geometry_source_record: "sealed-test-layout".to_string(),
                element_states: vec![ReplayComposedElementState {
                    window_id: id.to_string(),
                    runtime_instance_token: token.clone(),
                    source_file: "interface/test.gui".to_string(),
                    surface_id: "test".to_string(),
                    element_id: "test.button".to_string(),
                    authored_name: "button".to_string(),
                    child_path: vec![0],
                    global_root_sibling: false,
                    element_type: "buttonType".to_string(),
                    paint_ordinal: Some(0),
                    screen_rect: Some(rect),
                    clip_rect: Some(rect),
                    disposition: ReplayPointerDisposition::Interactive,
                    visible: true,
                    visual_enabled: Some(true),
                }],
                hit_targets: vec![ReplayHitTarget {
                    window_id: id.to_string(),
                    runtime_instance_token: token.clone(),
                    source_file: "interface/test.gui".to_string(),
                    surface_id: "test".to_string(),
                    element_id: "test.button".to_string(),
                    authored_name: "button".to_string(),
                    child_path: vec![0],
                    global_root_sibling: false,
                    element_type: "buttonType".to_string(),
                    screen_rect: rect,
                    clip_rect: rect,
                    paint_ordinal: 0,
                    disposition: ReplayPointerDisposition::Interactive,
                    visual_enabled: Some(true),
                }],
                scroll_regions: include_scroll
                    .then(|| ReplayScrollRegion {
                        window_id: id.to_string(),
                        runtime_instance_token: token.clone(),
                        element_id: "test.button".to_string(),
                        child_path: vec![0],
                        viewport_rect: rect,
                        axis: ReplayScrollAxis::Vertical,
                        min_offset_pixels: -100,
                        max_offset_pixels: 0,
                        offset_pixels: 0,
                        wheel_step_pixels: None,
                        smooth_scrolling: false,
                        extended_scrollbar: None,
                        geometry_source_record: "sealed-test-scroll".to_string(),
                    })
                    .into_iter()
                    .collect(),
            })
            .unwrap();
    }

    fn install_authenticated_tooltip_context(
        manager: &mut ReplayOwnedWindowManager,
        id: &str,
        frame_generation: u64,
    ) -> AuthenticatedReplayTooltipOwner {
        install_button_geometry(manager, id);
        manager.committed_frame_generation = frame_generation;
        manager.tooltip_selectors.insert(
            (id.to_string(), "test.button".to_string()),
            ReplayTooltipAlternateSelection::AuthoredDefault,
        );
        manager.pointer_move(1, 1).unwrap();
        manager
            .authenticated_tooltip_owner_at_cursor()
            .unwrap()
            .expect("authenticated tooltip owner")
    }

    #[test]
    fn tooltip_lifetime_requires_opaque_current_owner_and_clears_on_focus_transfer() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        let mut panel_activation = activation(&id);
        panel_activation.initial_focus = Some("test.button".to_string());
        panel_activation.keyboard_traversal = vec!["test.button".to_string()];
        panel_activation.tooltip_targets = vec!["test.button".to_string()];
        manager.open(panel_activation).unwrap();
        let owner = install_authenticated_tooltip_context(&mut manager, &id, 9);

        manager
            .set_authenticated_tooltip_owner(&owner, "A26.tooltip_instances[0]")
            .unwrap();
        assert_eq!(
            manager.tooltip().map(|tooltip| tooltip.widget_id.as_str()),
            Some("test.button")
        );
        manager.focus_widget(&id, "test.button").unwrap();
        assert!(manager.tooltip().is_none());

        manager
            .set_authenticated_tooltip_owner(&owner, "A26.tooltip_instances[1]")
            .unwrap();
        manager.close(&id).unwrap();
        assert!(manager.tooltip().is_none());

        let mut reopened = activation(&id);
        reopened.tooltip_targets = vec!["test.button".to_string()];
        manager.open(reopened).unwrap();
        let _current = install_authenticated_tooltip_context(&mut manager, &id, 10);
        assert!(matches!(
            manager.set_authenticated_tooltip_owner(&owner, "A26.tooltip_instances[2]"),
            Err(ReplayWindowManagerRejection::InvalidTooltipOwner(window_id))
                if window_id == id
        ));
    }

    #[test]
    fn authored_wheel_factor_converts_emitted_notches_and_clamps() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&id)).unwrap();
        install_button_geometry_at(&mut manager, &id, Rect::new(0, 0, 100, 100), true);
        {
            let region = &mut manager.geometry.get_mut(&id).unwrap().scroll_regions[0];
            region.min_offset_pixels = 0;
            region.max_offset_pixels = 100;
            region.offset_pixels = 50;
            region.wheel_step_pixels = Some(40);
            region.smooth_scrolling = true;
        }

        let up = manager
            .scroll_wheel_notches_at(10, 10, 1)
            .unwrap()
            .expect("authenticated wheel target");
        assert_eq!(up.prior_offset_pixels, 50);
        assert_eq!(up.offset_pixels, 10);

        let down = manager
            .scroll_wheel_notches_at(10, 10, -3)
            .unwrap()
            .expect("authenticated wheel target");
        assert_eq!(down.prior_offset_pixels, 10);
        assert_eq!(down.offset_pixels, 100);
    }

    #[test]
    fn persistent_hud_wheel_region_is_reachable_without_viewer_input_owner() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&id)).unwrap();
        install_button_geometry_at(&mut manager, &id, Rect::new(0, 0, 100, 100), true);
        let mut geometry = manager.geometry.remove(&id).unwrap();
        geometry.scroll_regions[0].min_offset_pixels = 0;
        geometry.scroll_regions[0].max_offset_pixels = 100;
        geometry.scroll_regions[0].offset_pixels = 50;
        geometry.scroll_regions[0].wheel_step_pixels = Some(40);
        manager
            .persistent_instances
            .insert(id.clone(), geometry.runtime_instance_token.clone());
        manager.stack.clear();
        let panel = manager.panels.get_mut(&id).unwrap();
        panel.runtime_instance_token = None;
        panel.open = false;
        panel.visible = false;
        panel.composed = false;
        panel.stack_ordinal = None;
        manager.persistent_geometry.push(geometry);

        assert!(manager.input_window_id().is_none());
        let outcome = manager
            .scroll_wheel_notches_at(10, 10, 1)
            .unwrap()
            .expect("persistent HUD wheel target");
        assert_eq!(outcome.prior_offset_pixels, 50);
        assert_eq!(outcome.offset_pixels, 10);
    }

    #[test]
    fn composed_geometry_rejects_materialized_placement_identity_mismatch() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&id)).unwrap();
        install_button_geometry(&mut manager, &id);
        let mut geometry = manager.geometry.remove(&id).unwrap();
        geometry.hit_targets[0].global_root_sibling = true;
        assert!(matches!(
            manager.install_composed_geometry(geometry),
            Err(ReplayWindowManagerRejection::InvalidComposedGeometry(_))
        ));
    }

    #[test]
    fn tooltip_owner_is_unavailable_without_exact_selector_and_target_census() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&id)).unwrap();
        install_button_geometry(&mut manager, &id);
        manager.committed_frame_generation = 9;
        manager.pointer_move(1, 1).unwrap();
        assert!(manager
            .authenticated_tooltip_owner_at_cursor()
            .unwrap()
            .is_none());
    }

    #[test]
    fn modal_backdrop_blocks_lower_pointer_input_without_invented_hit_geometry() {
        let registry = two_window_registry();
        let owner_id = registry.windows[0].window_id.clone();
        let modal_id = registry.windows[1].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&owner_id)).unwrap();
        install_button_geometry(&mut manager, &owner_id);

        let mut modal_activation = activation(&modal_id);
        modal_activation.modal_owner = Some(owner_id.clone());
        manager.open(modal_activation).unwrap();
        install_button_geometry_at(&mut manager, &modal_id, Rect::new(10, 10, 40, 20), false);

        assert!(manager.blocks_lower_pointer_input_at(100, 100).unwrap());
        assert!(manager.pointer_target_at(100, 100).unwrap().is_none());
        manager.close(&modal_id).unwrap();
        assert!(!manager.blocks_lower_pointer_input_at(100, 100).unwrap());
    }

    #[test]
    fn raised_global_materialized_target_matches_post_panel_paint_order() {
        let registry = two_window_registry();
        let global_owner = registry.windows[0].window_id.clone();
        let top_panel = registry.windows[1].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&global_owner)).unwrap();
        install_button_geometry(&mut manager, &global_owner);
        {
            let geometry = manager.geometry.get_mut(&global_owner).unwrap();
            let global_path = vec![u32::MAX, 0, 0];
            geometry.element_states[0].child_path = global_path.clone();
            geometry.element_states[0].global_root_sibling = true;
            geometry.hit_targets[0].child_path = global_path;
            geometry.hit_targets[0].global_root_sibling = true;
        }
        manager.open(activation(&top_panel)).unwrap();
        install_button_geometry(&mut manager, &top_panel);

        let target = manager
            .pointer_target_at(5, 5)
            .unwrap()
            .expect("raised global-root target");
        assert_eq!(target.window_id, global_owner);
        assert_eq!(target.child_path.first(), Some(&u32::MAX));
        assert!(target.global_root_sibling);
        assert_eq!(
            manager.scroll_at(
                5,
                5,
                ReplayScrollDeltaPixels::from_authenticated_family(-10),
            ),
            Err(ReplayWindowManagerRejection::ScrollTargetUnavailable)
        );
    }

    #[test]
    fn nested_materialized_target_stays_below_later_panel_and_modal_roots() {
        for modal in [false, true] {
            let registry = two_window_registry();
            let bottom = registry.windows[0].window_id.clone();
            let top = registry.windows[1].window_id.clone();
            let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
            manager.open(activation(&bottom)).unwrap();
            install_button_geometry(&mut manager, &bottom);
            {
                let geometry = manager.geometry.get_mut(&bottom).unwrap();
                let materialized_path = vec![u32::MAX, 0, 0];
                geometry.element_states[0].child_path = materialized_path.clone();
                geometry.element_states[0].global_root_sibling = false;
                geometry.hit_targets[0].child_path = materialized_path;
                geometry.hit_targets[0].global_root_sibling = false;
            }
            let mut top_activation = activation(&top);
            if modal {
                top_activation.modal_owner = Some(bottom.clone());
            }
            manager.open(top_activation).unwrap();
            install_button_geometry(&mut manager, &top);

            let target = manager
                .pointer_target_at(5, 5)
                .unwrap()
                .expect("later panel target");
            assert_eq!(target.window_id, top);
            assert!(!target.global_root_sibling);
        }
    }

    #[test]
    fn lease_currentness_is_bound_to_exact_manager_lineage_and_open_generation() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&id)).unwrap();
        let first_token = {
            let lease = manager.authenticated_runtime_instance(&id).unwrap();
            assert!(lease.remains_current());
            lease.runtime_instance_token().to_string()
        };
        manager.close(&id).unwrap();
        manager.open(activation(&id)).unwrap();
        let stale = AuthenticatedReplayPanelLease {
            manager: &manager,
            window_id: id.clone(),
            runtime_instance_token: first_token,
        };
        assert!(!stale.remains_current());
        assert!(manager
            .authenticated_runtime_instance(&id)
            .is_some_and(|lease| lease.remains_current()));

        let mut independent = ReplayOwnedWindowManager::new(&registry).unwrap();
        independent.open(activation(&id)).unwrap();
        let cross_manager = AuthenticatedReplayPanelLease {
            manager: &manager,
            window_id: id,
            runtime_instance_token: independent
                .authenticated_runtime_instance("interface/test.gui#00000")
                .unwrap()
                .runtime_instance_token()
                .to_string(),
        };
        assert!(!cross_manager.remains_current());
    }

    #[test]
    fn frame_authority_is_unique_abortable_cross_manager_safe_and_clock_monotonic() {
        let registry = registry();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        let first = manager.stage_frame_authority(100).unwrap();
        assert!(first.remains_staged_for(&manager));
        assert!(matches!(
            manager.stage_frame_authority(100),
            Err(ReplayWindowManagerRejection::InvalidPointerState(reason))
                if reason == "frame-generation-already-staged"
        ));
        manager.abort_frame_authority(first).unwrap();
        assert_eq!(manager.committed_frame_generation, 0);
        assert!(manager.canonical_widget_states.is_empty());

        let committed = manager.stage_frame_authority(101).unwrap();
        let generation = committed.frame_generation();
        assert_eq!(generation, 2);
        manager
            .commit_staged_canonical_states(StagedReplayCanonicalStates {
                frame_authority: committed,
                states: BTreeMap::new(),
                viewer_actions: BTreeMap::new(),
            })
            .unwrap();
        assert_eq!(manager.committed_frame_generation, generation);
        assert_eq!(manager.canonical_widget_state_generation, generation);
        assert_eq!(manager.last_viewer_clock_micros, Some(101));
        assert!(matches!(
            manager.stage_frame_authority(100),
            Err(ReplayWindowManagerRejection::InvalidPointerState(reason))
                if reason == "viewer-clock-rollback"
        ));

        let mut independent = ReplayOwnedWindowManager::new(&registry).unwrap();
        let foreign = independent.stage_frame_authority(101).unwrap();
        assert_eq!(
            manager.commit_staged_canonical_states(StagedReplayCanonicalStates {
                frame_authority: foreign,
                states: BTreeMap::new(),
                viewer_actions: BTreeMap::new(),
            }),
            Err(ReplayWindowManagerRejection::InvalidPointerState(
                "stale-frame-generation-authority".to_string()
            ))
        );
    }

    #[test]
    fn aborted_or_transition_invalidated_frame_generation_never_resurrects() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        let first = manager.stage_frame_authority(10).unwrap();
        let stale_after_abort = AuthenticatedReplayFrameAuthority {
            manager_namespace_ordinal: first.manager_namespace_ordinal,
            frame_generation: first.frame_generation,
            viewer_clock_micros: first.viewer_clock_micros,
        };
        manager.abort_frame_authority(first).unwrap();
        let second = manager.stage_frame_authority(10).unwrap();
        assert_eq!(second.frame_generation(), 2);
        assert!(!stale_after_abort.remains_staged_for(&manager));

        manager.open(activation(&id)).unwrap();
        let third = manager.stage_frame_authority(10).unwrap();
        assert_eq!(third.frame_generation(), 3);
        assert!(!second.remains_staged_for(&manager));
        assert!(third.remains_staged_for(&manager));
    }

    #[test]
    fn press_and_release_authorities_are_edge_bound_one_shot_and_cancel_on_close() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&id)).unwrap();
        install_button_geometry(&mut manager, &id);

        let (pressed, press_authority) = manager.canonical_pointer_press(5, 5).unwrap();
        assert_eq!(pressed.as_ref().unwrap().element_id, "test.button");
        let press_authority = press_authority.expect("interactive edge authority");
        assert!(press_authority.remains_active_for(&manager));
        assert!(matches!(
            manager.canonical_pointer_press(5, 5),
            Err(ReplayWindowManagerRejection::InvalidPointerState(reason))
                if reason == "primary-pointer-capture-already-active"
        ));

        let (release, release_authority) = manager.canonical_pointer_release(5, 5).unwrap();
        assert!(!release.activated);
        let release_authority = release_authority.expect("matching one-shot release");
        assert!(release_authority.matches_consumed_generation(&manager));
        assert_eq!(
            release_authority.input_transition_generation(),
            press_authority.input_transition_generation()
        );
        assert!(!press_authority.remains_active_for(&manager));

        let (repeated, repeated_authority) = manager.canonical_pointer_release(5, 5).unwrap();
        assert!(!repeated.activated);
        assert!(repeated_authority.is_none());

        let (_, second_press) = manager.canonical_pointer_press(5, 5).unwrap();
        assert!(second_press.is_some());
        let staged = manager.stage_frame_authority(200).unwrap();
        manager.close(&id).unwrap();
        assert!(manager.active_input_transition.is_none());
        assert!(manager.staged_frame_generation.is_none());
        assert!(!staged.remains_staged_for(&manager));
    }

    #[test]
    fn block_only_occluder_consumes_press_without_requiring_interactive_state() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&id)).unwrap();
        install_button_geometry(&mut manager, &id);
        let geometry = manager.geometry.get_mut(&id).unwrap();
        geometry.element_states[0].disposition = ReplayPointerDisposition::BlockOnly;
        geometry.hit_targets[0].disposition = ReplayPointerDisposition::BlockOnly;

        assert_eq!(manager.pointer_press(5, 5).unwrap(), None);
        assert!(manager.pressed.is_none());
        assert!(manager.active_input_transition.is_none());
    }

    #[test]
    fn active_modal_blocks_pointer_and_scroll_fallthrough_outside_its_clip() {
        let mut registry = registry();
        registry.windows.push(DormantWindowDefinition {
            window_id: "interface/test.gui#00001".to_string(),
            source_file: "interface/test.gui".to_string(),
            element_type: "windowType".to_string(),
            name: "modal".to_string(),
            hierarchy_path: "modal".to_string(),
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
        });
        let owner = registry.windows[0].window_id.clone();
        let modal = registry.windows[1].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&owner)).unwrap();
        install_button_geometry_at(&mut manager, &owner, Rect::new(0, 0, 100, 100), true);
        let mut modal_activation = activation(&modal);
        modal_activation.modal_owner = Some(owner.clone());
        manager.open(modal_activation).unwrap();
        install_button_geometry_at(&mut manager, &modal, Rect::new(200, 200, 50, 50), false);

        assert_eq!(manager.pointer_target_at(10, 10).unwrap(), None);
        assert_eq!(
            manager.scroll_at(
                10,
                10,
                ReplayScrollDeltaPixels::from_authenticated_family(-1),
            ),
            Err(ReplayWindowManagerRejection::ScrollTargetUnavailable)
        );
        assert_eq!(
            manager
                .pointer_target_at(210, 210)
                .unwrap()
                .expect("modal target")
                .window_id,
            modal
        );
    }

    #[test]
    fn clipped_top_target_does_not_block_scroll_outside_its_screen_rect() {
        let mut registry = registry();
        registry.windows.push(DormantWindowDefinition {
            window_id: "interface/test.gui#00001".to_string(),
            source_file: "interface/test.gui".to_string(),
            element_type: "windowType".to_string(),
            name: "top".to_string(),
            hierarchy_path: "top".to_string(),
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
        });
        let bottom = registry.windows[0].window_id.clone();
        let top = registry.windows[1].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&bottom)).unwrap();
        install_button_geometry_at(&mut manager, &bottom, Rect::new(0, 0, 100, 100), true);
        manager.open(activation(&top)).unwrap();
        install_button_geometry_at(&mut manager, &top, Rect::new(80, 80, 20, 20), false);
        let top_geometry = manager.geometry.get_mut(&top).unwrap();
        top_geometry.screen_rect = Rect::new(0, 0, 100, 100);
        top_geometry.clip_rect = Rect::new(0, 0, 100, 100);
        top_geometry.element_states[0].clip_rect = Some(Rect::new(0, 0, 100, 100));
        top_geometry.hit_targets[0].clip_rect = Rect::new(0, 0, 100, 100);

        let outcome = manager
            .scroll_at(
                10,
                10,
                ReplayScrollDeltaPixels::from_authenticated_family(-1),
            )
            .unwrap();
        assert_eq!(outcome.window_id, bottom);
        assert_eq!(outcome.offset_pixels, -1);
    }

    #[test]
    fn persistent_scroll_region_updates_its_manager_owned_geometry() {
        let registry = registry();
        let id = registry.windows[0].window_id.clone();
        let mut manager = ReplayOwnedWindowManager::new(&registry).unwrap();
        manager.open(activation(&id)).unwrap();
        install_button_geometry_at(&mut manager, &id, Rect::new(0, 0, 100, 100), true);
        let geometry = manager.geometry.remove(&id).unwrap();
        manager
            .persistent_instances
            .insert(id.clone(), geometry.runtime_instance_token.clone());
        manager.stack.clear();
        let panel = manager.panels.get_mut(&id).unwrap();
        panel.runtime_instance_token = None;
        panel.open = false;
        panel.visible = false;
        panel.composed = false;
        panel.stack_ordinal = None;
        manager.persistent_geometry.push(geometry);

        let outcome = manager
            .scroll_at(
                10,
                10,
                ReplayScrollDeltaPixels::from_authenticated_family(-7),
            )
            .unwrap();
        assert_eq!(outcome.window_id, id);
        assert_eq!(outcome.offset_pixels, -7);
        assert_eq!(
            manager.persistent_geometry[0].scroll_regions[0].offset_pixels,
            -7
        );
    }

    /// kills: a proven row-text store that is not actually in the frozen
    /// collection law, and a group claiming one whose family the contract does
    /// not declare. The store is what turns a carrier's `row_text` into a
    /// drawn string, so a wrong or invented one would put text on an element
    /// a729 never writes.
    #[test]
    fn exactly_one_collection_group_proves_a_row_text_store() {
        let proven = XUI26_COLLECTION_LAWS
            .iter()
            .filter_map(|law| {
                law.row_text_authored_name
                    .map(|name| (law.family_id, law.group_id, name))
            })
            .collect::<Vec<_>>();
        assert_eq!(proven, vec![("hotjoin", "hotjoin_requests", "name")]);
        // Each law is reached by its (family, group) pair, so two laws sharing
        // one pair would make `xui26_collection_law` return whichever came
        // first and silently apply the wrong store rule to the other group.
        let pairs = XUI26_COLLECTION_LAWS
            .iter()
            .map(|law| (law.family_id, law.group_id))
            .collect::<BTreeSet<_>>();
        assert_eq!(pairs.len(), XUI26_COLLECTION_LAWS.len());
    }
}

fn snapshot_geometry(
    geometry: &ReplayComposedPanelGeometry,
    stack_ordinal: u32,
    modal_owner: Option<String>,
    owns_input: bool,
    focused_widget: Option<&str>,
    hovered: Option<&ReplayPointerTarget>,
    pressed: Option<&ReplayPointerTarget>,
) -> ReplayComposedWindowSnapshot {
    ReplayComposedWindowSnapshot {
        window_id: geometry.window_id.clone(),
        runtime_instance_token: geometry.runtime_instance_token.clone(),
        source_file: geometry.source_file.clone(),
        surface_id: geometry.surface_id.clone(),
        stack_ordinal,
        modal_owner,
        owns_input,
        focused_widget: focused_widget.map(str::to_string),
        screen_rect: geometry.screen_rect,
        clip_rect: geometry.clip_rect,
        geometry_source_record: geometry.geometry_source_record.clone(),
        hit_targets: geometry.hit_targets.clone(),
        element_interactions: geometry
            .element_states
            .iter()
            .map(|element| ReplayElementInteractionSnapshot {
                window_id: geometry.window_id.clone(),
                runtime_instance_token: geometry.runtime_instance_token.clone(),
                source_file: element.source_file.clone(),
                surface_id: element.surface_id.clone(),
                element_id: element.element_id.clone(),
                authored_name: element.authored_name.clone(),
                child_path: element.child_path.clone(),
                global_root_sibling: element.global_root_sibling,
                element_type: element.element_type.clone(),
                stack_ordinal,
                paint_ordinal: element.paint_ordinal,
                screen_rect: element.screen_rect,
                clip_rect: element.clip_rect,
                disposition: element.disposition,
                visible: element.visible,
                visual_enabled: element.visual_enabled,
                hovered: hovered.is_some_and(|hovered| same_element_identity(hovered, element)),
                pressed: pressed.is_some_and(|pressed| same_element_identity(pressed, element)),
                focused: focused_widget == Some(element.element_id.as_str()),
            })
            .collect(),
        scroll_regions: geometry.scroll_regions.clone(),
    }
}

fn pointer_target(target: &ReplayHitTarget, stack_ordinal: u32) -> ReplayPointerTarget {
    ReplayPointerTarget {
        window_id: target.window_id.clone(),
        runtime_instance_token: target.runtime_instance_token.clone(),
        source_file: target.source_file.clone(),
        surface_id: target.surface_id.clone(),
        element_id: target.element_id.clone(),
        authored_name: target.authored_name.clone(),
        child_path: target.child_path.clone(),
        global_root_sibling: target.global_root_sibling,
        element_type: target.element_type.clone(),
        screen_rect: target.screen_rect,
        clip_rect: target.clip_rect,
        stack_ordinal,
        paint_ordinal: target.paint_ordinal,
        disposition: target.disposition,
        visual_enabled: target.visual_enabled,
    }
}

fn same_pointer_identity(pointer: &ReplayPointerTarget, target: &ReplayHitTarget) -> bool {
    pointer.window_id == target.window_id
        && pointer.runtime_instance_token == target.runtime_instance_token
        && pointer.source_file == target.source_file
        && pointer.surface_id == target.surface_id
        && pointer.element_id == target.element_id
        && pointer.authored_name == target.authored_name
        && pointer.child_path == target.child_path
        && pointer.global_root_sibling == target.global_root_sibling
}

fn same_element_identity(
    pointer: &ReplayPointerTarget,
    element: &ReplayComposedElementState,
) -> bool {
    pointer.window_id == element.window_id
        && pointer.runtime_instance_token == element.runtime_instance_token
        && pointer.source_file == element.source_file
        && pointer.surface_id == element.surface_id
        && pointer.element_id == element.element_id
        && pointer.authored_name == element.authored_name
        && pointer.child_path == element.child_path
        && pointer.global_root_sibling == element.global_root_sibling
}

fn rect_is_positive(rect: Rect) -> bool {
    rect.w() > 0 && rect.h() > 0
}

fn rect_contains(rect: Rect, x: i32, y: i32) -> Result<bool, ReplayWindowManagerRejection> {
    let right = rect
        .x()
        .checked_add(rect.w())
        .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
    let bottom = rect
        .y()
        .checked_add(rect.h())
        .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
    Ok(x >= rect.x() && x < right && y >= rect.y() && y < bottom)
}

fn rect_within(inner: Rect, outer: Rect) -> Result<bool, ReplayWindowManagerRejection> {
    let inner_right = inner
        .x()
        .checked_add(inner.w())
        .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
    let inner_bottom = inner
        .y()
        .checked_add(inner.h())
        .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
    let outer_right = outer
        .x()
        .checked_add(outer.w())
        .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
    let outer_bottom = outer
        .y()
        .checked_add(outer.h())
        .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
    Ok(inner.x() >= outer.x()
        && inner.y() >= outer.y()
        && inner_right <= outer_right
        && inner_bottom <= outer_bottom)
}

fn authenticate_viewer_actions(
    manager: &ReplayOwnedWindowManager,
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    inputs: &BTreeMap<String, &ReplayAuthenticatedPanelFrameInput<'_>>,
    preflights: &BTreeMap<String, (AuthenticatedLayoutPreflight, ReplayComposedPanelGeometry)>,
    frame_authority: &AuthenticatedReplayFrameAuthority,
) -> Result<
    BTreeMap<(String, Vec<u32>), AuthenticatedReplayViewerAction>,
    ReplayWindowManagerRejection,
> {
    if !frame_authority.remains_staged_for(manager) {
        return Err(ReplayWindowManagerRejection::InvalidPointerState(
            "stale-viewer-action-frame".to_string(),
        ));
    }
    let mut sealed = BTreeMap::new();
    for (window_id, input) in inputs {
        let surface = index
            .validate_panel_token(admission, input.panel)
            .map_err(|_| {
                ReplayWindowManagerRejection::InvalidPointerState(window_id.to_string())
            })?;
        let (preflight, _) = preflights.get(window_id).ok_or_else(|| {
            ReplayWindowManagerRejection::InvalidPointerState(window_id.to_string())
        })?;
        let lease = manager
            .authenticated_runtime_instance(window_id)
            .ok_or_else(|| {
                ReplayWindowManagerRejection::StaleRuntimeInstance(window_id.to_string())
            })?;
        let expected_interactive_paths = surface
            .elements
            .iter()
            .filter(|element| {
                matches!(
                    input.element_pointer_bindings.get(&element.element_id),
                    Some(ReplayElementPointerAuthority::Interactive { .. })
                )
            })
            .map(|element| element.child_path.clone())
            .collect::<BTreeSet<_>>();
        let mut supplied_paths = BTreeSet::new();
        for binding in input.viewer_actions {
            if binding.authored_tree_identity_sha256 != input.panel.authored_tree_identity_sha256()
                || !surface
                    .elements
                    .iter()
                    .any(|element| element.child_path == binding.authored_template_child_path)
                || !preflight
                    .elements()
                    .iter()
                    .any(|element| element.child_path() == binding.rendered_child_path.as_slice())
                || !expected_interactive_paths.contains(&binding.rendered_child_path)
                || !supplied_paths.insert(binding.rendered_child_path.clone())
            {
                return Err(ReplayWindowManagerRejection::InvalidPointerState(
                    window_id.to_string(),
                ));
            }
            let action = AuthenticatedReplayViewerAction {
                action: binding.action.clone(),
                window_id: window_id.clone(),
                runtime_instance_token: lease.runtime_instance_token().to_string(),
                frame_generation: frame_authority.frame_generation(),
                input_transition_generation: 0,
                rendered_child_path: binding.rendered_child_path.clone(),
            };
            if sealed
                .insert(
                    (window_id.clone(), binding.rendered_child_path.clone()),
                    action,
                )
                .is_some()
            {
                return Err(ReplayWindowManagerRejection::InvalidPointerState(
                    window_id.to_string(),
                ));
            }
        }
        if supplied_paths != expected_interactive_paths {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(format!(
                "{window_id}:incomplete-viewer-action-census"
            )));
        }
    }
    Ok(sealed)
}

fn authenticated_semantic_elements(
    input: &ReplayAuthenticatedPanelFrameInput<'_>,
) -> Result<BTreeMap<Vec<u32>, AuthenticatedElementSemantic>, ReplayWindowManagerRejection> {
    let roles = input.panel.element_roles();
    if input.semantics.len() != roles.len()
        || input
            .semantics
            .keys()
            .any(|element_id| !roles.iter().any(|role| role.element_id == *element_id))
    {
        return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
            input.panel.window_id().to_string(),
        ));
    }
    roles
        .iter()
        .map(|role| {
            let semantic = input.semantics.get(&role.element_id).ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(
                    input.panel.window_id().to_string(),
                )
            })?;
            let sealed = authenticated_element_semantic_from_replay(semantic);
            Ok((role.child_path.clone(), sealed))
        })
        .collect()
}

fn authenticated_element_semantic_from_replay(
    semantic: &ReplayAuthenticatedElementSemantic,
) -> AuthenticatedElementSemantic {
    match semantic {
        ReplayAuthenticatedElementSemantic::Hidden => AuthenticatedElementSemantic::Hidden,
        ReplayAuthenticatedElementSemantic::Structural => AuthenticatedElementSemantic::Structural,
        ReplayAuthenticatedElementSemantic::StandardButton => {
            AuthenticatedElementSemantic::StandardButton
        }
        ReplayAuthenticatedElementSemantic::ReadOnlyButton {
            one_based_frame,
            interaction,
        } => AuthenticatedElementSemantic::ReadOnlyButton {
            one_based_frame: *one_based_frame,
            interaction: *interaction,
        },
        ReplayAuthenticatedElementSemantic::PassiveSprite { one_based_frame } => {
            AuthenticatedElementSemantic::PassiveSprite {
                one_based_frame: *one_based_frame,
            }
        }
        ReplayAuthenticatedElementSemantic::Text { resolved_text } => {
            AuthenticatedElementSemantic::Text {
                resolved_text: resolved_text.clone(),
            }
        }
        ReplayAuthenticatedElementSemantic::Progress { fill } => {
            AuthenticatedElementSemantic::Progress { fill: *fill }
        }
        ReplayAuthenticatedElementSemantic::BlendFrames {
            current_one_based_frame,
            next_one_based_frame,
            animation_time,
        } => AuthenticatedElementSemantic::BlendFrames {
            current_one_based_frame: *current_one_based_frame,
            next_one_based_frame: *next_one_based_frame,
            animation_time: *animation_time,
        },
    }
}

fn validate_authenticated_frame_bindings(
    surface: &crate::authored_role_index::AuthoredSurfaceRole,
    input: &ReplayAuthenticatedPanelFrameInput<'_>,
) -> Result<(), ReplayWindowManagerRejection> {
    let materialization_capability_count = usize::from(input.theatre_capability.is_some())
        + usize::from(input.production_lines_capability.is_some())
        + usize::from(input.frontend_collections_capability.is_some());
    if materialization_capability_count > 1 {
        return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
            surface.root_window_id.clone(),
        ));
    }
    if let Some(binding) = input.frontend_collections_capability {
        if input.layer != ReplayAuthenticatedPanelLayer::ViewerPanel
            || binding.index_identity_sha256 != input.surface.index.index_identity_sha256
            || binding.manifest_digest_sha256 != input.surface.manifest_digest_sha256()
            || binding.product_modset_sha256 != input.surface.product_modset_sha256()
            || binding.owner_window_id != input.surface.window_id()
            || binding.viewport_size != [input.viewport_width, input.viewport_height]
            || binding.identity_sha256 != xui26_frontend_collections_binding_digest(binding)
        {
            return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                surface.root_window_id.clone(),
            ));
        }
    }
    match (input.layer, input.persistent_capability) {
        (ReplayAuthenticatedPanelLayer::PersistentHud, Some(binding))
            if binding.index_identity_sha256 == input.surface.index.index_identity_sha256
                && binding.manifest_digest_sha256 == input.surface.manifest_digest_sha256()
                && binding.product_modset_sha256 == input.surface.product_modset_sha256()
                && binding.window_id == input.surface.window_id()
                && binding.surface_id == input.surface.surface_id()
                && binding.viewport_width == input.viewport_width
                && binding.viewport_height == input.viewport_height
                && binding.identity_sha256 == persistent_hud_binding_digest(binding) => {}
        (ReplayAuthenticatedPanelLayer::ViewerPanel, None) => {}
        _ => {
            return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                surface.root_window_id.clone(),
            ))
        }
    }
    if input.element_pointer_bindings.len() != surface.elements.len()
        || surface.elements.iter().any(|element| {
            !input
                .element_pointer_bindings
                .contains_key(&element.element_id)
        })
        || input.element_pointer_bindings.keys().any(|element_id| {
            !surface
                .elements
                .iter()
                .any(|element| element.element_id == *element_id)
        })
    {
        return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
            surface.root_window_id.clone(),
        ));
    }
    match (input.layer, input.activation) {
        (ReplayAuthenticatedPanelLayer::PersistentHud, None) => {}
        (ReplayAuthenticatedPanelLayer::ViewerPanel, Some(activation))
            if activation.window_id == surface.root_window_id
                && activation.controller_traversal.is_empty()
                && activation
                    .keyboard_traversal
                    .iter()
                    .chain(&activation.controller_traversal)
                    .all(|element_id| {
                        matches!(
                            input.element_pointer_bindings.get(element_id),
                            Some(ReplayElementPointerAuthority::Interactive { .. })
                        )
                    })
                && activation.tooltip_targets.iter().all(|element_id| {
                    matches!(
                        input.element_pointer_bindings.get(element_id),
                        Some(
                            ReplayElementPointerAuthority::Interactive { .. }
                                | ReplayElementPointerAuthority::BlockOnly { .. }
                        )
                    )
                }) => {}
        _ => {
            return Err(ReplayWindowManagerRejection::InvalidTraversal(
                surface.root_window_id.clone(),
            ));
        }
    }
    let mut tooltip_ids = BTreeSet::new();
    if input.tooltip_targets.iter().any(|element_id| {
        !tooltip_ids.insert(element_id.as_str())
            || !matches!(
                input.element_pointer_bindings.get(element_id),
                Some(
                    ReplayElementPointerAuthority::Interactive { .. }
                        | ReplayElementPointerAuthority::BlockOnly { .. }
                )
            )
    }) {
        return Err(ReplayWindowManagerRejection::InvalidTooltipOwner(
            surface.root_window_id.clone(),
        ));
    }
    if input.tooltip_selectors.len() != tooltip_ids.len()
        || input
            .tooltip_selectors
            .keys()
            .any(|element_id| !tooltip_ids.contains(element_id.as_str()))
    {
        return Err(ReplayWindowManagerRejection::InvalidTooltipOwner(
            surface.root_window_id.clone(),
        ));
    }
    for element in &surface.elements {
        if matches!(
            input.element_pointer_bindings.get(&element.element_id),
            Some(ReplayElementPointerAuthority::Interactive { .. })
        ) && !WidgetFamily::from_element_type(&element.element_type)
            .is_some_and(WidgetFamily::standard_button_carrier)
        {
            return Err(ReplayWindowManagerRejection::InvalidPointerState(
                element.element_id.clone(),
            ));
        }
    }
    let mut scroll_ids = BTreeSet::new();
    for scroll in input.scroll_bindings {
        let Some(element) = surface
            .elements
            .iter()
            .find(|element| element.element_id == scroll.element_id)
        else {
            return Err(ReplayWindowManagerRejection::InvalidScrollRegion(
                scroll.element_id.clone(),
            ));
        };
        if !matches!(
            WidgetFamily::from_element_type(&element.element_type),
            Some(
                WidgetFamily::List
                    | WidgetFamily::SmoothList
                    | WidgetFamily::Grid
                    | WidgetFamily::Overlap
                    | WidgetFamily::LegacyScrollbar
                    | WidgetFamily::ExtendedScrollbar
            )
        ) || scroll.min_offset_pixels > scroll.max_offset_pixels
            || !(scroll.min_offset_pixels..=scroll.max_offset_pixels)
                .contains(&scroll.offset_pixels)
            || scroll.producer_record.trim().is_empty()
            || !scroll_ids.insert(scroll.element_id.as_str())
            || matches!(
                input.element_pointer_bindings.get(&scroll.element_id),
                Some(ReplayElementPointerAuthority::PassThrough) | None
            )
        {
            return Err(ReplayWindowManagerRejection::InvalidScrollRegion(
                scroll.element_id.clone(),
            ));
        }
    }
    Ok(())
}

fn geometry_from_authenticated_preflight(
    input: &ReplayAuthenticatedPanelFrameInput<'_>,
    lease: &AuthenticatedReplayPanelLease<'_>,
    preflight: &AuthenticatedLayoutPreflight,
    frame_generation: u64,
) -> Result<ReplayComposedPanelGeometry, ReplayWindowManagerRejection> {
    if !lease.remains_current()
        || lease.window_id() != input.panel.window_id()
        || preflight.window_id() != input.panel.window_id()
        || preflight.surface_id() != input.panel.surface_id()
        || preflight.source_content() != input.panel.source_content()
        || preflight.runtime_instance_token() != lease.runtime_instance_token()
        || preflight.frame_generation() != frame_generation
    {
        return Err(ReplayWindowManagerRejection::StaleRuntimeInstance(
            input.panel.window_id().to_string(),
        ));
    }
    let root = preflight
        .elements()
        .iter()
        .find(|element| element.child_path().is_empty())
        .ok_or_else(|| {
            ReplayWindowManagerRejection::InvalidComposedGeometry(
                input.panel.window_id().to_string(),
            )
        })?;
    let root_rect = root.screen_rect().ok_or_else(|| {
        ReplayWindowManagerRejection::InvalidComposedGeometry(input.panel.window_id().to_string())
    })?;
    let root_clip = root.clip_rect().ok_or_else(|| {
        ReplayWindowManagerRejection::InvalidComposedGeometry(input.panel.window_id().to_string())
    })?;
    let mut element_states = Vec::with_capacity(preflight.elements().len());
    let mut hit_targets = Vec::new();
    for element in preflight.elements() {
        let pointer = input
            .element_pointer_bindings
            .get(element.element_id())
            .ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(
                    input.panel.window_id().to_string(),
                )
            })?;
        let (disposition, visual_enabled) = match pointer {
            ReplayElementPointerAuthority::PassThrough => {
                (ReplayPointerDisposition::PassThrough, None)
            }
            ReplayElementPointerAuthority::BlockOnly { visual_enabled } => {
                (ReplayPointerDisposition::BlockOnly, *visual_enabled)
            }
            ReplayElementPointerAuthority::Interactive { visual_enabled } => {
                (ReplayPointerDisposition::Interactive, Some(*visual_enabled))
            }
        };
        let screen_rect = element
            .screen_rect()
            .map(|rect| Rect::new(rect.x(), rect.y(), rect.width(), rect.height()));
        let clip_rect = element
            .clip_rect()
            .map(|rect| Rect::new(rect.x(), rect.y(), rect.width(), rect.height()));
        if element.visible() && disposition != ReplayPointerDisposition::PassThrough {
            let (Some(screen_rect), Some(clip_rect), Some(paint_ordinal)) =
                (screen_rect, clip_rect, element.paint_ordinal())
            else {
                return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                    element.element_id().to_string(),
                ));
            };
            hit_targets.push(ReplayHitTarget {
                window_id: input.panel.window_id().to_string(),
                runtime_instance_token: lease.runtime_instance_token().to_string(),
                source_file: preflight
                    .source_content()
                    .normalized_game_relative_path
                    .clone(),
                surface_id: input.panel.surface_id().to_string(),
                element_id: element.element_id().to_string(),
                authored_name: element.authored_name().to_string(),
                child_path: element.child_path().to_vec(),
                global_root_sibling: false,
                element_type: element.element_type().to_string(),
                screen_rect,
                clip_rect,
                paint_ordinal,
                disposition,
                visual_enabled,
            });
        }
        element_states.push(ReplayComposedElementState {
            window_id: input.panel.window_id().to_string(),
            runtime_instance_token: lease.runtime_instance_token().to_string(),
            source_file: preflight
                .source_content()
                .normalized_game_relative_path
                .clone(),
            surface_id: input.panel.surface_id().to_string(),
            element_id: element.element_id().to_string(),
            authored_name: element.authored_name().to_string(),
            child_path: element.child_path().to_vec(),
            global_root_sibling: false,
            element_type: element.element_type().to_string(),
            paint_ordinal: element.paint_ordinal(),
            screen_rect,
            clip_rect,
            disposition,
            visible: element.visible(),
            visual_enabled,
        });
    }
    let mut scroll_regions = Vec::with_capacity(input.scroll_bindings.len());
    for scroll in input.scroll_bindings {
        let element = element_states
            .iter()
            .find(|element| element.element_id == scroll.element_id)
            .ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidScrollRegion(scroll.element_id.clone())
            })?;
        let viewport_rect = element.clip_rect.ok_or_else(|| {
            ReplayWindowManagerRejection::InvalidScrollRegion(scroll.element_id.clone())
        })?;
        scroll_regions.push(ReplayScrollRegion {
            window_id: input.panel.window_id().to_string(),
            runtime_instance_token: lease.runtime_instance_token().to_string(),
            element_id: scroll.element_id.clone(),
            child_path: element.child_path.clone(),
            viewport_rect,
            axis: scroll.axis,
            min_offset_pixels: scroll.min_offset_pixels,
            max_offset_pixels: scroll.max_offset_pixels,
            offset_pixels: scroll.offset_pixels,
            wheel_step_pixels: None,
            smooth_scrolling: false,
            extended_scrollbar: None,
            geometry_source_record: scroll.producer_record.clone(),
        });
    }
    Ok(ReplayComposedPanelGeometry {
        window_id: input.panel.window_id().to_string(),
        runtime_instance_token: lease.runtime_instance_token().to_string(),
        source_file: preflight
            .source_content()
            .normalized_game_relative_path
            .clone(),
        surface_id: input.panel.surface_id().to_string(),
        screen_rect: Rect::new(
            root_rect.x(),
            root_rect.y(),
            root_rect.width(),
            root_rect.height(),
        ),
        clip_rect: Rect::new(
            root_clip.x(),
            root_clip.y(),
            root_clip.width(),
            root_clip.height(),
        ),
        geometry_source_record: format!(
            "XUI02/index={}/window={}/instance={}/frame={}",
            preflight.index_identity_sha256(),
            input.panel.window_id(),
            lease.runtime_instance_token(),
            frame_generation
        ),
        element_states,
        hit_targets,
        scroll_regions,
    })
}

fn seal_materialized_viewer_actions(
    owner_window_id: &str,
    lease: &AuthenticatedReplayPanelLease<'_>,
    report: &AuthenticatedComposeReport,
    plan: &AuthenticatedMaterializationPlan,
    frame_generation: u64,
) -> Result<
    BTreeMap<(String, Vec<u32>), AuthenticatedReplayViewerAction>,
    ReplayWindowManagerRejection,
> {
    if report.window_id() != owner_window_id
        || report.runtime_instance_token() != lease.runtime_instance_token()
        || report.frame_generation() != frame_generation
    {
        return Err(theatre_materialization_rejection(owner_window_id));
    }
    let mut actions = BTreeMap::new();
    for instance in plan.instances() {
        let census = report
            .materialized_instances()
            .iter()
            .find(|census| {
                census.rendered_root_path().instance_ordinal() == instance.instance_ordinal()
            })
            .filter(|census| {
                census.instance_key() == instance.instance_key()
                    && census.template_tree_identity_sha256()
                        == instance.template_panel().authored_tree_identity_sha256()
            })
            .ok_or_else(|| theatre_materialization_rejection(owner_window_id))?;
        for (template_path, action) in instance.viewer_actions() {
            if !matches!(
                action,
                ReplayViewerAction::Theatre(_) | ReplayViewerAction::ProductionLines(_)
            ) {
                return Err(theatre_materialization_rejection(owner_window_id));
            }
            let rendered = census
                .elements()
                .iter()
                .find(|element| {
                    element.rendered_path().template_child_path() == template_path.as_slice()
                })
                .ok_or_else(|| theatre_materialization_rejection(owner_window_id))?;
            let element = rendered.element();
            let authored = instance
                .template_panel()
                .authored_element(template_path)
                .ok_or_else(|| theatre_materialization_rejection(owner_window_id))?;
            if !element.visible()
                || element.screen_rect().is_none()
                || !WidgetFamily::from_element_type(&authored.element_type)
                    .is_some_and(WidgetFamily::standard_button_carrier)
            {
                return Err(theatre_materialization_rejection(owner_window_id));
            }
            match (element.paint_ordinal(), element.clip_rect()) {
                (None, None) => continue,
                (Some(_), Some(_)) => {}
                _ => return Err(theatre_materialization_rejection(owner_window_id)),
            }
            let mut rendered_path = Vec::with_capacity(template_path.len() + 2);
            rendered_path.push(u32::MAX);
            rendered_path.push(instance.instance_ordinal());
            rendered_path.extend_from_slice(template_path);
            let sealed = AuthenticatedReplayViewerAction {
                action: action.clone(),
                window_id: owner_window_id.to_string(),
                runtime_instance_token: lease.runtime_instance_token().to_string(),
                frame_generation,
                input_transition_generation: 0,
                rendered_child_path: rendered_path.clone(),
            };
            if actions
                .insert((owner_window_id.to_string(), rendered_path), sealed)
                .is_some()
            {
                return Err(theatre_materialization_rejection(owner_window_id));
            }
        }
    }
    Ok(actions)
}

fn geometry_from_authenticated_compose_report(
    input: &ReplayAuthenticatedPanelFrameInput<'_>,
    lease: &AuthenticatedReplayPanelLease<'_>,
    report: &AuthenticatedComposeReport,
    plan: Option<&AuthenticatedMaterializationPlan>,
    materialized_actions: &BTreeMap<(String, Vec<u32>), AuthenticatedReplayViewerAction>,
) -> Result<ReplayComposedPanelGeometry, ReplayWindowManagerRejection> {
    if !lease.remains_current()
        || report.window_id() != input.panel.window_id()
        || report.surface_id() != input.panel.surface_id()
        || report.source_content() != input.panel.source_content()
        || report.runtime_instance_token() != lease.runtime_instance_token()
    {
        return Err(ReplayWindowManagerRejection::StaleRuntimeInstance(
            input.panel.window_id().to_string(),
        ));
    }
    let root = report
        .elements()
        .iter()
        .find(|element| element.child_path().is_empty())
        .ok_or_else(|| {
            ReplayWindowManagerRejection::InvalidComposedGeometry(
                input.panel.window_id().to_string(),
            )
        })?;
    let root_rect = root
        .screen_rect()
        .ok_or_else(|| theatre_materialization_rejection(input.panel.window_id()))?;
    let root_clip = root
        .clip_rect()
        .ok_or_else(|| theatre_materialization_rejection(input.panel.window_id()))?;
    let mut element_states = Vec::new();
    let mut hit_targets = Vec::new();
    for element in report.elements() {
        let pointer = input
            .element_pointer_bindings
            .get(element.element_id())
            .ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(
                    input.panel.window_id().to_string(),
                )
            })?;
        let (disposition, visual_enabled) = match pointer {
            ReplayElementPointerAuthority::PassThrough => {
                (ReplayPointerDisposition::PassThrough, None)
            }
            ReplayElementPointerAuthority::BlockOnly { visual_enabled } => {
                (ReplayPointerDisposition::BlockOnly, *visual_enabled)
            }
            ReplayElementPointerAuthority::Interactive { visual_enabled } => {
                (ReplayPointerDisposition::Interactive, Some(*visual_enabled))
            }
        };
        let screen_rect = element
            .screen_rect()
            .map(|rect| Rect::new(rect.x(), rect.y(), rect.width(), rect.height()));
        let clip_rect = element
            .clip_rect()
            .map(|rect| Rect::new(rect.x(), rect.y(), rect.width(), rect.height()));
        let child_path = element.child_path().to_vec();
        if element.visible() && disposition != ReplayPointerDisposition::PassThrough {
            let (Some(screen_rect), Some(clip_rect), Some(paint_ordinal)) =
                (screen_rect, clip_rect, element.paint_ordinal())
            else {
                return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                    element.element_id().to_string(),
                ));
            };
            hit_targets.push(ReplayHitTarget {
                window_id: input.panel.window_id().to_string(),
                runtime_instance_token: lease.runtime_instance_token().to_string(),
                source_file: report
                    .source_content()
                    .normalized_game_relative_path
                    .clone(),
                surface_id: report.surface_id().to_string(),
                element_id: element.element_id().to_string(),
                authored_name: element.authored_name().to_string(),
                child_path: child_path.clone(),
                global_root_sibling: false,
                element_type: element.element_type().to_string(),
                screen_rect,
                clip_rect,
                paint_ordinal,
                disposition,
                visual_enabled,
            });
        }
        element_states.push(ReplayComposedElementState {
            window_id: input.panel.window_id().to_string(),
            runtime_instance_token: lease.runtime_instance_token().to_string(),
            source_file: report
                .source_content()
                .normalized_game_relative_path
                .clone(),
            surface_id: report.surface_id().to_string(),
            element_id: element.element_id().to_string(),
            authored_name: element.authored_name().to_string(),
            child_path,
            global_root_sibling: false,
            element_type: element.element_type().to_string(),
            paint_ordinal: element.paint_ordinal(),
            screen_rect,
            clip_rect,
            disposition,
            visible: element.visible(),
            visual_enabled,
        });
    }
    if let Some(plan) = plan {
        for census in report.materialized_instances() {
            let instance = plan
                .instances()
                .get(
                    usize::try_from(census.rendered_root_path().instance_ordinal())
                        .map_err(|_| theatre_materialization_rejection(input.panel.window_id()))?,
                )
                .filter(|instance| {
                    instance.instance_ordinal() == census.rendered_root_path().instance_ordinal()
                        && instance.instance_key() == census.instance_key()
                })
                .ok_or_else(|| theatre_materialization_rejection(input.panel.window_id()))?;
            for rendered in census.elements() {
                let element = rendered.element();
                let mut child_path =
                    Vec::with_capacity(rendered.rendered_path().template_child_path().len() + 2);
                child_path.push(u32::MAX);
                child_path.push(instance.instance_ordinal());
                child_path.extend_from_slice(rendered.rendered_path().template_child_path());
                let interactive = materialized_actions
                    .contains_key(&(input.panel.window_id().to_string(), child_path.clone()));
                let (disposition, visual_enabled) =
                    if element.paint_ordinal().is_none() && element.clip_rect().is_none() {
                        (ReplayPointerDisposition::PassThrough, None)
                    } else {
                        materialized_pointer_disposition(
                            instance.semantic(rendered.rendered_path().template_child_path()),
                            interactive,
                        )
                    };
                let screen_rect = element
                    .screen_rect()
                    .map(|rect| Rect::new(rect.x(), rect.y(), rect.width(), rect.height()));
                let clip_rect = element
                    .clip_rect()
                    .map(|rect| Rect::new(rect.x(), rect.y(), rect.width(), rect.height()));
                if disposition != ReplayPointerDisposition::PassThrough {
                    let (Some(screen_rect), Some(clip_rect), Some(paint_ordinal)) =
                        (screen_rect, clip_rect, element.paint_ordinal())
                    else {
                        return Err(theatre_materialization_rejection(input.panel.window_id()));
                    };
                    hit_targets.push(ReplayHitTarget {
                        window_id: input.panel.window_id().to_string(),
                        runtime_instance_token: lease.runtime_instance_token().to_string(),
                        source_file: instance
                            .template_panel()
                            .source_content()
                            .normalized_game_relative_path
                            .clone(),
                        surface_id: instance.template_panel().surface_id().to_string(),
                        element_id: element.element_id().to_string(),
                        authored_name: element.authored_name().to_string(),
                        child_path: child_path.clone(),
                        global_root_sibling: instance.global_root_sibling(),
                        element_type: element.element_type().to_string(),
                        screen_rect,
                        clip_rect,
                        paint_ordinal,
                        disposition,
                        visual_enabled,
                    });
                }
                element_states.push(ReplayComposedElementState {
                    window_id: input.panel.window_id().to_string(),
                    runtime_instance_token: lease.runtime_instance_token().to_string(),
                    source_file: instance
                        .template_panel()
                        .source_content()
                        .normalized_game_relative_path
                        .clone(),
                    surface_id: instance.template_panel().surface_id().to_string(),
                    element_id: element.element_id().to_string(),
                    authored_name: element.authored_name().to_string(),
                    child_path,
                    global_root_sibling: instance.global_root_sibling(),
                    element_type: element.element_type().to_string(),
                    paint_ordinal: element.paint_ordinal(),
                    screen_rect,
                    clip_rect,
                    disposition,
                    visible: element.visible(),
                    visual_enabled,
                });
            }
        }
    }
    hit_targets.sort_by_key(|target| target.paint_ordinal);
    let mut scroll_regions = Vec::with_capacity(input.scroll_bindings.len());
    for scroll in input.scroll_bindings {
        let element = element_states
            .iter()
            .find(|element| element.element_id == scroll.element_id)
            .ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidScrollRegion(scroll.element_id.clone())
            })?;
        if plan.is_some() && element.authored_name.eq_ignore_ascii_case("elements_list") {
            continue;
        }
        let viewport_rect = element.clip_rect.ok_or_else(|| {
            ReplayWindowManagerRejection::InvalidScrollRegion(scroll.element_id.clone())
        })?;
        scroll_regions.push(ReplayScrollRegion {
            window_id: input.panel.window_id().to_string(),
            runtime_instance_token: lease.runtime_instance_token().to_string(),
            element_id: scroll.element_id.clone(),
            child_path: element.child_path.clone(),
            viewport_rect,
            axis: scroll.axis,
            min_offset_pixels: scroll.min_offset_pixels,
            max_offset_pixels: scroll.max_offset_pixels,
            offset_pixels: scroll.offset_pixels,
            wheel_step_pixels: None,
            smooth_scrolling: false,
            extended_scrollbar: None,
            geometry_source_record: scroll.producer_record.clone(),
        });
    }
    if let Some(plan) = plan {
        if input.frontend_collections_capability.is_some() {
            if input.theatre_capability.is_some()
                || input.production_lines_capability.is_some()
                || !input.scroll_bindings.is_empty()
                || plan.extended_scrollbar().is_some()
                || plan.scroll_offset_pixels() != 0
            {
                return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                    format!("{}:XUI26-materialization", input.panel.window_id()),
                ));
            }
            for planned in plan.collection_scroll_regions() {
                let hosts = element_states
                    .iter()
                    .filter(|element| {
                        element.child_path == planned.child_path
                            && element.element_id == planned.element_id
                    })
                    .collect::<Vec<_>>();
                let [host] = hosts.as_slice() else {
                    return Err(ReplayWindowManagerRejection::InvalidScrollRegion(
                        planned.element_id.clone(),
                    ));
                };
                let viewport_rect = Rect::new(
                    planned.viewport_rect.x(),
                    planned.viewport_rect.y(),
                    planned.viewport_rect.width(),
                    planned.viewport_rect.height(),
                );
                if host.clip_rect != Some(viewport_rect) {
                    return Err(ReplayWindowManagerRejection::InvalidScrollRegion(
                        planned.element_id.clone(),
                    ));
                }
                scroll_regions.push(ReplayScrollRegion {
                    window_id: input.panel.window_id().to_string(),
                    runtime_instance_token: lease.runtime_instance_token().to_string(),
                    element_id: planned.element_id.clone(),
                    child_path: planned.child_path.clone(),
                    viewport_rect,
                    axis: ReplayScrollAxis::Vertical,
                    min_offset_pixels: 0,
                    max_offset_pixels: planned.max_offset_pixels,
                    offset_pixels: planned.offset_pixels,
                    wheel_step_pixels: Some(planned.wheel_step_pixels),
                    smooth_scrolling: planned.smooth_scrolling,
                    extended_scrollbar: None,
                    geometry_source_record: planned.geometry_source_record.clone(),
                });
            }
        } else {
            let production_lines = input.production_lines_capability;
            let materialization_rejection = |reason: &str| {
                ReplayWindowManagerRejection::InvalidComposedGeometry(format!(
                    "{}:{}-materialization:{reason}",
                    input.panel.window_id(),
                    if production_lines.is_some() {
                        "XUI12-production-lines"
                    } else {
                        "XUI07-theatre"
                    }
                ))
            };
            let list = element_states
                .iter()
                .filter(|element| match production_lines {
                    Some(binding) => element.child_path == binding.list_child_path,
                    None => element.authored_name.eq_ignore_ascii_case("elements_list"),
                })
                .collect::<Vec<_>>();
            let [list] = list.as_slice() else {
                return Err(materialization_rejection("list-cardinality"));
            };
            let list = (*list).clone();
            let authored_list = input
                .panel
                .authored_element(&list.child_path)
                .ok_or_else(|| materialization_rejection("authored-list"))?;
            if production_lines.is_none()
                && !authored_list
                    .element_type
                    .eq_ignore_ascii_case("gridBoxType")
            {
                return Err(materialization_rejection("list-family"));
            }
            let wheel_step_pixels = authored_list
                .raw
                .first_scalar_ci("scroll_wheel_factor")
                .and_then(|raw| raw.trim().parse::<i32>().ok())
                .filter(|step| *step > 0)
                .ok_or_else(|| materialization_rejection("wheel-step"))?;
            let smooth_scrolling = match authored_list
                .raw
                .first_scalar_ci("smooth_scrolling")
                .map(str::trim)
            {
                Some(value) if value.eq_ignore_ascii_case("yes") => true,
                Some(value) if value.eq_ignore_ascii_case("no") => false,
                _ => return Err(materialization_rejection("smooth-scrolling")),
            };
            let viewport_rect = list
                .clip_rect
                .ok_or_else(|| materialization_rejection("list-clip"))?;
            let max_offset_pixels = plan
                .content_rect()
                .height()
                .checked_sub(viewport_rect.h())
                .unwrap_or(0)
                .max(0);
            if plan.scroll_offset_pixels() > max_offset_pixels {
                return Err(materialization_rejection("scroll-offset"));
            }
            let extended_scrollbar = plan.extended_scrollbar().map(|scrollbar| {
                let rect = |rect: AuthenticatedElementRect| {
                    Rect::new(rect.x(), rect.y(), rect.width(), rect.height())
                };
                ReplayExtendedScrollbarGeometry {
                    background_rect: rect(scrollbar.background.rect),
                    track_rect: rect(scrollbar.track.rect),
                    decrease_rect: rect(scrollbar.decrease.rect),
                    increase_rect: rect(scrollbar.increase.rect),
                    slider_rect: rect(scrollbar.slider.rect),
                    thumb_base_pixels: scrollbar.thumb_base_pixels,
                    thumb_travel_pixels: scrollbar.thumb_travel_pixels,
                    thumb_authored_offset_pixels: scrollbar.thumb_authored_offset_pixels,
                }
            });
            if production_lines.is_some() && extended_scrollbar.is_none() {
                return Err(materialization_rejection("extended-scrollbar"));
            }
            if let Some(scrollbar) = extended_scrollbar.as_ref() {
                let child_path = vec![u32::MAX, u32::MAX, u32::MAX];
                let element_id = format!("{}.extended-scrollbar-slider", list.element_id);
                let paint_ordinal = hit_targets
                    .iter()
                    .map(|target| target.paint_ordinal)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| materialization_rejection("scrollbar-paint-ordinal"))?;
                let disposition = if max_offset_pixels == 0 {
                    ReplayPointerDisposition::BlockOnly
                } else {
                    ReplayPointerDisposition::Interactive
                };
                let visual_enabled = (max_offset_pixels != 0).then_some(true);
                element_states.push(ReplayComposedElementState {
                    window_id: input.panel.window_id().to_string(),
                    runtime_instance_token: lease.runtime_instance_token().to_string(),
                    source_file: plan
                        .extended_scrollbar()
                        .expect("XUI12 scrollbar checked above")
                        .source_content
                        .normalized_game_relative_path
                        .clone(),
                    surface_id: report.surface_id().to_string(),
                    element_id: element_id.clone(),
                    authored_name: "Slider".to_string(),
                    child_path: child_path.clone(),
                    global_root_sibling: false,
                    element_type: "extendedScrollbarSlider".to_string(),
                    paint_ordinal: Some(paint_ordinal),
                    screen_rect: Some(scrollbar.slider_rect),
                    clip_rect: Some(scrollbar.slider_rect),
                    disposition,
                    visible: true,
                    visual_enabled,
                });
                hit_targets.push(ReplayHitTarget {
                    window_id: input.panel.window_id().to_string(),
                    runtime_instance_token: lease.runtime_instance_token().to_string(),
                    source_file: plan
                        .extended_scrollbar()
                        .expect("XUI12 scrollbar checked above")
                        .source_content
                        .normalized_game_relative_path
                        .clone(),
                    surface_id: report.surface_id().to_string(),
                    element_id,
                    authored_name: "Slider".to_string(),
                    child_path,
                    global_root_sibling: false,
                    element_type: "extendedScrollbarSlider".to_string(),
                    screen_rect: scrollbar.slider_rect,
                    clip_rect: scrollbar.slider_rect,
                    paint_ordinal,
                    disposition,
                    visual_enabled,
                });
                hit_targets.sort_by_key(|target| target.paint_ordinal);
            }
            scroll_regions.push(ReplayScrollRegion {
            window_id: input.panel.window_id().to_string(),
            runtime_instance_token: lease.runtime_instance_token().to_string(),
            element_id: list.element_id.clone(),
            child_path: list.child_path.clone(),
            viewport_rect,
            axis: ReplayScrollAxis::Vertical,
            min_offset_pixels: 0,
            max_offset_pixels,
            offset_pixels: plan.scroll_offset_pixels(),
            wheel_step_pixels: Some(wheel_step_pixels),
            smooth_scrolling,
            extended_scrollbar,
            geometry_source_record: format!(
                "{}/materialization={}/scroll-extent={}/wheel-step={wheel_step_pixels}/smooth={smooth_scrolling}",
                if production_lines.is_some() { "XUI12/production-lines" } else { "XUI07/theatre" },
                plan.materialization_digest_sha256(),
                plan.scroll_extent_identity_sha256(),
            ),
        });
        }
    }
    Ok(ReplayComposedPanelGeometry {
        window_id: input.panel.window_id().to_string(),
        runtime_instance_token: lease.runtime_instance_token().to_string(),
        source_file: report
            .source_content()
            .normalized_game_relative_path
            .clone(),
        surface_id: report.surface_id().to_string(),
        screen_rect: Rect::new(
            root_rect.x(),
            root_rect.y(),
            root_rect.width(),
            root_rect.height(),
        ),
        clip_rect: Rect::new(
            root_clip.x(),
            root_clip.y(),
            root_clip.width(),
            root_clip.height(),
        ),
        geometry_source_record: format!(
            "XUI02/final-report={}/window={}/instance={}/frame={}",
            report.index_identity_sha256(),
            input.panel.window_id(),
            lease.runtime_instance_token(),
            report.frame_generation()
        ),
        element_states,
        hit_targets,
        scroll_regions,
    })
}

fn validate_geometry(
    geometry: &ReplayComposedPanelGeometry,
) -> Result<(), ReplayWindowManagerRejection> {
    if geometry.source_file.trim().is_empty()
        || geometry.surface_id.trim().is_empty()
        || geometry.geometry_source_record.trim().is_empty()
        || !rect_is_positive(geometry.screen_rect)
        || !rect_is_positive(geometry.clip_rect)
        || !rect_within(geometry.clip_rect, geometry.screen_rect)?
    {
        return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
            geometry.window_id.clone(),
        ));
    }
    let mut element_identities = BTreeSet::new();
    for element in &geometry.element_states {
        let visible_geometry_is_complete = match (
            element.visible,
            element.paint_ordinal,
            element.screen_rect,
            element.clip_rect,
        ) {
            (true, Some(_), Some(screen), Some(clip)) => {
                rect_is_positive(screen)
                    && rect_is_positive(clip)
                    && rect_within(clip, screen)?
                    && rect_within(clip, geometry.clip_rect)?
            }
            (true, None, Some(screen), Some(clip)) => {
                element.disposition == ReplayPointerDisposition::PassThrough
                    && rect_is_positive(screen)
                    && rect_is_positive(clip)
                    && rect_within(clip, screen)?
                    && rect_within(clip, geometry.clip_rect)?
            }
            (true, Some(_), Some(screen), None) => rect_is_positive(screen),
            (true, None, Some(screen), None) => rect_is_positive(screen),
            (false, None, None, None) => true,
            _ => false,
        };
        if element.window_id != geometry.window_id
            || element.runtime_instance_token != geometry.runtime_instance_token
            || element.source_file.trim().is_empty()
            || element.surface_id.trim().is_empty()
            || element.element_id.trim().is_empty()
            || element.authored_name.trim().is_empty()
            || element.element_type.trim().is_empty()
            || !visible_geometry_is_complete
            || !element_identities
                .insert((element.element_id.as_str(), element.child_path.as_slice()))
        {
            return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                geometry.window_id.clone(),
            ));
        }
    }
    let mut identities = BTreeSet::new();
    let mut prior_paint_ordinal = None;
    for target in &geometry.hit_targets {
        if target.window_id != geometry.window_id
            || target.runtime_instance_token != geometry.runtime_instance_token
            || target.source_file.trim().is_empty()
            || target.surface_id.trim().is_empty()
            || target.element_id.trim().is_empty()
            || target.authored_name.trim().is_empty()
            || target.element_type.trim().is_empty()
            || target.disposition == ReplayPointerDisposition::Interactive
                && target.visual_enabled.is_none()
            || !rect_is_positive(target.screen_rect)
            || !rect_is_positive(target.clip_rect)
            || !rect_within(target.clip_rect, target.screen_rect)?
            || !rect_within(target.clip_rect, geometry.clip_rect)?
            || prior_paint_ordinal.is_some_and(|prior| target.paint_ordinal <= prior)
            || !identities.insert((target.element_id.as_str(), target.child_path.as_slice()))
            || !geometry.element_states.iter().any(|element| {
                element.element_id == target.element_id
                    && element.source_file == target.source_file
                    && element.surface_id == target.surface_id
                    && element.authored_name == target.authored_name
                    && element.child_path == target.child_path
                    && element.global_root_sibling == target.global_root_sibling
                    && element.visible
                    && element.screen_rect == Some(target.screen_rect)
                    && element.clip_rect == Some(target.clip_rect)
                    && element.paint_ordinal == Some(target.paint_ordinal)
                    && element.disposition == target.disposition
                    && element.visual_enabled == target.visual_enabled
            })
        {
            return Err(ReplayWindowManagerRejection::InvalidComposedGeometry(
                geometry.window_id.clone(),
            ));
        }
        prior_paint_ordinal = Some(target.paint_ordinal);
    }
    let mut scroll_identities = BTreeSet::new();
    for region in &geometry.scroll_regions {
        if region.window_id != geometry.window_id
            || region.runtime_instance_token != geometry.runtime_instance_token
            || region.element_id.trim().is_empty()
            || region.geometry_source_record.trim().is_empty()
            || !rect_is_positive(region.viewport_rect)
            || !rect_within(region.viewport_rect, geometry.clip_rect)?
            || region.min_offset_pixels > region.max_offset_pixels
            || !(region.min_offset_pixels..=region.max_offset_pixels)
                .contains(&region.offset_pixels)
            || region.wheel_step_pixels.is_some_and(|step| step <= 0)
            || (region.smooth_scrolling && region.wheel_step_pixels.is_none())
            || !scroll_identities.insert((region.element_id.as_str(), region.child_path.as_slice()))
        {
            return Err(ReplayWindowManagerRejection::InvalidScrollRegion(
                geometry.window_id.clone(),
            ));
        }
    }
    Ok(())
}

fn exact_extended_scrollbar_offset_for_pixel(
    max_offset_pixels: i32,
    thumb_base_pixels: i32,
    thumb_travel_pixels: i32,
    desired_mapped_pixel: i32,
) -> Result<i32, ReplayWindowManagerRejection> {
    if max_offset_pixels < 0 || thumb_travel_pixels < 0 {
        return Err(ReplayWindowManagerRejection::InvalidScrollRegion(
            "extended-scrollbar-range".to_string(),
        ));
    }
    if max_offset_pixels == 0 || thumb_travel_pixels == 0 {
        return Ok(0);
    }
    let desired = desired_mapped_pixel.clamp(
        thumb_base_pixels,
        thumb_base_pixels
            .checked_add(thumb_travel_pixels)
            .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?,
    );
    let mapped = |offset: i32| -> Result<i32, ReplayWindowManagerRejection> {
        extended_thumb_pixel(ExtendedScrollbarCarrier {
            authority: ScrollbarCarrierAuthority::ExactA729Runtime,
            axis: ScrollbarAxis::Vertical,
            min_fixed: 0,
            max_fixed: i64::from(max_offset_pixels) * 100_000,
            value_fixed: i64::from(offset) * 100_000,
            base_pixel: (0, thumb_base_pixels),
            travel_pixels: (0, thumb_travel_pixels),
        })
        .map(|(_, y)| y)
        .map_err(|_| {
            ReplayWindowManagerRejection::InvalidScrollRegion(
                "extended-scrollbar-forward-law".to_string(),
            )
        })
    };
    let mut low = 0i32;
    let mut high = max_offset_pixels;
    while low < high {
        let mid = low + (high - low) / 2;
        if mapped(mid)? < desired {
            low = mid
                .checked_add(1)
                .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
        } else {
            high = mid;
        }
    }
    let upper = low;
    let lower = upper.saturating_sub(1);
    let upper_distance = i64::from(mapped(upper)?).abs_diff(i64::from(desired));
    let lower_distance = i64::from(mapped(lower)?).abs_diff(i64::from(desired));
    Ok(if lower_distance <= upper_distance {
        lower
    } else {
        upper
    })
}

fn set_scroll_region_offset(
    geometry: &mut ReplayComposedPanelGeometry,
    region_index: usize,
    requested_offset: i32,
) -> Result<ReplayScrollOutcome, ReplayWindowManagerRejection> {
    let (outcome, slider_update) = {
        let region = geometry
            .scroll_regions
            .get_mut(region_index)
            .ok_or_else(|| {
                ReplayWindowManagerRejection::InvalidScrollRegion("region-index".to_string())
            })?;
        let prior_offset_pixels = region.offset_pixels;
        region.offset_pixels =
            requested_offset.clamp(region.min_offset_pixels, region.max_offset_pixels);
        let slider_update = if let Some(scrollbar) = region.extended_scrollbar.as_mut() {
            let (_, mapped_y) = extended_thumb_pixel(ExtendedScrollbarCarrier {
                authority: ScrollbarCarrierAuthority::ExactA729Runtime,
                axis: ScrollbarAxis::Vertical,
                min_fixed: i64::from(region.min_offset_pixels) * 100_000,
                max_fixed: i64::from(region.max_offset_pixels) * 100_000,
                value_fixed: i64::from(region.offset_pixels) * 100_000,
                base_pixel: (0, scrollbar.thumb_base_pixels),
                travel_pixels: (0, scrollbar.thumb_travel_pixels),
            })
            .map_err(|_| {
                ReplayWindowManagerRejection::InvalidScrollRegion(region.element_id.clone())
            })?;
            let slider_y = mapped_y
                .checked_add(scrollbar.thumb_authored_offset_pixels)
                .ok_or(ReplayWindowManagerRejection::PixelCoordinateOverflow)?;
            scrollbar.slider_rect = Rect::new(
                scrollbar.slider_rect.x(),
                slider_y,
                scrollbar.slider_rect.w(),
                scrollbar.slider_rect.h(),
            );
            Some((
                format!("{}.extended-scrollbar-slider", region.element_id),
                scrollbar.slider_rect,
            ))
        } else {
            None
        };
        (
            ReplayScrollOutcome {
                window_id: region.window_id.clone(),
                runtime_instance_token: region.runtime_instance_token.clone(),
                element_id: region.element_id.clone(),
                child_path: region.child_path.clone(),
                axis: region.axis,
                prior_offset_pixels,
                offset_pixels: region.offset_pixels,
            },
            slider_update,
        )
    };
    if let Some((element_id, rect)) = slider_update {
        for target in &mut geometry.hit_targets {
            if target.element_id == element_id {
                target.screen_rect = rect;
                target.clip_rect = rect;
            }
        }
        for element in &mut geometry.element_states {
            if element.element_id == element_id {
                element.screen_rect = Some(rect);
                element.clip_rect = Some(rect);
            }
        }
    }
    Ok(outcome)
}
