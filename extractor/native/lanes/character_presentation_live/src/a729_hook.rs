//! Exact-a729 process-local observers for the proven A39 refresh/bind roots.
//!
//! This module intentionally stops before production activation. The hooks
//! retain only short-lived process-local pointers for an authenticated decoder;
//! no pointer is representable in CPR1. The source remains unauthenticated
//! until the post-hook receipt and required mixed fixtures are admitted.

use crate::{
    validate_snapshot, AceListContext, AcePortraitObservation, CharacterPresentationSnapshot,
    ExactCharacterPresentationSource, TraitNodeState, TraitTreeObservation, TraitTreeRoot,
    MAX_TOKEN_BYTES, MAX_TRAIT_NODES,
};
use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released, ExpectedRvaHook,
    HookPatchPlacement, HookTeardownAttestation, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
    A729_EXE_SHA256, BATTLE_PLAN_MODULE_NAME, MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use runtime_text_inputs_live_lane::a729_hook::{
    pin_observed_textbox_context, register_post_setter_observer, register_pre_setter_observer,
    register_sprite_setter_observer, unregister_post_setter_observer,
    unregister_pre_setter_observer, unregister_sprite_setter_observer,
};
use runtime_text_inputs_live_lane::{
    TEXTBOX_DESTRUCTOR_RVA, TEXTBOX_REGISTRATION_RVA, TEXT_SETTER_RVA,
};
use std::collections::BTreeMap;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const TRAIT_REFRESH_RVA: u32 = 0x016B_DC20;
pub const ACE_REFRESH_RVA: u32 = 0x0201_E420;
pub const ACE_ROW_BIND_RVA: u32 = 0x0201_EDC0;
pub const SPRITE_SET_RVA: u32 = 0x0229_A780;
pub const TRAIT_TOOLTIP_FORMATTER_RVA: u32 = 0x01CF_F880;

// Retained exact-a729 ABI/sentinel authorities:
// - trait refresh decompile d44c1228..., instructions ea9ad44f...;
// - ace refresh decompile cfc1c6ea..., instructions 6059b45a...;
// - ace bind decompile abb2ce24..., instructions ae58cf9c...;
// - sprite assignment decompile d26b9550..., instructions d9794c3d...; and
// - trait tooltip formatter decompile 7283c463..., instructions 87bf138e....
// The complete hashes and closed historical job identities are recorded in
// H_A39_PREREQUISITE_ENGINEERING_20260720.md.

/// The current eight-site owner (four local D82 hooks plus four shared A25
/// runtime-text hooks) is UI-conditional. It is not an unconditional
/// simulation source and therefore is not promotable for passive production.
/// A future re-sourced owner must explicitly replace this authority gate; the
/// receipt/custody seams below remain fail-closed in the meantime.
pub const PRODUCTION_ACTIVATION_AUTHORIZED: bool = false;
pub const CURRENT_EIGHT_SITE_UI_OWNER_PROMOTABLE: bool = false;
pub const SHARED_RUNTIME_TYPED_OWNER_CUSTODY_AUTHORIZED: bool = false;

const TRAIT_REFRESH_SENTINEL: [u8; 18] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec, 0x30, 0x49,
    0x63, 0xf0,
];
const ACE_REFRESH_SENTINEL: [u8; 21] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac,
    0x24, 0x28, 0xf0, 0xff, 0xff,
];
const ACE_ROW_BIND_SENTINEL: [u8; 17] = [
    0x48, 0x8b, 0xc4, 0x4c, 0x89, 0x48, 0x20, 0x4c, 0x89, 0x40, 0x18, 0x55, 0x53, 0x56, 0x57, 0x41,
    0x54,
];
const TRAIT_TOOLTIP_FORMATTER_SENTINEL: [u8; 24] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
    0x48, 0x8d, 0xac, 0x24, 0x30, 0xf8, 0xff, 0xff,
];
const SPRITE_SET_SENTINEL: [u8; 20] = [
    0x48, 0x85, 0xc9, 0x0f, 0x84, 0x47, 0x01, 0x00, 0x00, 0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89,
    0x74, 0x24, 0x20, 0x57,
];
const LOCAL_PRODUCTION_HOOK_SPECS: [ExpectedRvaHook; 4] = [
    ExpectedRvaHook {
        logical_name: "presentation.d82.trait_refresh",
        requested_rva: TRAIT_REFRESH_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &TRAIT_REFRESH_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "presentation.d82.ace_refresh",
        requested_rva: ACE_REFRESH_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &ACE_REFRESH_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "presentation.d82.ace_row_bind",
        requested_rva: ACE_ROW_BIND_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &ACE_ROW_BIND_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "presentation.d82.trait_tooltip_formatter",
        requested_rva: TRAIT_TOOLTIP_FORMATTER_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &TRAIT_TOOLTIP_FORMATTER_SENTINEL,
    },
];
const SHARED_SPRITE_PRODUCTION_HOOK_SPEC: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: "presentation.d82.sprite_set",
    requested_rva: SPRITE_SET_RVA as u64,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &SPRITE_SET_SENTINEL,
};
const MAX_ACE_BINDS_PER_REFRESH: usize = 4_096;
const TRAIT_SELECTED_LEADER_REFERENCE: usize = 0x78;
const ACE_OBJECT_ID: usize = 0x0c;
const ACE_ROW_PORTRAIT_WIDGET: usize = 0x48;
const ACE_ROW_MODIFIER_TEXT_WIDGET: usize = 0x60;
const ACE_CONTROLLER_SELECTED_WING_REFERENCE: usize = 0x5a8;
const ACE_CONTROLLER_PLANE_TYPE_MASK: usize = 0x5e4;
const TRAIT_TREE_VECTOR: usize = 0x28;
const TRAIT_TREE_CAPACITY: usize = 0x30;
const TRAIT_TREE_COUNT: usize = 0x34;
const TRAIT_ITEM_DEFINITION: usize = 0x20;
const TRAIT_ITEM_LEADER_REFERENCE: usize = 0x2c;
const TRAIT_ITEM_DISABLED_BUTTON: usize = 0x40;
const TRAIT_ITEM_PICKABLE_BUTTON: usize = 0x598;
const TRAIT_ITEM_PICKED_BUTTON: usize = 0xaf0;
const TRAIT_DEFINITION_KIND: usize = 0x920;
const UNIT_LEADER_TRAIT_XP_VECTOR: usize = 0xdf8;
const UNIT_LEADER_TRAIT_XP_COUNT: usize = 0xe04;
const UNIT_LEADER_TRAIT_XP_ENTRY_BYTES: usize = 0x10;
const GUI_WIDGET_FLAGS: usize = 0x75;
const GUI_WIDGET_HIDDEN: u8 = 0x10;
const REFERENCE_REGISTRY_LARGE_RVA: usize = 0x0343_8e50;
const REFERENCE_REGISTRY_DEFAULT_RVA: usize = 0x0343_8e58;
const REFERENCE_REGISTRY_SMALL_TABLE_RVA: usize = 0x0343_8e60;
const REFERENCE_SMALL_TABLE_COUNT: u32 = 100;
const REFERENCE_DEFAULT_LIMIT: u32 = 0x1269;
const REFERENCE_HASH_MULTIPLIER: u64 = 0x4963_1a45;
const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6e90;
const CGAMESTATE_CURRENT_DATE_OFF: usize = 0x468;

type TraitRefreshFn = unsafe extern "system" fn(usize, usize, i32);
type AceRefreshFn = unsafe extern "system" fn(usize) -> u64;
type AceRowBindFn = unsafe extern "system" fn(usize, usize, usize, usize, u32);
type TraitTooltipFormatterFn = unsafe extern "system" fn(usize, usize, usize) -> u64;

static ORIGINAL_TRAIT_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ACE_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ACE_ROW_BIND: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TRAIT_TOOLTIP_FORMATTER: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_COMPONENT_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static HOOK_SET_PRESENT: AtomicBool = AtomicBool::new(false);
static LIFECYCLE_HEALTHY: AtomicBool = AtomicBool::new(false);
static TEARDOWN_ACTIVE: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static HEALTH_EPOCH: AtomicU64 = AtomicU64::new(1);
static COLLECTOR_GAME_HOUR_RAW: AtomicI32 = AtomicI32::new(i32::MIN);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static FIXTURE_DECODER: OnceLock<Mutex<Option<Arc<dyn ExactA729BoundaryDecoder>>>> =
    OnceLock::new();
static COLLECTOR_DECODER: OnceLock<Arc<CollectorBoundaryDecoder>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static HOOK_RECEIPTS: OnceLock<Mutex<HookReceiptState>> = OnceLock::new();
static PARTIAL_LOCAL_INSTALL_RECEIPTS: OnceLock<
    Mutex<Vec<(ExpectedRvaHook, InstalledRvaHookReceipt)>>,
> = OnceLock::new();
static PARTIAL_LOCAL_RELEASE_RECEIPTS: OnceLock<Mutex<Vec<ReleasedRvaHookReceipt>>> =
    OnceLock::new();

