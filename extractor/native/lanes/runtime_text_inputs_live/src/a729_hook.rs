//! Signature-gated a729 hook implementation for the exact A25 source.

use crate::{
    ExactRuntimeTextSource, RuntimeTextInput, RuntimeTextLayout, RuntimeTextSourceIdentity,
    TEXTBOX_DESTRUCTOR_RVA, TEXTBOX_REGISTRATION_RVA, TEXT_SETTER_RVA,
};
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, RtlCaptureStackBackTrace};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const TEXTBOX_TERNARY_VTABLE_RVA: usize = 0x02B3_8350;
const TEXTBOX_VTABLE_RVA: usize = 0x02B2_B418;
const TEXTBOX_CHILD_OWNER_OFFSET: usize = 0x148;
const TEXTBOX_GRAPHICS_CHILD_OFFSET: usize = 0x88;
const GRAPHICS_CHILD_LAYOUT_OBJECT_OFFSET: usize = 0xC8;
const LAYOUT_OBJECT_FINAL_TEXT_OFFSET: usize = 0x188;
const FIXED_WINDOW_MAP_OFFSET: usize = 0x1C60;
const FIXED_WINDOW_CGUI_TYPE_OFFSET: usize = 0x90;
const CGUI_TYPE_NAME_OFFSET: usize = 0x28;
const TEXTBOX_AUTHORED_NAME_OFFSET: usize = 0x98;
const TEXTBOX_CGUI_TYPE_OFFSET: usize = 0x60;
const TEXTBOX_TYPE_SOURCE_TOKEN_OFFSET: usize = 0x138;
const MAX_PENDING: usize = 262_144;
const MAX_TERNARY_NODES: usize = MAX_PENDING * 64;

const SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x57,
];
const REGISTRATION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x80, 0x79, 0x10, 0x01, 0x49, 0x8B,
];
const DESTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x8B, 0xDA, 0x48, 0x8B, 0xF9, 0xE8,
];

type SetterFn = unsafe extern "system" fn(usize, usize);
type RegistrationFn = unsafe extern "system" fn(usize, usize, usize);
type DestructorFn = unsafe extern "system" fn(usize, u32) -> usize;

static ORIGINAL_SETTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_REGISTRATION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DESTRUCTOR: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static BASELINE_COMPLETE: AtomicBool = AtomicBool::new(false);
// A25 owns the sole setter detour but several exact lanes may observe it.
// Fixed atomic slots keep dispatch lock-free on the game thread.
static POST_SETTER_OBSERVER_0: AtomicUsize = AtomicUsize::new(0);
static POST_SETTER_OBSERVER_1: AtomicUsize = AtomicUsize::new(0);
static POST_SETTER_OBSERVER_2: AtomicUsize = AtomicUsize::new(0);
static POST_SETTER_OBSERVER_3: AtomicUsize = AtomicUsize::new(0);
static PRE_DESTRUCTOR_OBSERVER: AtomicUsize = AtomicUsize::new(0);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

#[derive(Clone)]
struct WidgetIdentity {
    view_id: String,
    element_id: String,
    source_token: String,
    ordinal: u32,
}

#[derive(Clone)]
struct SetterValue {
    bytes: Vec<u8>,
    caller_rva: u32,
}

