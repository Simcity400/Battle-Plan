#![recursion_limit = "256"]

#[path = "../domain44_compact.rs"]
pub mod compact;

pub mod decoder_gate;
pub mod extractor;
pub mod replay_verify;

pub use compact::DOMAIN_ID_PROPOSED as DOMAIN_ID;