thread_local! {
    static ACTIVE_CALLBACKS: Cell<u8> = const { Cell::new(0) };
    static ACTIVE_ACE_BINDS: RefCell<Option<Vec<ProcessLocalAceBind>>> = const { RefCell::new(None) };
    static ACTIVE_ACE_PORTRAIT_SET: RefCell<Option<ActiveAcePortraitSet>> = const { RefCell::new(None) };
    static ACTIVE_ACE_MODIFIER_SET: RefCell<Option<ActiveAceModifierSet>> = const { RefCell::new(None) };
    static ACTIVE_TRAIT_REASON: RefCell<Option<PendingTraitReason>> = const { RefCell::new(None) };
    static DECODER_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

const CALLBACK_TRAIT_REFRESH: u8 = 1;
const CALLBACK_ACE_REFRESH: u8 = 1 << 1;
const CALLBACK_ACE_BIND: u8 = 1 << 2;
const CALLBACK_SPRITE_SET: u8 = 1 << 3;
const CALLBACK_TRAIT_TOOLTIP_FORMATTER: u8 = 1 << 4;
const CALLBACK_QUIESCE_TIMEOUT: Duration = Duration::from_secs(30);

struct CallbackEpoch;

impl CallbackEpoch {
    fn enter() -> Self {
        IN_FLIGHT_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for CallbackEpoch {
    fn drop(&mut self) {
        IN_FLIGHT_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn capture_gate_open() -> bool {
    INSTALLED.load(Ordering::Acquire)
        && LIFECYCLE_HEALTHY.load(Ordering::Acquire)
        && !TEARDOWN_ACTIVE.load(Ordering::Acquire)
}

fn capture_epoch_witness() -> Option<u64> {
    let epoch = HEALTH_EPOCH.load(Ordering::Acquire);
    (capture_gate_open() && HEALTH_EPOCH.load(Ordering::Acquire) == epoch).then_some(epoch)
}

fn capture_epoch_is_current(epoch: u64) -> bool {
    epoch_witness_matches(
        epoch,
        HEALTH_EPOCH.load(Ordering::Acquire),
        capture_gate_open(),
    )
}

/// Publish the collector's expected raw game-hour label. Hook callbacks
/// independently read CGameState's current date at materialization and accept
/// it only when it equals this value, so an old GUI row can never be relabeled
/// as current evidence.
pub fn publish_collector_game_hour_raw(game_hour_raw: i32) {
    COLLECTOR_GAME_HOUR_RAW.store(game_hour_raw, Ordering::Release);
}

fn observed_game_hour_raw() -> Result<i32, String> {
    let expected = COLLECTOR_GAME_HOUR_RAW.load(Ordering::Acquire);
    if expected == i32::MIN {
        return Err("A39 collector game hour is unavailable".to_string());
    }
    let module_base = MODULE_BASE.load(Ordering::Acquire);
    let game_state_slot = module_base
        .checked_add(CGAMESTATE_SINGLETON_RVA)
        .ok_or_else(|| "A39 CGameState singleton address overflowed".to_string())?;
    let game_state_first = read_usize(game_state_slot)
        .filter(|address| *address != 0)
        .ok_or_else(|| "A39 CGameState singleton was unreadable".to_string())?;
    let game_state_second = read_usize(game_state_slot)
        .filter(|address| *address == game_state_first)
        .ok_or_else(|| "A39 CGameState singleton changed during callback".to_string())?;
    let date_address = game_state_second
        .checked_add(CGAMESTATE_CURRENT_DATE_OFF)
        .ok_or_else(|| "A39 current-date address overflowed".to_string())?;
    let observed_first = read_u32(date_address)
        .map(|raw| raw as i32)
        .ok_or_else(|| "A39 current game hour was unreadable".to_string())?;
    let observed_second = read_u32(date_address)
        .map(|raw| raw as i32)
        .filter(|raw| *raw == observed_first)
        .ok_or_else(|| "A39 current game hour changed during callback".to_string())?;
    read_usize(game_state_slot)
        .filter(|address| *address == game_state_second)
        .ok_or_else(|| "A39 CGameState singleton changed around the date read".to_string())?;
    if observed_second != expected {
        return Err("A39 callback game hour differs from the collector frame".to_string());
    }
    Ok(observed_second)
}

fn epoch_witness_matches(entry_epoch: u64, current_epoch: u64, gate_open: bool) -> bool {
    gate_open && entry_epoch == current_epoch
}

fn same_decoder(
    existing: &Arc<dyn ExactA729BoundaryDecoder>,
    candidate: &Arc<dyn ExactA729BoundaryDecoder>,
) -> bool {
    Arc::ptr_eq(existing, candidate)
}

struct DecoderCallGuard;

impl DecoderCallGuard {
    fn enter() -> Result<Self, String> {
        let nested = DECODER_ACTIVE.with(|active| active.replace(true));
        if nested {
            return Err("A39 fixture decoder re-entered the adapter".into());
        }
        Ok(Self)
    }
}

impl Drop for DecoderCallGuard {
    fn drop(&mut self) {
        DECODER_ACTIVE.with(|active| active.set(false));
    }
}

#[derive(Default)]
struct HookState {
    latest_trait_component: Option<DecodedTraitComponent>,
    latest_ace_component: Option<DecodedAceComponent>,
    trait_refresh_count: u64,
    trait_tooltip_count: u64,
    trait_reason_join_count: u64,
    ace_refresh_count: u64,
    ace_bind_count: u64,
    rejected_observation_count: u64,
    /// Last exact sprite-set observation for a materialized portrait widget.
    /// This is process-local cache state only and is cleared on reinstall.
    ace_portrait_by_widget: BTreeMap<usize, String>,
    /// Current post-refresh process-local item correlation. These addresses
    /// are never copied into CPR1 and are discarded on every new refresh.
    trait_items: BTreeMap<usize, TraitItemLocator>,
    /// Intentional fatal fixture-epoch poison. Any ambiguous callback requires
    /// uninstall/reinstall; a success from the other component cannot revive
    /// stale evidence.
    current_error: Option<String>,
    component_reservation: Option<ComponentPairReservationState>,
}

#[derive(Default)]
struct HookReceiptState {
    installed_local: Option<[InstalledRvaHookReceipt; 4]>,
    installed_shared_runtime: Option<[InstalledRvaHookReceipt; 4]>,
    released_local_progress: [Option<ReleasedRvaHookReceipt>; 4],
    released_local: Option<[ReleasedRvaHookReceipt; 4]>,
    released_shared_sprite: Option<ReleasedRvaHookReceipt>,
}

struct SharedRuntimeObserverGuard {
    armed: bool,
}

impl SharedRuntimeObserverGuard {
    fn begin(module_base: usize) -> Result<Self, String> {
        if shared_runtime_targets_are_original(module_base)? {
            return Err("A39 requires the registry-owned shared-runtime lease first".into());
        }
        // D82 never installs or removes the shared set. The registry/evidence
        // harness must hold the complete A25 four-site lease throughout this
        // observer registration.
        Ok(Self { armed: true })
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SharedRuntimeObserverGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        unregister_a39_shared_observers();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TraitItemLocator {
    tree_index: usize,
    node_index: usize,
    trait_definition_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingTraitReason {
    item_ptr: usize,
    locator: TraitItemLocator,
    expected_text: String,
    matched_owner_widget: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveAcePortraitSet {
    widget_ptr: usize,
    portrait_gfx: Option<String>,
    ambiguous: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveAceModifierSet {
    widget_ptr: usize,
    modifier_text: Option<String>,
    write_count: u32,
    ambiguous: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecodedTraitComponent {
    sequence: u64,
    observed_game_hour_raw: i32,
    identity_catalog_epoch: Option<u64>,
    rows: Vec<TraitTreeObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecodedAceComponent {
    sequence: u64,
    observed_game_hour_raw: i32,
    identity_catalog_epoch: Option<u64>,
    context: AceListContext,
    rows: Vec<AcePortraitObservation>,
}

/// Short-lived input to the not-yet-promoted trait decoder. These addresses
/// are never serialized and must be consumed in the hooked process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalTraitRefresh {
    pub sequence: u64,
    pub observed_game_hour_raw: i32,
    pub controller_ptr: usize,
    /// Exact CReferenceObject pair copied from controller+0x78 after refresh.
    /// It is a process-local join key, not a portable unit-leader identity.
    pub selected_leader_reference_raw: u64,
    /// Exact live object resolved through the read-only 0x14220A3D0 hash law.
    /// It is used only to join the authenticated d47 root registry.
    pub selected_leader_ptr: usize,
    pub refresh_argument_raw: i32,
    /// Exact post-refresh tree/item layout decoded before the callback epoch
    /// closes. Definition pointers remain process-local and must be joined to
    /// the authenticated shared trait-definition registry by the decoder.
    pub trees: Vec<ProcessLocalTraitTree>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalTraitTree {
    pub root: TraitTreeRoot,
    pub nodes: Vec<ProcessLocalTraitNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalTraitNode {
    pub source_ordinal: u32,
    /// Exact live tree-item address retained only for the formatter/setter
    /// transition join. The synchronous decoder may inspect it as a local
    /// lookup key, but it is never copied into its portable rows or CPR1.
    pub item_ptr: usize,
    pub trait_definition_ptr: usize,
    /// Decoded from the exact one-hot visibility law materialized by
    /// 0x141D011A0, not re-evaluated from static prerequisites.
    pub state: TraitNodeState,
    /// Exact raw numerator from the selected leader's +0xDF8/+0xE04 sparse
    /// trait-XP pairs for trait kinds 2/3. `None` is not applicable.
    pub progress_raw: Option<i64>,
    /// Exact definition kind read at `trait_definition_ptr + 0x920`, the same
    /// value that decides `progress_raw`'s applicability. It used to be read
    /// and thrown away; the XUI17 admission needs it per node, and re-reading
    /// it later would be a second observation of a value that can change.
    pub definition_kind: u32,
}

/// One exact row-bind call in post-sort callback order. The row and ace
/// pointers are process-local correlation evidence, not replay identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalAceBind {
    pub visible_ordinal: u32,
    /// Stable CAceEntry+0x0C identity copied while `ace_ptr` is live.
    pub ace_id: u32,
    pub row_ptr: usize,
    pub ace_ptr: usize,
    /// Final sprite key observed at 0x14229A780 while the exact row portrait
    /// widget is bound. This string is pointer-free even though the enclosing
    /// record remains process-local correlation evidence.
    pub portrait_gfx: String,
    /// Exact final newline-delimited modifier string supplied by
    /// 0x14201EDC0 to the row+0x60 textbox. Empty is measured-empty.
    pub modifier_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalAceRefresh {
    pub sequence: u64,
    pub observed_game_hour_raw: i32,
    pub controller_ptr: usize,
    pub context: ProcessLocalAceListContext,
    pub binds: Vec<ProcessLocalAceBind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessLocalAceListContext {
    SelectedWing { wing_ptr: usize },
    PlaneTypeMask { mask_raw: u32 },
}

/// Future receipt-bound identity-decoder seam. The hook has already copied the
/// exact tree layout/state and final portrait string; implementations join only
/// stable owner/leader/trait/ace identities and must never persist an address
/// from either process-local refresh record. A decoder may perform only
/// crash-safe read-only memory/registry joins; it must not call engine/UI code,
/// install hooks, or otherwise re-enter the observed callback graph.
pub trait ExactA729BoundaryDecoder: Send + Sync {
    /// `Some` only for the sealed collector decoder. The adapter samples this
    /// before and after each synchronous decode to reject catalog replacement
    /// during a callback and to prevent cross-epoch trait/ace pairing.
    fn identity_catalog_epoch(&self) -> Option<u64> {
        None
    }

    fn decode_trait_refresh(
        &self,
        boundary: &ProcessLocalTraitRefresh,
    ) -> Result<Vec<TraitTreeObservation>, String>;

    fn decode_ace_refresh(
        &self,
        boundary: &ProcessLocalAceRefresh,
    ) -> Result<(AceListContext, Vec<AcePortraitObservation>), String>;
}

/// Process-local identity joins supplied by the already authenticated d47/d46
/// root registries. Addresses are lookup keys only and can never enter CPR1.
/// Constructing this catalog does not authenticate a source. Fixture decoders
/// may consume an explicitly supplied catalog, while production accepts only
/// the sealed collector-owned decoder and still requires the activation gate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessLocalIdentityCatalog {
    pub leaders_by_address: BTreeMap<usize, (u32, u64)>,
    pub trait_tokens_by_definition: BTreeMap<usize, String>,
    pub aces_by_address: BTreeMap<usize, (u32, u64)>,
    pub wings_by_address: BTreeMap<usize, (u32, u32)>,
}

pub fn decode_trait_rows_with_catalog(
    boundary: &ProcessLocalTraitRefresh,
    catalog: &ProcessLocalIdentityCatalog,
) -> Result<Vec<TraitTreeObservation>, String> {
    let &(owner_tag_db_id, unit_leader_id) = catalog
        .leaders_by_address
        .get(&boundary.selected_leader_ptr)
        .ok_or_else(|| {
            "A39 selected leader address is absent from exact d47 registry".to_string()
        })?;
    if owner_tag_db_id == 0 || unit_leader_id == 0 {
        return Err("A39 d47 leader registry carried a zero identity".into());
    }
    let mut rows = Vec::with_capacity(boundary.trees.len());
    for tree in &boundary.trees {
        let mut nodes = Vec::with_capacity(tree.nodes.len());
        for (expected_ordinal, node) in tree.nodes.iter().enumerate() {
            if node.source_ordinal != expected_ordinal as u32 {
                return Err("A39 process-local trait source ordinal is non-contiguous".into());
            }
            let trait_token = catalog
                .trait_tokens_by_definition
                .get(&node.trait_definition_ptr)
                .filter(|token| !token.trim().is_empty() && token.len() <= MAX_TOKEN_BYTES)
                .cloned()
                .ok_or_else(|| {
                    "A39 trait definition is absent from exact d47 registry".to_string()
                })?;
            nodes.push(crate::TraitNodeObservation {
                trait_token,
                state: node.state,
                progress_raw: node.progress_raw,
                definition_kind: node.definition_kind,
                reason_context_id: None,
            });
        }
        rows.push(TraitTreeObservation {
            owner_tag_db_id,
            unit_leader_id,
            root: tree.root,
            nodes,
        });
    }
    validate_snapshot(&CharacterPresentationSnapshot {
        source_sequence: boundary.sequence,
        trait_trees: rows.clone(),
        ace_context: AceListContext::PlaneTypeMask { mask_raw: 1 },
        ace_portraits: Vec::new(),
    })
    .map_err(|error| format!("{error:?}"))?;
    Ok(rows)
}

pub fn decode_ace_rows_with_catalog(
    boundary: &ProcessLocalAceRefresh,
    catalog: &ProcessLocalIdentityCatalog,
) -> Result<Vec<AcePortraitObservation>, String> {
    let context = decode_ace_context_with_catalog(&boundary.context, catalog)?;
    let mut rows = Vec::with_capacity(boundary.binds.len());
    for (expected_ordinal, bind) in boundary.binds.iter().enumerate() {
        if bind.visible_ordinal != expected_ordinal as u32 {
            return Err("A39 process-local ace source ordinal is non-contiguous".into());
        }
        let &(owner_tag_db_id, ace_id) = catalog
            .aces_by_address
            .get(&bind.ace_ptr)
            .ok_or_else(|| "A39 ace pointer is absent from exact d46 registry".to_string())?;
        if owner_tag_db_id == 0 || ace_id == 0 || ace_id != u64::from(bind.ace_id) {
            return Err("A39 d46 ace registry contradicts the bound ace identity".into());
        }
        rows.push(AcePortraitObservation {
            owner_tag_db_id,
            ace_id,
            visible_ordinal: bind.visible_ordinal,
            portrait_gfx: bind.portrait_gfx.clone(),
            modifier_text: bind.modifier_text.clone(),
        });
    }
    validate_snapshot(&CharacterPresentationSnapshot {
        source_sequence: boundary.sequence,
        // Component-local validation supplies the two carrier-required roots
        // only as an internal shape witness. The combined snapshot later
        // replaces these placeholders with the authenticated trait component.
        trait_trees: vec![
            TraitTreeObservation {
                owner_tag_db_id: 1,
                unit_leader_id: 1,
                root: TraitTreeRoot::ControllerOffset120,
                nodes: Vec::new(),
            },
            TraitTreeObservation {
                owner_tag_db_id: 1,
                unit_leader_id: 1,
                root: TraitTreeRoot::ControllerOffset130,
                nodes: Vec::new(),
            },
        ],
        ace_context: context,
        ace_portraits: rows.clone(),
    })
    .map_err(|error| format!("{error:?}"))?;
    Ok(rows)
}

fn decode_ace_context_with_catalog(
    context: &ProcessLocalAceListContext,
    catalog: &ProcessLocalIdentityCatalog,
) -> Result<AceListContext, String> {
    match context {
        ProcessLocalAceListContext::SelectedWing { wing_ptr } => {
            let &(owner_tag_db_id, wing_id) =
                catalog.wings_by_address.get(wing_ptr).ok_or_else(|| {
                    "A39 selected wing is absent from exact air-wing registry".to_string()
                })?;
            if owner_tag_db_id == 0 || wing_id == 0 {
                return Err("A39 selected-wing registry carried a zero identity".into());
            }
            Ok(AceListContext::SelectedWing {
                owner_tag_db_id,
                wing_id,
            })
        }
        ProcessLocalAceListContext::PlaneTypeMask { mask_raw } if *mask_raw != 0 => {
            Ok(AceListContext::PlaneTypeMask {
                mask_raw: *mask_raw,
            })
        }
        ProcessLocalAceListContext::PlaneTypeMask { .. } => {
            Err("A39 plane-type context mask is zero".into())
        }
    }
}

/// Concrete fixture decoder for an immutable authenticated root snapshot. The
/// caller must rebuild it when d47/d46 root membership changes; stale or
/// missing entries reject the entire fixture epoch.
#[derive(Clone, Debug)]
pub struct CatalogBoundaryDecoder {
    catalog: Arc<ProcessLocalIdentityCatalog>,
}

/// Sealed collector-owned decoder whose catalog is atomically replaced from
/// one complete d47+d46 root-read epoch. Callback readers clone one immutable
/// snapshot, so a collector refresh can never tear an in-flight UI decode.
/// This is not an authentication switch: production hook/source activation
/// remains governed by `PRODUCTION_ACTIVATION_AUTHORIZED`.
#[derive(Debug, Default)]
pub struct CollectorBoundaryDecoder {
    state: RwLock<CollectorCatalogState>,
}

#[derive(Debug, Default)]
struct CollectorCatalogState {
    epoch: u64,
    catalog: Option<Arc<ProcessLocalIdentityCatalog>>,
}

impl CollectorBoundaryDecoder {
    pub fn publish(&self, catalog: ProcessLocalIdentityCatalog) -> Result<bool, String> {
        validate_identity_catalog(&catalog)?;
        let mut state = self
            .state
            .write()
            .map_err(|_| "A39 collector identity catalog lock poisoned")?;
        if state.catalog.as_deref() == Some(&catalog) {
            return Ok(false);
        }
        state.epoch = state.epoch.saturating_add(1).max(1);
        state.catalog = Some(Arc::new(catalog));
        Ok(true)
    }

    /// Any incomplete d47/d46 root epoch removes the prior catalog. Retaining
    /// stale process addresses would turn known source loss into false identity.
    pub fn invalidate(&self) -> bool {
        if let Ok(mut state) = self.state.write() {
            if state.catalog.take().is_some() {
                state.epoch = state.epoch.saturating_add(1).max(1);
                return true;
            }
        }
        false
    }

    fn snapshot(&self) -> Result<(u64, Arc<ProcessLocalIdentityCatalog>), String> {
        let state = self
            .state
            .read()
            .map_err(|_| "A39 collector identity catalog lock poisoned")?;
        let catalog = state
            .catalog
            .as_ref()
            .cloned()
            .ok_or_else(|| "A39 collector identity catalog is unavailable".to_string())?;
        Ok((state.epoch, catalog))
    }
}

impl ExactA729BoundaryDecoder for CollectorBoundaryDecoder {
    fn identity_catalog_epoch(&self) -> Option<u64> {
        self.snapshot().ok().map(|(epoch, _)| epoch)
    }

    fn decode_trait_refresh(
        &self,
        boundary: &ProcessLocalTraitRefresh,
    ) -> Result<Vec<TraitTreeObservation>, String> {
        let (_, catalog) = self.snapshot()?;
        decode_trait_rows_with_catalog(boundary, &catalog)
    }

    fn decode_ace_refresh(
        &self,
        boundary: &ProcessLocalAceRefresh,
    ) -> Result<(AceListContext, Vec<AcePortraitObservation>), String> {
        let (_, catalog) = self.snapshot()?;
        let context = decode_ace_context_with_catalog(&boundary.context, &catalog)?;
        Ok((context, decode_ace_rows_with_catalog(boundary, &catalog)?))
    }
}

fn validate_identity_catalog(catalog: &ProcessLocalIdentityCatalog) -> Result<(), String> {
    if catalog
        .leaders_by_address
        .iter()
        .any(|(address, (owner, leader))| *address == 0 || *owner == 0 || *leader == 0)
    {
        return Err("A39 collector catalog contains an invalid leader identity".into());
    }
    if catalog
        .trait_tokens_by_definition
        .iter()
        .any(|(address, token)| {
            *address == 0 || token.trim().is_empty() || token.len() > MAX_TOKEN_BYTES
        })
    {
        return Err("A39 collector catalog contains an invalid trait identity".into());
    }
    if catalog
        .aces_by_address
        .iter()
        .any(|(address, (owner, ace))| *address == 0 || *owner == 0 || *ace == 0)
    {
        return Err("A39 collector catalog contains an invalid ace identity".into());
    }
    if catalog
        .wings_by_address
        .iter()
        .any(|(address, (owner, wing))| *address == 0 || *owner == 0 || *wing == 0)
    {
        return Err("A39 collector catalog contains an invalid wing identity".into());
    }
    Ok(())
}

pub fn collector_boundary_decoder() -> Arc<CollectorBoundaryDecoder> {
    Arc::clone(COLLECTOR_DECODER.get_or_init(|| Arc::new(CollectorBoundaryDecoder::default())))
}

pub fn publish_collector_identity_catalog(
    catalog: ProcessLocalIdentityCatalog,
) -> Result<(), String> {
    if collector_boundary_decoder().publish(catalog)? {
        clear_decoded_components_for_catalog_change();
    }
    Ok(())
}

pub fn invalidate_collector_identity_catalog() {
    if collector_boundary_decoder().invalidate() {
        clear_decoded_components_for_catalog_change();
    }
}

fn clear_decoded_components_for_catalog_change() {
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.latest_trait_component = None;
        state.latest_ace_component = None;
    }
}

impl CatalogBoundaryDecoder {
    pub fn new(catalog: ProcessLocalIdentityCatalog) -> Self {
        Self {
            catalog: Arc::new(catalog),
        }
    }
}

impl ExactA729BoundaryDecoder for CatalogBoundaryDecoder {
    fn decode_trait_refresh(
        &self,
        boundary: &ProcessLocalTraitRefresh,
    ) -> Result<Vec<TraitTreeObservation>, String> {
        decode_trait_rows_with_catalog(boundary, &self.catalog)
    }

    fn decode_ace_refresh(
        &self,
        boundary: &ProcessLocalAceRefresh,
    ) -> Result<(AceListContext, Vec<AcePortraitObservation>), String> {
        let context = decode_ace_context_with_catalog(&boundary.context, &self.catalog)?;
        Ok((
            context,
            decode_ace_rows_with_catalog(boundary, &self.catalog)?,
        ))
    }
}

/// Reserved d82 source. It is useful to the capture harness now, but cannot
/// authenticate as a production source while the promotion constant is false.
#[derive(Clone, Copy, Debug)]
pub struct A729ObservedCharacterPresentationSource {
    expected_game_hour_raw: i32,
}

impl A729ObservedCharacterPresentationSource {
    pub const fn new(expected_game_hour_raw: i32) -> Self {
        Self {
            expected_game_hour_raw,
        }
    }

    /// Harness-only candidate decode used to generate the missing mixed
    /// fixtures. This never changes the source authentication result.
    pub fn decode_fixture_candidate(&self) -> Result<CharacterPresentationSnapshot, String> {
        let reservation = peek_character_presentation_candidate(self.expected_game_hour_raw)?
            .ok_or("A39 decoded trait/ace component pair is incomplete")?;
        let snapshot = reservation.snapshot.clone();
        commit_character_presentation_candidate(reservation.reservation_id)?;
        Ok(snapshot)
    }
}

impl ExactCharacterPresentationSource for A729ObservedCharacterPresentationSource {
    fn is_exact_a729_character_presentation_source(&self) -> bool {
        // The legacy trait cannot carry a publication reservation across its
        // caller's encode/stage boundary. Production must use the explicit
        // peek/commit/rollback API below.
        false
    }

    fn read_character_presentation(&self) -> Result<CharacterPresentationSnapshot, String> {
        Err("A39 production requires transactional peek/commit/rollback publication custody".into())
    }
}

pub fn production_transaction_ready() -> bool {
    if !PRODUCTION_ACTIVATION_AUTHORIZED
        || !INSTALLED.load(Ordering::Acquire)
        || !LIFECYCLE_HEALTHY.load(Ordering::Acquire)
        || installed_hook_receipts().is_err()
    {
        return false;
    }
    let registered = FIXTURE_DECODER
        .get()
        .and_then(|slot| slot.lock().ok())
        .and_then(|slot| slot.as_ref().cloned());
    let collector: Arc<dyn ExactA729BoundaryDecoder> = collector_boundary_decoder();
    registered.is_some_and(|registered| same_decoder(&registered, &collector))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledAddresses {
    pub trait_refresh: usize,
    pub ace_refresh: usize,
    pub ace_row_bind: usize,
    pub sprite_set: usize,
    pub trait_tooltip_formatter: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub installed_for_fixture_capture: bool,
    pub lifecycle_healthy: bool,
    pub teardown_active: bool,
    pub in_flight_callbacks: usize,
    pub production_activation_authorized: bool,
    pub trait_refresh_count: u64,
    pub trait_tooltip_count: u64,
    pub trait_reason_join_count: u64,
    pub ace_refresh_count: u64,
    pub ace_bind_count: u64,
    pub rejected_observation_count: u64,
    pub has_complete_boundary_batch: bool,
    pub current_error: Option<String>,
}

/// Production entry point. It cannot install until the evidence gate changes.
pub fn install() -> Result<InstalledAddresses, String> {
    if !PRODUCTION_ACTIVATION_AUTHORIZED {
        return Err(
            "A39 production hook activation is not authorized; use the capture harness entry point"
                .into(),
        );
    }
    register_decoder(collector_boundary_decoder())?;
    install_hook_set(true)
}

#[derive(Clone, Debug)]
struct ComponentPairReservationState {
    reservation_id: u64,
    active: bool,
    trait_component: DecodedTraitComponent,
    ace_component: DecodedAceComponent,
}

impl HookState {
    fn reserve_component_pair(
        &mut self,
        reservation_id: u64,
    ) -> Result<Option<(u64, DecodedTraitComponent, DecodedAceComponent)>, String> {
        if let Some(reservation) = self.component_reservation.as_mut() {
            if reservation.active {
                return Err(format!(
                    "A39 component reservation {} is already active",
                    reservation.reservation_id
                ));
            }
            reservation.active = true;
            return Ok(Some((
                reservation.reservation_id,
                reservation.trait_component.clone(),
                reservation.ace_component.clone(),
            )));
        }
        if self.latest_trait_component.is_none() || self.latest_ace_component.is_none() {
            return Ok(None);
        }
        let trait_component = self.latest_trait_component.take().unwrap();
        let ace_component = self.latest_ace_component.take().unwrap();
        if reservation_id == 0 || reservation_id == u64::MAX {
            self.latest_trait_component = Some(trait_component);
            self.latest_ace_component = Some(ace_component);
            return Err("A39 component reservation id space exhausted".into());
        }
        self.component_reservation = Some(ComponentPairReservationState {
            reservation_id,
            active: true,
            trait_component: trait_component.clone(),
            ace_component: ace_component.clone(),
        });
        Ok(Some((reservation_id, trait_component, ace_component)))
    }

    fn commit_component_pair(&mut self, reservation_id: u64) -> Result<(), String> {
        let reservation = self
            .component_reservation
            .as_ref()
            .ok_or_else(|| format!("A39 has no active component reservation {reservation_id}"))?;
        if reservation.reservation_id != reservation_id || !reservation.active {
            return Err(format!(
                "A39 component reservation mismatch: active={} requested={reservation_id}",
                reservation.reservation_id
            ));
        }
        self.component_reservation = None;
        Ok(())
    }

    fn rollback_component_pair(&mut self, reservation_id: u64) -> Result<(), String> {
        let reservation = self
            .component_reservation
            .as_mut()
            .ok_or_else(|| format!("A39 has no active component reservation {reservation_id}"))?;
        if reservation.reservation_id != reservation_id || !reservation.active {
            return Err(format!(
                "A39 component reservation mismatch: active={} requested={reservation_id}",
                reservation.reservation_id
            ));
        }
        reservation.active = false;
        Ok(())
    }
}

/// Pointer-free D82 candidate held under a bounded single reservation.
///
/// The caller may encode and stage this snapshot without removing the source
/// pair. It must commit only after successful publication, or roll back on any
/// failure so the same pair remains first in order ahead of newer callbacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterPresentationCandidateReservation {
    pub reservation_id: u64,
    pub snapshot: CharacterPresentationSnapshot,
}

/// Explicit harness-only entry point for collecting the receipts and mixed
/// fixtures required to promote d82. Every root is sentinel-gated before any
/// detour is created, and partial installation is rolled back.
#[cfg(feature = "a729-fixture-hook")]
pub fn install_for_fixture_capture(
    decoder: Arc<dyn ExactA729BoundaryDecoder>,
) -> Result<InstalledAddresses, String> {
    register_decoder(decoder)?;
    let result = install_hook_set(false);
    if result.is_err() && !HOOK_SET_PRESENT.load(Ordering::Acquire) {
        if let Some(slot) = FIXTURE_DECODER.get() {
            if let Ok(mut slot) = slot.lock() {
                *slot = None;
            }
        }
    }
    result
}

fn register_decoder(decoder: Arc<dyn ExactA729BoundaryDecoder>) -> Result<(), String> {
    let slot = FIXTURE_DECODER.get_or_init(|| Mutex::new(None));
    {
        let mut slot = slot
            .lock()
            .map_err(|_| "A39 fixture decoder mutex poisoned")?;
        if let Some(existing) = slot.as_ref() {
            if !same_decoder(existing, &decoder) {
                return Err("A39 a different fixture decoder is already registered".into());
            }
        } else {
            *slot = Some(decoder);
        }
    }
    Ok(())
}

fn install_hook_set(production_receipts: bool) -> Result<InstalledAddresses, String> {
    let _install_guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A39 install mutex poisoned")?;
    if INSTALLED.load(Ordering::Acquire)
        && LIFECYCLE_HEALTHY.load(Ordering::Acquire)
        && MODULE_BASE.load(Ordering::Acquire) != 0
        && originals_ready()
    {
        let resolved = addresses(MODULE_BASE.load(Ordering::Acquire));
        if production_receipts {
            if local_installed_hook_receipts_available() {
                reobserve_local_install_receipts(resolved)?;
            } else {
                observe_install_receipts(resolved)?;
            }
        }
        return Ok(resolved);
    }
    if HOOK_SET_PRESENT.load(Ordering::Acquire) {
        return Err(
            "A39 hook set is lifecycle-poisoned; complete uninstall before reinstalling".into(),
        );
    }
    if FIXTURE_DECODER
        .get()
        .and_then(|slot| slot.lock().ok())
        .and_then(|slot| slot.as_ref().cloned())
        .is_none()
    {
        return Err("A39 fixture decoder is not registered".into());
    }
    if production_receipts {
        ensure_receipt_install_admissible()?;
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A39 GetModuleHandleW(NULL) returned null".into());
    }
    let resolved = addresses(base);
    for (address, sentinel, label) in [
        (
            resolved.trait_refresh,
            &TRAIT_REFRESH_SENTINEL[..],
            "trait refresh",
        ),
        (
            resolved.ace_refresh,
            &ACE_REFRESH_SENTINEL[..],
            "ace refresh",
        ),
        (
            resolved.ace_row_bind,
            &ACE_ROW_BIND_SENTINEL[..],
            "ace row bind",
        ),
        (
            resolved.trait_tooltip_formatter,
            &TRAIT_TOOLTIP_FORMATTER_SENTINEL[..],
            "trait tooltip formatter",
        ),
    ] {
        verify_sentinel(address, sentinel, label)?;
    }
    let mut shared_runtime_guard = SharedRuntimeObserverGuard::begin(base)?;
    register_pre_setter_observer(a39_pre_text_setter_observer)
        .map_err(|error| format!("A39 shared A25 pre-setter observer failed: {error}"))?;
    register_post_setter_observer(a39_post_text_setter_observer)
        .map_err(|error| format!("A39 shared A25 post-setter observer failed: {error}"))?;
    register_sprite_setter_observer(a39_sprite_setter_observer)
        .map_err(|error| format!("A39 shared sprite observer failed: {error}"))?;
    let state = STATE.get_or_init(|| Mutex::new(HookState::default()));
    *state.lock().map_err(|_| "A39 hook state poisoned")? = HookState::default();
    MODULE_BASE.store(base, Ordering::Release);
    LIFECYCLE_HEALTHY.store(false, Ordering::Release);
    let specs = [
        (
            resolved.trait_refresh,
            hook_trait_refresh as *mut c_void,
            &ORIGINAL_TRAIT_REFRESH,
        ),
        (
            resolved.ace_refresh,
            hook_ace_refresh as *mut c_void,
            &ORIGINAL_ACE_REFRESH,
        ),
        (
            resolved.ace_row_bind,
            hook_ace_row_bind as *mut c_void,
            &ORIGINAL_ACE_ROW_BIND,
        ),
        (
            resolved.trait_tooltip_formatter,
            hook_trait_tooltip_formatter as *mut c_void,
            &ORIGINAL_TRAIT_TOOLTIP_FORMATTER,
        ),
    ];
    HOOK_SET_PRESENT.store(true, Ordering::Release);
    for (target, detour, original) in specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(value) => {
                original.store(value as usize, Ordering::Release);
            }
            Err(status) => {
                TEARDOWN_ACTIVE.store(true, Ordering::Release);
                HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
                let rollback = disable_and_remove(resolved);
                if rollback.all_removed && IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) == 0 {
                    clear_removed_hook_set();
                }
                return Err(match rollback.error {
                    Some(rollback_error) => {
                        format!("A39 {target:#x} hook creation failed: {status}; rollback anomaly: {rollback_error}")
                    }
                    None => format!("A39 {target:#x} hook creation failed: {status}"),
                });
            }
        }
    }
    for target in targets(resolved) {
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            TEARDOWN_ACTIVE.store(true, Ordering::Release);
            HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
            let rollback = disable_and_remove(resolved);
            if rollback.all_removed && IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) == 0 {
                clear_removed_hook_set();
            }
            return Err(match rollback.error {
                Some(rollback_error) => {
                    format!("A39 {target:#x} hook enable failed: {status}; rollback anomaly: {rollback_error}")
                }
                None => format!("A39 {target:#x} hook enable failed: {status}"),
            });
        }
    }
    if production_receipts {
        if let Err(error) = observe_install_receipts(resolved) {
            TEARDOWN_ACTIVE.store(true, Ordering::Release);
            HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
            let rollback = disable_and_remove(resolved);
            let partial_release_error =
                if rollback.all_removed && IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) == 0 {
                    release_partial_local_install_receipts().err()
                } else {
                    None
                };
            if rollback.all_removed && IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) == 0 {
                clear_removed_hook_set();
            }
            if let Some(partial_release_error) = partial_release_error {
                return Err(format!(
                    "A39 exact installed-hook receipt failed: {error}; partial receipt rollback failed: {partial_release_error}"
                ));
            }
            return Err(match rollback.error {
                Some(rollback_error) => {
                    format!("A39 exact installed-hook receipt failed: {error}; rollback anomaly: {rollback_error}")
                }
                None => format!("A39 exact installed-hook receipt failed: {error}"),
            });
        }
    }
    *state.lock().map_err(|_| "A39 hook state poisoned")? = HookState::default();
    HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
    INSTALLED.store(true, Ordering::Release);
    LIFECYCLE_HEALTHY.store(true, Ordering::Release);
    shared_runtime_guard.disarm();
    Ok(resolved)
}

pub fn uninstall() -> Result<(), String> {
    let _install_guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A39 install mutex poisoned")?;
    if !HOOK_SET_PRESENT.load(Ordering::Acquire) {
        // Hook restoration may have succeeded while receipt observation
        // failed. Preserve retryable custody instead of silently converting
        // that failure into success on the next teardown call.
        observe_local_release_receipts_if_owned()?;
        release_partial_local_install_receipts()?;
        return Ok(());
    }
    let resolved = addresses(MODULE_BASE.load(Ordering::Acquire));
    // Close admission before inspecting reservation custody. A reserving
    // caller rechecks these gates while holding STATE, so it cannot enter
    // after this transition.
    TEARDOWN_ACTIVE.store(true, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    LIFECYCLE_HEALTHY.store(false, Ordering::Release);
    HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
    if STATE
        .get()
        .and_then(|state| state.lock().ok())
        .and_then(|state| state.component_reservation.as_ref().map(|_| true))
        .unwrap_or(false)
    {
        return Err(
            "A39 teardown refused while component publication custody is unresolved".into(),
        );
    }
    ACTIVE_TRAIT_REASON.with(|slot| *slot.borrow_mut() = None);
    unregister_a39_shared_observers();
    let removal = disable_and_remove(resolved);
    let in_flight = IN_FLIGHT_CALLBACKS.load(Ordering::Acquire);
    let release_receipt_error = if removal.all_removed && in_flight == 0 {
        observe_local_release_receipts_if_owned()
            .and_then(|_| release_partial_local_install_receipts())
            .err()
    } else {
        None
    };
    if removal.all_removed && in_flight == 0 {
        clear_removed_hook_set();
        if let Some(state) = STATE.get() {
            *state.lock().map_err(|_| "A39 hook state poisoned")? = HookState::default();
        }
    }
    if let Some(error) = removal.error {
        // Keep the trampolines and module base intact: a failed disable/remove
        // may have left a detour callable unless every remove succeeded.
        return Err(error);
    }
    if let Some(error) = release_receipt_error {
        return Err(format!(
            "A39 local hooks were restored but exact release receipt failed: {error}"
        ));
    }
    if in_flight != 0 {
        return Err(format!(
            "A39 teardown has {in_flight} in-flight callback(s); retry uninstall after quiescence"
        ));
    }
    if !removal.all_removed {
        return Err("A39 hook removal was incomplete without a MinHook status".into());
    }
    Ok(())
}

/// Bind the shared owner's complete four-site installed receipt set and retain
/// only D82's non-owning sprite observation. The shared owner has already
/// authenticated each MinHook relay against its actual detour.
pub fn bind_shared_runtime_installed_receipts(
    shared_runtime_installed_receipts: &[InstalledRvaHookReceipt; 4],
) -> Result<(), String> {
    if !SHARED_RUNTIME_TYPED_OWNER_CUSTODY_AUTHORIZED {
        return Err(
            "D82 shared runtime owner custody is not implemented or promotion-authorized".into(),
        );
    }
    validate_shared_runtime_install_custody(shared_runtime_installed_receipts)?;
    let mut receipts = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "D82 production hook receipt mutex poisoned")?;
    if receipts.installed_local.is_none() || receipts.released_local.is_some() {
        return Err("D82 shared runtime custody cannot bind before local install".into());
    }
    if receipts
        .installed_shared_runtime
        .is_some_and(|installed| installed != *shared_runtime_installed_receipts)
    {
        return Err("D82 shared runtime installed custody changed without release".into());
    }
    receipts.installed_shared_runtime = Some(*shared_runtime_installed_receipts);
    receipts.released_shared_sprite = None;
    Ok(())
}

/// Exact enabled receipts for D82's four locally owned hooks plus its
/// non-owning observation of the shared A25 sprite-set hook.
///
/// This five-entry D82 observation is not the complete active-hook census:
/// `install_a729_runtime_text_hooks` also owns registration, setter, and
/// destructor detours. The fifth receipt does not transfer removal ownership
/// to D82.
pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; 5], String> {
    let receipts = HOOK_RECEIPTS
        .get()
        .ok_or("D82 production hook receipts are not initialized")?
        .lock()
        .map_err(|_| "D82 production hook receipt mutex poisoned")?;
    let local = receipts
        .installed_local
        .ok_or("D82 exact local installed-hook receipts are unavailable")?;
    let shared = receipts
        .installed_shared_runtime
        .as_ref()
        .and_then(|installed| {
            installed
                .iter()
                .find(|receipt| receipt.observed_rva == SPRITE_SET_RVA as u64)
        })
        .copied()
        .ok_or("D82 exact shared sprite installed-hook receipt is unavailable")?;
    Ok([local[0], local[1], local[2], local[3], shared])
}

/// Observe the shared sprite target only after the A25 owner has uninstalled
/// its complete hook set. This refuses to manufacture a complete D82 release
/// while the shared target is still patched.
pub fn finalize_shared_sprite_release_receipt(
    shared_runtime_release_receipts: &[ReleasedRvaHookReceipt; 4],
) -> Result<(), String> {
    if !SHARED_RUNTIME_TYPED_OWNER_CUSTODY_AUTHORIZED {
        return Err(
            "D82 shared runtime owner custody is not implemented or promotion-authorized".into(),
        );
    }
    validate_shared_runtime_release_custody(shared_runtime_release_receipts)?;
    let receipts = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let mut receipts = receipts
        .lock()
        .map_err(|_| "D82 production hook receipt mutex poisoned")?;
    if receipts.released_shared_sprite.is_some() {
        return Ok(());
    }
    if receipts.installed_local.is_some() || receipts.released_local.is_none() {
        return Err("D82 shared runtime release requires completed local teardown first".into());
    }
    let installed_shared_runtime = receipts
        .installed_shared_runtime
        .ok_or("D82 shared runtime installed-hook custody is unavailable")?;
    for installed in installed_shared_runtime {
        let released = shared_runtime_release_receipts
            .iter()
            .find(|receipt| {
                receipt.observed_rva == installed.observed_rva
                    && receipt.teardown_owner_token == installed.teardown_owner_token
            })
            .ok_or("D82 shared runtime release does not match installed custody")?;
        if released.logical_name != installed.logical_name
            || released.observed_module_name != installed.observed_module_name
            || released.observed_module_sha256 != installed.observed_module_sha256
            || released.observed_rva != installed.observed_rva
            || released.observed_runtime_address != installed.observed_runtime_address
            || released.target_patch_len != installed.target_patch_len
            || released.primitive_id != installed.primitive_id
        {
            return Err(
                "D82 shared runtime release identity changed from installed custody".into(),
            );
        }
    }
    let installed = installed_shared_runtime
        .iter()
        .find(|receipt| receipt.observed_rva == SPRITE_SET_RVA as u64)
        .copied()
        .ok_or("D82 shared runtime installed custody omitted sprite target")?;
    let released = shared_runtime_release_receipts
        .iter()
        .find(|receipt| {
            receipt.observed_rva == SPRITE_SET_RVA as u64
                && receipt.teardown_owner_token == installed.teardown_owner_token
        })
        .copied()
        .ok_or("D82 shared sprite release does not match installed shared-owner custody")?;
    receipts.installed_shared_runtime = None;
    receipts.released_shared_sprite = Some(released);
    Ok(())
}

/// Consume D82's exact five-observation release set. It remains unavailable
/// until D82's four local targets are restored and the shared owner supplies
/// release custody for all four runtime-text targets. It is not a replacement
/// for the shared owner's complete four-site release set.
pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; 5], String> {
    let receipts = HOOK_RECEIPTS
        .get()
        .ok_or("D82 production hook receipts are not initialized")?;
    let mut receipts = receipts
        .lock()
        .map_err(|_| "D82 production hook receipt mutex poisoned")?;
    let local = receipts
        .released_local
        .ok_or("D82 exact local released-hook receipts are unavailable")?;
    let shared = receipts
        .released_shared_sprite
        .ok_or("D82 shared sprite release is still owned by runtime-text teardown")?;
    receipts.released_local = None;
    receipts.released_shared_sprite = None;
    Ok([local[0], local[1], local[2], local[3], shared])
}

fn validate_shared_runtime_release_custody(
    receipts: &[ReleasedRvaHookReceipt; 4],
) -> Result<(), String> {
    let mut observed = [
        receipts[0].observed_rva,
        receipts[1].observed_rva,
        receipts[2].observed_rva,
        receipts[3].observed_rva,
    ];
    observed.sort_unstable();
    let mut expected = [
        TEXTBOX_REGISTRATION_RVA as u64,
        TEXT_SETTER_RVA as u64,
        TEXTBOX_DESTRUCTOR_RVA as u64,
        SPRITE_SET_RVA as u64,
    ];
    expected.sort_unstable();
    let owner_tokens = [
        receipts[0].teardown_owner_token,
        receipts[1].teardown_owner_token,
        receipts[2].teardown_owner_token,
        receipts[3].teardown_owner_token,
    ];
    if observed != expected
        || receipts.iter().any(|receipt| {
            !receipt.released
                || receipt.observed_module_name != BATTLE_PLAN_MODULE_NAME
                || receipt.observed_module_sha256 != A729_EXE_SHA256
                || receipt.target_patch_len != 5
                || receipt.primitive_id != MINHOOK_NORMAL_REL5_PRIMITIVE_ID
                || receipt.teardown_owner_token == 0
                || receipt.observed_runtime_address == 0
                || receipt.restored_prefix_sha256 == [0; 32]
        })
        || owner_tokens
            .iter()
            .enumerate()
            .any(|(index, token)| owner_tokens[..index].contains(token))
    {
        return Err(
            "D82 shared sprite release lacks complete exact A25 four-target custody".into(),
        );
    }
    Ok(())
}

fn validate_shared_runtime_install_custody(
    receipts: &[InstalledRvaHookReceipt; 4],
) -> Result<(), String> {
    let mut observed = [
        receipts[0].observed_rva,
        receipts[1].observed_rva,
        receipts[2].observed_rva,
        receipts[3].observed_rva,
    ];
    observed.sort_unstable();
    let mut expected = [
        TEXTBOX_REGISTRATION_RVA as u64,
        TEXT_SETTER_RVA as u64,
        TEXTBOX_DESTRUCTOR_RVA as u64,
        SPRITE_SET_RVA as u64,
    ];
    expected.sort_unstable();
    let module_base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    let owner_tokens = [
        receipts[0].teardown_owner_token,
        receipts[1].teardown_owner_token,
        receipts[2].teardown_owner_token,
        receipts[3].teardown_owner_token,
    ];
    if observed != expected
        || receipts.iter().any(|receipt| {
            !receipt.enabled
                || receipt.patch_above
                || receipt.observed_module_name != BATTLE_PLAN_MODULE_NAME
                || receipt.observed_module_sha256 != A729_EXE_SHA256
                || receipt.target_patch_len != 5
                || receipt.primitive_id != MINHOOK_NORMAL_REL5_PRIMITIVE_ID
                || receipt.teardown_owner_token == 0
                || receipt.requested_rva != receipt.observed_rva
                || receipt.placement != HookPatchPlacement::NormalTarget
                || receipt.observed_runtime_address
                    != module_base.saturating_add(receipt.observed_rva as usize)
        })
        || module_base == 0
        || owner_tokens
            .iter()
            .enumerate()
            .any(|(index, token)| owner_tokens[..index].contains(token))
    {
        return Err(
            "D82 shared sprite install lacks complete exact A25 four-target custody".into(),
        );
    }
    Ok(())
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return HookDiagnostics {
            production_activation_authorized: PRODUCTION_ACTIVATION_AUTHORIZED,
            lifecycle_healthy: LIFECYCLE_HEALTHY.load(Ordering::Acquire),
            teardown_active: TEARDOWN_ACTIVE.load(Ordering::Acquire),
            in_flight_callbacks: IN_FLIGHT_CALLBACKS.load(Ordering::Acquire),
            ..HookDiagnostics::default()
        };
    };
    HookDiagnostics {
        installed_for_fixture_capture: INSTALLED.load(Ordering::Acquire)
            && LIFECYCLE_HEALTHY.load(Ordering::Acquire),
        lifecycle_healthy: LIFECYCLE_HEALTHY.load(Ordering::Acquire),
        teardown_active: TEARDOWN_ACTIVE.load(Ordering::Acquire),
        in_flight_callbacks: IN_FLIGHT_CALLBACKS.load(Ordering::Acquire),
        production_activation_authorized: PRODUCTION_ACTIVATION_AUTHORIZED,
        trait_refresh_count: state.trait_refresh_count,
        trait_tooltip_count: state.trait_tooltip_count,
        trait_reason_join_count: state.trait_reason_join_count,
        ace_refresh_count: state.ace_refresh_count,
        ace_bind_count: state.ace_bind_count,
        rejected_observation_count: state.rejected_observation_count,
        has_complete_boundary_batch: INSTALLED.load(Ordering::Acquire)
            && LIFECYCLE_HEALTHY.load(Ordering::Acquire)
            && state
                .latest_trait_component
                .as_ref()
                .zip(state.latest_ace_component.as_ref())
                .is_some_and(|(traits, aces)| {
                    traits.identity_catalog_epoch == aces.identity_catalog_epoch
                })
            && state.current_error.is_none(),
        current_error: state.current_error.clone(),
    }
}

/// Consumes one already pointer-free trait fixture component.
pub fn drain_decoded_trait_candidate() -> Result<(u64, Vec<TraitTreeObservation>), String> {
    if !INSTALLED.load(Ordering::Acquire) || !LIFECYCLE_HEALTHY.load(Ordering::Acquire) {
        return Err("A39 exact-a729 fixture hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("A39 hook state is unavailable")?
        .lock()
        .map_err(|_| "A39 hook state poisoned")?;
    if !INSTALLED.load(Ordering::Acquire)
        || !LIFECYCLE_HEALTHY.load(Ordering::Acquire)
        || TEARDOWN_ACTIVE.load(Ordering::Acquire)
    {
        return Err("A39 component reservation rejected during teardown".into());
    }
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    if state.component_reservation.is_some() {
        return Err("A39 trait drain is blocked by ordered pair reservation custody".into());
    }
    state
        .latest_trait_component
        .take()
        .map(|component| (component.sequence, component.rows))
        .ok_or_else(|| "A39 has not decoded a complete trait refresh".into())
}

/// Consumes one already pointer-free ace fixture component.
pub fn drain_decoded_ace_candidate() -> Result<(u64, Vec<AcePortraitObservation>), String> {
    if !INSTALLED.load(Ordering::Acquire) || !LIFECYCLE_HEALTHY.load(Ordering::Acquire) {
        return Err("A39 exact-a729 fixture hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("A39 hook state is unavailable")?
        .lock()
        .map_err(|_| "A39 hook state poisoned")?;
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    if state.component_reservation.is_some() {
        return Err("A39 ace drain is blocked by ordered pair reservation custody".into());
    }
    state
        .latest_ace_component
        .take()
        .map(|component| (component.sequence, component.rows))
        .ok_or_else(|| "A39 has not decoded a complete ace refresh".into())
}

/// Reserve the oldest complete pointer-free D82 pair without consuming it.
///
/// Exactly one reservation may be active. New callbacks continue into the
/// bounded latest-pending slots while a reservation is held. A rollback keeps
/// the reserved pair first; a later commit reveals the newer pending pair.
pub fn peek_character_presentation_candidate(
    expected_game_hour_raw: i32,
) -> Result<Option<CharacterPresentationCandidateReservation>, String> {
    let Some((reservation_id, trait_component, ace_component)) = reserve_decoded_component_pair()?
    else {
        return Ok(None);
    };
    let result = build_reserved_snapshot(expected_game_hour_raw, &trait_component, &ace_component);
    match result {
        Ok(snapshot) => Ok(Some(CharacterPresentationCandidateReservation {
            reservation_id,
            snapshot,
        })),
        Err(error) => {
            // A source-invalid/cross-hour pair can never become publishable.
            // Discard it so the collector can emit a typed gap and later
            // observe a newer pair. Rollback is reserved for failures after a
            // valid snapshot has entered publication staging.
            commit_character_presentation_candidate(reservation_id)?;
            Err(error)
        }
    }
}

pub fn commit_character_presentation_candidate(reservation_id: u64) -> Result<(), String> {
    let mut state = STATE
        .get()
        .ok_or("A39 hook state is unavailable")?
        .lock()
        .map_err(|_| "A39 hook state poisoned")?;
    state.commit_component_pair(reservation_id)
}

pub fn rollback_character_presentation_candidate(reservation_id: u64) -> Result<(), String> {
    let mut state = STATE
        .get()
        .ok_or("A39 hook state is unavailable")?
        .lock()
        .map_err(|_| "A39 hook state poisoned")?;
    state.rollback_component_pair(reservation_id)
}

fn reserve_decoded_component_pair(
) -> Result<Option<(u64, DecodedTraitComponent, DecodedAceComponent)>, String> {
    if !INSTALLED.load(Ordering::Acquire) || !LIFECYCLE_HEALTHY.load(Ordering::Acquire) {
        return Err("A39 exact-a729 fixture hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("A39 hook state is unavailable")?
        .lock()
        .map_err(|_| "A39 hook state poisoned")?;
    if !INSTALLED.load(Ordering::Acquire)
        || !LIFECYCLE_HEALTHY.load(Ordering::Acquire)
        || TEARDOWN_ACTIVE.load(Ordering::Acquire)
    {
        return Err("A39 component reservation rejected during teardown".into());
    }
    if state.component_reservation.is_none() {
        if let Some(error) = &state.current_error {
            return Err(error.clone());
        }
    }
    let reservation_id = NEXT_COMPONENT_RESERVATION_ID.fetch_add(1, Ordering::AcqRel);
    state.reserve_component_pair(reservation_id)
}

fn build_reserved_snapshot(
    expected_game_hour_raw: i32,
    trait_component: &DecodedTraitComponent,
    ace_component: &DecodedAceComponent,
) -> Result<CharacterPresentationSnapshot, String> {
    if trait_component.observed_game_hour_raw != expected_game_hour_raw
        || ace_component.observed_game_hour_raw != expected_game_hour_raw
        || trait_component.observed_game_hour_raw != ace_component.observed_game_hour_raw
    {
        return Err("A39 UI observations are stale or cross-hour for this collector frame".into());
    }
    let shared_epoch = match (
        trait_component.identity_catalog_epoch,
        ace_component.identity_catalog_epoch,
    ) {
        (Some(trait_epoch), Some(ace_epoch)) if trait_epoch == ace_epoch => trait_epoch,
        _ => {
            return Err("A39 trait/ace components do not share one collector catalog epoch".into())
        }
    };
    if collector_boundary_decoder().identity_catalog_epoch() != Some(shared_epoch) {
        return Err("A39 trait/ace components are stale for the collector catalog epoch".into());
    }
    let snapshot = CharacterPresentationSnapshot {
        source_sequence: trait_component.sequence.max(ace_component.sequence),
        trait_trees: trait_component.rows.clone(),
        ace_context: ace_component.context.clone(),
        ace_portraits: ace_component.rows.clone(),
    };
    validate_decoded_snapshot(&snapshot)?;
    if collector_boundary_decoder().identity_catalog_epoch() != Some(shared_epoch) {
        return Err("A39 collector catalog changed while reading the reserved batch".into());
    }
    Ok(snapshot)
}

unsafe extern "system" fn hook_trait_refresh(
    controller: usize,
    refresh_argument_ptr: usize,
    refresh_argument_raw: i32,
) {
    let _epoch = CallbackEpoch::enter();
    let admitted_epoch = capture_epoch_witness();
    let prior_callbacks = enter_callback(CALLBACK_TRAIT_REFRESH);
    let nested = prior_callbacks & CALLBACK_TRAIT_REFRESH != 0;
    if nested {
        reject_callback("A39 recursive trait refresh rejected");
    }
    let original = ORIGINAL_TRAIT_REFRESH.load(Ordering::Acquire);
    if original != 0 {
        let original: TraitRefreshFn = unsafe { mem::transmute(original) };
        unsafe { original(controller, refresh_argument_ptr, refresh_argument_raw) };
    } else {
        reject_callback("A39 trait refresh trampoline unavailable");
    }
    if admitted_epoch.is_some_and(capture_epoch_is_current) && !nested && original != 0 {
        record_trait_refresh(controller, refresh_argument_raw);
    }
    leave_callback(prior_callbacks);
}

unsafe extern "system" fn hook_ace_refresh(controller: usize) -> u64 {
    let _epoch = CallbackEpoch::enter();
    let admitted_epoch = capture_epoch_witness();
    let prior_callbacks = enter_callback(CALLBACK_ACE_REFRESH);
    let nested = prior_callbacks & CALLBACK_ACE_REFRESH != 0;
    if nested {
        reject_callback("A39 recursive ace refresh rejected");
    }
    let capture = admitted_epoch.is_some() && !nested && begin_ace_refresh();
    let original = ORIGINAL_ACE_REFRESH.load(Ordering::Acquire);
    let result = if original == 0 {
        reject_callback("A39 ace refresh trampoline unavailable");
        0
    } else {
        let original: AceRefreshFn = unsafe { mem::transmute(original) };
        unsafe { original(controller) }
    };
    if capture && admitted_epoch.is_some_and(capture_epoch_is_current) {
        finish_ace_refresh(controller);
    } else if capture {
        cancel_ace_refresh();
    }
    leave_callback(prior_callbacks);
    result
}

unsafe extern "system" fn hook_ace_row_bind(
    row_ptr: usize,
    ace_ptr: usize,
    argument_3: usize,
    argument_4: usize,
    argument_5_raw: u32,
) {
    let _epoch = CallbackEpoch::enter();
    let admitted_epoch = capture_epoch_witness();
    let prior_callbacks = enter_callback(CALLBACK_ACE_BIND);
    let nested = prior_callbacks & CALLBACK_ACE_BIND != 0;
    if nested {
        reject_callback("A39 recursive ace row bind rejected");
    }
    let capture_portrait = admitted_epoch.is_some() && !nested && begin_ace_portrait_set(row_ptr);
    let capture_modifiers = admitted_epoch.is_some() && !nested && begin_ace_modifier_set(row_ptr);
    let original = ORIGINAL_ACE_ROW_BIND.load(Ordering::Acquire);
    if original != 0 {
        let original: AceRowBindFn = unsafe { mem::transmute(original) };
        unsafe { original(row_ptr, ace_ptr, argument_3, argument_4, argument_5_raw) };
    } else {
        reject_callback("A39 ace row-bind trampoline unavailable");
    }
    let portrait_set = capture_portrait.then(finish_ace_portrait_set);
    let modifier_set = capture_modifiers.then(finish_ace_modifier_set);
    if admitted_epoch.is_some_and(capture_epoch_is_current) && !nested && original != 0 {
        record_ace_bind(row_ptr, ace_ptr, portrait_set, modifier_set);
    }
    leave_callback(prior_callbacks);
}

unsafe extern "system" fn a39_sprite_setter_observer(
    widget_ptr: usize,
    string_ptr: usize,
    _mode: u8,
    _caller_rva: u32,
) {
    let _epoch = CallbackEpoch::enter();
    let admitted_epoch = capture_epoch_witness();
    let prior_callbacks = enter_callback(CALLBACK_SPRITE_SET);
    let nested = prior_callbacks & CALLBACK_SPRITE_SET != 0;
    let target_matches = ACTIVE_ACE_PORTRAIT_SET.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|capture| capture.widget_ptr == widget_ptr)
    });
    // This is a generic GUI setter and therefore observes unrelated traffic.
    // Only recursion for the active row portrait target can make the A39
    // observation ambiguous; foreign widget recursion must not poison d82.
    if nested && target_matches {
        reject_callback("A39 recursive sprite set rejected");
    }
    if admitted_epoch.is_some_and(capture_epoch_is_current) && !nested && target_matches {
        capture_ace_portrait_sprite(widget_ptr, string_ptr);
    }
    leave_callback(prior_callbacks);
}

unsafe extern "system" fn hook_trait_tooltip_formatter(
    item_ptr: usize,
    argument_2: usize,
    output_text_ptr: usize,
) -> u64 {
    let _epoch = CallbackEpoch::enter();
    let admitted_epoch = capture_epoch_witness();
    let prior_callbacks = enter_callback(CALLBACK_TRAIT_TOOLTIP_FORMATTER);
    let nested = prior_callbacks & CALLBACK_TRAIT_TOOLTIP_FORMATTER != 0;
    ACTIVE_TRAIT_REASON.with(|slot| *slot.borrow_mut() = None);
    if nested {
        reject_callback("A39 recursive trait tooltip formatter rejected");
    }
    let mapped_locator = if admitted_epoch.is_some() && !nested && item_ptr != 0 {
        STATE
            .get()
            .and_then(|state| state.lock().ok())
            .and_then(|state| state.trait_items.get(&item_ptr).copied())
    } else {
        None
    };
    let locator = mapped_locator.filter(|locator| {
        item_ptr
            .checked_add(TRAIT_ITEM_DEFINITION)
            .and_then(read_usize)
            == Some(locator.trait_definition_ptr)
    });
    if mapped_locator.is_some() && locator.is_none() {
        reject_callback("A39 trait tooltip item definition changed after exact refresh");
    }
    let original = ORIGINAL_TRAIT_TOOLTIP_FORMATTER.load(Ordering::Acquire);
    let result = if original == 0 {
        reject_callback("A39 trait tooltip formatter trampoline unavailable");
        0
    } else {
        let original: TraitTooltipFormatterFn = unsafe { mem::transmute(original) };
        unsafe { original(item_ptr, argument_2, output_text_ptr) }
    };
    if result != 0
        && original != 0
        && admitted_epoch.is_some_and(capture_epoch_is_current)
        && !nested
    {
        if let Some(locator) = locator {
            match read_msvc_text(output_text_ptr) {
                Ok(expected_text) if !expected_text.is_empty() => {
                    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
                        state.trait_tooltip_count = state.trait_tooltip_count.saturating_add(1);
                    }
                    ACTIVE_TRAIT_REASON.with(|slot| {
                        *slot.borrow_mut() = Some(PendingTraitReason {
                            item_ptr,
                            locator,
                            expected_text,
                            matched_owner_widget: None,
                        });
                    });
                }
                Ok(_) => {}
                Err(error) => reject_callback(&error),
            }
        }
    }
    leave_callback(prior_callbacks);
    result
}

fn record_trait_refresh(controller_ptr: usize, refresh_argument_raw: i32) {
    ACTIVE_TRAIT_REASON.with(|slot| *slot.borrow_mut() = None);
    if controller_ptr == 0 {
        reject_callback("A39 trait refresh supplied null controller");
        return;
    }
    let Some(selected_leader_reference_raw) = controller_ptr
        .checked_add(TRAIT_SELECTED_LEADER_REFERENCE)
        .and_then(read_u64)
    else {
        reject_callback("A39 selected leader reference was unreadable after trait refresh");
        return;
    };
    let selected_leader_ptr = match resolve_reference_object(
        MODULE_BASE.load(Ordering::Acquire),
        selected_leader_reference_raw,
    ) {
        Ok(pointer) => pointer,
        Err(error) => {
            reject_callback(&error);
            return;
        }
    };
    let trees = match read_process_local_trait_trees(
        controller_ptr,
        selected_leader_reference_raw,
        selected_leader_ptr,
    ) {
        Ok(trees) => trees,
        Err(error) => {
            reject_callback(&error);
            return;
        }
    };
    let boundary = ProcessLocalTraitRefresh {
        sequence: next_sequence(),
        observed_game_hour_raw: match observed_game_hour_raw() {
            Ok(hour) => hour,
            Err(error) => {
                reject_callback(&error);
                return;
            }
        },
        controller_ptr,
        selected_leader_reference_raw,
        selected_leader_ptr,
        refresh_argument_raw,
        trees,
    };
    let decoded = decode_trait_synchronously(&boundary);
    let trait_items = decoded
        .as_ref()
        .ok()
        .and_then(|component| trait_item_locators(&boundary, component).ok());
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.trait_refresh_count = state.trait_refresh_count.saturating_add(1);
    state.trait_items.clear();
    match decoded {
        Ok(component) if state.current_error.is_none() && trait_items.is_some() => {
            state.trait_items = trait_items.unwrap();
            state.latest_trait_component = Some(component);
        }
        Ok(_) if state.current_error.is_none() => reject(
            &mut state,
            "A39 decoded trait rows do not preserve the exact process-local tree/item shape",
        ),
        Ok(_) => {}
        Err(error) => reject(&mut state, &error),
    }
}

fn trait_item_locators(
    boundary: &ProcessLocalTraitRefresh,
    component: &DecodedTraitComponent,
) -> Result<BTreeMap<usize, TraitItemLocator>, String> {
    if boundary.trees.len() != component.rows.len() {
        return Err("A39 decoded trait tree count differs from process-local refresh".into());
    }
    let mut items = BTreeMap::new();
    for (tree_index, (boundary_tree, decoded_tree)) in
        boundary.trees.iter().zip(&component.rows).enumerate()
    {
        if boundary_tree.root != decoded_tree.root
            || boundary_tree.nodes.len() != decoded_tree.nodes.len()
        {
            return Err(
                "A39 decoded trait tree shape/order differs from process-local refresh".into(),
            );
        }
        for (node_index, node) in boundary_tree.nodes.iter().enumerate() {
            if items
                .insert(
                    node.item_ptr,
                    TraitItemLocator {
                        tree_index,
                        node_index,
                        trait_definition_ptr: node.trait_definition_ptr,
                    },
                )
                .is_some()
            {
                return Err("A39 process-local trait item address is duplicated".into());
            }
        }
    }
    Ok(items)
}

fn begin_ace_refresh() -> bool {
    let started = ACTIVE_ACE_BINDS.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        if scratch.is_some() {
            false
        } else {
            *scratch = Some(Vec::new());
            true
        }
    });
    if !started {
        reject_callback("A39 overlapping ace refresh rejected");
    }
    started
}

fn cancel_ace_refresh() {
    ACTIVE_ACE_BINDS.with(|scratch| *scratch.borrow_mut() = None);
    ACTIVE_ACE_PORTRAIT_SET.with(|slot| *slot.borrow_mut() = None);
    ACTIVE_ACE_MODIFIER_SET.with(|slot| *slot.borrow_mut() = None);
}

fn record_ace_bind(
    row_ptr: usize,
    ace_ptr: usize,
    portrait_set: Option<Result<ActiveAcePortraitSet, String>>,
    modifier_set: Option<Result<ActiveAceModifierSet, String>>,
) {
    if !ACTIVE_ACE_BINDS.with(|scratch| scratch.borrow().is_some()) {
        // A foreign-thread or out-of-refresh bind cannot enter another
        // thread's post-sort ordinal set and cannot poison its batch.
        return;
    }
    if row_ptr == 0 || ace_ptr == 0 {
        reject_callback("A39 ace bind supplied null row or ace");
        return;
    }
    let Some(ace_id) = ace_ptr.checked_add(ACE_OBJECT_ID).and_then(read_u32) else {
        reject_callback("A39 ace id was unreadable after row bind");
        return;
    };
    if ace_id == 0 || ace_id > i32::MAX as u32 {
        reject_callback("A39 ace row bind carried invalid ace id");
        return;
    }
    let Some(Ok(portrait_set)) = portrait_set else {
        reject_callback("A39 ace bind did not own a portrait-set capture epoch");
        return;
    };
    if portrait_set.ambiguous {
        reject_callback("A39 ace bind observed conflicting portrait sprite writes");
        return;
    }
    let Some(Ok(modifier_set)) = modifier_set else {
        reject_callback("A39 ace bind did not own a modifier-text capture epoch");
        return;
    };
    if modifier_set.ambiguous || modifier_set.write_count != 1 {
        reject_callback("A39 ace bind did not observe exactly one modifier-text write");
        return;
    }
    let Some(modifier_text) = modifier_set.modifier_text else {
        reject_callback("A39 ace bind modifier text was unavailable");
        return;
    };
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.ace_bind_count = state.ace_bind_count.saturating_add(1);
    let portrait_gfx = match portrait_set.portrait_gfx {
        Some(portrait_gfx) => {
            state
                .ace_portrait_by_widget
                .insert(portrait_set.widget_ptr, portrait_gfx.clone());
            portrait_gfx
        }
        None => match state.ace_portrait_by_widget.get(&portrait_set.widget_ptr) {
            Some(portrait_gfx) => portrait_gfx.clone(),
            None => {
                reject(
                    &mut state,
                    "A39 first observation of an ace portrait widget had no exact sprite write",
                );
                return;
            }
        },
    };
    drop(state);
    let result = ACTIVE_ACE_BINDS.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        let Some(binds) = scratch.as_mut() else {
            return Ok(());
        };
        if binds.len() >= MAX_ACE_BINDS_PER_REFRESH {
            return Err("A39 ace bind count exceeded fixture bound");
        }
        let visible_ordinal = binds.len() as u32;
        binds.push(ProcessLocalAceBind {
            visible_ordinal,
            ace_id,
            row_ptr,
            ace_ptr,
            portrait_gfx,
            modifier_text,
        });
        Ok(())
    });
    if let Err(error) = result {
        reject_callback(error);
    }
}

