use crate::owner_glyph_capture::{
    FontAssetReceipt, InlineIconAssetReceipt, GAME_BITMAP_FONT_VTABLE_RVA,
    GLYPH_POST_EMIT_HOOK_RVA, GLYPH_VERTEX_BYTES, GLYPH_VERTEX_SCRATCH_RVA, GLYPH_VERTEX_STRIDE,
};
use crate::owner_glyph_probe::{
    observe_post_emit_ordinary_glyph_if_active, record_production_inline_icon_absolute,
    record_production_inline_icon_asset, PostEmitOrdinaryGlyphObservation,
};
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const POST_EMIT_OVERWRITE_LEN: usize = 7;
pub const FALLBACK_CODEPOINT_RVA: usize = 0x030A_5E80;
pub const MAX_AUTHENTICATED_FONTS: usize = 4_096;
pub const MAX_KERNING_PAIRS: usize = 65_536;
pub const INLINE_ICON_DRAW_RVA: usize = 0x0162_EFE0;
pub const INLINE_ICON_LOOKUP_RVA: usize = 0x0162_E880;
pub const INLINE_ICON_ADVANCE_RVA: usize = 0x0162_EB10;
pub const MAX_INLINE_ICON_CONTENT: usize = 262_144;

pub const POST_EMIT_SENTINEL: [u8; 16] = [
    0x48, 0x8B, 0x4D, 0xD0, 0x48, 0x85, 0xC9, 0x4C, 0x8B, 0x54, 0x24, 0x78, 0x74, 0x10, 0x41, 0x8B,
];
pub const INLINE_ICON_DRAW_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x55, 0x56, 0x41, 0x57, 0x48, 0x83, 0xEC, 0x58, 0x80, 0xB9, 0x30, 0x36, 0x00, 0x00,
];

static POST_EMIT_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static INLINE_ICON_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static INSTALL_EPOCH: AtomicUsize = AtomicUsize::new(0);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static STATE: Mutex<PostEmitState> = Mutex::new(PostEmitState::new());
static AUTHENTICATED_FONT_ASSETS: Mutex<BTreeMap<String, FontAssetReceipt>> =
    Mutex::new(BTreeMap::new());
static AUTHENTICATED_INLINE_ICON_CONTENT: Mutex<BTreeMap<String, String>> =
    Mutex::new(BTreeMap::new());

struct PostEmitState {
    authenticated_fonts: BTreeMap<usize, String>,
    observed_count: u64,
    inactive_count: u64,
    rejected_count: u64,
    last_error: Option<String>,
}

pub fn bind_authenticated_font_assets(receipts: Vec<FontAssetReceipt>) -> Result<(), String> {
    if receipts.is_empty() {
        return Err("A25 authenticated font asset index is empty".into());
    }
    let mut index = BTreeMap::new();
    for receipt in receipts {
        if receipt.identity.trim().is_empty()
            || receipt.identity != receipt.identity.trim()
            || !valid_lower_sha256(&receipt.descriptor_sha256)
            || receipt.atlas_sha256.is_empty()
            || receipt
                .atlas_sha256
                .iter()
                .any(|digest| !valid_lower_sha256(digest))
        {
            return Err("A25 authenticated font asset receipt is incomplete".into());
        }
        if index.insert(receipt.identity.clone(), receipt).is_some() {
            return Err("A25 authenticated font asset index contains duplicate identities".into());
        }
    }
    let mut bound = AUTHENTICATED_FONT_ASSETS
        .lock()
        .map_err(|_| "A25 authenticated font asset index is poisoned".to_string())?;
    if !bound.is_empty() && *bound != index {
        return Err("A25 authenticated font asset index was rebound".into());
    }
    *bound = index;
    Ok(())
}

pub fn bind_authenticated_inline_icon_content(rows: Vec<(String, String)>) -> Result<(), String> {
    if rows.is_empty() || rows.len() > MAX_INLINE_ICON_CONTENT {
        return Err("A25 authenticated inline-icon content index is empty or unbounded".into());
    }
    let mut index = BTreeMap::new();
    for (path, payload_sha256) in rows {
        if !canonical_game_path(&path) || !valid_lower_sha256(&payload_sha256) {
            return Err("A25 authenticated inline-icon content row is incomplete".into());
        }
        if index.insert(path, payload_sha256).is_some() {
            return Err(
                "A25 authenticated inline-icon content index contains duplicate paths".into(),
            );
        }
    }
    let mut bound = AUTHENTICATED_INLINE_ICON_CONTENT
        .lock()
        .map_err(|_| "A25 authenticated inline-icon content index is poisoned".to_string())?;
    if !bound.is_empty() && *bound != index {
        return Err("A25 authenticated inline-icon content index was rebound".into());
    }
    *bound = index;
    Ok(())
}

fn valid_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_game_path(path: &str) -> bool {
    !path.is_empty()
        && path == path.trim()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains("//")
        && !path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
}

impl PostEmitState {
    const fn new() -> Self {
        Self {
            authenticated_fonts: BTreeMap::new(),
            observed_count: 0,
            inactive_count: 0,
            rejected_count: 0,
            last_error: None,
        }
    }