#[derive(Default)]
struct HookState {
    identities: HashMap<usize, WidgetIdentity>,
    latest: HashMap<usize, SetterValue>,
    /// Exact rows pinned by lifecycle consumers before textbox destruction.
    pinned: BTreeMap<String, (WidgetIdentity, SetterValue)>,
    next_ordinal: BTreeMap<(String, String), u32>,
    rejected_identity: u64,
    rejected_text: u64,
    evicted_pending: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub registered_widget_count: u64,
    pub observed_context_count: u64,
    pub rejected_identity_count: u64,
    pub rejected_text_count: u64,
    pub evicted_pending_count: u64,
    pub baseline_complete: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BaselineSeedDiagnostics {
    pub validated_map_count: u64,
    pub seeded_widget_count: u64,
    pub retained_hook_value_count: u64,
}

struct BaselineCandidate {
    map: usize,
    owner: usize,
    view_id: String,
    element_id: String,
    source_token: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub textbox_registration: usize,
    pub text_setter: usize,
    pub textbox_destructor: usize,
}

pub struct A729RuntimeTextHookSource;

/// True only after a pre-hook fixed-window census has been admitted. Once
/// true, registration/setter/destructor hooks maintain the complete live set,
/// so the collector does not need another whole-heap baseline pass.
pub fn baseline_complete() -> bool {
    BASELINE_COMPLETE.load(Ordering::Acquire)
}

/// Non-owning observer invoked after the original setter has recomputed text
/// layout. `owner` is the validated CTextBox at `setter_widget+0x148` and
pub type PostSetterObserver =
    unsafe extern "system" fn(owner: usize, setter_widget: usize, caller_rva: u32);

/// Register a post-setter observer (used by A26 and XUI21). The
/// callback does not install another detour and therefore cannot conflict with
/// A25 hook ownership.
pub fn register_post_setter_observer(observer: PostSetterObserver) -> Result<(), String> {
    let value = observer as usize;
    if post_setter_observer_slots()
        .iter()
        .any(|slot| slot.load(Ordering::Acquire) == value)
    {
        return Ok(());
    }
    for slot in post_setter_observer_slots() {
        if slot
            .compare_exchange(0, value, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("all runtime-text post-setter observer slots are occupied".to_string())
}

fn post_setter_observer_slots() -> [&'static AtomicUsize; 4] {
    [
        &POST_SETTER_OBSERVER_0,
        &POST_SETTER_OBSERVER_1,
        &POST_SETTER_OBSERVER_2,
        &POST_SETTER_OBSERVER_3,
    ]
}

/// Non-owning observer invoked for a validated CTextBox owner immediately
/// before A25 removes its identity/latest rows and before the original deleting
/// destructor runs. The pointer is process-local and valid only for the call.
pub type PreDestructorObserver = unsafe extern "system" fn(owner: usize, flags: u32);

/// Register one pre-destructor observer without installing another detour.
pub fn register_pre_destructor_observer(observer: PreDestructorObserver) -> Result<(), String> {
    PRE_DESTRUCTOR_OBSERVER
        .compare_exchange(0, observer as usize, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| "a runtime-text pre-destructor observer is already registered".to_string())
}

/// Exact live owner pointers currently registered under an authored element
/// name. Pointers are process-local and must never enter replay payloads.
pub fn observed_textbox_owners(authored_name: &str) -> Vec<usize> {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return Vec::new();
    };
    let mut owners = state
        .identities
        .iter()
        .filter_map(|(owner, identity)| (identity.element_id == authored_name).then_some(*owner))
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners
}

impl A729RuntimeTextHookSource {
    pub fn diagnostics(&self) -> HookDiagnostics {
        let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
            return HookDiagnostics::default();
        };
        HookDiagnostics {
            registered_widget_count: state.identities.len() as u64,
            observed_context_count: state
                .identities
                .keys()
                .filter(|widget| state.latest.contains_key(widget))
                .count() as u64,
            rejected_identity_count: state.rejected_identity,
            rejected_text_count: state.rejected_text,
            evicted_pending_count: state.evicted_pending,
            baseline_complete: BASELINE_COMPLETE.load(Ordering::Acquire),
        }
    }

    fn rows(&self, pov: &str) -> Result<Vec<RuntimeTextInput>, String> {
        let state = STATE
            .get()
            .ok_or_else(|| "a729 runtime-text hook state is not initialized".to_string())?
            .lock()
            .map_err(|_| "a729 runtime-text hook state mutex poisoned".to_string())?;
        let mut rows = BTreeMap::new();
        for (context_id, (identity, value)) in &state.pinned {
            rows.insert(
                context_id.clone(),
                runtime_text_input(context_id.clone(), identity, value, pov),
            );
        }
        for (widget, identity) in &state.identities {
            let Some(value) = state.latest.get(widget) else {
                continue;
            };
            let context_id = textbox_context_id(identity);
            rows.insert(
                context_id.clone(),
                runtime_text_input(context_id, identity, value, pov),
            );
        }
        Ok(rows.into_values().collect())
    }
}

fn textbox_context_id(identity: &WidgetIdentity) -> String {
    format!(
        "{}::{}::{}",
        identity.view_id, identity.element_id, identity.ordinal
    )
}

fn runtime_text_input(
    context_id: String,
    identity: &WidgetIdentity,
    value: &SetterValue,
    pov: &str,
) -> RuntimeTextInput {
    RuntimeTextInput {
        context_id,
        pov_country_or_global: pov.to_string(),
        view_id: identity.view_id.clone(),
        element_id: identity.element_id.clone(),
        source: if identity.source_token.is_empty() {
            RuntimeTextSourceIdentity::SetterCallerRva(value.caller_rva)
        } else {
            RuntimeTextSourceIdentity::AuthoredToken(identity.source_token.clone())
        },
        final_text_bytes: value.bytes.clone(),
        setter_caller_rva: value.caller_rva,
        formatter_observed: false,
        layout: RuntimeTextLayout::AuthoredReproducible,
    }
}

impl ExactRuntimeTextSource for A729RuntimeTextHookSource {
    fn is_exact_a729_runtime_text_source(&self) -> bool {
        INSTALLED.load(Ordering::Acquire)
    }

