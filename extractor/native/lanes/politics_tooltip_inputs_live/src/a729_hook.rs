//! Signature-gated a729 scheduler and authenticated source for A24.
//!
//! The hook at the exact politics-entry refresh records the engine producer
//! tuple, while the remaining detours retain pointer-free observations of the
//! exact UI/value producers. Those observations are diagnostic evidence, not
//! PTI1 rows. A separate semantic producer must submit engine-evaluated rows
//! associated with the tuple; a frame is visible through
//! [`ExactPoliticsTooltipSource`] only after that producer seals all six row
//! families, exact count agreement, and an authenticated dispatch chain.

use crate::{
    validate_frame, ExactPoliticsTooltipSource, PoliticsLineKind, PoliticsTooltipFrame,
    PoliticsTooltipInput, PoliticsSelectedPresentBinding,
};
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::{
    ReadProcessMemory, RtlCaptureStackBackTrace,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

pub const POLITICS_ENTRY_REFRESH_RVA: u32 = 0x01C3_3FE0;
pub const ADVISOR_COMPACT_COST_RVA: u32 = 0x0128_DC50;
pub const ADVISOR_FULL_TOOLTIP_RVA: u32 = 0x0128_A780;
pub const IDEOLOGY_POPULARITY_CHANGE_RVA: u32 = 0x0037_6270;
pub const TIMED_IDEA_TOOLTIP_RVA: u32 = 0x01C7_D4D0;
pub const ACTIVE_IDEA_TOOLTIP_RVA: u32 = 0x01C2_B640;
pub const POLITICAL_IDEA_TOOLTIP_RVA: u32 = 0x01C3_02E0;
pub const IDEA_CATEGORY_TOOLTIP_RVA: u32 = 0x01C2_E8C0;
pub const BOOST_PARTY_TOOLTIP_RVA: u32 = 0x01C6_AA70;
pub const IDEA_SELECTION_REFRESH_RVA: u32 = 0x01BF_E110;
pub const IDEA_COMPACT_COST_REFRESH_RVA: u32 = 0x01BF_C990;
pub const IDEA_AVAILABILITY_RVA: u32 = 0x00FB_EB00;
pub const EFFECTIVE_IDEA_COST_RVA: u32 = 0x00B9_A060;
pub const RESOURCE_AFFORDABILITY_RVA: u32 = 0x00FB_8F30;
pub const ALLOWED_TO_REMOVE_RVA: u32 = 0x00FB_EA50;
pub const IDEA_ICON_AND_MODIFIER_BIND_RVA: u32 = 0x01C3_50F0;
pub const IDEA_MODIFIER_DESCRIPTION_RVA: u32 = 0x01C3_5770;
pub const PARTY_POPULARITY_COMPUTATION_RVA: u32 = 0x00D0_5A40;
pub const TIMED_IDEA_REMAINING_DAYS_RVA: u32 = 0x00B9_C980;
pub const COUNTRY_CAN_TAKE_IDEAS_RVA: u32 = 0x00B9_8A20;
pub const BOOST_PARTY_WHY_TEXT_RVA: u32 = 0x0118_F860;
pub const BOOST_PARTY_NAME_TEXT_RVA: u32 = 0x0061_6BC0;
pub const MODIFIER_VALUE_LOOKUP_RVA: u32 = 0x0055_1820;
// Exact return address immediately after 0x14054C770 calls 0x140551820.
// Only that formatter callsite proves that lookup owner `a1` is a labelled
// modifier-source node with its authored MSVC string at +0x48.
const MODIFIER_SOURCE_LOOKUP_RETURN_RVA: u32 = 0x0054_C93B;
pub const TOOLTIP_FORMATTER_CALLSITE_A_RVA: u32 = 0x0188_963C;
pub const TOOLTIP_FORMATTER_CALLSITE_B_RVA: u32 = 0x0188_97B4;
pub const TOOLTIP_DISPATCH_CHAIN_TAIL: [u32; 5] = [
    0x0162_961E,
    0x00DB_7668,
    0x002A_4F1C,
    0x00DC_16E9,
    0x0221_A1D5,
];

/// Whole-function evidence manifest for the exact A24 producer graph.
pub const PRODUCER_MANIFEST_SHA256: [u8; 32] = [
    0x18, 0xba, 0xc1, 0x9d, 0xc5, 0xde, 0x7f, 0xc0, 0xef, 0x90, 0xa9, 0x34, 0x1e, 0x0b, 0xc5, 0xed,
    0x1a, 0xc7, 0x3c, 0x74, 0x5a, 0x1d, 0xc9, 0x6f, 0x61, 0x77, 0xed, 0x08, 0xc8, 0xf7, 0x0b, 0x35,
];

const REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
pub const REQUIRED_PRODUCER_SENTINELS: &[(u32, [u8; 16])] = &[
    (ADVISOR_COMPACT_COST_RVA, [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x50, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54]),
    (ADVISOR_FULL_TOOLTIP_RVA, [0x48, 0x8b, 0xc4, 0x44, 0x88, 0x48, 0x20, 0x44, 0x88, 0x40, 0x18, 0x48, 0x89, 0x50, 0x10, 0x55]),
    (IDEOLOGY_POPULARITY_CHANGE_RVA, [0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41]),
    (TIMED_IDEA_TOOLTIP_RVA, [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x10, 0x48, 0x89, 0x78, 0x18, 0x55]),
    (ACTIVE_IDEA_TOOLTIP_RVA, [0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57]),
    (POLITICAL_IDEA_TOOLTIP_RVA, [0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57]),
    (IDEA_CATEGORY_TOOLTIP_RVA, [0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57]),
    (BOOST_PARTY_TOOLTIP_RVA, [0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac, 0x24]),
    (IDEA_SELECTION_REFRESH_RVA, [0x44, 0x88, 0x44, 0x24, 0x18, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41]),
    (IDEA_COMPACT_COST_REFRESH_RVA, [0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x60, 0x48, 0x8b, 0xd9, 0x48, 0x83, 0x79]),
    (IDEA_AVAILABILITY_RVA, [0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x5b, 0x08, 0x49, 0x89, 0x73, 0x10, 0x57, 0x48, 0x81, 0xec, 0xf0]),
    (EFFECTIVE_IDEA_COST_RVA, [0x40, 0x57, 0x41, 0x57, 0x48, 0x83, 0xec, 0x38, 0x49, 0x8b, 0xf8, 0x4c, 0x8b, 0xfa, 0x4c, 0x8b]),
    (RESOURCE_AFFORDABILITY_RVA, [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57]),
    (ALLOWED_TO_REMOVE_RVA, [0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x81, 0xec, 0xe0, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xfa]),
    (IDEA_ICON_AND_MODIFIER_BIND_RVA, [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x60, 0x48]),
    (IDEA_MODIFIER_DESCRIPTION_RVA, [0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57]),
    (PARTY_POPULARITY_COMPUTATION_RVA, [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55]),
    (TIMED_IDEA_REMAINING_DAYS_RVA, [0x48, 0x8b, 0x41, 0x68, 0x48, 0x63, 0x49, 0x74, 0x4c, 0x8d, 0x04, 0x49, 0x4a, 0x8d, 0x0c, 0xc0]),
    (COUNTRY_CAN_TAKE_IDEAS_RVA, [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x70, 0x48]),
    (BOOST_PARTY_WHY_TEXT_RVA, [0x40, 0x53, 0x48, 0x83, 0xec, 0x30, 0x80, 0x79, 0x68, 0x00, 0x48, 0x8b, 0xda, 0x75, 0x25, 0x48]),
    (BOOST_PARTY_NAME_TEXT_RVA, [0x40, 0x53, 0x48, 0x83, 0xec, 0x30, 0x48, 0x83, 0x79, 0x30, 0x0f, 0x48, 0x8b, 0xda, 0x48, 0x8d]),
    (MODIFIER_VALUE_LOOKUP_RVA, [0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec, 0x20, 0x4c]),
];
const ALL_HOOK_RVAS: [u32; 23] = [
    POLITICS_ENTRY_REFRESH_RVA,
    ADVISOR_COMPACT_COST_RVA,
    ADVISOR_FULL_TOOLTIP_RVA,
    IDEOLOGY_POPULARITY_CHANGE_RVA,
    TIMED_IDEA_TOOLTIP_RVA,
    ACTIVE_IDEA_TOOLTIP_RVA,
    POLITICAL_IDEA_TOOLTIP_RVA,
    IDEA_CATEGORY_TOOLTIP_RVA,
    BOOST_PARTY_TOOLTIP_RVA,
    IDEA_MODIFIER_DESCRIPTION_RVA,
    IDEA_AVAILABILITY_RVA,
    EFFECTIVE_IDEA_COST_RVA,
    RESOURCE_AFFORDABILITY_RVA,
    ALLOWED_TO_REMOVE_RVA,
    PARTY_POPULARITY_COMPUTATION_RVA,
    TIMED_IDEA_REMAINING_DAYS_RVA,
    COUNTRY_CAN_TAKE_IDEAS_RVA,
    BOOST_PARTY_WHY_TEXT_RVA,
    BOOST_PARTY_NAME_TEXT_RVA,
    IDEA_SELECTION_REFRESH_RVA,
    IDEA_COMPACT_COST_REFRESH_RVA,
    IDEA_ICON_AND_MODIFIER_BIND_RVA,
    MODIFIER_VALUE_LOOKUP_RVA,
];
/// Set only after the exact detours are accompanied by a production semantic
/// row producer which calls the frame/batch/seal path below. The detours alone
/// cannot reconstruct typed reason trees, source-term lists, or complete target
/// rosters from final localized strings, so this remains false in production.
/// Unit tests exercise the downstream sealing logic through the private atomic.
pub const LIVE_PRODUCER_TAPS_COMPLETE: bool = false;
const MAX_RECENT_TUPLES: usize = 262_144;
const MAX_SEALED_FRAMES: usize = 8;

type RefreshFn = unsafe extern "system" fn(usize, usize, usize);
type TapFn1 = unsafe extern "system" fn(usize) -> usize;
type TapFn2 = unsafe extern "system" fn(usize, usize) -> usize;
type TapFn3 = unsafe extern "system" fn(usize, usize, usize) -> usize;
type TapFn4 = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
type TapFn5 = unsafe extern "system" fn(usize, usize, usize, usize, usize) -> usize;
type TapFn2U32 = unsafe extern "system" fn(usize, usize) -> u32;
type TapFn2U8 = unsafe extern "system" fn(usize, usize) -> u8;
type TapFn3U8 = unsafe extern "system" fn(usize, usize, usize) -> u8;
type TapFn3U32Arg = unsafe extern "system" fn(usize, usize, u32) -> usize;
type VoidTapFn1 = unsafe extern "system" fn(usize);
type VoidTapFn2U8 = unsafe extern "system" fn(usize, u8);
type VoidTapFn3U8 = unsafe extern "system" fn(usize, usize, u8);

static ORIGINAL_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ADVISOR_COMPACT_COST: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ADVISOR_FULL_TOOLTIP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_IDEOLOGY_POPULARITY_CHANGE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TIMED_IDEA_TOOLTIP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ACTIVE_IDEA_TOOLTIP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_POLITICAL_IDEA_TOOLTIP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_IDEA_CATEGORY_TOOLTIP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_BOOST_PARTY_TOOLTIP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_IDEA_MODIFIER_DESCRIPTION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_IDEA_AVAILABILITY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_EFFECTIVE_IDEA_COST: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_RESOURCE_AFFORDABILITY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ALLOWED_TO_REMOVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PARTY_POPULARITY_COMPUTATION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TIMED_IDEA_REMAINING_DAYS: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_COUNTRY_CAN_TAKE_IDEAS: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_BOOST_PARTY_WHY_TEXT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_BOOST_PARTY_NAME_TEXT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_IDEA_SELECTION_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_IDEA_COMPACT_COST_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_IDEA_ICON_AND_MODIFIER_BIND: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MODIFIER_VALUE_LOOKUP: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static TEARING_DOWN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
// The complete evidence detour graph is independently installable, but static
// implementation is not live semantic evidence. Keep source admission disabled
// until the selected-Present fixture proves every PTI1 family and visible-roster
// completeness; caller-supplied `DispatchIdentity` values are never authority.
static FULL_PRODUCER_GRAPH_INSTALLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static TAP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static TAP_INVOCATION: AtomicU64 = AtomicU64::new(1);
static TAP_OBSERVATIONS: OnceLock<Mutex<VecDeque<A24ProducerTapObservation>>> = OnceLock::new();
static CAPTURE_EPOCH: OnceLock<Mutex<Option<A24CaptureEpoch>>> = OnceLock::new();
thread_local! {
    static TAP_STACK: RefCell<Vec<(u64, A24ProducerTapKind, u32, bool)>> = const { RefCell::new(Vec::new()) };
}

const CALLBACK_QUIESCE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
struct A24CaptureEpoch {
    frame_hour: u64,
    arm_qpc_ticks: u64,
}

fn full_producer_graph_installed() -> bool {
    #[cfg(test)]
    {
        FULL_PRODUCER_GRAPH_INSTALLED.load(Ordering::Acquire)
    }
    #[cfg(not(test))]
    {
        LIVE_PRODUCER_TAPS_COMPLETE && FULL_PRODUCER_GRAPH_INSTALLED.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum A24ProducerTapKind {
    PoliticsEntryRefresh,
    AdvisorCompactCost,
    AdvisorFullTooltip,
    IdeologyPopularityChange,
    TimedIdeaTooltip,
    ActiveIdeaTooltip,
    PoliticalIdeaTooltip,
    IdeaCategoryTooltip,
    BoostPartyTooltip,
    IdeaModifierDescription,
    IdeaAvailability,
    EffectiveIdeaCost,
    ResourceAffordability,
    AllowedToRemove,
    PartyPopularityComputation,
    TimedIdeaRemainingDays,
    CountryCanTakeIdeas,
    BoostPartyWhyText,
    BoostPartyNameText,
    IdeaSelectionRefresh,
    IdeaCompactCostRefresh,
    IdeaIconAndModifierBind,
    ModifierValueLookup,
}

impl A24ProducerTapKind {
    pub const fn rva(self) -> u32 {
        match self {
            Self::PoliticsEntryRefresh => POLITICS_ENTRY_REFRESH_RVA,
            Self::AdvisorCompactCost => ADVISOR_COMPACT_COST_RVA,
            Self::AdvisorFullTooltip => ADVISOR_FULL_TOOLTIP_RVA,
            Self::IdeologyPopularityChange => IDEOLOGY_POPULARITY_CHANGE_RVA,
            Self::TimedIdeaTooltip => TIMED_IDEA_TOOLTIP_RVA,
            Self::ActiveIdeaTooltip => ACTIVE_IDEA_TOOLTIP_RVA,
            Self::PoliticalIdeaTooltip => POLITICAL_IDEA_TOOLTIP_RVA,
            Self::IdeaCategoryTooltip => IDEA_CATEGORY_TOOLTIP_RVA,
            Self::BoostPartyTooltip => BOOST_PARTY_TOOLTIP_RVA,
            Self::IdeaModifierDescription => IDEA_MODIFIER_DESCRIPTION_RVA,
            Self::IdeaAvailability => IDEA_AVAILABILITY_RVA,
            Self::EffectiveIdeaCost => EFFECTIVE_IDEA_COST_RVA,
            Self::ResourceAffordability => RESOURCE_AFFORDABILITY_RVA,
            Self::AllowedToRemove => ALLOWED_TO_REMOVE_RVA,
            Self::PartyPopularityComputation => PARTY_POPULARITY_COMPUTATION_RVA,
            Self::TimedIdeaRemainingDays => TIMED_IDEA_REMAINING_DAYS_RVA,
            Self::CountryCanTakeIdeas => COUNTRY_CAN_TAKE_IDEAS_RVA,
            Self::BoostPartyWhyText => BOOST_PARTY_WHY_TEXT_RVA,
            Self::BoostPartyNameText => BOOST_PARTY_NAME_TEXT_RVA,
            Self::IdeaSelectionRefresh => IDEA_SELECTION_REFRESH_RVA,
            Self::IdeaCompactCostRefresh => IDEA_COMPACT_COST_REFRESH_RVA,
            Self::IdeaIconAndModifierBind => IDEA_ICON_AND_MODIFIER_BIND_RVA,
            Self::ModifierValueLookup => MODIFIER_VALUE_LOOKUP_RVA,
        }
    }

    const fn is_capture_root(self) -> bool {
        matches!(
            self,
            Self::PoliticsEntryRefresh
                | Self::AdvisorCompactCost
                | Self::AdvisorFullTooltip
                | Self::IdeologyPopularityChange
                | Self::TimedIdeaTooltip
                | Self::ActiveIdeaTooltip
                | Self::PoliticalIdeaTooltip
                | Self::IdeaCategoryTooltip
                | Self::BoostPartyTooltip
                | Self::IdeaModifierDescription
                | Self::IdeaSelectionRefresh
                | Self::IdeaCompactCostRefresh
                | Self::IdeaIconAndModifierBind
        )
    }
}

/// Pointer-free entry/return observation. These rows are evidence for the
/// owner-runnable fixture regrader; they are not PTI1 semantic rows by
/// themselves and cannot activate d73.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct A24ProducerTapObservation {
    pub sequence: u64,
    pub invocation_id: u64,
    pub parent_invocation_id: Option<u64>,
    pub thread_id: u32,
    pub tap_kind: A24ProducerTapKind,
    pub tap_rva: u32,
    pub frame_hour: u64,
    pub entry_qpc_ticks: u64,
    pub return_qpc_ticks: u64,
    pub stack_rvas: Vec<u32>,
    pub engine_target_identity: Option<String>,
    pub resolved_text: Option<String>,
    pub raw_value: Option<i64>,
    pub predicate_result: Option<bool>,
}

#[derive(serde::Serialize)]
struct A24ProducerCaptureReceipt {
    schema: &'static str,
    status: &'static str,
    executable_sha256: &'static str,
    producer_manifest_sha256: &'static str,
    helper_manifest_sha256: &'static str,
    internal_producer_manifest_sha256: &'static str,
    sentinel_artifact_sha256: &'static str,
    modifier_sentinel_artifact_sha256: &'static str,
    internal_sentinel_artifact_sha256: &'static str,
    frame_hour: u64,
    arm_qpc_ticks: u64,
    seal_qpc_ticks: u64,
    country_tag: String,
    selected_present: PoliticsSelectedPresentBinding,
    capture_case_identity: String,
    completeness_record_identity: String,
    observed_tap_kinds: Vec<A24ProducerTapKind>,
    observations: Vec<A24ProducerTapObservation>,
}

struct OpenTap {
    invocation_id: u64,
    parent_invocation_id: Option<u64>,
    thread_id: u32,
    kind: A24ProducerTapKind,
    frame_hour: Option<u64>,
    entry_qpc_ticks: Option<u64>,
    stack_rvas: Vec<u32>,
    engine_target_identity: Option<String>,
}

const MAX_TAP_OBSERVATIONS: usize = 262_144;

struct CallbackGuard;

impl CallbackGuard {
    fn enter() -> Self {
        IN_FLIGHT_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        IN_FLIGHT_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Open one evidence-only A24 acquisition epoch before the scripted UI
/// choreography begins. Producer observations outside an armed epoch are
/// deliberately ignored so unrelated background tooltip work cannot be
/// relabelled after the fact. The selected-Present receipt is joined only by
/// [`capture_a24_producer_receipt`] after every retained return precedes the
/// selected Present.
pub fn arm_a24_capture_epoch(frame_hour: u64) -> Result<u64, String> {
    if !INSTALLED.load(Ordering::Acquire) || TEARING_DOWN.load(Ordering::Acquire) {
        return Err("A24 producer hooks are not installed and live".into());
    }
    let arm_qpc_ticks = query_performance_counter()
        .ok_or("QueryPerformanceCounter failed while arming A24 capture")?;
    if IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) != 0
        || TAP_STACK.with(|stack| !stack.borrow().is_empty())
    {
        return Err("A24 capture cannot arm while a producer callback is active".into());
    }
    let mut epoch = CAPTURE_EPOCH
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "A24 capture epoch mutex poisoned".to_string())?;
    if epoch.is_some() {
        return Err("A24 capture epoch is already armed".into());
    }
    if let Ok(mut rows) = TAP_OBSERVATIONS
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
    {
        rows.clear();
    } else {
        return Err("A24 observation queue mutex poisoned".into());
    }
    TAP_STACK.with(|stack| stack.borrow_mut().clear());
    {
        let mut state = lock_state()?;
        state.rejected_tuple_count = 0;
        state.rejected_batch_count = 0;
        state.rejected_seal_count = 0;
        state.evicted_tuple_count = 0;
    }
    *epoch = Some(A24CaptureEpoch {
        frame_hour,
        arm_qpc_ticks,
    });
    Ok(arm_qpc_ticks)
}

pub fn cancel_a24_capture_epoch() {
    if let Some(epoch) = CAPTURE_EPOCH.get() {
        if let Ok(mut epoch) = epoch.lock() {
            *epoch = None;
        }
    }
    if let Some(rows) = TAP_OBSERVATIONS.get() {
        if let Ok(mut rows) = rows.lock() {
            rows.clear();
        }
    }
    TAP_STACK.with(|stack| stack.borrow_mut().clear());
}

/// Drain pointer-free exact producer observations for an owner-runnable
/// capture receipt. A caller cannot submit or amend observations.
pub fn drain_a24_producer_tap_observations() -> Vec<A24ProducerTapObservation> {
    let Ok(mut rows) = TAP_OBSERVATIONS
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
    else {
        return Vec::new();
    };
    rows.drain(..).collect()
}

/// Build the owner-runnable, pointer-free producer receipt used to decide
/// whether the compile-time d73 activation latch may be reviewed. This never
/// changes the latch and never converts observations into semantic PTI1 rows.
pub fn capture_a24_producer_receipt(
    frame_hour: u64,
    country_tag: String,
    selected_present: PoliticsSelectedPresentBinding,
) -> Result<Vec<u8>, String> {
    if country_tag.len() != 3
        || !country_tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        || selected_present.candidate_id == 0
        || selected_present.qpc_ticks == 0
        || selected_present.cutoff_qpc_ticks < selected_present.qpc_ticks
        || selected_present.receipt_sha256.len() != 64
        || !selected_present
            .receipt_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("A24 receipt frame/country/selected-Present identity invalid".into());
    }
    let epoch = CAPTURE_EPOCH
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "A24 capture epoch mutex poisoned".to_string())?
        .take()
        .ok_or("A24 capture receipt requested without an armed epoch")?;
    if epoch.frame_hour != frame_hour {
        cancel_a24_capture_epoch();
        return Err("A24 capture epoch hour differs from receipt hour".into());
    }
    let seal_qpc_ticks = query_performance_counter()
        .ok_or("QueryPerformanceCounter failed while sealing A24 capture")?;
    let observations = drain_a24_producer_tap_observations();
    let observed = observations
        .iter()
        .map(|row| row.tap_kind)
        .collect::<BTreeSet<_>>();
    if observed.is_empty()
        || !observed.iter().any(|kind| kind.is_capture_root())
        || observations.iter().any(|row| {
            row.thread_id == 0
                || row.stack_rvas.is_empty()
                || row.tap_rva != row.tap_kind.rva()
                || row.frame_hour != frame_hour
                || row.entry_qpc_ticks < epoch.arm_qpc_ticks
                || row.return_qpc_ticks < row.entry_qpc_ticks
                || row.return_qpc_ticks > selected_present.qpc_ticks
                || row.return_qpc_ticks > seal_qpc_ticks
                || row.stack_rvas.iter().any(|rva| *rva >= 0x0300_0000)
        })
    {
        return Err("A24 capture case has no rooted producer observation or its contemporaneous frame binding is invalid".into());
    }
    let mut sequences = BTreeSet::new();
    let mut invocations = BTreeSet::new();
    if observations.iter().any(|row| {
        !sequences.insert(row.sequence)
            || !invocations.insert((row.thread_id, row.invocation_id))
    }) {
        return Err("A24 producer receipt contains duplicate sequence/invocation identity".into());
    }
    let invocation_keys = observations
        .iter()
        .map(|row| (row.thread_id, row.invocation_id))
        .collect::<BTreeSet<_>>();
    if observations.iter().any(|row| {
        (!row.tap_kind.is_capture_root() && row.parent_invocation_id.is_none())
            || row.parent_invocation_id.is_some_and(|parent| {
                parent >= row.invocation_id
                    || !invocation_keys.contains(&(row.thread_id, parent))
            })
    }) {
        return Err("A24 producer receipt contains an unjoined native-stack parent".into());
    }
    if IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) != 0
        || TAP_STACK.with(|stack| !stack.borrow().is_empty())
    {
        return Err("A24 producer receipt has an unpaired entry/return invocation".into());
    }
    let diagnostics = A729PoliticsHookSource.diagnostics();
    if diagnostics.rejected_tuple_count != 0
        || diagnostics.rejected_batch_count != 0
        || diagnostics.rejected_seal_count != 0
        || diagnostics.evicted_tuple_count != 0
    {
        return Err("A24 producer capture diagnostics are not clean".into());
    }
    let observed_tap_kinds = observed.into_iter().collect::<Vec<_>>();
    let capture_case_identity = format!(
        "a24:a729:frame={frame_hour}:arm={}:present={}",
        epoch.arm_qpc_ticks, selected_present.candidate_id
    );
    let completeness_record_identity = format!(
        "a24:a729:frame={frame_hour}:present={}:case_taps={}:observations={}",
        selected_present.candidate_id,
        observed_tap_kinds.len(),
        observations.len()
    );
    serde_json::to_vec_pretty(&A24ProducerCaptureReceipt {
        schema: "hoi4-a24-producer-capture-case-v3",
        status: "complete_pointer_free_exact_a729_capture_case",
        executable_sha256:
            "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        producer_manifest_sha256:
            "18bac19dc5de7fc0ef90a9341e0bc5ed1ac73c745a1dc96f6177ed08c8f70b35",
        helper_manifest_sha256:
            "989783832d3fe63a4f87bef2e2ba4e3c34c2048f11e87d55afb15164073e1503",
        internal_producer_manifest_sha256:
            "ea5a7659b80b58cfc3f7e41d518da67b1d9114d9f966a66fe7c069a463f79eb1",
        sentinel_artifact_sha256:
            "1bcf5829cd3a6af2ca1720ee26f63285ec374fdbc609adb27152769122203270",
        modifier_sentinel_artifact_sha256:
            "7f78ebae425582b1be1ab5ae9b3cd7f0d2c7a174cd4d526d0188b22f5d4e34fa",
        internal_sentinel_artifact_sha256:
            "e354be21660d67e8589f3ff8e8ad98c8eb7b5d40859a67102449ce05754d00ab",
        frame_hour,
        arm_qpc_ticks: epoch.arm_qpc_ticks,
        seal_qpc_ticks,
        country_tag,
        selected_present,
        capture_case_identity,
        completeness_record_identity,
        observed_tap_kinds,
        observations,
    })
    .map_err(|error| format!("A24 producer receipt serialization failed: {error}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PoliticsProducerTuple {
    pub entry: usize,
    pub idea_id: u32,
    pub country_context: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DispatchIdentity {
    pub frames: [u32; 6],
}

impl DispatchIdentity {
    pub const fn callsite_a() -> Self {
        Self {
            frames: [
                TOOLTIP_FORMATTER_CALLSITE_A_RVA,
                TOOLTIP_DISPATCH_CHAIN_TAIL[0],
                TOOLTIP_DISPATCH_CHAIN_TAIL[1],
                TOOLTIP_DISPATCH_CHAIN_TAIL[2],
                TOOLTIP_DISPATCH_CHAIN_TAIL[3],
                TOOLTIP_DISPATCH_CHAIN_TAIL[4],
            ],
        }
    }

    pub const fn callsite_b() -> Self {
        Self {
            frames: [
                TOOLTIP_FORMATTER_CALLSITE_B_RVA,
                TOOLTIP_DISPATCH_CHAIN_TAIL[0],
                TOOLTIP_DISPATCH_CHAIN_TAIL[1],
                TOOLTIP_DISPATCH_CHAIN_TAIL[2],
                TOOLTIP_DISPATCH_CHAIN_TAIL[3],
                TOOLTIP_DISPATCH_CHAIN_TAIL[4],
            ],
        }
    }

    fn authenticated(self) -> bool {
        self == Self::callsite_a() || self == Self::callsite_b()
    }
}

#[derive(Clone, Debug)]
struct ExactPoliticsProducerBatch {
    pub frame_hour: u64,
    pub producer_tuple: PoliticsProducerTuple,
    pub dispatch_identity: DispatchIdentity,
    pub rows: Vec<PoliticsTooltipInput>,
}

#[derive(Clone, Debug)]
struct OpenFrame {
    completeness_record_identity: String,
    selected_present: PoliticsSelectedPresentBinding,
    expected_producer_tuple_count: u32,
    batches: BTreeMap<PoliticsProducerTuple, Vec<PoliticsTooltipInput>>,
    dispatches: BTreeSet<[u32; 6]>,
}

#[derive(Clone, Debug)]
struct SealedFrame {
    completeness_record_identity: String,
    selected_present: PoliticsSelectedPresentBinding,
    rows: Vec<PoliticsTooltipInput>,
}

#[derive(Default)]
struct HookState {
    recent_completed_tuples: VecDeque<PoliticsProducerTuple>,
    open_frames: BTreeMap<u64, OpenFrame>,
    sealed_frames: BTreeMap<u64, SealedFrame>,
    observed_refresh_count: u64,
    rejected_tuple_count: u64,
    rejected_batch_count: u64,
    rejected_seal_count: u64,
    evicted_tuple_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub observed_refresh_count: u64,
    pub rejected_tuple_count: u64,
    pub rejected_batch_count: u64,
    pub rejected_seal_count: u64,
    pub evicted_tuple_count: u64,
    pub open_frame_count: u64,
    pub sealed_frame_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddress {
    pub politics_entry_refresh: usize,
}

pub struct A729PoliticsHookSource;

impl A729PoliticsHookSource {
    pub fn diagnostics(&self) -> HookDiagnostics {
        let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
            return HookDiagnostics::default();
        };
        HookDiagnostics {
            observed_refresh_count: state.observed_refresh_count,
            rejected_tuple_count: state.rejected_tuple_count,
            rejected_batch_count: state.rejected_batch_count,
            rejected_seal_count: state.rejected_seal_count,
            evicted_tuple_count: state.evicted_tuple_count,
            open_frame_count: state.open_frames.len() as u64,
            sealed_frame_count: state.sealed_frames.len() as u64,
        }
    }

    fn sealed(&self, frame_hour: u64) -> Result<SealedFrame, String> {
        let state = STATE
            .get()
            .ok_or_else(|| "a729 politics hook state is not initialized".to_string())?
            .lock()
            .map_err(|_| "a729 politics hook state mutex poisoned".to_string())?;
        if state.rejected_tuple_count != 0
            || state.rejected_batch_count != 0
            || state.rejected_seal_count != 0
            || state.evicted_tuple_count != 0
        {
            return Err(format!(
                "a729 politics completeness violated: tuple_rejects={} batch_rejects={} seal_rejects={} tuple_evictions={}",
                state.rejected_tuple_count,
                state.rejected_batch_count,
                state.rejected_seal_count,
                state.evicted_tuple_count
            ));
        }
        state
            .sealed_frames
            .get(&frame_hour)
            .cloned()
            .ok_or_else(|| format!("a729 politics frame {frame_hour} is not sealed"))
    }
}

impl ExactPoliticsTooltipSource for A729PoliticsHookSource {
    fn is_exact_a729_politics_source(&self) -> bool {
        INSTALLED.load(Ordering::Acquire)
            && full_producer_graph_installed()
    }

    fn producer_manifest_sha256(&self) -> [u8; 32] {
        PRODUCER_MANIFEST_SHA256
    }

    fn selected_present_binding(
        &self,
        frame_hour: u64,
    ) -> Result<PoliticsSelectedPresentBinding, String> {
        Ok(self.sealed(frame_hour)?.selected_present)
    }

    fn completeness_record_identity(&self, frame_hour: u64) -> Result<String, String> {
        Ok(self.sealed(frame_hour)?.completeness_record_identity)
    }

    fn expected_row_count(&self, frame_hour: u64) -> Result<u32, String> {
        u32::try_from(self.sealed(frame_hour)?.rows.len())
            .map_err(|_| "a729 politics row count exceeds u32".to_string())
    }

    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<PoliticsTooltipInput>, String> {
        Ok(self.sealed(frame_hour)?.rows)
    }
}

/// Open the frame-level completeness record before exact producer batches are
/// submitted. Reopening or replacing a frame is rejected.
fn begin_exact_politics_frame(
    frame_hour: u64,
    selected_present: PoliticsSelectedPresentBinding,
    completeness_record_identity: String,
    expected_producer_tuple_count: u32,
) -> Result<(), String> {
    if completeness_record_identity.is_empty() || expected_producer_tuple_count == 0 {
        return reject_seal("empty completeness identity or zero expected producer tuples");
    }
    let mut state = lock_state()?;
    if state.open_frames.contains_key(&frame_hour) || state.sealed_frames.contains_key(&frame_hour)
    {
        state.rejected_seal_count = state.rejected_seal_count.saturating_add(1);
        return Err(format!("a729 politics frame {frame_hour} already exists"));
    }
    state.open_frames.insert(
        frame_hour,
        OpenFrame {
            completeness_record_identity,
            selected_present,
            expected_producer_tuple_count,
            batches: BTreeMap::new(),
            dispatches: BTreeSet::new(),
        },
    );
    Ok(())
}

/// Admit one exact engine-evaluated producer batch. The tuple must have been
/// observed at the signature-gated refresh hook and is consumed exactly once.
fn submit_exact_politics_batch(batch: ExactPoliticsProducerBatch) -> Result<(), String> {
    if !batch.dispatch_identity.authenticated() {
        return reject_batch("dispatch identity is not one of the retained authenticated chains");
    }
    if batch.producer_tuple.entry == 0
        || batch.producer_tuple.idea_id == 0
        || batch.producer_tuple.country_context == 0
    {
        return reject_batch("producer tuple contains a null/zero identity component");
    }
    let mut state = lock_state()?;
    let Some(position) = state
        .recent_completed_tuples
        .iter()
        .position(|tuple| *tuple == batch.producer_tuple)
    else {
        state.rejected_batch_count = state.rejected_batch_count.saturating_add(1);
        return Err(
            "producer tuple was not observed at 0x141C33FE0 or was already consumed".into(),
        );
    };
    let mut probe_rows = batch.rows.clone();
    probe_rows.sort_by(row_order);
    let probe = PoliticsTooltipFrame {
        elapsed_wall_ms: 0,
        status_flags: 0,
        frame_hour: batch.frame_hour,
        selected_present_candidate_id: 1,
        selected_present_ordinal: 0,
        selected_present_qpc_ticks: 1,
        selected_present_cutoff_qpc_ticks: 1,
        selected_present_receipt_sha256: "0".repeat(64),
        producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
        completeness_record_identity: "a24:batch-probe".into(),
        expected_row_count: probe_rows.len() as u32,
        rows: probe_rows,
    };
    if let Err(error) = validate_frame(&probe) {
        state.rejected_batch_count = state.rejected_batch_count.saturating_add(1);
        return Err(format!("producer batch failed PTI1 validation: {error:?}"));
    }
    if !state.open_frames.contains_key(&batch.frame_hour) {
        state.rejected_batch_count = state.rejected_batch_count.saturating_add(1);
        return Err(format!(
            "a729 politics frame {} is not open",
            batch.frame_hour
        ));
    }
    if state
        .open_frames
        .get(&batch.frame_hour)
        .is_some_and(|frame| frame.batches.contains_key(&batch.producer_tuple))
    {
        state.rejected_batch_count = state.rejected_batch_count.saturating_add(1);
        return Err("duplicate producer tuple in frame".into());
    }
    state.recent_completed_tuples.remove(position);
    let frame = state
        .open_frames
        .get_mut(&batch.frame_hour)
        .expect("checked");
    frame.dispatches.insert(batch.dispatch_identity.frames);
    frame.batches.insert(batch.producer_tuple, batch.rows);
    Ok(())
}

/// Seal a complete frame. Every required semantic family must be represented;
/// an omitted family is not converted into an empty or unavailable value.
fn seal_exact_politics_frame(frame_hour: u64) -> Result<(), String> {
    let mut state = lock_state()?;
    let Some(frame) = state.open_frames.remove(&frame_hour) else {
        state.rejected_seal_count = state.rejected_seal_count.saturating_add(1);
        return Err(format!("a729 politics frame {frame_hour} is not open"));
    };
    if frame.batches.len() != frame.expected_producer_tuple_count as usize {
        state.rejected_seal_count = state.rejected_seal_count.saturating_add(1);
        return Err(format!(
            "producer tuple completeness mismatch: expected {} observed {}",
            frame.expected_producer_tuple_count,
            frame.batches.len()
        ));
    }
    if frame.dispatches.is_empty() {
        state.rejected_seal_count = state.rejected_seal_count.saturating_add(1);
        return Err("no authenticated dispatch identity was observed".into());
    }
    let mut rows = frame.batches.into_values().flatten().collect::<Vec<_>>();
    rows.sort_by(row_order);
    let observed = rows
        .iter()
        .map(|row| row.line_kind)
        .collect::<BTreeSet<_>>();
    let required = required_line_kinds();
    if observed != required {
        state.rejected_seal_count = state.rejected_seal_count.saturating_add(1);
        return Err(format!(
            "required politics families incomplete: observed={} required={}",
            observed.len(),
            required.len()
        ));
    }
    let probe = PoliticsTooltipFrame {
        elapsed_wall_ms: 0,
        status_flags: 0,
        frame_hour,
        selected_present_candidate_id: frame.selected_present.candidate_id,
        selected_present_ordinal: frame.selected_present.ordinal,
        selected_present_qpc_ticks: frame.selected_present.qpc_ticks,
        selected_present_cutoff_qpc_ticks: frame.selected_present.cutoff_qpc_ticks,
        selected_present_receipt_sha256: frame.selected_present.receipt_sha256.clone(),
        producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
        completeness_record_identity: frame.completeness_record_identity.clone(),
        expected_row_count: rows.len() as u32,
        rows: rows.clone(),
    };
    if let Err(error) = validate_frame(&probe) {
        state.rejected_seal_count = state.rejected_seal_count.saturating_add(1);
        return Err(format!(
            "sealed producer frame failed PTI1 validation: {error:?}"
        ));
    }
    state.sealed_frames.insert(
        frame_hour,
        SealedFrame {
            completeness_record_identity: frame.completeness_record_identity,
            selected_present: frame.selected_present,
            rows,
        },
    );
    while state.sealed_frames.len() > MAX_SEALED_FRAMES {
        let oldest = *state.sealed_frames.keys().next().expect("non-empty");
        state.sealed_frames.remove(&oldest);
    }
    Ok(())
}

fn begin_tap(kind: A24ProducerTapKind, owner: usize) -> OpenTap {
    let invocation_id = TAP_INVOCATION.fetch_add(1, Ordering::Relaxed);
    let thread_id = unsafe { GetCurrentThreadId() };
    let epoch = CAPTURE_EPOCH
        .get()
        .and_then(|epoch| epoch.lock().ok())
        .and_then(|epoch| epoch.as_ref().copied())
        .filter(|_| !TEARING_DOWN.load(Ordering::Acquire));
    let (parent_invocation_id, recording) = TAP_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        let parent = stack.iter().rev().find(|entry| entry.3).map(|entry| entry.0);
        let recording = epoch.is_some() && (kind.is_capture_root() || parent.is_some());
        stack.push((invocation_id, kind, thread_id, recording));
        (parent, recording)
    });
    OpenTap {
        invocation_id,
        parent_invocation_id,
        thread_id,
        kind,
        frame_hour: recording.then_some(epoch.expect("recording requires epoch").frame_hour),
        entry_qpc_ticks: recording.then(query_performance_counter).flatten(),
        stack_rvas: recording.then(capture_stack_rvas).unwrap_or_default(),
        engine_target_identity: recording
            .then(|| capture_engine_target_identity(kind, owner))
            .flatten(),
    }
}

