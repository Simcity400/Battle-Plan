//! Fail-closed framing and byte-I/O seam for the recorder/collector control pipe.
//!
//! The implementation now lives in the shared `compact_contract` crate so the
//! recorder host and the injected collector DLL consume exactly one copy of the
//! H4CT v1 wire format (`extractor/documents/DUPLEX_CONTROL_TRANSPORT.md` §2-3).
//! This module is a transparent re-export: every existing
//! `crate::control_transport::*` path keeps resolving, and any drift between the
//! two consumers is now a shared-crate compile/test failure rather than two
//! diverging local copies.

pub use compact_contract::control_transport::*;