fn begin_ace_portrait_set(row_ptr: usize) -> bool {
    let Some(widget_ptr) = row_ptr
        .checked_add(ACE_ROW_PORTRAIT_WIDGET)
        .and_then(read_usize)
        .filter(|value| *value != 0)
    else {
        reject_callback("A39 ace row portrait widget was unreadable");
        return false;
    };
    let started = ACTIVE_ACE_PORTRAIT_SET.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            false
        } else {
            *slot = Some(ActiveAcePortraitSet {
                widget_ptr,
                portrait_gfx: None,
                ambiguous: false,
            });
            true
        }
    });
    if !started {
        reject_callback("A39 overlapping ace portrait-set capture rejected");
    }
    started
}

fn finish_ace_portrait_set() -> Result<ActiveAcePortraitSet, String> {
    ACTIVE_ACE_PORTRAIT_SET
        .with(|slot| slot.borrow_mut().take())
        .ok_or_else(|| "A39 ace portrait-set capture disappeared before row bind returned".into())
}

fn capture_ace_portrait_sprite(widget_ptr: usize, string_ptr: usize) {
    let target_matches = ACTIVE_ACE_PORTRAIT_SET.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|capture| capture.widget_ptr == widget_ptr)
    });
    if !target_matches {
        return;
    }
    let portrait_gfx = match read_msvc_string(string_ptr) {
        Ok(value) => value,
        Err(error) => {
            reject_callback(&error);
            return;
        }
    };
    ACTIVE_ACE_PORTRAIT_SET.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(capture) = slot.as_mut() else {
            return;
        };
        match capture.portrait_gfx.as_deref() {
            None => capture.portrait_gfx = Some(portrait_gfx),
            Some(previous) if previous == portrait_gfx => {}
            Some(_) => capture.ambiguous = true,
        }
    });
}