fn finish_tap(
    open: OpenTap,
    output_string: Option<usize>,
    raw_value: Option<i64>,
    predicate_result: Option<bool>,
) {
    let return_qpc_ticks = query_performance_counter();
    let thread_id = unsafe { GetCurrentThreadId() };
    let paired = TAP_STACK.with(|stack| {
        matches!(
            stack.borrow_mut().pop(),
            Some((invocation_id, kind, entry_thread, _))
                if invocation_id == open.invocation_id
                    && kind == open.kind
                    && entry_thread == open.thread_id
                    && thread_id == open.thread_id
        )
    });
    if !paired {
        reject_tuple();
        return;
    }
    let (Some(frame_hour), Some(entry_qpc_ticks), Some(return_qpc_ticks)) =
        (open.frame_hour, open.entry_qpc_ticks, return_qpc_ticks)
    else {
        return;
    };
    let observation = A24ProducerTapObservation {
        sequence: TAP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        invocation_id: open.invocation_id,
        parent_invocation_id: open.parent_invocation_id,
        thread_id,
        tap_kind: open.kind,
        tap_rva: open.kind.rva(),
        frame_hour,
        entry_qpc_ticks,
        return_qpc_ticks,
        stack_rvas: open.stack_rvas,
        engine_target_identity: open.engine_target_identity,
        resolved_text: output_string.and_then(read_engine_string),
        raw_value,
        predicate_result,
    };
    let Ok(mut rows) = TAP_OBSERVATIONS
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
    else {
        reject_tuple();
        return;
    };
    if rows.len() >= MAX_TAP_OBSERVATIONS {
        rows.pop_front();
        if let Ok(mut state) = lock_state() {
            state.evicted_tuple_count = state.evicted_tuple_count.saturating_add(1);
        }
    }
    rows.push_back(observation);
}