    fn reset(&mut self) {
        self.authenticated_fonts.clear();
        self.observed_count = 0;
        self.inactive_count = 0;
        self.rejected_count = 0;
        self.last_error = None;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostEmitHookDiagnostics {
    pub installed: bool,
    pub callbacks_open: bool,
    pub in_flight_callbacks: usize,
    pub authenticated_font_count: usize,
    pub observed_count: u64,
    pub inactive_count: u64,
    pub rejected_count: u64,
    pub last_error: Option<String>,
}

/// Sole owner of the exact interior detour.  Font pointers acquire meaning
pub struct A729PostEmitHookOwner {
    target: usize,
    inline_icon_target: usize,
    epoch: usize,
    active: bool,
}

impl A729PostEmitHookOwner {
    pub fn install() -> Result<Self, String> {
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A25 post-emit install mutex poisoned".to_string())?;
        if INSTALLED.load(Ordering::Acquire) {
            return Err("A25 post-emit hook already has a live owner".into());
        }
        let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
        if base == 0 {
            return Err("GetModuleHandleW(NULL) returned null".into());
        }
        let target = base
            .checked_add(GLYPH_POST_EMIT_HOOK_RVA as usize)
            .ok_or("A25 post-emit target overflow")?;
        verify_sentinel(target, &POST_EMIT_SENTINEL)?;
        let inline_icon_target = base
            .checked_add(INLINE_ICON_DRAW_RVA)
            .ok_or("A25 inline-icon target overflow")?;
        verify_sentinel(inline_icon_target, &INLINE_ICON_DRAW_SENTINEL)?;

        let mut state = state_lock()?;
        state.reset();
        drop(state);
        let epoch = INSTALL_EPOCH
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "A25 post-emit install epoch exhausted".to_string())?
            + 1;

        let trampoline = unsafe {
            MinHook::create_hook(target as *mut c_void, a25_post_emit_detour as *mut c_void)
        }
        .map_err(|status| format!("A25 post-emit hook creation failed: {status}"))?;
        if trampoline.is_null() {
            let _ = unsafe { inline_detour::remove(target as *mut c_void) };
            return Err("A25 post-emit hook returned a null trampoline".into());
        }
        let inline_icon_trampoline = match unsafe {
            MinHook::create_hook(
                inline_icon_target as *mut c_void,
                a25_inline_icon_detour as *mut c_void,
            )
        } {
            Ok(value) if !value.is_null() => value,
            Ok(_) => {
                let _ = unsafe { inline_detour::remove(inline_icon_target as *mut c_void) };
                let _ = unsafe { inline_detour::remove(target as *mut c_void) };
                return Err("A25 inline-icon hook returned a null trampoline".into());
            }
            Err(status) => {
                let _ = unsafe { inline_detour::remove(target as *mut c_void) };
                return Err(format!("A25 inline-icon hook creation failed: {status}"));
            }
        };

        POST_EMIT_TRAMPOLINE.store(trampoline as usize, Ordering::Release);
        INLINE_ICON_TRAMPOLINE.store(inline_icon_trampoline as usize, Ordering::Release);
        MODULE_BASE.store(base, Ordering::Release);
        CALLBACKS_OPEN.store(true, Ordering::Release);
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            CALLBACKS_OPEN.store(false, Ordering::Release);
            let _ = unsafe { inline_detour::remove(target as *mut c_void) };
            POST_EMIT_TRAMPOLINE.store(0, Ordering::Release);
            INLINE_ICON_TRAMPOLINE.store(0, Ordering::Release);
            let _ = unsafe { inline_detour::remove(inline_icon_target as *mut c_void) };
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("A25 post-emit hook enable failed: {status}"));
        }
        if let Err(status) = unsafe { inline_detour::enable(inline_icon_target as *mut c_void) } {
            CALLBACKS_OPEN.store(false, Ordering::Release);
            let _ = unsafe { inline_detour::disable(target as *mut c_void) };
            let _ = unsafe { inline_detour::remove(inline_icon_target as *mut c_void) };
            let _ = unsafe { inline_detour::remove(target as *mut c_void) };
            POST_EMIT_TRAMPOLINE.store(0, Ordering::Release);
            INLINE_ICON_TRAMPOLINE.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("A25 inline-icon hook enable failed: {status}"));
        }
        INSTALLED.store(true, Ordering::Release);
        Ok(Self {
            target,
            inline_icon_target,
            epoch,
            active: true,
        })
    }

    pub fn register_authenticated_font_identity(
        &self,
        font_address: usize,
        font_identity: String,
    ) -> Result<(), String> {
        self.require_active()?;
        let identity = font_identity.trim();
        if identity.is_empty() || identity.len() > 4_096 {
            return Err("A25 authenticated font identity is empty or unbounded".into());
        }
        let base = MODULE_BASE.load(Ordering::Acquire);
        let expected_vtable = base
            .checked_add(GAME_BITMAP_FONT_VTABLE_RVA as usize)
            .ok_or("A25 CGameBitmapFont vtable overflow")?;
        if read_usize(font_address)
            .filter(|vtable| *vtable == expected_vtable)
            .is_none()
        {
            return Err("A25 font pointer does not carry the exact CGameBitmapFont vtable".into());
        }
        let mut state = state_lock()?;
        if state.authenticated_fonts.len() >= MAX_AUTHENTICATED_FONTS
            && !state.authenticated_fonts.contains_key(&font_address)
        {
            return Err("A25 authenticated font registry capacity exceeded".into());
        }
        if let Some(existing) = state.authenticated_fonts.get(&font_address) {
            if existing != identity {
                return Err("A25 font pointer was rebound to a different identity".into());
            }
            return Ok(());
        }
        state
            .authenticated_fonts
            .insert(font_address, identity.to_string());
        Ok(())
    }

    pub fn unregister_authenticated_font_identity(
        &self,
        font_address: usize,
        expected_identity: &str,
    ) -> Result<(), String> {
        self.require_active()?;
        let mut state = state_lock()?;
        match state.authenticated_fonts.get(&font_address) {
            Some(identity) if identity == expected_identity => {
                state.authenticated_fonts.remove(&font_address);
                Ok(())
            }
            Some(_) => Err("A25 font unregister identity disagrees with registration".into()),
            None => Err("A25 font unregister address is not registered".into()),
        }
    }

    pub fn diagnostics(&self) -> Result<PostEmitHookDiagnostics, String> {
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A25 post-emit hook owner epoch is stale".into());
        }
        diagnostics()
    }

    fn require_active(&self) -> Result<(), String> {
        if !self.active || !INSTALLED.load(Ordering::Acquire) {
            return Err("A25 post-emit hook owner is not active".into());
        }
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A25 post-emit hook owner epoch is stale".into());
        }
        Ok(())
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A25 post-emit install mutex poisoned".to_string())?;
        CALLBACKS_OPEN.store(false, Ordering::Release);
        match unsafe { inline_detour::disable(self.inline_icon_target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                CALLBACKS_OPEN.store(true, Ordering::Release);
                return Err(format!("A25 inline-icon hook disable failed: {status}"));
            }
        }
        match unsafe { inline_detour::disable(self.target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                CALLBACKS_OPEN.store(true, Ordering::Release);
                return Err(format!("A25 post-emit hook disable failed: {status}"));
            }
        }
        wait_for_quiescence(Duration::from_secs(30))?;
        match unsafe { inline_detour::remove(self.target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                return Err(format!("A25 post-emit hook removal failed: {status}"));
            }
        }
        match unsafe { inline_detour::remove(self.inline_icon_target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                return Err(format!("A25 inline-icon hook removal failed: {status}"));
            }
        }
        POST_EMIT_TRAMPOLINE.store(0, Ordering::Release);
        INLINE_ICON_TRAMPOLINE.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        state_lock()?.authenticated_fonts.clear();
        self.active = false;
        Ok(())
    }
}

