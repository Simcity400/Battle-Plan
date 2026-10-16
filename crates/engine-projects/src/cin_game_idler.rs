//! CInGameIdler, the in-game application owner, distinct from CGameState.
//!
//! In-game activation publishes both owner globals; frontend activation sets
//! only the first. Pause-writer coverage does not establish every indirect write.

/// Active application-idler pointer. In-game activation fills both roots;
/// frontend activation fills only this root.
pub const SINGLETON_A_RVA: usize = 0x0331_72e0;
/// Active in-game idler pointer, cleared by frontend activation.
pub const SINGLETON_B_RVA: usize = 0x0331_72e8;
/// In-game idler primary vtable.
pub const VTABLE_RVA: usize = 0x0295_2510;
/// Graphics task pointer, initially null; update retains, waits and clears it.
pub const GRAPHICS_TASK: usize = 0xa28;

/// CInGameIdler update vtable slot.
pub const UPDATE_SLOT_RVA: usize = 0x0295_2530;
/// CInGameIdler update.
pub const UPDATE_RVA: usize = 0x00db_f240;
/// Ambient-presentation dispatch result combines with input eligibility;
/// full method semantics remain unproven.
pub const AMBIENT_PRESENTATION_GATE_SLOT: usize = 0x68;
/// CInGameIdler map-container accessor slot.
pub const MAP_CONTAINER_ACCESSOR_SLOT: usize = 0x80;
/// Visibility-map accessor dispatch displacement.
pub const VISIBILITY_MAP_ACCESSOR_SLOT: usize = 0x78;
/// Order-edit preview accessor displacement; its result's preview flag is tested, while return type
/// and declaring base remain unproven.
pub const ORDER_EDIT_PREVIEW_ACCESSOR_SLOT: usize = 0xc8;
/// CInGameIdler map-container accessor.
pub const MAP_CONTAINER_ACCESSOR_RVA: usize = 0x0029_e8c0;
/// CInGameIdler GUI registry pointer.
pub const GUI_REGISTRY: usize = 0x4f8;
/// CInGameIdler cursor-source pointer.
pub const CURSOR_SOURCE: usize = 0x500;
/// CInGameIdler action-source pointer.
pub const ACTION_SOURCE: usize = 0x508;
/// CInGameIdler embedded selection manager.
pub const SELECTION_MANAGER: usize = 0x538;
/// CInGameIdler map delegate pointer.
pub const MAP_DELEGATE: usize = 0x530;
/// The map delegate's CGraphicalMap pointer.
pub const GRAPHICAL_MAP: usize = 0x370;
/// CInGameIdler interface-handler pointer.
pub const INTERFACE_HANDLER: usize = 0x6b8;
/// CInGameIdler pause-route byte.
pub const PAUSE_ROUTE: usize = 0x6c0;
/// In-game idler paused byte, requiring zero or one.
pub const PAUSED: usize = 0x6c1;
/// Pause-kind byte, separate from paused state.
pub const PAUSE_KIND: usize = 0x6c2;
/// CInGameIdler forced-pause byte.
pub const PAUSE_FORCED: usize = 0x6c3;
/// Camera-root pointer. Its backlink must name this owner; eye and look-at values correspond to the
/// current render view.
pub const CAMERA: usize = 0x750;
/// Event-head candidate aliases the camera owner pointer; its interpretation remains conflicting.
pub const LEGACY_CAMERA_EVENT_HEAD: usize = 0x758;
/// CInGameIdler alert-manager pointer.
pub const ALERT_MANAGER: usize = 0x798;
/// CInGameIdler tutorial-highlight owner pointer array.
pub const TUTORIAL_HIGHLIGHT_OWNERS: usize = 0x960;
/// CInGameIdler tutorial-highlight owner count i32.
pub const TUTORIAL_HIGHLIGHT_OWNER_COUNT: usize = 0x96c;
/// CInGameIdler tutorial-highlight owner getter.
pub const GET_CURRENT_TUTORIAL_HIGHLIGHT_OWNER_RVA: usize = 0x00db_0bb0;
/// CInGameIdler forced-pause function.
pub const PAUSE_FORCED_RVA: usize = 0x00db_e260;
/// Exact forced-pause entry bytes.
pub const PAUSE_FORCED_SENTINEL: [u8; 16] = [
    0x83, 0xea, 0x09, 0x74, 0x15, 0x83, 0xea, 0x01, 0x74, 0x09, 0x83, 0xfa, 0x03, 0x75, 0x2a, 0x33,
];
/// General pause function, distinct from the player toggle.
pub const PAUSE_ENTRY_RVA: usize = 0x00dc_6520;
/// Exact pause-entry bytes.
pub const PAUSE_ENTRY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
/// Load/setup function also writes paused state.
pub const LOAD_SETUP_RVA: usize = 0x00dc_5310;
/// CInGameIdler non-server pause function.
pub const PAUSE_NON_SERVER_RVA: usize = 0x00dc_bf00;
/// CInGameIdler player pause toggle.
pub const PLAYER_PAUSE_TOGGLE_RVA: usize = 0x00dc_d3e0;
/// CInGameIdler routed player toggle.
pub const PLAYER_PAUSE_ROUTE_RVA: usize = 0x00dc_d770;
/// Pause counter global u32.
pub const PAUSE_COUNTER_RVA: usize = 0x0337_23ec;
/// CInGameIdler vertical camera-input producer.
pub const CAMERA_VERTICAL_INPUT_RVA: usize = 0x00db_a7c0;
/// Exact vertical camera-input entry bytes.
pub const CAMERA_VERTICAL_INPUT_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x88, 0x50, 0x10, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
];