fn capture_stack_rvas() -> Vec<u32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Vec::new();
    }
    let mut frames = [ptr::null_mut::<c_void>(); 16];
    let count = unsafe {
        RtlCaptureStackBackTrace(
            0,
            frames.len() as u32,
            frames.as_mut_ptr(),
            ptr::null_mut(),
        )
    } as usize;
    frames[..count.min(frames.len())]
        .iter()
        .filter_map(|frame| (*frame as usize).checked_sub(base))
        .filter_map(|rva| u32::try_from(rva).ok())
        .collect()
}

fn capture_engine_target_identity(kind: A24ProducerTapKind, owner: usize) -> Option<String> {
    // Do not probe a menu of plausible offsets here. A coincidental valid
    // `std::string` would fabricate a durable target identity. These are the
    // only target paths proven by the retained exact-a729 producer exports:
    //
    // * 0x141C33FE0: entry+0x30 is CIdea; and 0x141C2B640 /
    //   0x141C302E0: owner+0x18 is CIdea. CIdea+0x40 is the authored idea
    //   identity (`+0x50` length, `+0x58` capacity).
    // * 0x140FBEB00 / 0x140FBEA50 receive CIdea directly; 0x140B9C980
    //   receives CIdea as its second argument.
    // * 0x141C6AA70 -> 0x140616BC0: owner+0x518 reaches the selected-party
    //   holder, holder+0x18 is the ideology object, and ideology+0x18 is the
    //   identity string consumed by the exact name producer.
    //
    // Every other tap remains explicitly identity-unavailable until its own
    // object path is proven. Final resolved text/raw values are still retained;
    // no unproven identity is substituted for them.
    match kind {
        A24ProducerTapKind::PoliticsEntryRefresh => read_usize(owner.checked_add(0x30)?)?
            .checked_add(0x40)
            .and_then(read_engine_string),
        A24ProducerTapKind::ActiveIdeaTooltip
        | A24ProducerTapKind::PoliticalIdeaTooltip => read_usize(owner.checked_add(0x18)?)?
            .checked_add(0x40)
            .and_then(read_engine_string),
        A24ProducerTapKind::IdeaAvailability
        | A24ProducerTapKind::AllowedToRemove
        | A24ProducerTapKind::TimedIdeaRemainingDays => owner
            .checked_add(0x40)
            .and_then(read_engine_string),
        A24ProducerTapKind::BoostPartyTooltip => {
            let holder = read_usize(owner.checked_add(0x518)?)?;
            let ideology = read_usize(holder.checked_add(0x18)?)?;
            ideology.checked_add(0x18).and_then(read_engine_string)
        }
        _ => None,
    }
}