    fn completeness_record_identity(&self, frame_hour: u64) -> Result<String, String> {
        let diagnostics = self.diagnostics();
        if diagnostics.rejected_identity_count != 0
            || diagnostics.rejected_text_count != 0
            || diagnostics.evicted_pending_count != 0
            || !diagnostics.baseline_complete
        {
            return Err(format!(
                "a729 hook completeness violated: identity_rejects={} text_rejects={} evictions={} baseline_complete={}",
                diagnostics.rejected_identity_count,
                diagnostics.rejected_text_count,
                diagnostics.evicted_pending_count,
                diagnostics.baseline_complete,
            ));
        }
        Ok(format!(
            "a25:a729:{frame_hour}:registered={}:observed={}",
            diagnostics.registered_widget_count, diagnostics.observed_context_count
        ))
    }

    fn expected_context_count(&self, _: u64) -> Result<u32, String> {
        let count = self.diagnostics().observed_context_count;
        u32::try_from(count).map_err(|_| "runtime-text context count exceeds u32".to_string())
    }

    fn collect_frame(
        &self,
        _: u64,
        pov_country_or_global: &str,
    ) -> Result<Vec<RuntimeTextInput>, String> {
        self.rows(pov_country_or_global)
    }
}

/// Install and individually enable all three exact hooks after verifying the
/// pinned a729 sentinels. No hook is created when any sentinel differs.
pub fn install_a729_runtime_text_hooks() -> Result<InstalledHookAddresses, String> {
    if INSTALLED.load(Ordering::Acquire) {
        let base = MODULE_BASE.load(Ordering::Acquire);
        return Ok(addresses(base));
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".to_string());
    }
    let addresses = addresses(base);
    verify_sentinel(
        addresses.textbox_registration,
        &REGISTRATION_SENTINEL,
        "textbox registration",
    )?;
    verify_sentinel(addresses.text_setter, &SETTER_SENTINEL, "final text setter")?;
    verify_sentinel(
        addresses.textbox_destructor,
        &DESTRUCTOR_SENTINEL,
        "textbox destructor",
    )?;

    STATE.get_or_init(|| Mutex::new(HookState::default()));
    BASELINE_COMPLETE.store(false, Ordering::Release);
    let registration_original = unsafe {
        MinHook::create_hook(
            addresses.textbox_registration as *mut c_void,
            hook_textbox_registration as *mut c_void,
        )
    }
    .map_err(|status| format!("textbox registration hook creation failed: {status}"))?;
    let setter_original = match unsafe {
        MinHook::create_hook(
            addresses.text_setter as *mut c_void,
            hook_text_setter as *mut c_void,
        )
    } {
        Ok(value) => value,
        Err(status) => {
            let _ = unsafe { MinHook::remove_hook(addresses.textbox_registration as *mut c_void) };
            return Err(format!("text setter hook creation failed: {status}"));
        }
    };
    let destructor_original = match unsafe {
        MinHook::create_hook(
            addresses.textbox_destructor as *mut c_void,
            hook_textbox_destructor as *mut c_void,
        )
    } {
        Ok(value) => value,
        Err(status) => {
            let _ = unsafe { MinHook::remove_hook(addresses.text_setter as *mut c_void) };
            let _ = unsafe { MinHook::remove_hook(addresses.textbox_registration as *mut c_void) };
            return Err(format!("textbox destructor hook creation failed: {status}"));
        }
    };
    ORIGINAL_REGISTRATION.store(registration_original as usize, Ordering::Release);
    ORIGINAL_SETTER.store(setter_original as usize, Ordering::Release);
    ORIGINAL_DESTRUCTOR.store(destructor_original as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);

    for target in [
        addresses.textbox_registration,
        addresses.text_setter,
        addresses.textbox_destructor,
    ] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            for enabled in [
                addresses.textbox_registration,
                addresses.text_setter,
                addresses.textbox_destructor,
            ] {
                let _ = unsafe { MinHook::disable_hook(enabled as *mut c_void) };
                let _ = unsafe { MinHook::remove_hook(enabled as *mut c_void) };
            }
            return Err(format!(
                "runtime-text hook enable failed at 0x{target:X}: {status}"
            ));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(addresses)
}

/// Remove all A25 detours before the collector DLL is unloaded. This is
/// retry-safe after a partially completed teardown: already-disabled and
/// already-removed targets count as success, while any other MinHook failure
/// leaves `INSTALLED` set so the caller can keep the DLL pinned and retry.
pub fn uninstall_a729_runtime_text_hooks() -> Result<(), String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("runtime-text module base is unavailable during uninstall".into());
    }
    let addresses = addresses(base);
    for target in [
        addresses.textbox_registration,
        addresses.text_setter,
        addresses.textbox_destructor,
    ] {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                return Err(format!(
                    "runtime-text hook disable failed at 0x{target:X}: {status}"
                ));
            }
        }
    }
    for target in [
        addresses.textbox_registration,
        addresses.text_setter,
        addresses.textbox_destructor,
    ] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                return Err(format!(
                    "runtime-text hook removal failed at 0x{target:X}: {status}"
                ));
            }
        }
    }
    INSTALLED.store(false, Ordering::Release);
    BASELINE_COMPLETE.store(false, Ordering::Release);
    ORIGINAL_REGISTRATION.store(0, Ordering::Release);
    ORIGINAL_SETTER.store(0, Ordering::Release);
    ORIGINAL_DESTRUCTOR.store(0, Ordering::Release);
    for observer in post_setter_observer_slots() {
        observer.store(0, Ordering::Release);
    }
    PRE_DESTRUCTOR_OBSERVER.store(0, Ordering::Release);
    if let Some(state) = STATE.get() {
        let mut state = state
            .lock()
            .map_err(|_| "runtime-text hook state mutex poisoned during uninstall".to_string())?;
        *state = HookState::default();
    }
    MODULE_BASE.store(0, Ordering::Release);
    Ok(())
}