impl Drop for A729PostEmitHookOwner {
    fn drop(&mut self) {
        if self.active && self.uninstall().is_err() {
            // unsafe. Preserve the ownership latch and trampoline instead of
            INSTALLED.store(true, Ordering::Release);
        }
    }
}

pub fn diagnostics() -> Result<PostEmitHookDiagnostics, String> {
    let state = state_lock()?;
    Ok(PostEmitHookDiagnostics {
        installed: INSTALLED.load(Ordering::Acquire),
        callbacks_open: CALLBACKS_OPEN.load(Ordering::Acquire),
        in_flight_callbacks: IN_FLIGHT.load(Ordering::Acquire),
        authenticated_font_count: state.authenticated_fonts.len(),
        observed_count: state.observed_count,
        inactive_count: state.inactive_count,
        rejected_count: state.rejected_count,
        last_error: state.last_error.clone(),
    })
}

pub fn authenticated_font_identity(font_address: usize) -> Result<Option<String>, String> {
    if font_address == 0 {
        return Ok(None);
    }
    Ok(state_lock()?
        .authenticated_fonts
        .get(&font_address)
        .cloned())
}

type InlineIconDrawFn = unsafe extern "system" fn(usize, usize, *const u8, u32, u32, usize) -> u64;
type InlineIconLookupFn = unsafe extern "system" fn(usize, *const u8, *mut u32) -> usize;
type InlineIconAdvanceFn = unsafe extern "system" fn(usize, *const u8) -> u32;

unsafe extern "system" fn a25_inline_icon_detour(
    font: usize,
    draw_context: usize,
    token: *const u8,
    x: u32,
    y: u32,
    transform: usize,
) -> u64 {
    let _in_flight = InFlight::enter();
    let trampoline = INLINE_ICON_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return 0;
    }
    let original: InlineIconDrawFn = unsafe { mem::transmute(trampoline) };
    let result = unsafe { original(font, draw_context, token, x, y, transform) };
    if result == 0 || !CALLBACKS_OPEN.load(Ordering::Acquire) {
        return result;
    }
    let captured = catch_unwind(AssertUnwindSafe(|| capture_inline_icon(font, token, x, y)));
    match captured {
        Ok(Ok(true)) => record_result(true, false, None),
        Ok(Ok(false)) => record_result(false, true, None),
        Ok(Err(error)) => record_result(false, false, Some(error)),
        Err(_) => record_result(
            false,
            false,
            Some("A25 inline-icon callback panicked".into()),
        ),
    }
    result
}