fn begin_ace_modifier_set(row_ptr: usize) -> bool {
    let Some(widget_ptr) = row_ptr
        .checked_add(ACE_ROW_MODIFIER_TEXT_WIDGET)
        .and_then(read_usize)
        .filter(|value| *value != 0)
    else {
        reject_callback("A39 ace row modifier textbox was unreadable");
        return false;
    };
    let started = ACTIVE_ACE_MODIFIER_SET.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            false
        } else {
            *slot = Some(ActiveAceModifierSet {
                widget_ptr,
                modifier_text: None,
                write_count: 0,
                ambiguous: false,
            });
            true
        }
    });
    if !started {
        reject_callback("A39 overlapping ace modifier-text capture rejected");
    }
    started
}

fn finish_ace_modifier_set() -> Result<ActiveAceModifierSet, String> {
    ACTIVE_ACE_MODIFIER_SET
        .with(|slot| slot.borrow_mut().take())
        .ok_or_else(|| "A39 ace modifier-text capture disappeared before row bind returned".into())
}

/// A25 owns the sole 0x1422B59E0 detour. This observer runs before its
/// trampoline consumes the MSVC string. Ace text requires the exact row-bind
/// TLS epoch plus row+0x60 widget equality; trait text requires exact equality
/// with the immediately preceding mapped tooltip formatter output.
unsafe extern "system" fn a39_pre_text_setter_observer(
    owner_ptr: usize,
    widget_ptr: usize,
    text_object_ptr: usize,
    _caller_rva: u32,
) {
    let ace_target_matches = ACTIVE_ACE_MODIFIER_SET.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|capture| capture.widget_ptr == widget_ptr)
    });
    let reason_pending = ACTIVE_TRAIT_REASON.with(|slot| slot.borrow().is_some());
    if !ace_target_matches && !reason_pending {
        return;
    }
    if capture_epoch_witness().is_none() {
        ACTIVE_TRAIT_REASON.with(|slot| *slot.borrow_mut() = None);
        return;
    }
    let observed_text = match read_msvc_text(text_object_ptr) {
        Ok(value) => value,
        Err(error) => {
            reject_callback(&error);
            ACTIVE_TRAIT_REASON.with(|slot| *slot.borrow_mut() = None);
            return;
        }
    };
    if ace_target_matches {
        ACTIVE_ACE_MODIFIER_SET.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(capture) = slot.as_mut() else {
                return;
            };
            capture.write_count = capture.write_count.saturating_add(1);
            if capture.write_count != 1 || capture.modifier_text.is_some() {
                capture.ambiguous = true;
                return;
            }
            capture.modifier_text = Some(observed_text.clone());
        });
    }
    if reason_pending {
        ACTIVE_TRAIT_REASON.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(pending) = slot.as_mut() else {
                return;
            };
            // Admit only the immediately following exact setter input. The
            // first foreign setter invalidates this transition so a stale
            // tooltip string cannot collide with later UI traffic.
            if pending.expected_text == observed_text && pending.matched_owner_widget.is_none() {
                pending.matched_owner_widget = Some((owner_ptr, widget_ptr));
            } else {
                *slot = None;
            }
        });
    }
}

