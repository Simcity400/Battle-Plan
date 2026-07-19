//! XUI02 runtime-window-manager admission gate.
//!
//! XUI01 admits all exact file-derived window definitions, but admits no
//! runtime compositor state. This module registers every admitted definition
//! in a dormant manager registry and rejects fabricated activation until a
//! typed owner contract supplies the complete state.

use crate::definition_admission::{GuiDefinitionCatalog, FROZEN_FILE_COUNT, FROZEN_WINDOW_COUNT};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::sync::OnceLock;

pub const XUI02_ACTIVE_GUI_FILES: usize = FROZEN_FILE_COUNT;
pub const XUI02_WINDOW_DEFINITIONS: usize = FROZEN_WINDOW_COUNT;
pub const XUI02_ADMITTED_DEFINITIONS: usize = 9_155;
pub const XUI02_REJECTED_DEFINITIONS: usize = 0;
pub const XUI02_RUNTIME_ACTIVATIONS: usize = 0;
pub const XUI02_DEPENDENCY_WINDOWS: usize = 190;
pub const XUI02_DEPENDENCY_FILES: usize = 77;
const ACTIVATION_INVENTORY_JSON: &str =
    include_str!("../../../re_evidence/rendering_laws/XUI02/activation_dependency_inventory.json");

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

/// Register every exact XUI01 definition as dormant and reject all fabricated
/// runtime state. Dormant fields are product fail-closed defaults, not claims
/// about a live engine window.
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

/// Register the exact effective GUI namespace selected by the admitted VFS.
///
/// Unlike [`register_dormant_window_manager`], this production entry point
/// does not compare a modded/DLC profile to the frozen vanilla census. The
/// replay manifest and VFS capability authenticate the files; this gate owns
/// only the namespace invariants needed by the replay-owned manager.
pub fn register_effective_dormant_window_manager(
    catalog: &GuiDefinitionCatalog,
    runtime_states: &[RuntimeWindowManagerState],
) -> Result<DormantWindowManagerRegistry, WindowManagerRejection> {
    if catalog.files.is_empty()
        || catalog.admitted_windows.is_empty()
        || catalog.rejected_file_count() != 0
        || !catalog.rejected_windows.is_empty()
        || catalog.admitted_file_count() != catalog.files.len()
        || catalog.admitted_windows.len() != catalog.total_window_count()
    {
        return Err(WindowManagerRejection::DefinitionRejectionsPresent);
    }
    let mut file_rows = BTreeMap::new();
    for file in &catalog.files {
        if file.rejected_windows != 0
            || file_rows
                .insert(file.source_file.as_str(), file.admitted_windows)
                .is_some()
        {
            return Err(WindowManagerRejection::DefinitionRejectionsPresent);
        }
    }
    let mut actual_by_file = file_rows
        .keys()
        .map(|source| (*source, 0usize))
        .collect::<BTreeMap<_, _>>();
    for row in &catalog.admitted_windows {
        let Some(count) = actual_by_file.get_mut(row.source_file.as_str()) else {
            return Err(WindowManagerRejection::NamespaceDrift);
        };
        *count += 1;
    }
    if file_rows != actual_by_file {
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

/// Parse the embedded activation inventory exactly once per process. The JSON
/// is a compile-time constant, so the parse outcome is deterministic; a parse
/// failure stays a typed fatal rejection on every call.
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

/// Return the exact source-window rows for which current controller or state-
/// seam code can supply a runtime manager record. Descendants are expanded by
/// [`apply_runtime_activation_frame`] from their nearest effective source.
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

/// Validate a controller-produced manager frame and expand every supplied
/// source state through exact GUI window ancestry. All other rows retain an
/// explicit, narrow dependency instead of being collapsed into one dormant
/// bucket.
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

/// Input family used by the replay-owned focus traversal law. This is product
/// view state: it is intentionally separate from captured simulation state and
/// from the game's private platform event enums.
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
    /// Identity of the already-resolved tooltip payload. XUI02 owns only its
    /// presentation lifetime; tooltip content remains validated separately.
    pub tooltip_instance_record: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayPanelActivation {
    pub window_id: String,
    /// `Some(owner)` opens this window as a modal child of the current input
    /// owner. `None` opens an ordinary replay panel.
    pub modal_owner: Option<String>,
    pub initial_focus: Option<String>,
    /// Exact product traversal order supplied by the admitted panel
    /// implementation. XUI02 never reconstructs this from source order.
    pub keyboard_traversal: Vec<String>,
    pub controller_traversal: Vec<String>,
    /// Exact widget identities permitted to own an already-resolved tooltip
    /// payload. Empty means the panel admits no tooltip ownership.
    pub tooltip_targets: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayPanelState {
    pub window_id: String,
    pub open: bool,
    pub visible: bool,
    pub composed: bool,
    /// Contiguous back-to-front ordinal among open replay panels.
    pub stack_ordinal: Option<u32>,
    pub modal_owner: Option<String>,
    pub focused_widget: Option<String>,
    pub keyboard_traversal: Vec<String>,
    pub controller_traversal: Vec<String>,
    pub tooltip_targets: Vec<String>,
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
    StackOrdinalOverflow,
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
            Self::StackOrdinalOverflow => write!(formatter, "XUI02 replay stack ordinal overflow"),
        }
    }
}

impl Error for ReplayWindowManagerRejection {}

/// Deterministic replay-owned window state for the complete XUI01 namespace.
///
/// This manager does not claim to capture a remote player's private UI. It
/// owns the replay application's view state and applies exact, testable
/// invariants: visibility equals open state, composition equals stack
/// membership, ordinals are contiguous, an active modal exclusively owns
/// input, focus restores when that modal closes, and tooltip ownership cannot
/// outlive its input panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayOwnedWindowManager {
    panels: BTreeMap<String, ReplayPanelState>,
    stack: Vec<String>,
    focus: Option<ReplayFocusTarget>,
    focus_restore: BTreeMap<String, Option<ReplayFocusTarget>>,
    tooltip: Option<ReplayTooltipOwnership>,
    transition_ordinal: u64,
}