fn capture_inline_icon(
    font: usize,
    token_pointer: *const u8,
    x: u32,
    y: u32,
) -> Result<bool, String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("A25 inline-icon module base is absent".into());
    }
    let expected_vtable = base
        .checked_add(GAME_BITMAP_FONT_VTABLE_RVA as usize)
        .ok_or("A25 inline-icon font vtable overflow")?;
    if read_usize(font) != Some(expected_vtable) {
        return Err("A25 inline-icon font lacks the exact CGameBitmapFont vtable".into());
    }
    let token = read_stable_c_string(token_pointer as usize, 4_096)
        .ok_or("A25 inline-icon token is unreadable or unstable")?;
    let sprite_identity = token
        .split_once('|')
        .map_or(token.as_str(), |(name, _)| name)
        .to_string();
    if sprite_identity.is_empty() {
        return Err("A25 inline-icon sprite identity is empty".into());
    }

    let lookup_address = base
        .checked_add(INLINE_ICON_LOOKUP_RVA)
        .ok_or("A25 inline-icon lookup address overflow")?;
    let lookup: InlineIconLookupFn = unsafe { mem::transmute(lookup_address) };
    let mut frame = 0u32;
    let sprite = unsafe { lookup(font, token_pointer, &mut frame) };
    if sprite == 0 {
        return Err("A25 drawn inline icon no longer resolves to a sprite".into());
    }
    let advance_address = base
        .checked_add(INLINE_ICON_ADVANCE_RVA)
        .ok_or("A25 inline-icon advance address overflow")?;
    let advance_fn: InlineIconAdvanceFn = unsafe { mem::transmute(advance_address) };
    let advance = unsafe { advance_fn(font, token_pointer) };
    let advance = i32::try_from(advance)
        .ok()
        .filter(|value| *value > 0)
        .ok_or("A25 inline-icon advance is zero or exceeds i32")?;

    let definition_path = runtime_content_path(
        sprite
            .checked_add(0x48)
            .ok_or("A25 inline-icon definition-path address overflow")?,
        "sprite definition",
    )?;
    let texture_index = read_i32(
        sprite
            .checked_add(0x1A0)
            .ok_or("A25 inline-icon texture-index address overflow")?,
    )
    .filter(|index| *index >= 0)
    .ok_or("A25 inline-icon texture index is unreadable or negative")?
        as usize;
    if texture_index > MAX_INLINE_ICON_CONTENT {
        return Err("A25 inline-icon texture index is unbounded".into());
    }
    let graphics = read_usize(
        font.checked_add(0x48)
            .ok_or("A25 font graphics-context address overflow")?,
    )
    .filter(|pointer| *pointer != 0)
    .ok_or("A25 font graphics context is null")?;
    let resource_manager = read_usize(
        graphics
            .checked_add(0x170)
            .ok_or("A25 texture resource-manager address overflow")?,
    )
    .filter(|pointer| *pointer != 0)
    .ok_or("A25 texture resource manager is null")?;
    let resource_entries = read_usize(
        resource_manager
            .checked_add(8)
            .ok_or("A25 texture resource-entry vector address overflow")?,
    )
    .filter(|pointer| *pointer != 0)
    .ok_or("A25 texture resource-entry vector is null")?;
    let resource_entry = resource_entries
        .checked_add(
            texture_index
                .checked_mul(0x48)
                .ok_or("A25 texture resource-entry offset overflow")?,
        )
        .ok_or("A25 texture resource-entry address overflow")?;
    let texture_path = runtime_content_path(
        resource_entry
            .checked_add(8)
            .ok_or("A25 texture resource-path address overflow")?,
        "texture resource",
    )?;

    let content = AUTHENTICATED_INLINE_ICON_CONTENT
        .lock()
        .map_err(|_| "A25 authenticated inline-icon content index is poisoned".to_string())?;
    let definition_sha256 = content
        .get(&definition_path)
        .cloned()
        .ok_or("A25 inline-icon definition is absent from the authenticated VFS")?;
    let texture_sha256 = content
        .get(&texture_path)
        .cloned()
        .ok_or("A25 inline-icon texture is absent from the authenticated VFS")?;
    drop(content);

    let asset = InlineIconAssetReceipt {
        identity: sprite_identity.clone(),
        definition_sha256: definition_sha256.clone(),
        texture_sha256: vec![texture_sha256.clone()],
    };
    let mut digest = Sha256::new();
    digest.update(b"hoi4-a729-inline-icon-draw-v1\0");
    hash_field(&mut digest, token.as_bytes())?;
    hash_field(&mut digest, sprite_identity.as_bytes())?;
    digest.update(frame.to_le_bytes());
    digest.update(x.to_le_bytes());
    digest.update(y.to_le_bytes());
    digest.update(advance.to_le_bytes());
    digest.update((texture_index as u64).to_le_bytes());
    hash_field(&mut digest, definition_path.as_bytes())?;
    digest.update(definition_sha256.as_bytes());
    hash_field(&mut digest, texture_path.as_bytes())?;
    digest.update(texture_sha256.as_bytes());
    let draw_receipt_sha256 = format!("{:x}", digest.finalize());

    if !record_production_inline_icon_asset(asset)? {
        return Ok(false);
    }
    record_production_inline_icon_absolute(
        token,
        sprite_identity,
        frame,
        i32::try_from(x).map_err(|_| "A25 inline-icon x exceeds i32")?,
        i32::try_from(y).map_err(|_| "A25 inline-icon y exceeds i32")?,
        advance,
        draw_receipt_sha256,
    )
}

fn runtime_content_path(address: usize, label: &str) -> Result<String, String> {
    let path = read_msvc_string(address, 4_096)
        .ok_or_else(|| format!("A25 inline-icon {label} path is not a stable MSVC string"))?
        .replace('\\', "/");
    canonical_game_path(&path)
        .then_some(path)
        .ok_or_else(|| format!("A25 inline-icon {label} path is not canonical"))
}

fn read_stable_c_string(address: usize, max_len: usize) -> Option<String> {
    fn read_once(address: usize, max_len: usize) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        for offset in 0..=max_len {
            let byte = *read_exact(address.checked_add(offset)?, 1)?.first()?;
            if byte == 0 {
                return (!bytes.is_empty()).then_some(bytes);
            }
            if offset == max_len {
                return None;
            }
            bytes.push(byte);
        }
        None
    }
    let first = read_once(address, max_len)?;
    let second = read_once(address, max_len)?;
    (first == second)
        .then(|| String::from_utf8(first).ok())
        .flatten()
}