unsafe extern "system" fn a39_post_text_setter_observer(
    owner_ptr: usize,
    widget_ptr: usize,
    _caller_rva: u32,
) {
    let Some(admitted_epoch) = capture_epoch_witness() else {
        ACTIVE_TRAIT_REASON.with(|slot| *slot.borrow_mut() = None);
        return;
    };
    let pending = ACTIVE_TRAIT_REASON.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|pending| pending.matched_owner_widget == Some((owner_ptr, widget_ptr)))
        {
            slot.take()
        } else {
            None
        }
    });
    let Some(pending) = pending else {
        return;
    };
    let Some(context) = pin_observed_textbox_context(owner_ptr) else {
        reject_callback("A39 trait reason setter had no pinnable A25 context");
        return;
    };
    if !capture_epoch_is_current(admitted_epoch) {
        return;
    }
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    if state.trait_items.get(&pending.item_ptr).copied() != Some(pending.locator) {
        reject(
            &mut state,
            "A39 trait reason item changed before the A25 context join completed",
        );
        return;
    }
    let locator_in_bounds = state
        .latest_trait_component
        .as_ref()
        .and_then(|component| component.rows.get(pending.locator.tree_index))
        .is_some_and(|tree| pending.locator.node_index < tree.nodes.len());
    if !locator_in_bounds {
        reject(
            &mut state,
            "A39 trait reason locator escaped the decoded trait component",
        );
        return;
    }
    let component = state.latest_trait_component.as_mut().unwrap();
    for tree in &mut component.rows {
        for node in &mut tree.nodes {
            if node.reason_context_id.as_deref() == Some(context.context_id.as_str()) {
                node.reason_context_id = None;
            }
        }
    }
    let target = &mut component.rows[pending.locator.tree_index].nodes[pending.locator.node_index];
    target.reason_context_id = Some(context.context_id);
    state.trait_reason_join_count = state.trait_reason_join_count.saturating_add(1);
}

