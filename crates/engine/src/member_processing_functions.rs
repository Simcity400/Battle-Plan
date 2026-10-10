//! Function locations for member copies, text reads and attribute processing.
//! These image-relative locations describe callable layouts, not hook safety.

/// Copies members between two unidentified object arguments.
pub const COPY_OBJECT_RVA: usize = 0x0015_94f0;

/// Processes an object's members with opaque identifier arguments.
pub const KEYED_MEMBER_PROCESSOR_RVA: usize = 0x0068_5fd0;

/// Populates members on an unidentified object argument.
pub const MEMBER_READER_RVA: usize = 0x00ac_3c70;

/// Scans source objects and reads their embedded strings.
pub const STRING_SOURCE_SCAN_RVA: usize = 0x00b3_6c60;

/// Reads a source object's embedded string using a separate caller.
pub const STRING_SOURCE_READ_RVA: usize = 0x00b3_a080;

/// Looks up a source object's embedded string and invokes a reader.
pub const STRING_SOURCE_LOOKUP_RVA: usize = 0x00b3_b940;

/// Updates a dated collection on an unidentified owner.
pub const DATED_RECORD_UPDATE_RVA: usize = 0x01b6_f0c0;

/// Updates a nested object's member and sets its update flag.
pub const INDIRECT_MEMBER_UPDATE_RVA: usize = 0x01b7_3510;

/// Resolves a source object and passes its members to the fixed-window constructor.
pub const FIXED_WINDOW_FACTORY_RVA: usize = 0x022e_22b0;

/// Compiles authored particle subsystem records into distinct output records.
pub const PARTICLE_SUBSYSTEM_COMPILE_RVA: usize = 0x0232_ecc0;

/// Checks attributes recursively on an unidentified record hierarchy.
pub const RECURSIVE_ATTRIBUTE_CHECK_RVA: usize = 0x0232_f270;

/// Reader whose second argument uses the supported embedded-string layout.
pub const EMBEDDED_TEXT_READER_RVA: usize = 0x00b3_d930;

/// Second reader whose second argument uses the supported embedded-string layout.
pub const SECONDARY_EMBEDDED_TEXT_READER_RVA: usize = 0x00b3_40c0;

/// Lookup helper receiving the source's embedded string as its second argument.
pub const STRING_LOOKUP_RVA: usize = 0x00ab_ce70;

/// Copies the member at the shared source and destination displacement.
pub const COPY_MEMBER_RVA: usize = 0x0053_dcc0;

/// Helper called with opaque identifier 0x414d and the +0x3b0 member.
pub const PRIMARY_KEYED_MEMBER_HANDLER_RVA: usize = 0x0068_6760;

/// Helper called with opaque identifier 0x3e91 and the +0x4d0 member.
pub const SECONDARY_KEYED_MEMBER_HANDLER_RVA: usize = 0x0068_6a70;

/// Helper receiving the +0x3b0 member and a temporary value.
pub const TEMPORARY_MEMBER_READER_RVA: usize = 0x0045_82b0;

/// Helper receiving the +0x548 member as its second argument.
pub const STRING_MEMBER_READER_RVA: usize = 0x024a_bb50;

/// Helper receiving a collection base, an element index, and a temporary record.
pub const INDEXED_RECORD_UPDATE_RVA: usize = 0x01b6_1640;

/// Helper receiving the nested object's +0x3b0 member.
pub const INDIRECT_MEMBER_HELPER_RVA: usize = 0x01b6_17b0;

/// Compiles two authored attributes into a member of a different output object.
pub const ATTRIBUTE_COMPILE_RVA: usize = 0x0231_78c0;

/// Helper checking the recursive record's +0x3b0 member.
pub const ATTRIBUTE_CHECK_RVA: usize = 0x0232_f820;