fn hash_field(digest: &mut Sha256, bytes: &[u8]) -> Result<(), String> {
    let length = u32::try_from(bytes.len())
        .map_err(|_| "A25 inline-icon receipt field exceeds u32".to_string())?;
    digest.update(length.to_le_bytes());
    digest.update(bytes);
    Ok(())
}

// callable Rust or Windows ABI. At entry R13 is the descriptor, R15 the font,
#[unsafe(naked)]
unsafe extern "C" fn a25_post_emit_detour() -> ! {
    core::arch::naked_asm!(
        "pushfq",
        "push rax",
        "push rcx",
        "push rdx",
        "push rbx",
        "push rbp",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "sub rsp, 0xb0",
        "movdqu xmmword ptr [rsp + 0x40], xmm0",
        "movdqu xmmword ptr [rsp + 0x50], xmm1",
        "movdqu xmmword ptr [rsp + 0x60], xmm2",
        "movdqu xmmword ptr [rsp + 0x70], xmm3",
        "movdqu xmmword ptr [rsp + 0x80], xmm4",
        "movdqu xmmword ptr [rsp + 0x90], xmm5",
        "stmxcsr dword ptr [rsp + 0xa0]",
        "lea rax, [rsp + 0xb0]",
        "lea rcx, [rax + 0x80]",
        "mov rdx, qword ptr [rax + 0x50]",
        "mov r8, qword ptr [rax + 0x58]",
        "mov r9, qword ptr [rax + 0x40]",
        "mov r10, qword ptr [rax + 0x10]",
        "mov qword ptr [rsp + 0x20], r10",
        "mov r10, qword ptr [rax]",
        "mov qword ptr [rsp + 0x28], r10",
        "call {capture}",
        "ldmxcsr dword ptr [rsp + 0xa0]",
        "movdqu xmm0, xmmword ptr [rsp + 0x40]",
        "movdqu xmm1, xmmword ptr [rsp + 0x50]",
        "movdqu xmm2, xmmword ptr [rsp + 0x60]",
        "movdqu xmm3, xmmword ptr [rsp + 0x70]",
        "movdqu xmm4, xmmword ptr [rsp + 0x80]",
        "movdqu xmm5, xmmword ptr [rsp + 0x90]",
        "add rsp, 0xb0",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdi",
        "pop rsi",
        "pop rbp",
        "pop rbx",
        "pop rdx",
        "pop rcx",
        "pop rax",
        "popfq",
        "jmp qword ptr [rip + {trampoline}]",
        capture = sym a25_post_emit_capture_bridge,
        trampoline = sym POST_EMIT_TRAMPOLINE,
    );
}

unsafe extern "C" fn a25_post_emit_capture_bridge(
    original_rsp: usize,
    rbp: usize,
    rbx: usize,
    rdi: usize,
    descriptor: usize,
    font: usize,
) {
    let _in_flight = InFlight::enter();
    if !CALLBACKS_OPEN.load(Ordering::Acquire) {
        return;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        capture_post_emit(original_rsp, rbp, rbx, rdi, descriptor, font)
    }));
    match result {
        Ok(Ok(true)) => record_result(true, false, None),
        Ok(Ok(false)) => record_result(false, true, None),
        Ok(Err(error)) => record_result(false, false, Some(error)),
        Err(_) => record_result(false, false, Some("A25 post-emit callback panicked".into())),
    }
}

fn capture_post_emit(
    original_rsp: usize,
    rbp: usize,
    rbx: usize,
    utf32_vector: usize,
    descriptor: usize,
    font: usize,
) -> Result<bool, String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("A25 post-emit module base is absent".into());
    }
    let expected_vtable = base
        .checked_add(GAME_BITMAP_FONT_VTABLE_RVA as usize)
        .ok_or("A25 CGameBitmapFont vtable overflow")?;
    if read_usize(font) != Some(expected_vtable) {
        return Err("A25 post-emit font lacks the exact CGameBitmapFont vtable".into());
    }
    let font_asset = {
        let mut state = state_lock()?;
        if let Some(identity) = state.authenticated_fonts.get(&font) {
            AUTHENTICATED_FONT_ASSETS
                .lock()
                .map_err(|_| "A25 authenticated font asset index is poisoned".to_string())?
                .get(identity)
                .cloned()
                .ok_or("A25 registered font identity has no authenticated asset receipt")?
        } else {
            let receipt = resolve_runtime_font_asset(font)?;
            state
                .authenticated_fonts
                .insert(font, receipt.identity.clone());
            receipt
        }
    };
    crate::owner_glyph_probe::record_production_font_asset(font_asset.clone())?;
    let font_identity = font_asset.identity;

    let saved_ordinal = read_u32(
        original_rsp
            .checked_add(0x6C)
            .ok_or("A25 source-ordinal address overflow")?,
    )
    .ok_or("A25 saved source ordinal is unreadable")?;
    let register_ordinal =
        u32::try_from(rbx).map_err(|_| "A25 source-ordinal register exceeds u32".to_string())?;
    if saved_ordinal != register_ordinal {
        return Err("A25 saved and register source ordinals disagree".into());
    }
    let requested_codepoint = read_utf32_codepoint(utf32_vector, register_ordinal)?;
    if char::from_u32(requested_codepoint).is_none() {
        return Err("A25 requested codepoint is not a Unicode scalar".into());
    }
    let resolved_codepoint =
        resolve_descriptor_codepoint(base, font, requested_codepoint, descriptor)?;

    let post_count = read_usize(
        rbp.checked_sub(0x60)
            .ok_or("A25 post-count address underflow")?,
    )
    .ok_or("A25 post-write vertex count is unreadable")?;
    let emitted_vertex_end_count = u32::try_from(post_count)
        .map_err(|_| "A25 post-write vertex count exceeds u32".to_string())?;
    let scratch_group_start_rva = scratch_group_start_rva(emitted_vertex_end_count)?;
    let scratch_address = base
        .checked_add(scratch_group_start_rva as usize)
        .ok_or("A25 scratch group address overflow")?;
    let generated_vertices = read_exact(scratch_address, GLYPH_VERTEX_BYTES)
        .ok_or("A25 six-vertex scratch group is unreadable")?;
    let descriptor_sha256 = canonical_descriptor_sha256(descriptor)?;

    observe_post_emit_ordinary_glyph_if_active(PostEmitOrdinaryGlyphObservation {
        font_identity,
        requested_codepoint,
        resolved_codepoint,
        descriptor_sha256,
        source_ordinal: register_ordinal,
        emitted_vertex_end_count,
        scratch_group_start_rva,
        generated_vertices,
    })
}