fn read_process_local_trait_trees(
    controller_ptr: usize,
    selected_leader_reference_raw: u64,
    selected_leader_ptr: usize,
) -> Result<Vec<ProcessLocalTraitTree>, String> {
    let progress_by_definition = read_trait_progress_catalog(selected_leader_ptr)?;
    let mut trees = Vec::with_capacity(2);
    let mut total_nodes = 0usize;
    for root in [
        TraitTreeRoot::ControllerOffset120,
        TraitTreeRoot::ControllerOffset130,
    ] {
        let tree_ptr = controller_ptr
            .checked_add(root.raw_offset() as usize)
            .and_then(read_usize)
            .filter(|value| *value != 0)
            .ok_or_else(|| format!("A39 trait tree {:?} pointer was unreadable", root))?;
        let count = tree_ptr
            .checked_add(TRAIT_TREE_COUNT)
            .and_then(read_u32)
            .ok_or_else(|| format!("A39 trait tree {:?} count was unreadable", root))?
            as usize;
        let capacity = tree_ptr
            .checked_add(TRAIT_TREE_CAPACITY)
            .and_then(read_u32)
            .ok_or_else(|| format!("A39 trait tree {:?} capacity was unreadable", root))?
            as usize;
        total_nodes = total_nodes
            .checked_add(count)
            .ok_or_else(|| "A39 trait tree node count overflow".to_string())?;
        if count > capacity || total_nodes > MAX_TRAIT_NODES {
            return Err(format!(
                "A39 trait tree {:?} has invalid count/capacity {count}/{capacity}",
                root
            ));
        }
        let vector_ptr = tree_ptr
            .checked_add(TRAIT_TREE_VECTOR)
            .and_then(read_usize)
            .unwrap_or(0);
        if count != 0 && vector_ptr == 0 {
            return Err(format!("A39 trait tree {:?} vector is null", root));
        }
        let mut nodes = Vec::with_capacity(count);
        for source_ordinal in 0..count {
            let item_ptr = vector_ptr
                .checked_add(source_ordinal.saturating_mul(mem::size_of::<usize>()))
                .and_then(read_usize)
                .filter(|value| *value != 0)
                .ok_or_else(|| {
                    format!(
                        "A39 trait tree {:?} item {source_ordinal} pointer was unreadable",
                        root
                    )
                })?;
            let trait_definition_ptr = item_ptr
                .checked_add(TRAIT_ITEM_DEFINITION)
                .and_then(read_usize)
                .filter(|value| *value != 0)
                .ok_or_else(|| {
                    format!(
                        "A39 trait tree {:?} item {source_ordinal} definition was unreadable",
                        root
                    )
                })?;
            let item_leader_reference = item_ptr
                .checked_add(TRAIT_ITEM_LEADER_REFERENCE)
                .and_then(read_u64)
                .ok_or_else(|| {
                    format!(
                        "A39 trait tree {:?} item {source_ordinal} leader reference was unreadable",
                        root
                    )
                })?;
            if item_leader_reference != selected_leader_reference_raw {
                return Err(format!(
                    "A39 trait tree {:?} item {source_ordinal} belongs to a different leader reference",
                    root
                ));
            }
            let (definition_kind, progress_raw) =
                trait_definition_kind_and_progress(trait_definition_ptr, &progress_by_definition)?;
            nodes.push(ProcessLocalTraitNode {
                source_ordinal: source_ordinal as u32,
                item_ptr,
                trait_definition_ptr,
                state: read_trait_node_state(item_ptr)?,
                progress_raw,
                definition_kind,
            });
        }
        trees.push(ProcessLocalTraitTree { root, nodes });
    }
    Ok(trees)
}

fn read_trait_progress_catalog(selected_leader_ptr: usize) -> Result<BTreeMap<usize, i64>, String> {
    if selected_leader_ptr == 0 {
        return Err("A39 selected leader pointer is null for trait XP read".into());
    }
    let read_header = || -> Result<(usize, usize), String> {
        let count = selected_leader_ptr
            .checked_add(UNIT_LEADER_TRAIT_XP_COUNT)
            .and_then(read_u32)
            .ok_or_else(|| "A39 leader trait-XP count was unreadable".to_string())?
            as usize;
        if count > MAX_TRAIT_NODES {
            return Err("A39 leader trait-XP count exceeded the fixture bound".into());
        }
        let vector = selected_leader_ptr
            .checked_add(UNIT_LEADER_TRAIT_XP_VECTOR)
            .and_then(read_usize)
            .unwrap_or(0);
        if count != 0 && vector == 0 {
            return Err("A39 leader trait-XP vector is null with nonzero count".into());
        }
        Ok((vector, count))
    };
    let read_rows = |vector: usize, count: usize| -> Result<Vec<(usize, i64)>, String> {
        let mut rows = Vec::with_capacity(count);
        for ordinal in 0..count {
            let byte_offset = ordinal
                .checked_mul(UNIT_LEADER_TRAIT_XP_ENTRY_BYTES)
                .ok_or_else(|| "A39 leader trait-XP entry offset overflowed".to_string())?;
            let entry = vector
                .checked_add(byte_offset)
                .ok_or_else(|| "A39 leader trait-XP entry address overflowed".to_string())?;
            let definition = read_usize(entry)
                .filter(|value| *value != 0)
                .ok_or_else(|| "A39 leader trait-XP definition was unreadable".to_string())?;
            let raw = entry
                .checked_add(mem::size_of::<usize>())
                .and_then(read_u64)
                .ok_or_else(|| "A39 leader trait-XP numerator was unreadable".to_string())?
                as i64;
            rows.push((definition, raw));
        }
        Ok(rows)
    };
    let first_header = read_header()?;
    let first_rows = read_rows(first_header.0, first_header.1)?;
    let second_header = read_header()?;
    let second_rows = read_rows(second_header.0, second_header.1)?;
    if first_header != second_header || first_rows != second_rows {
        return Err("A39 leader trait-XP vector changed during observation".into());
    }
    let mut by_definition = BTreeMap::new();
    for (definition, raw) in first_rows {
        if by_definition.insert(definition, raw).is_some() {
            return Err("A39 leader trait-XP vector repeated a definition".into());
        }
    }
    Ok(by_definition)
}

/// The exact definition kind, and the trait-XP numerator its value selects.
///
/// One read, two results, deliberately. The kind decides whether a progress
/// value applies at all, so reading it twice -- once to decide and once to
/// carry -- would allow the carried kind to disagree with the applicability it
/// is supposed to explain. The tear-detecting re-read stays inside this
/// function, which is what makes the returned pair a single observation.
fn trait_definition_kind_and_progress(
    trait_definition_ptr: usize,
    progress_by_definition: &BTreeMap<usize, i64>,
) -> Result<(u32, Option<i64>), String> {
    let kind_address = trait_definition_ptr
        .checked_add(TRAIT_DEFINITION_KIND)
        .ok_or_else(|| "A39 trait kind address overflowed".to_string())?;
    let first_kind =
        read_u32(kind_address).ok_or_else(|| "A39 trait kind was unreadable".to_string())?;
    let second_kind = read_u32(kind_address)
        .ok_or_else(|| "A39 trait kind re-read was unreadable".to_string())?;
    if first_kind != second_kind {
        return Err("A39 trait kind changed during observation".into());
    }
    let progress = matches!(first_kind, 2 | 3).then(|| {
        progress_by_definition
            .get(&trait_definition_ptr)
            .copied()
            .unwrap_or(0)
    });
    Ok((first_kind, progress))
}

fn read_trait_node_state(item_ptr: usize) -> Result<TraitNodeState, String> {
    let visible: [Result<bool, String>; 3] = [
        TRAIT_ITEM_DISABLED_BUTTON,
        TRAIT_ITEM_PICKABLE_BUTTON,
        TRAIT_ITEM_PICKED_BUTTON,
    ]
    .map(|pointer_offset| {
        let widget_ptr = item_ptr
            .checked_add(pointer_offset)
            .and_then(read_usize)
            .filter(|value| *value != 0)
            .ok_or_else(|| "A39 trait state widget pointer was unreadable".to_string())?;
        let flags = widget_ptr
            .checked_add(GUI_WIDGET_FLAGS)
            .and_then(read_u8)
            .ok_or_else(|| "A39 trait state widget flags were unreadable".to_string())?;
        Ok(flags & GUI_WIDGET_HIDDEN == 0)
    });
    let [disabled, pickable, picked] = visible;
    match (disabled?, pickable?, picked?) {
        (true, false, false) => Ok(TraitNodeState::Blocked),
        (false, true, false) => Ok(TraitNodeState::Pickable),
        (false, false, true) => Ok(TraitNodeState::Picked),
        pattern => Err(format!(
            "A39 trait state widgets are not one-hot: {pattern:?}"
        )),
    }
}

fn finish_ace_refresh(controller_ptr: usize) {
    let Some(binds) = ACTIVE_ACE_BINDS.with(|scratch| scratch.borrow_mut().take()) else {
        return;
    };
    if controller_ptr == 0 {
        reject_callback("A39 ace refresh supplied null controller");
        return;
    }
    let context = match read_process_local_ace_context(controller_ptr) {
        Ok(context) => context,
        Err(error) => {
            reject_callback(&error);
            return;
        }
    };
    let boundary = ProcessLocalAceRefresh {
        sequence: next_sequence(),
        observed_game_hour_raw: match observed_game_hour_raw() {
            Ok(hour) => hour,
            Err(error) => {
                reject_callback(&error);
                return;
            }
        },
        controller_ptr,
        context,
        binds,
    };
    let decoded = decode_ace_synchronously(&boundary);
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.ace_refresh_count = state.ace_refresh_count.saturating_add(1);
    match decoded {
        Ok(component) if state.current_error.is_none() => {
            state.latest_ace_component = Some(component);
        }
        Ok(_) => {}
        Err(error) => reject(&mut state, &error),
    }
}

fn read_process_local_ace_context(
    controller_ptr: usize,
) -> Result<ProcessLocalAceListContext, String> {
    let selected_reference = controller_ptr
        .checked_add(ACE_CONTROLLER_SELECTED_WING_REFERENCE)
        .and_then(read_u64)
        .ok_or_else(|| "A39 selected-wing reference was unreadable".to_string())?;
    let plane_type_mask = controller_ptr
        .checked_add(ACE_CONTROLLER_PLANE_TYPE_MASK)
        .and_then(read_u32)
        .ok_or_else(|| "A39 plane-type mask was unreadable".to_string())?;
    if selected_reference != 0 {
        let wing_ptr =
            resolve_reference_object(MODULE_BASE.load(Ordering::Acquire), selected_reference)?;
        if wing_ptr == 0 {
            return Err("A39 selected-wing reference resolved to null".into());
        }
        Ok(ProcessLocalAceListContext::SelectedWing { wing_ptr })
    } else if plane_type_mask != 0 {
        Ok(ProcessLocalAceListContext::PlaneTypeMask {
            mask_raw: plane_type_mask,
        })
    } else {
        Err("A39 ace refresh has neither selected wing nor plane-type mask".into())
    }
}

fn reject(state: &mut HookState, message: &str) {
    state.rejected_observation_count = state.rejected_observation_count.saturating_add(1);
    state.current_error = Some(message.into());
    state.latest_trait_component = None;
    state.latest_ace_component = None;
    ACTIVE_ACE_BINDS.with(|scratch| *scratch.borrow_mut() = None);
    ACTIVE_ACE_PORTRAIT_SET.with(|slot| *slot.borrow_mut() = None);
    ACTIVE_ACE_MODIFIER_SET.with(|slot| *slot.borrow_mut() = None);
}

fn reject_callback(message: &str) {
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    reject(&mut state, message);
}

fn next_sequence() -> u64 {
    NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed).max(1)
}

fn enter_callback(bit: u8) -> u8 {
    ACTIVE_CALLBACKS.with(|guard| {
        let prior = guard.get();
        guard.set(prior | bit);
        prior
    })
}

fn leave_callback(prior: u8) {
    ACTIVE_CALLBACKS.with(|guard| guard.set(prior));
}

fn decoder() -> Result<Arc<dyn ExactA729BoundaryDecoder>, String> {
    FIXTURE_DECODER
        .get()
        .ok_or("A39 fixture decoder is unavailable")?
        .lock()
        .map_err(|_| "A39 fixture decoder mutex poisoned")?
        .as_ref()
        .cloned()
        .ok_or_else(|| "A39 fixture decoder is unavailable".into())
}

fn decode_trait_synchronously(
    boundary: &ProcessLocalTraitRefresh,
) -> Result<DecodedTraitComponent, String> {
    let _decoder_guard = DecoderCallGuard::enter()?;
    let decoder = decoder()?;
    let identity_catalog_epoch = decoder.identity_catalog_epoch();
    let rows = catch_unwind(AssertUnwindSafe(|| decoder.decode_trait_refresh(boundary)))
        .map_err(|_| "A39 trait fixture decoder panicked")??;
    if decoder.identity_catalog_epoch() != identity_catalog_epoch {
        return Err("A39 identity catalog changed during trait decode".into());
    }
    let candidate = CharacterPresentationSnapshot {
        source_sequence: boundary.sequence,
        trait_trees: rows.clone(),
        ace_context: AceListContext::PlaneTypeMask { mask_raw: 1 },
        ace_portraits: Vec::new(),
    };
    validate_decoded_snapshot(&candidate)?;
    Ok(DecodedTraitComponent {
        sequence: boundary.sequence,
        observed_game_hour_raw: boundary.observed_game_hour_raw,
        identity_catalog_epoch,
        rows,
    })
}