/// Seed every textbox that existed before hook installation from one exact,
/// full-coverage heap-sweep result for the fixed-window CTernary vtable. The
/// caller must not invoke this for a degraded/skipped-range/not-yet-warm sweep.
/// Live hooks are already enabled, so objects created after the sweep began are
/// admitted by the normal registration/setter path while this baseline covers
/// the pre-existing set.
pub fn seed_existing_textbox_maps(
    map_addresses: &[usize],
) -> Result<BaselineSeedDiagnostics, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("runtime-text hooks must be installed before baseline seeding".into());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("runtime-text module base is unavailable".into());
    }
    if BASELINE_COMPLETE.load(Ordering::Acquire) {
        let state = STATE
            .get()
            .ok_or_else(|| "runtime-text hook state is not initialized".to_string())?
            .lock()
            .map_err(|_| "runtime-text hook state mutex poisoned".to_string())?;
        return Ok(BaselineSeedDiagnostics {
            validated_map_count: 0,
            seeded_widget_count: state.identities.len() as u64,
            retained_hook_value_count: state.latest.len() as u64,
        });
    }

    let mut maps = map_addresses.to_vec();
    maps.sort_unstable();
    maps.dedup();
    let mut candidates = Vec::new();
    let mut validated_map_count = 0u64;
    for map in maps {
        if read_usize(map) != base.checked_add(TEXTBOX_TERNARY_VTABLE_RVA) {
            continue;
        }
        let Some(window) = map.checked_sub(FIXED_WINDOW_MAP_OFFSET) else {
            continue;
        };
        let Some(window_type) = read_usize(
            window
                .checked_add(FIXED_WINDOW_CGUI_TYPE_OFFSET)
                .ok_or("fixed-window type address overflow")?,
        ) else {
            continue;
        };
        let Some(view_bytes) = read_engine_string(
            window_type
                .checked_add(CGUI_TYPE_NAME_OFFSET)
                .ok_or("fixed-window view address overflow")?,
        ) else {
            continue;
        };
        let Ok(view_id) = String::from_utf8(view_bytes) else {
            continue;
        };
        if view_id.is_empty() {
            continue;
        }
        let owners = enumerate_textbox_map_owners(map)?;
        validated_map_count = validated_map_count.saturating_add(1);
        for owner in owners {
            let (_, element_id, source_token) = registration_identity(base, map, owner)
                .ok_or_else(|| {
                    format!("validated textbox map contains invalid owner 0x{owner:X}")
                })?;
            let child = read_usize(
                owner
                    .checked_add(TEXTBOX_GRAPHICS_CHILD_OFFSET)
                    .ok_or("textbox child address overflow")?,
            )
            .ok_or_else(|| format!("textbox 0x{owner:X} has no graphics child"))?;
            read_post_layout_text(child)
                .ok_or_else(|| format!("textbox 0x{owner:X} post-layout text is unreadable"))?;
            candidates.push(BaselineCandidate {
                map,
                owner,
                view_id: view_id.clone(),
                element_id,
                source_token,
            });
        }
    }

    candidates.sort_by(|left, right| {
        (&left.view_id, &left.element_id, left.owner).cmp(&(
            &right.view_id,
            &right.element_id,
            right.owner,
        ))
    });
    let state = STATE
        .get()
        .ok_or_else(|| "runtime-text hook state is not initialized".to_string())?;
    let mut state = state
        .lock()
        .map_err(|_| "runtime-text hook state mutex poisoned".to_string())?;
    for candidate in &candidates {
        let Some((view_id, element_id, source_token)) =
            registration_identity(base, candidate.map, candidate.owner)
        else {
            return Err(format!(
                "baseline textbox 0x{:X} changed or was destroyed before admission",
                candidate.owner
            ));
        };
        if (view_id, element_id, source_token)
            != (
                candidate.view_id.clone(),
                candidate.element_id.clone(),
                candidate.source_token.clone(),
            )
        {
            return Err(format!(
                "baseline textbox 0x{:X} identity changed before admission",
                candidate.owner
            ));
        }
        if candidate.source_token.is_empty() && !state.latest.contains_key(&candidate.owner) {
            return Err(format!(
                "pre-hook textbox 0x{:X} has no authored token or observed setter caller",
                candidate.owner
            ));
        }
    }
    let mut retained_hook_value_count = 0u64;
    for candidate in candidates {
        if !state.identities.contains_key(&candidate.owner) {
            let ordinal = state
                .next_ordinal
                .entry((candidate.view_id.clone(), candidate.element_id.clone()))
                .or_default();
            let assigned = *ordinal;
            *ordinal = ordinal.saturating_add(1);
            state.identities.insert(
                candidate.owner,
                WidgetIdentity {
                    view_id: candidate.view_id,
                    element_id: candidate.element_id,
                    source_token: candidate.source_token,
                    ordinal: assigned,
                },
            );
        }
        match state.latest.entry(candidate.owner) {
            Entry::Occupied(_) => {
                retained_hook_value_count = retained_hook_value_count.saturating_add(1);
            }
            Entry::Vacant(entry) => {
                let child = read_usize(
                    candidate
                        .owner
                        .checked_add(TEXTBOX_GRAPHICS_CHILD_OFFSET)
                        .ok_or("textbox child address overflow")?,
                )
                .ok_or_else(|| format!("textbox 0x{:X} has no graphics child", candidate.owner))?;
                let final_text_bytes = read_post_layout_text(child)
                .ok_or_else(|| {
                    format!(
                        "textbox 0x{:X} post-layout text changed or became unreadable before admission",
                        candidate.owner
                    )
                })?;
                entry.insert(SetterValue {
                    bytes: final_text_bytes,
                    // The baseline proves the authored source and exact current
                    // child bytes, but no historical setter return address.
                    caller_rva: 0,
                });
            }
        }
    }
    let seeded_widget_count = state.identities.len() as u64;
    BASELINE_COMPLETE.store(true, Ordering::Release);
    Ok(BaselineSeedDiagnostics {
        validated_map_count,
        seeded_widget_count,
        retained_hook_value_count,
    })
}