fn read_utf32_codepoint(vector: usize, ordinal: u32) -> Result<u32, String> {
    let length = read_u32(
        vector
            .checked_add(0x10)
            .ok_or("A25 UTF-32 vector length overflow")?,
    )
    .ok_or("A25 UTF-32 vector length is unreadable")?;
    if ordinal >= length {
        return Err("A25 source ordinal exceeds the UTF-32 vector".into());
    }
    let capacity = read_usize(
        vector
            .checked_add(0x18)
            .ok_or("A25 UTF-32 vector capacity overflow")?,
    )
    .ok_or("A25 UTF-32 vector capacity is unreadable")?;
    if capacity < length as usize {
        return Err("A25 UTF-32 vector capacity is smaller than its length".into());
    }
    let data = if capacity <= 3 {
        vector
    } else {
        read_usize(vector).ok_or("A25 UTF-32 heap vector pointer is unreadable")?
    };
    let offset = (ordinal as usize)
        .checked_mul(4)
        .ok_or("A25 UTF-32 ordinal offset overflow")?;
    read_u32(
        data.checked_add(offset)
            .ok_or("A25 UTF-32 codepoint address overflow")?,
    )
    .ok_or_else(|| "A25 UTF-32 codepoint is unreadable".into())
}

fn resolve_descriptor_codepoint(
    base: usize,
    font: usize,
    requested: u32,
    descriptor: usize,
) -> Result<u32, String> {
    if descriptor == 0 {
        return Err("A25 post-emit descriptor is null".into());
    }
    if lookup_exact_descriptor(font, requested)? == Some(descriptor) {
        return Ok(requested);
    }
    let fallback = read_u32(
        base.checked_add(FALLBACK_CODEPOINT_RVA)
            .ok_or("A25 fallback-codepoint address overflow")?,
    )
    .ok_or("A25 fallback codepoint is unreadable")?;
    if !native_fallback_is_eligible(requested, fallback) {
        return Err("A25 fallback codepoint is absent or invalid".into());
    }
    if lookup_exact_descriptor(font, fallback)? != Some(descriptor) {
        return Err("A25 descriptor matches neither requested nor fallback codepoint".into());
    }
    Ok(fallback)
}

fn native_fallback_is_eligible(requested: u32, fallback: u32) -> bool {
    requested > 0x1F && fallback > 0x1F && char::from_u32(fallback).is_some()
}

