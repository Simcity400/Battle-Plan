//! NIndustrialOrganisation::COrganisationListItem, the owner of the MIO detail window.
//! Knowledge only: this presentation path is excluded from hourly recording.
//! Sources are preserved in archive/pre-rewrite-2026-09-09 under
//! extractor/native/lanes/dynamic_collection_trace_live/src/.
//! a729_mio_bonus_hook.rs:352-362 follows this reference; the old history callback does not.
//! Exact-build Ghidra job 20260722T073034Z-a28-mio-controller-org-field-confirm-02849eb3,
//! FUN141f0bb10, identifies this class and constructs the embedded detail window.

/// COrganisationListItem primary vtable; FUN141f0bb10 at 141f0bbc8 installs this RTTI identity.
pub const VTABLE_RVA: usize = 0x02a7_d6e0;
/// COrganisationListItem GUI vtable; FUN141f0bb10 at 141f0bbd2 installs it at +0x38.
pub const GUI_VTABLE_RVA: usize = 0x02a7_d750;
/// COrganisationListItem third vtable; FUN141f0bb10 at 141f0bbdd installs it at +0x68.
pub const THIRD_VTABLE_RVA: usize = 0x02a7_d818;
/// COrganisationListItem fourth vtable; FUN141f0bb10 at 141f0bbe8 installs it at +0x70.
pub const FOURTH_VTABLE_RVA: usize = 0x02a7_d830;
/// COrganisationListItem GUI interface at +0x38; FUN141f0bb10 at 141f0bbd9 initializes its vtable.
pub const GUI: usize = 0x38;
/// COrganisationListItem third interface at +0x68; FUN141f0bb10 at 141f0bbe4 initializes its vtable.
pub const THIRD_INTERFACE: usize = 0x68;
/// COrganisationListItem fourth interface at +0x70; FUN141f0bb10 at 141f0bbef initializes its vtable.
pub const FOURTH_INTERFACE: usize = 0x70;
/// COrganisationListItem +0x84 pair of u32 reference words; bonus hook:357 and FUN141f0bb10 confirm this field.
pub const ORGANISATION_REFERENCE: usize = 0x84;
/// COrganisationListItem +0x8c native i32 constructor variant; FUN141f0bb10 decompile:44 stores param_3.
pub const VARIANT: usize = 0x8c;
/// COrganisationListItem +0x90 embedded detail window; FUN141f0bb10 decompile:45 passes this item as its owner.
pub const DETAIL_WINDOW: usize = 0x90;
