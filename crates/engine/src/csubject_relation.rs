//! CSubjectRelation, a directed overlord-to-subject relation.

/// CSubjectRelation primary vtable; build_profile.rs:349, confirmed by pinned-image RTTI.
pub const VTABLE_RVA: usize = 0x0294_9f18;
/// CSubjectRelation inherited type word; build_profile.rs:629 and the D53 intel-visibility subject check.
pub const TYPE_ID: usize = 0x08;
/// CSubjectRelation expected type word; build_profile.rs:632, D53 intel-visibility reader compares this exact value.
pub const TYPE_VALUE: u32 = 0x30d1;
/// CSubjectRelation overlord country link u32; build_profile.rs:361, D53 read_subject_relation_from_ptr.
pub const OVERLORD: usize = 0x10;
/// CSubjectRelation subject country link u32; build_profile.rs:362, D53 authenticates direction from this pair.
pub const SUBJECT: usize = 0x14;
/// CSubjectRelation start hour i32; build_profile.rs:378 and D53 subject reader.
pub const START_HOUR: usize = 0x20;
/// CSubjectRelation autonomy-state definition pointer; build_profile.rs:363, D53 reads its name at +8.
pub const AUTONOMY_STATE: usize = 0x58;
/// CSubjectRelation raw autonomy ratio i64/100000; build_profile.rs:365, old D53 narrowed it to i32.
pub const AUTONOMY_RATIO: usize = 0x60;

pub const CONSTRUCTOR_RVA: usize = 0x00d0_3810;