fn lookup_exact_descriptor(font: usize, codepoint: u32) -> Result<Option<usize>, String> {
    let family_table = read_usize(
        font.checked_add(0xE8)
            .ok_or("A25 font family-table address overflow")?,
    )
    .ok_or("A25 font family table is unreadable")?;
    let family_index = read_i32(
        font.checked_add(0x100)
            .ok_or("A25 font family-index address overflow")?,
    )
    .ok_or("A25 font family index is unreadable")?;
    if family_index < 0 {
        return Err("A25 font family index is negative".into());
    }
    let family = family_table
        .checked_add(
            (family_index as usize)
                .checked_mul(0x50)
                .ok_or("A25 font family offset overflow")?,
        )
        .ok_or("A25 font family address overflow")?;
    let bucket = family
        .checked_add(
            ((codepoint & 0x7F) as usize)
                .checked_mul(0x18)
                .ok_or("A25 glyph bucket offset overflow")?,
        )
        .ok_or("A25 glyph bucket address overflow")?;
    let entries = read_usize(bucket).ok_or("A25 glyph bucket entries are unreadable")?;
    let count = read_i32(
        bucket
            .checked_add(0x0C)
            .ok_or("A25 glyph bucket count overflow")?,
    )
    .ok_or("A25 glyph bucket count is unreadable")?;
    if count < 0 || count as usize > MAX_KERNING_PAIRS {
        return Err("A25 glyph bucket count is invalid or unbounded".into());
    }
    if count == 0 {
        return Ok(None);
    }
    if entries == 0 {
        return Err("A25 non-empty glyph bucket has a null entry pointer".into());
    }

    let mut low = 0usize;
    let mut high = count as usize;
    while low < high {
        let middle = low + (high - low) / 2;
        let entry = entries
            .checked_add(
                middle
                    .checked_mul(0x10)
                    .ok_or("A25 glyph entry offset overflow")?,
            )
            .ok_or("A25 glyph entry address overflow")?;
        let key = read_u32(entry).ok_or("A25 glyph entry key is unreadable")?;
        if key < codepoint {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low == count as usize {
        return Ok(None);
    }
    let entry = entries
        .checked_add(
            low.checked_mul(0x10)
                .ok_or("A25 glyph result offset overflow")?,
        )
        .ok_or("A25 glyph result address overflow")?;
    if read_u32(entry) != Some(codepoint) {
        return Ok(None);
    }
    read_usize(
        entry
            .checked_add(8)
            .ok_or("A25 glyph descriptor field overflow")?,
    )
    .map(Some)
    .ok_or_else(|| "A25 glyph descriptor field is unreadable".into())
}

fn canonical_descriptor_sha256(descriptor: usize) -> Result<String, String> {
    let metrics =
        read_exact(descriptor, 0x20).ok_or("A25 glyph descriptor metrics are unreadable")?;
    let height = i32::from_le_bytes(metrics[0x10..0x14].try_into().unwrap());
    if height < 1 {
        return Err("A25 post-emit descriptor has non-positive height".into());
    }
    let kerning_pointer = read_usize(
        descriptor
            .checked_add(0x20)
            .ok_or("A25 kerning pointer address overflow")?,
    )
    .ok_or("A25 kerning pointer is unreadable")?;
    let kerning_count = read_i32(
        descriptor
            .checked_add(0x2C)
            .ok_or("A25 kerning count address overflow")?,
    )
    .ok_or("A25 kerning count is unreadable")?;
    if kerning_count < 0 || kerning_count as usize > MAX_KERNING_PAIRS {
        return Err("A25 kerning count is invalid or unbounded".into());
    }
    let pair_bytes = (kerning_count as usize)
        .checked_mul(8)
        .ok_or("A25 kerning byte count overflow")?;
    let kerning = if pair_bytes == 0 {
        Vec::new()
    } else {
        read_exact(kerning_pointer, pair_bytes).ok_or("A25 kerning pairs are unreadable")?
    };
    let mut digest = Sha256::new();
    digest.update(b"hoi4-a729-bmfont-glyph-descriptor-v1\0");
    digest.update(&metrics);
    digest.update(kerning_count.to_le_bytes());
    digest.update(&kerning);
    Ok(format!("{:x}", digest.finalize()))
}

fn resolve_runtime_font_asset(font: usize) -> Result<FontAssetReceipt, String> {
    let active_page = read_i32(
        font.checked_add(0x100)
            .ok_or("A25 runtime font active-page address overflow")?,
    )
    .filter(|page| *page >= 0)
    .ok_or("A25 runtime font active page is unreadable or negative")?
        as usize;
    let entries = read_usize(
        font.checked_add(0xe8)
            .ok_or("A25 runtime font entry-vector address overflow")?,
    )
    .filter(|pointer| *pointer != 0)
    .ok_or("A25 runtime font entry vector is null")?;
    let entry_count = read_u32(
        font.checked_add(0xf4)
            .ok_or("A25 runtime font entry-count address overflow")?,
    )
    .ok_or("A25 runtime font entry count is unreadable")? as usize;
    if active_page >= entry_count || entry_count > 4_096 {
        return Err("A25 runtime font active page exceeds the entry vector".into());
    }
    let entry = entries
        .checked_add(
            active_page
                .checked_mul(0x50)
                .ok_or("A25 runtime font entry offset overflow")?,
        )
        .ok_or("A25 runtime font entry address overflow")?;
    let source_count = read_u32(
        entry
            .checked_add(0x44)
            .ok_or("A25 runtime font source-count address overflow")?,
    )
    .ok_or("A25 runtime font source count is unreadable")?;
    if source_count != 1 {
        return Err("A25 runtime font page does not have one exact physical source".into());
    }
    let source_vector = read_usize(
        entry
            .checked_add(0x38)
            .ok_or("A25 runtime font source-vector address overflow")?,
    )
    .filter(|pointer| *pointer != 0)
    .ok_or("A25 runtime font source vector is null")?;
    let raw_base = read_msvc_string(source_vector, 4_096)
        .ok_or("A25 runtime font physical source is not one stable MSVC string")?;
    let normalized_base = raw_base.replace('\\', "/");
    if normalized_base.is_empty()
        || normalized_base.starts_with('/')
        || normalized_base.contains("//")
        || normalized_base
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(
            "A25 runtime font physical source is not a canonical game-relative path".into(),
        );
    }
    let definition_path = if normalized_base.ends_with(".fnt") {
        normalized_base
    } else {
        format!("{normalized_base}.fnt")
    };
    AUTHENTICATED_FONT_ASSETS
        .lock()
        .map_err(|_| "A25 authenticated font asset index is poisoned".to_string())?
        .get(&definition_path)
        .cloned()
        .ok_or_else(|| {
            format!(
                "A25 runtime font source {definition_path} is absent from the authenticated VFS"
            )
        })
}

fn read_msvc_string(address: usize, max_len: usize) -> Option<String> {
    if address == 0 || max_len == 0 {
        return None;
    }
    let first = read_exact(address, 32)?;
    let second = read_exact(address, 32)?;
    if first != second {
        return None;
    }
    let length = usize::from_le_bytes(first[0x10..0x18].try_into().ok()?);
    let capacity = usize::from_le_bytes(first[0x18..0x20].try_into().ok()?);
    if length == 0 || length > max_len || capacity < length {
        return None;
    }
    let bytes = if capacity < 16 {
        if length >= 16 {
            return None;
        }
        first[..length].to_vec()
    } else {
        let data_ptr = usize::from_le_bytes(first[..8].try_into().ok()?);
        let before = read_exact(data_ptr, length)?;
        let after = read_exact(data_ptr, length)?;
        if before != after {
            return None;
        }
        before
    };
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn scratch_group_start_rva(post_count: u32) -> Result<u32, String> {
    if post_count < 6 || post_count > 3_072 || post_count % 6 != 0 {
        return Err("A25 post-write vertex count is outside an exact scratch partition".into());
    }
    GLYPH_VERTEX_SCRATCH_RVA
        .checked_add(
            post_count
                .checked_sub(6)
                .and_then(|count| count.checked_mul(GLYPH_VERTEX_STRIDE as u32))
                .ok_or("A25 scratch group offset overflow")?,
        )
        .ok_or_else(|| "A25 scratch group RVA overflow".into())
}

fn record_result(observed: bool, inactive: bool, error: Option<String>) {
    if let Ok(mut state) = STATE.lock() {
        if observed {
            state.observed_count = state.observed_count.saturating_add(1);
        } else if inactive {
            state.inactive_count = state.inactive_count.saturating_add(1);
        } else {
            state.rejected_count = state.rejected_count.saturating_add(1);
            state.last_error = error;
        }
    }
}

fn wait_for_quiescence(timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let in_flight = IN_FLIGHT.load(Ordering::Acquire);
        if in_flight == 0 {
            return Ok(());
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "A25 post-emit callbacks did not quiesce ({in_flight} still active); containing DLL must remain resident"
            ));
        }
        std::thread::yield_now();
    }
}

