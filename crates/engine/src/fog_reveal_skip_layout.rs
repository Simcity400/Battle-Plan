//! Pointee layout used by the fog division skip predicate; concrete record type is unproven.
//! No identity with CReferencedDivisionTemplate or a recorded division field is asserted.
//! QP evidence: `extractor/native/runs/qp_fog_bindings/evidence/`.

/// Fog skip byte at pointee+234; QP func_140c79aa0.decompile.c returns it and func_141606390 skips flagged divisions; writer and pointee type remain unproven.
pub const FLAG: usize = 0x234;
