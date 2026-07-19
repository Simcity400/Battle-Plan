//! Signature-gated a729 scheduler and authenticated source for A24.
//!
//! The hook at the exact politics-entry refresh records the engine producer
//! tuple. Producer taps submit only engine-evaluated rows associated with that
//! tuple. A frame is visible through [`ExactPoliticsTooltipSource`] only after
//! the scheduler seals it and proves all six required row families, exact
//! count agreement, and one of the retained authenticated dispatch chains.

use crate::{
    validate_frame, ExactPoliticsTooltipSource, PoliticsLineKind, PoliticsTooltipFrame,
    PoliticsTooltipInput,
};
use core::ffi::c_void;
use minhook::MinHook;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const POLITICS_ENTRY_REFRESH_RVA: u32 = 0x01C3_3FE0;
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
    0xae, 0xc7, 0x20, 0xfe, 0x93, 0x71, 0xb0, 0x12, 0xbf, 0xb6, 0xa8, 0xf0, 0x00, 0xf7, 0x4a, 0xb3,
    0x92, 0x32, 0xf3, 0x48, 0x2a, 0x76, 0x6c, 0x0e, 0xdf, 0xac, 0x75, 0x82, 0xce, 0x48, 0x9c, 0xe5,
];

const REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
const MAX_RECENT_TUPLES: usize = 262_144;
const MAX_SEALED_FRAMES: usize = 8;

type RefreshFn = unsafe extern "system" fn(usize, usize, usize);

static ORIGINAL_REFRESH: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PoliticsProducerTuple {
    pub entry: usize,
    pub idea_id: u32,
    pub country_context: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchIdentity {
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
pub struct ExactPoliticsProducerBatch {
    pub frame_hour: u64,
    pub producer_tuple: PoliticsProducerTuple,
    pub dispatch_identity: DispatchIdentity,
    pub rows: Vec<PoliticsTooltipInput>,
}

#[derive(Clone, Debug)]
struct OpenFrame {
    completeness_record_identity: String,
    expected_producer_tuple_count: u32,
    batches: BTreeMap<PoliticsProducerTuple, Vec<PoliticsTooltipInput>>,
    dispatches: BTreeSet<[u32; 6]>,
}

#[derive(Clone, Debug)]
struct SealedFrame {
    completeness_record_identity: String,
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
    }

    fn producer_manifest_sha256(&self) -> [u8; 32] {
        PRODUCER_MANIFEST_SHA256
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
pub fn begin_exact_politics_frame(
    frame_hour: u64,
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
            expected_producer_tuple_count,
            batches: BTreeMap::new(),
            dispatches: BTreeSet::new(),
        },
    );
    Ok(())
}

/// Admit one exact engine-evaluated producer batch. The tuple must have been
/// observed at the signature-gated refresh hook and is consumed exactly once.
pub fn submit_exact_politics_batch(batch: ExactPoliticsProducerBatch) -> Result<(), String> {
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
pub fn seal_exact_politics_frame(frame_hour: u64) -> Result<(), String> {
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
            rows,
        },
    );
    while state.sealed_frames.len() > MAX_SEALED_FRAMES {
        let oldest = *state.sealed_frames.keys().next().expect("non-empty");
        state.sealed_frames.remove(&oldest);
    }
    Ok(())
}

pub fn install_a729_politics_hook() -> Result<InstalledHookAddress, String> {
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
    STATE.get_or_init(|| Mutex::new(HookState::default()));
    let original = unsafe {
        MinHook::create_hook(
            target as *mut c_void,
            hook_politics_entry_refresh as *mut c_void,
        )
    }
    .map_err(|status| format!("politics entry refresh hook creation failed: {status}"))?;
    ORIGINAL_REFRESH.store(original as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
        let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
        return Err(format!(
            "politics entry refresh hook enable failed: {status}"
        ));
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(InstalledHookAddress {
        politics_entry_refresh: target,
    })
}

unsafe extern "system" fn hook_politics_entry_refresh(
    entry: usize,
    idea_id_pointer: usize,
    country_context: usize,
) {
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
        begin_exact_politics_frame(hour, "a24:a729:900:tuples=1".into(), 1).unwrap();
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
        begin_exact_politics_frame(hour, "complete".into(), 1).unwrap();
        assert!(submit_exact_politics_batch(ExactPoliticsProducerBatch {
            frame_hour: hour,
            producer_tuple: tuple,
            dispatch_identity: DispatchIdentity { frames: [0; 6] },
            rows: rows(hour),
        })
        .is_err());

        reset();
        record_completed_tuple(tuple);
        begin_exact_politics_frame(hour, "complete".into(), 1).unwrap();
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
