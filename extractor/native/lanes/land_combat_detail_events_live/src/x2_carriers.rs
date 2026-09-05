#![cfg(windows)]

use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use core::{mem, ptr};
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5,
    observe_released_set_with_retained_relay, observe_released_with_retained_relay,
    ExpectedRvaHook, HookTeardownAttestation, InstallAbortDiscard, InstalledRvaHookReceipt,
    ReleasedRvaHookReceipt, MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::{
    CombatSide, CurrentLossObservationRow, PanelObservationRow, WidthObservationRow,
    CURRENT_LOSS_FLAG_SOURCE_ACTIVE_DAMAGE, CURRENT_LOSS_FLAG_SOURCE_PARTICIPANT_LIFECYCLE,
    CURRENT_LOSS_REQUIRED_FLAG_MASK, MAX_DIVISION_ID, MAX_SIDE_UNITS,
    PANEL_MODIFIER_CANONICAL_ORDER, PANEL_MODIFIER_KNOWN_MASK, PANEL_REQUIRED_FLAG_MASK,
    WIDTH_REQUIRED_FLAG_MASK,
};

pub const WIDTH_CARRIER_ACTIVATION: bool = true;
pub const CURRENT_LOSS_CARRIER_ACTIVATION: bool = true;
pub const PANEL_CARRIER_ACTIVATION: bool = true;
pub const WIDTH_EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_X2_WIDTH_EVIDENCE_CAPTURE";
pub const LOSS_EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_X2_LOSS_EVIDENCE_CAPTURE";
pub const LOSS_EVIDENCE_NONCE_ENV: &str = "BATTLE_PLAN_X2_LOSS_EVIDENCE_NONCE";
pub const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const WIDTH_PRODUCER_INSTRUCTION_EXPORT_SHA256: &str =
    "753c4a74263fb9d791307442420fd3f2b4729a5f2e973ae26378b39e00fe2ff1";
pub const EXACT10_POOL_STATUS_RECEIPT_SHA256: &str =
    "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395";
pub const EXACT20_LOSS_BASE_JOB_ID: &str = "20260720T020812Z-w3-x2-loss-owner-5246af02";
pub const EXACT20_LOSS_BASE_MANIFEST_SHA256: &str =
    "fe48d284c6c70452d5afa31cf982d8646a3cb0a9ede11781c986d6fc087d5645";
pub const EXACT20_LOSS_SUPPLEMENT_JOB_ID: &str = "20260720T021842Z-w3-x2-loss-supplement-b7d24965";
pub const EXACT20_LOSS_SUPPLEMENT_MANIFEST_SHA256: &str =
    "75bce4aae8857b18188689fdc8d3ac1f2af30c8d4581f049b2e4d7a794e218b0";
pub const WAVE4_LOSS_LIFECYCLE_JOB_ID: &str =
    "20260720T045704Z-w4-x2-loss-producer-lifecycle-root-chain-78cee861";
pub const WAVE4_LOSS_LIFECYCLE_MANIFEST_SHA256: &str =
    "d98faa826db8fa83dfa98618a308fcae68fbb08a446dba1fe69806b0ef7d6a4f";
pub const WAVE4_WIDTH_LIFECYCLE_JOB_ID: &str =
    "20260720T051439Z-w4-x2-width-producer-lifecycle-final-5e249bfe";
pub const WAVE4_WIDTH_LIFECYCLE_JOB_JSON_SHA256: &str =
    "df15532a849e9c5f4419dfd47f6d957b6a45a3ccce66e9a2be07ac0f0a98f198";
pub const WAVE4_WIDTH_LIFECYCLE_MANIFEST_SHA256: &str =
    "b17fdbf47dfb9f150b9dd55018b81765b8e3777c42cf64b96bf04a7f3e6cd1fb";
pub const WIDTH_PRODUCER_MANIFEST_SHA256: &str =
    "a89ca3fa33225c8059a8f50a370aaba7329ac671b3f6c7ac1733822bfb9899ab";
pub const WIDTH_PRODUCER_PCODE_SHA256: &str =
    "89c077608595ffbff0de24382d70209672e790869c12b36f19cf1479c62dcd55";
pub const WIDTH_LEAF_MANIFEST_SHA256: &str =
    "875367e4aa124667515c1cdde14daed988d0a78e6d914972beed3476bca2dfb0";
pub const WIDTH_LEAF_INSTRUCTION_EXPORT_SHA256: &str =
    "8fd7bf09afeebdba78a45d0e4bfc326ea3f2612a64cfa9706cdc7172dd43e794";
pub const WIDTH_LEAF_PCODE_SHA256: &str =
    "59278dd430871b91cda3f5777a04750e1694ff96f571308e3e25ca079e40462b";
pub const PANEL_RE_JOB_ID: &str =
    "20260720T124732Z-w5-x2-combat-detail-width-leader-modifie-6852a822";
pub const PANEL_RE_MANIFEST_SHA256: &str =
    "667c7cc5105af519d951cb792c01ae85ddd9891e723dde4dc4dc1ff92f4c53dd";
pub const LEADER_RE_JOB_ID: &str =
    "20260720T124952Z-w5-x2-ccombatside-leader-resolver-and-ne-4368918d";
pub const LEADER_RE_MANIFEST_SHA256: &str =
    "598a53d39b1b32f0f36a197eb0d6b5c52fd45887bc6699579fa06cd02822f723";
pub const MODIFIER_RE_JOB_ID: &str =
    "20260720T125017Z-w5-x2-effective-combat-modifier-producer-80fc479d";
pub const MODIFIER_RE_MANIFEST_SHA256: &str =
    "cec881160fc1b8b5e3963bdbf3aee5281e4390321482936c0745780326925d1d";
pub const MODIFIER_TOOLTIP_RE_MANIFEST_SHA256: &str =
    "54c9c9f6dcb26e50f94206bb646e04ca6714661b072ff59b6b03bc73d8403ad0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedLossWriterSite {
    pub writer_rva: usize,
    pub sentinel: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedLossWriterContract {
    pub active_damage: AuthenticatedLossWriterSite,
    pub participant_lifecycle: AuthenticatedLossWriterSite,
}

const ACTIVE_DAMAGE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x44, 0x88, 0x44, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
const PARTICIPANT_LIFECYCLE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54,
];

pub const AUTHENTICATED_LOSS_WRITER: Option<AuthenticatedLossWriterContract> =
    Some(AuthenticatedLossWriterContract {
        active_damage: AuthenticatedLossWriterSite {
            writer_rva: 0x0C7_BFC0,
            sentinel: ACTIVE_DAMAGE_SENTINEL,
        },
        participant_lifecycle: AuthenticatedLossWriterSite {
            writer_rva: 0x129_9850,
            sentinel: PARTICIPANT_LIFECYCLE_SENTINEL,
        },
    });

pub fn width_observer_requested() -> bool {
    WIDTH_CARRIER_ACTIVATION
        || std::env::var(WIDTH_EVIDENCE_ACTIVATION_ENV).ok().as_deref() == Some("1")
}

pub fn width_evidence_capture_enabled() -> bool {
    !WIDTH_CARRIER_ACTIVATION && width_observer_requested()
}

fn valid_loss_evidence_nonce(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn loss_evidence_request_valid(flag: Option<&str>, nonce: Option<&str>) -> bool {
    flag == Some("1") && nonce.is_some_and(valid_loss_evidence_nonce)
}

pub fn current_loss_observer_requested() -> bool {
    CURRENT_LOSS_CARRIER_ACTIVATION
        || loss_evidence_request_valid(
            std::env::var(LOSS_EVIDENCE_ACTIVATION_ENV).ok().as_deref(),
            std::env::var(LOSS_EVIDENCE_NONCE_ENV).ok().as_deref(),
        )
}

pub fn current_loss_evidence_capture_enabled() -> bool {
    !CURRENT_LOSS_CARRIER_ACTIVATION && current_loss_observer_requested()
}

pub const WIDTH_PRODUCER_RVA: usize = 0x1299_610;
pub const DIVISION_WIDTH_LEAF_RVA: usize = 0x0C6_E270;
pub const DIVISION_WRAPPER_VTABLE_RVA: usize = 0x2943_AD0;
pub const DIVISION_VTABLE_RVA: usize = 0x2943_CB0;
const DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA: usize = 0x293_AFB8;
const DIVISION_EQUIPMENT_VARIANT_TYPE_OFFSET: usize = 0x08;
const DIVISION_EQUIPMENT_VARIANT_ID_OFFSET: usize = 0x0C;
const DIVISION_EQUIPMENT_VARIANT_TYPE_ID: u32 = 70;
const MAX_DIVISION_EQUIPMENT_VARIANT_ID: u32 = i32::MAX as u32;
pub const PANEL_SIDE_MATERIALIZER_RVA: usize = 0x15C_9860;
pub const PANEL_MODIFIER_APPEND_RVA: usize = 0x15B_BF00;
pub const PANEL_LEADER_RESOLVER_RVA: usize = 0x129_E190;
pub const ARMY_LEADER_VTABLE_RVA: usize = 0x293_F538;
const ARMY_LEADER_TYPE_OFFSET: usize = 0x08;
const ARMY_LEADER_TYPE_ID: u32 = 4_713;
const ARMY_LEADER_ID_OFFSET: usize = 0x0c;

const DIVISION_BODY_DELTA: usize = 0x10;
const DIVISION_ID_OFFSET: usize = 0x0c;
const MAX_WRAPPERS_PER_CALL: usize = 128;
const MAX_PENDING_WIDTH_CALLS: usize = 8_192;
const MAX_PENDING_LOSS_OBSERVATIONS: usize = 8_192;
const MAX_PENDING_PANEL_OBSERVATIONS: usize = 8_192;
const MAX_LOSS_EQUIPMENT_VARIANTS: usize = MAX_SIDE_UNITS;

const LOSS_OWNER_OFFSET_FROM_SIDE: usize = 0x1B0;
const LOSS_MANPOWER_OFFSET_FROM_OWNER: usize = 0x60;
const LOSS_TOTAL_EQUIPMENT_MAP_OFFSET_FROM_OWNER: usize = 0x70;
const LOSS_VARIANT_VECTOR_OFFSET_FROM_MAP: usize = 0x20;
const LOSS_VARIANT_VECTOR_ENTRY_LEN: usize = 0x10;

const WIDTH_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
const DIVISION_WIDTH_LEAF_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];
const WIDTH_PRODUCER_RECEIPT_SPEC: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: "gameplay.d55.width_producer",
    requested_rva: WIDTH_PRODUCER_RVA as u64,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &WIDTH_PRODUCER_SENTINEL,
};
const DIVISION_WIDTH_LEAF_RECEIPT_SPEC: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: "gameplay.d55.division_width_leaf",
    requested_rva: DIVISION_WIDTH_LEAF_RVA as u64,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &DIVISION_WIDTH_LEAF_SENTINEL,
};
const ACTIVE_DAMAGE_RECEIPT_SPEC: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: "gameplay.d55.active_damage_loss_writer",
    requested_rva: 0x0C7_BFC0,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &ACTIVE_DAMAGE_SENTINEL,
};
const PARTICIPANT_LIFECYCLE_RECEIPT_SPEC: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: "gameplay.d55.participant_lifecycle_loss_writer",
    requested_rva: 0x129_9850,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &PARTICIPANT_LIFECYCLE_SENTINEL,
};
const PANEL_SIDE_MATERIALIZER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x10, 0x48, 0x89, 0x78, 0x20, 0x55,
];
const PANEL_MODIFIER_APPEND_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x74, 0x24, 0x18, 0x89, 0x54, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x0f,
];
const PANEL_LEADER_RESOLVER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x18, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x33, 0xff,
];
const PANEL_SIDE_MATERIALIZER_RECEIPT_SPEC: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: "gameplay.d55.panel_side_materializer",
    requested_rva: PANEL_SIDE_MATERIALIZER_RVA as u64,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &PANEL_SIDE_MATERIALIZER_SENTINEL,
};
const PANEL_MODIFIER_APPEND_RECEIPT_SPEC: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: "gameplay.d55.panel_modifier_append",
    requested_rva: PANEL_MODIFIER_APPEND_RVA as u64,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &PANEL_MODIFIER_APPEND_SENTINEL,
};
const PANEL_LEADER_RESOLVER_RECEIPT_SPEC: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: "gameplay.d55.panel_leader_resolver",
    requested_rva: PANEL_LEADER_RESOLVER_RVA as u64,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &PANEL_LEADER_RESOLVER_SENTINEL,
};

type WidthProducerFn = unsafe extern "system" fn(usize, *mut i64) -> *mut i64;
type DivisionWidthLeafFn = unsafe extern "system" fn(usize, *mut i64, usize) -> *mut i64;
type ActiveDivisionDamageFn =
    unsafe extern "system" fn(usize, i64, u8, i64, usize, usize, usize, usize);
type ParticipantLifecycleFn = unsafe extern "system" fn(usize, usize);
type PanelSideMaterializerFn = unsafe extern "system" fn(usize, usize, u8, usize) -> usize;
type PanelModifierAppendFn = unsafe extern "system" fn(usize, u32, usize, u8, usize) -> usize;
type PanelLeaderResolverFn = unsafe extern "system" fn(usize) -> usize;

mod x2_relay;

use x2_relay::{RelayCustody, RelaySlot};

static WIDTH_PRODUCER_RELAY: RelaySlot = RelaySlot::new("width producer");
static DIVISION_WIDTH_LEAF_RELAY: RelaySlot = RelaySlot::new("division width leaf");
static ACTIVE_DIVISION_DAMAGE_RELAY: RelaySlot = RelaySlot::new("active division loss");
static PARTICIPANT_LIFECYCLE_RELAY: RelaySlot = RelaySlot::new("participant-lifecycle loss");
static PANEL_SIDE_MATERIALIZER_RELAY: RelaySlot = RelaySlot::new("combat side materializer");
static PANEL_MODIFIER_APPEND_RELAY: RelaySlot = RelaySlot::new("combat modifier append");
static PANEL_LEADER_RESOLVER_RELAY: RelaySlot = RelaySlot::new("combat leader resolver");

static WIDTH_RELAYS: [&RelaySlot; 2] = [&WIDTH_PRODUCER_RELAY, &DIVISION_WIDTH_LEAF_RELAY];
static LOSS_RELAYS: [&RelaySlot; 2] = [&ACTIVE_DIVISION_DAMAGE_RELAY, &PARTICIPANT_LIFECYCLE_RELAY];
static PANEL_RELAYS: [&RelaySlot; 3] = [
    &PANEL_SIDE_MATERIALIZER_RELAY,
    &PANEL_MODIFIER_APPEND_RELAY,
    &PANEL_LEADER_RESOLVER_RELAY,
];

static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static WIDTH_INSTALLED: AtomicBool = AtomicBool::new(false);
static LOSS_MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static LOSS_INSTALLED: AtomicBool = AtomicBool::new(false);
static PANEL_MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static PANEL_INSTALLED: AtomicBool = AtomicBool::new(false);
static NEXT_CAPTURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_LOSS_CAPTURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_PANEL_CAPTURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_WIDTH_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_LOSS_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_PANEL_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);
static WIDTH_QUEUE_SEALED: AtomicBool = AtomicBool::new(false);
static LOSS_QUEUE_SEALED: AtomicBool = AtomicBool::new(false);
static PANEL_QUEUE_SEALED: AtomicBool = AtomicBool::new(false);
static WIDTH_QUEUE_SEALING: AtomicBool = AtomicBool::new(false);
static LOSS_QUEUE_SEALING: AtomicBool = AtomicBool::new(false);
static PANEL_QUEUE_SEALING: AtomicBool = AtomicBool::new(false);
static ACTIVE_WIDTH_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_LOSS_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_PANEL_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static COMBINED_ADMITTED_WIDTH_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static COMBINED_ADMITTED_LOSS_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static COMBINED_ADMITTED_PANEL_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static WIDTH_DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static WIDTH_DROPPED_UNBOUND: AtomicU64 = AtomicU64::new(0);
static WIDTH_DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static WIDTH_DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static WIDTH_DROPPED_NESTED: AtomicU64 = AtomicU64::new(0);
static LOSS_DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static LOSS_DROPPED_UNBOUND: AtomicU64 = AtomicU64::new(0);
static LOSS_DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static LOSS_DROPPED_INVALID: AtomicU64 = AtomicU64::new(0);
static LOSS_DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static LOSS_DROPPED_NESTED: AtomicU64 = AtomicU64::new(0);
static PANEL_DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static PANEL_DROPPED_UNBOUND: AtomicU64 = AtomicU64::new(0);
static PANEL_DROPPED_INVALID: AtomicU64 = AtomicU64::new(0);
static PANEL_DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static PANEL_DROPPED_NESTED: AtomicU64 = AtomicU64::new(0);
static WIDTH_STATE: OnceLock<Mutex<WidthState>> = OnceLock::new();
static LOSS_STATE: OnceLock<Mutex<LossState>> = OnceLock::new();
static PANEL_STATE: OnceLock<Mutex<PanelState>> = OnceLock::new();
static WIDTH_RECEIPT_CUSTODY: Mutex<HookReceiptCustody<2>> = Mutex::new(HookReceiptCustody::new());
static LOSS_RECEIPT_CUSTODY: Mutex<HookReceiptCustody<2>> = Mutex::new(HookReceiptCustody::new());
static PANEL_RECEIPT_CUSTODY: Mutex<HookReceiptCustody<3>> = Mutex::new(HookReceiptCustody::new());
static WIDTH_CREATED_BY_OWNER: [AtomicBool; 2] = [const { AtomicBool::new(false) }; 2];
/// detour back out of the target. Under the process-lifetime retention policy
static WIDTH_REMOVED_BY_OWNER: [AtomicBool; 2] = [const { AtomicBool::new(false) }; 2];
static LOSS_CREATED_BY_OWNER: [AtomicBool; 2] = [const { AtomicBool::new(false) }; 2];
static LOSS_REMOVED_BY_OWNER: [AtomicBool; 2] = [const { AtomicBool::new(false) }; 2];
static PANEL_CREATED_BY_OWNER: [AtomicBool; 3] = [const { AtomicBool::new(false) }; 3];
static PANEL_REMOVED_BY_OWNER: [AtomicBool; 3] = [const { AtomicBool::new(false) }; 3];
/// disable, the release attestation and the rollback all name a site the same
const PANEL_SITE_LABELS: [&str; 3] = [
    "combat side panel materializer",
    "combat panel modifier append",
    "combat side leader resolver",
];
static WIDTH_ABORTED_RELEASE_HISTORY: Mutex<Vec<ReleasedRvaHookReceipt>> = Mutex::new(Vec::new());
static LOSS_ABORTED_RELEASE_HISTORY: Mutex<Vec<ReleasedRvaHookReceipt>> = Mutex::new(Vec::new());
static PANEL_ABORTED_RELEASE_HISTORY: Mutex<Vec<ReleasedRvaHookReceipt>> = Mutex::new(Vec::new());
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static WIDTH_CONTEXT: RefCell<Option<WidthContext>> = const { RefCell::new(None) };
    static LOSS_WRITER_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static PANEL_CONTEXT: RefCell<Option<PanelContext>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct PanelContext {
    binding: SideBinding,
    capture_sequence: u64,
    modifier_bits: u32,
    next_modifier_rank: usize,
    leader_seen: bool,
    leader_id: u32,
    invalid: bool,
}

