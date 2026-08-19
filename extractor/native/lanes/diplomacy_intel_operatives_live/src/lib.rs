#![recursion_limit = "256"]

#[path = "../domain44_compact.rs"]
pub mod compact;

pub mod a30_companion_carrier;
#[cfg(windows)]
pub mod a30_controller_evidence;
#[cfg(windows)]
pub mod a30_non_operation_action_identity;
#[cfg(windows)]
pub mod a30_operative_operation_evidence;
#[cfg(windows)]
pub mod a30_operative_portrait_join;
pub mod a30_recruitable_capture_join;
#[cfg(windows)]
pub mod a30_visibility;
pub mod agency_upgrade_controller;
pub mod cipher_controller;
pub mod decoder_gate;
pub mod extractor;
pub mod replay_verify;
pub mod stream_gate;

pub use compact::DOMAIN_ID_PROPOSED as DOMAIN_ID;
