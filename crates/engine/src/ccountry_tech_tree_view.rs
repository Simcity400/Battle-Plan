//! CCountryTechTreeView presentation layout, excluded from hourly recording.
//!
//! These constants describe static layouts and sites; complete live presentation
//! authority is unavailable.

/// CCountryTechTreeView primary vtable.
pub const VTABLE_RVA: usize = 0x029a_43f0;
/// CCountryTechTreeView +28 secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x029a_4480;
/// CCountryTechTreeView +30 graphical vtable.
pub const GRAPHICAL_VTABLE_RVA: usize = 0x029a_4498;
/// CCountryTechTreeView secondary subobject displacement.
pub const SECONDARY_SUBOBJECT: usize = 0x28;
/// CCountryTechTreeView graphical subobject displacement.
pub const GRAPHICAL_SUBOBJECT: usize = 0x30;
/// CCountryTechTreeView constructor.
pub const CONSTRUCTOR_RVA: usize = 0x013b_e7f0;
/// CCountryTechTreeView constructor entry.
pub const CONSTRUCTOR_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x10\x48\x89\x4c\x24\x08\x55\x56\x57\x41\x54\x41";
/// CCountryTechTreeView population commit.
pub const POPULATION_COMMIT_RVA: usize = 0x013c_7410;
/// CCountryTechTreeView population entry.
pub const POPULATION_COMMIT_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x08\x55\x56\x57\x41\x54\x41\x55\x41\x56\x41\x57";
/// CCountryTechTreeView folder materializer.
pub const FOLDER_MATERIALIZER_RVA: usize = 0x013c_1390;
/// CCountryTechTreeView folder materializer entry.
pub const FOLDER_MATERIALIZER_SENTINEL: [u8; 16] =
    *b"\x48\x8b\xc4\x4c\x89\x48\x20\x48\x89\x50\x10\x48\x89\x48\x08\x55";
/// CCountryTechTreeView refresh commit.
pub const REFRESH_COMMIT_RVA: usize = 0x013c_4890;
/// CCountryTechTreeView refresh entry.
pub const REFRESH_COMMIT_SENTINEL: [u8; 16] =
    *b"\x48\x8b\xc4\x48\x89\x48\x08\x55\x53\x56\x57\x41\x54\x41\x55\x41";
/// CCountryTechTreeView item rebind.
pub const ITEM_REBIND_RVA: usize = 0x013c_8b70;
/// CCountryTechTreeView item-rebind entry.
pub const ITEM_REBIND_SENTINEL: [u8; 16] =
    *b"\x40\x53\x57\x41\x54\x41\x55\x48\x83\xec\x58\x4c\x8b\xe9\xe8\xed";
/// Technology-view observer glue helper; its concrete receiver remains unknown.
pub const CALLBACK_OBSERVER_GLUE_RVA: usize = 0x013b_e010;
/// CCountryTechTreeView observer-glue helper entry.
pub const CALLBACK_OBSERVER_GLUE_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x10\x48\x89\x74\x24\x18\x48\x89\x7c\x24\x20\x48";
/// CCountryTechTreeView grid-map insertion helper.
pub const GRID_LOOKUP_MAP_INSERT_RVA: usize = 0x013b_d410;
/// CCountryTechTreeView grid-map insertion entry.
pub const GRID_LOOKUP_MAP_INSERT_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x18\x55\x56\x57\x41\x56\x41\x57\x48\x8b\xec\x48";
/// CCountryTechTreeView item-pool acquisition helper.
pub const ITEM_POOL_ACQUIRE_RVA: usize = 0x013b_dd80;
/// CCountryTechTreeView item-pool acquisition entry.
pub const ITEM_POOL_ACQUIRE_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x10\x48\x89\x6c\x24\x18\x48\x89\x74\x24\x20\x57";
/// CCountryTechTreeView post-population tab bind.
pub const POST_POPULATION_TAB_BIND_RVA: usize = 0x013b_f850;
/// CCountryTechTreeView post-population tab-bind entry.
pub const POST_POPULATION_TAB_BIND_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x08\x48\x89\x74\x24\x18\x48\x89\x7c\x24\x20\x48";
/// CCountryTechTreeView connector-path materializer.
pub const CONNECTOR_PATH_MATERIALIZER_RVA: usize = 0x013c_10a0;
/// CCountryTechTreeView connector-path materializer entry.
pub const CONNECTOR_PATH_MATERIALIZER_SENTINEL: [u8; 16] =
    *b"\x48\x89\x54\x24\x10\x53\x55\x56\x57\x41\x56\x48\x83\xec\x60\x4c";
/// CCountryTechTreeView grid resolver.
pub const GRID_RESOLVER_RVA: usize = 0x013c_3040;
/// CCountryTechTreeView grid-resolver entry.
pub const GRID_RESOLVER_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x18\x48\x89\x4c\x24\x08\x55\x56\x57\x41\x54\x41";
/// CCountryTechTreeView materialized-vector clear.
pub const MATERIALIZED_VECTOR_CLEAR_RVA: usize = 0x013c_4480;
/// CCountryTechTreeView materialized-vector clear entry.
pub const MATERIALIZED_VECTOR_CLEAR_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x18\x56\x48\x83\xec\x20\x8b\x81\x4c\x1a\x00\x00";
/// CCountryTechTreeView internal observer callback labels.
pub const INTERNAL_OBSERVER_CALLBACK_RVAS: [usize; 3] = [0x013c_4550, 0x013c_45b0, 0x013c_4720];
/// CCountryTechTreeView mode u32.
pub const MODE: usize = 0x1aa4;
/// Technology-view technology mode value, not an hourly state flag.
pub const TECHNOLOGY_MODE: u32 = 6;
/// CCountryTechTreeView active-folder pointer array.
pub const ACTIVE_FOLDER_DATA: usize = 0x19f8;
/// Active-folder signed i32 count, requiring nonnegative values.
pub const ACTIVE_FOLDER_COUNT: usize = 0x1a04;
/// Selected-folder signed i32 ordinal, bounded by the active-folder count.
pub const ACTIVE_FOLDER_ORDINAL: usize = 0x1a90;
/// CCountryTechTreeView close_button field.
pub const CLOSE_BUTTON: usize = 0x578;
/// Materialized-vector signed i32 count; field meaning remains unproven.
pub const MATERIALIZED_VECTOR_COUNT: usize = 0x1a4c;