struct PanelState {
    bindings: BTreeMap<usize, SideBinding>,
    observations: VecDeque<PanelObservationRow>,
    reservation: Option<PanelQueueReservation>,
    terminal: Option<PanelTerminalQueueState>,
    sealed: bool,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            bindings: BTreeMap::new(),
            observations: VecDeque::with_capacity(MAX_PENDING_PANEL_OBSERVATIONS),
            reservation: None,
            terminal: None,
            sealed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PanelDiagnostics {
    dropped_lock_busy: u64,
    dropped_unbound: u64,
    dropped_invalid: u64,
    dropped_overflow: u64,
    dropped_nested: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PanelQueueReservation {
    reservation_id: u64,
    row_count: usize,
    diagnostics: PanelDiagnostics,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelDrain {
    pub observations: Vec<PanelObservationRow>,
    pub dropped_lock_busy: u64,
    pub dropped_unbound: u64,
    pub dropped_invalid: u64,
    pub dropped_overflow: u64,
    pub dropped_nested: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelBatchReservation {
    pub reservation_id: u64,
    pub drain: PanelDrain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PanelTerminalQueueState {
    active_reservation_id: Option<u64>,
    row_count: usize,
    diagnostics: PanelDiagnostics,
    drain: PanelDrain,
    released_hooks: [ReleasedRvaHookReceipt; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelTerminalReservation {
    pub reservation_id: u64,
    pub drain: PanelDrain,
    pub released_hooks: [ReleasedRvaHookReceipt; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RawWrapperResult {
    division_id: Option<u32>,
    width_raw: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawWidthCallCapture {
    pub observed_game_hour_raw: Option<i32>,
    pub side_ptr: usize,
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    pub capture_sequence: u64,
    pub final_width_raw: i64,
    wrappers: [RawWrapperResult; MAX_WRAPPERS_PER_CALL],
    wrapper_count: usize,
    pub byte_gates_validated: bool,
}

impl RawWidthCallCapture {
    pub fn wrapper_count(&self) -> usize {
        self.wrapper_count
    }
}

#[derive(Clone)]
struct WidthContext {
    side_ptr: usize,
    capture_sequence: u64,
    wrappers: [RawWrapperResult; MAX_WRAPPERS_PER_CALL],
    wrapper_count: usize,
    overflowed: bool,
    unreadable: bool,
}

impl WidthContext {
    fn new(side_ptr: usize, capture_sequence: u64) -> Self {
        Self {
            side_ptr,
            capture_sequence,
            wrappers: [RawWrapperResult::default(); MAX_WRAPPERS_PER_CALL],
            wrapper_count: 0,
            overflowed: false,
            unreadable: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideBinding {
    pub side_ptr: usize,
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
}

struct WidthState {
    bindings: BTreeMap<usize, SideBinding>,
    calls: VecDeque<RawWidthCallCapture>,
    reservation: Option<WidthQueueReservation>,
    terminal: Option<WidthTerminalQueueState>,
    combined_projection: Option<WidthTerminalDrain>,
    sealed: bool,
}

impl Default for WidthState {
    fn default() -> Self {
        Self {
            bindings: BTreeMap::new(),
            calls: VecDeque::with_capacity(MAX_PENDING_WIDTH_CALLS),
            reservation: None,
            terminal: None,
            combined_projection: None,
            sealed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WidthQueueReservation {
    reservation_id: u64,
    row_count: usize,
    diagnostics: WidthDiagnostics,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidthBatchReservation {
    pub reservation_id: u64,
    pub drain: WidthDrain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WidthTerminalQueueState {
    active_reservation_id: Option<u64>,
    row_count: usize,
    diagnostics: WidthDiagnostics,
    drain: WidthTerminalDrain,
    released_hooks: [ReleasedRvaHookReceipt; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidthTerminalReservation {
    pub reservation_id: u64,
    pub drain: WidthTerminalDrain,
    pub released_hooks: [ReleasedRvaHookReceipt; 2],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WidthTerminalDrain {
    pub rows: Vec<WidthObservationRow>,
    pub dropped_lock_busy: u64,
    pub dropped_unbound: u64,
    pub dropped_unreadable: u64,
    pub dropped_overflow: u64,
    pub dropped_nested: u64,
    pub projection_failures: u64,
    pub completeness_poisoned: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WidthDrain {
    pub calls: Vec<RawWidthCallCapture>,
    pub dropped_lock_busy: u64,
    pub dropped_unbound: u64,
    pub dropped_unreadable: u64,
    pub dropped_overflow: u64,
    pub dropped_nested: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct WidthDiagnostics {
    dropped_lock_busy: u64,
    dropped_unbound: u64,
    dropped_unreadable: u64,
    dropped_overflow: u64,
    dropped_nested: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawLossWriterObservation {
    pub captured_epoch: (u32, u64, CombatSide),
    pub observed_game_hour_raw: Option<i32>,
    pub combat_ptr: usize,
    pub side_ptr: usize,
    pub capture_sequence: u64,
    pub writer_rva: usize,
    pub observed_entry_bytes: [u8; 16],
    pub manpower_before_bytes: [u8; 8],
    pub manpower_after_bytes: [u8; 8],
    pub equipment: Vec<RawEquipmentLossDelta>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawEquipmentLossDelta {
    pub equipment_id: Option<u32>,
    pub equipment_key_ptr: usize,
    pub before_bytes: [u8; 8],
    pub after_bytes: [u8; 8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LossSideBinding {
    pub combat_ptr: usize,
    pub side_ptr: usize,
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
}

pub trait LossIdentityResolver {
    fn active_epoch(&self, combat_ptr: usize, side_ptr: usize) -> Option<(u32, u64, CombatSide)>;
    fn equipment_id(&self, equipment_key_ptr: usize) -> Option<u32>;
}

impl LossIdentityResolver for RawLossWriterObservation {
    fn active_epoch(&self, combat_ptr: usize, side_ptr: usize) -> Option<(u32, u64, CombatSide)> {
        (combat_ptr == self.combat_ptr && side_ptr == self.side_ptr).then_some(self.captured_epoch)
    }
    fn equipment_id(&self, equipment_key_ptr: usize) -> Option<u32> {
        self.equipment.iter().find(|row| row.equipment_key_ptr == equipment_key_ptr)?.equipment_id
    }
}

fn capture_loss_equipment_id(equipment_key_ptr: usize) -> Option<u32> {
        let module_base = MODULE_BASE.load(Ordering::Acquire);
        if module_base == 0
            || read_usize(equipment_key_ptr)?
                != module_base
                    .checked_add(DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA)?
            || read_u32(equipment_key_ptr.checked_add(DIVISION_EQUIPMENT_VARIANT_TYPE_OFFSET)?)?
                != DIVISION_EQUIPMENT_VARIANT_TYPE_ID
        {
            return None;
        }
        let id = read_u32(equipment_key_ptr.checked_add(DIVISION_EQUIPMENT_VARIANT_ID_OFFSET)?)?;
        (id > 0 && id <= MAX_DIVISION_EQUIPMENT_VARIANT_ID).then_some(id)
    }

struct LossState {
    bindings: BTreeMap<usize, LossSideBinding>,
    observations: VecDeque<RawLossWriterObservation>,
    reservation: Option<LossQueueReservation>,
    terminal: Option<LossTerminalQueueState>,
    combined_projection: Option<LossTerminalDrain>,
    sealed: bool,
}

impl Default for LossState {
    fn default() -> Self {
        Self {
            bindings: BTreeMap::new(),
            observations: VecDeque::with_capacity(MAX_PENDING_LOSS_OBSERVATIONS),
            reservation: None,
            terminal: None,
            combined_projection: None,
            sealed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LossQueueReservation {
    reservation_id: u64,
    row_count: usize,
    diagnostics: LossDiagnostics,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LossBatchReservation {
    pub reservation_id: u64,
    pub drain: LossDrain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LossTerminalQueueState {
    active_reservation_id: Option<u64>,
    row_count: usize,
    diagnostics: LossDiagnostics,
    drain: LossTerminalDrain,
    released_hooks: [ReleasedRvaHookReceipt; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LossTerminalReservation {
    pub reservation_id: u64,
    pub drain: LossTerminalDrain,
    pub released_hooks: [ReleasedRvaHookReceipt; 2],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LossTerminalDrain {
    pub rows: Vec<CurrentLossObservationRow>,
    pub dropped_lock_busy: u64,
    pub dropped_unbound: u64,
    pub dropped_unreadable: u64,
    pub dropped_invalid: u64,
    pub dropped_overflow: u64,
    pub dropped_nested: u64,
    pub projection_failures: u64,
    pub completeness_poisoned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CombinedProjectedTerminal {
    pub width: WidthTerminalDrain,
    pub current_loss: LossTerminalDrain,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LossDrain {
    pub observations: Vec<RawLossWriterObservation>,
    pub dropped_lock_busy: u64,
    pub dropped_unbound: u64,
    pub dropped_unreadable: u64,
    pub dropped_invalid: u64,
    pub dropped_overflow: u64,
    pub dropped_nested: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LossDiagnostics {
    dropped_lock_busy: u64,
    dropped_unbound: u64,
    dropped_unreadable: u64,
    dropped_invalid: u64,
    dropped_overflow: u64,
    dropped_nested: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledWidthHookAddresses {
    pub producer: usize,
    pub division_leaf: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledLossHookAddresses {
    pub active_damage: usize,
    pub participant_lifecycle: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledPanelHookAddresses {
    pub side_materializer: usize,
    pub modifier_append: usize,
    pub leader_resolver: usize,
}

struct HookReceiptCustody<const N: usize> {
    installed: [Option<InstalledRvaHookReceipt>; N],
    released: [Option<ReleasedRvaHookReceipt>; N],
    rollback_relays: Vec<usize>,
    installed_published: bool,
    release_published: bool,
}

impl<const N: usize> HookReceiptCustody<N> {
    const fn new() -> Self {
        Self {
            installed: [None; N],
            released: [None; N],
            rollback_relays: Vec::new(),
            installed_published: false,
            release_published: false,
        }
    }
}

#[derive(Clone)]
struct LossWriterBefore {
    observed_hour_before: Option<i32>,
    binding: LossSideBinding,
    capture_sequence: u64,
    writer: AuthenticatedLossWriterSite,
    observed_entry_bytes: [u8; 16],
    manpower_before: i64,
    equipment_before: BTreeMap<usize, i64>,
}

struct ActiveWidthCallback {
    combined_admitted: bool,
}

impl ActiveWidthCallback {
    fn enter() -> Self {
        ACTIVE_WIDTH_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self {
            combined_admitted: crate::terminal_coordinator::try_admit_callback(
                &COMBINED_ADMITTED_WIDTH_CALLBACKS,
            ),
        }
    }

    fn combined_admitted(&self) -> bool {
        self.combined_admitted
    }
}

impl Drop for ActiveWidthCallback {
    fn drop(&mut self) {
        if self.combined_admitted {
            crate::terminal_coordinator::release_admitted_callback(
                &COMBINED_ADMITTED_WIDTH_CALLBACKS,
            );
        }
        ACTIVE_WIDTH_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ActiveLossCallback {
    combined_admitted: bool,
}

impl ActiveLossCallback {
    fn enter() -> Self {
        ACTIVE_LOSS_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self {
            combined_admitted: crate::terminal_coordinator::try_admit_callback(
                &COMBINED_ADMITTED_LOSS_CALLBACKS,
            ),
        }
    }

    fn combined_admitted(&self) -> bool {
        self.combined_admitted
    }
}

struct ActivePanelCallback {
    combined_admitted: bool,
}

impl ActivePanelCallback {
    fn enter() -> Self {
        ACTIVE_PANEL_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self {
            combined_admitted: crate::terminal_coordinator::try_admit_callback(
                &COMBINED_ADMITTED_PANEL_CALLBACKS,
            ),
        }
    }

    fn combined_admitted(&self) -> bool {
        self.combined_admitted
    }
}

impl Drop for ActivePanelCallback {
    fn drop(&mut self) {
        if self.combined_admitted {
            crate::terminal_coordinator::release_admitted_callback(
                &COMBINED_ADMITTED_PANEL_CALLBACKS,
            );
        }
        ACTIVE_PANEL_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for ActiveLossCallback {
    fn drop(&mut self) {
        if self.combined_admitted {
            crate::terminal_coordinator::release_admitted_callback(
                &COMBINED_ADMITTED_LOSS_CALLBACKS,
            );
        }
        ACTIVE_LOSS_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn width_addresses(base: usize) -> InstalledWidthHookAddresses {
    InstalledWidthHookAddresses {
        producer: base.saturating_add(WIDTH_PRODUCER_RVA),
        division_leaf: base.saturating_add(DIVISION_WIDTH_LEAF_RVA),
    }
}

fn loss_addresses(
    base: usize,
    contract: AuthenticatedLossWriterContract,
) -> InstalledLossHookAddresses {
    InstalledLossHookAddresses {
        active_damage: base.saturating_add(contract.active_damage.writer_rva),
        participant_lifecycle: base.saturating_add(contract.participant_lifecycle.writer_rva),
    }
}

fn panel_addresses(base: usize) -> InstalledPanelHookAddresses {
    InstalledPanelHookAddresses {
        side_materializer: base.saturating_add(PANEL_SIDE_MATERIALIZER_RVA),
        modifier_append: base.saturating_add(PANEL_MODIFIER_APPEND_RVA),
        leader_resolver: base.saturating_add(PANEL_LEADER_RESOLVER_RVA),
    }
}

const WIDTH_HOOK_NAMES: [&str; 2] = [
    "gameplay.d55.division_width_leaf",
    "gameplay.d55.width_producer",
];
const LOSS_HOOK_NAMES: [&str; 2] = [
    "gameplay.d55.participant_lifecycle_loss_writer",
    "gameplay.d55.active_damage_loss_writer",
];
const PANEL_HOOK_NAMES: [&str; 3] = [
    "gameplay.d55.panel_leader_resolver",
    "gameplay.d55.panel_modifier_append",
    "gameplay.d55.panel_side_materializer",
];

fn width_hook_batch(sites: InstalledWidthHookAddresses) -> [(&'static str, *mut c_void); 2] {
    let targets = [sites.division_leaf, sites.producer];
    core::array::from_fn(|index| (WIDTH_HOOK_NAMES[index], targets[index] as *mut c_void))
}

fn loss_hook_batch(sites: InstalledLossHookAddresses) -> [(&'static str, *mut c_void); 2] {
    let targets = [sites.participant_lifecycle, sites.active_damage];
    core::array::from_fn(|index| (LOSS_HOOK_NAMES[index], targets[index] as *mut c_void))
}

fn panel_hook_batch(sites: InstalledPanelHookAddresses) -> [(&'static str, *mut c_void); 3] {
    let targets = [
        sites.leader_resolver,
        sites.modifier_append,
        sites.side_materializer,
    ];
    core::array::from_fn(|index| (PANEL_HOOK_NAMES[index], targets[index] as *mut c_void))
}

fn reset_panel_owner_progress() {
    for index in 0..3 {
        PANEL_CREATED_BY_OWNER[index].store(false, Ordering::Release);
        PANEL_REMOVED_BY_OWNER[index].store(false, Ordering::Release);
    }
}

fn reset_width_owner_progress() {
    for index in 0..2 {
        WIDTH_CREATED_BY_OWNER[index].store(false, Ordering::Release);
        WIDTH_REMOVED_BY_OWNER[index].store(false, Ordering::Release);
    }
}

fn reset_loss_owner_progress() {
    for index in 0..2 {
        LOSS_CREATED_BY_OWNER[index].store(false, Ordering::Release);
        LOSS_REMOVED_BY_OWNER[index].store(false, Ordering::Release);
    }
}

fn require_empty_receipt_custody<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<(), String> {
    let custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    if custody.installed.iter().any(Option::is_some)
        || custody.released.iter().any(Option::is_some)
        || !custody.rollback_relays.is_empty()
        || custody.installed_published
        || custody.release_published
    {
        return Err(format!(
            "{label} receipt custody is not empty; partial or complete prior custody blocks reinstall"
        ));
    }
    Ok(())
}

fn receipt_custody_is_empty<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<bool, String> {
    let custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    Ok(custody.installed.iter().all(Option::is_none)
        && custody.released.iter().all(Option::is_none)
        && custody.rollback_relays.is_empty()
        && !custody.installed_published
        && !custody.release_published)
}

fn record_installed_receipt<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
    index: usize,
    receipt: InstalledRvaHookReceipt,
) -> Result<(), String> {
    let mut custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    if index >= N || custody.release_published || custody.released[index].is_some() {
        return Err(format!(
            "{label} receipt slot {index} is not available for install"
        ));
    }
    match custody.installed[index] {
        Some(existing) if existing == receipt => Ok(()),
        Some(_) => Err(format!("{label} installed receipt slot {index} changed")),
        None => {
            custody.installed[index] = Some(receipt);
            Ok(())
        }
    }
}

fn mark_installed_receipts_published<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<(), String> {
    let mut custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    if custody.installed.iter().any(Option::is_none)
        || custody.released.iter().any(Option::is_some)
        || custody.release_published
    {
        return Err(format!(
            "{label} cannot publish an incomplete installed roster"
        ));
    }
    custody.installed_published = true;
    Ok(())
}

fn installed_receipts<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<[InstalledRvaHookReceipt; N], String> {
    let custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    if !custody.installed_published || custody.installed.iter().any(Option::is_none) {
        return Err(format!("{label} installed hook receipts are unavailable"));
    }
    Ok(custody
        .installed
        .map(|receipt| receipt.expect("complete installed receipt roster")))
}

fn receipt_slot_for_release<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
    index: usize,
) -> Result<Option<InstalledRvaHookReceipt>, String> {
    let custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    if index >= N {
        return Err(format!(
            "{label} release receipt index {index} is out of range"
        ));
    }
    if custody.released[index].is_some() {
        return Ok(None);
    }
    Ok(custody.installed[index])
}

fn require_complete_release_custody<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<(), String> {
    let custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    if !custody.installed_published
        || custody
            .installed
            .iter()
            .zip(custody.released.iter())
            .any(|(installed, released)| installed.is_none() && released.is_none())
    {
        return Err(format!("{label} has no complete install roster to release"));
    }
    Ok(())
}

fn installed_set_for_release<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<Option<[InstalledRvaHookReceipt; N]>, String> {
    let custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    if custody.released.iter().all(Option::is_some) && custody.installed.iter().all(Option::is_none)
    {
        return Ok(None);
    }
    if !custody.installed_published
        || custody.installed.iter().any(Option::is_none)
        || custody.released.iter().any(Option::is_some)
    {
        return Err(format!(
            "{label} complete release set custody is unavailable"
        ));
    }
    Ok(Some(custody.installed.map(|receipt| {
        receipt.expect("complete installed release set")
    })))
}

fn record_released_receipt<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
    index: usize,
    installed: InstalledRvaHookReceipt,
    released: ReleasedRvaHookReceipt,
) -> Result<(), String> {
    let mut custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    if index >= N || custody.installed[index] != Some(installed) {
        return Err(format!(
            "{label} installed receipt slot {index} changed during release"
        ));
    }
    match custody.released[index] {
        Some(existing) if existing == released => {}
        Some(_) => return Err(format!("{label} released receipt slot {index} changed")),
        None => custody.released[index] = Some(released),
    }
    custody.installed[index] = None;
    Ok(())
}

fn record_released_receipt_set<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
    installed: [InstalledRvaHookReceipt; N],
    released: [ReleasedRvaHookReceipt; N],
) -> Result<(), String> {
    let mut custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    for index in 0..N {
        if custody.installed[index] != Some(installed[index]) {
            return Err(format!(
                "{label} installed receipt slot {index} changed during set release"
            ));
        }
        if custody.released[index].is_some_and(|existing| existing != released[index]) {
            return Err(format!("{label} released receipt slot {index} changed"));
        }
    }
    for index in 0..N {
        custody.released[index] = Some(released[index]);
        custody.installed[index] = None;
    }
    Ok(())
}

fn take_released_receipts<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<[ReleasedRvaHookReceipt; N], String> {
    let mut custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    if !custody.release_published || custody.released.iter().any(Option::is_none) {
        return Err(format!("{label} released hook receipts are unavailable"));
    }
    let released = custody
        .released
        .map(|receipt| receipt.expect("complete released receipt roster"));
    custody.released = [None; N];
    custody.release_published = false;
    Ok(released)
}

fn archive_resolved_partial_receipts<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    history: &Mutex<Vec<ReleasedRvaHookReceipt>>,
    label: &str,
) -> Result<Vec<ReleasedRvaHookReceipt>, String> {
    let mut custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned"))?;
    if custody.installed_published
        || custody.release_published
        || custody.installed.iter().any(Option::is_some)
        || !custody.rollback_relays.is_empty()
    {
        return Err(format!(
            "{label} has no resolved partial-install custody to archive"
        ));
    }
    if custody.released.iter().all(Option::is_none) {
        return Ok(Vec::new());
    }
    let archived: Vec<_> = custody.released.iter().flatten().copied().collect();
    history
        .lock()
        .map_err(|_| format!("{label} aborted release history mutex poisoned"))?
        .extend(archived.iter().copied());
    *custody = HookReceiptCustody::new();
    Ok(archived)
}

/// ownership bits are the only record of it.
/// nothing to release. Without it the registry had to assume every D55 group
/// might be live and plan a release unconditionally, which turned an ordinary
pub fn width_observer_release_pending() -> bool {
    WIDTH_INSTALLED.load(Ordering::Acquire)
        || WIDTH_QUEUE_SEALED.load(Ordering::Acquire)
        || (0..2).any(|index| {
            WIDTH_CREATED_BY_OWNER[index].load(Ordering::Acquire)
                && !WIDTH_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
        })
}

/// See [`width_observer_release_pending`].
pub fn current_loss_observer_release_pending() -> bool {
    LOSS_INSTALLED.load(Ordering::Acquire)
        || LOSS_QUEUE_SEALED.load(Ordering::Acquire)
        || (0..2).any(|index| {
            LOSS_CREATED_BY_OWNER[index].load(Ordering::Acquire)
                && !LOSS_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
        })
}

/// See [`width_observer_release_pending`].
pub fn panel_observer_release_pending() -> bool {
    PANEL_INSTALLED.load(Ordering::Acquire)
        || PANEL_QUEUE_SEALED.load(Ordering::Acquire)
        || (0..3).any(|index| {
            PANEL_CREATED_BY_OWNER[index].load(Ordering::Acquire)
                && !PANEL_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
        })
}

pub fn installed_width_hook_receipts() -> Result<[InstalledRvaHookReceipt; 2], String> {
    if WIDTH_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 width installed receipts are frozen by terminal teardown".into());
    }
    installed_receipts(&WIDTH_RECEIPT_CUSTODY, "X2 width")
}

pub fn take_released_width_hook_receipts() -> Result<[ReleasedRvaHookReceipt; 2], String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 width receipt handoff")?;
    if WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?
        .terminal
        .is_some()
    {
        return Err(
            "X2 width released receipts remain bound to unresolved terminal evidence".into(),
        );
    }
    take_released_receipts(&WIDTH_RECEIPT_CUSTODY, "X2 width")
}

pub fn installed_loss_hook_receipts() -> Result<[InstalledRvaHookReceipt; 2], String> {
    if LOSS_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 loss installed receipts are frozen by terminal teardown".into());
    }
    installed_receipts(&LOSS_RECEIPT_CUSTODY, "X2 loss")
}

pub fn take_released_loss_hook_receipts() -> Result<[ReleasedRvaHookReceipt; 2], String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 loss receipt handoff")?;
    if LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?
        .terminal
        .is_some()
    {
        return Err(
            "X2 loss released receipts remain bound to unresolved terminal evidence".into(),
        );
    }
    take_released_receipts(&LOSS_RECEIPT_CUSTODY, "X2 loss")
}

pub fn installed_panel_hook_receipts() -> Result<[InstalledRvaHookReceipt; 3], String> {
    if PANEL_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 panel installed receipts are frozen by terminal teardown".into());
    }
    installed_receipts(&PANEL_RECEIPT_CUSTODY, "X2 panel")
}

pub fn take_released_panel_hook_receipts() -> Result<[ReleasedRvaHookReceipt; 3], String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 panel receipt handoff")?;
    if PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?
        .terminal
        .is_some()
    {
        return Err(
            "X2 panel released receipts remain bound to unresolved terminal evidence".into(),
        );
    }
    take_released_receipts(&PANEL_RECEIPT_CUSTODY, "X2 panel")
}

pub fn aborted_width_hook_receipt_history() -> Result<Vec<ReleasedRvaHookReceipt>, String> {
    WIDTH_ABORTED_RELEASE_HISTORY
        .lock()
        .map(|history| history.clone())
        .map_err(|_| "X2 width aborted release history mutex poisoned".to_string())
}

pub fn aborted_loss_hook_receipt_history() -> Result<Vec<ReleasedRvaHookReceipt>, String> {
    LOSS_ABORTED_RELEASE_HISTORY
        .lock()
        .map(|history| history.clone())
        .map_err(|_| "X2 loss aborted release history mutex poisoned".to_string())
}

pub fn aborted_panel_hook_receipt_history() -> Result<Vec<ReleasedRvaHookReceipt>, String> {
    PANEL_ABORTED_RELEASE_HISTORY
        .lock()
        .map(|history| history.clone())
        .map_err(|_| "X2 panel aborted release history mutex poisoned".to_string())
}

pub fn archive_aborted_width_hook_receipts() -> Result<Vec<ReleasedRvaHookReceipt>, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 width receipt archive")?;
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 width aborted receipt archive mutex poisoned".to_string())?;
    if WIDTH_INSTALLED.load(Ordering::Acquire)
        || MODULE_BASE.load(Ordering::Acquire) != 0
        || WIDTH_RELAYS.iter().any(|relay| relay.is_live())
    {
        return Err(
            "X2 width aborted receipt custody cannot be archived with live hook state".into(),
        );
    }
    if WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?
        .terminal
        .is_some()
    {
        return Err("X2 width terminal receipt custody cannot be archived as an abort".into());
    }
    release_recorded_width_receipts(RETAINED_RELAY_TEARDOWN)?;
    let archived = archive_resolved_partial_receipts(
        &WIDTH_RECEIPT_CUSTODY,
        &WIDTH_ABORTED_RELEASE_HISTORY,
        "X2 width",
    )?;
    reset_width_owner_progress();
    Ok(archived)
}

pub fn archive_aborted_loss_hook_receipts() -> Result<Vec<ReleasedRvaHookReceipt>, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 loss receipt archive")?;
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 loss aborted receipt archive mutex poisoned".to_string())?;
    if LOSS_INSTALLED.load(Ordering::Acquire)
        || LOSS_MODULE_BASE.load(Ordering::Acquire) != 0
        || LOSS_RELAYS.iter().any(|relay| relay.is_live())
    {
        return Err(
            "X2 loss aborted receipt custody cannot be archived with live hook state".into(),
        );
    }
    if LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?
        .terminal
        .is_some()
    {
        return Err("X2 loss terminal receipt custody cannot be archived as an abort".into());
    }
    release_recorded_loss_receipts(RETAINED_RELAY_TEARDOWN)?;
    let archived = archive_resolved_partial_receipts(
        &LOSS_RECEIPT_CUSTODY,
        &LOSS_ABORTED_RELEASE_HISTORY,
        "X2 loss",
    )?;
    reset_loss_owner_progress();
    Ok(archived)
}

pub fn archive_aborted_panel_hook_receipts() -> Result<Vec<ReleasedRvaHookReceipt>, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 panel receipt archive")?;
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 panel aborted receipt archive mutex poisoned".to_string())?;
    if PANEL_INSTALLED.load(Ordering::Acquire)
        || PANEL_MODULE_BASE.load(Ordering::Acquire) != 0
        || PANEL_RELAYS.iter().any(|relay| relay.is_live())
    {
        return Err(
            "X2 panel aborted receipt custody cannot be archived with live hook state".into(),
        );
    }
    if PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?
        .terminal
        .is_some()
    {
        return Err("X2 panel terminal receipt custody cannot be archived as an abort".into());
    }
    release_recorded_panel_receipts(RETAINED_RELAY_TEARDOWN)?;
    let archived = archive_resolved_partial_receipts(
        &PANEL_RECEIPT_CUSTODY,
        &PANEL_ABORTED_RELEASE_HISTORY,
        "X2 panel",
    )?;
    reset_panel_owner_progress();
    Ok(archived)
}

fn observe_width_install_receipts(sites: InstalledWidthHookAddresses) -> Result<(), String> {
    let teardown_owner_token = next_teardown_owner_token()?;
    for (index, expected, target, detour) in [
        (
            0,
            WIDTH_PRODUCER_RECEIPT_SPEC,
            sites.producer,
            hook_width_producer as usize,
        ),
        (
            1,
            DIVISION_WIDTH_LEAF_RECEIPT_SPEC,
            sites.division_leaf,
            hook_division_width_leaf as usize,
        ),
    ] {
        let receipt = observe_enabled_minhook_rel5(expected, target, detour, teardown_owner_token)?;
        record_installed_receipt(&WIDTH_RECEIPT_CUSTODY, "X2 width", index, receipt)?;
    }
    mark_installed_receipts_published(&WIDTH_RECEIPT_CUSTODY, "X2 width")
}

fn observe_loss_install_receipts(sites: InstalledLossHookAddresses) -> Result<(), String> {
    let teardown_owner_token = next_teardown_owner_token()?;
    for (index, expected, target, detour) in [
        (
            0,
            ACTIVE_DAMAGE_RECEIPT_SPEC,
            sites.active_damage,
            hook_active_division_damage as usize,
        ),
        (
            1,
            PARTICIPANT_LIFECYCLE_RECEIPT_SPEC,
            sites.participant_lifecycle,
            hook_participant_lifecycle as usize,
        ),
    ] {
        let receipt = observe_enabled_minhook_rel5(expected, target, detour, teardown_owner_token)?;
        record_installed_receipt(&LOSS_RECEIPT_CUSTODY, "X2 loss", index, receipt)?;
    }
    mark_installed_receipts_published(&LOSS_RECEIPT_CUSTODY, "X2 loss")
}

fn observe_panel_install_receipts(sites: InstalledPanelHookAddresses) -> Result<(), String> {
    let teardown_owner_token = next_teardown_owner_token()?;
    for (index, expected, target, detour) in [
        (
            0,
            PANEL_SIDE_MATERIALIZER_RECEIPT_SPEC,
            sites.side_materializer,
            hook_panel_side_materializer as usize,
        ),
        (
            1,
            PANEL_MODIFIER_APPEND_RECEIPT_SPEC,
            sites.modifier_append,
            hook_panel_modifier_append as usize,
        ),
        (
            2,
            PANEL_LEADER_RESOLVER_RECEIPT_SPEC,
            sites.leader_resolver,
            hook_panel_leader_resolver as usize,
        ),
    ] {
        let receipt = observe_enabled_minhook_rel5(expected, target, detour, teardown_owner_token)?;
        record_installed_receipt(&PANEL_RECEIPT_CUSTODY, "X2 panel", index, receipt)?;
    }
    mark_installed_receipts_published(&PANEL_RECEIPT_CUSTODY, "X2 panel")
}

fn reobserve_installed_width_receipts(sites: InstalledWidthHookAddresses) -> Result<(), String> {
    let stored = installed_width_hook_receipts()?;
    let observed = [
        observe_enabled_minhook_rel5(
            WIDTH_PRODUCER_RECEIPT_SPEC,
            sites.producer,
            hook_width_producer as usize,
            stored[0].teardown_owner_token,
        )?,
        observe_enabled_minhook_rel5(
            DIVISION_WIDTH_LEAF_RECEIPT_SPEC,
            sites.division_leaf,
            hook_division_width_leaf as usize,
            stored[1].teardown_owner_token,
        )?,
    ];
    if observed != stored {
        return Err("X2 width live hook receipts changed after installation".into());
    }
    Ok(())
}

fn reobserve_installed_loss_receipts(sites: InstalledLossHookAddresses) -> Result<(), String> {
    let stored = installed_loss_hook_receipts()?;
    let observed = [
        observe_enabled_minhook_rel5(
            ACTIVE_DAMAGE_RECEIPT_SPEC,
            sites.active_damage,
            hook_active_division_damage as usize,
            stored[0].teardown_owner_token,
        )?,
        observe_enabled_minhook_rel5(
            PARTICIPANT_LIFECYCLE_RECEIPT_SPEC,
            sites.participant_lifecycle,
            hook_participant_lifecycle as usize,
            stored[1].teardown_owner_token,
        )?,
    ];
    if observed != stored {
        return Err("X2 loss live hook receipts changed after installation".into());
    }
    Ok(())
}

fn reobserve_installed_panel_receipts(sites: InstalledPanelHookAddresses) -> Result<(), String> {
    let stored = installed_panel_hook_receipts()?;
    let observed = [
        observe_enabled_minhook_rel5(
            PANEL_SIDE_MATERIALIZER_RECEIPT_SPEC,
            sites.side_materializer,
            hook_panel_side_materializer as usize,
            stored[0].teardown_owner_token,
        )?,
        observe_enabled_minhook_rel5(
            PANEL_MODIFIER_APPEND_RECEIPT_SPEC,
            sites.modifier_append,
            hook_panel_modifier_append as usize,
            stored[1].teardown_owner_token,
        )?,
        observe_enabled_minhook_rel5(
            PANEL_LEADER_RESOLVER_RECEIPT_SPEC,
            sites.leader_resolver,
            hook_panel_leader_resolver as usize,
            stored[2].teardown_owner_token,
        )?,
    ];
    if observed != stored {
        return Err("X2 panel live hook receipts changed after installation".into());
    }
    Ok(())
}

/// The one teardown attestation every X2 release path passes, and the only one
/// (`observe_released_with_retained_relay` /
/// `observe_released_set_with_retained_relay`) refuse a claimed free outright,
const RETAINED_RELAY_TEARDOWN: HookTeardownAttestation = HookTeardownAttestation {
    callbacks_quiesced: true,
    hook_removed: true,
    relay_freed: false,
};

/// Refuse to release a receipt whose relay the retention census never recorded.
/// `close_*_detours_and_retain_regions` step inside each `disable_*_for_release`.
/// that claims process-lifetime retention.
fn require_retained_relay_census(
    label: &str,
    relay: usize,
    teardown_owner_token: u64,
) -> Result<(), String> {
    let end = relay
        .checked_add(crate::a729_hook::MINHOOK_RELAY_LEN)
        .ok_or_else(|| format!("{label} retained MinHook relay range overflowed"))?;
    crate::a729_hook::require_regions_recorded_retained(
        label,
        teardown_owner_token,
        &[(relay, end)],
    )
}

/// Release the width receipts that are individually resolvable, retaining every
/// Nothing here frees anything: `disable_*_hooks_for_release` restored the
fn release_recorded_width_receipts(teardown: HookTeardownAttestation) -> Result<(), String> {
    for (index, expected) in [
        (0, WIDTH_PRODUCER_RECEIPT_SPEC),
        (1, DIVISION_WIDTH_LEAF_RECEIPT_SPEC),
    ] {
        let Some(installed) = receipt_slot_for_release(&WIDTH_RECEIPT_CUSTODY, "X2 width", index)?
        else {
            continue;
        };
        if !WIDTH_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            || !WIDTH_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
        {
            return Err(format!(
                "X2 width receipt slot {index} lacks owner-performed removal custody"
            ));
        }
        require_retained_relay_census(
            "X2 width",
            installed.observed_relay_address,
            installed.teardown_owner_token,
        )?;
        let released = observe_released_with_retained_relay(expected, installed, teardown)?;
        record_released_receipt(
            &WIDTH_RECEIPT_CUSTODY,
            "X2 width",
            index,
            installed,
            released,
        )?;
    }
    if (0..2).any(|index| {
        WIDTH_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            && !WIDTH_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
    }) {
        return Err("X2 width retains an owner-created hook during receipt release".into());
    }
    clear_rollback_relays(&WIDTH_RECEIPT_CUSTODY, "X2 width")
}

/// Release the loss receipts that are individually resolvable, retaining every
/// [`release_recorded_width_receipts`] for the shape and
fn release_recorded_loss_receipts(teardown: HookTeardownAttestation) -> Result<(), String> {
    for (index, expected) in [
        (0, ACTIVE_DAMAGE_RECEIPT_SPEC),
        (1, PARTICIPANT_LIFECYCLE_RECEIPT_SPEC),
    ] {
        let Some(installed) = receipt_slot_for_release(&LOSS_RECEIPT_CUSTODY, "X2 loss", index)?
        else {
            continue;
        };
        if !LOSS_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            || !LOSS_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
        {
            return Err(format!(
                "X2 loss receipt slot {index} lacks owner-performed removal custody"
            ));
        }
        require_retained_relay_census(
            "X2 loss",
            installed.observed_relay_address,
            installed.teardown_owner_token,
        )?;
        let released = observe_released_with_retained_relay(expected, installed, teardown)?;
        record_released_receipt(&LOSS_RECEIPT_CUSTODY, "X2 loss", index, installed, released)?;
    }
    if (0..2).any(|index| {
        LOSS_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            && !LOSS_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
    }) {
        return Err("X2 loss retains an owner-created hook during receipt release".into());
    }
    clear_rollback_relays(&LOSS_RECEIPT_CUSTODY, "X2 loss")
}

/// Release the panel receipts that are individually resolvable, retaining every
/// [`release_recorded_width_receipts`] for the shape and
fn release_recorded_panel_receipts(teardown: HookTeardownAttestation) -> Result<(), String> {
    for (index, expected) in [
        (0, PANEL_SIDE_MATERIALIZER_RECEIPT_SPEC),
        (1, PANEL_MODIFIER_APPEND_RECEIPT_SPEC),
        (2, PANEL_LEADER_RESOLVER_RECEIPT_SPEC),
    ] {
        let Some(installed) = receipt_slot_for_release(&PANEL_RECEIPT_CUSTODY, "X2 panel", index)?
        else {
            continue;
        };
        if !PANEL_CREATED_BY_OWNER[index].load(Ordering::Acquire) {
            return Err(format!(
                "X2 panel receipt slot {index} lost created-by-owner custody"
            ));
        }
        if !PANEL_REMOVED_BY_OWNER[index].load(Ordering::Acquire) {
            return Err(format!(
                "X2 panel receipt slot {index} cannot be released before owner-performed removal"
            ));
        }
        require_retained_relay_census(
            "X2 panel",
            installed.observed_relay_address,
            installed.teardown_owner_token,
        )?;
        let released = observe_released_with_retained_relay(expected, installed, teardown)?;
        record_released_receipt(
            &PANEL_RECEIPT_CUSTODY,
            "X2 panel",
            index,
            installed,
            released,
        )?;
    }
    if (0..3).any(|index| {
        PANEL_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            && !PANEL_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
    }) {
        return Err("X2 panel retains an owner-created hook during receipt release".into());
    }
    clear_rollback_relays(&PANEL_RECEIPT_CUSTODY, "X2 panel")
}

fn release_complete_width_receipt_set(teardown: HookTeardownAttestation) -> Result<(), String> {
    let Some(installed) = installed_set_for_release(&WIDTH_RECEIPT_CUSTODY, "X2 width")? else {
        return Ok(());
    };
    if (0..2).any(|index| {
        !WIDTH_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            || !WIDTH_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
    }) {
        return Err("X2 width complete release lacks owner-performed removal custody".into());
    }
    for receipt in &installed {
        require_retained_relay_census(
            "X2 width",
            receipt.observed_relay_address,
            receipt.teardown_owner_token,
        )?;
    }
    let released = observe_released_set_with_retained_relay(
        [
            WIDTH_PRODUCER_RECEIPT_SPEC,
            DIVISION_WIDTH_LEAF_RECEIPT_SPEC,
        ],
        installed,
        [teardown; 2],
    )?;
    record_released_receipt_set(&WIDTH_RECEIPT_CUSTODY, "X2 width", installed, released)
}

fn release_complete_loss_receipt_set(teardown: HookTeardownAttestation) -> Result<(), String> {
    let Some(installed) = installed_set_for_release(&LOSS_RECEIPT_CUSTODY, "X2 loss")? else {
        return Ok(());
    };
    if (0..2).any(|index| {
        !LOSS_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            || !LOSS_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
    }) {
        return Err("X2 loss complete release lacks owner-performed removal custody".into());
    }
    for receipt in &installed {
        require_retained_relay_census(
            "X2 loss",
            receipt.observed_relay_address,
            receipt.teardown_owner_token,
        )?;
    }
    let released = observe_released_set_with_retained_relay(
        [
            ACTIVE_DAMAGE_RECEIPT_SPEC,
            PARTICIPANT_LIFECYCLE_RECEIPT_SPEC,
        ],
        installed,
        [teardown; 2],
    )?;
    record_released_receipt_set(&LOSS_RECEIPT_CUSTODY, "X2 loss", installed, released)
}

fn wait_for_width_quiescence() -> Result<(), String> {
    let mut yields = 0u32;
    while ACTIVE_WIDTH_CALLBACKS.load(Ordering::Acquire) != 0 {
        if yields == 1_000_000 {
            return Err("X2 width callbacks did not quiesce".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    Ok(())
}

fn wait_for_loss_quiescence() -> Result<(), String> {
    let mut yields = 0u32;
    while ACTIVE_LOSS_CALLBACKS.load(Ordering::Acquire) != 0 {
        if yields == 1_000_000 {
            return Err("X2 loss callbacks did not quiesce".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    Ok(())
}

fn wait_for_panel_quiescence() -> Result<(), String> {
    let mut yields = 0u32;
    while ACTIVE_PANEL_CALLBACKS.load(Ordering::Acquire) != 0 {
        if yields == 1_000_000 {
            return Err("X2 panel callbacks did not quiesce".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    Ok(())
}

/// receipt, in which case nothing that requires one can be released against it.
fn receipt_roster_owner_token<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<Option<u64>, String> {
    let custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned reading owner token"))?;
    let mut token = None;
    for receipt in custody.installed.iter().flatten() {
        match token {
            None => token = Some(receipt.teardown_owner_token),
            Some(existing) if existing == receipt.teardown_owner_token => {}
            Some(_) => {
                return Err(format!(
                    "{label} installed receipts disagree about their teardown owner"
                ))
            }
        }
    }
    Ok(token)
}

fn receipt_relay_ranges<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<Vec<(usize, usize)>, String> {
    let custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned during removal census"))?;
    let mut ranges: Vec<_> = custody
        .installed
        .iter()
        .flatten()
        .map(|receipt| {
            let start = receipt.observed_relay_address;
            let end = start
                .checked_add(crate::a729_hook::MINHOOK_RELAY_LEN)
                .ok_or_else(|| format!("{label} MinHook relay range overflowed"))?;
            if start == 0 {
                return Err(format!(
                    "{label} installed receipt has no MinHook relay address"
                ));
            }
            Ok((start, end))
        })
        .collect::<Result<_, _>>()?;
    for relay in &custody.rollback_relays {
        let end = relay
            .checked_add(crate::a729_hook::MINHOOK_RELAY_LEN)
            .ok_or_else(|| format!("{label} retained MinHook relay range overflowed"))?;
        if *relay == 0 {
            return Err(format!("{label} retained a null MinHook relay"));
        }
        if !ranges.contains(&(*relay, end)) {
            ranges.push((*relay, end));
        }
    }
    Ok(ranges)
}

fn observe_live_relay(
    target: usize,
    original_prefix: &[u8],
    expected_detour: usize,
    label: &str,
) -> Result<Option<usize>, String> {
    let bytes = read_process_bytes(target, 5)
        .ok_or_else(|| format!("{label} target prefix is unreadable during rollback"))?;
    if bytes == original_prefix[..5] {
        return Ok(None);
    }
    if bytes.first().copied() != Some(0xE9) {
        return Err(format!(
            "{label} has neither its original prefix nor an E9 MinHook relay"
        ));
    }
    let displacement = i32::from_le_bytes(
        bytes[1..5]
            .try_into()
            .map_err(|_| format!("{label} MinHook displacement is truncated"))?,
    ) as isize;
    let relay = target
        .checked_add(5)
        .and_then(|next| next.checked_add_signed(displacement))
        .ok_or_else(|| format!("{label} MinHook relay destination overflowed"))?;
    let stub = read_process_bytes(relay, crate::a729_hook::MINHOOK_RELAY_LEN)
        .ok_or_else(|| format!("{label} MinHook relay stub is unreadable"))?;
    if stub[..6] != [0xFF, 0x25, 0, 0, 0, 0] {
        return Err(format!("{label} MinHook relay stub is not FF25 absolute"));
    }
    let detour = usize::try_from(u64::from_le_bytes(
        stub[6..14]
            .try_into()
            .map_err(|_| format!("{label} MinHook detour is truncated"))?,
    ))
    .map_err(|_| format!("{label} MinHook detour does not fit usize"))?;
    if detour != expected_detour {
        return Err(format!(
            "{label} MinHook relay targets 0x{detour:X}, expected 0x{expected_detour:X}"
        ));
    }
    Ok(Some(relay))
}

fn retain_live_relays<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
    relays: impl IntoIterator<Item = usize>,
) -> Result<(), String> {
    let mut custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned retaining live relays"))?;
    for relay in relays {
        if !custody.rollback_relays.contains(&relay) {
            custody.rollback_relays.push(relay);
        }
    }
    Ok(())
}

fn clear_rollback_relays<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<(), String> {
    custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned clearing live relays"))?
        .rollback_relays
        .clear();
    Ok(())
}

fn retain_width_live_relays(sites: InstalledWidthHookAddresses) -> Result<(), String> {
    let relays = [
        observe_live_relay(
            sites.producer,
            &WIDTH_PRODUCER_SENTINEL,
            hook_width_producer as usize,
            "X2 width producer",
        )?,
        observe_live_relay(
            sites.division_leaf,
            &DIVISION_WIDTH_LEAF_SENTINEL,
            hook_division_width_leaf as usize,
            "X2 division-width leaf",
        )?,
    ]
    .into_iter()
    .flatten();
    retain_live_relays(&WIDTH_RECEIPT_CUSTODY, "X2 width", relays)
}

fn retain_loss_live_relays(sites: InstalledLossHookAddresses) -> Result<(), String> {
    let relays = [
        observe_live_relay(
            sites.active_damage,
            &ACTIVE_DAMAGE_SENTINEL,
            hook_active_division_damage as usize,
            "X2 active-damage writer",
        )?,
        observe_live_relay(
            sites.participant_lifecycle,
            &PARTICIPANT_LIFECYCLE_SENTINEL,
            hook_participant_lifecycle as usize,
            "X2 participant lifecycle",
        )?,
    ]
    .into_iter()
    .flatten();
    retain_live_relays(&LOSS_RECEIPT_CUSTODY, "X2 loss", relays)
}

fn retain_panel_live_relays(sites: InstalledPanelHookAddresses) -> Result<(), String> {
    let relays = [
        observe_live_relay(
            sites.side_materializer,
            &PANEL_SIDE_MATERIALIZER_SENTINEL,
            hook_panel_side_materializer as usize,
            "X2 panel side materializer",
        )?,
        observe_live_relay(
            sites.modifier_append,
            &PANEL_MODIFIER_APPEND_SENTINEL,
            hook_panel_modifier_append as usize,
            "X2 panel modifier append",
        )?,
        observe_live_relay(
            sites.leader_resolver,
            &PANEL_LEADER_RESOLVER_SENTINEL,
            hook_panel_leader_resolver as usize,
            "X2 panel leader resolver",
        )?,
    ]
    .into_iter()
    .flatten();
    retain_live_relays(&PANEL_RECEIPT_CUSTODY, "X2 panel", relays)
}

fn close_width_detours_and_retain_regions() -> Result<(), String> {
    let mut ranges = receipt_relay_ranges(&WIDTH_RECEIPT_CUSTODY, "X2 width")?;
    ranges.push(crate::a729_hook::exact_runtime_function_range(
        hook_width_producer as usize,
        "X2 width producer detour",
    )?);
    ranges.push(crate::a729_hook::exact_runtime_function_range(
        hook_division_width_leaf as usize,
        "X2 division-width leaf detour",
    )?);
    crate::a729_hook::close_callbacks_and_retain_regions(
        "X2 width teardown",
        receipt_roster_owner_token(&WIDTH_RECEIPT_CUSTODY, "X2 width")?,
        &[&ACTIVE_WIDTH_CALLBACKS],
        &ranges,
    )
    .map(|_retained| ())
}

fn close_loss_detours_and_retain_regions() -> Result<(), String> {
    let mut ranges = receipt_relay_ranges(&LOSS_RECEIPT_CUSTODY, "X2 loss")?;
    ranges.push(crate::a729_hook::exact_runtime_function_range(
        hook_active_division_damage as usize,
        "X2 active-damage detour",
    )?);
    ranges.push(crate::a729_hook::exact_runtime_function_range(
        hook_participant_lifecycle as usize,
        "X2 participant-lifecycle detour",
    )?);
    crate::a729_hook::close_callbacks_and_retain_regions(
        "X2 loss teardown",
        receipt_roster_owner_token(&LOSS_RECEIPT_CUSTODY, "X2 loss")?,
        &[&ACTIVE_LOSS_CALLBACKS],
        &ranges,
    )
    .map(|_retained| ())
}

fn close_panel_detours_and_retain_regions() -> Result<(), String> {
    let mut ranges = receipt_relay_ranges(&PANEL_RECEIPT_CUSTODY, "X2 panel")?;
    ranges.push(crate::a729_hook::exact_runtime_function_range(
        hook_panel_side_materializer as usize,
        "X2 panel side-materializer detour",
    )?);
    ranges.push(crate::a729_hook::exact_runtime_function_range(
        hook_panel_modifier_append as usize,
        "X2 panel modifier-append detour",
    )?);
    ranges.push(crate::a729_hook::exact_runtime_function_range(
        hook_panel_leader_resolver as usize,
        "X2 panel leader-resolver detour",
    )?);
    crate::a729_hook::close_callbacks_and_retain_regions(
        "X2 panel teardown",
        receipt_roster_owner_token(&PANEL_RECEIPT_CUSTODY, "X2 panel")?,
        &[&ACTIVE_PANEL_CALLBACKS],
        &ranges,
    )
    .map(|_retained| ())
}

fn combined_receipt_roster_ready<const N: usize>(
    custody: &Mutex<HookReceiptCustody<N>>,
    label: &str,
) -> Result<(), String> {
    let custody = custody
        .lock()
        .map_err(|_| format!("{label} receipt custody mutex poisoned during combined preflight"))?;
    if !custody.installed_published
        || custody.release_published
        || custody.installed.iter().any(Option::is_none)
        || custody.released.iter().any(Option::is_some)
        || !custody.rollback_relays.is_empty()
    {
        return Err(format!(
            "{label} combined preflight requires one complete live installed receipt roster"
        ));
    }
    Ok(())
}

pub(crate) fn wait_for_combined_capture_quiescence() -> Result<(), String> {
    let mut yields = 0u32;
    while COMBINED_ADMITTED_WIDTH_CALLBACKS.load(Ordering::Acquire) != 0
        || COMBINED_ADMITTED_LOSS_CALLBACKS.load(Ordering::Acquire) != 0
        || COMBINED_ADMITTED_PANEL_CALLBACKS.load(Ordering::Acquire) != 0
    {
        if yields == 1_000_000 {
            return Err("X2 combined D55 capture callbacks did not quiesce".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    let width_state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned after combined quiescence".to_string())?;
    let loss_state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned after combined quiescence".to_string())?;
    let panel_state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned after combined quiescence".to_string())?;
    if width_state.reservation.is_some()
        || loss_state.reservation.is_some()
        || panel_state.reservation.is_some()
        || width_state.terminal.is_some()
        || loss_state.terminal.is_some()
        || panel_state.terminal.is_some()
        || width_state.sealed
        || loss_state.sealed
        || panel_state.sealed
    {
        return Err(
            "X2 combined cutoff found unresolved individual reservation or terminal custody".into(),
        );
    }
    if !WIDTH_INSTALLED.load(Ordering::Acquire)
        || !LOSS_INSTALLED.load(Ordering::Acquire)
        || !PANEL_INSTALLED.load(Ordering::Acquire)
        || MODULE_BASE.load(Ordering::Acquire) == 0
        || LOSS_MODULE_BASE.load(Ordering::Acquire) == 0
        || PANEL_MODULE_BASE.load(Ordering::Acquire) == 0
        || WIDTH_RELAYS
            .iter()
            .chain(LOSS_RELAYS.iter())
            .chain(PANEL_RELAYS.iter())
            .any(|relay| !relay.is_live())
        || WIDTH_QUEUE_SEALING.load(Ordering::Acquire)
        || LOSS_QUEUE_SEALING.load(Ordering::Acquire)
        || PANEL_QUEUE_SEALING.load(Ordering::Acquire)
        || WIDTH_QUEUE_SEALED.load(Ordering::Acquire)
        || LOSS_QUEUE_SEALED.load(Ordering::Acquire)
        || PANEL_QUEUE_SEALED.load(Ordering::Acquire)
    {
        return Err("X2 combined cutoff requires all seven hooks in complete live custody".into());
    }
    if WIDTH_CREATED_BY_OWNER
        .iter()
        .any(|flag| !flag.load(Ordering::Acquire))
        || LOSS_CREATED_BY_OWNER
            .iter()
            .any(|flag| !flag.load(Ordering::Acquire))
        || PANEL_CREATED_BY_OWNER
            .iter()
            .any(|flag| !flag.load(Ordering::Acquire))
        || WIDTH_REMOVED_BY_OWNER
            .iter()
            .chain(LOSS_REMOVED_BY_OWNER.iter())
            .chain(PANEL_REMOVED_BY_OWNER.iter())
            .any(|flag| flag.load(Ordering::Acquire))
    {
        return Err("X2 combined cutoff found incomplete physical hook-owner custody".into());
    }
    combined_receipt_roster_ready(&WIDTH_RECEIPT_CUSTODY, "X2 width")?;
    combined_receipt_roster_ready(&LOSS_RECEIPT_CUSTODY, "X2 loss")?;
    combined_receipt_roster_ready(&PANEL_RECEIPT_CUSTODY, "X2 panel")?;
    reobserve_installed_width_receipts(width_addresses(MODULE_BASE.load(Ordering::Acquire)))?;
    let contract = AUTHENTICATED_LOSS_WRITER
        .ok_or_else(|| "X2 combined cutoff lost authenticated loss contract".to_string())?;
    reobserve_installed_loss_receipts(loss_addresses(
        LOSS_MODULE_BASE.load(Ordering::Acquire),
        contract,
    ))?;
    reobserve_installed_panel_receipts(panel_addresses(PANEL_MODULE_BASE.load(Ordering::Acquire)))?;
    Ok(())
}

fn create_or_adopt_hook(target: usize, detour: usize, relay: &RelaySlot) -> Result<usize, String> {
    let created = unsafe { MinHook::create_hook(target as *mut c_void, detour as *mut c_void) }
        .map(|trampoline| trampoline as usize);
    resolve_created_or_adopted(created, relay)
}

fn resolve_created_or_adopted(
    created: Result<usize, MH_STATUS>,
    relay: &RelaySlot,
) -> Result<usize, String> {
    match created {
        Ok(trampoline) => relay.record_created(trampoline),
        Err(MH_STATUS::MH_ERROR_ALREADY_CREATED) => relay.adopt_retained(),
        Err(status) => Err(format!("{} hook creation failed: {status}", relay.label())),
    }
}

/// code, which `verify_sentinel` and the release receipt each re-read out of
/// by this owner, and claiming otherwise would let a release attest custody
fn disable_and_retain_relay(
    target: usize,
    label: &str,
    removed_by_owner: &AtomicBool,
) -> Result<(), String> {
    match unsafe { inline_detour::disable(target as *mut c_void) } {
        Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED)) => {
            removed_by_owner.store(true, Ordering::Release);
            Ok(())
        }
        Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => Err(format!(
            "{label} hook at 0x{target:X} vanished before owner-performed removal"
        )),
        Err(status) => Err(format!("{label} disable failed at 0x{target:X}: {status}")),
    }
}

fn disable_width_hooks_for_release(sites: InstalledWidthHookAddresses) -> Result<(), String> {
    retain_width_live_relays(sites)?;
    for (index, target, label) in [
        (0, sites.producer, "width producer"),
        (1, sites.division_leaf, "division width leaf"),
    ] {
        if !WIDTH_CREATED_BY_OWNER[index].load(Ordering::Acquire) {
            continue;
        }
        if WIDTH_REMOVED_BY_OWNER[index].load(Ordering::Acquire) {
            continue;
        }
        disable_and_retain_relay(target, label, &WIDTH_REMOVED_BY_OWNER[index])?;
    }
    wait_for_width_quiescence()?;
    close_width_detours_and_retain_regions()
}

fn disable_loss_hooks_for_release(sites: InstalledLossHookAddresses) -> Result<(), String> {
    retain_loss_live_relays(sites)?;
    let mut failures = Vec::new();
    for (index, target, label) in [
        (0, sites.active_damage, "active division loss writer"),
        (
            1,
            sites.participant_lifecycle,
            "participant-lifecycle loss writer",
        ),
    ] {
        if !LOSS_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            || LOSS_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
        {
            continue;
        }
        if let Err(error) = disable_and_retain_relay(target, label, &LOSS_REMOVED_BY_OWNER[index]) {
            failures.push(error);
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "X2 loss hook disable failed: {}",
            failures.join("; ")
        ));
    }
    wait_for_loss_quiescence()?;
    close_loss_detours_and_retain_regions()
}

fn disable_panel_hooks_for_release(sites: InstalledPanelHookAddresses) -> Result<(), String> {
    retain_panel_live_relays(sites)?;
    for (index, target, label) in [
        (0, sites.side_materializer, PANEL_SITE_LABELS[0]),
        (1, sites.modifier_append, PANEL_SITE_LABELS[1]),
        (2, sites.leader_resolver, PANEL_SITE_LABELS[2]),
    ] {
        if receipt_slot_for_release(&PANEL_RECEIPT_CUSTODY, "X2 panel", index)?.is_none() {
            continue;
        }
        if !PANEL_CREATED_BY_OWNER[index].load(Ordering::Acquire) {
            return Err(format!(
                "X2 panel receipt slot {index} lost created-by-owner custody"
            ));
        }
        if PANEL_REMOVED_BY_OWNER[index].load(Ordering::Acquire) {
            continue;
        }
        disable_and_retain_relay(target, label, &PANEL_REMOVED_BY_OWNER[index])?;
    }
    wait_for_panel_quiescence()?;
    close_panel_detours_and_retain_regions()
}

/// Attest and release all three panel sites, each with its relay retained.
/// belong to `disable_panel_hooks_for_release`, which runs immediately before
/// that disable cannot be released.
fn attest_and_release_retained_panel_hooks(
    sites: InstalledPanelHookAddresses,
) -> Result<(), String> {
    let teardown = RETAINED_RELAY_TEARDOWN;
    for (index, expected, target, sentinel, label) in [
        (
            0,
            PANEL_SIDE_MATERIALIZER_RECEIPT_SPEC,
            sites.side_materializer,
            PANEL_SIDE_MATERIALIZER_SENTINEL.as_slice(),
            "combat side panel materializer",
        ),
        (
            1,
            PANEL_MODIFIER_APPEND_RECEIPT_SPEC,
            sites.modifier_append,
            PANEL_MODIFIER_APPEND_SENTINEL.as_slice(),
            "combat panel modifier append",
        ),
        (
            2,
            PANEL_LEADER_RESOLVER_RECEIPT_SPEC,
            sites.leader_resolver,
            PANEL_LEADER_RESOLVER_SENTINEL.as_slice(),
            "combat side leader resolver",
        ),
    ] {
        let Some(installed) = receipt_slot_for_release(&PANEL_RECEIPT_CUSTODY, "X2 panel", index)?
        else {
            continue;
        };
        if !PANEL_CREATED_BY_OWNER[index].load(Ordering::Acquire) {
            return Err(format!(
                "X2 panel receipt slot {index} lost created-by-owner custody"
            ));
        }
        if !PANEL_REMOVED_BY_OWNER[index].load(Ordering::Acquire) {
            return Err(format!(
                "{label} hook at 0x{target:X} reached release without the owner-performed \
                 disable that retains its relay"
            ));
        }
        verify_sentinel(target, sentinel, label)?;
        require_retained_relay_census(
            "X2 panel",
            installed.observed_relay_address,
            installed.teardown_owner_token,
        )?;
        let released = observe_released_with_retained_relay(expected, installed, teardown)?;
        record_released_receipt(
            &PANEL_RECEIPT_CUSTODY,
            "X2 panel",
            index,
            installed,
            released,
        )?;
    }
    let custody = PANEL_RECEIPT_CUSTODY
        .lock()
        .map_err(|_| "X2 panel receipt custody mutex poisoned".to_string())?;
    if custody.installed.iter().any(Option::is_some) || custody.released.iter().any(Option::is_none)
    {
        return Err("X2 panel release roster is incomplete".into());
    }
    Ok(())
}

fn rollback_failed_width_install(sites: InstalledWidthHookAddresses) -> Result<(), String> {
    disable_width_hooks_for_release(sites)?;
    let mut failures = Vec::new();
    // Nothing is removed or freed here. `disable_width_hooks_for_release` above
    for (index, label) in [(1, "division width leaf"), (0, "width producer")] {
        if !WIDTH_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            || WIDTH_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
        {
            continue;
        }
        failures.push(format!(
            "{label} hook was not disabled before owner-performed rollback retention"
        ));
    }
    for (index, target, sentinel, label) in [
        (
            0,
            sites.producer,
            WIDTH_PRODUCER_SENTINEL.as_slice(),
            "width producer",
        ),
        (
            1,
            sites.division_leaf,
            DIVISION_WIDTH_LEAF_SENTINEL.as_slice(),
            "division width leaf",
        ),
    ] {
        if WIDTH_REMOVED_BY_OWNER[index].load(Ordering::Acquire) {
            if let Err(error) = verify_sentinel(target, sentinel, label) {
                failures.push(error);
            }
        }
    }
    if failures.is_empty() {
        if let Err(error) = clear_rollback_relays(&WIDTH_RECEIPT_CUSTODY, "X2 width") {
            failures.push(error);
        }
    }
    if failures.is_empty() {
        match receipt_custody_is_empty(&WIDTH_RECEIPT_CUSTODY, "X2 width") {
            Ok(true) => reset_width_owner_progress(),
            Ok(false) => {}
            Err(error) => failures.push(error),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn rollback_failed_loss_install(sites: InstalledLossHookAddresses) -> Result<(), String> {
    disable_loss_hooks_for_release(sites)?;
    let mut failures = Vec::new();
    for (index, label) in [
        (0, "active division loss writer"),
        (1, "participant-lifecycle loss writer"),
    ] {
        if !LOSS_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            || LOSS_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
        {
            continue;
        }
        failures.push(format!(
            "{label} hook was not disabled before owner-performed rollback retention"
        ));
    }
    for (index, target, sentinel, label) in [
        (
            0,
            sites.active_damage,
            ACTIVE_DAMAGE_SENTINEL.as_slice(),
            "active division loss writer",
        ),
        (
            1,
            sites.participant_lifecycle,
            PARTICIPANT_LIFECYCLE_SENTINEL.as_slice(),
            "participant-lifecycle loss writer",
        ),
    ] {
        if LOSS_REMOVED_BY_OWNER[index].load(Ordering::Acquire) {
            if let Err(error) = verify_sentinel(target, sentinel, label) {
                failures.push(error);
            }
        }
    }
    if failures.is_empty() {
        if let Err(error) = clear_rollback_relays(&LOSS_RECEIPT_CUSTODY, "X2 loss") {
            failures.push(error);
        }
    }
    if failures.is_empty() {
        match receipt_custody_is_empty(&LOSS_RECEIPT_CUSTODY, "X2 loss") {
            Ok(true) => reset_loss_owner_progress(),
            Ok(false) => {}
            Err(error) => failures.push(error),
        }
    }
    if failures.is_empty() {
        clear_rollback_relays(&PANEL_RECEIPT_CUSTODY, "X2 panel")
    } else {
        Err(failures.join("; "))
    }
}

fn rollback_failed_panel_install(sites: InstalledPanelHookAddresses) -> Result<(), String> {
    retain_panel_live_relays(sites)?;
    let mut failures = Vec::new();
    for (index, target, label) in [
        (0, sites.side_materializer, PANEL_SITE_LABELS[0]),
        (1, sites.modifier_append, PANEL_SITE_LABELS[1]),
        (2, sites.leader_resolver, PANEL_SITE_LABELS[2]),
    ] {
        if !PANEL_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            || PANEL_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
        {
            continue;
        }
        if let Err(error) = disable_and_retain_relay(target, label, &PANEL_REMOVED_BY_OWNER[index])
        {
            failures.push(error);
        }
    }
    if !failures.is_empty() {
        return Err(failures.join("; "));
    }
    wait_for_panel_quiescence()?;
    close_panel_detours_and_retain_regions()?;
    for (index, target, sentinel, label) in [
        (
            0,
            sites.side_materializer,
            PANEL_SIDE_MATERIALIZER_SENTINEL.as_slice(),
            "combat side panel materializer",
        ),
        (
            1,
            sites.modifier_append,
            PANEL_MODIFIER_APPEND_SENTINEL.as_slice(),
            "combat panel modifier append",
        ),
        (
            2,
            sites.leader_resolver,
            PANEL_LEADER_RESOLVER_SENTINEL.as_slice(),
            "combat side leader resolver",
        ),
    ] {
        if !PANEL_CREATED_BY_OWNER[index].load(Ordering::Acquire) {
            continue;
        }
        if !PANEL_REMOVED_BY_OWNER[index].load(Ordering::Acquire) {
            failures.push(format!(
                "{label} hook was not disabled before owner-performed rollback retention"
            ));
        }
        if PANEL_REMOVED_BY_OWNER[index].load(Ordering::Acquire) {
            if let Err(error) = verify_sentinel(target, sentinel, label) {
                failures.push(error);
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn retain_width_relays_at_teardown() {
    for relay in WIDTH_RELAYS {
        relay.retain_at_teardown();
    }
    MODULE_BASE.store(0, Ordering::Release);
}

fn retain_loss_relays_at_teardown() {
    for relay in LOSS_RELAYS {
        relay.retain_at_teardown();
    }
    LOSS_MODULE_BASE.store(0, Ordering::Release);
}

fn retain_panel_relays_at_teardown() {
    for relay in PANEL_RELAYS {
        relay.retain_at_teardown();
    }
    PANEL_MODULE_BASE.store(0, Ordering::Release);
}

pub fn width_engine_original_skipped_invocations() -> u64 {
    sum_engine_original_skipped(&WIDTH_RELAYS)
}

pub fn loss_engine_original_skipped_invocations() -> u64 {
    sum_engine_original_skipped(&LOSS_RELAYS)
}

pub fn panel_engine_original_skipped_invocations() -> u64 {
    sum_engine_original_skipped(&PANEL_RELAYS)
}

fn sum_engine_original_skipped(relays: &[&RelaySlot]) -> u64 {
    relays.iter().fold(0u64, |total, relay| {
        total.saturating_add(relay.engine_original_skipped())
    })
}

fn abort_width_install(sites: InstalledWidthHookAddresses) -> Result<(), String> {
    rollback_failed_width_install(sites)?;
    retain_width_relays_at_teardown();
    release_recorded_width_receipts(RETAINED_RELAY_TEARDOWN)?;
    archive_resolved_partial_receipts(
        &WIDTH_RECEIPT_CUSTODY,
        &WIDTH_ABORTED_RELEASE_HISTORY,
        "X2 width",
    )?;
    reset_width_owner_progress();
    Ok(())
}

fn abort_loss_install(sites: InstalledLossHookAddresses) -> Result<(), String> {
    rollback_failed_loss_install(sites)?;
    retain_loss_relays_at_teardown();
    release_recorded_loss_receipts(RETAINED_RELAY_TEARDOWN)?;
    archive_resolved_partial_receipts(
        &LOSS_RECEIPT_CUSTODY,
        &LOSS_ABORTED_RELEASE_HISTORY,
        "X2 loss",
    )?;
    reset_loss_owner_progress();
    Ok(())
}

fn abort_panel_install(sites: InstalledPanelHookAddresses) -> Result<(), String> {
    rollback_failed_panel_install(sites)?;
    retain_panel_relays_at_teardown();
    release_recorded_panel_receipts(RETAINED_RELAY_TEARDOWN)?;
    archive_resolved_partial_receipts(
        &PANEL_RECEIPT_CUSTODY,
        &PANEL_ABORTED_RELEASE_HISTORY,
        "X2 panel",
    )?;
    reset_panel_owner_progress();
    Ok(())
}

pub fn install_width_observer() -> Result<InstalledWidthHookAddresses, String> {
    let _combined_api = crate::terminal_coordinator::try_admit_individual_api("X2 width install")?;
    if !width_observer_requested() {
        return Err("X2 width carrier inactive: production promotion is false and evidence capture was not explicitly requested".into());
    }
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 width install mutex poisoned".to_string())?;
    if WIDTH_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 width install refused unresolved terminal teardown".into());
    }
    if WIDTH_INSTALLED.load(Ordering::Acquire) {
        let sites = width_addresses(MODULE_BASE.load(Ordering::Acquire));
        reobserve_installed_width_receipts(sites)?;
        return Ok(sites);
    }
    if MODULE_BASE.load(Ordering::Acquire) != 0
        || WIDTH_RELAYS.iter().any(|relay| relay.is_live())
        || WIDTH_CREATED_BY_OWNER
            .iter()
            .chain(WIDTH_REMOVED_BY_OWNER.iter())
            .any(|flag| flag.load(Ordering::Acquire))
    {
        let base = MODULE_BASE.load(Ordering::Acquire);
        if base == 0 {
            return Err("X2 width failed-install owner custody lost its module base".into());
        }
        abort_width_install(width_addresses(base))
            .map_err(|error| format!("X2 width prior failed-install cleanup failed: {error}"))?;
    }
    {
        let state = WIDTH_STATE
            .get_or_init(|| Mutex::new(WidthState::default()))
            .lock()
            .map_err(|_| "X2 width state mutex poisoned".to_string())?;
        if state.sealed || state.terminal.is_some() {
            return Err("X2 width install refused unresolved terminal evidence custody".into());
        }
    }
    require_empty_receipt_custody(&WIDTH_RECEIPT_CUSTODY, "X2 width")?;
    if WIDTH_CREATED_BY_OWNER
        .iter()
        .chain(WIDTH_REMOVED_BY_OWNER.iter())
        .any(|flag| flag.load(Ordering::Acquire))
    {
        return Err("X2 width install refused unresolved physical hook-owner custody".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let sites = width_addresses(base);
    verify_sentinel(sites.producer, &WIDTH_PRODUCER_SENTINEL, "width producer")?;
    verify_sentinel(
        sites.division_leaf,
        &DIVISION_WIDTH_LEAF_SENTINEL,
        "division width leaf",
    )?;
    let width_state = WIDTH_STATE.get_or_init(|| Mutex::new(WidthState::default()));
    {
        let mut state = width_state
            .lock()
            .map_err(|_| "X2 width state mutex poisoned".to_string())?;
        state.sealed = false;
        WIDTH_QUEUE_SEALING.store(false, Ordering::Release);
        WIDTH_QUEUE_SEALED.store(false, Ordering::Release);
    }

    create_or_adopt_hook(
        sites.division_leaf,
        hook_division_width_leaf as usize,
        &DIVISION_WIDTH_LEAF_RELAY,
    )?;
    WIDTH_CREATED_BY_OWNER[1].store(true, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    match create_or_adopt_hook(
        sites.producer,
        hook_width_producer as usize,
        &WIDTH_PRODUCER_RELAY,
    ) {
        Ok(_) => {
            WIDTH_CREATED_BY_OWNER[0].store(true, Ordering::Release);
        }
        Err(error) => {
            let cleanup = rollback_failed_width_install(sites);
            if cleanup.is_ok() {
                retain_width_relays_at_teardown();
            }
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; rollback failed: {cleanup}"),
            });
        }
    };
    let enabled = unsafe { inline_detour::enable_batch(&width_hook_batch(sites)) };
    if let Some((name, status)) = enabled.refusal() {
        let cleanup = rollback_failed_width_install(sites);
        if cleanup.is_ok() {
            retain_width_relays_at_teardown();
        }
        return Err(match cleanup {
            Ok(()) => format!("width hook enable failed for {name}: {status}"),
            Err(cleanup) => {
                format!("width hook enable failed for {name}: {status}; rollback failed: {cleanup}")
            }
        });
    }
    if let Err(error) = observe_width_install_receipts(sites) {
        let cleanup = abort_width_install(sites);
        return Err(match cleanup {
            Ok(()) => format!("width hook install receipt failed: {error}"),
            Err(cleanup) => {
                format!("width hook install receipt failed: {error}; abort failed: {cleanup}")
            }
        });
    }
    WIDTH_INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

pub fn width_observer_installed() -> bool {
    WIDTH_INSTALLED.load(Ordering::Acquire)
}

pub fn install_current_loss_observer() -> Result<InstalledLossHookAddresses, String> {
    let _combined_api = crate::terminal_coordinator::try_admit_individual_api("X2 loss install")?;
    if !current_loss_observer_requested() {
        return Err(
            "X2 loss carrier inactive: production promotion is false and no nonce-bound evidence capture was requested".into(),
        );
    }
    let contract = AUTHENTICATED_LOSS_WRITER.ok_or_else(|| {
        "X2 loss carrier has no authenticated writer RVA/sentinel contract".to_string()
    })?;
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 loss install mutex poisoned".to_string())?;
    if LOSS_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 loss install refused unresolved terminal teardown".into());
    }
    if LOSS_INSTALLED.load(Ordering::Acquire) {
        let sites = loss_addresses(LOSS_MODULE_BASE.load(Ordering::Acquire), contract);
        reobserve_installed_loss_receipts(sites)?;
        return Ok(sites);
    }
    if LOSS_MODULE_BASE.load(Ordering::Acquire) != 0
        || LOSS_RELAYS.iter().any(|relay| relay.is_live())
        || LOSS_CREATED_BY_OWNER
            .iter()
            .chain(LOSS_REMOVED_BY_OWNER.iter())
            .any(|flag| flag.load(Ordering::Acquire))
    {
        let base = LOSS_MODULE_BASE.load(Ordering::Acquire);
        if base == 0 {
            return Err("X2 loss failed-install owner custody lost its module base".into());
        }
        abort_loss_install(loss_addresses(base, contract))
            .map_err(|error| format!("X2 loss prior failed-install cleanup failed: {error}"))?;
    }
    {
        let state = LOSS_STATE
            .get_or_init(|| Mutex::new(LossState::default()))
            .lock()
            .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
        if state.sealed || state.terminal.is_some() {
            return Err("X2 loss install refused unresolved terminal evidence custody".into());
        }
    }
    require_empty_receipt_custody(&LOSS_RECEIPT_CUSTODY, "X2 loss")?;
    if LOSS_CREATED_BY_OWNER
        .iter()
        .chain(LOSS_REMOVED_BY_OWNER.iter())
        .any(|flag| flag.load(Ordering::Acquire))
    {
        return Err("X2 loss install refused unresolved physical hook-owner custody".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let sites = loss_addresses(base, contract);
    verify_sentinel(
        sites.active_damage,
        &contract.active_damage.sentinel,
        "active division loss writer",
    )?;
    verify_sentinel(
        sites.participant_lifecycle,
        &contract.participant_lifecycle.sentinel,
        "participant-lifecycle loss writer",
    )?;
    let loss_state = LOSS_STATE.get_or_init(|| Mutex::new(LossState::default()));
    {
        let mut state = loss_state
            .lock()
            .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
        state.sealed = false;
        LOSS_QUEUE_SEALING.store(false, Ordering::Release);
        LOSS_QUEUE_SEALED.store(false, Ordering::Release);
    }

    create_or_adopt_hook(
        sites.participant_lifecycle,
        hook_participant_lifecycle as usize,
        &PARTICIPANT_LIFECYCLE_RELAY,
    )?;
    LOSS_CREATED_BY_OWNER[1].store(true, Ordering::Release);
    LOSS_MODULE_BASE.store(base, Ordering::Release);
    match create_or_adopt_hook(
        sites.active_damage,
        hook_active_division_damage as usize,
        &ACTIVE_DIVISION_DAMAGE_RELAY,
    ) {
        Ok(_) => {
            LOSS_CREATED_BY_OWNER[0].store(true, Ordering::Release);
        }
        Err(error) => {
            let cleanup = rollback_failed_loss_install(sites);
            if cleanup.is_ok() {
                retain_loss_relays_at_teardown();
            }
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; rollback failed: {cleanup}"),
            });
        }
    };
    let enabled = unsafe { inline_detour::enable_batch(&loss_hook_batch(sites)) };
    if let Some((name, status)) = enabled.refusal() {
        let cleanup = rollback_failed_loss_install(sites);
        if cleanup.is_ok() {
            retain_loss_relays_at_teardown();
        }
        return Err(match cleanup {
            Ok(()) => format!("loss hook enable failed for {name}: {status}"),
            Err(cleanup) => {
                format!("loss hook enable failed for {name}: {status}; rollback failed: {cleanup}")
            }
        });
    }
    if let Err(error) = observe_loss_install_receipts(sites) {
        let cleanup = abort_loss_install(sites);
        return Err(match cleanup {
            Ok(()) => format!("loss hook install receipt failed: {error}"),
            Err(cleanup) => {
                format!("loss hook install receipt failed: {error}; abort failed: {cleanup}")
            }
        });
    }
    LOSS_INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

pub fn current_loss_observer_installed() -> bool {
    LOSS_INSTALLED.load(Ordering::Acquire)
}

pub fn install_panel_observer() -> Result<InstalledPanelHookAddresses, String> {
    let _combined_api = crate::terminal_coordinator::try_admit_individual_api("X2 panel install")?;
    if !PANEL_CARRIER_ACTIVATION {
        return Err("X2 panel carrier inactive".into());
    }
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 panel install mutex poisoned".to_string())?;
    if PANEL_INSTALLED.load(Ordering::Acquire) {
        let sites = panel_addresses(PANEL_MODULE_BASE.load(Ordering::Acquire));
        reobserve_installed_panel_receipts(sites)?;
        return Ok(sites);
    }
    if PANEL_QUEUE_SEALED.load(Ordering::Acquire) || PANEL_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 panel install refused after queue teardown began".into());
    }
    if PANEL_MODULE_BASE.load(Ordering::Acquire) != 0
        || PANEL_RELAYS.iter().any(|relay| relay.is_live())
    {
        let sites = panel_addresses(PANEL_MODULE_BASE.load(Ordering::Acquire));
        abort_panel_install(sites).map_err(|error| {
            format!("X2 panel install refused incomplete prior hook rollback: {error}")
        })?;
    }
    require_empty_receipt_custody(&PANEL_RECEIPT_CUSTODY, "X2 panel")?;
    if (0..3).any(|index| {
        PANEL_CREATED_BY_OWNER[index].load(Ordering::Acquire)
            || PANEL_REMOVED_BY_OWNER[index].load(Ordering::Acquire)
    }) {
        return Err("X2 panel install refused stale removal-progress custody".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let sites = panel_addresses(base);
    verify_sentinel(
        sites.side_materializer,
        &PANEL_SIDE_MATERIALIZER_SENTINEL,
        "combat side panel materializer",
    )?;
    verify_sentinel(
        sites.modifier_append,
        &PANEL_MODIFIER_APPEND_SENTINEL,
        "combat panel modifier append",
    )?;
    verify_sentinel(
        sites.leader_resolver,
        &PANEL_LEADER_RESOLVER_SENTINEL,
        "combat side leader resolver",
    )?;
    PANEL_STATE.get_or_init(|| Mutex::new(PanelState::default()));
    PANEL_MODULE_BASE.store(base, Ordering::Release);

    match create_or_adopt_hook(
        sites.leader_resolver,
        hook_panel_leader_resolver as usize,
        &PANEL_LEADER_RESOLVER_RELAY,
    ) {
        Ok(_) => {}
        Err(error) => {
            retain_panel_relays_at_teardown();
            return Err(error);
        }
    };
    PANEL_CREATED_BY_OWNER[2].store(true, Ordering::Release);
    match create_or_adopt_hook(
        sites.modifier_append,
        hook_panel_modifier_append as usize,
        &PANEL_MODIFIER_APPEND_RELAY,
    ) {
        Ok(_) => {}
        Err(error) => {
            let cleanup = abort_panel_install(sites);
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; rollback failed: {cleanup}"),
            });
        }
    };
    PANEL_CREATED_BY_OWNER[1].store(true, Ordering::Release);
    match create_or_adopt_hook(
        sites.side_materializer,
        hook_panel_side_materializer as usize,
        &PANEL_SIDE_MATERIALIZER_RELAY,
    ) {
        Ok(_) => {}
        Err(error) => {
            let cleanup = abort_panel_install(sites);
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; rollback failed: {cleanup}"),
            });
        }
    };
    PANEL_CREATED_BY_OWNER[0].store(true, Ordering::Release);
    let enabled = unsafe { inline_detour::enable_batch(&panel_hook_batch(sites)) };
    if let Some((name, status)) = enabled.refusal() {
        let cleanup = abort_panel_install(sites);
        return Err(match cleanup {
            Ok(()) => format!("combat panel hook enable failed for {name}: {status}"),
            Err(cleanup) => format!(
                "combat panel hook enable failed for {name}: {status}; rollback failed: {cleanup}"
            ),
        });
    }
    if let Err(error) = observe_panel_install_receipts(sites) {
        let cleanup = abort_panel_install(sites);
        return Err(match cleanup {
            Ok(()) => format!("panel hook install receipt failed: {error}"),
            Err(cleanup) => {
                format!("panel hook install receipt failed: {error}; abort failed: {cleanup}")
            }
        });
    }
    PANEL_INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

pub fn panel_observer_installed() -> bool {
    PANEL_INSTALLED.load(Ordering::Acquire)
}

pub fn uninstall_width_observer() -> Result<(), String> {
    Err(
        "X2 width uninstall requires terminal reserve/commit custody; implicit evidence commit is forbidden"
            .into(),
    )
}

pub fn seal_and_reserve_terminal_width_observer() -> Result<WidthTerminalReservation, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 width terminal seal")?;
    seal_and_reserve_terminal_width_observer_inner()
}

fn seal_and_reserve_terminal_width_observer_inner() -> Result<WidthTerminalReservation, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 width uninstall mutex poisoned".to_string())?;
    if !WIDTH_INSTALLED.load(Ordering::Acquire) {
        if !WIDTH_QUEUE_SEALED.load(Ordering::Acquire) {
            return Err(
                "X2 width observer is not installed and has no sealed terminal custody".into(),
            );
        }
        return reserve_width_terminal_locked();
    }
    let sites = width_addresses(MODULE_BASE.load(Ordering::Acquire));
    require_complete_release_custody(&WIDTH_RECEIPT_CUSTODY, "X2 width")?;
    begin_width_queue_teardown_seal()?;
    disable_width_hooks_for_release(sites)?;
    let release = (|| {
        for (index, target, label) in [
            (0, sites.producer, "width producer"),
            (1, sites.division_leaf, "division width leaf"),
        ] {
            if !WIDTH_CREATED_BY_OWNER[index].load(Ordering::Acquire) {
                return Err(format!(
                    "{label} hook lost created-by-owner custody before teardown"
                ));
            }
            // `disable_width_hooks_for_release` above performed the removal, as
            if !WIDTH_REMOVED_BY_OWNER[index].load(Ordering::Acquire) {
                return Err(format!(
                    "{label} hook at 0x{target:X} reached its retained release without the \
                     owner-performed disable"
                ));
            }
        }
        verify_sentinel(sites.producer, &WIDTH_PRODUCER_SENTINEL, "width producer")?;
        verify_sentinel(
            sites.division_leaf,
            &DIVISION_WIDTH_LEAF_SENTINEL,
            "division width leaf",
        )?;
        let teardown = RETAINED_RELAY_TEARDOWN;
        release_complete_width_receipt_set(teardown)?;
        // Publish both exact release receipts and the pointer-free terminal
        publish_release_and_seal_width()
    })();
    release?;
    WIDTH_INSTALLED.store(false, Ordering::Release);
    retain_width_relays_at_teardown();
    WIDTH_CONTEXT.with(|slot| *slot.borrow_mut() = None);
    reserve_width_terminal_locked()
}

pub fn seal_drain_and_uninstall_width_observer() -> Result<WidthTerminalReservation, String> {
    seal_and_reserve_terminal_width_observer()
}

pub fn uninstall_current_loss_observer() -> Result<(), String> {
    Err(
        "X2 loss uninstall requires terminal reserve/commit custody; implicit evidence commit is forbidden"
            .into(),
    )
}

pub fn uninstall_panel_observer() -> Result<(), String> {
    Err(
        "X2 panel uninstall requires terminal reserve/commit custody; implicit evidence commit is forbidden"
            .into(),
    )
}

/// tail. Rows, diagnostics, and released-hook receipts remain lane-owned until
pub fn seal_and_reserve_terminal_panel_observer() -> Result<PanelTerminalReservation, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 panel terminal seal")?;
    seal_and_reserve_terminal_panel_observer_inner()
}

fn seal_and_reserve_terminal_panel_observer_inner() -> Result<PanelTerminalReservation, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 panel uninstall mutex poisoned".to_string())?;
    if !PANEL_INSTALLED.load(Ordering::Acquire) {
        if !PANEL_QUEUE_SEALED.load(Ordering::Acquire) {
            return Err(
                "X2 panel observer is not installed and has no sealed terminal custody".into(),
            );
        }
        return reserve_panel_terminal_locked();
    }
    let sites = panel_addresses(PANEL_MODULE_BASE.load(Ordering::Acquire));
    require_complete_release_custody(&PANEL_RECEIPT_CUSTODY, "X2 panel")?;
    begin_panel_queue_teardown_seal()?;
    disable_panel_hooks_for_release(sites)?;
    attest_and_release_retained_panel_hooks(sites)?;
    publish_release_and_seal_panel()?;
    PANEL_INSTALLED.store(false, Ordering::Release);
    retain_panel_relays_at_teardown();
    PANEL_CONTEXT.with(|slot| *slot.borrow_mut() = None);
    reserve_panel_terminal_locked()
}

pub fn seal_drain_and_uninstall_panel_observer() -> Result<PanelTerminalReservation, String> {
    seal_and_reserve_terminal_panel_observer()
}

pub fn seal_and_reserve_terminal_current_loss_observer() -> Result<LossTerminalReservation, String>
{
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 loss terminal seal")?;
    seal_and_reserve_terminal_current_loss_observer_inner()
}

fn seal_and_reserve_terminal_current_loss_observer_inner() -> Result<LossTerminalReservation, String>
{
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 loss uninstall mutex poisoned".to_string())?;
    if !LOSS_INSTALLED.load(Ordering::Acquire) {
        if !LOSS_QUEUE_SEALED.load(Ordering::Acquire) {
            return Err(
                "X2 loss observer is not installed and has no sealed terminal custody".into(),
            );
        }
        return reserve_loss_terminal_locked();
    }
    let contract = AUTHENTICATED_LOSS_WRITER
        .ok_or_else(|| "installed X2 loss hook lost its authenticated contract".to_string())?;
    let sites = loss_addresses(LOSS_MODULE_BASE.load(Ordering::Acquire), contract);
    require_complete_release_custody(&LOSS_RECEIPT_CUSTODY, "X2 loss")?;
    begin_loss_queue_teardown_seal()?;
    disable_loss_hooks_for_release(sites)?;
    let release = (|| {
        for (index, target, label) in [
            (0, sites.active_damage, "active division loss writer"),
            (
                1,
                sites.participant_lifecycle,
                "participant-lifecycle loss writer",
            ),
        ] {
            if !LOSS_CREATED_BY_OWNER[index].load(Ordering::Acquire) {
                return Err(format!(
                    "{label} hook lost created-by-owner custody before teardown"
                ));
            }
            // `disable_loss_hooks_for_release` above performed the removal, as a
            if !LOSS_REMOVED_BY_OWNER[index].load(Ordering::Acquire) {
                return Err(format!(
                    "{label} hook at 0x{target:X} reached its retained release without the \
                     owner-performed disable"
                ));
            }
        }
        // Do not partially consume shared release custody when either writer
        verify_sentinel(
            sites.active_damage,
            &ACTIVE_DAMAGE_SENTINEL,
            "active division loss writer",
        )?;
        verify_sentinel(
            sites.participant_lifecycle,
            &PARTICIPANT_LIFECYCLE_SENTINEL,
            "participant-lifecycle loss writer",
        )?;
        let teardown = RETAINED_RELAY_TEARDOWN;
        release_complete_loss_receipt_set(teardown)?;
        // Only a complete published release may seal the immutable pointer-free
        publish_release_and_seal_loss()
    })();
    release?;
    LOSS_INSTALLED.store(false, Ordering::Release);
    retain_loss_relays_at_teardown();
    LOSS_WRITER_ACTIVE.with(|active| active.set(false));
    reserve_loss_terminal_locked()
}

pub fn seal_drain_and_uninstall_current_loss_observer() -> Result<LossTerminalReservation, String> {
    seal_and_reserve_terminal_current_loss_observer()
}

pub fn publish_side_bindings(bindings: &[SideBinding]) -> Result<(), String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 width binding publication")?;
    if width_queue_admission_closed() {
        return Err("X2 width bindings are frozen for terminal teardown".into());
    }
    let mut next = BTreeMap::new();
    for binding in bindings {
        if binding.side_ptr == 0
            || binding.combat_id == 0
            || binding.generation == 0
            || next.insert(binding.side_ptr, binding.clone()).is_some()
        {
            return Err("invalid or duplicate X2 width side binding".into());
        }
    }
    let state = WIDTH_STATE.get_or_init(|| Mutex::new(WidthState::default()));
    let mut state = state
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?;
    if state.sealed || WIDTH_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 width bindings are frozen for terminal teardown".into());
    }
    state.bindings = next;
    Ok(())
}

pub(crate) fn publish_combined_final_bindings(
    width: &[SideBinding],
    current_loss: &[LossSideBinding],
    panel: &[SideBinding],
) -> Result<CombinedProjectedTerminal, String> {
    if !crate::terminal_coordinator::individual_terminal_api_blocked() {
        return Err("X2 combined final bindings require active combined custody".into());
    }
    let mut next_width = BTreeMap::new();
    for binding in width {
        if binding.side_ptr == 0
            || binding.combat_id == 0
            || binding.generation == 0
            || next_width
                .insert(binding.side_ptr, binding.clone())
                .is_some()
        {
            return Err("invalid or duplicate combined X2 width binding".into());
        }
    }
    let mut next_loss = BTreeMap::new();
    for binding in current_loss {
        let owner_ptr = binding
            .side_ptr
            .checked_add(LOSS_OWNER_OFFSET_FROM_SIDE)
            .ok_or_else(|| "combined X2 loss side-owner address overflow".to_string())?;
        if binding.combat_ptr == 0
            || binding.side_ptr == 0
            || binding.combat_id == 0
            || binding.generation == 0
            || next_loss.insert(owner_ptr, binding.clone()).is_some()
        {
            return Err("invalid or duplicate combined X2 loss binding".into());
        }
    }
    let mut next_panel = BTreeMap::new();
    for binding in panel {
        if binding.side_ptr == 0
            || binding.combat_id == 0
            || binding.generation == 0
            || next_panel
                .insert(binding.side_ptr, binding.clone())
                .is_some()
        {
            return Err("invalid or duplicate combined X2 panel binding".into());
        }
    }

    let mut width_state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "combined X2 width state mutex poisoned".to_string())?;
    let mut loss_state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "combined X2 loss state mutex poisoned".to_string())?;
    let mut panel_state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "combined X2 panel state mutex poisoned".to_string())?;
    if width_state.sealed
        || loss_state.sealed
        || panel_state.sealed
        || width_state.reservation.is_some()
        || loss_state.reservation.is_some()
        || panel_state.reservation.is_some()
    {
        return Err("combined X2 final bindings overlap sealed or reserved custody".into());
    }
    let width_diagnostics = snapshot_width_diagnostics();
    if width_diagnostics != WidthDiagnostics::default() {
        return Err(format!(
            "combined X2 width projection is not lossless: diagnostics={width_diagnostics:?}"
        ));
    }
    let mut width_rows = Vec::new();
    for capture in &width_state.calls {
        let mut rows = project_width_call(capture)
            .map_err(|error| {
                format!(
                    "combined X2 width capture {} failed exact projection: {error}",
                    capture.capture_sequence
                )
            })?;
        width_rows.append(&mut rows);
    }
    let loss_diagnostics = snapshot_loss_diagnostics();
    if loss_diagnostics != LossDiagnostics::default() {
        return Err(format!(
            "combined X2 loss projection is not lossless: diagnostics={loss_diagnostics:?}"
        ));
    }
    let mut loss_rows = Vec::new();
    for observation in &loss_state.observations {
        let mut rows = project_loss_observation(observation, observation).map_err(|error| {
            format!(
                "combined X2 loss capture {} failed exact projection: {error}",
                observation.capture_sequence
            )
        })?;
        loss_rows.append(&mut rows);
    }
    let projected = CombinedProjectedTerminal {
        width: WidthTerminalDrain {
            rows: width_rows,
            projection_failures: 0,
            completeness_poisoned: false,
            ..WidthTerminalDrain::default()
        },
        current_loss: LossTerminalDrain {
            rows: loss_rows,
            projection_failures: 0,
            completeness_poisoned: false,
            ..LossTerminalDrain::default()
        },
    };
    width_state.bindings = next_width;
    loss_state.bindings = next_loss;
    panel_state.bindings = next_panel;
    width_state.combined_projection = Some(projected.width.clone());
    loss_state.combined_projection = Some(projected.current_loss.clone());
    Ok(projected)
}

pub(crate) fn seal_and_reserve_width_for_combined() -> Result<WidthTerminalReservation, String> {
    seal_and_reserve_terminal_width_observer_inner()
}

pub(crate) fn seal_and_reserve_loss_for_combined() -> Result<LossTerminalReservation, String> {
    seal_and_reserve_terminal_current_loss_observer_inner()
}

pub(crate) fn seal_and_reserve_panel_for_combined() -> Result<PanelTerminalReservation, String> {
    seal_and_reserve_terminal_panel_observer_inner()
}

fn begin_width_queue_teardown_seal() -> Result<(), String> {
    let retrying = WIDTH_QUEUE_SEALING.swap(true, Ordering::AcqRel);
    let state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| {
            if !retrying {
                WIDTH_QUEUE_SEALING.store(false, Ordering::Release);
            }
            "X2 width state mutex poisoned".to_string()
        })?;
    if state.sealed {
        if !retrying {
            WIDTH_QUEUE_SEALING.store(false, Ordering::Release);
        }
        return Err("X2 width queue was already sealed before teardown".into());
    }
    if let Some(reservation) = state.reservation {
        if !retrying {
            WIDTH_QUEUE_SEALING.store(false, Ordering::Release);
        }
        return Err(format!(
            "X2 width teardown blocked by reservation {}: reserved={}, pending_unreserved={}",
            reservation.reservation_id,
            reservation.row_count,
            state.calls.len().saturating_sub(reservation.row_count)
        ));
    }
    Ok(())
}

fn begin_loss_queue_teardown_seal() -> Result<(), String> {
    let retrying = LOSS_QUEUE_SEALING.swap(true, Ordering::AcqRel);
    let state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| {
            if !retrying {
                LOSS_QUEUE_SEALING.store(false, Ordering::Release);
            }
            "X2 loss state mutex poisoned".to_string()
        })?;
    if state.sealed {
        if !retrying {
            LOSS_QUEUE_SEALING.store(false, Ordering::Release);
        }
        return Err("X2 loss queue was already sealed before teardown".into());
    }
    if let Some(reservation) = state.reservation {
        if !retrying {
            LOSS_QUEUE_SEALING.store(false, Ordering::Release);
        }
        return Err(format!(
            "X2 loss teardown blocked by reservation {}: reserved={}, pending_unreserved={}",
            reservation.reservation_id,
            reservation.row_count,
            state
                .observations
                .len()
                .saturating_sub(reservation.row_count)
        ));
    }
    Ok(())
}

fn begin_panel_queue_teardown_seal() -> Result<(), String> {
    PANEL_QUEUE_SEALING.store(true, Ordering::Release);
    let state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?;
    if state.sealed {
        return Err("X2 panel queue was already sealed before teardown".into());
    }
    if let Some(reservation) = state.reservation {
        PANEL_QUEUE_SEALING.store(false, Ordering::Release);
        return Err(format!(
            "X2 panel teardown blocked by reservation {}: reserved={}, pending_unreserved={}",
            reservation.reservation_id,
            reservation.row_count,
            state
                .observations
                .len()
                .saturating_sub(reservation.row_count)
        ));
    }
    Ok(())
}

fn publish_release_and_seal_width() -> Result<(), String> {
    let mut state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?;
    if state.sealed {
        return Err("X2 width queue was already sealed before final drain".into());
    }
    if state.reservation.is_some() || state.terminal.is_some() {
        return Err("X2 width final seal cannot replace existing evidence custody".into());
    }
    let mut custody = WIDTH_RECEIPT_CUSTODY
        .lock()
        .map_err(|_| "X2 width receipt custody mutex poisoned".to_string())?;
    if custody.installed.iter().any(Option::is_some) || custody.released.iter().any(Option::is_none)
    {
        return Err("X2 width cannot publish an incomplete released roster".into());
    }
    let released_hooks = custody
        .released
        .map(|receipt| receipt.expect("complete X2 width released receipt roster"));
    let diagnostics = snapshot_width_diagnostics();
    let drain = if crate::terminal_coordinator::individual_terminal_api_blocked() {
        if diagnostics != WidthDiagnostics::default() {
            return Err("combined X2 width diagnostics changed after exact preprojection".into());
        }
        state
            .combined_projection
            .take()
            .ok_or_else(|| "combined X2 width exact preprojection is missing".to_string())?
    } else {
        let mut rows = Vec::new();
        let mut projection_failures = 0u64;
        for capture in &state.calls {
            let projected = project_width_call(capture);
            match projected {
                Ok(mut projected) => rows.append(&mut projected),
                Err(_) => projection_failures += 1,
            }
        }
        let completeness_poisoned =
            diagnostics != WidthDiagnostics::default() || projection_failures != 0;
        WidthTerminalDrain {
            rows,
            dropped_lock_busy: diagnostics.dropped_lock_busy,
            dropped_unbound: diagnostics.dropped_unbound,
            dropped_unreadable: diagnostics.dropped_unreadable,
            dropped_overflow: diagnostics.dropped_overflow,
            dropped_nested: diagnostics.dropped_nested,
            projection_failures,
            completeness_poisoned,
        }
    };

    custody.installed_published = false;
    custody.release_published = true;
    state.sealed = true;
    state.terminal = Some(WidthTerminalQueueState {
        active_reservation_id: None,
        row_count: state.calls.len(),
        diagnostics,
        drain,
        released_hooks,
    });
    WIDTH_QUEUE_SEALED.store(true, Ordering::Release);
    Ok(())
}

fn publish_release_and_seal_loss() -> Result<(), String> {
    let mut state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
    if state.sealed {
        return Err("X2 loss queue was already sealed before final drain".into());
    }
    if state.reservation.is_some() || state.terminal.is_some() {
        return Err("X2 loss final seal cannot replace existing evidence custody".into());
    }
    let mut custody = LOSS_RECEIPT_CUSTODY
        .lock()
        .map_err(|_| "X2 loss receipt custody mutex poisoned".to_string())?;
    if custody.installed.iter().any(Option::is_some) || custody.released.iter().any(Option::is_none)
    {
        return Err("X2 loss cannot publish an incomplete released roster".into());
    }
    let released_hooks = custody
        .released
        .map(|receipt| receipt.expect("complete X2 loss released receipt roster"));
    let diagnostics = snapshot_loss_diagnostics();
    let drain = if crate::terminal_coordinator::individual_terminal_api_blocked() {
        if diagnostics != LossDiagnostics::default() {
            return Err("combined X2 loss diagnostics changed after exact preprojection".into());
        }
        state
            .combined_projection
            .take()
            .ok_or_else(|| "combined X2 loss exact preprojection is missing".to_string())?
    } else {
        let mut rows = Vec::new();
        let mut projection_failures = 0u64;
        for observation in &state.observations {
            match project_loss_observation(observation, observation) {
                Ok(mut projected) => rows.append(&mut projected),
                Err(_) => projection_failures += 1,
            }
        }
        let completeness_poisoned =
            diagnostics != LossDiagnostics::default() || projection_failures != 0;
        LossTerminalDrain {
            rows,
            dropped_lock_busy: diagnostics.dropped_lock_busy,
            dropped_unbound: diagnostics.dropped_unbound,
            dropped_unreadable: diagnostics.dropped_unreadable,
            dropped_invalid: diagnostics.dropped_invalid,
            dropped_overflow: diagnostics.dropped_overflow,
            dropped_nested: diagnostics.dropped_nested,
            projection_failures,
            completeness_poisoned,
        }
    };

    custody.installed_published = false;
    custody.release_published = true;
    state.sealed = true;
    state.terminal = Some(LossTerminalQueueState {
        active_reservation_id: None,
        row_count: state.observations.len(),
        diagnostics,
        drain,
        released_hooks,
    });
    LOSS_QUEUE_SEALED.store(true, Ordering::Release);
    Ok(())
}

fn reserve_width_terminal_locked() -> Result<WidthTerminalReservation, String> {
    let mut state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?;
    if !state.sealed || !WIDTH_QUEUE_SEALED.load(Ordering::Acquire) {
        return Err("X2 width terminal custody is not sealed".into());
    }
    let terminal = state
        .terminal
        .as_mut()
        .ok_or_else(|| "X2 width sealed queue has no terminal custody".to_string())?;
    if let Some(active) = terminal.active_reservation_id {
        return Err(format!(
            "X2 width terminal custody already has reservation {active}"
        ));
    }
    let reservation_id = next_queue_reservation_id(&NEXT_WIDTH_RESERVATION_ID, "width terminal")?;
    terminal.active_reservation_id = Some(reservation_id);
    Ok(WidthTerminalReservation {
        reservation_id,
        drain: terminal.drain.clone(),
        released_hooks: terminal.released_hooks,
    })
}

/// released-hook receipts have been acknowledged by the enclosing sink.
pub fn commit_terminal_width_observer(reservation_id: u64) -> Result<(), String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 width terminal commit")?;
    consume_terminal_width_observer_inner(reservation_id, "commit").map(|_| ())
}

/// finish, and resolves its release custody.
/// The seal above already removed both detours and published their release
/// teardown, diagnostic and release-custody equality gates still run.
pub fn abort_terminal_width_observer(reservation_id: u64) -> Result<InstallAbortDiscard, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 width terminal install abort")?;
    consume_terminal_width_observer_inner(reservation_id, "install abort")
}

/// Seals, releases and discards the whole width carrier for a production hook
/// registry that could not complete its install. The exact release receipts are
pub fn abort_partial_install_and_uninstall_width_observer(
) -> Result<(InstallAbortDiscard, [ReleasedRvaHookReceipt; 2]), String> {
    let reservation = seal_and_reserve_terminal_width_observer()?;
    let released = reservation.released_hooks;
    let discard = abort_terminal_width_observer(reservation.reservation_id)?;
    Ok((discard, released))
}

fn consume_terminal_width_observer_inner(
    reservation_id: u64,
    label: &str,
) -> Result<InstallAbortDiscard, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| format!("X2 width terminal {label} mutex poisoned"))?;
    if WIDTH_INSTALLED.load(Ordering::Acquire)
        || MODULE_BASE.load(Ordering::Acquire) != 0
        || !WIDTH_QUEUE_SEALED.load(Ordering::Acquire)
    {
        return Err(format!(
            "X2 width terminal {label} requires completed physical teardown"
        ));
    }
    let mut state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?;
    let terminal = state
        .terminal
        .as_ref()
        .ok_or_else(|| format!("X2 width has no terminal custody to {label}"))?;
    if terminal.active_reservation_id != Some(reservation_id) {
        return Err(format!(
            "X2 width terminal reservation mismatch: active={:?}, supplied={reservation_id}",
            terminal.active_reservation_id
        ));
    }
    if !state.sealed
        || state.reservation.is_some()
        || state.calls.len() != terminal.row_count
        || snapshot_width_diagnostics() != terminal.diagnostics
    {
        return Err(format!(
            "X2 width terminal rows or diagnostics changed before {label}"
        ));
    }
    let discard = InstallAbortDiscard {
        retained_rows: terminal.row_count as u64,
        reserved_rows: terminal.drain.rows.len() as u64,
        discarded_diagnostics: terminal
            .drain
            .dropped_lock_busy
            .saturating_add(terminal.drain.dropped_unbound)
            .saturating_add(terminal.drain.dropped_unreadable)
            .saturating_add(terminal.drain.dropped_overflow)
            .saturating_add(terminal.drain.dropped_nested)
            .saturating_add(terminal.drain.projection_failures),
        poisoned: terminal.drain.completeness_poisoned
            || width_engine_original_skipped_invocations() != 0,
    };
    let expected_diagnostics = terminal.diagnostics;
    let expected_releases = terminal.released_hooks;
    let mut custody = WIDTH_RECEIPT_CUSTODY
        .lock()
        .map_err(|_| "X2 width receipt custody mutex poisoned".to_string())?;
    let observed_releases = custody
        .released
        .map(|receipt| receipt.expect("complete X2 width released receipt roster"));
    if custody.installed_published
        || !custody.release_published
        || custody.installed.iter().any(Option::is_some)
        || observed_releases != expected_releases
    {
        return Err(format!(
            "X2 width released-hook custody changed before terminal {label}"
        ));
    }

    commit_width_diagnostics(expected_diagnostics)?;
    *state = WidthState::default();
    *custody = HookReceiptCustody::new();
    reset_width_owner_progress();
    WIDTH_QUEUE_SEALING.store(false, Ordering::Release);
    WIDTH_QUEUE_SEALED.store(false, Ordering::Release);
    Ok(discard)
}

/// Release terminal ownership after a failed publication. Hooks stay detached,
/// while the exact same rows, diagnostics, and release receipts remain sealed
pub fn rollback_terminal_width_observer(reservation_id: u64) -> Result<(), String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 width terminal rollback")?;
    rollback_terminal_width_observer_inner(reservation_id)
}

fn rollback_terminal_width_observer_inner(reservation_id: u64) -> Result<(), String> {
    let mut state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?;
    let terminal = state
        .terminal
        .as_mut()
        .ok_or_else(|| "X2 width has no terminal custody to roll back".to_string())?;
    if terminal.active_reservation_id != Some(reservation_id) {
        return Err(format!(
            "X2 width terminal reservation mismatch: active={:?}, supplied={reservation_id}",
            terminal.active_reservation_id
        ));
    }
    terminal.active_reservation_id = None;
    Ok(())
}

fn reserve_loss_terminal_locked() -> Result<LossTerminalReservation, String> {
    let mut state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
    if !state.sealed || !LOSS_QUEUE_SEALED.load(Ordering::Acquire) {
        return Err("X2 loss terminal custody is not sealed".into());
    }
    let terminal = state
        .terminal
        .as_mut()
        .ok_or_else(|| "X2 loss sealed queue has no terminal custody".to_string())?;
    if let Some(active) = terminal.active_reservation_id {
        return Err(format!(
            "X2 loss terminal custody already has reservation {active}"
        ));
    }
    let reservation_id = next_queue_reservation_id(&NEXT_LOSS_RESERVATION_ID, "loss terminal")?;
    terminal.active_reservation_id = Some(reservation_id);
    Ok(LossTerminalReservation {
        reservation_id,
        drain: terminal.drain.clone(),
        released_hooks: terminal.released_hooks,
    })
}

pub fn commit_terminal_current_loss_observer(reservation_id: u64) -> Result<(), String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 loss terminal commit")?;
    consume_terminal_current_loss_observer_inner(reservation_id, "commit").map(|_| ())
}

/// finish, and resolves its release custody. See
pub fn abort_terminal_current_loss_observer(
    reservation_id: u64,
) -> Result<InstallAbortDiscard, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 loss terminal install abort")?;
    consume_terminal_current_loss_observer_inner(reservation_id, "install abort")
}

/// Seals, releases and discards the whole current-loss carrier for a production
pub fn abort_partial_install_and_uninstall_current_loss_observer(
) -> Result<(InstallAbortDiscard, [ReleasedRvaHookReceipt; 2]), String> {
    let reservation = seal_and_reserve_terminal_current_loss_observer()?;
    let released = reservation.released_hooks;
    let discard = abort_terminal_current_loss_observer(reservation.reservation_id)?;
    Ok((discard, released))
}

fn consume_terminal_current_loss_observer_inner(
    reservation_id: u64,
    label: &str,
) -> Result<InstallAbortDiscard, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| format!("X2 loss terminal {label} mutex poisoned"))?;
    if LOSS_INSTALLED.load(Ordering::Acquire)
        || LOSS_MODULE_BASE.load(Ordering::Acquire) != 0
        || !LOSS_QUEUE_SEALED.load(Ordering::Acquire)
    {
        return Err(format!(
            "X2 loss terminal {label} requires completed physical teardown"
        ));
    }
    let mut state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
    let terminal = state
        .terminal
        .as_ref()
        .ok_or_else(|| format!("X2 loss has no terminal custody to {label}"))?;
    if terminal.active_reservation_id != Some(reservation_id) {
        return Err(format!(
            "X2 loss terminal reservation mismatch: active={:?}, supplied={reservation_id}",
            terminal.active_reservation_id
        ));
    }
    if !state.sealed
        || state.reservation.is_some()
        || state.observations.len() != terminal.row_count
        || snapshot_loss_diagnostics() != terminal.diagnostics
    {
        return Err(format!(
            "X2 loss terminal rows or diagnostics changed before {label}"
        ));
    }
    let discard = InstallAbortDiscard {
        retained_rows: terminal.row_count as u64,
        reserved_rows: terminal.drain.rows.len() as u64,
        discarded_diagnostics: terminal
            .drain
            .dropped_lock_busy
            .saturating_add(terminal.drain.dropped_unbound)
            .saturating_add(terminal.drain.dropped_unreadable)
            .saturating_add(terminal.drain.dropped_invalid)
            .saturating_add(terminal.drain.dropped_overflow)
            .saturating_add(terminal.drain.dropped_nested)
            .saturating_add(terminal.drain.projection_failures),
        poisoned: terminal.drain.completeness_poisoned
            || loss_engine_original_skipped_invocations() != 0,
    };
    let expected_diagnostics = terminal.diagnostics;
    let expected_releases = terminal.released_hooks;
    let mut custody = LOSS_RECEIPT_CUSTODY
        .lock()
        .map_err(|_| "X2 loss receipt custody mutex poisoned".to_string())?;
    let observed_releases = custody
        .released
        .map(|receipt| receipt.expect("complete X2 loss released receipt roster"));
    if custody.installed_published
        || !custody.release_published
        || custody.installed.iter().any(Option::is_some)
        || observed_releases != expected_releases
    {
        return Err(format!(
            "X2 loss released-hook custody changed before terminal {label}"
        ));
    }

    commit_loss_diagnostics(expected_diagnostics)?;
    *state = LossState::default();
    *custody = HookReceiptCustody::new();
    reset_loss_owner_progress();
    LOSS_QUEUE_SEALING.store(false, Ordering::Release);
    LOSS_QUEUE_SEALED.store(false, Ordering::Release);
    Ok(discard)
}

/// Release a failed loss-tail publication without reopening callbacks or
pub fn rollback_terminal_current_loss_observer(reservation_id: u64) -> Result<(), String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 loss terminal rollback")?;
    rollback_terminal_current_loss_observer_inner(reservation_id)
}

fn rollback_terminal_current_loss_observer_inner(reservation_id: u64) -> Result<(), String> {
    let mut state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
    let terminal = state
        .terminal
        .as_mut()
        .ok_or_else(|| "X2 loss has no terminal custody to roll back".to_string())?;
    if terminal.active_reservation_id != Some(reservation_id) {
        return Err(format!(
            "X2 loss terminal reservation mismatch: active={:?}, supplied={reservation_id}",
            terminal.active_reservation_id
        ));
    }
    terminal.active_reservation_id = None;
    Ok(())
}

fn publish_release_and_seal_panel() -> Result<(), String> {
    let mut state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?;
    if state.sealed {
        return Err("X2 panel queue was already sealed before final drain".into());
    }
    if state.reservation.is_some() || state.terminal.is_some() {
        return Err("X2 panel final seal cannot replace existing evidence custody".into());
    }
    let mut custody = PANEL_RECEIPT_CUSTODY
        .lock()
        .map_err(|_| "X2 panel receipt custody mutex poisoned".to_string())?;
    if custody.installed.iter().any(Option::is_some) || custody.released.iter().any(Option::is_none)
    {
        return Err("X2 panel cannot publish an incomplete released roster".into());
    }
    let released_hooks = custody
        .released
        .map(|receipt| receipt.expect("complete X2 panel released receipt roster"));
    let diagnostics = snapshot_panel_diagnostics();
    let drain = panel_drain(state.observations.iter().cloned().collect(), diagnostics);
    custody.installed_published = false;
    custody.release_published = true;
    state.sealed = true;
    state.bindings.clear();
    state.terminal = Some(PanelTerminalQueueState {
        active_reservation_id: None,
        row_count: state.observations.len(),
        diagnostics,
        drain,
        released_hooks,
    });
    PANEL_QUEUE_SEALED.store(true, Ordering::Release);
    Ok(())
}

fn reserve_panel_terminal_locked() -> Result<PanelTerminalReservation, String> {
    let mut state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?;
    if !state.sealed || !PANEL_QUEUE_SEALED.load(Ordering::Acquire) {
        return Err("X2 panel terminal custody is not sealed".into());
    }
    let terminal = state
        .terminal
        .as_mut()
        .ok_or_else(|| "X2 panel sealed queue has no terminal custody".to_string())?;
    if let Some(active) = terminal.active_reservation_id {
        return Err(format!(
            "X2 panel terminal custody already has reservation {active}"
        ));
    }
    let reservation_id = next_queue_reservation_id(&NEXT_PANEL_RESERVATION_ID, "panel terminal")?;
    terminal.active_reservation_id = Some(reservation_id);
    Ok(PanelTerminalReservation {
        reservation_id,
        drain: terminal.drain.clone(),
        released_hooks: terminal.released_hooks,
    })
}

/// release receipts have all been acknowledged by the enclosing sink.
pub fn commit_terminal_panel_observer(reservation_id: u64) -> Result<(), String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 panel terminal commit")?;
    consume_terminal_panel_observer_inner(reservation_id, "commit").map(|_| ())
}

/// finish, and resolves its release custody. See
pub fn abort_terminal_panel_observer(reservation_id: u64) -> Result<InstallAbortDiscard, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 panel terminal install abort")?;
    consume_terminal_panel_observer_inner(reservation_id, "install abort")
}

/// Seals, releases and discards the whole panel carrier for a production hook
pub fn abort_partial_install_and_uninstall_panel_observer(
) -> Result<(InstallAbortDiscard, [ReleasedRvaHookReceipt; 3]), String> {
    let reservation = seal_and_reserve_terminal_panel_observer()?;
    let released = reservation.released_hooks;
    let discard = abort_terminal_panel_observer(reservation.reservation_id)?;
    Ok((discard, released))
}

fn consume_terminal_panel_observer_inner(
    reservation_id: u64,
    label: &str,
) -> Result<InstallAbortDiscard, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| format!("X2 panel terminal {label} mutex poisoned"))?;
    if PANEL_INSTALLED.load(Ordering::Acquire)
        || PANEL_MODULE_BASE.load(Ordering::Acquire) != 0
        || !PANEL_QUEUE_SEALED.load(Ordering::Acquire)
    {
        return Err(format!(
            "X2 panel terminal {label} requires completed physical teardown"
        ));
    }
    let mut state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?;
    let terminal = state
        .terminal
        .as_ref()
        .ok_or_else(|| format!("X2 panel has no terminal custody to {label}"))?;
    if terminal.active_reservation_id != Some(reservation_id) {
        return Err(format!(
            "X2 panel terminal reservation mismatch: active={:?}, supplied={reservation_id}",
            terminal.active_reservation_id
        ));
    }
    if !state.sealed
        || state.reservation.is_some()
        || state.observations.len() != terminal.row_count
        || snapshot_panel_diagnostics() != terminal.diagnostics
    {
        return Err(format!(
            "X2 panel terminal rows or diagnostics changed before {label}"
        ));
    }
    let discarded_panel_diagnostics = terminal
        .drain
        .dropped_lock_busy
        .saturating_add(terminal.drain.dropped_unbound)
        .saturating_add(terminal.drain.dropped_invalid)
        .saturating_add(terminal.drain.dropped_overflow)
        .saturating_add(terminal.drain.dropped_nested);
    let discard = InstallAbortDiscard {
        retained_rows: terminal.row_count as u64,
        reserved_rows: terminal.drain.observations.len() as u64,
        discarded_diagnostics: discarded_panel_diagnostics,
        poisoned: discarded_panel_diagnostics != 0
            || panel_engine_original_skipped_invocations() != 0,
    };
    let expected_diagnostics = terminal.diagnostics;
    let expected_releases = terminal.released_hooks;
    let mut custody = PANEL_RECEIPT_CUSTODY
        .lock()
        .map_err(|_| "X2 panel receipt custody mutex poisoned".to_string())?;
    if custody.released.iter().any(Option::is_none) {
        return Err(format!(
            "X2 panel released-hook custody became incomplete before {label}"
        ));
    }
    let observed_releases = custody
        .released
        .map(|receipt| receipt.expect("complete X2 panel released receipt roster"));
    if custody.installed_published
        || !custody.release_published
        || custody.installed.iter().any(Option::is_some)
        || observed_releases != expected_releases
    {
        return Err(format!(
            "X2 panel released-hook custody changed before terminal {label}"
        ));
    }

    commit_panel_diagnostics(expected_diagnostics)?;
    *state = PanelState::default();
    *custody = HookReceiptCustody::new();
    reset_panel_owner_progress();
    PANEL_QUEUE_SEALING.store(false, Ordering::Release);
    PANEL_QUEUE_SEALED.store(false, Ordering::Release);
    Ok(discard)
}

/// hooks, exact tail, diagnostics, and release receipts remain sealed so the
pub fn rollback_terminal_panel_observer(reservation_id: u64) -> Result<(), String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 panel terminal rollback")?;
    rollback_terminal_panel_observer_inner(reservation_id)
}

fn rollback_terminal_panel_observer_inner(reservation_id: u64) -> Result<(), String> {
    let mut state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?;
    let terminal = state
        .terminal
        .as_mut()
        .ok_or_else(|| "X2 panel has no terminal custody to roll back".to_string())?;
    if terminal.active_reservation_id != Some(reservation_id) {
        return Err(format!(
            "X2 panel terminal reservation mismatch: active={:?}, supplied={reservation_id}",
            terminal.active_reservation_id
        ));
    }
    terminal.active_reservation_id = None;
    Ok(())
}

pub(crate) fn preflight_combined_terminal_commit(
    width_id: u64,
    loss_id: u64,
    panel_id: u64,
) -> Result<(), String> {
    if !crate::terminal_coordinator::individual_terminal_api_blocked() {
        return Err("X2 combined commit preflight requires combined custody".into());
    }
    if WIDTH_INSTALLED.load(Ordering::Acquire)
        || LOSS_INSTALLED.load(Ordering::Acquire)
        || PANEL_INSTALLED.load(Ordering::Acquire)
        || MODULE_BASE.load(Ordering::Acquire) != 0
        || LOSS_MODULE_BASE.load(Ordering::Acquire) != 0
        || PANEL_MODULE_BASE.load(Ordering::Acquire) != 0
    {
        return Err("X2 combined commit preflight requires complete physical teardown".into());
    }
    let width_state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "combined X2 width state mutex poisoned".to_string())?;
    let loss_state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "combined X2 loss state mutex poisoned".to_string())?;
    let panel_state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "combined X2 panel state mutex poisoned".to_string())?;
    let width_terminal = width_state
        .terminal
        .as_ref()
        .ok_or_else(|| "combined X2 width terminal custody is missing".to_string())?;
    let loss_terminal = loss_state
        .terminal
        .as_ref()
        .ok_or_else(|| "combined X2 loss terminal custody is missing".to_string())?;
    let panel_terminal = panel_state
        .terminal
        .as_ref()
        .ok_or_else(|| "combined X2 panel terminal custody is missing".to_string())?;
    if width_terminal.active_reservation_id != Some(width_id)
        || loss_terminal.active_reservation_id != Some(loss_id)
        || panel_terminal.active_reservation_id != Some(panel_id)
        || !width_state.sealed
        || !loss_state.sealed
        || !panel_state.sealed
        || width_state.reservation.is_some()
        || loss_state.reservation.is_some()
        || panel_state.reservation.is_some()
        || width_state.calls.len() != width_terminal.row_count
        || loss_state.observations.len() != loss_terminal.row_count
        || panel_state.observations.len() != panel_terminal.row_count
        || snapshot_width_diagnostics() != width_terminal.diagnostics
        || snapshot_loss_diagnostics() != loss_terminal.diagnostics
        || snapshot_panel_diagnostics() != panel_terminal.diagnostics
    {
        return Err("combined X2 terminal rows, diagnostics, or reservation ids changed".into());
    }
    let width_custody = WIDTH_RECEIPT_CUSTODY
        .lock()
        .map_err(|_| "combined X2 width receipt mutex poisoned".to_string())?;
    let loss_custody = LOSS_RECEIPT_CUSTODY
        .lock()
        .map_err(|_| "combined X2 loss receipt mutex poisoned".to_string())?;
    let panel_custody = PANEL_RECEIPT_CUSTODY
        .lock()
        .map_err(|_| "combined X2 panel receipt mutex poisoned".to_string())?;
    if !complete_terminal_receipts_match(&width_custody, &width_terminal.released_hooks)
        || !complete_terminal_receipts_match(&loss_custody, &loss_terminal.released_hooks)
        || !complete_terminal_receipts_match(&panel_custody, &panel_terminal.released_hooks)
    {
        return Err("combined X2 released-hook custody changed before commit".into());
    }
    Ok(())
}

fn complete_terminal_receipts_match<const N: usize>(
    custody: &HookReceiptCustody<N>,
    expected: &[ReleasedRvaHookReceipt; N],
) -> bool {
    if custody.installed_published
        || !custody.release_published
        || custody.installed.iter().any(Option::is_some)
        || custody.released.iter().any(Option::is_none)
        || !custody.rollback_relays.is_empty()
    {
        return false;
    }
    custody
        .released
        .iter()
        .zip(expected)
        .all(|(observed, expected)| observed.as_ref() == Some(expected))
}

pub(crate) fn commit_combined_terminal_preflighted(width_id: u64, loss_id: u64, panel_id: u64) {
    let _install = INSTALL_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut width_state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut loss_state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut panel_state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut width_custody = WIDTH_RECEIPT_CUSTODY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut loss_custody = LOSS_RECEIPT_CUSTODY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut panel_custody = PANEL_RECEIPT_CUSTODY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    debug_assert_eq!(
        width_state
            .terminal
            .as_ref()
            .and_then(|terminal| terminal.active_reservation_id),
        Some(width_id)
    );
    debug_assert_eq!(
        loss_state
            .terminal
            .as_ref()
            .and_then(|terminal| terminal.active_reservation_id),
        Some(loss_id)
    );
    debug_assert_eq!(
        panel_state
            .terminal
            .as_ref()
            .and_then(|terminal| terminal.active_reservation_id),
        Some(panel_id)
    );
    let width_diagnostics = width_state
        .terminal
        .as_ref()
        .expect("preflighted combined width terminal")
        .diagnostics;
    let loss_diagnostics = loss_state
        .terminal
        .as_ref()
        .expect("preflighted combined loss terminal")
        .diagnostics;
    let panel_diagnostics = panel_state
        .terminal
        .as_ref()
        .expect("preflighted combined panel terminal")
        .diagnostics;
    subtract_width_diagnostics(width_diagnostics);
    subtract_loss_diagnostics(loss_diagnostics);
    subtract_panel_diagnostics(panel_diagnostics);
    *width_state = WidthState::default();
    *loss_state = LossState::default();
    *panel_state = PanelState::default();
    *width_custody = HookReceiptCustody::new();
    *loss_custody = HookReceiptCustody::new();
    *panel_custody = HookReceiptCustody::new();
    reset_width_owner_progress();
    reset_loss_owner_progress();
    reset_panel_owner_progress();
    WIDTH_QUEUE_SEALING.store(false, Ordering::Release);
    WIDTH_QUEUE_SEALED.store(false, Ordering::Release);
    LOSS_QUEUE_SEALING.store(false, Ordering::Release);
    LOSS_QUEUE_SEALED.store(false, Ordering::Release);
    PANEL_QUEUE_SEALING.store(false, Ordering::Release);
    PANEL_QUEUE_SEALED.store(false, Ordering::Release);
}

fn subtract_width_diagnostics(snapshot: WidthDiagnostics) {
    WIDTH_DROPPED_LOCK_BUSY.fetch_sub(snapshot.dropped_lock_busy, Ordering::AcqRel);
    WIDTH_DROPPED_UNBOUND.fetch_sub(snapshot.dropped_unbound, Ordering::AcqRel);
    WIDTH_DROPPED_UNREADABLE.fetch_sub(snapshot.dropped_unreadable, Ordering::AcqRel);
    WIDTH_DROPPED_OVERFLOW.fetch_sub(snapshot.dropped_overflow, Ordering::AcqRel);
    WIDTH_DROPPED_NESTED.fetch_sub(snapshot.dropped_nested, Ordering::AcqRel);
}

fn subtract_loss_diagnostics(snapshot: LossDiagnostics) {
    LOSS_DROPPED_LOCK_BUSY.fetch_sub(snapshot.dropped_lock_busy, Ordering::AcqRel);
    LOSS_DROPPED_UNBOUND.fetch_sub(snapshot.dropped_unbound, Ordering::AcqRel);
    LOSS_DROPPED_UNREADABLE.fetch_sub(snapshot.dropped_unreadable, Ordering::AcqRel);
    LOSS_DROPPED_INVALID.fetch_sub(snapshot.dropped_invalid, Ordering::AcqRel);
    LOSS_DROPPED_OVERFLOW.fetch_sub(snapshot.dropped_overflow, Ordering::AcqRel);
    LOSS_DROPPED_NESTED.fetch_sub(snapshot.dropped_nested, Ordering::AcqRel);
}

fn subtract_panel_diagnostics(snapshot: PanelDiagnostics) {
    PANEL_DROPPED_LOCK_BUSY.fetch_sub(snapshot.dropped_lock_busy, Ordering::AcqRel);
    PANEL_DROPPED_UNBOUND.fetch_sub(snapshot.dropped_unbound, Ordering::AcqRel);
    PANEL_DROPPED_INVALID.fetch_sub(snapshot.dropped_invalid, Ordering::AcqRel);
    PANEL_DROPPED_OVERFLOW.fetch_sub(snapshot.dropped_overflow, Ordering::AcqRel);
    PANEL_DROPPED_NESTED.fetch_sub(snapshot.dropped_nested, Ordering::AcqRel);
}

pub(crate) fn preflight_combined_terminal_rollback(
    width_id: u64,
    loss_id: u64,
    panel_id: u64,
) -> Result<(), String> {
    let width_state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "combined X2 width rollback mutex poisoned".to_string())?;
    let loss_state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "combined X2 loss rollback mutex poisoned".to_string())?;
    let panel_state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "combined X2 panel rollback mutex poisoned".to_string())?;
    if width_state
        .terminal
        .as_ref()
        .and_then(|terminal| terminal.active_reservation_id)
        != Some(width_id)
        || loss_state
            .terminal
            .as_ref()
            .and_then(|terminal| terminal.active_reservation_id)
            != Some(loss_id)
        || panel_state
            .terminal
            .as_ref()
            .and_then(|terminal| terminal.active_reservation_id)
            != Some(panel_id)
    {
        return Err("combined X2 rollback reservation id changed".into());
    }
    Ok(())
}

pub(crate) fn rollback_combined_terminal_preflighted(width_id: u64, loss_id: u64, panel_id: u64) {
    let mut width_state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut loss_state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut panel_state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    debug_assert_eq!(
        width_state
            .terminal
            .as_ref()
            .and_then(|terminal| terminal.active_reservation_id),
        Some(width_id)
    );
    debug_assert_eq!(
        loss_state
            .terminal
            .as_ref()
            .and_then(|terminal| terminal.active_reservation_id),
        Some(loss_id)
    );
    debug_assert_eq!(
        panel_state
            .terminal
            .as_ref()
            .and_then(|terminal| terminal.active_reservation_id),
        Some(panel_id)
    );
    width_state
        .terminal
        .as_mut()
        .expect("preflighted combined width rollback")
        .active_reservation_id = None;
    loss_state
        .terminal
        .as_mut()
        .expect("preflighted combined loss rollback")
        .active_reservation_id = None;
    panel_state
        .terminal
        .as_mut()
        .expect("preflighted combined panel rollback")
        .active_reservation_id = None;
}

pub fn publish_loss_side_bindings(bindings: &[LossSideBinding]) -> Result<(), String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 loss binding publication")?;
    if loss_queue_admission_closed() {
        return Err("X2 loss bindings are frozen for terminal teardown".into());
    }
    let mut next = BTreeMap::new();
    for binding in bindings {
        let owner_ptr = binding
            .side_ptr
            .checked_add(LOSS_OWNER_OFFSET_FROM_SIDE)
            .ok_or_else(|| "X2 loss side-owner address overflow".to_string())?;
        if binding.combat_ptr == 0
            || binding.side_ptr == 0
            || binding.combat_id == 0
            || binding.generation == 0
            || next.insert(owner_ptr, binding.clone()).is_some()
        {
            return Err("invalid or duplicate X2 loss side binding".into());
        }
    }
    let state = LOSS_STATE.get_or_init(|| Mutex::new(LossState::default()));
    let mut state = state
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
    if state.sealed || LOSS_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 loss bindings are frozen for terminal teardown".into());
    }
    state.bindings = next;
    Ok(())
}

pub fn publish_panel_side_bindings(bindings: &[SideBinding]) -> Result<(), String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 panel binding publication")?;
    if panel_queue_admission_closed() {
        return Err("X2 panel bindings are frozen for terminal teardown".into());
    }
    let mut next = BTreeMap::new();
    for binding in bindings {
        if binding.side_ptr == 0
            || binding.combat_id == 0
            || binding.generation == 0
            || next.insert(binding.side_ptr, binding.clone()).is_some()
        {
            return Err("invalid or duplicate X2 panel side binding".into());
        }
    }
    let mut state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?;
    if state.sealed || PANEL_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 panel bindings are frozen for terminal teardown".into());
    }
    state.bindings = next;
    Ok(())
}

fn next_queue_reservation_id(counter: &AtomicU64, label: &str) -> Result<u64, String> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .map_err(|_| format!("X2 {label} reservation ID exhausted"))
}

fn width_queue_admission_closed() -> bool {
    crate::terminal_coordinator::individual_terminal_api_blocked()
        || WIDTH_QUEUE_SEALED.load(Ordering::Acquire)
        || WIDTH_QUEUE_SEALING.load(Ordering::Acquire)
}

fn loss_queue_admission_closed() -> bool {
    crate::terminal_coordinator::individual_terminal_api_blocked()
        || LOSS_QUEUE_SEALED.load(Ordering::Acquire)
        || LOSS_QUEUE_SEALING.load(Ordering::Acquire)
}

fn panel_queue_admission_closed() -> bool {
    crate::terminal_coordinator::individual_terminal_api_blocked()
        || PANEL_QUEUE_SEALED.load(Ordering::Acquire)
        || PANEL_QUEUE_SEALING.load(Ordering::Acquire)
}

fn snapshot_panel_diagnostics() -> PanelDiagnostics {
    PanelDiagnostics {
        dropped_lock_busy: PANEL_DROPPED_LOCK_BUSY.load(Ordering::Acquire),
        dropped_unbound: PANEL_DROPPED_UNBOUND.load(Ordering::Acquire),
        dropped_invalid: PANEL_DROPPED_INVALID.load(Ordering::Acquire),
        dropped_overflow: PANEL_DROPPED_OVERFLOW.load(Ordering::Acquire),
        dropped_nested: PANEL_DROPPED_NESTED.load(Ordering::Acquire),
    }
}

fn commit_panel_diagnostics(snapshot: PanelDiagnostics) -> Result<(), String> {
    if PANEL_DROPPED_LOCK_BUSY.load(Ordering::Acquire) < snapshot.dropped_lock_busy
        || PANEL_DROPPED_UNBOUND.load(Ordering::Acquire) < snapshot.dropped_unbound
        || PANEL_DROPPED_INVALID.load(Ordering::Acquire) < snapshot.dropped_invalid
        || PANEL_DROPPED_OVERFLOW.load(Ordering::Acquire) < snapshot.dropped_overflow
        || PANEL_DROPPED_NESTED.load(Ordering::Acquire) < snapshot.dropped_nested
    {
        return Err("X2 panel diagnostic reservation snapshot is no longer retained".into());
    }
    PANEL_DROPPED_LOCK_BUSY.fetch_sub(snapshot.dropped_lock_busy, Ordering::AcqRel);
    PANEL_DROPPED_UNBOUND.fetch_sub(snapshot.dropped_unbound, Ordering::AcqRel);
    PANEL_DROPPED_INVALID.fetch_sub(snapshot.dropped_invalid, Ordering::AcqRel);
    PANEL_DROPPED_OVERFLOW.fetch_sub(snapshot.dropped_overflow, Ordering::AcqRel);
    PANEL_DROPPED_NESTED.fetch_sub(snapshot.dropped_nested, Ordering::AcqRel);
    Ok(())
}

fn panel_drain(
    observations: Vec<PanelObservationRow>,
    diagnostics: PanelDiagnostics,
) -> PanelDrain {
    PanelDrain {
        observations,
        dropped_lock_busy: diagnostics.dropped_lock_busy,
        dropped_unbound: diagnostics.dropped_unbound,
        dropped_invalid: diagnostics.dropped_invalid,
        dropped_overflow: diagnostics.dropped_overflow,
        dropped_nested: diagnostics.dropped_nested,
    }
}

pub fn peek_panel_observations(
    max_observations: usize,
) -> Result<Option<PanelBatchReservation>, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 panel reservation")?;
    if max_observations == 0 || max_observations > MAX_PENDING_PANEL_OBSERVATIONS {
        return Err(format!(
            "X2 panel reservation bound must be in 1..={MAX_PENDING_PANEL_OBSERVATIONS}"
        ));
    }
    if panel_queue_admission_closed() {
        return Err("X2 panel queue is sealed for final teardown drain".into());
    }
    let mut state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?;
    if state.sealed || PANEL_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 panel queue is sealed for final teardown drain".into());
    }
    if let Some(reservation) = state.reservation {
        return Err(format!(
            "X2 panel queue already has reservation {}: reserved={}, pending_unreserved={}",
            reservation.reservation_id,
            reservation.row_count,
            state
                .observations
                .len()
                .saturating_sub(reservation.row_count)
        ));
    }
    let row_count = state.observations.len().min(max_observations);
    let diagnostics = snapshot_panel_diagnostics();
    if row_count == 0 && diagnostics == PanelDiagnostics::default() {
        return Ok(None);
    }
    let reservation_id = next_queue_reservation_id(&NEXT_PANEL_RESERVATION_ID, "panel")?;
    state.reservation = Some(PanelQueueReservation {
        reservation_id,
        row_count,
        diagnostics,
    });
    Ok(Some(PanelBatchReservation {
        reservation_id,
        drain: panel_drain(
            state.observations.iter().take(row_count).cloned().collect(),
            diagnostics,
        ),
    }))
}

pub fn commit_panel_observations(reservation_id: u64) -> Result<(), String> {
    let _combined_api = crate::terminal_coordinator::try_admit_individual_api("X2 panel commit")?;
    let mut state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?;
    if state.sealed || PANEL_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 panel queue is sealed for final teardown drain".into());
    }
    let reservation = state
        .reservation
        .ok_or_else(|| "X2 panel queue has no active reservation".to_string())?;
    if reservation.reservation_id != reservation_id {
        return Err(format!(
            "X2 panel reservation ID mismatch: active={}, supplied={reservation_id}",
            reservation.reservation_id
        ));
    }
    if state.observations.len() < reservation.row_count {
        return Err("X2 panel reserved prefix is no longer present".into());
    }
    commit_panel_diagnostics(reservation.diagnostics)?;
    state.observations.drain(..reservation.row_count);
    state.reservation = None;
    Ok(())
}

pub fn rollback_panel_observations(reservation_id: u64) -> Result<(), String> {
    let _combined_api = crate::terminal_coordinator::try_admit_individual_api("X2 panel rollback")?;
    let mut state = PANEL_STATE
        .get_or_init(|| Mutex::new(PanelState::default()))
        .lock()
        .map_err(|_| "X2 panel state mutex poisoned".to_string())?;
    if state.sealed || PANEL_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 panel queue is sealed for final teardown drain".into());
    }
    let reservation = state
        .reservation
        .ok_or_else(|| "X2 panel queue has no active reservation".to_string())?;
    if reservation.reservation_id != reservation_id {
        return Err(format!(
            "X2 panel reservation ID mismatch: active={}, supplied={reservation_id}",
            reservation.reservation_id
        ));
    }
    state.reservation = None;
    Ok(())
}

#[deprecated(note = "use peek_panel_observations plus commit/rollback after sink acknowledgement")]
pub fn drain_panel_observations() -> Result<PanelDrain, String> {
    Err("destructive X2 panel drain is disabled; use transactional panel reservation APIs".into())
}

fn snapshot_width_diagnostics() -> WidthDiagnostics {
    WidthDiagnostics {
        dropped_lock_busy: WIDTH_DROPPED_LOCK_BUSY.load(Ordering::Acquire),
        dropped_unbound: WIDTH_DROPPED_UNBOUND.load(Ordering::Acquire),
        dropped_unreadable: WIDTH_DROPPED_UNREADABLE.load(Ordering::Acquire),
        dropped_overflow: WIDTH_DROPPED_OVERFLOW.load(Ordering::Acquire),
        dropped_nested: WIDTH_DROPPED_NESTED.load(Ordering::Acquire),
    }
}

fn commit_width_diagnostics(snapshot: WidthDiagnostics) -> Result<(), String> {
    if WIDTH_DROPPED_LOCK_BUSY.load(Ordering::Acquire) < snapshot.dropped_lock_busy
        || WIDTH_DROPPED_UNBOUND.load(Ordering::Acquire) < snapshot.dropped_unbound
        || WIDTH_DROPPED_UNREADABLE.load(Ordering::Acquire) < snapshot.dropped_unreadable
        || WIDTH_DROPPED_OVERFLOW.load(Ordering::Acquire) < snapshot.dropped_overflow
        || WIDTH_DROPPED_NESTED.load(Ordering::Acquire) < snapshot.dropped_nested
    {
        return Err("X2 width diagnostic reservation snapshot is no longer retained".into());
    }
    WIDTH_DROPPED_LOCK_BUSY.fetch_sub(snapshot.dropped_lock_busy, Ordering::AcqRel);
    WIDTH_DROPPED_UNBOUND.fetch_sub(snapshot.dropped_unbound, Ordering::AcqRel);
    WIDTH_DROPPED_UNREADABLE.fetch_sub(snapshot.dropped_unreadable, Ordering::AcqRel);
    WIDTH_DROPPED_OVERFLOW.fetch_sub(snapshot.dropped_overflow, Ordering::AcqRel);
    WIDTH_DROPPED_NESTED.fetch_sub(snapshot.dropped_nested, Ordering::AcqRel);
    Ok(())
}

fn width_drain(calls: Vec<RawWidthCallCapture>, diagnostics: WidthDiagnostics) -> WidthDrain {
    WidthDrain {
        calls,
        dropped_lock_busy: diagnostics.dropped_lock_busy,
        dropped_unbound: diagnostics.dropped_unbound,
        dropped_unreadable: diagnostics.dropped_unreadable,
        dropped_overflow: diagnostics.dropped_overflow,
        dropped_nested: diagnostics.dropped_nested,
    }
}

pub fn peek_width_calls(max_calls: usize) -> Result<Option<WidthBatchReservation>, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 width reservation")?;
    if max_calls == 0 || max_calls > MAX_PENDING_WIDTH_CALLS {
        return Err(format!(
            "X2 width reservation bound must be in 1..={MAX_PENDING_WIDTH_CALLS}"
        ));
    }
    if width_queue_admission_closed() {
        return Err("X2 width queue is sealed for final teardown drain".into());
    }
    let mut state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?;
    if state.sealed || WIDTH_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 width queue is sealed for final teardown drain".into());
    }
    if let Some(reservation) = state.reservation {
        return Err(format!(
            "X2 width queue already has reservation {}: reserved={}, pending_unreserved={}",
            reservation.reservation_id,
            reservation.row_count,
            state.calls.len().saturating_sub(reservation.row_count)
        ));
    }
    let row_count = state.calls.len().min(max_calls);
    let diagnostics = snapshot_width_diagnostics();
    if row_count == 0 && diagnostics == WidthDiagnostics::default() {
        return Ok(None);
    }
    let reservation_id = next_queue_reservation_id(&NEXT_WIDTH_RESERVATION_ID, "width")?;
    state.reservation = Some(WidthQueueReservation {
        reservation_id,
        row_count,
        diagnostics,
    });
    Ok(Some(WidthBatchReservation {
        reservation_id,
        drain: width_drain(
            state.calls.iter().take(row_count).cloned().collect(),
            diagnostics,
        ),
    }))
}

pub fn commit_width_calls(reservation_id: u64) -> Result<(), String> {
    let _combined_api = crate::terminal_coordinator::try_admit_individual_api("X2 width commit")?;
    let mut state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?;
    if state.sealed || WIDTH_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 width queue is sealed for final teardown drain".into());
    }
    let reservation = state
        .reservation
        .ok_or_else(|| "X2 width queue has no active reservation".to_string())?;
    if reservation.reservation_id != reservation_id {
        return Err(format!(
            "X2 width reservation ID mismatch: active={}, supplied={reservation_id}",
            reservation.reservation_id
        ));
    }
    if state.calls.len() < reservation.row_count {
        return Err("X2 width reserved prefix is no longer present".into());
    }
    commit_width_diagnostics(reservation.diagnostics)?;
    state.calls.drain(..reservation.row_count);
    state.reservation = None;
    Ok(())
}

/// Release ownership without removing the reserved prefix.
pub fn rollback_width_calls(reservation_id: u64) -> Result<(), String> {
    let _combined_api = crate::terminal_coordinator::try_admit_individual_api("X2 width rollback")?;
    let mut state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?;
    if state.sealed || WIDTH_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 width queue is sealed for final teardown drain".into());
    }
    let reservation = state
        .reservation
        .ok_or_else(|| "X2 width queue has no active reservation".to_string())?;
    if reservation.reservation_id != reservation_id {
        return Err(format!(
            "X2 width reservation ID mismatch: active={}, supplied={reservation_id}",
            reservation.reservation_id
        ));
    }
    state.reservation = None;
    Ok(())
}

#[deprecated(note = "use peek_width_calls plus commit/rollback after sink acknowledgement")]
pub fn drain_width_calls() -> Result<WidthDrain, String> {
    Err("destructive X2 width drain is disabled; use transactional width reservation APIs".into())
}

pub fn project_width_call(capture: &RawWidthCallCapture) -> Result<Vec<WidthObservationRow>, String> {
    if !capture.byte_gates_validated || capture.wrapper_count == 0 {
        return Err("width capture lacks byte gates or wrapper rows".into());
    }
    let mut sum = 0i64;
    let mut rows = Vec::with_capacity(capture.wrapper_count);
    let mut division_ids = BTreeSet::new();
    for (ordinal, raw) in capture.wrappers[..capture.wrapper_count].iter().enumerate() {
        let division_id = raw.division_id.ok_or("width wrapper identity was unavailable at capture")?;
        if !division_ids.insert(division_id) {
            return Err("width wrapper division identity repeats within one call".into());
        }
        sum = sum
            .checked_add(raw.width_raw)
            .ok_or_else(|| "width exact sum overflow".to_string())?;
        rows.push(WidthObservationRow {
        observed_game_hour_raw: capture.observed_game_hour_raw,
            combat_id: capture.combat_id,
            generation: capture.generation,
            side: capture.side,
            capture_sequence: capture.capture_sequence,
            ordinal: ordinal as u32,
            wrapper_count: capture.wrapper_count as u32,
            division_id,
            control_flags: WIDTH_REQUIRED_FLAG_MASK,
            wrapper_width_raw: raw.width_raw,
            final_width_raw: capture.final_width_raw,
        });
    }
    if sum != capture.final_width_raw {
        return Err("width wrapper sum does not equal producer final result".into());
    }
    Ok(rows)
}

pub fn submit_raw_loss_observation(observation: RawLossWriterObservation) -> Result<(), String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 loss submission")?;
    enqueue_raw_loss_observation(observation, false)
}

fn enqueue_raw_loss_observation(
    observation: RawLossWriterObservation,
    admitted_callback: bool,
) -> Result<(), String> {
    if !current_loss_observer_requested() {
        return Err(
            "X2 loss carrier inactive: production promotion is false and no nonce-bound evidence capture was requested".into(),
        );
    }
    if !admitted_callback && loss_queue_admission_closed() {
        return Err("X2 loss queue is sealed for final teardown drain".into());
    }
    let state_mutex = LOSS_STATE.get_or_init(|| Mutex::new(LossState::default()));
    let mut state = match state_mutex.try_lock() {
        Ok(state) => state,
        Err(_) if admitted_callback && LOSS_QUEUE_SEALING.load(Ordering::Acquire) => {
            // deadlock with the lifecycle owner.
            state_mutex.lock().map_err(|_| {
                LOSS_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
                "X2 loss queue mutex poisoned during admitted teardown callback".to_string()
            })?
        }
        Err(_) => {
            if !admitted_callback && loss_queue_admission_closed() {
                return Err("X2 loss queue is sealed for final teardown drain".into());
            }
            LOSS_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
            return Err("X2 loss queue busy".into());
        }
    };
    if state.sealed || (!admitted_callback && LOSS_QUEUE_SEALING.load(Ordering::Acquire)) {
        if admitted_callback {
            LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
            return Err("X2 admitted loss callback reached an already sealed queue".into());
        }
        if loss_queue_admission_closed() {
            return Err("X2 loss queue is sealed for final teardown drain".into());
        }
    }
    validate_raw_loss_observation(&observation).map_err(|error| {
        LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        error
    })?;
    if state.observations.len() >= MAX_PENDING_LOSS_OBSERVATIONS {
        LOSS_DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        return Err("X2 loss queue full".into());
    }
    state.observations.push_back(observation);
    Ok(())
}

fn snapshot_loss_diagnostics() -> LossDiagnostics {
    LossDiagnostics {
        dropped_lock_busy: LOSS_DROPPED_LOCK_BUSY.load(Ordering::Acquire),
        dropped_unbound: LOSS_DROPPED_UNBOUND.load(Ordering::Acquire),
        dropped_unreadable: LOSS_DROPPED_UNREADABLE.load(Ordering::Acquire),
        dropped_invalid: LOSS_DROPPED_INVALID.load(Ordering::Acquire),
        dropped_overflow: LOSS_DROPPED_OVERFLOW.load(Ordering::Acquire),
        dropped_nested: LOSS_DROPPED_NESTED.load(Ordering::Acquire),
    }
}

fn commit_loss_diagnostics(snapshot: LossDiagnostics) -> Result<(), String> {
    if LOSS_DROPPED_LOCK_BUSY.load(Ordering::Acquire) < snapshot.dropped_lock_busy
        || LOSS_DROPPED_UNBOUND.load(Ordering::Acquire) < snapshot.dropped_unbound
        || LOSS_DROPPED_UNREADABLE.load(Ordering::Acquire) < snapshot.dropped_unreadable
        || LOSS_DROPPED_INVALID.load(Ordering::Acquire) < snapshot.dropped_invalid
        || LOSS_DROPPED_OVERFLOW.load(Ordering::Acquire) < snapshot.dropped_overflow
        || LOSS_DROPPED_NESTED.load(Ordering::Acquire) < snapshot.dropped_nested
    {
        return Err("X2 loss diagnostic reservation snapshot is no longer retained".into());
    }
    LOSS_DROPPED_LOCK_BUSY.fetch_sub(snapshot.dropped_lock_busy, Ordering::AcqRel);
    LOSS_DROPPED_UNBOUND.fetch_sub(snapshot.dropped_unbound, Ordering::AcqRel);
    LOSS_DROPPED_UNREADABLE.fetch_sub(snapshot.dropped_unreadable, Ordering::AcqRel);
    LOSS_DROPPED_INVALID.fetch_sub(snapshot.dropped_invalid, Ordering::AcqRel);
    LOSS_DROPPED_OVERFLOW.fetch_sub(snapshot.dropped_overflow, Ordering::AcqRel);
    LOSS_DROPPED_NESTED.fetch_sub(snapshot.dropped_nested, Ordering::AcqRel);
    Ok(())
}

fn loss_drain(
    observations: Vec<RawLossWriterObservation>,
    diagnostics: LossDiagnostics,
) -> LossDrain {
    LossDrain {
        observations,
        dropped_lock_busy: diagnostics.dropped_lock_busy,
        dropped_unbound: diagnostics.dropped_unbound,
        dropped_unreadable: diagnostics.dropped_unreadable,
        dropped_invalid: diagnostics.dropped_invalid,
        dropped_overflow: diagnostics.dropped_overflow,
        dropped_nested: diagnostics.dropped_nested,
    }
}

pub fn peek_loss_observations(
    max_observations: usize,
) -> Result<Option<LossBatchReservation>, String> {
    let _combined_api =
        crate::terminal_coordinator::try_admit_individual_api("X2 loss reservation")?;
    if max_observations == 0 || max_observations > MAX_PENDING_LOSS_OBSERVATIONS {
        return Err(format!(
            "X2 loss reservation bound must be in 1..={MAX_PENDING_LOSS_OBSERVATIONS}"
        ));
    }
    if loss_queue_admission_closed() {
        return Err("X2 loss queue is sealed for final teardown drain".into());
    }
    let mut state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
    if state.sealed || LOSS_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 loss queue is sealed for final teardown drain".into());
    }
    if let Some(reservation) = state.reservation {
        return Err(format!(
            "X2 loss queue already has reservation {}: reserved={}, pending_unreserved={}",
            reservation.reservation_id,
            reservation.row_count,
            state
                .observations
                .len()
                .saturating_sub(reservation.row_count)
        ));
    }
    let row_count = state.observations.len().min(max_observations);
    let diagnostics = snapshot_loss_diagnostics();
    if row_count == 0 && diagnostics == LossDiagnostics::default() {
        return Ok(None);
    }
    let reservation_id = next_queue_reservation_id(&NEXT_LOSS_RESERVATION_ID, "loss")?;
    state.reservation = Some(LossQueueReservation {
        reservation_id,
        row_count,
        diagnostics,
    });
    Ok(Some(LossBatchReservation {
        reservation_id,
        drain: loss_drain(
            state.observations.iter().take(row_count).cloned().collect(),
            diagnostics,
        ),
    }))
}

pub fn commit_loss_observations(reservation_id: u64) -> Result<(), String> {
    let _combined_api = crate::terminal_coordinator::try_admit_individual_api("X2 loss commit")?;
    let mut state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
    if state.sealed || LOSS_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 loss queue is sealed for final teardown drain".into());
    }
    let reservation = state
        .reservation
        .ok_or_else(|| "X2 loss queue has no active reservation".to_string())?;
    if reservation.reservation_id != reservation_id {
        return Err(format!(
            "X2 loss reservation ID mismatch: active={}, supplied={reservation_id}",
            reservation.reservation_id
        ));
    }
    if state.observations.len() < reservation.row_count {
        return Err("X2 loss reserved prefix is no longer present".into());
    }
    commit_loss_diagnostics(reservation.diagnostics)?;
    state.observations.drain(..reservation.row_count);
    state.reservation = None;
    Ok(())
}

/// Release ownership without removing the reserved loss observations.
pub fn rollback_loss_observations(reservation_id: u64) -> Result<(), String> {
    let _combined_api = crate::terminal_coordinator::try_admit_individual_api("X2 loss rollback")?;
    let mut state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
    if state.sealed || LOSS_QUEUE_SEALING.load(Ordering::Acquire) {
        return Err("X2 loss queue is sealed for final teardown drain".into());
    }
    let reservation = state
        .reservation
        .ok_or_else(|| "X2 loss queue has no active reservation".to_string())?;
    if reservation.reservation_id != reservation_id {
        return Err(format!(
            "X2 loss reservation ID mismatch: active={}, supplied={reservation_id}",
            reservation.reservation_id
        ));
    }
    state.reservation = None;
    Ok(())
}

#[deprecated(note = "use peek_loss_observations plus commit/rollback after sink acknowledgement")]
pub fn drain_loss_observations() -> Result<LossDrain, String> {
    Err("destructive X2 loss drain is disabled; use transactional loss reservation APIs".into())
}

pub fn project_loss_observation(
    observation: &RawLossWriterObservation,
    resolver: &impl LossIdentityResolver,
) -> Result<Vec<CurrentLossObservationRow>, String> {
    validate_raw_loss_observation(observation)?;
    let (combat_id, generation, side) = resolver
        .active_epoch(observation.combat_ptr, observation.side_ptr)
        .ok_or_else(|| "X2 loss pointers do not resolve to one current active epoch".to_string())?;
    if combat_id == 0 || generation == 0 {
        return Err("X2 loss active-epoch identity is null".into());
    }
    let contract = AUTHENTICATED_LOSS_WRITER
        .ok_or_else(|| "X2 loss writer contract is not authenticated".to_string())?;
    let source_flag = if observation.writer_rva == contract.active_damage.writer_rva {
        CURRENT_LOSS_FLAG_SOURCE_ACTIVE_DAMAGE
    } else if observation.writer_rva == contract.participant_lifecycle.writer_rva {
        CURRENT_LOSS_FLAG_SOURCE_PARTICIPANT_LIFECYCLE
    } else {
        return Err("X2 loss observation has an unknown producer".into());
    };
    let manpower_before_raw = i64::from_le_bytes(observation.manpower_before_bytes);
    let manpower_after_raw = i64::from_le_bytes(observation.manpower_after_bytes);
    if manpower_before_raw == manpower_after_raw {
        return Err("X2 loss manpower control has no delta".into());
    }
    let mut equipment_ids = BTreeSet::new();
    let mut rows = Vec::with_capacity(observation.equipment.len());
    for equipment in &observation.equipment {
        let equipment_id = resolver
            .equipment_id(equipment.equipment_key_ptr)
            .filter(|id| *id != 0)
            .ok_or_else(|| {
                "X2 loss equipment key has no stable pointer-free identity".to_string()
            })?;
        let equipment_before_raw = i64::from_le_bytes(equipment.before_bytes);
        let equipment_after_raw = i64::from_le_bytes(equipment.after_bytes);
        if !equipment_ids.insert(equipment_id) || equipment_before_raw == equipment_after_raw {
            return Err("X2 loss equipment identity repeats or has no delta".into());
        }
        rows.push(CurrentLossObservationRow {
        observed_game_hour_raw: observation.observed_game_hour_raw,
            combat_id,
            generation,
            side,
            capture_sequence: observation.capture_sequence,
            ordinal: 0,
            equipment_count: observation.equipment.len() as u32,
            equipment_id,
            control_flags: CURRENT_LOSS_REQUIRED_FLAG_MASK | source_flag,
            manpower_before_raw,
            manpower_after_raw,
            equipment_before_raw,
            equipment_after_raw,
        });
    }
    rows.sort_by_key(|row| row.equipment_id);
    for (ordinal, row) in rows.iter_mut().enumerate() {
        row.ordinal = ordinal as u32;
    }
    Ok(rows)
}

fn validate_raw_loss_observation(observation: &RawLossWriterObservation) -> Result<(), String> {
    let contract = AUTHENTICATED_LOSS_WRITER
        .ok_or_else(|| "X2 loss writer contract is not authenticated".to_string())?;
    let writer_matches = [contract.active_damage, contract.participant_lifecycle]
        .into_iter()
        .any(|writer| {
            observation.writer_rva == writer.writer_rva
                && observation.observed_entry_bytes == writer.sentinel
        });
    if observation.combat_ptr == 0
        || observation.side_ptr == 0
        || observation.capture_sequence == 0
        || observation.equipment.is_empty()
        || observation.equipment.len() > MAX_SIDE_UNITS
        || !writer_matches
        || observation.manpower_before_bytes == observation.manpower_after_bytes
    {
        return Err("X2 loss observation fails pointer, byte, or manpower-delta gate".into());
    }
    if observation
        .equipment
        .iter()
        .any(|entry| entry.equipment_key_ptr == 0 || entry.before_bytes == entry.after_bytes)
    {
        return Err("X2 loss observation fails raw equipment key/delta gate".into());
    }
    Ok(())
}

struct LossWriterScope;

impl LossWriterScope {
    fn try_enter() -> Option<Self> {
        LOSS_WRITER_ACTIVE.with(|active| {
            if active.replace(true) {
                LOSS_DROPPED_NESTED.fetch_add(1, Ordering::Relaxed);
                None
            } else {
                Some(Self)
            }
        })
    }
}

impl Drop for LossWriterScope {
    fn drop(&mut self) {
        LOSS_WRITER_ACTIVE.with(|active| active.set(false));
    }
}

unsafe extern "system" fn hook_active_division_damage(
    division_ptr: usize,
    damage_raw: i64,
    damage_kind: u8,
    max_damage_raw: i64,
    selector_ptr: usize,
    owner_ptr: usize,
    opposing_owner_ptr: usize,
    attribution_ptr: usize,
) {
    let active = ActiveLossCallback::enter();
    let Some(original_addr) = ACTIVE_DIVISION_DAMAGE_RELAY.original_for_detour() else {
        return;
    };
    let original: ActiveDivisionDamageFn = unsafe { mem::transmute(original_addr) };
    if !active.combined_admitted() {
        unsafe {
            original(
                division_ptr,
                damage_raw,
                damage_kind,
                max_damage_raw,
                selector_ptr,
                owner_ptr,
                opposing_owner_ptr,
                attribution_ptr,
            )
        };
        return;
    }
    let Some(_scope) = LossWriterScope::try_enter() else {
        unsafe {
            original(
                division_ptr,
                damage_raw,
                damage_kind,
                max_damage_raw,
                selector_ptr,
                owner_ptr,
                opposing_owner_ptr,
                attribution_ptr,
            )
        };
        return;
    };
    let writer = AUTHENTICATED_LOSS_WRITER.map(|contract| contract.active_damage);
    let before = writer.and_then(|writer| begin_loss_writer(owner_ptr, writer));
    unsafe {
        original(
            division_ptr,
            damage_raw,
            damage_kind,
            max_damage_raw,
            selector_ptr,
            owner_ptr,
            opposing_owner_ptr,
            attribution_ptr,
        )
    };
    if let Some(before) = before {
        finish_loss_writer(before);
    }
}

unsafe extern "system" fn hook_participant_lifecycle(side_ptr: usize, division_wrapper_ptr: usize) {
    let active = ActiveLossCallback::enter();
    let Some(original_addr) = PARTICIPANT_LIFECYCLE_RELAY.original_for_detour() else {
        return;
    };
    let original: ParticipantLifecycleFn = unsafe { mem::transmute(original_addr) };
    if !active.combined_admitted() {
        unsafe { original(side_ptr, division_wrapper_ptr) };
        return;
    }
    let Some(_scope) = LossWriterScope::try_enter() else {
        unsafe { original(side_ptr, division_wrapper_ptr) };
        return;
    };
    let owner_ptr = side_ptr.checked_add(LOSS_OWNER_OFFSET_FROM_SIDE);
    let writer = AUTHENTICATED_LOSS_WRITER.map(|contract| contract.participant_lifecycle);
    let before = owner_ptr
        .zip(writer)
        .and_then(|(owner_ptr, writer)| begin_loss_writer(owner_ptr, writer));
    unsafe { original(side_ptr, division_wrapper_ptr) };
    if let Some(before) = before {
        finish_loss_writer(before);
    }
}

fn begin_loss_writer(
    owner_ptr: usize,
    writer: AuthenticatedLossWriterSite,
) -> Option<LossWriterBefore> {
    if !LOSS_INSTALLED.load(Ordering::Acquire) {
        return None;
    }
    if owner_ptr == 0 {
        LOSS_DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let state_mutex = LOSS_STATE.get_or_init(|| Mutex::new(LossState::default()));
    let state = match state_mutex.try_lock() {
        Ok(state) => state,
        Err(_) if LOSS_QUEUE_SEALING.load(Ordering::Acquire) => match state_mutex.lock() {
            Ok(state) => state,
            Err(_) => {
                LOSS_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        },
        Err(_) => {
            LOSS_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    if state.sealed {
        LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let Some(binding) = state.bindings.get(&owner_ptr).cloned() else {
        LOSS_DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    drop(state);
    if binding.side_ptr.checked_add(LOSS_OWNER_OFFSET_FROM_SIDE) != Some(owner_ptr) {
        LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let Some(contract) = AUTHENTICATED_LOSS_WRITER else {
        LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    if writer != contract.active_damage && writer != contract.participant_lifecycle {
        LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let observed_entry_bytes = writer.sentinel;
    let Some(manpower_before) = read_loss_manpower(owner_ptr) else {
        LOSS_DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let Some(equipment_before) = read_loss_equipment_variants(owner_ptr) else {
        LOSS_DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    Some(LossWriterBefore {
        observed_hour_before: crate::hour_custody::current_game_hour_raw(),
        binding,
        capture_sequence: NEXT_LOSS_CAPTURE_SEQUENCE.fetch_add(1, Ordering::AcqRel),
        writer,
        observed_entry_bytes,
        manpower_before,
        equipment_before,
    })
}

fn finish_loss_writer(before: LossWriterBefore) {
    let owner_ptr = match before
        .binding
        .side_ptr
        .checked_add(LOSS_OWNER_OFFSET_FROM_SIDE)
    {
        Some(owner_ptr) => owner_ptr,
        None => {
            LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let Some(manpower_after) = read_loss_manpower(owner_ptr) else {
        LOSS_DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(equipment_after) = read_loss_equipment_variants(owner_ptr) else {
        LOSS_DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if before.manpower_before == manpower_after {
        LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let mut equipment = Vec::new();
    for equipment_key_ptr in before
        .equipment_before
        .keys()
        .chain(equipment_after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let before_raw = before
            .equipment_before
            .get(&equipment_key_ptr)
            .copied()
            .unwrap_or(0);
        let after_raw = equipment_after
            .get(&equipment_key_ptr)
            .copied()
            .unwrap_or(0);
        if before_raw != after_raw {
            equipment.push(RawEquipmentLossDelta {
                equipment_id: capture_loss_equipment_id(equipment_key_ptr),
                equipment_key_ptr,
                before_bytes: before_raw.to_le_bytes(),
                after_bytes: after_raw.to_le_bytes(),
            });
        }
    }
    if equipment.is_empty() || equipment.len() > MAX_LOSS_EQUIPMENT_VARIANTS {
        LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let state_mutex = LOSS_STATE.get_or_init(|| Mutex::new(LossState::default()));
    let state = match state_mutex.try_lock() {
        Ok(state) => state,
        Err(_) if LOSS_QUEUE_SEALING.load(Ordering::Acquire) => match state_mutex.lock() {
            Ok(state) => state,
            Err(_) => {
                LOSS_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
                return;
            }
        },
        Err(_) => {
            LOSS_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    if state.sealed {
        LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if state.bindings.get(&owner_ptr) != Some(&before.binding) {
        LOSS_DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
        return;
    }
    drop(state);
    let observation = RawLossWriterObservation {
        captured_epoch: (before.binding.combat_id, before.binding.generation, before.binding.side),
        observed_game_hour_raw: crate::hour_custody::witnessed_event_hour(before.observed_hour_before, crate::hour_custody::current_game_hour_raw()),
        combat_ptr: before.binding.combat_ptr,
        side_ptr: before.binding.side_ptr,
        capture_sequence: before.capture_sequence,
        writer_rva: before.writer.writer_rva,
        observed_entry_bytes: before.observed_entry_bytes,
        manpower_before_bytes: before.manpower_before.to_le_bytes(),
        manpower_after_bytes: manpower_after.to_le_bytes(),
        equipment,
    };
    let _ = enqueue_raw_loss_observation(observation, true);
}

fn read_loss_manpower(owner_ptr: usize) -> Option<i64> {
    let address = owner_ptr.checked_add(LOSS_MANPOWER_OFFSET_FROM_OWNER)?;
    Some(i64::from(read_i32(address)?))
}

fn read_loss_equipment_variants(owner_ptr: usize) -> Option<BTreeMap<usize, i64>> {
    let vector = owner_ptr
        .checked_add(LOSS_TOTAL_EQUIPMENT_MAP_OFFSET_FROM_OWNER)?
        .checked_add(LOSS_VARIANT_VECTOR_OFFSET_FROM_MAP)?;
    let data_a = read_usize(vector)?;
    let capacity_a = read_u32(vector.checked_add(8)?)?;
    let size_a = read_u32(vector.checked_add(12)?)?;
    if size_a > capacity_a || size_a as usize > MAX_LOSS_EQUIPMENT_VARIANTS {
        return None;
    }
    if size_a == 0 {
        let data_b = read_usize(vector)?;
        let capacity_b = read_u32(vector.checked_add(8)?)?;
        let size_b = read_u32(vector.checked_add(12)?)?;
        return (data_a == data_b && capacity_a == capacity_b && size_b == 0).then(BTreeMap::new);
    }
    if data_a == 0 {
        return None;
    }
    let byte_len = (size_a as usize).checked_mul(LOSS_VARIANT_VECTOR_ENTRY_LEN)?;
    let raw = read_process_bytes(data_a, byte_len)?;
    let data_b = read_usize(vector)?;
    let capacity_b = read_u32(vector.checked_add(8)?)?;
    let size_b = read_u32(vector.checked_add(12)?)?;
    if (data_a, capacity_a, size_a) != (data_b, capacity_b, size_b) {
        return None;
    }
    let mut variants = BTreeMap::new();
    for entry in raw.chunks_exact(LOSS_VARIANT_VECTOR_ENTRY_LEN) {
        let equipment_key_ptr = u64::from_le_bytes(entry[..8].try_into().ok()?) as usize;
        let amount_raw = i64::from_le_bytes(entry[8..16].try_into().ok()?);
        if equipment_key_ptr == 0 || variants.insert(equipment_key_ptr, amount_raw).is_some() {
            return None;
        }
    }
    Some(variants)
}

unsafe extern "system" fn hook_width_producer(side_ptr: usize, out: *mut i64) -> *mut i64 {
    let active = ActiveWidthCallback::enter();
    let Some(original_addr) = WIDTH_PRODUCER_RELAY.original_for_detour() else {
        return ptr::null_mut();
    };
    let original: WidthProducerFn = unsafe { mem::transmute(original_addr) };
    if !active.combined_admitted() {
        return unsafe { original(side_ptr, out) };
    }
    let nested = WIDTH_CONTEXT.with(|slot| slot.borrow().is_some());
    if nested {
        WIDTH_DROPPED_NESTED.fetch_add(1, Ordering::Relaxed);
        let outer = WIDTH_CONTEXT.with(|slot| slot.borrow_mut().take());
        let returned = unsafe { original(side_ptr, out) };
        WIDTH_CONTEXT.with(|slot| *slot.borrow_mut() = outer);
        return returned;
    }
    let observed_hour_before = crate::hour_custody::current_game_hour_raw();
    let sequence = NEXT_CAPTURE_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    WIDTH_CONTEXT.with(|slot| *slot.borrow_mut() = Some(WidthContext::new(side_ptr, sequence)));
    let returned = unsafe { original(side_ptr, out) };
    let context = WIDTH_CONTEXT.with(|slot| slot.borrow_mut().take());
    let Some(context) = context else {
        WIDTH_DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return returned;
    };
    if context.unreadable {
        WIDTH_DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return returned;
    }
    if context.overflowed {
        WIDTH_DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        return returned;
    }
    let final_ptr = if returned.is_null() { out } else { returned };
    let Some(final_width_raw) = read_i64(final_ptr as usize) else {
        WIDTH_DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return returned;
    };
    let state_mutex = WIDTH_STATE.get_or_init(|| Mutex::new(WidthState::default()));
    let mut state = match state_mutex.try_lock() {
        Ok(state) => state,
        Err(_) if WIDTH_QUEUE_SEALING.load(Ordering::Acquire) => match state_mutex.lock() {
            Ok(state) => state,
            Err(_) => {
                WIDTH_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
                return returned;
            }
        },
        Err(_) => {
            WIDTH_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
            return returned;
        }
    };
    if state.sealed {
        WIDTH_DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return returned;
    }
    let Some(binding) = state.bindings.get(&side_ptr).cloned() else {
        WIDTH_DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
        return returned;
    };
    if state.calls.len() >= MAX_PENDING_WIDTH_CALLS {
        WIDTH_DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        return returned;
    }
    state.calls.push_back(RawWidthCallCapture {
        observed_game_hour_raw: crate::hour_custody::witnessed_event_hour(observed_hour_before, crate::hour_custody::current_game_hour_raw()),
        side_ptr: context.side_ptr,
        combat_id: binding.combat_id,
        generation: binding.generation,
        side: binding.side,
        capture_sequence: context.capture_sequence,
        final_width_raw,
        wrappers: context.wrappers,
        wrapper_count: context.wrapper_count,
        byte_gates_validated: true,
    });
    returned
}

unsafe extern "system" fn hook_division_width_leaf(
    wrapper_ptr: usize,
    out: *mut i64,
    debug_sink: usize,
) -> *mut i64 {
    let _active = ActiveWidthCallback::enter();
    let Some(original_addr) = DIVISION_WIDTH_LEAF_RELAY.original_for_detour() else {
        return ptr::null_mut();
    };
    let original: DivisionWidthLeafFn = unsafe { mem::transmute(original_addr) };
    let returned = unsafe { original(wrapper_ptr, out, debug_sink) };
    let result_ptr = if returned.is_null() { out } else { returned };
    let width_raw = read_i64(result_ptr as usize);
    WIDTH_CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(context) = slot.as_mut() else {
            return;
        };
        record_leaf_result(context, width_raw, || {
            resolve_wrapper_division_id(wrapper_ptr)
        });
    });
    returned
}

fn record_leaf_result(
    context: &mut WidthContext,
    width_raw: Option<i64>,
    division_id: impl FnOnce() -> Option<u32>,
) {
    if context.wrapper_count >= MAX_WRAPPERS_PER_CALL {
        context.overflowed = true;
        return;
    }
    let Some(width_raw) = width_raw else {
        context.unreadable = true;
        return;
    };
    context.wrappers[context.wrapper_count] = RawWrapperResult {
        division_id: division_id(),
        width_raw,
    };
    context.wrapper_count += 1;
}

unsafe extern "system" fn hook_panel_side_materializer(
    view_ptr: usize,
    side_ptr: usize,
    is_attacker: u8,
    gui_ptr: usize,
) -> usize {
    let active = ActivePanelCallback::enter();
    let Some(original_addr) = PANEL_SIDE_MATERIALIZER_RELAY.original_for_detour() else {
        return 0;
    };
    let original: PanelSideMaterializerFn = unsafe { mem::transmute(original_addr) };
    if !active.combined_admitted() {
        return unsafe { original(view_ptr, side_ptr, is_attacker, gui_ptr) };
    }
    if PANEL_CONTEXT.with(|slot| slot.borrow().is_some()) {
        PANEL_DROPPED_NESTED.fetch_add(1, Ordering::Relaxed);
        let outer = PANEL_CONTEXT.with(|slot| slot.borrow_mut().take());
        let returned = unsafe { original(view_ptr, side_ptr, is_attacker, gui_ptr) };
        PANEL_CONTEXT.with(|slot| *slot.borrow_mut() = outer);
        return returned;
    }
    let binding = {
        let state = PANEL_STATE.get_or_init(|| Mutex::new(PanelState::default()));
        let Ok(state) = state.try_lock() else {
            PANEL_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
            return unsafe { original(view_ptr, side_ptr, is_attacker, gui_ptr) };
        };
        state.bindings.get(&side_ptr).cloned()
    };
    let Some(binding) = binding else {
        PANEL_DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
        return unsafe { original(view_ptr, side_ptr, is_attacker, gui_ptr) };
    };
    let side_matches = matches!(
        (binding.side, is_attacker != 0),
        (CombatSide::Attacker, true) | (CombatSide::Defender, false)
    );
    if !side_matches {
        PANEL_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        return unsafe { original(view_ptr, side_ptr, is_attacker, gui_ptr) };
    }
    let observed_hour_before = crate::hour_custody::current_game_hour_raw();
    let capture_sequence = NEXT_PANEL_CAPTURE_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    PANEL_CONTEXT.with(|slot| {
        *slot.borrow_mut() = Some(PanelContext {
            binding,
            capture_sequence,
            modifier_bits: 0,
            next_modifier_rank: 0,
            leader_seen: false,
            leader_id: 0,
            invalid: false,
        })
    });
    let returned = unsafe { original(view_ptr, side_ptr, is_attacker, gui_ptr) };
    let Some(context) = PANEL_CONTEXT.with(|slot| slot.borrow_mut().take()) else {
        PANEL_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        return returned;
    };
    if context.invalid
        || !context.leader_seen
        || context.modifier_bits & !PANEL_MODIFIER_KNOWN_MASK != 0
    {
        PANEL_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        return returned;
    }
    let row = PanelObservationRow {
        observed_game_hour_raw: crate::hour_custody::witnessed_event_hour(observed_hour_before, crate::hour_custody::current_game_hour_raw()),
        combat_id: context.binding.combat_id,
        generation: context.binding.generation,
        side: context.binding.side,
        capture_sequence: context.capture_sequence,
        leader_id: context.leader_id,
        modifier_bits: context.modifier_bits,
        control_flags: PANEL_REQUIRED_FLAG_MASK,
    };
    let state = PANEL_STATE.get_or_init(|| Mutex::new(PanelState::default()));
    let Ok(mut state) = state.try_lock() else {
        PANEL_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
        return returned;
    };
    if state.bindings.get(&side_ptr) != Some(&context.binding) {
        PANEL_DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
    } else if state.observations.len() >= MAX_PENDING_PANEL_OBSERVATIONS {
        PANEL_DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
    } else {
        state.observations.push_back(row);
    }
    returned
}

unsafe extern "system" fn hook_panel_modifier_append(
    view_ptr: usize,
    modifier_enum: u32,
    side_ptr: usize,
    is_attacker: u8,
    list_ptr: usize,
) -> usize {
    let _active = ActivePanelCallback::enter();
    let Some(original_addr) = PANEL_MODIFIER_APPEND_RELAY.original_for_detour() else {
        return 0;
    };
    let original: PanelModifierAppendFn = unsafe { mem::transmute(original_addr) };
    let returned = unsafe { original(view_ptr, modifier_enum, side_ptr, is_attacker, list_ptr) };
    PANEL_CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(context) = slot.as_mut() else {
            return;
        };
        if context.binding.side_ptr != side_ptr || modifier_enum >= 10 {
            context.invalid = true;
            return;
        }
        let Some(rank) = PANEL_MODIFIER_CANONICAL_ORDER
            .iter()
            .position(|value| u32::from(*value) == modifier_enum)
        else {
            context.invalid = true;
            return;
        };
        let bit = 1u32 << modifier_enum;
        if rank < context.next_modifier_rank || context.modifier_bits & bit != 0 {
            context.invalid = true;
            return;
        }
        context.modifier_bits |= bit;
        context.next_modifier_rank = rank + 1;
    });
    returned
}

unsafe extern "system" fn hook_panel_leader_resolver(side_ptr: usize) -> usize {
    let _active = ActivePanelCallback::enter();
    let Some(original_addr) = PANEL_LEADER_RESOLVER_RELAY.original_for_detour() else {
        return 0;
    };
    let original: PanelLeaderResolverFn = unsafe { mem::transmute(original_addr) };
    let leader_ptr = unsafe { original(side_ptr) };
    PANEL_CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(context) = slot.as_mut() else {
            return;
        };
        if context.binding.side_ptr != side_ptr || context.leader_seen {
            context.invalid = true;
            return;
        }
        context.leader_seen = true;
        if leader_ptr == 0 {
            context.leader_id = 0;
            return;
        }
        let base = PANEL_MODULE_BASE.load(Ordering::Acquire);
        let leader_id = base
            .checked_add(ARMY_LEADER_VTABLE_RVA)
            .filter(|expected_vtable| read_usize(leader_ptr) == Some(*expected_vtable))
            .and_then(|_| leader_ptr.checked_add(ARMY_LEADER_TYPE_OFFSET))
            .filter(|address| read_u32(*address) == Some(ARMY_LEADER_TYPE_ID))
            .and_then(|_| leader_ptr.checked_add(ARMY_LEADER_ID_OFFSET))
            .and_then(read_u32)
            .filter(|id| *id > 0 && *id <= MAX_DIVISION_ID);
        match leader_id {
            Some(id) => context.leader_id = id,
            None => context.invalid = true,
        }
    });
    leader_ptr
}

fn resolve_wrapper_division_id(wrapper_ptr: usize) -> Option<u32> {
    resolve_wrapper_division_id_at_base(wrapper_ptr, MODULE_BASE.load(Ordering::Acquire))
}

fn resolve_wrapper_division_id_at_base(wrapper_ptr: usize, module_base: usize) -> Option<u32> {
    if module_base == 0
        || read_usize(wrapper_ptr)? != module_base.checked_add(DIVISION_WRAPPER_VTABLE_RVA)?
    {
        return None;
    }
    let division = wrapper_ptr.checked_add(DIVISION_BODY_DELTA)?;
    if read_usize(division)? != module_base.checked_add(DIVISION_VTABLE_RVA)? {
        return None;
    }
    let id = read_u32(division.checked_add(DIVISION_ID_OFFSET)?)?;
    (id > 0 && id <= MAX_DIVISION_ID).then_some(id)
}

fn verify_sentinel(address: usize, expected: &[u8], name: &str) -> Result<(), String> {
    let actual = read_process_bytes(address, expected.len())
        .ok_or_else(|| format!("{name} sentinel unreadable at 0x{address:X}"))?;
    if actual != expected {
        return Err(format!("{name} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn read_process_bytes(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 || len == 0 {
        return None;
    }
    let mut out = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr().cast::<c_void>(),
            len,
            &mut read,
        )
    };
    (ok != 0 && read == len).then_some(out)
}

fn read_i64(address: usize) -> Option<i64> {
    let bytes = read_process_bytes(address, 8)?;
    Some(i64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_i32(address: usize) -> Option<i32> {
    let bytes = read_process_bytes(address, 4)?;
    Some(i32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u32(address: usize) -> Option<u32> {
    let bytes = read_process_bytes(address, 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_usize(address: usize) -> Option<usize> {
    let bytes = read_process_bytes(address, mem::size_of::<usize>())?;
    Some(usize::from_le_bytes(bytes.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// singleton, the attestation singleton, the release roster) it can move a
    fn production_code() -> String {
        let source = include_str!("x2_carriers.rs").replace("\r\n", "\n");
        let production = source
            .split_once("#[cfg(test)]")
            .expect("x2_carriers.rs has a test module")
            .0
            .to_string();
        production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `observe_released (`, or `MinHook :: remove_hook` cannot slip past a
    fn relay_code() -> String {
        let source = include_str!("x2_carriers/x2_relay.rs").replace("\r\n", "\n");
        source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn strip_whitespace(source: &str) -> String {
        source.chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn production_code_dense() -> String {
        production_code()
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect()
    }

    fn body_of<'a>(production: &'a str, signature: &str) -> &'a str {
        let start = production
            .find(signature)
            .unwrap_or_else(|| panic!("{signature} is declared in production"));
        let body = &production[start..];
        let end = body
            .find("\n}\n")
            .unwrap_or_else(|| panic!("{signature}'s body is closed"));
        &body[..end]
    }

    #[test]
    fn no_x2_teardown_path_frees_a_minhook_trampoline() {
        let production = production_code();
        let dense = production_code_dense();
        for forbidden in [
            "remove_hook",
            "MH_RemoveHook",
            "RemoveHook",
            "inline_detour::remove(",
        ] {
            assert!(
                !dense.contains(forbidden),
                "an X2 path reaches {forbidden}, which frees a relay this lane retains"
            );
        }
        assert_eq!(
            production.matches("inline_detour::disable(").count(),
            1,
            "every X2 detour must be taken out through the one helper that latches the removal"
        );
        let helper = body_of(&production, "fn disable_and_retain_relay(");
        assert!(helper.contains("inline_detour::disable("), "{helper}");
        assert!(
            helper.contains("removed_by_owner.store(true, Ordering::Release)"),
            "the disable is what records owner-performed removal: {helper}"
        );
        assert_eq!(
            dense.matches("_REMOVED_BY_OWNER[index].store(true").count(),
            0,
            "no path may latch owner-performed removal outside the disable helper"
        );
    }

    #[test]
    fn a_second_install_adopts_the_retained_creation_rather_than_failing() {
        let relay = RelaySlot::new("probe");

        assert_eq!(
            resolve_created_or_adopted(Ok(0x4100), &relay).unwrap(),
            0x4100
        );
        assert_eq!(relay.recorded_original(), 0x4100);
        assert_eq!(relay.custody(), RelayCustody::Live);

        relay.retain_at_teardown();
        assert_eq!(relay.custody(), RelayCustody::Retained);
        assert_eq!(relay.recorded_original(), 0x4100);

        assert_eq!(
            resolve_created_or_adopted(Err(MH_STATUS::MH_ERROR_ALREADY_CREATED), &relay).unwrap(),
            0x4100,
            "an already-created entry must resolve to the retained trampoline"
        );
        assert_eq!(relay.custody(), RelayCustody::Live);

        let unknown = RelaySlot::new("probe");
        let error = resolve_created_or_adopted(Err(MH_STATUS::MH_ERROR_ALREADY_CREATED), &unknown)
            .expect_err("adopting an unknown trampoline must be refused");
        assert!(error.contains("retained no trampoline"), "{error}");
        assert_eq!(unknown.custody(), RelayCustody::NeverCreated);

        let null = RelaySlot::new("probe");
        assert!(resolve_created_or_adopted(Ok(0), &null)
            .expect_err("a null trampoline is not a usable original")
            .contains("null trampoline"));
        assert_eq!(null.recorded_original(), 0);
        assert_eq!(null.custody(), RelayCustody::NeverCreated);

        let moved = resolve_created_or_adopted(Ok(0x4200), &relay)
            .expect_err("a relay address may not move under a live mapping");
        assert!(moved.contains("already recorded"), "{moved}");
        assert_eq!(relay.recorded_original(), 0x4100);

        assert!(
            resolve_created_or_adopted(Err(MH_STATUS::MH_ERROR_NOT_EXECUTABLE), &relay).is_err()
        );
    }

    static RELAY_TEST_LOCK: Mutex<()> = Mutex::new(());

    static PROBE_CALLS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

    unsafe extern "system" fn probe_width_producer(_side_ptr: usize, out: *mut i64) -> *mut i64 {
        PROBE_CALLS[0].fetch_add(1, Ordering::AcqRel);
        unsafe { *out = 0x5511 };
        out
    }

    unsafe extern "system" fn probe_division_width_leaf(
        _wrapper_ptr: usize,
        out: *mut i64,
        _debug_sink: usize,
    ) -> *mut i64 {
        PROBE_CALLS[1].fetch_add(1, Ordering::AcqRel);
        unsafe { *out = 0x5512 };
        out
    }

    #[allow(clippy::too_many_arguments)]
    unsafe extern "system" fn probe_active_division_damage(
        _division_ptr: usize,
        _damage_raw: i64,
        _damage_kind: u8,
        _max_damage_raw: i64,
        _selector_ptr: usize,
        _owner_ptr: usize,
        _opposing_owner_ptr: usize,
        _attribution_ptr: usize,
    ) {
        PROBE_CALLS[2].fetch_add(1, Ordering::AcqRel);
    }

    unsafe extern "system" fn probe_participant_lifecycle(_side_ptr: usize, _wrapper_ptr: usize) {
        PROBE_CALLS[3].fetch_add(1, Ordering::AcqRel);
    }

    unsafe extern "system" fn probe_panel_side_materializer(
        _view_ptr: usize,
        _side_ptr: usize,
        _is_attacker: u8,
        _gui_ptr: usize,
    ) -> usize {
        PROBE_CALLS[4].fetch_add(1, Ordering::AcqRel);
        0x5514
    }

    unsafe extern "system" fn probe_panel_modifier_append(
        _view_ptr: usize,
        _modifier_enum: u32,
        _side_ptr: usize,
        _is_attacker: u8,
        _list_ptr: usize,
    ) -> usize {
        PROBE_CALLS[5].fetch_add(1, Ordering::AcqRel);
        0x5515
    }

    unsafe extern "system" fn probe_panel_leader_resolver(_side_ptr: usize) -> usize {
        PROBE_CALLS[6].fetch_add(1, Ordering::AcqRel);
        0x5516
    }

    fn probe_roster() -> [(&'static RelaySlot, usize); 7] {
        [
            (&WIDTH_PRODUCER_RELAY, probe_width_producer as usize),
            (
                &DIVISION_WIDTH_LEAF_RELAY,
                probe_division_width_leaf as usize,
            ),
            (
                &ACTIVE_DIVISION_DAMAGE_RELAY,
                probe_active_division_damage as usize,
            ),
            (
                &PARTICIPANT_LIFECYCLE_RELAY,
                probe_participant_lifecycle as usize,
            ),
            (
                &PANEL_SIDE_MATERIALIZER_RELAY,
                probe_panel_side_materializer as usize,
            ),
            (
                &PANEL_MODIFIER_APPEND_RELAY,
                probe_panel_modifier_append as usize,
            ),
            (
                &PANEL_LEADER_RESOLVER_RELAY,
                probe_panel_leader_resolver as usize,
            ),
        ]
    }

    fn probe_calls() -> [u64; 7] {
        std::array::from_fn(|index| PROBE_CALLS[index].load(Ordering::Acquire))
    }

    fn all_engine_original_skipped() -> u64 {
        width_engine_original_skipped_invocations()
            + loss_engine_original_skipped_invocations()
            + panel_engine_original_skipped_invocations()
    }

    fn drop_tallies() -> [&'static AtomicU64; 12] {
        [
            &WIDTH_DROPPED_LOCK_BUSY,
            &WIDTH_DROPPED_UNBOUND,
            &WIDTH_DROPPED_UNREADABLE,
            &WIDTH_DROPPED_OVERFLOW,
            &WIDTH_DROPPED_NESTED,
            &PANEL_DROPPED_LOCK_BUSY,
            &PANEL_DROPPED_UNBOUND,
            &PANEL_DROPPED_INVALID,
            &PANEL_DROPPED_OVERFLOW,
            &PANEL_DROPPED_NESTED,
            &LOSS_DROPPED_UNBOUND,
            &LOSS_DROPPED_INVALID,
        ]
    }

    fn enter_every_detour() -> ([*mut i64; 2], [usize; 3]) {
        let mut width_out = 0i64;
        let mut leaf_out = 0i64;
        let produced = unsafe { hook_width_producer(0x5500_1000, &mut width_out) };
        let leaf = unsafe { hook_division_width_leaf(0x5500_2000, &mut leaf_out, 0) };
        unsafe {
            hook_active_division_damage(0x5500_3000, 5, 0, 10, 0, 0x5500_4000, 0x5500_5000, 0)
        };
        unsafe { hook_participant_lifecycle(0x5500_6000, 0x5500_7000) };
        let materialized = unsafe { hook_panel_side_materializer(0, 0x5500_8000, 1, 0) };
        let appended = unsafe { hook_panel_modifier_append(0, 0, 0x5500_8000, 1, 0) };
        let leader = unsafe { hook_panel_leader_resolver(0x5500_8000) };
        ([produced, leaf], [materialized, appended, leader])
    }

    #[test]
    fn a_late_entrant_after_teardown_still_reaches_the_engine_function() {
        let _serial = RELAY_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tallies_before: Vec<u64> = drop_tallies()
            .iter()
            .map(|tally| tally.load(Ordering::Acquire))
            .collect();

        for (relay, _) in probe_roster() {
            assert_eq!(
                relay.custody(),
                RelayCustody::NeverCreated,
                "a production relay was already created before this test ran"
            );
        }
        let poisoned_before = all_engine_original_skipped();
        let calls_before = probe_calls();
        let (width_pointers, panel_values) = enter_every_detour();
        assert!(
            width_pointers.iter().all(|pointer| pointer.is_null()),
            "a width detour with no trampoline must not pretend to have produced a width"
        );
        assert_eq!(
            panel_values,
            [0, 0, 0],
            "a panel detour with no trampoline returns the fabricated value"
        );
        assert_eq!(
            probe_calls(),
            calls_before,
            "no engine function exists to reach yet"
        );
        assert_eq!(
            all_engine_original_skipped(),
            poisoned_before + 7,
            "each of the seven detours must count the invocation it could not pass through"
        );

        for (relay, probe) in probe_roster() {
            resolve_created_or_adopted(Ok(probe), relay).expect("a fresh create is recorded");
            assert_eq!(relay.custody(), RelayCustody::Live);
        }
        retain_width_relays_at_teardown();
        retain_loss_relays_at_teardown();
        retain_panel_relays_at_teardown();
        for (relay, probe) in probe_roster() {
            assert_eq!(
                relay.custody(),
                RelayCustody::Retained,
                "teardown must leave every relay retained, not live"
            );
            assert_eq!(
                relay.recorded_original(),
                probe,
                "teardown took the trampoline address away, so the late entrant below \
                 cannot reach the engine's own function"
            );
        }
        assert_eq!(MODULE_BASE.load(Ordering::Acquire), 0);
        assert_eq!(LOSS_MODULE_BASE.load(Ordering::Acquire), 0);
        assert_eq!(PANEL_MODULE_BASE.load(Ordering::Acquire), 0);

        let poisoned_before = all_engine_original_skipped();
        let calls_before = probe_calls();
        let (width_pointers, panel_values) = enter_every_detour();
        assert!(
            width_pointers.iter().all(|pointer| !pointer.is_null()),
            "a late width entrant returned a null to the engine instead of the engine's own result"
        );
        assert_eq!(
            panel_values,
            [0x5514, 0x5515, 0x5516],
            "a late panel entrant returned a fabricated value instead of the engine's own"
        );
        for (index, (before, after)) in calls_before.iter().zip(probe_calls()).enumerate() {
            assert_eq!(
                after,
                before + 1,
                "detour {index} did not reach the engine's own function after teardown"
            );
        }
        assert_eq!(
            all_engine_original_skipped(),
            poisoned_before,
            "a retained relay is not a poisoned one"
        );

        for (tally, before) in drop_tallies().iter().zip(tallies_before) {
            tally.store(before, Ordering::Release);
        }
    }

    #[test]
    fn the_three_relay_states_stay_distinguishable() {
        let relay = RelaySlot::new("probe");
        assert_eq!(relay.custody(), RelayCustody::NeverCreated);
        assert_eq!(relay.recorded_original(), 0);
        assert!(!relay.is_live());
        assert_eq!(relay.original_for_detour(), None);
        assert_eq!(relay.engine_original_skipped(), 1);

        relay.record_created(0x4300).expect("a create is recorded");
        assert_eq!(relay.custody(), RelayCustody::Live);
        assert!(relay.is_live(), "an owned relay is live install custody");

        relay.retain_at_teardown();
        assert_eq!(relay.custody(), RelayCustody::Retained);
        assert!(
            !relay.is_live(),
            "a completed teardown must not read as an install that did not finish"
        );
        assert_eq!(relay.original_for_detour(), Some(0x4300));
        assert_eq!(
            relay.engine_original_skipped(),
            1,
            "a retained relay must not count a poisoned invocation"
        );

        let never = RelaySlot::new("probe");
        never.retain_at_teardown();
        assert_eq!(never.custody(), RelayCustody::NeverCreated);
        assert_eq!(never.recorded_original(), 0);
    }

    #[test]
    fn no_x2_path_takes_a_relay_address_away() {
        let dense = production_code_dense();
        let production = production_code();

        assert_eq!(
            dense.matches("trampoline_cell").count(),
            0,
            "x2_carriers.rs names the relay address field; it must not be able \
             to, and if it can the module boundary has been dissolved"
        );
        assert_eq!(
            production.matches("\nmod x2_relay;\n").count(),
            1,
            "the relay module must stay a separate file"
        );

        let relay = relay_code();
        let relay_dense = strip_whitespace(&relay);
        assert_eq!(
            relay
                .matches("\n    trampoline_cell: AtomicUsize,\n")
                .count(),
            1,
            "the relay address field must be declared exactly once, private, and \
             inside RelaySlot"
        );
        for once in [
            "pub(super) struct RelaySlot {",
            "impl RelaySlot {",
            "pub(super) enum RelayCustody {",
        ] {
            assert_eq!(
                relay.matches(once).count(),
                1,
                "`{once}` must be declared exactly once, in the relay module"
            );
        }

        let admitted = [
            ("trampoline_cell:AtomicUsize,", 1, "the field declaration"),
            (
                "trampoline_cell:AtomicUsize::new(0),",
                1,
                "the one initializer",
            ),
            (
                "self.trampoline_cell.load(",
                3,
                "the detour read, the adoption read and the recorded-address accessor",
            ),
            (
                "self.trampoline_cell.compare_exchange(0,",
                1,
                "the one writer, RelaySlot::record_created",
            ),
        ];
        for (spelling, expected, why) in admitted {
            assert_eq!(
                relay_dense.matches(spelling).count(),
                expected,
                "`{spelling}` must appear exactly {expected} times -- {why}"
            );
        }
        assert_eq!(
            relay_dense.matches("trampoline_cell").count(),
            admitted
                .iter()
                .map(|(_, expected, _)| *expected)
                .sum::<usize>(),
            "the relay module names its address field somewhere this law does \
             not admit -- including by aliasing it into a local"
        );

        assert_eq!(
            dense.matches(".original_for_detour()").count(),
            7,
            "every one of the seven detours must read its relay through the counting accessor"
        );
        for (signature, relay) in [
            (
                "unsafe extern \"system\" fn hook_width_producer(",
                "WIDTH_PRODUCER_RELAY",
            ),
            (
                "unsafe extern \"system\" fn hook_division_width_leaf(",
                "DIVISION_WIDTH_LEAF_RELAY",
            ),
            (
                "unsafe extern \"system\" fn hook_active_division_damage(",
                "ACTIVE_DIVISION_DAMAGE_RELAY",
            ),
            (
                "unsafe extern \"system\" fn hook_participant_lifecycle(",
                "PARTICIPANT_LIFECYCLE_RELAY",
            ),
            (
                "unsafe extern \"system\" fn hook_panel_side_materializer(",
                "PANEL_SIDE_MATERIALIZER_RELAY",
            ),
            (
                "unsafe extern \"system\" fn hook_panel_modifier_append(",
                "PANEL_MODIFIER_APPEND_RELAY",
            ),
            (
                "unsafe extern \"system\" fn hook_panel_leader_resolver(",
                "PANEL_LEADER_RESOLVER_RELAY",
            ),
        ] {
            let body = body_of(&production, signature);
            assert!(
                body.contains(&format!("{relay}.original_for_detour()")),
                "{signature} must call its OWN relay: {body}"
            );
        }
    }

    #[test]
    fn every_teardown_and_abort_path_retains_and_reports_its_own_domain() {
        let production = production_code();
        for (domain, retain, skipped, abort, seal, commit) in [
            (
                "width",
                "retain_width_relays_at_teardown()",
                "width_engine_original_skipped_invocations()",
                "fn abort_width_install(",
                "fn seal_and_reserve_terminal_width_observer_inner(",
                "fn consume_terminal_width_observer_inner(",
            ),
            (
                "loss",
                "retain_loss_relays_at_teardown()",
                "loss_engine_original_skipped_invocations()",
                "fn abort_loss_install(",
                "fn seal_and_reserve_terminal_current_loss_observer_inner(",
                "fn consume_terminal_current_loss_observer_inner(",
            ),
            (
                "panel",
                "retain_panel_relays_at_teardown()",
                "panel_engine_original_skipped_invocations()",
                "fn abort_panel_install(",
                "fn seal_and_reserve_terminal_panel_observer_inner(",
                "fn consume_terminal_panel_observer_inner(",
            ),
        ] {
            for signature in [abort, seal] {
                let body = body_of(&production, signature);
                assert!(
                    body.contains(retain),
                    "X2 {domain}: {signature} must hand its own relays back retained"
                );
            }
            let body = body_of(&production, commit);
            assert!(
                body.contains(skipped),
                "X2 {domain}: {commit} must report an invocation that could not reach the \
                 engine as a poisoned capture"
            );
            assert!(
                body.contains(&format!("poisoned: ")),
                "X2 {domain}: {commit} must still carry a poisoned verdict"
            );
        }
    }

    #[test]
    fn every_relay_roster_consumer_asks_its_own_domain() {
        let production = production_code();
        for (signature, own, others) in [
            (
                "pub fn install_width_observer(",
                "WIDTH_RELAYS",
                ["LOSS_RELAYS", "PANEL_RELAYS"],
            ),
            (
                "pub fn install_current_loss_observer(",
                "LOSS_RELAYS",
                ["WIDTH_RELAYS", "PANEL_RELAYS"],
            ),
            (
                "pub fn install_panel_observer(",
                "PANEL_RELAYS",
                ["WIDTH_RELAYS", "LOSS_RELAYS"],
            ),
            (
                "pub fn archive_aborted_width_hook_receipts(",
                "WIDTH_RELAYS",
                ["LOSS_RELAYS", "PANEL_RELAYS"],
            ),
            (
                "pub fn archive_aborted_loss_hook_receipts(",
                "LOSS_RELAYS",
                ["WIDTH_RELAYS", "PANEL_RELAYS"],
            ),
            (
                "pub fn archive_aborted_panel_hook_receipts(",
                "PANEL_RELAYS",
                ["WIDTH_RELAYS", "LOSS_RELAYS"],
            ),
        ] {
            let body = body_of(&production, signature);
            assert!(
                body.contains(&format!("{own}.iter().any(|relay| relay.is_live())")),
                "{signature} must refuse only LIVE custody, from its own roster: {body}"
            );
            for other in others {
                assert!(
                    !body.contains(other),
                    "{signature} consults {other}, which belongs to another domain"
                );
            }
        }
        let combined = body_of(
            &production,
            "pub(crate) fn wait_for_combined_capture_quiescence(",
        );
        for roster in ["WIDTH_RELAYS", "LOSS_RELAYS", "PANEL_RELAYS"] {
            assert!(
                combined.contains(roster),
                "the X2 combined cutoff does not require {roster} in live custody"
            );
        }
    }

    #[test]
    fn every_production_relay_names_its_own_site() {
        for (relay, expected) in [
            (&WIDTH_PRODUCER_RELAY, "width producer"),
            (&DIVISION_WIDTH_LEAF_RELAY, "division width leaf"),
            (&ACTIVE_DIVISION_DAMAGE_RELAY, "active division loss"),
            (&PARTICIPANT_LIFECYCLE_RELAY, "participant-lifecycle loss"),
            (&PANEL_SIDE_MATERIALIZER_RELAY, "combat side materializer"),
            (&PANEL_MODIFIER_APPEND_RELAY, "combat modifier append"),
            (&PANEL_LEADER_RESOLVER_RELAY, "combat leader resolver"),
        ] {
            let error = resolve_created_or_adopted(Err(MH_STATUS::MH_ERROR_NOT_EXECUTABLE), relay)
                .expect_err("a failed create is an error");
            assert!(
                error.starts_with(&format!("{expected} hook creation failed")),
                "a production relay raises another site's name: {error}"
            );
        }
        let labels: BTreeSet<&str> = WIDTH_RELAYS
            .iter()
            .chain(LOSS_RELAYS.iter())
            .chain(PANEL_RELAYS.iter())
            .map(|relay| relay.label())
            .collect();
        assert_eq!(labels.len(), 7, "two X2 relays share a label");
    }

    #[test]
    fn every_x2_install_creates_through_the_adopting_helper() {
        let production = production_code();
        assert_eq!(
            production_code_dense()
                .matches("MinHook::create_hook")
                .count(),
            1,
            "every X2 site must create through create_or_adopt_hook"
        );
        let helper = body_of(&production, "fn create_or_adopt_hook(");
        assert!(helper.contains("MinHook::create_hook"), "{helper}");
        assert!(
            helper.contains("resolve_created_or_adopted("),
            "the helper must delegate the decision to the tested resolver: {helper}"
        );
    }

    #[test]
    fn every_x2_teardown_attestation_denies_a_relay_free() {
        assert!(
            !RETAINED_RELAY_TEARDOWN.relay_freed,
            "nothing here frees a relay"
        );
        assert!(RETAINED_RELAY_TEARDOWN.hook_removed);
        assert!(RETAINED_RELAY_TEARDOWN.callbacks_quiesced);
        let dense = production_code_dense();
        assert!(
            !dense.contains("relay_freed:true"),
            "an X2 teardown attests a relay free it did not perform"
        );
        assert_eq!(
            dense.matches("HookTeardownAttestation{").count(),
            1,
            "RETAINED_RELAY_TEARDOWN must be the only teardown attestation in this file"
        );
    }

    /// Falsifiable: fails if any X2 release goes back through the freeing
    /// and the release now fails instead of attesting a retention that never
    #[test]
    fn every_x2_release_mints_retained_and_consults_the_census_first() {
        let production = production_code();
        let dense = production_code_dense();
        assert!(
            !dense.contains("observe_released("),
            "an X2 release still mints through the relay-freeing route"
        );
        assert!(
            !dense.contains("observe_released_set("),
            "an X2 set release still mints through the relay-freeing route"
        );
        let mut minted_here = 0;
        for signature in [
            "fn release_recorded_width_receipts(",
            "fn release_recorded_loss_receipts(",
            "fn release_recorded_panel_receipts(",
            "fn release_complete_width_receipt_set(",
            "fn release_complete_loss_receipt_set(",
            "fn attest_and_release_retained_panel_hooks(",
        ] {
            let body = body_of(&production, signature);
            let census = body
                .find("require_retained_relay_census")
                .unwrap_or_else(|| panic!("{signature} must consult the retention census"));
            let mint = body
                .find("_with_retained_relay(")
                .unwrap_or_else(|| panic!("{signature} must mint through the retained route"));
            assert!(
                census < mint,
                "{signature} mints a retained receipt before checking the census"
            );
            minted_here += body.matches("_with_retained_relay(").count();
        }
        // a seventh release function added later would mint retained receipts
        assert_eq!(
            dense.matches("_with_retained_relay(").count(),
            minted_here,
            "a release mints a retained receipt outside the six checked functions"
        );
    }

    /// `a_release_is_refused_when_the_census_never_recorded_its_relay`. This
    #[test]
    fn every_retention_record_is_stamped_with_its_own_roster_token() {
        let production = production_code();
        for (signature, custody) in [
            (
                "fn close_width_detours_and_retain_regions(",
                "WIDTH_RECEIPT_CUSTODY",
            ),
            (
                "fn close_loss_detours_and_retain_regions(",
                "LOSS_RECEIPT_CUSTODY",
            ),
            (
                "fn close_panel_detours_and_retain_regions(",
                "PANEL_RECEIPT_CUSTODY",
            ),
        ] {
            let body = body_of(&production, signature);
            assert_eq!(
                body.matches("close_callbacks_and_retain_regions(").count(),
                1,
                "{signature} must record its retention exactly once"
            );
            assert!(
                body.contains(&format!("receipt_roster_owner_token(&{custody}")),
                "{signature} must stamp its retention with its own roster owner token"
            );
        }
    }

    #[test]
    fn a_release_is_refused_when_the_census_never_recorded_its_relay() {
        static IDLE: AtomicUsize = AtomicUsize::new(0);
        let relay = 0x5300_0000usize;
        const OWNER: u64 = 0x5300;
        let error = require_retained_relay_census("X2 census probe", relay, OWNER)
            .expect_err("a relay the census never recorded is not releasable as retained");
        assert!(
            error.contains("the retention census does not record"),
            "{error}"
        );

        crate::a729_hook::close_callbacks_and_retain_regions(
            "X2 census probe roster",
            Some(OWNER),
            &[&IDLE],
            &[(relay, relay + crate::a729_hook::MINHOOK_RELAY_LEN)],
        )
        .expect("an idle roster records its regions as retained");

        require_retained_relay_census("X2 census probe", relay, OWNER)
            .expect("the census now records exactly this relay");
        require_retained_relay_census("X2 census probe", relay + 8, OWNER)
            .expect_err("a neighbouring relay the census never saw stays refused");
        require_retained_relay_census("X2 census probe", relay, OWNER + 1)
            .expect_err("an earlier lifecycle's retention must not release a later one");
        let null_error = require_retained_relay_census("X2 census probe", 0, OWNER)
            .expect_err("a null relay address is not a retained range");
        assert!(
            null_error.contains("is not a real range"),
            "a null relay must be refused as a range, not merely missed by the census: {null_error}"
        );
        let overflow_error = require_retained_relay_census("X2 census probe", usize::MAX, OWNER)
            .expect_err("a relay whose range overflows is not attestable");
        assert!(overflow_error.contains("overflowed"), "{overflow_error}");
    }

    /// every release path into a hard error at runtime, where this test turns it
    #[test]
    fn every_production_detour_has_a_resolvable_runtime_range() {
        for (detour, label) in [
            (hook_width_producer as usize, "width producer"),
            (hook_division_width_leaf as usize, "division-width leaf"),
            (hook_active_division_damage as usize, "active-damage"),
            (hook_participant_lifecycle as usize, "participant-lifecycle"),
            (
                hook_panel_side_materializer as usize,
                "panel side-materializer",
            ),
            (hook_panel_modifier_append as usize, "panel modifier-append"),
            (hook_panel_leader_resolver as usize, "panel leader-resolver"),
        ] {
            let (start, end) = crate::a729_hook::exact_runtime_function_range(detour, label)
                .unwrap_or_else(|error| panic!("{label} detour range unresolvable: {error}"));
            assert!(
                (start..end).contains(&detour),
                "{label} detour 0x{detour:X} outside 0x{start:X}..0x{end:X}"
            );
        }
    }

    struct Resolver;

    impl LossIdentityResolver for Resolver {
        fn active_epoch(
            &self,
            combat_ptr: usize,
            side_ptr: usize,
        ) -> Option<(u32, u64, CombatSide)> {
            (combat_ptr == 0x1000 && side_ptr == 0x2000).then_some((31, 7, CombatSide::Defender))
        }

        fn equipment_id(&self, equipment_key_ptr: usize) -> Option<u32> {
            (equipment_key_ptr == 0x3000).then_some(91)
        }
    }

    fn observation(writer: AuthenticatedLossWriterSite) -> RawLossWriterObservation {
        RawLossWriterObservation {
        captured_epoch: (5, 2, CombatSide::Attacker),
        observed_game_hour_raw: None,
            combat_ptr: 0x1000,
            side_ptr: 0x2000,
            capture_sequence: 44,
            writer_rva: writer.writer_rva,
            observed_entry_bytes: writer.sentinel,
            manpower_before_bytes: 100i64.to_le_bytes(),
            manpower_after_bytes: 95i64.to_le_bytes(),
            equipment: vec![RawEquipmentLossDelta {
                equipment_id: None,
                equipment_key_ptr: 0x3000,
                before_bytes: 30i64.to_le_bytes(),
                after_bytes: 27i64.to_le_bytes(),
            }],
        }
    }

    #[test]
    fn dormant_loss_carrier_diagnostic_request_is_nonce_bound() {
        let nonce = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(WIDTH_CARRIER_ACTIVATION);
        assert!(CURRENT_LOSS_CARRIER_ACTIVATION);
        assert!(!loss_evidence_request_valid(None, None));
        assert!(!loss_evidence_request_valid(Some("1"), None));
        assert!(!loss_evidence_request_valid(Some("0"), Some(nonce)));
        assert!(!loss_evidence_request_valid(Some("1"), Some(&nonce[..63])));
        assert!(!loss_evidence_request_valid(
            Some("1"),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeF")
        ));
        assert!(loss_evidence_request_valid(Some("1"), Some(nonce)));
    }

    #[test]
    fn production_loss_projection_rejects_unbound_identity_and_sentinel_drift() {
        struct Unbound;

        impl LossIdentityResolver for Unbound {
            fn active_epoch(
                &self,
                _combat_ptr: usize,
                _side_ptr: usize,
            ) -> Option<(u32, u64, CombatSide)> {
                None
            }

            fn equipment_id(&self, _equipment_key_ptr: usize) -> Option<u32> {
                None
            }
        }

        let contract = AUTHENTICATED_LOSS_WRITER.expect("authenticated contract");
        let exact = observation(contract.active_damage);
        assert!(project_loss_observation(&exact, &Unbound)
            .expect_err("an unbound live identity must fail closed")
            .contains("current active epoch"));

        let mut drifted = exact;
        drifted.observed_entry_bytes[0] ^= 0xff;
        assert!(project_loss_observation(&drifted, &Resolver)
            .expect_err("entry-byte drift must fail closed")
            .contains("pointer, byte, or manpower-delta gate"));
    }

    #[test]
    fn pointer_erasure_preserves_exact_active_damage_source() {
        let contract = AUTHENTICATED_LOSS_WRITER.expect("authenticated contract");
        let rows = project_loss_observation(&observation(contract.active_damage), &Resolver)
            .expect("active-damage projection");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].control_flags,
            CURRENT_LOSS_REQUIRED_FLAG_MASK | CURRENT_LOSS_FLAG_SOURCE_ACTIVE_DAMAGE
        );
        assert_eq!(rows[0].equipment_id, 91);
    }

    fn reset_width_carrier() {
        for counter in [
            &WIDTH_DROPPED_LOCK_BUSY,
            &WIDTH_DROPPED_UNBOUND,
            &WIDTH_DROPPED_UNREADABLE,
            &WIDTH_DROPPED_OVERFLOW,
            &WIDTH_DROPPED_NESTED,
        ] {
            counter.store(0, Ordering::Release);
        }
        let mut state = WIDTH_STATE
            .get_or_init(|| Mutex::new(WidthState::default()))
            .lock()
            .expect("width state");
        state.calls.clear();
        state.bindings.clear();
        state.reservation = None;
        state.sealed = false;
    }

    fn queued_capture(side_ptr: usize) -> RawWidthCallCapture {
        RawWidthCallCapture {
        observed_game_hour_raw: None,
            side_ptr,
            combat_id: 5,
            generation: 2,
            side: CombatSide::Attacker,
            capture_sequence: 1,
            final_width_raw: 0,
            wrappers: [RawWrapperResult::default(); MAX_WRAPPERS_PER_CALL],
            wrapper_count: 0,
            byte_gates_validated: true,
        }
    }

    #[test]
    fn d55_hour_custody_reserved_drain_publishes_each_observed_hour() {
        reset_width_carrier();
        for (index, hour) in [100, 101].into_iter().enumerate() {
            let mut capture = queued_capture(0x4000);
            capture.observed_game_hour_raw = Some(hour);
            capture.capture_sequence = index as u64 + 1;
            capture.final_width_raw = 50;
            capture.wrapper_count = 1;
            capture.wrappers[0] = RawWrapperResult {
                division_id: Some(7),
                width_raw: 50,
            };
            WIDTH_STATE.get().unwrap().lock().unwrap().calls.push_back(capture);
        }
        let reservation = peek_width_calls(8_192).unwrap().unwrap();
        let rows = reservation.drain.calls.iter()
            .flat_map(|capture| project_width_call(capture).unwrap())
            .collect::<Vec<_>>();
        let custody = crate::hour_custody::FrameHourCustody {
            snapshot: crate::hour_custody::ReadHours { before: Some(100), after: Some(100) },
            carrier: crate::hour_custody::ReadHours { before: Some(100), after: Some(101) },
        };
        let payload = crate::encode_detail_events_payload_v10(
            1, crate::STATUS_DELTA_DATA, &[], &[], &[], &[], &[], &rows, &[], &[],
            &Default::default(), custody,
        );
        let decoded = crate::decode_detail_events_payload(0, &payload).unwrap();
        let published = crate::hour_custody::PublishedHookCustody::admit(&decoded, 100).unwrap();
        assert_eq!(published.width_observation_row_count, 2);
        assert_eq!(decoded.width_observation_rows[0].observed_game_hour_raw, Some(100));
        assert_eq!(decoded.width_observation_rows[1].observed_game_hour_raw, Some(101));
        assert_eq!(decoded.width_observation_rows, rows);
        assert_eq!(published.carrier_refusal, Some(crate::hour_custody::ReadRefusal::Torn));
        rollback_width_calls(reservation.reservation_id).unwrap();
        let retry = peek_width_calls(8_192).unwrap().unwrap();
        assert_eq!(retry.drain.calls, reservation.drain.calls);
        commit_width_calls(retry.reservation_id).unwrap();
        assert!(peek_width_calls(8_192).unwrap().is_none());
    }

    #[test]
    fn a_self_rejected_width_batch_clears_only_when_it_is_committed() {
        reset_width_carrier();
        WIDTH_DROPPED_UNBOUND.store(1, Ordering::Release);
        WIDTH_STATE
            .get()
            .expect("width state")
            .lock()
            .expect("width state")
            .calls
            .push_back(queued_capture(0x4000));

        let first = peek_width_calls(8_192)
            .expect("first peek")
            .expect("a batch to reject");
        assert_eq!(first.drain.dropped_unbound, 1);
        assert_eq!(first.drain.calls.len(), 1);
        rollback_width_calls(first.reservation_id).expect("rollback");

        let second = peek_width_calls(8_192)
            .expect("second peek")
            .expect("the same batch again");
        assert_eq!(
            second.drain.dropped_unbound, 1,
            "a rolled-back batch re-reports the same drop to the next frame"
        );
        assert_eq!(
            second.drain.calls.len(),
            1,
            "a rolled-back batch leaves its captures queued forever"
        );

        commit_width_calls(second.reservation_id).expect("commit");
        assert_eq!(WIDTH_DROPPED_UNBOUND.load(Ordering::Acquire), 0);
        assert!(WIDTH_STATE
            .get()
            .expect("width state")
            .lock()
            .expect("width state")
            .calls
            .is_empty());
        assert!(
            peek_width_calls(8_192).expect("third peek").is_none(),
            "a consumed carrier offers the next frame nothing, so it can pass the gate"
        );
        reset_width_carrier();
    }

    #[test]
    fn a_full_wrapper_buffer_and_an_unreadable_width_are_separate_faults() {
        let mut context = WidthContext::new(0x4000, 1);
        for index in 0..MAX_WRAPPERS_PER_CALL {
            record_leaf_result(&mut context, Some(20), || Some(7));
        }
        assert_eq!(context.wrapper_count, MAX_WRAPPERS_PER_CALL);
        assert!(!context.overflowed && !context.unreadable);
        record_leaf_result(&mut context, Some(20), || Some(7));
        assert!(
            context.overflowed && !context.unreadable,
            "exceeding the per-call wrapper bound is a capacity overflow"
        );

        let mut unreadable = WidthContext::new(0x4000, 2);
        record_leaf_result(&mut unreadable, None, || Some(7));
        assert!(
            unreadable.unreadable && !unreadable.overflowed,
            "a width word that could not be read is a read failure, not an overflow"
        );
        assert_eq!(unreadable.wrapper_count, 0);
    }

    #[test]
    fn a_dropped_wrapper_never_pays_for_an_identity_read() {
        let mut full = WidthContext::new(0x4000, 1);
        for index in 0..MAX_WRAPPERS_PER_CALL {
            record_leaf_result(&mut full, Some(20), || Some(7));
        }
        let mut resolved = false;
        record_leaf_result(&mut full, Some(20), || {
            resolved = true;
            Some(7)
        });
        assert!(
            !resolved,
            "a wrapper past the per-call bound is not resolved"
        );

        let mut unreadable = WidthContext::new(0x4000, 2);
        let mut resolved = false;
        record_leaf_result(&mut unreadable, None, || {
            resolved = true;
            Some(7)
        });
        assert!(
            !resolved,
            "a wrapper with no readable width is not resolved"
        );

        let mut good = WidthContext::new(0x4000, 3);
        let mut resolved = false;
        record_leaf_result(&mut good, Some(20), || {
            resolved = true;
            Some(7)
        });
        assert!(resolved, "a wrapper that is actually recorded is resolved");
        assert_eq!(good.wrappers[0].division_id, Some(7));
    }

    #[test]
    fn pointer_erasure_preserves_exact_participant_lifecycle_source() {
        let contract = AUTHENTICATED_LOSS_WRITER.expect("authenticated contract");
        let rows =
            project_loss_observation(&observation(contract.participant_lifecycle), &Resolver)
                .expect("participant-lifecycle projection");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].control_flags,
            CURRENT_LOSS_REQUIRED_FLAG_MASK | CURRENT_LOSS_FLAG_SOURCE_PARTICIPANT_LIFECYCLE
        );
    }
}


#[cfg(test)]
mod observed_identity_custody_tests {
    use super::*;
    #[test]
    fn d55_hour_custody_width_projection_never_rereads_released_wrappers() {
        let mut wrappers = [RawWrapperResult::default(); MAX_WRAPPERS_PER_CALL];
        wrappers[0] = RawWrapperResult { division_id: Some(7), width_raw: 50 };
        let capture = RawWidthCallCapture { observed_game_hour_raw: Some(100), side_ptr: 0xdead,
            combat_id: 5, generation: 2, side: CombatSide::Attacker, capture_sequence: 1,
            final_width_raw: 50, wrappers, wrapper_count: 1, byte_gates_validated: true };
        let rows = project_width_call(&capture).unwrap();
        assert_eq!(rows[0].division_id, 7);
        assert_eq!(rows[0].observed_game_hour_raw, Some(100));
        assert_eq!((rows[0].combat_id, rows[0].generation), (5, 2));
    }
    #[test]
    fn d55_hour_custody_loss_projection_uses_captured_epoch_and_equipment() {
        let writer = AUTHENTICATED_LOSS_WRITER.unwrap().active_damage;
        let observation = RawLossWriterObservation { captured_epoch: (5, 2, CombatSide::Attacker),
            observed_game_hour_raw: Some(101), combat_ptr: 0xdead, side_ptr: 0xbeef, capture_sequence: 2,
            writer_rva: writer.writer_rva, observed_entry_bytes: writer.sentinel,
            manpower_before_bytes: 100i64.to_le_bytes(), manpower_after_bytes: 90i64.to_le_bytes(),
            equipment: vec![RawEquipmentLossDelta { equipment_id: Some(91), equipment_key_ptr: 0xdeadbeef,
                before_bytes: 10i64.to_le_bytes(), after_bytes: 9i64.to_le_bytes() }] };
        let rows = project_loss_observation(&observation, &observation).unwrap();
        assert_eq!(rows[0].equipment_id, 91);
        assert_eq!(rows[0].observed_game_hour_raw, Some(101));
        assert_eq!((rows[0].combat_id, rows[0].generation), (5, 2));
    }
}
