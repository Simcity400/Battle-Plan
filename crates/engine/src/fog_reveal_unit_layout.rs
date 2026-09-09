//! The same numeric offset as CArmy::STANDARD_TEMPLATE does not authenticate that identity.
//! QP evidence: `extractor/native/runs/qp_fog_bindings/evidence/`.

/// Nullable skip-record pointer at receiver+3b8; QP func_140c79aa0.decompile.c reads its +234 byte or returns zero for null, confirmed by the fog rebuild call.
pub const SKIP_RECORD_POINTER: usize = 0x3b8;
