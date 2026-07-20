#![recursion_limit = "256"]

#[path = "../domain44_compact.rs"]
pub mod compact;

pub mod decoder_gate;
pub mod agency_upgrade_controller;
pub mod cipher_controller;
#[cfg(windows)]
pub mod a30_controller_evidence;
pub mod extractor;
pub mod replay_verify;
pub mod stream_gate;

pub use compact::DOMAIN_ID_PROPOSED as DOMAIN_ID;