fn decode_ace_synchronously(
    boundary: &ProcessLocalAceRefresh,
) -> Result<DecodedAceComponent, String> {
    let _decoder_guard = DecoderCallGuard::enter()?;
    let decoder = decoder()?;
    let identity_catalog_epoch = decoder.identity_catalog_epoch();
    let (context, rows) = catch_unwind(AssertUnwindSafe(|| decoder.decode_ace_refresh(boundary)))
        .map_err(|_| "A39 ace fixture decoder panicked")??;
    if decoder.identity_catalog_epoch() != identity_catalog_epoch {
        return Err("A39 identity catalog changed during ace decode".into());
    }
    validate_snapshot(&CharacterPresentationSnapshot {
        source_sequence: boundary.sequence,
        trait_trees: vec![
            TraitTreeObservation {
                owner_tag_db_id: 1,
                unit_leader_id: 1,
                root: TraitTreeRoot::ControllerOffset120,
                nodes: Vec::new(),
            },
            TraitTreeObservation {
                owner_tag_db_id: 1,
                unit_leader_id: 1,
                root: TraitTreeRoot::ControllerOffset130,
                nodes: Vec::new(),
            },
        ],
        ace_context: context.clone(),
        ace_portraits: rows.clone(),
    })
    .map_err(|error| format!("{error:?}"))?;
    Ok(DecodedAceComponent {
        sequence: boundary.sequence,
        observed_game_hour_raw: boundary.observed_game_hour_raw,
        identity_catalog_epoch,
        context,
        rows,
    })
}

fn validate_decoded_snapshot(snapshot: &CharacterPresentationSnapshot) -> Result<(), String> {
    validate_snapshot(snapshot).map_err(|error| format!("{error:?}"))
}

fn addresses(base: usize) -> InstalledAddresses {
    InstalledAddresses {
        trait_refresh: base + TRAIT_REFRESH_RVA as usize,
        ace_refresh: base + ACE_REFRESH_RVA as usize,
        ace_row_bind: base + ACE_ROW_BIND_RVA as usize,
        sprite_set: base + SPRITE_SET_RVA as usize,
        trait_tooltip_formatter: base + TRAIT_TOOLTIP_FORMATTER_RVA as usize,
    }
}

fn observe_install_receipts(addresses: InstalledAddresses) -> Result<(), String> {
    let targets = [
        addresses.trait_refresh,
        addresses.ace_refresh,
        addresses.ace_row_bind,
        addresses.trait_tooltip_formatter,
    ];
    let detours = [
        hook_trait_refresh as usize,
        hook_ace_refresh as usize,
        hook_ace_row_bind as usize,
        hook_trait_tooltip_formatter as usize,
    ];
    let mut local = Vec::with_capacity(LOCAL_PRODUCTION_HOOK_SPECS.len());
    for ((expected, target), detour) in LOCAL_PRODUCTION_HOOK_SPECS
        .into_iter()
        .zip(targets)
        .zip(detours)
    {
        match observe_enabled_minhook_rel5(expected, target, detour, next_teardown_owner_token()?) {
            Ok(receipt) => {
                PARTIAL_LOCAL_INSTALL_RECEIPTS
                    .get_or_init(|| Mutex::new(Vec::new()))
                    .lock()
                    .map_err(|_| "D82 partial hook receipt mutex poisoned")?
                    .push((expected, receipt));
                local.push(receipt);
            }
            Err(error) => return Err(error),
        }
    }
    let local: [InstalledRvaHookReceipt; 4] = local
        .try_into()
        .map_err(|_| "D82 local installed-hook receipt cardinality mismatch")?;
    let mut partial = PARTIAL_LOCAL_INSTALL_RECEIPTS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "D82 partial hook receipt mutex poisoned")?;
    if partial.len() != local.len()
        || partial
            .iter()
            .enumerate()
            .any(|(index, (observed_spec, observed_receipt))| {
                *observed_spec != LOCAL_PRODUCTION_HOOK_SPECS[index]
                    || *observed_receipt != local[index]
            })
    {
        return Err("D82 partial installed-hook receipt ordering changed".into());
    }
    let mut receipts = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "D82 production hook receipt mutex poisoned")?;
    if receipts.installed_local.is_some()
        || receipts.released_local.is_some()
        || receipts.released_local_progress.iter().any(Option::is_some)
        || receipts.installed_shared_runtime.is_some()
        || receipts.released_shared_sprite.is_some()
    {
        return Err("D82 prior hook receipt custody is unresolved".into());
    }
    receipts.installed_local = Some(local);
    receipts.installed_shared_runtime = None;
    receipts.released_local = None;
    receipts.released_local_progress = [None; 4];
    receipts.released_shared_sprite = None;
    partial.clear();
    Ok(())
}

fn ensure_receipt_install_admissible() -> Result<(), String> {
    let receipts = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let receipts = receipts
        .lock()
        .map_err(|_| "D82 production hook receipt mutex poisoned")?;
    if receipts.installed_local.is_some()
        || receipts.released_local.is_some()
        || receipts.released_local_progress.iter().any(Option::is_some)
        || receipts.installed_shared_runtime.is_some()
        || receipts.released_shared_sprite.is_some()
    {
        return Err("D82 prior hook receipt custody must be released and consumed".into());
    }
    if PARTIAL_LOCAL_INSTALL_RECEIPTS
        .get()
        .and_then(|partial| partial.lock().ok())
        .is_some_and(|partial| !partial.is_empty())
    {
        return Err("D82 partial hook receipt rollback custody is unresolved".into());
    }
    if PARTIAL_LOCAL_RELEASE_RECEIPTS
        .get()
        .and_then(|partial| partial.lock().ok())
        .is_some_and(|partial| !partial.is_empty())
    {
        return Err("D82 partial hook release receipts must be consumed".into());
    }
    Ok(())
}

fn release_partial_local_install_receipts() -> Result<(), String> {
    loop {
        let mut partial = PARTIAL_LOCAL_INSTALL_RECEIPTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map_err(|_| "D82 partial hook receipt mutex poisoned")?;
        let next = partial.first().copied();
        let Some((expected, installed)) = next else {
            return Ok(());
        };
        let mut released_receipts = PARTIAL_LOCAL_RELEASE_RECEIPTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map_err(|_| "D82 partial release receipt mutex poisoned")?;
        let released = observe_released(
            expected,
            installed,
            HookTeardownAttestation {
                callbacks_quiesced: true,
                hook_removed: true,
                relay_freed: true,
            },
        )?;
        if partial.first().copied() != Some((expected, installed)) {
            return Err("D82 partial hook receipt custody changed during release".into());
        }
        released_receipts.push(released);
        partial.remove(0);
    }
}

/// Consume release proof from a failed partial local install. Reinstallation
/// remains blocked until the caller has taken this custody.
pub fn take_partial_install_rollback_release_receipts(
) -> Result<Vec<ReleasedRvaHookReceipt>, String> {
    if PARTIAL_LOCAL_INSTALL_RECEIPTS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "D82 partial hook receipt mutex poisoned")?
        .is_empty()
    {
        let mut released = PARTIAL_LOCAL_RELEASE_RECEIPTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map_err(|_| "D82 partial release receipt mutex poisoned")?;
        return Ok(mem::take(&mut *released));
    }
    Err("D82 partial local hook rollback is not fully released".into())
}

fn local_installed_hook_receipts_available() -> bool {
    HOOK_RECEIPTS
        .get()
        .and_then(|receipts| receipts.lock().ok())
        .is_some_and(|receipts| receipts.installed_local.is_some())
}

fn reobserve_local_install_receipts(addresses: InstalledAddresses) -> Result<(), String> {
    let stored = HOOK_RECEIPTS
        .get()
        .ok_or("D82 production hook receipts are not initialized")?
        .lock()
        .map_err(|_| "D82 production hook receipt mutex poisoned")?
        .installed_local
        .ok_or("D82 exact local installed-hook receipts are unavailable")?;
    let targets = [
        addresses.trait_refresh,
        addresses.ace_refresh,
        addresses.ace_row_bind,
        addresses.trait_tooltip_formatter,
    ];
    let detours = [
        hook_trait_refresh as usize,
        hook_ace_refresh as usize,
        hook_ace_row_bind as usize,
        hook_trait_tooltip_formatter as usize,
    ];
    let mut observed = Vec::with_capacity(LOCAL_PRODUCTION_HOOK_SPECS.len());
    for (((expected, target), detour), installed) in LOCAL_PRODUCTION_HOOK_SPECS
        .into_iter()
        .zip(targets)
        .zip(detours)
        .zip(stored)
    {
        observed.push(observe_enabled_minhook_rel5(
            expected,
            target,
            detour,
            installed.teardown_owner_token,
        )?);
    }
    let observed: [InstalledRvaHookReceipt; 4] = observed
        .try_into()
        .map_err(|_| "D82 local reobservation receipt cardinality mismatch")?;
    if observed != stored {
        return Err("D82 local installed-hook receipts changed on install retry".into());
    }
    Ok(())
}

fn observe_local_release_receipts_if_owned() -> Result<(), String> {
    let receipts = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let mut receipts = receipts
        .lock()
        .map_err(|_| "D82 production hook receipt mutex poisoned")?;
    let Some(installed) = receipts.installed_local else {
        return Ok(());
    };
    for index in 0..LOCAL_PRODUCTION_HOOK_SPECS.len() {
        if receipts.released_local_progress[index].is_some() {
            continue;
        }
        let released = observe_released(
            LOCAL_PRODUCTION_HOOK_SPECS[index],
            installed[index],
            HookTeardownAttestation {
                callbacks_quiesced: true,
                hook_removed: true,
                relay_freed: true,
            },
        )?;
        receipts.released_local_progress[index] = Some(released);
    }
    let released = [
        receipts.released_local_progress[0]
            .ok_or("D82 local release progress omitted trait refresh")?,
        receipts.released_local_progress[1]
            .ok_or("D82 local release progress omitted ace refresh")?,
        receipts.released_local_progress[2]
            .ok_or("D82 local release progress omitted ace row bind")?,
        receipts.released_local_progress[3]
            .ok_or("D82 local release progress omitted tooltip formatter")?,
    ];
    receipts.installed_local = None;
    receipts.released_local_progress = [None; 4];
    receipts.released_local = Some(released);
    Ok(())
}

fn targets(addresses: InstalledAddresses) -> [usize; 4] {
    [
        addresses.trait_refresh,
        addresses.ace_refresh,
        addresses.ace_row_bind,
        addresses.trait_tooltip_formatter,
    ]
}

fn originals_ready() -> bool {
    [
        &ORIGINAL_TRAIT_REFRESH,
        &ORIGINAL_ACE_REFRESH,
        &ORIGINAL_ACE_ROW_BIND,
        &ORIGINAL_TRAIT_TOOLTIP_FORMATTER,
    ]
    .into_iter()
    .all(|original| original.load(Ordering::Acquire) != 0)
}

struct HookRemovalOutcome {
    error: Option<String>,
    all_removed: bool,
}

fn disable_and_remove(addresses: InstalledAddresses) -> HookRemovalOutcome {
    let mut first_error = None;
    let mut all_removed = true;
    let mut disable_failed = false;
    for target in targets(addresses) {
        match unsafe { inline_detour::disable(target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                disable_failed = true;
                first_error.get_or_insert_with(|| {
                    format!("A39 {target:#x} hook disable failed: {status}")
                });
            }
        }
    }
    if disable_failed {
        return HookRemovalOutcome {
            error: first_error,
            all_removed: false,
        };
    }
    if let Err(error) = wait_for_callback_quiescence(CALLBACK_QUIESCE_TIMEOUT) {
        first_error.get_or_insert(error);
        return HookRemovalOutcome {
            error: first_error,
            all_removed: false,
        };
    }
    for target in targets(addresses) {
        match unsafe { inline_detour::remove(target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                all_removed = false;
                first_error.get_or_insert_with(|| {
                    format!("A39 {target:#x} hook removal failed: {status}")
                });
            }
        }
    }
    HookRemovalOutcome {
        error: first_error,
        all_removed,
    }
}

fn wait_for_callback_quiescence(timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let in_flight = IN_FLIGHT_CALLBACKS.load(Ordering::Acquire);
        if in_flight == 0 {
            return Ok(());
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "A39 teardown timed out with {in_flight} in-flight callback(s); removal deferred"
            ));
        }
        std::thread::yield_now();
    }
}

fn clear_removed_hook_set() {
    INSTALLED.store(false, Ordering::Release);
    HOOK_SET_PRESENT.store(false, Ordering::Release);
    LIFECYCLE_HEALTHY.store(false, Ordering::Release);
    TEARDOWN_ACTIVE.store(false, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    COLLECTOR_GAME_HOUR_RAW.store(i32::MIN, Ordering::Release);
    ACTIVE_TRAIT_REASON.with(|slot| *slot.borrow_mut() = None);
    clear_originals();
    invalidate_collector_identity_catalog();
    if let Some(slot) = FIXTURE_DECODER.get() {
        if let Ok(mut slot) = slot.lock() {
            *slot = None;
        }
    }
}

fn clear_originals() {
    for original in [
        &ORIGINAL_TRAIT_REFRESH,
        &ORIGINAL_ACE_REFRESH,
        &ORIGINAL_ACE_ROW_BIND,
        &ORIGINAL_TRAIT_TOOLTIP_FORMATTER,
    ] {
        original.store(0, Ordering::Release);
    }
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let mut actual = vec![0u8; expected.len()];
    if !read_exact(address, &mut actual) {
        return Err(format!("A39 {label} sentinel unreadable"));
    }
    if actual.as_slice() != expected {
        return Err(format!(
            "A39 {label} sentinel mismatch: expected {expected:02X?}, got {actual:02X?}"
        ));
    }
    Ok(())
}

fn shared_runtime_targets_are_original(module_base: usize) -> Result<bool, String> {
    let sites = [
        (
            module_base
                .checked_add(TEXTBOX_REGISTRATION_RVA as usize)
                .ok_or("A39 shared registration target overflow")?,
            [0x48, 0x89, 0x5c, 0x24, 0x08],
        ),
        (
            module_base
                .checked_add(TEXT_SETTER_RVA as usize)
                .ok_or("A39 shared setter target overflow")?,
            [0x48, 0x89, 0x5c, 0x24, 0x08],
        ),
        (
            module_base
                .checked_add(TEXTBOX_DESTRUCTOR_RVA as usize)
                .ok_or("A39 shared destructor target overflow")?,
            [0x48, 0x89, 0x5c, 0x24, 0x08],
        ),
        (
            module_base
                .checked_add(SPRITE_SET_RVA as usize)
                .ok_or("A39 shared sprite target overflow")?,
            SPRITE_SET_SENTINEL[..5]
                .try_into()
                .map_err(|_| "A39 shared sprite prefix is truncated")?,
        ),
    ];
    let mut original_count = 0usize;
    let mut normal_patch_count = 0usize;
    for (address, original) in sites {
        let mut observed = [0u8; 5];
        if !read_exact(address, &mut observed) {
            return Err(format!(
                "A39 shared runtime target is unreadable at 0x{address:X}"
            ));
        }
        if observed == original {
            original_count += 1;
        } else if observed[0] == 0xe9 {
            normal_patch_count += 1;
        } else if observed.starts_with(&[0xeb, 0xf9]) {
            return Err("A39 shared runtime target uses forbidden patchAbove placement".into());
        } else {
            return Err(format!(
                "A39 shared runtime target has unauthenticated bytes at 0x{address:X}"
            ));
        }
    }
    match (original_count, normal_patch_count) {
        (4, 0) => Ok(true),
        (0, 4) => Ok(false),
        _ => Err("A39 shared runtime hook set is partially installed; custody is ambiguous".into()),
    }
}

fn unregister_a39_shared_observers() {
    unregister_pre_setter_observer(a39_pre_text_setter_observer);
    unregister_post_setter_observer(a39_post_text_setter_observer);
    unregister_sprite_setter_observer(a39_sprite_setter_observer);
}

fn read_exact(address: usize, out: &mut [u8]) -> bool {
    if address == 0 || out.is_empty() {
        return false;
    }
    let mut read = 0usize;
    unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr() as *mut c_void,
            out.len(),
            &mut read,
        ) != 0
            && read == out.len()
    }
}

fn read_u32(address: usize) -> Option<u32> {
    let mut bytes = [0u8; 4];
    read_exact(address, &mut bytes).then(|| u32::from_le_bytes(bytes))
}

fn read_u8(address: usize) -> Option<u8> {
    let mut byte = [0u8; 1];
    read_exact(address, &mut byte).then_some(byte[0])
}

fn read_u64(address: usize) -> Option<u64> {
    let mut bytes = [0u8; 8];
    read_exact(address, &mut bytes).then(|| u64::from_le_bytes(bytes))
}

