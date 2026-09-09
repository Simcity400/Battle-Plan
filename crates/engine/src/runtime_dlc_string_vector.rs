//! DLC enumerator output ABI recovered by the old render roster reader; concrete template name unproven.
//!
//! This is the old reader's authenticated engine-call buffer, not a claim that every CPdxArray has this size.

/// Enumerated std::string element pointer; production_render_submit_live RuntimeDlcStringVector repr(C) begins here.
pub const DATA: usize = 0;
/// Output vector capacity u32; RuntimeDlcStringVector repr(C) places it at +8.
pub const CAPACITY: usize = 8;
/// Output vector count u32; RuntimeDlcStringVector repr(C) places it at +0xc.
pub const COUNT: usize = 0xc;
/// Output vector allocator pointer; sample_runtime_dlc_names initializes this qword before the engine call.
pub const ALLOCATOR: usize = 0x10;
/// Output vector allocator auxiliary qword; sample_runtime_dlc_names initializes this field from the next global.
pub const ALLOCATOR_AUX: usize = 0x18;
/// Output vector ABI byte size; derived from the old RuntimeDlcStringVector repr(C) field layout on x64.
pub const SIZE: usize = 0x20;
