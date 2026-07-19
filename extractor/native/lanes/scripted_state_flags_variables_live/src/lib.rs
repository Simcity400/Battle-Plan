#[path = "../country_variables_extractor.rs"]
pub mod country_variables_extractor;
pub mod scripted_gui_runtime_carrier;
#[path = "../scripted_state_domain51_collector.rs"]
pub mod scripted_state_domain51_collector;
#[path = "../scripted_state_domain51_gate.rs"]
pub mod scripted_state_domain51_gate;

pub use country_variables_extractor::{
    read_variables_container, rtti_type_name, MemoryRead, VariableReadStatus, RTTI_VARIABLES,
};
pub use scripted_gui_runtime_carrier::{
    collect_and_validate_scripted_gui_runtime, validate_scripted_gui_runtime_frame, EngineResult,
    ExactScriptedGuiRuntimeSource, ScriptedGuiRuntimeFrame, ScriptedGuiRuntimeRow,
};
pub use scripted_state_domain51_collector::{
    collect_domain51_frame, registration_descriptor, DelayedEventRow, Domain51CollectedFrame,
    Domain51FrameInput, FlagEntry, FlagScopeRead, OpaqueRootState, ScopeReadStatus, VariableEntry,
    VariableScopeRead,
};
pub use scripted_state_domain51_gate::compact::{
    Domain51GateSummary, DOMAIN_ID, DOMAIN_NAME, STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
    STATUS_FLAG_NAMES_PARTIAL, STATUS_PARTIAL_DATA, STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
    STATUS_SOURCE_UNAVAILABLE,
};
pub use scripted_state_domain51_gate::{gate_domain51_payload, Domain51GateVerdict};