/// Read the exact MSVC `basic_string<char>` layout used by the pinned a729
/// producers. This copies bytes before the engine destroys any temporary.
fn read_engine_string(address: usize) -> Option<String> {
    let length = read_usize(address.checked_add(0x10)?)?;
    let capacity = read_usize(address.checked_add(0x18)?)?;
    if length == 0
        || length > crate::MAX_STRING_BYTES
        || capacity < length
        || (capacity != 15 && !(16..=crate::MAX_STRING_BYTES).contains(&capacity))
    {
        return None;
    }
    let data = if capacity <= 15 {
        address
    } else {
        read_usize(address)?
    };
    let bytes = rpm(data, length)?;
    String::from_utf8(bytes).ok().filter(|value| !value.trim().is_empty())
}

unsafe extern "system" fn hook_advisor_compact_cost(
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::AdvisorCompactCost, a1);
    let original: TapFn4 = unsafe {
        mem::transmute(ORIGINAL_ADVISOR_COMPACT_COST.load(Ordering::Acquire))
    };
    let result = unsafe { original(a1, a2, a3, a4) };
    finish_tap(open, Some(a2), None, None);
    result
}

unsafe extern "system" fn hook_advisor_full_tooltip(
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::AdvisorFullTooltip, a1);
    let original: TapFn4 = unsafe {
        mem::transmute(ORIGINAL_ADVISOR_FULL_TOOLTIP.load(Ordering::Acquire))
    };
    let result = unsafe { original(a1, a2, a3, a4) };
    finish_tap(open, Some(a2), None, None);
    result
}

