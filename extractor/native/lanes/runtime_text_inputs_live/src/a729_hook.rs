use crate::owner_glyph_capture::{
    AdmittedOwnerGlyphCapture, FontAssetReceipt, InlineIconAssetReceipt,
    OwnerGlyphCaptureAdmission, OwnerScopedGlyphCapture,
};
use crate::{
    RuntimeTextInput, RuntimeTextLayout, RuntimeTextSourceIdentity, SPRITE_SETTER_RVA,
    TEXTBOX_DESTRUCTOR_RVA, TEXTBOX_REGISTRATION_RVA, TEXT_SETTER_RVA,
};
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
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

const SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x57,
];
const REGISTRATION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x80, 0x79, 0x10, 0x01, 0x49, 0x8B,
];
const DESTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x8B, 0xDA, 0x48, 0x8B, 0xF9, 0xE8,
];
const SPRITE_SETTER_SENTINEL: [u8; 20] = [
    0x48, 0x85, 0xc9, 0x0f, 0x84, 0x47, 0x01, 0x00, 0x00, 0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89,
    0x74, 0x24, 0x20, 0x57,
];

type SetterFn = unsafe extern "system" fn(usize, usize);
type RegistrationFn = unsafe extern "system" fn(usize, usize, usize);
type DestructorFn = unsafe extern "system" fn(usize, u32) -> usize;
type SpriteSetterFn = unsafe extern "system" fn(usize, usize, u8);

static ORIGINAL_SETTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_REGISTRATION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DESTRUCTOR: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SPRITE_SETTER: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());
const CALLBACK_QUIESCE_TIMEOUT: Duration = Duration::from_secs(30);
static POST_SETTER_OBSERVER_0: AtomicUsize = AtomicUsize::new(0);
static POST_SETTER_OBSERVER_1: AtomicUsize = AtomicUsize::new(0);
static POST_SETTER_OBSERVER_2: AtomicUsize = AtomicUsize::new(0);
static POST_SETTER_OBSERVER_3: AtomicUsize = AtomicUsize::new(0);
static PRE_SETTER_OBSERVER_0: AtomicUsize = AtomicUsize::new(0);
static PRE_SETTER_OBSERVER_1: AtomicUsize = AtomicUsize::new(0);
static PRE_SETTER_OBSERVER_2: AtomicUsize = AtomicUsize::new(0);
static PRE_SETTER_OBSERVER_3: AtomicUsize = AtomicUsize::new(0);
static PRE_DESTRUCTOR_OBSERVER: AtomicUsize = AtomicUsize::new(0);
static SPRITE_SETTER_OBSERVER_0: AtomicUsize = AtomicUsize::new(0);
static SPRITE_SETTER_OBSERVER_1: AtomicUsize = AtomicUsize::new(0);
static SPRITE_SETTER_OBSERVER_2: AtomicUsize = AtomicUsize::new(0);
static SPRITE_SETTER_OBSERVER_3: AtomicUsize = AtomicUsize::new(0);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

struct InFlightCallbackGuard;