fn enumerate_textbox_map_owners(map: usize) -> Result<Vec<usize>, String> {
    let expected_count = read_u32(map.checked_add(0x14).ok_or("map count address overflow")?)
        .ok_or("textbox map count unreadable")? as usize;
    if expected_count > MAX_PENDING {
        return Err("textbox map terminal count exceeds bound".into());
    }
    let root = read_usize(map.checked_add(8).ok_or("map root address overflow")?)
        .ok_or("textbox map root unreadable")?;
    if root == 0 {
        return if expected_count == 0 {
            Ok(Vec::new())
        } else {
            Err("textbox map has null root with nonzero count".into())
        };
    }
    let mut stack = vec![root];
    let mut seen = BTreeSet::new();
    let mut owners = Vec::with_capacity(expected_count);
    while let Some(node) = stack.pop() {
        if node == 0 || !seen.insert(node) {
            continue;
        }
        if seen.len() > MAX_TERNARY_NODES {
            return Err("textbox map node count exceeds bound".into());
        }
        let bytes = rpm(node, 0x28).ok_or("textbox map node unreadable")?;
        let qword = |offset: usize| -> usize {
            usize::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
        };
        let value = qword(0);
        if value != 0 {
            owners.push(value);
        }
        stack.push(qword(0x10));
        stack.push(qword(0x18));
        stack.push(qword(0x20));
    }
    owners.sort_unstable();
    owners.dedup();
    if owners.len() != expected_count {
        return Err(format!(
            "textbox map terminal mismatch: expected {expected_count}, decoded {}",
            owners.len()
        ));
    }
    Ok(owners)
}