unsafe extern "system" fn hook_ideology_popularity_change(
    a1: usize,
    a2: usize,
    a3: usize,
) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::IdeologyPopularityChange, a1);
    let original: TapFn3 = unsafe {
        mem::transmute(ORIGINAL_IDEOLOGY_POPULARITY_CHANGE.load(Ordering::Acquire))
    };
    let result = unsafe { original(a1, a2, a3) };
    finish_tap(open, Some(a2), None, None);
    result
}

unsafe extern "system" fn hook_timed_idea_tooltip(a1: usize, a2: usize, a3: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::TimedIdeaTooltip, a2);
    let original: TapFn3 =
        unsafe { mem::transmute(ORIGINAL_TIMED_IDEA_TOOLTIP.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2, a3) };
    finish_tap(open, Some(a3), None, None);
    result
}

unsafe extern "system" fn hook_active_idea_tooltip(a1: usize, a2: usize, a3: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::ActiveIdeaTooltip, a1);
    let original: TapFn3 =
        unsafe { mem::transmute(ORIGINAL_ACTIVE_IDEA_TOOLTIP.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2, a3) };
    finish_tap(open, Some(a2), None, None);
    result
}

unsafe extern "system" fn hook_political_idea_tooltip(a1: usize, a2: usize, a3: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::PoliticalIdeaTooltip, a1);
    let original: TapFn3 =
        unsafe { mem::transmute(ORIGINAL_POLITICAL_IDEA_TOOLTIP.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2, a3) };
    finish_tap(open, Some(a3), None, None);
    result
}

unsafe extern "system" fn hook_idea_category_tooltip(a1: usize, a2: usize, a3: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::IdeaCategoryTooltip, a1);
    let original: TapFn3 =
        unsafe { mem::transmute(ORIGINAL_IDEA_CATEGORY_TOOLTIP.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2, a3) };
    finish_tap(open, Some(a3), None, None);
    result
}

