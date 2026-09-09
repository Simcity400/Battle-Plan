//! CCurrentAutonomyStatus; preserve raw scores rather than the old derived ratio.
//! The old profile copied 0x88 bytes; it does not independently prove allocation size.

/// CCurrentAutonomyStatus primary vtable; build_profile.rs:367 and pinned-image RTTI agree.
pub const VTABLE_RVA: usize = 0x027c_d3a0;
/// CCurrentAutonomyStatus current score raw i64; build_profile.rs:369 and D53 read_subject_autonomy_status.
pub const SCORE: usize = 0x08;
/// CCurrentAutonomyStatus current autonomy-state definition pointer; build_profile.rs:370 and D53 status reader.
pub const STATE: usize = 0x28;
/// CCurrentAutonomyStatus previous autonomy-state definition pointer; build_profile.rs:371 and D53 status reader.
pub const PREVIOUS_STATE: usize = 0x30;
/// CCurrentAutonomyStatus lower score raw i64; build_profile.rs:372 and D53 status reader.
pub const LOWER_SCORE: usize = 0x38;
/// CCurrentAutonomyStatus next autonomy-state definition pointer; build_profile.rs:373 and D53 status reader.
pub const NEXT_STATE: usize = 0x40;
/// CCurrentAutonomyStatus upper score raw i64; build_profile.rs:374 and D53 status reader.
pub const UPPER_SCORE: usize = 0x48;
/// CCurrentAutonomyStatus subject CCountry pointer; build_profile.rs:375, D53 validates it against the subject.
pub const SUBJECT_COUNTRY: usize = 0x50;
/// CCurrentAutonomyStatus last-modified raw date word read u32; build_profile.rs:376, native signedness unproven.
pub const LAST_MODIFIED_HOUR: usize = 0x60;