/// Production entry used by the shared collector once d76 is scheduled.
/// Installation is idempotent, signature-gated, and followed immediately by
/// authenticated collection from the sole hook owner.
pub fn install_collect_and_encode(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
    pov_country_or_global: &str,
) -> Result<Vec<u8>, crate::CarrierError> {
    install_a729_runtime_text_hooks().map_err(crate::CarrierError::Source)?;
    crate::collect_and_encode(
        &A729RuntimeTextHookSource,
        frame_hour,
        elapsed_wall_ms,
        status_flags,
        pov_country_or_global,
    )
}

fn addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        textbox_registration: base + TEXTBOX_REGISTRATION_RVA as usize,
        text_setter: base + TEXT_SETTER_RVA as usize,
        textbox_destructor: base + TEXTBOX_DESTRUCTOR_RVA as usize,
    }
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let actual =
        rpm(address, expected.len()).ok_or_else(|| format!("{label} sentinel unreadable"))?;
    if actual != expected {
        return Err(format!("{label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

unsafe extern "system" fn hook_textbox_registration(map: usize, key: usize, widget: usize) {
    let original = ORIGINAL_REGISTRATION.load(Ordering::Acquire);
    if original != 0 {
        let original: RegistrationFn = unsafe { mem::transmute(original) };
        unsafe { original(map, key, widget) };
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let identity = registration_identity(base, map, widget);
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        match identity {
            Some((view_id, element_id, source_token)) => {
                let ordinal = state
                    .next_ordinal
                    .entry((view_id.clone(), element_id.clone()))
                    .or_default();
                let assigned = *ordinal;
                *ordinal = ordinal.saturating_add(1);
                state.identities.insert(
                    widget,
                    WidgetIdentity {
                        view_id,
                        element_id,
                        source_token,
                        ordinal: assigned,
                    },
                );
            }
            None => state.rejected_identity = state.rejected_identity.saturating_add(1),
        }
    }
}

unsafe extern "system" fn hook_text_setter(widget: usize, text_object: usize) {
    let caller_rva = immediate_game_return_rva();
    // Admit the setter input only as proof that this is a readable final-text
    // invocation. Replay bytes are read after the original returns, from the
    // CInstantTextObject string selected by the engine's layout virtual.
    let setter_input = read_engine_string(text_object);
    let owner = setter_owner(widget);
    let original = ORIGINAL_SETTER.load(Ordering::Acquire);
    if original != 0 {
        let original: SetterFn = unsafe { mem::transmute(original) };
        unsafe { original(widget, text_object) };
    }
    let post_layout_bytes = (original != 0)
        .then(|| read_post_layout_text(widget))
        .flatten();
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        match (owner, setter_input, post_layout_bytes, caller_rva) {
            (Some(owner), Some(_), Some(bytes), Some(caller_rva)) => {
                if state.latest.len() >= MAX_PENDING && !state.latest.contains_key(&owner) {
                    state.evicted_pending = state.evicted_pending.saturating_add(1);
                } else {
                    state
                        .latest
                        .insert(owner, SetterValue { bytes, caller_rva });
                }
            }
            _ => state.rejected_text = state.rejected_text.saturating_add(1),
        }
    }
    if let (Some(owner), Some(caller_rva)) = (owner, caller_rva) {
        for observer in post_setter_observer_slots() {
            let observer = observer.load(Ordering::Acquire);
            if observer != 0 {
                let observer: PostSetterObserver = unsafe { mem::transmute(observer) };
                unsafe { observer(owner, widget, caller_rva) };
            }
        }
    }
}

unsafe extern "system" fn hook_textbox_destructor(widget: usize, flags: u32) -> usize {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let is_valid_owner = base != 0 && read_usize(widget) == base.checked_add(TEXTBOX_VTABLE_RVA);
    if is_valid_owner {
        let observer = PRE_DESTRUCTOR_OBSERVER.load(Ordering::Acquire);
        if observer != 0 {
            let observer: PreDestructorObserver = unsafe { mem::transmute(observer) };
            unsafe { observer(widget, flags) };
        }
    }
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.identities.remove(&widget);
        state.latest.remove(&widget);
    }
    let original = ORIGINAL_DESTRUCTOR.load(Ordering::Acquire);
    if original == 0 {
        return widget;
    }
    let original: DestructorFn = unsafe { mem::transmute(original) };
    unsafe { original(widget, flags) }
}

fn registration_identity(
    base: usize,
    map: usize,
    widget: usize,
) -> Option<(String, String, String)> {
    if base == 0 || map < FIXED_WINDOW_MAP_OFFSET || widget == 0 {
        return None;
    }
    if read_usize(map)? != base.checked_add(TEXTBOX_TERNARY_VTABLE_RVA)? {
        return None;
    }
    let window = map.checked_sub(FIXED_WINDOW_MAP_OFFSET)?;
    let window_type = read_usize(window.checked_add(FIXED_WINDOW_CGUI_TYPE_OFFSET)?)?;
    let view_id = read_engine_string(window_type.checked_add(CGUI_TYPE_NAME_OFFSET)?)?;
    let element_id = read_engine_string(widget.checked_add(TEXTBOX_AUTHORED_NAME_OFFSET)?)?;
    let textbox_type = read_usize(widget.checked_add(TEXTBOX_CGUI_TYPE_OFFSET)?)?;
    let source_token =
        read_engine_string(textbox_type.checked_add(TEXTBOX_TYPE_SOURCE_TOKEN_OFFSET)?)?;
    Some((
        String::from_utf8(view_id).ok()?,
        String::from_utf8(element_id).ok()?,
        String::from_utf8(source_token).ok()?,
    ))
}

fn setter_owner(setter_widget: usize) -> Option<usize> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let owner = read_usize(setter_widget.checked_add(TEXTBOX_CHILD_OWNER_OFFSET)?)?;
    if read_usize(owner)? != base.checked_add(TEXTBOX_VTABLE_RVA)? {
        return None;
    }
    Some(owner)
}

