//! `NProject::CProject` layout retained from archived Domain 41.
//!
//! `PROJECT_PROJECT_OFF_ITERATIONS` and `PROJECT_PROJECT_OFF_HISTORY_COUNT`
//! both named +0x74. The serializer-owned history vector provides the latter
//! interpretation; the old project row called the same bytes iterations.
//! Both candidates are preserved explicitly, not treated as independent fields.
//! Local project ids are unique only within the owning country, as observed
//! by the archived owner-joined lifecycle walk.
//! Sources: `collector/src/attach_log/offsets.rs:1214-1270`,
//! `read_project_row`, `read_country_project_lifecycle`, and
//! `read_project_reward_history_from_bytes` in archived `attach_log.rs`.

/// NProject::CProject primary vtable; confirmed by pinned executable RTTI.
pub const VTABLE_RVA: usize = 0x0296_c270;
/// CProject reference type discriminator 86; checked by archived `project_pin`.
pub const REFERENCE_TYPE: u32 = 86;
/// CProject contiguous pool stride 0x1c0; archived lifecycle walks call this PROJECT_COUNTRY_STRIDE.
pub const POOL_STRIDE: usize = 0x1c0;
/// CProject +0x08 reference type u32; checked by archived project pin and lifecycle walks.
pub const TYPE: usize = 0x08;
/// CProject +0x0c local project id u32; checked by archived project pin and lifecycle walks.
pub const ID: usize = 0x0c;
/// CProject +0x18 static project definition handle u32; checked by archived `project_pin`.
pub const DEFINITION_HANDLE: usize = 0x18;
/// CProject +0x28 CProjectStateMachine pointer; vtable-checked by archived `read_project_state_row`.
pub const STATE_MACHINE: usize = 0x28;
/// CProject +0x60 worked-at reference type u32, checked as CProgram type 83; old OWNER_TAG name was misleading.
pub const PROGRAM_REFERENCE_TYPE: usize = 0x60;
/// CProject +0x64 worked-at CProgram id u32; read by archived `project_row_from_fields`.
pub const PROGRAM_REFERENCE_ID: usize = 0x64;
/// CProject +0x68 SProjectHistory vector data pointer; archived serializer-derived history walker.
pub const HISTORY: usize = 0x68;
/// CProject +0x70 history vector capacity u32; archived history walker checks count <= capacity.
pub const HISTORY_CAPACITY: usize = 0x70;
/// CProject +0x74 history vector count u32 candidate; conflicts with the old iterations interpretation.
pub const HISTORY_COUNT_CANDIDATE: usize = 0x74;
/// CProject +0x74 old project-row iterations u32 candidate; aliases history count and is not separately proved.
pub const ITERATIONS_CONFLICTING_CANDIDATE: usize = 0x74;
/// CProject +0x1bc started flag u8; archived lifecycle walker accepts only 0 or 1.
pub const STARTED: usize = 0x1bc;
/// CProject +0x1bd completed flag u8; archived lifecycle walker accepts only 0 or 1.
pub const COMPLETED: usize = 0x1bd;