unsafe extern "system" fn hook_boost_party_tooltip(a1: usize, a2: usize, a3: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::BoostPartyTooltip, a1);
    let original: TapFn3 =
        unsafe { mem::transmute(ORIGINAL_BOOST_PARTY_TOOLTIP.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2, a3) };
    finish_tap(open, Some(a3), None, None);
    result
}

unsafe extern "system" fn hook_idea_modifier_description(a1: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::IdeaModifierDescription, a1);
    let original: TapFn1 =
        unsafe { mem::transmute(ORIGINAL_IDEA_MODIFIER_DESCRIPTION.load(Ordering::Acquire)) };
    let result = unsafe { original(a1) };
    finish_tap(open, None, None, None);
    result
}

unsafe extern "system" fn hook_idea_availability(a1: usize, a2: usize) -> u8 {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::IdeaAvailability, a1);
    let original: TapFn2U8 =
        unsafe { mem::transmute(ORIGINAL_IDEA_AVAILABILITY.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2) };
    finish_tap(open, None, None, Some(result != 0));
    result
}

unsafe extern "system" fn hook_effective_idea_cost(a1: usize, a2: usize, a3: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::EffectiveIdeaCost, a1);
    let original: TapFn3 =
        unsafe { mem::transmute(ORIGINAL_EFFECTIVE_IDEA_COST.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2, a3) };
    finish_tap(open, None, read_i64(a2), None);
    result
}

unsafe extern "system" fn hook_resource_affordability(a1: usize, a2: usize, a3: usize) -> u8 {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::ResourceAffordability, a1);
    let original: TapFn3U8 =
        unsafe { mem::transmute(ORIGINAL_RESOURCE_AFFORDABILITY.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2, a3) };
    finish_tap(open, None, Some(a3 as i64), Some(result != 0));
    result
}

unsafe extern "system" fn hook_allowed_to_remove(a1: usize, a2: usize) -> u8 {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::AllowedToRemove, a1);
    let original: TapFn2U8 =
        unsafe { mem::transmute(ORIGINAL_ALLOWED_TO_REMOVE.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2) };
    finish_tap(open, None, None, Some(result != 0));
    result
}

unsafe extern "system" fn hook_party_popularity_computation(
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::PartyPopularityComputation, a2);
    let original: TapFn5 = unsafe {
        mem::transmute(ORIGINAL_PARTY_POPULARITY_COMPUTATION.load(Ordering::Acquire))
    };
    let result = unsafe { original(a1, a2, a3, a4, a5) };
    finish_tap(open, (a5 != 0).then_some(a5), read_i64(a1), None);
    result
}

unsafe extern "system" fn hook_timed_idea_remaining_days(a1: usize, a2: usize) -> u32 {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::TimedIdeaRemainingDays, a2);
    let original: TapFn2U32 = unsafe {
        mem::transmute(ORIGINAL_TIMED_IDEA_REMAINING_DAYS.load(Ordering::Acquire))
    };
    let result = unsafe { original(a1, a2) };
    finish_tap(open, None, Some((result as i32) as i64), None);
    result
}

unsafe extern "system" fn hook_country_can_take_ideas(a1: usize, a2: usize) -> u8 {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::CountryCanTakeIdeas, a1);
    let original: TapFn2U8 = unsafe {
        mem::transmute(ORIGINAL_COUNTRY_CAN_TAKE_IDEAS.load(Ordering::Acquire))
    };
    let result = unsafe { original(a1, a2) };
    finish_tap(open, (a2 != 0).then_some(a2), None, Some(result != 0));
    result
}

unsafe extern "system" fn hook_boost_party_why_text(a1: usize, a2: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::BoostPartyWhyText, a1);
    let original: TapFn2 =
        unsafe { mem::transmute(ORIGINAL_BOOST_PARTY_WHY_TEXT.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2) };
    finish_tap(open, Some(a2), None, None);
    result
}

unsafe extern "system" fn hook_boost_party_name_text(a1: usize, a2: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::BoostPartyNameText, a1);
    let original: TapFn2 =
        unsafe { mem::transmute(ORIGINAL_BOOST_PARTY_NAME_TEXT.load(Ordering::Acquire)) };
    let result = unsafe { original(a1, a2) };
    finish_tap(open, Some(a2), None, None);
    result
}

unsafe extern "system" fn hook_idea_selection_refresh(a1: usize, a2: usize, a3: u8) {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::IdeaSelectionRefresh, a1);
    let original: VoidTapFn3U8 = unsafe {
        mem::transmute(ORIGINAL_IDEA_SELECTION_REFRESH.load(Ordering::Acquire))
    };
    unsafe { original(a1, a2, a3) };
    finish_tap(open, None, read_u32(a2).map(i64::from), None);
}

unsafe extern "system" fn hook_idea_compact_cost_refresh(a1: usize, a2: u8) {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::IdeaCompactCostRefresh, a1);
    let original: VoidTapFn2U8 = unsafe {
        mem::transmute(ORIGINAL_IDEA_COMPACT_COST_REFRESH.load(Ordering::Acquire))
    };
    unsafe { original(a1, a2) };
    finish_tap(open, None, None, Some(a2 != 0));
}

unsafe extern "system" fn hook_idea_icon_and_modifier_bind(a1: usize) {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::IdeaIconAndModifierBind, a1);
    let original: VoidTapFn1 = unsafe {
        mem::transmute(ORIGINAL_IDEA_ICON_AND_MODIFIER_BIND.load(Ordering::Acquire))
    };
    unsafe { original(a1) };
    finish_tap(open, None, None, None);
}

unsafe extern "system" fn hook_modifier_value_lookup(a1: usize, a2: usize, a3: u32) -> usize {
    let _callback = CallbackGuard::enter();
    let mut open = begin_tap(A24ProducerTapKind::ModifierValueLookup, a1);
    if open.frame_hour.is_some() {
        // 0x14054C770 proves that a modifier source node owns its authored
        // identity as an MSVC string at +0x48 before its single lookup call at
        // 0x14054C936. Other callers pass aggregate containers whose +0x48
        // field has no proven string meaning, so label only the exact
        // post-call return address and leave every other lookup ID-only.
        let source_node_call = open
            .stack_rvas
            .contains(&MODIFIER_SOURCE_LOOKUP_RETURN_RVA);
        open.engine_target_identity = if source_node_call {
            a1.checked_add(0x48)
                .and_then(read_engine_string)
                .map(|source| format!("modifier_id:{a3}:source:{source}"))
                .or_else(|| Some(format!("modifier_id:{a3}")))
        } else {
            Some(format!("modifier_id:{a3}"))
        };
    }
    let original: TapFn3U32Arg = unsafe {
        mem::transmute(ORIGINAL_MODIFIER_VALUE_LOOKUP.load(Ordering::Acquire))
    };
    let result = unsafe { original(a1, a2, a3) };
    finish_tap(open, None, read_i64(a2), None);
    result
}