fn immediate_game_return_rva() -> Option<u32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let end = base.checked_add(55_680_632)?;
    let mut stack = [ptr::null_mut::<c_void>(); 8];
    let count = unsafe {
        RtlCaptureStackBackTrace(0, stack.len() as u32, stack.as_mut_ptr(), ptr::null_mut())
    } as usize;
    stack[..count]
        .iter()
        .map(|address| *address as usize)
        .find(|address| {
            *address >= base && *address < end && *address != base + TEXT_SETTER_RVA as usize
        })
        .and_then(|address| u32::try_from(address.checked_sub(base)?).ok())
}

fn read_engine_string(address: usize) -> Option<Vec<u8>> {
    let len = read_usize(address.checked_add(0x10)?)?;
    let capacity = read_usize(address.checked_add(0x18)?)?;
    if len > capacity || len > crate::MAX_TEXT_BYTES {
        return None;
    }
    if len == 0 {
        return Some(Vec::new());
    }
    let data = if capacity <= 15 {
        address
    } else {
        read_usize(address)?
    };
    rpm(data, len)
}

/// Process-local identity needed by exact cross-lane joins such as XUI21.
/// The owner pointer is accepted only as a lookup key and is never serialized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedTextboxContext {
    pub context_id: String,
    pub view_id: String,
    pub element_id: String,
}

/// Resolve one currently live, setter-observed textbox owner to the same A25
/// context id that `rows()` will emit. Returning `None` keeps consumers
/// fail-closed when registration, setter output, or ownership is unavailable.
pub fn observed_textbox_context(owner: usize) -> Option<ObservedTextboxContext> {
    let state = STATE.get()?.lock().ok()?;
    let identity = state.identities.get(&owner)?;
    state.latest.get(&owner)?;
    Some(ObservedTextboxContext {
        context_id: textbox_context_id(identity),
        view_id: identity.view_id.clone(),
        element_id: identity.element_id.clone(),
    })
}