impl InFlightCallbackGuard {
    fn enter() -> Self {
        IN_FLIGHT_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for InFlightCallbackGuard {
    fn drop(&mut self) {
        IN_FLIGHT_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

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
    pinned: BTreeMap<
        String,
        (
            WidgetIdentity,
            SetterValue,
            Option<AdmittedOwnerGlyphCapture>,
        ),
    >,
    captured_layouts: BTreeMap<String, AdmittedOwnerGlyphCapture>,
    next_ordinal: BTreeMap<(String, String), u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub textbox_registration: usize,
    pub text_setter: usize,
    pub textbox_destructor: usize,
    pub sprite_setter: usize,
}

pub type PostSetterObserver =
    unsafe extern "system" fn(owner: usize, setter_widget: usize, caller_rva: u32);

pub type PreSetterObserver = unsafe extern "system" fn(
    owner: usize,
    setter_widget: usize,
    text_object: usize,
    caller_rva: u32,
);

pub type SpriteSetterObserver =
    unsafe extern "system" fn(widget: usize, string_object: usize, mode: u8, caller_rva: u32);

pub fn register_sprite_setter_observer(observer: SpriteSetterObserver) -> Result<(), String> {
    let value = observer as usize;
    if sprite_setter_observer_slots()
        .iter()
        .any(|slot| slot.load(Ordering::Acquire) == value)
    {
        return Ok(());
    }
    for slot in sprite_setter_observer_slots() {
        if slot
            .compare_exchange(0, value, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("all shared sprite-setter observer slots are occupied".to_string())
}

pub fn unregister_sprite_setter_observer(observer: SpriteSetterObserver) {
    let value = observer as usize;
    for slot in sprite_setter_observer_slots() {
        let _ = slot.compare_exchange(value, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}

fn sprite_setter_observer_slots() -> [&'static AtomicUsize; 4] {
    [
        &SPRITE_SETTER_OBSERVER_0,
        &SPRITE_SETTER_OBSERVER_1,
        &SPRITE_SETTER_OBSERVER_2,
        &SPRITE_SETTER_OBSERVER_3,
    ]
}

pub fn register_pre_setter_observer(observer: PreSetterObserver) -> Result<(), String> {
    let value = observer as usize;
    if pre_setter_observer_slots()
        .iter()
        .any(|slot| slot.load(Ordering::Acquire) == value)
    {
        return Ok(());
    }
    for slot in pre_setter_observer_slots() {
        if slot
            .compare_exchange(0, value, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("all runtime-text pre-setter observer slots are occupied".to_string())
}

pub fn unregister_pre_setter_observer(observer: PreSetterObserver) {
    let value = observer as usize;
    for slot in pre_setter_observer_slots() {
        let _ = slot.compare_exchange(value, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}

fn pre_setter_observer_slots() -> [&'static AtomicUsize; 4] {
    [
        &PRE_SETTER_OBSERVER_0,
        &PRE_SETTER_OBSERVER_1,
        &PRE_SETTER_OBSERVER_2,
        &PRE_SETTER_OBSERVER_3,
    ]
}

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

pub fn unregister_post_setter_observer(observer: PostSetterObserver) {
    let value = observer as usize;
    for slot in post_setter_observer_slots() {
        let _ = slot.compare_exchange(value, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}

fn post_setter_observer_slots() -> [&'static AtomicUsize; 4] {
    [
        &POST_SETTER_OBSERVER_0,
        &POST_SETTER_OBSERVER_1,
        &POST_SETTER_OBSERVER_2,
        &POST_SETTER_OBSERVER_3,
    ]
}

pub type PreDestructorObserver = unsafe extern "system" fn(owner: usize, flags: u32);

pub fn register_pre_destructor_observer(observer: PreDestructorObserver) -> Result<(), String> {
    PRE_DESTRUCTOR_OBSERVER
        .compare_exchange(0, observer as usize, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| "a runtime-text pre-destructor observer is already registered".to_string())
}

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

fn textbox_context_id(identity: &WidgetIdentity) -> String {
    format!(
        "{}::{}::{}",
        identity.view_id, identity.element_id, identity.ordinal
    )
}

pub fn install_a729_runtime_text_hooks() -> Result<InstalledHookAddresses, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "runtime-text hook lifecycle mutex poisoned".to_string())?;
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
    verify_sentinel(
        addresses.sprite_setter,
        &SPRITE_SETTER_SENTINEL,
        "generic sprite setter",
    )?;

    STATE.get_or_init(|| Mutex::new(HookState::default()));
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
            let _ = unsafe { inline_detour::remove(addresses.textbox_registration as *mut c_void) };
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
            let _ = unsafe { inline_detour::remove(addresses.text_setter as *mut c_void) };
            let _ = unsafe { inline_detour::remove(addresses.textbox_registration as *mut c_void) };
            return Err(format!("textbox destructor hook creation failed: {status}"));
        }
    };
    let sprite_original = match unsafe {
        MinHook::create_hook(
            addresses.sprite_setter as *mut c_void,
            hook_sprite_setter as *mut c_void,
        )
    } {
        Ok(value) => value,
        Err(status) => {
            for target in [
                addresses.textbox_destructor,
                addresses.text_setter,
                addresses.textbox_registration,
            ] {
                let _ = unsafe { inline_detour::remove(target as *mut c_void) };
            }
            return Err(format!("sprite setter hook creation failed: {status}"));
        }
    };
    ORIGINAL_REGISTRATION.store(registration_original as usize, Ordering::Release);
    ORIGINAL_SETTER.store(setter_original as usize, Ordering::Release);
    ORIGINAL_DESTRUCTOR.store(destructor_original as usize, Ordering::Release);
    ORIGINAL_SPRITE_SETTER.store(sprite_original as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);

    for target in [
        addresses.textbox_registration,
        addresses.text_setter,
        addresses.textbox_destructor,
        addresses.sprite_setter,
    ] {
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            for enabled in [
                addresses.textbox_registration,
                addresses.text_setter,
                addresses.textbox_destructor,
                addresses.sprite_setter,
            ] {
                let _ = unsafe { inline_detour::disable(enabled as *mut c_void) };
            }
            wait_for_callback_quiescence(CALLBACK_QUIESCE_TIMEOUT)?;
            for created in [
                addresses.sprite_setter,
                addresses.textbox_destructor,
                addresses.text_setter,
                addresses.textbox_registration,
            ] {
                let _ = unsafe { inline_detour::remove(created as *mut c_void) };
            }
            ORIGINAL_REGISTRATION.store(0, Ordering::Release);
            ORIGINAL_SETTER.store(0, Ordering::Release);
            ORIGINAL_DESTRUCTOR.store(0, Ordering::Release);
            ORIGINAL_SPRITE_SETTER.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!(
                "runtime-text hook enable failed at 0x{target:X}: {status}"
            ));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(addresses)
}

pub fn uninstall_a729_runtime_text_hooks() -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "runtime-text hook lifecycle mutex poisoned".to_string())?;
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
        addresses.sprite_setter,
    ] {
        match unsafe { inline_detour::disable(target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                return Err(format!(
                    "runtime-text hook disable failed at 0x{target:X}: {status}"
                ));
            }
        }
    }
    wait_for_callback_quiescence(CALLBACK_QUIESCE_TIMEOUT)?;
    for target in [
        addresses.textbox_registration,
        addresses.text_setter,
        addresses.textbox_destructor,
        addresses.sprite_setter,
    ] {
        match unsafe { inline_detour::remove(target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                return Err(format!(
                    "runtime-text hook removal failed at 0x{target:X}: {status}"
                ));
            }
        }
    }
    INSTALLED.store(false, Ordering::Release);
    ORIGINAL_REGISTRATION.store(0, Ordering::Release);
    ORIGINAL_SETTER.store(0, Ordering::Release);
    ORIGINAL_DESTRUCTOR.store(0, Ordering::Release);
    ORIGINAL_SPRITE_SETTER.store(0, Ordering::Release);
    for observer in post_setter_observer_slots() {
        observer.store(0, Ordering::Release);
    }
    for observer in pre_setter_observer_slots() {
        observer.store(0, Ordering::Release);
    }
    PRE_DESTRUCTOR_OBSERVER.store(0, Ordering::Release);
    for observer in sprite_setter_observer_slots() {
        observer.store(0, Ordering::Release);
    }
    if let Some(state) = STATE.get() {
        let mut state = state
            .lock()
            .map_err(|_| "runtime-text hook state mutex poisoned during uninstall".to_string())?;
        *state = HookState::default();
    }
    MODULE_BASE.store(0, Ordering::Release);
    Ok(())
}

fn addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        textbox_registration: base + TEXTBOX_REGISTRATION_RVA as usize,
        text_setter: base + TEXT_SETTER_RVA as usize,
        textbox_destructor: base + TEXTBOX_DESTRUCTOR_RVA as usize,
        sprite_setter: base + SPRITE_SETTER_RVA as usize,
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
    let _in_flight = InFlightCallbackGuard::enter();
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
            None => {}
        }
    }
}

unsafe extern "system" fn hook_text_setter(widget: usize, text_object: usize) {
    let _in_flight = InFlightCallbackGuard::enter();
    let caller_rva = immediate_game_return_rva();
    let setter_input = read_engine_string(text_object);
    let owner = setter_owner(widget);
    if let (Some(owner), Some(_), Some(caller_rva)) = (owner, setter_input.as_ref(), caller_rva) {
        for observer in pre_setter_observer_slots() {
            let observer = observer.load(Ordering::Acquire);
            if observer != 0 {
                let observer: PreSetterObserver = unsafe { mem::transmute(observer) };
                unsafe { observer(owner, widget, text_object, caller_rva) };
            }
        }
    }
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
                if state.latest.len() < MAX_PENDING || state.latest.contains_key(&owner) {
                    let captured_context = state.identities.get(&owner).map(textbox_context_id);
                    if let Some(context_id) = captured_context {
                        state.captured_layouts.remove(&context_id);
                    }
                    state
                        .latest
                        .insert(owner, SetterValue { bytes, caller_rva });
                }
            }
            _ => {}
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

unsafe extern "system" fn hook_sprite_setter(widget: usize, string_object: usize, mode: u8) {
    let _in_flight = InFlightCallbackGuard::enter();
    if let Some(caller_rva) = immediate_game_return_rva() {
        for observer in sprite_setter_observer_slots() {
            let observer = observer.load(Ordering::Acquire);
            if observer != 0 {
                let observer: SpriteSetterObserver = unsafe { mem::transmute(observer) };
                unsafe { observer(widget, string_object, mode, caller_rva) };
            }
        }
    }
    let original = ORIGINAL_SPRITE_SETTER.load(Ordering::Acquire);
    if original != 0 {
        let original: SpriteSetterFn = unsafe { mem::transmute(original) };
        unsafe { original(widget, string_object, mode) };
    }
}

unsafe extern "system" fn hook_textbox_destructor(widget: usize, flags: u32) -> usize {
    let _in_flight = InFlightCallbackGuard::enter();
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
        if let Some(identity) = state.identities.remove(&widget) {
            let context_id = textbox_context_id(&identity);
            if !state.pinned.contains_key(&context_id) {
                state.captured_layouts.remove(&context_id);
            }
        }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedTextboxContext {
    pub context_id: String,
    pub view_id: String,
    pub element_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedOwnerRenderJoin {
    pub owner: usize,
    pub context_id: String,
    pub view_id: String,
    pub element_id: String,
    pub source: RuntimeTextSourceIdentity,
    pub selected_final_text_bytes: Vec<u8>,
    pub selected_final_text_sha256: String,
}

pub fn observed_owner_for_render_layout(layout_object: usize) -> Option<ObservedOwnerRenderJoin> {
    let state = STATE.get()?.lock().ok()?;
    let mut matched = None;
    for (owner, identity) in &state.identities {
        let child = read_usize(owner.checked_add(TEXTBOX_GRAPHICS_CHILD_OFFSET)?)?;
        let layout = read_usize(child.checked_add(GRAPHICS_CHILD_LAYOUT_OBJECT_OFFSET)?)?;
        if layout != layout_object {
            continue;
        }
        if matched.is_some() {
            return None;
        }
        let value = state.latest.get(owner)?;
        matched = Some(ObservedOwnerRenderJoin {
            owner: *owner,
            context_id: textbox_context_id(identity),
            view_id: identity.view_id.clone(),
            element_id: identity.element_id.clone(),
            source: if identity.source_token.is_empty() {
                RuntimeTextSourceIdentity::SetterCallerRva(value.caller_rva)
            } else {
                RuntimeTextSourceIdentity::AuthoredToken(identity.source_token.clone())
            },
            selected_final_text_bytes: value.bytes.clone(),
            selected_final_text_sha256: sha256_hex(&value.bytes),
        });
    }
    matched
}

pub fn admit_observed_owner_capture(
    owner: usize,
    capture: OwnerScopedGlyphCapture,
    admission: &mut OwnerGlyphCaptureAdmission,
    frame_hour: u64,
) -> Result<AdmittedOwnerGlyphCapture, String> {
    let state = STATE
        .get()
        .ok_or_else(|| "a729 runtime-text hook state is not initialized".to_string())?;
    let mut state = state
        .lock()
        .map_err(|_| "a729 runtime-text hook state mutex poisoned".to_string())?;
    let identity = state
        .identities
        .get(&owner)
        .ok_or_else(|| "owner-glyph capture references an unregistered textbox".to_string())?
        .clone();
    let value = state
        .latest
        .get(&owner)
        .ok_or_else(|| {
            "owner-glyph capture references a textbox without selected text".to_string()
        })?
        .clone();
    let context_id = textbox_context_id(&identity);
    let row = owner_capture_input(context_id.clone(), &identity, &value);
    let admitted = admission.admit(capture, &row, frame_hour)?;
    install_admitted_owner_capture_locked(
        &mut state,
        owner,
        &context_id,
        &sha256_hex(&value.bytes),
        &admitted,
    )?;
    Ok(admitted)
}

fn owner_capture_input(
    context_id: String,
    identity: &WidgetIdentity,
    value: &SetterValue,
) -> RuntimeTextInput {
    RuntimeTextInput {
        context_id,
        pov_country_or_global: "CAPTURE".to_string(),
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

pub fn install_admitted_owner_capture(
    owner: usize,
    expected_context_id: &str,
    expected_selected_final_text_sha256: &str,
    admitted: &AdmittedOwnerGlyphCapture,
) -> Result<(), String> {
    let state = STATE
        .get()
        .ok_or_else(|| "a729 runtime-text hook state is not initialized".to_string())?;
    let mut state = state
        .lock()
        .map_err(|_| "a729 runtime-text hook state mutex poisoned".to_string())?;
    install_admitted_owner_capture_locked(
        &mut state,
        owner,
        expected_context_id,
        expected_selected_final_text_sha256,
        admitted,
    )
}

fn install_admitted_owner_capture_locked(
    state: &mut HookState,
    owner: usize,
    expected_context_id: &str,
    expected_selected_final_text_sha256: &str,
    admitted: &AdmittedOwnerGlyphCapture,
) -> Result<(), String> {
    let identity = state
        .identities
        .get(&owner)
        .ok_or_else(|| "owner-glyph selected capture owner is no longer registered".to_string())?;
    let context_id = textbox_context_id(identity);
    let current_bytes = state
        .latest
        .get(&owner)
        .ok_or_else(|| {
            "owner-glyph selected capture owner no longer has selected text".to_string()
        })?
        .bytes
        .clone();
    if context_id != expected_context_id
        || sha256_hex(&current_bytes) != expected_selected_final_text_sha256
    {
        return Err("owner-glyph selected capture was invalidated before publication".into());
    }
    if state.captured_layouts.len() >= MAX_PENDING
        && !state.captured_layouts.contains_key(&context_id)
    {
        return Err("owner-glyph captured-layout capacity exceeded".into());
    }
    if let Some((_, pinned_value, pinned_capture)) = state.pinned.get_mut(&context_id) {
        if pinned_value.bytes == current_bytes {
            *pinned_capture = Some(admitted.clone());
        }
    }
    state.captured_layouts.insert(context_id, admitted.clone());
    Ok(())
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
                "runtime-text teardown timed out with {in_flight} in-flight callback(s); removal deferred"
            ));
        }
        std::thread::yield_now();
    }
}

pub fn observed_capture_asset_requirements(
    context_id: &str,
) -> Option<(Vec<FontAssetReceipt>, Vec<InlineIconAssetReceipt>)> {
    let state = STATE.get()?.lock().ok()?;
    let capture = state.captured_layouts.get(context_id).or_else(|| {
        state
            .pinned
            .get(context_id)
            .and_then(|(_, _, capture)| capture.as_ref())
    })?;
    Some((capture.fonts.clone(), capture.icon_assets.clone()))
}

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

pub fn pin_observed_textbox_context(owner: usize) -> Option<ObservedTextboxContext> {
    let mut state = STATE.get()?.lock().ok()?;
    pin_textbox_context(&mut state, owner)
}

fn pin_textbox_context(state: &mut HookState, owner: usize) -> Option<ObservedTextboxContext> {
    let identity = state.identities.get(&owner)?.clone();
    let value = state.latest.get(&owner)?.clone();
    let context_id = textbox_context_id(&identity);
    let captured_layout = state.captured_layouts.get(&context_id).cloned();
    if state.pinned.len() >= MAX_PENDING && !state.pinned.contains_key(&context_id) {
        return None;
    }
    state.pinned.insert(
        context_id.clone(),
        (identity.clone(), value, captured_layout),
    );
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

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn read_usize(address: usize) -> Option<usize> {
    Some(usize::from_le_bytes(
        rpm(address, mem::size_of::<usize>())?.try_into().ok()?,
    ))
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
        let context_id = "EventWindow::Description::7".to_string();
        state.captured_layouts.insert(
            context_id,
            AdmittedOwnerGlyphCapture {
                layout: RuntimeTextLayout::CapturedRuntime {
                    clip: Some([0, 0, 100, 30]),
                    glyphs: vec![crate::GlyphRun {
                        run_ordinal: 0,
                        font: "hoi_16b".into(),
                        glyph_id: 83,
                        x: 1,
                        y: 2,
                        rgba: [255; 4],
                    }],
                    inline_icons: Vec::new(),
                },
                gui_engine_instance_token: 7,
                paint_ordinal: 0,
                destination_rect: [10, 20, 110, 50],
                ui_clip: None,
                fonts: Vec::new(),
                icon_assets: Vec::new(),
            },
        );
        let context = pin_textbox_context(&mut state, owner).expect("pin context");
        state.identities.remove(&owner);
        state.latest.remove(&owner);
        let (identity, value, captured_layout) = state
            .pinned
            .get(&context.context_id)
            .expect("pinned row survives owner destruction");
        assert_eq!(identity.view_id, "EventWindow");
        assert_eq!(identity.element_id, "Description");
        assert_eq!(value.bytes, b"Scoped description");
        assert!(matches!(
            captured_layout.as_ref().map(|capture| &capture.layout),
            Some(RuntimeTextLayout::CapturedRuntime { .. })
        ));
    }
}