pub fn install_a729_politics_hook() -> Result<InstalledHookAddress, String> {
    let _install_guard = INSTALL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "A24 hook lifecycle mutex poisoned".to_string())?;
    if TEARING_DOWN.load(Ordering::Acquire) {
        return Err("A24 hook lifecycle is quiescing or requires uninstall retry".into());
    }
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(InstalledHookAddress {
            politics_entry_refresh: MODULE_BASE.load(Ordering::Acquire)
                + POLITICS_ENTRY_REFRESH_RVA as usize,
        });
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let target = base + POLITICS_ENTRY_REFRESH_RVA as usize;
    verify_sentinel(target, &REFRESH_SENTINEL)?;
    for (rva, sentinel) in REQUIRED_PRODUCER_SENTINELS {
        verify_sentinel(base + *rva as usize, sentinel)?;
    }
    STATE.get_or_init(|| Mutex::new(HookState::default()));
    TEARING_DOWN.store(false, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    let specifications = [
        (POLITICS_ENTRY_REFRESH_RVA, hook_politics_entry_refresh as *mut c_void, &ORIGINAL_REFRESH),
        (ADVISOR_COMPACT_COST_RVA, hook_advisor_compact_cost as *mut c_void, &ORIGINAL_ADVISOR_COMPACT_COST),
        (ADVISOR_FULL_TOOLTIP_RVA, hook_advisor_full_tooltip as *mut c_void, &ORIGINAL_ADVISOR_FULL_TOOLTIP),
        (IDEOLOGY_POPULARITY_CHANGE_RVA, hook_ideology_popularity_change as *mut c_void, &ORIGINAL_IDEOLOGY_POPULARITY_CHANGE),
        (TIMED_IDEA_TOOLTIP_RVA, hook_timed_idea_tooltip as *mut c_void, &ORIGINAL_TIMED_IDEA_TOOLTIP),
        (ACTIVE_IDEA_TOOLTIP_RVA, hook_active_idea_tooltip as *mut c_void, &ORIGINAL_ACTIVE_IDEA_TOOLTIP),
        (POLITICAL_IDEA_TOOLTIP_RVA, hook_political_idea_tooltip as *mut c_void, &ORIGINAL_POLITICAL_IDEA_TOOLTIP),
        (IDEA_CATEGORY_TOOLTIP_RVA, hook_idea_category_tooltip as *mut c_void, &ORIGINAL_IDEA_CATEGORY_TOOLTIP),
        (BOOST_PARTY_TOOLTIP_RVA, hook_boost_party_tooltip as *mut c_void, &ORIGINAL_BOOST_PARTY_TOOLTIP),
        (IDEA_MODIFIER_DESCRIPTION_RVA, hook_idea_modifier_description as *mut c_void, &ORIGINAL_IDEA_MODIFIER_DESCRIPTION),
        (IDEA_AVAILABILITY_RVA, hook_idea_availability as *mut c_void, &ORIGINAL_IDEA_AVAILABILITY),
        (EFFECTIVE_IDEA_COST_RVA, hook_effective_idea_cost as *mut c_void, &ORIGINAL_EFFECTIVE_IDEA_COST),
        (RESOURCE_AFFORDABILITY_RVA, hook_resource_affordability as *mut c_void, &ORIGINAL_RESOURCE_AFFORDABILITY),
        (ALLOWED_TO_REMOVE_RVA, hook_allowed_to_remove as *mut c_void, &ORIGINAL_ALLOWED_TO_REMOVE),
        (PARTY_POPULARITY_COMPUTATION_RVA, hook_party_popularity_computation as *mut c_void, &ORIGINAL_PARTY_POPULARITY_COMPUTATION),
        (TIMED_IDEA_REMAINING_DAYS_RVA, hook_timed_idea_remaining_days as *mut c_void, &ORIGINAL_TIMED_IDEA_REMAINING_DAYS),
        (COUNTRY_CAN_TAKE_IDEAS_RVA, hook_country_can_take_ideas as *mut c_void, &ORIGINAL_COUNTRY_CAN_TAKE_IDEAS),
        (BOOST_PARTY_WHY_TEXT_RVA, hook_boost_party_why_text as *mut c_void, &ORIGINAL_BOOST_PARTY_WHY_TEXT),
        (BOOST_PARTY_NAME_TEXT_RVA, hook_boost_party_name_text as *mut c_void, &ORIGINAL_BOOST_PARTY_NAME_TEXT),
        (IDEA_SELECTION_REFRESH_RVA, hook_idea_selection_refresh as *mut c_void, &ORIGINAL_IDEA_SELECTION_REFRESH),
        (IDEA_COMPACT_COST_REFRESH_RVA, hook_idea_compact_cost_refresh as *mut c_void, &ORIGINAL_IDEA_COMPACT_COST_REFRESH),
        (IDEA_ICON_AND_MODIFIER_BIND_RVA, hook_idea_icon_and_modifier_bind as *mut c_void, &ORIGINAL_IDEA_ICON_AND_MODIFIER_BIND),
        (MODIFIER_VALUE_LOOKUP_RVA, hook_modifier_value_lookup as *mut c_void, &ORIGINAL_MODIFIER_VALUE_LOOKUP),
    ];
    let mut created = Vec::with_capacity(specifications.len());
    for (rva, detour, original_slot) in specifications {
        let hook_target = base + rva as usize;
        let original = match unsafe {
            MinHook::create_hook(hook_target as *mut c_void, detour)
        } {
            Ok(original) => original,
            Err(status) => {
                let mut cleanup_errors = Vec::new();
                for created_target in created.iter().rev() {
                    if let Err(remove_status) =
                        unsafe { MinHook::remove_hook(*created_target as *mut c_void) }
                    {
                        cleanup_errors.push(format!(
                            "remove 0x{created_target:X}: {remove_status}"
                        ));
                    }
                }
                if cleanup_errors.is_empty() {
                    for (_, _, original_slot) in specifications.iter().take(created.len()) {
                        original_slot.store(0, Ordering::Release);
                    }
                    MODULE_BASE.store(0, Ordering::Release);
                } else {
                    TEARING_DOWN.store(true, Ordering::Release);
                    INSTALLED.store(true, Ordering::Release);
                }
                return Err(format!(
                    "A24 producer hook creation failed at RVA 0x{rva:08X}: {status}{}",
                    if cleanup_errors.is_empty() {
                        String::new()
                    } else {
                        format!("; cleanup retained hook state: {}", cleanup_errors.join("; "))
                    }
                ));
            }
        };
        original_slot.store(original as usize, Ordering::Release);
        created.push(hook_target);
    }
    for hook_target in &created {
        if let Err(status) = unsafe { MinHook::enable_hook(*hook_target as *mut c_void) } {
            TEARING_DOWN.store(true, Ordering::Release);
            let mut cleanup_errors = Vec::new();
            for enabled_target in created.iter().rev() {
                match unsafe { MinHook::disable_hook(*enabled_target as *mut c_void) } {
                    Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) => {}
                    Err(disable_status) => cleanup_errors.push(format!(
                        "disable 0x{enabled_target:X}: {disable_status}"
                    )),
                }
            }
            if cleanup_errors.is_empty() {
                if let Err(error) = wait_for_callback_quiescence() {
                    cleanup_errors.push(error);
                }
            }
            if cleanup_errors.is_empty() {
                for enabled_target in created.iter().rev() {
                    match unsafe { MinHook::remove_hook(*enabled_target as *mut c_void) } {
                        Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                        Err(remove_status) => cleanup_errors.push(format!(
                            "remove 0x{enabled_target:X}: {remove_status}"
                        )),
                    }
                }
            }
            if cleanup_errors.is_empty() {
                for (_, _, original_slot) in specifications {
                    original_slot.store(0, Ordering::Release);
                }
                MODULE_BASE.store(0, Ordering::Release);
                TEARING_DOWN.store(false, Ordering::Release);
            } else {
                // Keep enough lifecycle state for the owner to retry the normal
                // quiescing uninstall path; never discard live trampolines.
                INSTALLED.store(true, Ordering::Release);
            }
            return Err(format!(
                "A24 producer hook enable failed at 0x{hook_target:X}: {status}{}",
                if cleanup_errors.is_empty() {
                    String::new()
                } else {
                    format!("; cleanup retained hook state: {}", cleanup_errors.join("; "))
                }
            ));
        }
    }
    FULL_PRODUCER_GRAPH_INSTALLED.store(true, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(InstalledHookAddress {
        politics_entry_refresh: target,
    })
}

/// Disable, quiesce, and remove every A24 detour before the collector DLL may
/// unload. A partially failed teardown leaves `INSTALLED` true and the module
/// pinned so the owner can retry instead of returning into unloaded code.
pub fn uninstall_a729_politics_hook() -> Result<(), String> {
    let _install_guard = INSTALL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "A24 hook lifecycle mutex poisoned".to_string())?;
    if !INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("A24 module base is unavailable during uninstall".into());
    }
    TEARING_DOWN.store(true, Ordering::Release);
    for rva in ALL_HOOK_RVAS {
        let target = base + rva as usize;
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                return Err(format!(
                    "A24 producer hook disable failed at RVA 0x{rva:08X}: {status}"
                ));
            }
        }
    }
    wait_for_callback_quiescence()?;
    for rva in ALL_HOOK_RVAS {
        let target = base + rva as usize;
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                return Err(format!(
                    "A24 producer hook removal failed at RVA 0x{rva:08X}: {status}"
                ));
            }
        }
    }
    cancel_a24_capture_epoch();
    if let Some(state) = STATE.get() {
        *state
            .lock()
            .map_err(|_| "A24 hook state mutex poisoned during uninstall".to_string())? =
            HookState::default();
    }
    for slot in [
        &ORIGINAL_REFRESH,
        &ORIGINAL_ADVISOR_COMPACT_COST,
        &ORIGINAL_ADVISOR_FULL_TOOLTIP,
        &ORIGINAL_IDEOLOGY_POPULARITY_CHANGE,
        &ORIGINAL_TIMED_IDEA_TOOLTIP,
        &ORIGINAL_ACTIVE_IDEA_TOOLTIP,
        &ORIGINAL_POLITICAL_IDEA_TOOLTIP,
        &ORIGINAL_IDEA_CATEGORY_TOOLTIP,
        &ORIGINAL_BOOST_PARTY_TOOLTIP,
        &ORIGINAL_IDEA_MODIFIER_DESCRIPTION,
        &ORIGINAL_IDEA_AVAILABILITY,
        &ORIGINAL_EFFECTIVE_IDEA_COST,
        &ORIGINAL_RESOURCE_AFFORDABILITY,
        &ORIGINAL_ALLOWED_TO_REMOVE,
        &ORIGINAL_PARTY_POPULARITY_COMPUTATION,
        &ORIGINAL_TIMED_IDEA_REMAINING_DAYS,
        &ORIGINAL_COUNTRY_CAN_TAKE_IDEAS,
        &ORIGINAL_BOOST_PARTY_WHY_TEXT,
        &ORIGINAL_BOOST_PARTY_NAME_TEXT,
        &ORIGINAL_IDEA_SELECTION_REFRESH,
        &ORIGINAL_IDEA_COMPACT_COST_REFRESH,
        &ORIGINAL_IDEA_ICON_AND_MODIFIER_BIND,
        &ORIGINAL_MODIFIER_VALUE_LOOKUP,
    ] {
        slot.store(0, Ordering::Release);
    }
    FULL_PRODUCER_GRAPH_INSTALLED.store(false, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    TEARING_DOWN.store(false, Ordering::Release);
    Ok(())
}

fn wait_for_callback_quiescence() -> Result<(), String> {
    let deadline = Instant::now()
        .checked_add(CALLBACK_QUIESCE_TIMEOUT)
        .ok_or("A24 callback quiescence deadline overflow")?;
    while IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            return Err(format!(
                "A24 callback quiescence timed out with {} callbacks active",
                IN_FLIGHT_CALLBACKS.load(Ordering::Acquire)
            ));
        }
        std::thread::yield_now();
    }
    Ok(())
}

unsafe extern "system" fn hook_politics_entry_refresh(
    entry: usize,
    idea_id_pointer: usize,
    country_context: usize,
) {
    let _callback = CallbackGuard::enter();
    let open = begin_tap(A24ProducerTapKind::PoliticsEntryRefresh, entry);
    let idea_id = read_u32(idea_id_pointer);
    let original = ORIGINAL_REFRESH.load(Ordering::Acquire);
    if original != 0 {
        let original: RefreshFn = unsafe { mem::transmute(original) };
        unsafe { original(entry, idea_id_pointer, country_context) };
    }
    match idea_id {
        Some(idea_id) if entry != 0 && idea_id != 0 && country_context != 0 => {
            record_completed_tuple(PoliticsProducerTuple {
                entry,
                idea_id,
                country_context,
            });
        }
        _ => reject_tuple(),
    }
    finish_tap(open, None, idea_id.map(i64::from), None);
}

