//! Recorder-host view of the single shared H4CT v1 codec.
//!
//! The injected collector and recorder must exchange the exact same capability,
//! session-authority, frame, and error types. Keeping a second local copy made
//! apparently identical values type-incompatible and allowed the wire contract
//! to drift. All production implementation and tests live in
//! `compact_contract::control_transport`; this module preserves the historical
//! recorder-host path as a transparent re-export.

pub use compact_contract::control_transport::*;