struct InFlight;

impl InFlight {
    fn enter() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

fn state_lock() -> Result<MutexGuard<'static, PostEmitState>, String> {
    STATE
        .lock()
        .map_err(|_| "A25 post-emit state mutex poisoned".to_string())
}

fn verify_sentinel(target: usize, expected: &[u8]) -> Result<(), String> {
    let actual =
        read_exact(target, expected.len()).ok_or("A25 post-emit sentinel bytes are unreadable")?;
    if actual != expected {
        return Err(format!(
            "A25 post-emit exact-a729 sentinel mismatch: expected {}, got {}",
            hex(expected),
            hex(&actual)
        ));
    }
    Ok(())
}

fn read_usize(address: usize) -> Option<usize> {
    Some(usize::from_le_bytes(
        read_exact(address, mem::size_of::<usize>())?
            .try_into()
            .ok()?,
    ))
}

fn read_u32(address: usize) -> Option<u32> {
    Some(u32::from_le_bytes(read_exact(address, 4)?.try_into().ok()?))
}

fn read_i32(address: usize) -> Option<i32> {
    Some(i32::from_le_bytes(read_exact(address, 4)?.try_into().ok()?))
}

fn read_exact(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 || len == 0 {
        return (len == 0).then(Vec::new);
    }
    let mut output = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            output.as_mut_ptr() as *mut c_void,
            len,
            &mut read,
        )
    };
    (ok != 0 && read == len).then_some(output)
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_covers_exact_two_instruction_overwrite_and_successor() {
        assert_eq!(POST_EMIT_OVERWRITE_LEN, 7);
        assert_eq!(
            &POST_EMIT_SENTINEL[..7],
            &[0x48, 0x8B, 0x4D, 0xD0, 0x48, 0x85, 0xC9]
        );
        assert_eq!(&POST_EMIT_SENTINEL[7..12], &[0x4C, 0x8B, 0x54, 0x24, 0x78]);
    }

    #[test]
    fn scratch_partition_math_is_exact_and_bounded() {
        assert_eq!(
            scratch_group_start_rva(6).unwrap(),
            GLYPH_VERTEX_SCRATCH_RVA
        );
        assert_eq!(
            scratch_group_start_rva(12).unwrap(),
            GLYPH_VERTEX_SCRATCH_RVA + 6 * GLYPH_VERTEX_STRIDE as u32
        );
        assert!(scratch_group_start_rva(0).is_err());
        assert!(scratch_group_start_rva(7).is_err());
        assert!(scratch_group_start_rva(3_078).is_err());
        assert_eq!(GLYPH_VERTEX_BYTES, 6 * GLYPH_VERTEX_STRIDE);
    }

    #[test]
    fn diagnostic_state_is_fail_closed_not_measured_empty() {
        let mut state = PostEmitState::new();
        state.rejected_count = 1;
        state.last_error = Some("unresolved font".into());
        assert_eq!(state.observed_count, 0);
        assert_eq!(state.inactive_count, 0);
        assert_eq!(state.last_error.as_deref(), Some("unresolved font"));
    }

    #[test]
    fn native_fallback_gate_rejects_control_requests_after_exact_miss() {
        assert!(!native_fallback_is_eligible(0x1F, 0xFFFD));
        assert!(!native_fallback_is_eligible(0x20, 0x1F));
        assert!(!native_fallback_is_eligible(0x20, 0xD800));
        assert!(native_fallback_is_eligible(0x20, 0xFFFD));
    }
}