fn record_completed_tuple(tuple: PoliticsProducerTuple) {
    let Ok(mut state) = lock_state() else {
        return;
    };
    state.observed_refresh_count = state.observed_refresh_count.saturating_add(1);
    if state.recent_completed_tuples.len() >= MAX_RECENT_TUPLES {
        state.recent_completed_tuples.pop_front();
        state.evicted_tuple_count = state.evicted_tuple_count.saturating_add(1);
    }
    state.recent_completed_tuples.push_back(tuple);
}

fn reject_tuple() {
    if let Ok(mut state) = lock_state() {
        state.rejected_tuple_count = state.rejected_tuple_count.saturating_add(1);
    }
}

fn reject_batch<T>(message: &str) -> Result<T, String> {
    if let Ok(mut state) = lock_state() {
        state.rejected_batch_count = state.rejected_batch_count.saturating_add(1);
    }
    Err(message.into())
}

fn reject_seal<T>(message: &str) -> Result<T, String> {
    if let Ok(mut state) = lock_state() {
        state.rejected_seal_count = state.rejected_seal_count.saturating_add(1);
    }
    Err(message.into())
}

fn lock_state() -> Result<std::sync::MutexGuard<'static, HookState>, String> {
    STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "a729 politics hook state mutex poisoned".into())
}

fn required_line_kinds() -> BTreeSet<PoliticsLineKind> {
    [
        PoliticsLineKind::EvaluatedTriggerResultAndReasonTree,
        PoliticsLineKind::EffectiveHireOrRemovalCost,
        PoliticsLineKind::AggregatedConditionalModifier,
        PoliticsLineKind::AdvisorSlotCapacityVacancyEligibilityAndDisabledReason,
        PoliticsLineKind::TimedDynamicModifierValueSourceAndExpiry,
        PoliticsLineKind::DisplayedPartyPopularityChangeSource,
    ]
    .into_iter()
    .collect()
}

fn row_order(left: &PoliticsTooltipInput, right: &PoliticsTooltipInput) -> std::cmp::Ordering {
    (
        left.country_tag.as_str(),
        left.target_kind.as_str(),
        left.target_token_or_slot.as_str(),
        left.line_kind,
    )
        .cmp(&(
            right.country_tag.as_str(),
            right.target_kind.as_str(),
            right.target_token_or_slot.as_str(),
            right.line_kind,
        ))
}

fn verify_sentinel(address: usize, expected: &[u8]) -> Result<(), String> {
    let actual = rpm(address, expected.len())
        .ok_or_else(|| "politics entry refresh sentinel unreadable".to_string())?;
    if actual != expected {
        return Err(format!(
            "politics entry refresh sentinel mismatch at 0x{address:X}"
        ));
    }
    Ok(())
}

fn read_u32(address: usize) -> Option<u32> {
    Some(u32::from_le_bytes(rpm(address, 4)?.try_into().ok()?))
}

fn read_usize(address: usize) -> Option<usize> {
    Some(usize::from_le_bytes(
        rpm(address, mem::size_of::<usize>())?.try_into().ok()?,
    ))
}

fn read_i64(address: usize) -> Option<i64> {
    Some(i64::from_le_bytes(rpm(address, 8)?.try_into().ok()?))
}

fn query_performance_counter() -> Option<u64> {
    let mut value = 0i64;
    (unsafe { QueryPerformanceCounter(&mut value) } != 0 && value > 0)
        .then_some(value as u64)
}

fn rpm(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 {
        return None;
    }
    let mut out = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr() as *mut c_void,
            len,
            &mut read,
        )
    };
    (ok != 0 && read == len).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AvailabilityState, CostAction, NumericSourceTerm, PoliticsTypedValue, ReasonNode};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn rows(hour: u64) -> Vec<PoliticsTooltipInput> {
        let reason = ReasonNode {
            source_ordinal: 1,
            node_id: "root".into(),
            parent_node_id: None,
            source_identity: "CIdea.allowed_to_remove".into(),
            result: true,
            resolved_reason: "allowed".into(),
        };
        let term = NumericSourceTerm {
            source_ordinal: 1,
            source_identity: "engine:term".into(),
            evaluation_context: "country=GER".into(),
            raw_value: 10,
            scale: 1000,
        };
        let values = [
            (
                PoliticsLineKind::EvaluatedTriggerResultAndReasonTree,
                PoliticsTypedValue::TriggerEvaluation {
                    result: true,
                    reason_tree: vec![reason.clone()],
                },
            ),
            (
                PoliticsLineKind::EffectiveHireOrRemovalCost,
                PoliticsTypedValue::EffectiveCost {
                    action: CostAction::Hire,
                    resource_token: "political_power".into(),
                    raw_value: 100,
                    scale: 1000,
                    affordable: true,
                },
            ),
            (
                PoliticsLineKind::AggregatedConditionalModifier,
                PoliticsTypedValue::ConditionalModifierAggregate {
                    modifier_token: "political_power_gain".into(),
                    raw_total: 10,
                    scale: 1000,
                    terms: vec![term.clone()],
                },
            ),
            (
                PoliticsLineKind::AdvisorSlotCapacityVacancyEligibilityAndDisabledReason,
                PoliticsTypedValue::AdvisorSlotState {
                    capacity: 3,
                    occupied: 2,
                    vacancy: true,
                    eligible: true,
                    disabled_reason_tree: vec![reason],
                },
            ),
            (
                PoliticsLineKind::TimedDynamicModifierValueSourceAndExpiry,
                PoliticsTypedValue::TimedDynamicModifier {
                    modifier_token: "timed".into(),
                    raw_value: 10,
                    scale: 1000,
                    source_token: "event:1".into(),
                    applied_hour: Some(hour - 1),
                    expiry_hour: Some(hour + 1),
                },
            ),
            (
                PoliticsLineKind::DisplayedPartyPopularityChangeSource,
                PoliticsTypedValue::PartyPopularityChange {
                    party_token: "fascism".into(),
                    raw_change: 10,
                    scale: 1000,
                    sources: vec![term],
                },
            ),
        ];
        values
            .into_iter()
            .enumerate()
            .map(|(index, (line_kind, typed_value))| PoliticsTooltipInput {
                frame_hour: hour,
                country_tag: "GER".into(),
                target_kind: "idea".into(),
                target_token_or_slot: format!("target:{index}"),
                line_kind,
                availability_state: AvailabilityState::Measured,
                typed_value: Some(typed_value),
                source_identity: "0x141C33FE0".into(),
                evaluation_context: "country=GER;pov=GER".into(),
            })
            .collect()
    }

    fn reset() {
        *lock_state().unwrap() = HookState::default();
        INSTALLED.store(true, Ordering::Release);
        FULL_PRODUCER_GRAPH_INSTALLED.store(true, Ordering::Release);
    }

    fn selected_present() -> PoliticsSelectedPresentBinding {
        PoliticsSelectedPresentBinding {
            candidate_id: 7,
            ordinal: 0,
            qpc_ticks: 100,
            cutoff_qpc_ticks: 100,
            receipt_sha256: "a".repeat(64),
        }
    }

    #[test]
    fn authenticated_complete_frame_implements_exact_source() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        let hour = 900;
        let tuple = PoliticsProducerTuple {
            entry: 0x1000,
            idea_id: 7,
            country_context: 0x2000,
        };
        record_completed_tuple(tuple);
        begin_exact_politics_frame(
            hour,
            selected_present(),
            "a24:a729:900:tuples=1".into(),
            1,
        )
        .unwrap();
        submit_exact_politics_batch(ExactPoliticsProducerBatch {
            frame_hour: hour,
            producer_tuple: tuple,
            dispatch_identity: DispatchIdentity::callsite_b(),
            rows: rows(hour),
        })
        .unwrap();
        seal_exact_politics_frame(hour).unwrap();
        let source = A729PoliticsHookSource;
        assert!(source.is_exact_a729_politics_source());
        assert_eq!(source.producer_manifest_sha256(), PRODUCER_MANIFEST_SHA256);
        assert_eq!(source.expected_row_count(hour).unwrap(), 6);
        assert_eq!(source.collect_frame(hour).unwrap().len(), 6);
        assert!(crate::collect_and_encode(&source, hour, 3, 0).is_ok());
        assert_eq!(
            source.completeness_record_identity(hour).unwrap(),
            "a24:a729:900:tuples=1"
        );
    }

    #[test]
    fn unobserved_tuple_bad_chain_and_missing_family_fail_closed() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        let hour = 901;
        let tuple = PoliticsProducerTuple {
            entry: 0x1000,
            idea_id: 8,
            country_context: 0x2000,
        };
        begin_exact_politics_frame(hour, selected_present(), "complete".into(), 1).unwrap();
        assert!(submit_exact_politics_batch(ExactPoliticsProducerBatch {
            frame_hour: hour,
            producer_tuple: tuple,
            dispatch_identity: DispatchIdentity { frames: [0; 6] },
            rows: rows(hour),
        })
        .is_err());

        reset();
        record_completed_tuple(tuple);
        begin_exact_politics_frame(hour, selected_present(), "complete".into(), 1).unwrap();
        let mut incomplete = rows(hour);
        incomplete.pop();
        submit_exact_politics_batch(ExactPoliticsProducerBatch {
            frame_hour: hour,
            producer_tuple: tuple,
            dispatch_identity: DispatchIdentity::callsite_a(),
            rows: incomplete,
        })
        .unwrap();
        assert!(seal_exact_politics_frame(hour).is_err());
        assert!(A729PoliticsHookSource.collect_frame(hour).is_err());
    }
}
