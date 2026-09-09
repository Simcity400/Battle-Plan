//! Callable identities from the G7 writer-census adjudication.
//!
//! These image-relative addresses describe functions, not invented receiver
//! classes or hook authorizations. The pinned artifacts identify a729 and image
//! base 0x140000000; the source records ghidra_verify agrees=true for its exports.

/// Copies members between two unidentified object arguments.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:65`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_1401594f0.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const COPY_OBJECT_RVA: usize = 0x0015_94f0;

/// Processes an object's members with opaque identifier arguments.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:69`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140685fd0.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const KEYED_MEMBER_PROCESSOR_RVA: usize = 0x0068_5fd0;

/// Populates members on an unidentified object argument.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:73`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140ac3c70.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const MEMBER_READER_RVA: usize = 0x00ac_3c70;

/// Scans source objects and reads their embedded strings.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:77`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140b36c60.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const STRING_SOURCE_SCAN_RVA: usize = 0x00b3_6c60;

/// Reads a source object's embedded string using a separate caller.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:81`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140b3a080.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const STRING_SOURCE_READ_RVA: usize = 0x00b3_a080;

/// Looks up a source object's embedded string and invokes a reader.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:85`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140b3b940.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const STRING_SOURCE_LOOKUP_RVA: usize = 0x00b3_b940;

/// Updates a dated collection on an unidentified owner.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:89`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_141b6f0c0.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const DATED_RECORD_UPDATE_RVA: usize = 0x01b6_f0c0;

/// Updates a nested object's member and sets its update flag.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:93`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_141b73510.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const INDIRECT_MEMBER_UPDATE_RVA: usize = 0x01b7_3510;

/// Resolves a source object and passes its members to the fixed-window constructor.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:105`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_1422e22b0.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const FIXED_WINDOW_FACTORY_RVA: usize = 0x022e_22b0;

/// Compiles authored particle subsystem records into distinct output records.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:109`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_14232ecc0.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const PARTICLE_SUBSYSTEM_COMPILE_RVA: usize = 0x0232_ecc0;

/// Checks attributes recursively on an unidentified record hierarchy.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:113`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_14232f270.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const RECURSIVE_ATTRIBUTE_CHECK_RVA: usize = 0x0232_f270;

/// Reader whose second argument uses the pinned std::string representation.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:117`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140b3d930.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const STRING_READER_140B3D930_RVA: usize = 0x00b3_d930;

/// Second reader whose second argument uses the pinned std::string representation.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:121`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140b340c0.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const STRING_READER_140B340C0_RVA: usize = 0x00b3_40c0;

/// Lookup helper receiving the source's embedded string as its second argument.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:125`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140abce70.decompile.c:4`
/// identifies the exact function address; line 3 supplies the image base.
pub const STRING_LOOKUP_RVA: usize = 0x00ab_ce70;

/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:265`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_1401594f0.decompile.c:36`
/// contains the named call; the RVA is its target address minus the pinned image base.
pub const COPY_MEMBER_RVA: usize = 0x0053_dcc0;

/// Helper called with opaque identifier 0x414d and the +0x3b0 member.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:271`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140685fd0.decompile.c:63`
/// contains the named call; the RVA is its target address minus the pinned image base.
pub const KEYED_MEMBER_140686760_RVA: usize = 0x0068_6760;

/// Helper called with opaque identifier 0x3e91 and the +0x4d0 member.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:273`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140685fd0.decompile.c:75`
/// contains the named call; the RVA is its target address minus the pinned image base.
pub const KEYED_MEMBER_140686A70_RVA: usize = 0x0068_6a70;

/// Helper receiving the +0x3b0 member and a temporary value.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:277`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140ac3c70.decompile.c:392`
/// contains the named call; the RVA is its target address minus the pinned image base.
pub const MEMBER_1404582B0_RVA: usize = 0x0045_82b0;

/// Helper receiving the +0x548 member as its second argument.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:279`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_140ac3c70.decompile.c:1150`
/// contains the named call; the RVA is its target address minus the pinned image base.
pub const MEMBER_1424ABB50_RVA: usize = 0x024a_bb50;

/// Helper receiving a collection base, an element index, and a temporary record.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:309`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_141b6f0c0.decompile.c:199`
/// contains the named call; the RVA is its target address minus the pinned image base.
pub const INDEXED_RECORD_UPDATE_RVA: usize = 0x01b6_1640;

/// Helper receiving the nested object's +0x3b0 member.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:320`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_141b73510.decompile.c:19`
/// contains the named call; the RVA is its target address minus the pinned image base.
pub const INDIRECT_MEMBER_HELPER_RVA: usize = 0x01b6_17b0;

/// Compiles two authored attributes into a member of a different output object.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:344`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_14232ecc0.decompile.c:64`
/// contains the named call; the RVA is its target address minus the pinned image base.
pub const ATTRIBUTE_COMPILE_RVA: usize = 0x0231_78c0;

/// Helper checking the recursive record's +0x3b0 member.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:350`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_14232f270.decompile.c:54`
/// contains the named call; the RVA is its target address minus the pinned image base.
pub const ATTRIBUTE_CHECK_RVA: usize = 0x0232_f820;
