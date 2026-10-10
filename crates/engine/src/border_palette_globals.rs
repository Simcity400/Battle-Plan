//! Border-palette define globals; no enclosing type is established. Each named
//! color has four scattered f32 globals. These identify loaded define storage,
//! independently of compiled or country colors. All scalar, pointer and count
//! sites are zero in the shipped image; values come from effective content files.

/// Loaded selection province r component, f32.
pub const SELECTION_PROVINCE_R_RVA: usize = 0x0331_e95c;
/// Loaded selection province g component, f32.
pub const SELECTION_PROVINCE_G_RVA: usize = 0x0331_e9ec;
/// Loaded selection province b component, f32.
pub const SELECTION_PROVINCE_B_RVA: usize = 0x0331_eaa0;
/// Loaded selection province a component, f32.
pub const SELECTION_PROVINCE_A_RVA: usize = 0x0331_eb2c;

/// Loaded selection state r component, f32.
pub const SELECTION_STATE_R_RVA: usize = 0x0331_ebf0;
/// Loaded selection state g component, f32.
pub const SELECTION_STATE_G_RVA: usize = 0x0331_ec88;
/// Loaded selection state b component, f32.
pub const SELECTION_STATE_B_RVA: usize = 0x0331_ed1c;
/// Loaded selection state a component, f32.
pub const SELECTION_STATE_A_RVA: usize = 0x0331_edb4;

/// Loaded selection supply area r component, f32.
pub const SELECTION_SUPPLY_AREA_R_RVA: usize = 0x0331_ee2c;
/// Loaded selection supply area g component, f32.
pub const SELECTION_SUPPLY_AREA_G_RVA: usize = 0x0331_eec8;
/// Loaded selection supply area b component, f32.
pub const SELECTION_SUPPLY_AREA_B_RVA: usize = 0x0331_ef5c;
/// Loaded selection supply area a component, f32.
pub const SELECTION_SUPPLY_AREA_A_RVA: usize = 0x0331_f034;

/// Loaded selection adjacency rule area r component, f32.
pub const SELECTION_ADJACENCY_RULE_AREA_R_RVA: usize = 0x0331_f0cc;
/// Loaded selection adjacency rule area g component, f32.
pub const SELECTION_ADJACENCY_RULE_AREA_G_RVA: usize = 0x0331_f168;
/// Loaded selection adjacency rule area b component, f32.
pub const SELECTION_ADJACENCY_RULE_AREA_B_RVA: usize = 0x0331_f1cc;
/// Loaded selection adjacency rule area a component, f32.
pub const SELECTION_ADJACENCY_RULE_AREA_A_RVA: usize = 0x0331_f2bc;

/// Loaded selection building area r component, f32.
pub const SELECTION_BUILDING_AREA_R_RVA: usize = 0x0331_f368;
/// Loaded selection building area g component, f32.
pub const SELECTION_BUILDING_AREA_G_RVA: usize = 0x0331_f404;
/// Loaded selection building area b component, f32.
pub const SELECTION_BUILDING_AREA_B_RVA: usize = 0x0331_f48c;
/// Loaded selection building area a component, f32.
pub const SELECTION_BUILDING_AREA_A_RVA: usize = 0x0331_8ab0;

/// Loaded tutorial highlight r component, f32.
pub const TUTORIAL_HIGHLIGHT_R_RVA: usize = 0x0331_8c24;
/// Loaded tutorial highlight g component, f32.
pub const TUTORIAL_HIGHLIGHT_G_RVA: usize = 0x0331_8cc8;
/// Loaded tutorial highlight b component, f32.
pub const TUTORIAL_HIGHLIGHT_B_RVA: usize = 0x0331_8d64;
/// Loaded tutorial highlight a component, f32.
pub const TUTORIAL_HIGHLIGHT_A_RVA: usize = 0x0331_8df4;

/// Loaded demilitarized r component, f32.
pub const DEMILITARIZED_R_RVA: usize = 0x0331_8e94;
/// Loaded demilitarized g component, f32.
pub const DEMILITARIZED_G_RVA: usize = 0x0331_8f30;
/// Loaded demilitarized b component, f32.
pub const DEMILITARIZED_B_RVA: usize = 0x0331_8fc0;
/// Loaded demilitarized a component, f32.
pub const DEMILITARIZED_A_RVA: usize = 0x0331_9040;

/// Loaded border conflict edge r component, f32.
pub const BORDER_CONFLICT_EDGE_R_RVA: usize = 0x0331_90e0;
/// Loaded border conflict edge g component, f32.
pub const BORDER_CONFLICT_EDGE_G_RVA: usize = 0x0331_9178;
/// Loaded border conflict edge b component, f32.
pub const BORDER_CONFLICT_EDGE_B_RVA: usize = 0x0331_9214;
/// Loaded border conflict edge a component, f32.
pub const BORDER_CONFLICT_EDGE_A_RVA: usize = 0x0331_92bc;

/// Loaded border conflict non edge r component, f32.
pub const BORDER_CONFLICT_NON_EDGE_R_RVA: usize = 0x0331_9360;
/// Loaded border conflict non edge g component, f32.
pub const BORDER_CONFLICT_NON_EDGE_G_RVA: usize = 0x0331_93fc;
/// Loaded border conflict non edge b component, f32.
pub const BORDER_CONFLICT_NON_EDGE_B_RVA: usize = 0x0331_94a4;
/// Loaded border conflict non edge a component, f32.
pub const BORDER_CONFLICT_NON_EDGE_A_RVA: usize = 0x0331_9524;

/// Loaded custom-highlight float-array pointer, read in four-component RGBA groups.
pub const CUSTOM_HIGHLIGHTS_DATA_RVA: usize = 0x0331_fae8;
/// Signed float-element count; divide by four for custom colors after ten leading palette entries.
pub const CUSTOM_HIGHLIGHTS_FLOAT_COUNT_RVA: usize = 0x0331_faf4;