impl ReplayOwnedWindowManager {
    pub fn new(
        registry: &DormantWindowManagerRegistry,
    ) -> Result<Self, ReplayWindowManagerRejection> {
        let mut panels = BTreeMap::new();
        for row in &registry.windows {
            let state = ReplayPanelState {
                window_id: row.window_id.clone(),
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
        Ok(Self {
            panels,
            stack: Vec::new(),
            focus: None,
            focus_restore: BTreeMap::new(),
            tooltip: None,
            transition_ordinal: 0,
        })
    }

    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }

    pub fn transition_ordinal(&self) -> u64 {
        self.transition_ordinal
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

    /// The topmost modal owns input. Without a modal, the topmost composed
    /// panel owns input. This is also the only panel allowed to own a tooltip.
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

    pub fn open(
        &mut self,
        activation: ReplayPanelActivation,
    ) -> Result<(), ReplayWindowManagerRejection> {
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
        let state = self
            .panels
            .get_mut(&activation.window_id)
            .expect("window existence checked before mutation");
        state.open = true;
        state.visible = true;
        state.composed = true;
        state.modal_owner = activation.modal_owner;
        state.focused_widget = initial_focus.clone();
        state.keyboard_traversal = activation.keyboard_traversal;
        state.controller_traversal = activation.controller_traversal;
        state.tooltip_targets = activation.tooltip_targets;
        self.focus_restore
            .insert(activation.window_id.clone(), prior_focus);
        self.stack.push(activation.window_id.clone());
        self.focus = initial_focus.map(|widget_id| ReplayFocusTarget {
            window_id: activation.window_id,
            widget_id,
        });
        self.tooltip = None;
        self.finish_transition()
    }

    pub fn close(&mut self, window_id: &str) -> Result<(), ReplayWindowManagerRejection> {
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
        let state = self
            .panels
            .get_mut(window_id)
            .expect("window existence checked before mutation");
        state.open = false;
        state.visible = false;
        state.composed = false;
        state.stack_ordinal = None;
        state.modal_owner = None;
        state.focused_widget = None;
        state.keyboard_traversal.clear();
        state.controller_traversal.clear();
        state.tooltip_targets.clear();
        if self
            .tooltip
            .as_ref()
            .is_some_and(|row| row.window_id == window_id)
        {
            self.tooltip = None;
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
        self.finish_transition()
    }

    pub fn bring_to_front(&mut self, window_id: &str) -> Result<(), ReplayWindowManagerRejection> {
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
        self.sync_focused_widgets();
        self.finish_transition()
    }

    pub fn focus_widget(
        &mut self,
        window_id: &str,
        widget_id: &str,
    ) -> Result<(), ReplayWindowManagerRejection> {
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

    pub fn set_tooltip_owner(
        &mut self,
        window_id: &str,
        widget_id: &str,
        tooltip_instance_record: &str,
    ) -> Result<(), ReplayWindowManagerRejection> {
        if self.input_window_id() != Some(window_id)
            || widget_id.trim().is_empty()
            || tooltip_instance_record.trim().is_empty()
            || !self
                .panels
                .get(window_id)
                .is_some_and(|state| state.tooltip_targets.iter().any(|id| id == widget_id))
        {
            return Err(ReplayWindowManagerRejection::InvalidTooltipOwner(
                window_id.to_string(),
            ));
        }
        self.tooltip = Some(ReplayTooltipOwnership {
            window_id: window_id.to_string(),
            widget_id: widget_id.to_string(),
            tooltip_instance_record: tooltip_instance_record.to_string(),
        });
        self.bump_transition();
        Ok(())
    }

    pub fn clear_tooltip(&mut self) {
        if self.tooltip.take().is_some() {
            self.bump_transition();
        }
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

    fn finish_transition(&mut self) -> Result<(), ReplayWindowManagerRejection> {
        for state in self.panels.values_mut() {
            state.stack_ordinal = None;
        }
        for (ordinal, window_id) in self.stack.iter().enumerate() {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| ReplayWindowManagerRejection::StackOrdinalOverflow)?;
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
        Ok(())
    }

    fn bump_transition(&mut self) {
        self.transition_ordinal = self.transition_ordinal.saturating_add(1);
    }
}