/// Pin the exact owner row so d76 can still emit it after the transient event
/// window is destroyed. This is required for historical d75 lifecycle rows:
/// a context id without a surviving A25 row is not a replay-admissible join.
pub fn pin_observed_textbox_context(owner: usize) -> Option<ObservedTextboxContext> {
    let mut state = STATE.get()?.lock().ok()?;
    pin_textbox_context(&mut state, owner)
}

fn pin_textbox_context(state: &mut HookState, owner: usize) -> Option<ObservedTextboxContext> {
    let identity = state.identities.get(&owner)?.clone();
    let value = state.latest.get(&owner)?.clone();
    let context_id = textbox_context_id(&identity);
    if state.pinned.len() >= MAX_PENDING && !state.pinned.contains_key(&context_id) {
        state.evicted_pending = state.evicted_pending.saturating_add(1);
        return None;
    }
    state
        .pinned
        .insert(context_id.clone(), (identity.clone(), value));
    Some(ObservedTextboxContext {
        context_id,
        view_id: identity.view_id,
        element_id: identity.element_id,
    })
}

fn read_post_layout_text(graphics_child: usize) -> Option<Vec<u8>> {
    let layout = read_usize(graphics_child.checked_add(GRAPHICS_CHILD_LAYOUT_OBJECT_OFFSET)?)?;
    read_engine_string(layout.checked_add(LAYOUT_OBJECT_FINAL_TEXT_OFFSET)?)
}

fn read_usize(address: usize) -> Option<usize> {
    Some(usize::from_le_bytes(
        rpm(address, mem::size_of::<usize>())?.try_into().ok()?,
    ))
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

    fn write_usize(bytes: &mut [u8], offset: usize, value: usize) {
        bytes[offset..offset + mem::size_of::<usize>()].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn exact_ternary_layout_enumerates_all_terminal_owners() {
        let mut greater = Box::new([0u8; 0x28]);
        write_usize(&mut greater[..], 0, 0x2222);
        let greater_address = greater.as_ptr() as usize;

        let mut root = Box::new([0u8; 0x28]);
        write_usize(&mut root[..], 0, 0x1111);
        write_usize(&mut root[..], 0x18, greater_address);
        let root_address = root.as_ptr() as usize;

        let mut map = Box::new([0u8; 0x20]);
        write_usize(&mut map[..], 8, root_address);
        write_u32(&mut map[..], 0x14, 2);

        assert_eq!(
            enumerate_textbox_map_owners(map.as_ptr() as usize).unwrap(),
            vec![0x1111, 0x2222]
        );
    }

    #[test]
    fn exact_ternary_layout_rejects_terminal_count_mismatch() {
        let mut root = Box::new([0u8; 0x28]);
        write_usize(&mut root[..], 0, 0x1111);

        let mut map = Box::new([0u8; 0x20]);
        write_usize(&mut map[..], 8, root.as_ptr() as usize);
        write_u32(&mut map[..], 0x14, 2);

        assert!(enumerate_textbox_map_owners(map.as_ptr() as usize).is_err());
    }

    #[test]
    fn post_layout_reader_uses_cinstanttext_selected_string() {
        let mut selected = Box::new([0u8; 0x1b0]);
        selected[0x188..0x18e].copy_from_slice(b"A\nB...");
        write_usize(&mut selected[..], 0x198, 6);
        write_usize(&mut selected[..], 0x1a0, 15);

        let mut child = Box::new([0u8; 0xd0]);
        write_usize(&mut child[..], 0xc8, selected.as_ptr() as usize);

        assert_eq!(
            read_post_layout_text(child.as_ptr() as usize).unwrap(),
            b"A\nB..."
        );
    }

    #[test]
    fn pinned_event_context_survives_transient_owner_destruction() {
        let owner = 0x1234;
        let mut state = HookState::default();
        state.identities.insert(
            owner,
            WidgetIdentity {
                view_id: "EventWindow".into(),
                element_id: "Description".into(),
                source_token: "probe.1.d".into(),
                ordinal: 7,
            },
        );
        state.latest.insert(
            owner,
            SetterValue {
                bytes: b"Scoped description".to_vec(),
                caller_rva: 0x123456,
            },
        );
        let context = pin_textbox_context(&mut state, owner).expect("pin context");
        state.identities.remove(&owner);
        state.latest.remove(&owner);
        let (identity, value) = state
            .pinned
            .get(&context.context_id)
            .expect("pinned row survives owner destruction");
        let row = runtime_text_input(context.context_id, identity, value, "GER");
        assert_eq!(row.final_text_bytes, b"Scoped description");
        assert_eq!(row.pov_country_or_global, "GER");
    }
}
