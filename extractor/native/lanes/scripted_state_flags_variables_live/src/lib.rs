#[cfg(windows)]
pub mod a729_delayed_event_lifecycle;
#[cfg(windows)]
pub mod a729_scripted_gui_runtime;
#[path = "../country_variables_extractor.rs"]
pub mod country_variables_extractor;
pub mod scripted_gui_runtime_carrier;
pub mod scripted_gui_runtime_compact;
#[path = "../scripted_state_domain51_collector.rs"]
pub mod scripted_state_domain51_collector;
#[path = "../scripted_state_domain51_gate.rs"]
pub mod scripted_state_domain51_gate;

pub use country_variables_extractor::{
    read_variables_container, rewitness_owed, rtti_type_name, MemoryRead, VariableReadStatus,
    RTTI_VARIABLES,
};
pub use scripted_gui_runtime_carrier::{
    collect_and_validate_scripted_gui_runtime, validate_scripted_gui_runtime_frame, EngineResult,
    ExactScriptedGuiRuntimeSource, ScriptedGuiDefinitionIdentity, ScriptedGuiRuntimeFrame,
    ScriptedGuiRuntimeRow, EXACT20_A38_JOB_ID, EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
};
pub use scripted_gui_runtime_compact::{
    collect_and_encode as collect_and_encode_scripted_gui_runtime,
    decode_payload as decode_scripted_gui_runtime_payload,
    encode_payload as encode_scripted_gui_runtime_payload,
    encode_source_unavailable as encode_scripted_gui_runtime_source_unavailable,
    validate_envelope as validate_scripted_gui_runtime_envelope,
    CompactError as ScriptedGuiCompactError, ScriptedGuiRuntimeEnvelope,
    DOMAIN_ID as SCRIPTED_GUI_RUNTIME_DOMAIN_ID, DOMAIN_NAME as SCRIPTED_GUI_RUNTIME_DOMAIN_NAME,
};
pub use scripted_state_domain51_collector::{
    collect_a13_domain51_frame, collect_a13_domain51_frame_shaped, collect_domain51_frame,
    collect_domain51_frame_shaped, registration_descriptor, A13CarrierError, A13FrameInput,
    DelayedEventRow, Domain51CollectedFrame, Domain51FrameInput, FlagEntry, FlagScopeRead,
    OpaqueRootState, PayloadShape, ScopeReadStatus, VariableEntry, VariableScopeRead,
};
pub use scripted_state_domain51_gate::compact::{
    Domain51GateSummary, DOMAIN_ID, DOMAIN_NAME, STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
    STATUS_FLAG_NAMES_PARTIAL, STATUS_PARTIAL_DATA, STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
    STATUS_SOURCE_UNAVAILABLE,
};
pub use scripted_state_domain51_gate::{gate_domain51_payload, Domain51GateVerdict};