fn read_usize(address: usize) -> Option<usize> {
    read_u64(address).and_then(|value| usize::try_from(value).ok())
}

fn read_msvc_string(address: usize) -> Result<String, String> {
    let value = read_msvc_text(address)?;
    if value.trim().is_empty() || value.as_bytes().contains(&0) {
        return Err("A39 sprite string was empty or contained NUL".into());
    }
    if value.len() > MAX_TOKEN_BYTES {
        return Err("A39 sprite string exceeded its token bound".into());
    }
    Ok(value)
}

fn read_msvc_text(address: usize) -> Result<String, String> {
    if address == 0 {
        return Err("A39 text setter supplied a null string".into());
    }
    let len = address
        .checked_add(0x10)
        .and_then(read_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "A39 text string length was unreadable".to_string())?;
    let capacity = address
        .checked_add(0x18)
        .and_then(read_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "A39 text string capacity was unreadable".to_string())?;
    if len > crate::MAX_MODIFIER_TEXT_BYTES || capacity < len {
        return Err(format!(
            "A39 text string has invalid length/capacity {len}/{capacity}"
        ));
    }
    if len == 0 {
        return Ok(String::new());
    }
    let data = if capacity <= 15 {
        address
    } else {
        read_usize(address)
            .filter(|value| *value != 0)
            .ok_or_else(|| "A39 heap text string pointer was unreadable".to_string())?
    };
    let mut bytes = vec![0u8; len];
    if !read_exact(data, &mut bytes) {
        return Err("A39 text string bytes were unreadable".into());
    }
    String::from_utf8(bytes).map_err(|_| "A39 text string was not valid UTF-8".to_string())
}

fn resolve_reference_object(module_base: usize, reference_raw: u64) -> Result<usize, String> {
    if module_base == 0 || reference_raw == 0 {
        return Err("A39 selected leader reference has no module or identity".into());
    }
    let low = reference_raw as u32;
    let high = (reference_raw >> 32) as u32;
    let registry_slot = if low < REFERENCE_SMALL_TABLE_COUNT {
        module_base
            .checked_add(REFERENCE_REGISTRY_SMALL_TABLE_RVA)
            .and_then(|address| address.checked_add(low as usize * mem::size_of::<usize>()))
    } else if low < REFERENCE_DEFAULT_LIMIT {
        module_base.checked_add(REFERENCE_REGISTRY_DEFAULT_RVA)
    } else {
        module_base.checked_add(REFERENCE_REGISTRY_LARGE_RVA)
    }
    .ok_or_else(|| "A39 reference registry address overflow".to_string())?;
    let registry = read_usize(registry_slot)
        .filter(|value| *value != 0)
        .ok_or_else(|| "A39 selected leader reference registry is unavailable".to_string())?;
    let slots = registry
        .checked_add(8)
        .and_then(read_usize)
        .filter(|value| *value != 0)
        .ok_or_else(|| "A39 selected leader reference slots are unavailable".to_string())?;
    let mask = registry
        .checked_add(0x14)
        .and_then(read_u32)
        .ok_or_else(|| "A39 selected leader reference mask is unreadable".to_string())?;
    let max_probe = registry
        .checked_add(0x18)
        .and_then(read_u8)
        .ok_or_else(|| "A39 selected leader reference probe bound is unreadable".to_string())?;
    if mask == 0 || mask > 0x3fff_ffff {
        return Err("A39 selected leader reference mask is implausible".into());
    }
    let key = (u64::from(low) << 32) | u64::from(high);
    let product = key.wrapping_mul(REFERENCE_HASH_MULTIPLIER);
    let mixed = (product as u32) ^ (product >> 32) as u32;
    let bucket = mixed & mask;
    let mut slot = slots
        .checked_add(bucket as usize * 0x18)
        .ok_or_else(|| "A39 selected leader reference bucket overflow".to_string())?;
    let mut probe = 1u16;
    loop {
        let occupied_distance = slot.checked_add(4).and_then(read_u8).ok_or_else(|| {
            "A39 selected leader reference bucket metadata is unreadable".to_string()
        })?;
        if occupied_distance == 0 || probe > u16::from(occupied_distance) {
            return Err("A39 selected leader reference was absent from its registry".into());
        }
        let observed_low = slot
            .checked_add(8)
            .and_then(read_u32)
            .ok_or_else(|| "A39 selected leader reference bucket key is unreadable".to_string())?;
        let observed_high = slot.checked_add(0x0c).and_then(read_u32).ok_or_else(|| {
            "A39 selected leader reference bucket generation is unreadable".to_string()
        })?;
        if observed_low == low && observed_high == high {
            return slot
                .checked_add(0x10)
                .and_then(read_usize)
                .filter(|value| *value != 0)
                .ok_or_else(|| "A39 selected leader reference resolved to null".to_string());
        }
        if probe > u16::from(max_probe).saturating_add(1) {
            return Err("A39 selected leader reference exceeded registry probe bound".into());
        }
        probe = probe.saturating_add(1);
        slot = slot
            .checked_add(0x18)
            .ok_or_else(|| "A39 selected leader reference probe overflow".to_string())?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TraitNodeObservation, TraitNodeState, TraitTreeRoot};

    fn candidate(progress_raw: Option<i64>) -> CharacterPresentationSnapshot {
        CharacterPresentationSnapshot {
            source_sequence: 1,
            trait_trees: vec![
                TraitTreeObservation {
                    owner_tag_db_id: 1,
                    unit_leader_id: 7,
                    root: TraitTreeRoot::ControllerOffset120,
                    nodes: vec![TraitNodeObservation {
                        trait_token: "panzer_leader".into(),
                        state: TraitNodeState::Picked,
                        progress_raw,
                        // The carrier enforces `kind in {2,3}` iff a numerator
                        // is present, so the fixture kind tracks the argument.
                        definition_kind: if progress_raw.is_some() { 2 } else { 4 },
                        reason_context_id: None,
                    }],
                },
                TraitTreeObservation {
                    owner_tag_db_id: 1,
                    unit_leader_id: 7,
                    root: TraitTreeRoot::ControllerOffset130,
                    nodes: Vec::new(),
                },
            ],
            ace_context: AceListContext::PlaneTypeMask { mask_raw: 0x400 },
            ace_portraits: vec![AcePortraitObservation {
                owner_tag_db_id: 1,
                ace_id: 11,
                visible_ordinal: 0,
                portrait_gfx: "GFX_GER_ace_m_7".into(),
                modifier_text: "+10% agility".into(),
            }],
        }
    }

    fn decoded_pair(
        trait_sequence: u64,
        ace_sequence: u64,
    ) -> (DecodedTraitComponent, DecodedAceComponent) {
        let snapshot = candidate(None);
        (
            DecodedTraitComponent {
                sequence: trait_sequence,
                observed_game_hour_raw: 1,
                identity_catalog_epoch: Some(1),
                rows: snapshot.trait_trees,
            },
            DecodedAceComponent {
                sequence: ace_sequence,
                observed_game_hour_raw: 1,
                identity_catalog_epoch: Some(1),
                context: snapshot.ace_context,
                rows: snapshot.ace_portraits,
            },
        )
    }

    struct Decoder;

    impl ExactA729BoundaryDecoder for Decoder {
        fn decode_trait_refresh(
            &self,
            _boundary: &ProcessLocalTraitRefresh,
        ) -> Result<Vec<TraitTreeObservation>, String> {
            Ok(candidate(None).trait_trees)
        }

        fn decode_ace_refresh(
            &self,
            _boundary: &ProcessLocalAceRefresh,
        ) -> Result<(AceListContext, Vec<AcePortraitObservation>), String> {
            let candidate = candidate(None);
            Ok((candidate.ace_context, candidate.ace_portraits))
        }
    }

    #[test]
    fn production_activation_is_fail_closed() {
        let _decoder = Decoder;
        let source = A729ObservedCharacterPresentationSource::new(1);
        assert!(!PRODUCTION_ACTIVATION_AUTHORIZED);
        assert!(!CURRENT_EIGHT_SITE_UI_OWNER_PROMOTABLE);
        assert!(!SHARED_RUNTIME_TYPED_OWNER_CUSTODY_AUTHORIZED);
        assert!(!source.is_exact_a729_character_presentation_source());
        assert!(source.read_character_presentation().is_err());
    }

    #[test]
    fn live_candidate_admits_authenticated_exact_progress() {
        assert!(validate_decoded_snapshot(&candidate(None)).is_ok());
        assert!(validate_decoded_snapshot(&candidate(Some(100_000))).is_ok());
    }

    #[test]
    fn process_local_catalog_joins_trait_and_final_portrait_without_addresses() {
        let trait_boundary = ProcessLocalTraitRefresh {
            sequence: 4,
            observed_game_hour_raw: 1,
            controller_ptr: 0x1000,
            selected_leader_reference_raw: 0x2000,
            selected_leader_ptr: 0x2500,
            refresh_argument_raw: 0,
            trees: vec![
                ProcessLocalTraitTree {
                    root: TraitTreeRoot::ControllerOffset120,
                    nodes: vec![ProcessLocalTraitNode {
                        source_ordinal: 0,
                        item_ptr: 0x2800,
                        trait_definition_ptr: 0x3000,
                        state: TraitNodeState::Pickable,
                        progress_raw: Some(42),
                        definition_kind: 2,
                    }],
                },
                ProcessLocalTraitTree {
                    root: TraitTreeRoot::ControllerOffset130,
                    nodes: Vec::new(),
                },
            ],
        };
        let ace_boundary = ProcessLocalAceRefresh {
            sequence: 5,
            observed_game_hour_raw: 1,
            controller_ptr: 0x4000,
            context: ProcessLocalAceListContext::PlaneTypeMask { mask_raw: 0x400 },
            binds: vec![ProcessLocalAceBind {
                visible_ordinal: 0,
                ace_id: 11,
                row_ptr: 0x5000,
                ace_ptr: 0x6000,
                portrait_gfx: "GFX_GER_ace_m_7".into(),
                modifier_text: "+10% agility".into(),
            }],
        };
        let mut catalog = ProcessLocalIdentityCatalog::default();
        catalog.leaders_by_address.insert(0x2500, (1, 7));
        catalog
            .trait_tokens_by_definition
            .insert(0x3000, "armor_officer".into());
        catalog.aces_by_address.insert(0x6000, (1, 11));

        let traits = decode_trait_rows_with_catalog(&trait_boundary, &catalog).unwrap();
        assert_eq!(traits[0].nodes[0].trait_token, "armor_officer");
        assert_eq!(traits[0].nodes[0].state, TraitNodeState::Pickable);
        assert_eq!(traits[0].nodes[0].progress_raw, Some(42));
        let aces = decode_ace_rows_with_catalog(&ace_boundary, &catalog).unwrap();
        assert_eq!(aces[0].portrait_gfx, "GFX_GER_ace_m_7");
    }

    #[test]
    fn collector_decoder_atomically_replaces_and_invalidates_catalog() {
        let boundary = ProcessLocalAceRefresh {
            sequence: 9,
            observed_game_hour_raw: 1,
            controller_ptr: 0x4000,
            context: ProcessLocalAceListContext::PlaneTypeMask { mask_raw: 0x400 },
            binds: vec![ProcessLocalAceBind {
                visible_ordinal: 0,
                ace_id: 11,
                row_ptr: 0x5000,
                ace_ptr: 0x6000,
                portrait_gfx: "GFX_GER_ace_m_7".into(),
                modifier_text: String::new(),
            }],
        };
        let decoder = CollectorBoundaryDecoder::default();
        assert!(decoder.decode_ace_refresh(&boundary).is_err());

        let mut catalog = ProcessLocalIdentityCatalog::default();
        catalog.aces_by_address.insert(0x6000, (1, 11));
        assert!(decoder.publish(catalog.clone()).unwrap());
        let epoch = decoder.identity_catalog_epoch().unwrap();
        assert!(!decoder.publish(catalog).unwrap());
        assert_eq!(decoder.identity_catalog_epoch(), Some(epoch));
        assert_eq!(
            decoder.decode_ace_refresh(&boundary).unwrap().1[0].ace_id,
            11
        );

        assert!(decoder.invalidate());
        assert_eq!(decoder.identity_catalog_epoch(), None);
        assert!(!decoder.invalidate());
        assert!(decoder.decode_ace_refresh(&boundary).is_err());
    }

    #[test]
    fn pinned_rvas_and_sentinels_match_retained_exports() {
        let base = 0x1400_0000_0usize;
        let resolved = addresses(base);
        assert_eq!(resolved.trait_refresh, base + 0x016B_DC20);
        assert_eq!(resolved.ace_refresh, base + 0x0201_E420);
        assert_eq!(resolved.ace_row_bind, base + 0x0201_EDC0);
        assert_eq!(resolved.sprite_set, base + 0x0229_A780);
        assert_eq!(resolved.trait_tooltip_formatter, base + 0x01CF_F880);
        assert_eq!(TRAIT_REFRESH_SENTINEL.len(), 18);
        assert_eq!(ACE_REFRESH_SENTINEL.len(), 21);
        assert_eq!(ACE_ROW_BIND_SENTINEL.len(), 17);
        assert_eq!(TRAIT_TOOLTIP_FORMATTER_SENTINEL.len(), 24);
        assert_eq!(SPRITE_SET_SENTINEL.len(), 20);
        assert_eq!(TRAIT_SELECTED_LEADER_REFERENCE, 0x78);
        assert_eq!(ACE_OBJECT_ID, 0x0c);
        assert_eq!(
            LOCAL_PRODUCTION_HOOK_SPECS.map(|spec| spec.logical_name),
            [
                "presentation.d82.trait_refresh",
                "presentation.d82.ace_refresh",
                "presentation.d82.ace_row_bind",
                "presentation.d82.trait_tooltip_formatter",
            ]
        );
        assert_eq!(
            SHARED_SPRITE_PRODUCTION_HOOK_SPEC.logical_name,
            "presentation.d82.sprite_set"
        );
        assert!(LOCAL_PRODUCTION_HOOK_SPECS
            .iter()
            .chain([&SHARED_SPRITE_PRODUCTION_HOOK_SPEC])
            .all(|spec| {
                spec.target_patch_len == 5 && spec.primitive_id == MINHOOK_NORMAL_REL5_PRIMITIVE_ID
            }));
    }

    #[test]
    fn component_reservation_rollback_preserves_order_before_newer_pending_pair() {
        let (first_traits, first_aces) = decoded_pair(10, 11);
        let mut state = HookState {
            latest_trait_component: Some(first_traits),
            latest_ace_component: Some(first_aces),
            ..HookState::default()
        };
        let (first_id, reserved_traits, reserved_aces) =
            state.reserve_component_pair(7).unwrap().unwrap();
        assert_eq!(first_id, 7);
        assert_eq!(reserved_traits.sequence, 10);
        assert_eq!(reserved_aces.sequence, 11);
        assert!(state.reserve_component_pair(8).is_err());

        let (newer_traits, newer_aces) = decoded_pair(20, 21);
        state.latest_trait_component = Some(newer_traits);
        state.latest_ace_component = Some(newer_aces);
        state.rollback_component_pair(7).unwrap();

        let (retried_id, retried_traits, retried_aces) =
            state.reserve_component_pair(8).unwrap().unwrap();
        assert_eq!(retried_id, 7);
        assert_eq!(retried_traits.sequence, 10);
        assert_eq!(retried_aces.sequence, 11);
        state.commit_component_pair(7).unwrap();

        let (newer_id, newer_traits, newer_aces) =
            state.reserve_component_pair(8).unwrap().unwrap();
        assert_eq!(newer_id, 8);
        assert_eq!(newer_traits.sequence, 20);
        assert_eq!(newer_aces.sequence, 21);
    }

    #[test]
    fn stale_or_prehealthy_epoch_witness_never_admits() {
        assert!(!epoch_witness_matches(7, 7, false));
        assert!(!epoch_witness_matches(7, 8, true));
        assert!(epoch_witness_matches(8, 8, true));
    }

    #[test]
    fn decoder_registration_accepts_only_the_same_arc() {
        let first: Arc<dyn ExactA729BoundaryDecoder> = Arc::new(Decoder);
        let same = Arc::clone(&first);
        let different: Arc<dyn ExactA729BoundaryDecoder> = Arc::new(Decoder);
        assert!(same_decoder(&first, &same));
        assert!(!same_decoder(&first, &different));
    }

    #[test]
    fn decoder_reentry_guard_is_fail_closed_and_releases() {
        let outer = DecoderCallGuard::enter().unwrap();
        assert!(DecoderCallGuard::enter().is_err());
        drop(outer);
        assert!(DecoderCallGuard::enter().is_ok());
    }
}
