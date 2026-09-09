//! CResearchSlot fields from the archived a729 active-research walk.
//!
//! The vtable's MSVC RTTI names CResearchSlot in the pinned executable.
//! Slot progress has a different representation from CTechnology progress.
//! Sources at `archive/pre-rewrite-2026-09-09`: collector
//! `src/attach_log/offsets.rs:330-336` and
//! `src/attach_log.rs::read_country_active_research_slots`.

/// CResearchSlot vtable; offsets.rs:330, old active-slot identity check, and a729 RTTI agree.
pub const VTABLE_RVA: usize = 0x0295_e328;
/// CResearchSlot technology definition handle u32; old active-slot reader compares it to CTechnology+0x08.
pub const TECHNOLOGY_HANDLE: usize = 0x08;
/// CResearchSlot CTechnologyStatus* owner; old active-slot reader validates the parent pointer.
pub const STATUS: usize = 0x10;
/// CResearchSlot CTechnology*; offsets.rs:333 and old active-slot reader confirm a pointer at +0x18.
pub const TECHNOLOGY: usize = 0x18;
/// CResearchSlot progress u32 fixed3; old active-slot reader reads four bytes and divides by 1000.
pub const PROGRESS: usize = 0x38;
/// CResearchSlot ordinal u32; old active-slot reader checks uniqueness and compares to the vector count.
pub const INDEX: usize = 0x40;
/// CResearchSlot idle handle; offsets.rs:336 and old active-slot reader skip this value (build-specific observation).
pub const IDLE_HANDLE: u32 = 0x2a4e;
/// CResearchSlot fixed3 progress scale; offsets.rs:358 and old active-slot reader use 1000.
pub const PROGRESS_SCALE: u32 = 1_000;
