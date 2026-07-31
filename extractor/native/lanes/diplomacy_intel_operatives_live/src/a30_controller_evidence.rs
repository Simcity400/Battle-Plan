//! Exact-a729 detours for the production A30 controller carriers.
//!
//! The production scheduler installs these hooks only when compact domain 53
//! is selected. The explicit evidence owner remains available behind its
//! environment gate. Both modes arm a single expected controller refresh.
//! Successful observations are
//! encoded immediately as the pointer-free `AUGC`/`CIPC` payloads consumed by
//! the domain-53 multiplex. Teardown disables outer producers first and does
//! not remove a trampoline until all in-flight callbacks have quiesced.

#![cfg(windows)]

use crate::agency_upgrade_controller::{
    AgencyUpgradeRefreshBuilder, A729_AGENCY_UPGRADE_BRANCH_MATERIALIZER_RVA,
    A729_AGENCY_UPGRADE_FILTER_RVA, A729_AGENCY_UPGRADE_REFRESH_RVA,
};
use crate::cipher_controller::{
    CipherControllerRefreshBuilder, A729_CIPHER_CONTROLLER_RVA, A729_CIPHER_ROW_BIND_RVA,
};
use crate::extractor::{Domain53Memory, ExtractError};
use core::{cell::RefCell, ffi::c_void, mem, ptr};
use minhook::{MinHook, MH_STATUS};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsString;
use std::io::Read;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetProcessTimes,
};

pub const EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_ENABLE_A30_CONTROLLER_EVIDENCE";
pub const EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const IMAGE_BASE: usize = 0x140000000;
pub const AGENCY_JOB_ID: &str =
    "20260720T051216Z-w4-a30-upgrade-network-controller-anchor-32d1d369";
pub const AGENCY_MANIFEST_SHA256: &str =
    "9935ea8f18fdc0e4debf615739f9ba1d05f42632c331ccc2e1ed4562536cc44d";
pub const CIPHER_CONTROLLER_JOB_ID: &str =
    "20260720T054113Z-w4-a30-cipher-country-list-network-row-f-dae92058";
pub const CIPHER_CONTROLLER_MANIFEST_SHA256: &str =
    "45d897e4b4a02a2f6ffd8baf621df05864976205e396eca067441b96cc8b1ca7";
pub const CIPHER_ROW_JOB_ID: &str =
    "20260720T054212Z-w4-a30-cipher-sort-materializer-final-7dc5c663";
pub const CIPHER_ROW_MANIFEST_SHA256: &str =
    "223d094e91be0cedef4c13ef0e60d5b020161ee39e2d9b133fc0a5a8afa31e82";
pub const SENTINEL_DUMP_JOB_ID: &str =
    "20260720T064043Z-w4-a30-controller-hook-entry-sentinels-3016b1ac";
pub const SENTINEL_DUMP_IDENTITY_MANIFEST_SHA256: &str =
    "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe";
pub const SENTINEL_DUMP_ARTIFACT_SHA256: &str =
    "8bfe8566aaa95d1f2916189d6c2097c5ec4eadab148fb39342c766aee68b392a";
pub const AGENCY_OWNER_RESOLVER_JOB_ID: &str =
    "20260722T070232Z-final-a30-production-scheduler-root-b-a5d56784";
pub const AGENCY_OWNER_RESOLVER_MANIFEST_SHA256: &str =
    "17eee03f84da87c2c1d864de1ea55e197e931418d51c31598494b475998143f1";
pub const AGENCY_OWNER_RESOLVER_INSTRUCTIONS_SHA256: &str =
    "6abb1fbcef1b230c8fc281a770e12e7eacf7ca8a2b58ba3e75166c76c03a5ccc";
pub const MAX_PENDING_RECORDS: usize = 1_024;

const AGENCY_OUTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x54,
    0x41, 0x56,
];
const AGENCY_MATERIALIZER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24,
    0x18, 0x48,
];
const AGENCY_FILTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
    0x20, 0x48,
];
const CIPHER_OUTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
    0x56, 0x41,
];
const CIPHER_ROW_SENTINEL: [u8; 16] = [
    0x44, 0x88, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x53, 0x56, 0x57,
    0x41, 0x54,
];
const AGENCY_OWNER_RESOLVER_RVA: usize = 0x0BA56A0;
const AGENCY_OWNER_RESOLVER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x41, 0x35,
    0xa2, 0x02,
];

type ControllerOuterFn = unsafe extern "system" fn(usize);
type AgencyMaterializerFn = unsafe extern "system" fn(usize, usize, usize, usize, usize) -> usize;
type AgencyFilterFn = unsafe extern "system" fn(usize, usize, usize) -> usize;
type CipherRowFn = unsafe extern "system" fn(usize, usize, u8);
type AgencyOwnerResolverFn = unsafe extern "system" fn(usize) -> usize;

static ORIGINAL_AGENCY_OUTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_AGENCY_MATERIALIZER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_AGENCY_FILTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CIPHER_OUTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CIPHER_ROW: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_AGENCY_OWNER_RESOLVER: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static EVIDENCE_INSTALL: AtomicBool = AtomicBool::new(false);
static TEARING_DOWN: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_HOOK_EPOCH: AtomicU64 = AtomicU64::new(1);
static ACTIVE_HOOK_EPOCH: AtomicU64 = AtomicU64::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static IGNORED_UNARMED_REFRESHES: AtomicU64 = AtomicU64::new(0);
static DROPPED_REENTRANT: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNCONSUMED_ARM: AtomicU64 = AtomicU64::new(0);
static COMPLETENESS_POISONED: AtomicBool = AtomicBool::new(false);
static LAST_AGENCY_CONTROLLER: AtomicUsize = AtomicUsize::new(0);
static LAST_CIPHER_CONTROLLER: AtomicUsize = AtomicUsize::new(0);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static ARMED: OnceLock<Mutex<ArmedRefreshes>> = OnceLock::new();
static QUEUE: OnceLock<Mutex<VecDeque<Domain53ControllerEvidenceRecord>>> = OnceLock::new();
static CURRENT_EXECUTABLE_VERIFICATION: OnceLock<Result<(), String>> = OnceLock::new();

thread_local! {
    static OPEN_AGENCY: RefCell<Option<OpenAgency>> = const { RefCell::new(None) };
    static OPEN_CIPHER: RefCell<Option<OpenCipher>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgencyRefreshArm {
    expected_controller: usize,
    frame_hour: u64,
    owner_tag_db_id: u32,
    valid_country_tag_db_ids: BTreeSet<u32>,
    producer_record_identity: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CipherRefreshArm {
    expected_controller: usize,
    frame_hour: u64,
    owner_tag_db_id: u32,
    selected_target_tag_db_id: u32,
    valid_country_tag_db_ids: BTreeSet<u32>,
    producer_record_identity: [u8; 32],
}

impl AgencyRefreshArm {
    /// Bind an arm to the same complete country-table authority as domain 53.
    /// The game-state and controller pointers are used only to arm this process
    /// epoch and never enter the encoded carrier or receipt.
    pub fn from_domain53_authority(
        expected_controller: usize,
        game_state: usize,
        frame_hour: u64,
        owner_tag_db_id: u32,
        producer_record_identity: [u8; 32],
    ) -> Result<Self, String> {
        let valid_country_tag_db_ids = crate::extractor::resolve_complete_country_tag_db_ids(
            &InProcessMemory,
            game_state as u64,
        )
        .map_err(|error| format!("A30 domain53 country authority unavailable: {error:?}"))?;
        let arm = Self {
            expected_controller,
            frame_hour,
            owner_tag_db_id,
            valid_country_tag_db_ids,
            producer_record_identity,
        };
        validate_agency_arm(&arm)?;
        Ok(arm)
    }
}

impl CipherRefreshArm {
    pub fn from_domain53_authority(
        expected_controller: usize,
        game_state: usize,
        frame_hour: u64,
        owner_tag_db_id: u32,
        selected_target_tag_db_id: u32,
        producer_record_identity: [u8; 32],
    ) -> Result<Self, String> {
        let valid_country_tag_db_ids = crate::extractor::resolve_complete_country_tag_db_ids(
            &InProcessMemory,
            game_state as u64,
        )
        .map_err(|error| format!("A30 domain53 country authority unavailable: {error:?}"))?;
        let arm = Self {
            expected_controller,
            frame_hour,
            owner_tag_db_id,
            selected_target_tag_db_id,
            valid_country_tag_db_ids,
            producer_record_identity,
        };
        validate_cipher_arm(&arm)?;
        Ok(arm)
    }
}

#[derive(Default)]
struct ArmedRefreshes {
    agency: Option<AgencyRefreshArm>,
    cipher: Option<CipherRefreshArm>,
}

struct OpenAgency {
    builder: Option<AgencyUpgradeRefreshBuilder>,
    arm: AgencyRefreshArm,
    owner_resolved: bool,
    expected_controller: usize,
    next_branch_index: u32,
    pending_branch: Option<(u32, u64)>,
    poisoned: bool,
}

struct OpenCipher {
    builder: Option<CipherControllerRefreshBuilder>,
    expected_controller: usize,
    poisoned: bool,
}

struct CallbackGuard;
impl CallbackGuard {
    fn enter() -> Self {
        ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for CallbackGuard {
    fn drop(&mut self) {
        ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControllerEvidenceFamily {
    AgencyUpgrade,
    CipherCountry,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Domain53ControllerEvidenceRecord {
    pub sequence: u64,
    pub thread_id: u32,
    pub family: ControllerEvidenceFamily,
    pub frame_hour: u64,
    pub payload: Vec<u8>,
}

impl Domain53ControllerEvidenceRecord {
    /// Wrap the already pointer-free carrier as an ordinary domain-53 H4CR
    /// record. Sequence ownership remains with the collector/recorder caller.
    pub fn encode_h4cr(&self, h4cr_sequence: u64) -> Result<Vec<u8>, String> {
        if h4cr_sequence == 0 {
            return Err("A30 H4CR sequence zero is not an owned recorder sequence".into());
        }
        self.validate_carrier()?;
        let game_hour_raw = i32::try_from(self.frame_hour)
            .map_err(|_| "A30 controller frame hour exceeds H4CR i32 authority")?;
        Ok(crate::decoder_gate::encode_domain53_data_record(
            h4cr_sequence,
            game_hour_raw,
            &self.payload,
        ))
    }

    pub fn validate_carrier(&self) -> Result<(), String> {
        let encoded_hour = match self.family {
            ControllerEvidenceFamily::AgencyUpgrade => {
                crate::agency_upgrade_controller::AgencyUpgradeControllerRefresh::decode(&self.payload)
                    .map_err(|error| format!("A30 AUGC admission failed: {error:?}"))?
                    .frame_hour
            }
            ControllerEvidenceFamily::CipherCountry => {
                crate::cipher_controller::CipherControllerRefresh::decode(&self.payload)
                    .map_err(|error| format!("A30 CIPC admission failed: {error:?}"))?
                    .frame_hour
            }
        };
        if encoded_hour != self.frame_hour {
            return Err("A30 carrier frame hour disagrees with drain envelope".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerEvidenceDiagnostics {
    pub retained_records: u64,
    /// Refreshes for controller objects that the owner did not arm. They are
    /// expected while evidence mode is installed and do not poison capture.
    pub ignored_unarmed_refreshes: u64,
    pub dropped_reentrant: u64,
    pub dropped_unreadable: u64,
    pub dropped_lock_busy: u64,
    pub dropped_overflow: u64,
    pub dropped_unconsumed_arm: u64,
    pub completeness_poisoned: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDrain {
    pub hook_epoch: u64,
    pub module_base: u64,
    pub evidence_install: bool,
    pub records: Vec<Domain53ControllerEvidenceRecord>,
    pub diagnostics: ControllerEvidenceDiagnostics,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookSiteReceipt {
    pub rva: u32,
    pub installed_address: u64,
    pub sentinel_sha256: String,
    pub artifact_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedRecordReceipt {
    pub sequence: u64,
    pub thread_id: u32,
    pub family: ControllerEvidenceFamily,
    pub frame_hour: u64,
    pub payload_bytes: u64,
    pub payload_sha256: String,
    pub owner_tag_db_id: Option<u32>,
    pub selected_target_tag_db_id: Option<u32>,
    pub producer_record_identity: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentityReceipt {
    pub pid: u32,
    pub create_time_100ns: u64,
    pub executable_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerEvidenceReceipt {
    pub schema: String,
    pub executable_sha256: String,
    pub image_base: u64,
    pub agency_job_id: String,
    pub agency_manifest_sha256: String,
    pub cipher_controller_job_id: String,
    pub cipher_controller_manifest_sha256: String,
    pub cipher_row_job_id: String,
    pub cipher_row_manifest_sha256: String,
    pub sentinel_dump_job_id: String,
    pub sentinel_dump_identity_manifest_sha256: String,
    pub sentinel_dump_artifact_sha256: String,
    pub agency_owner_resolver_job_id: String,
    pub agency_owner_resolver_manifest_sha256: String,
    pub process_identity: ProcessIdentityReceipt,
    pub evidence_install: bool,
    pub hook_epoch: u64,
    pub hook_sites: Vec<HookSiteReceipt>,
    pub records: Vec<EncodedRecordReceipt>,
    pub diagnostics: ControllerEvidenceDiagnostics,
    pub domain53_multiplex_ready: bool,
    pub production_scheduling_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub agency_outer: usize,
    pub agency_materializer: usize,
    pub agency_filter: usize,
    pub cipher_outer: usize,
    pub cipher_row: usize,
    pub agency_owner_resolver: usize,
}

pub fn evidence_activation_enabled() -> bool {
    std::env::var(EVIDENCE_ACTIVATION_ENV).ok().as_deref() == Some("1")
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Process-local discovery for the opt-in autonomous owner.  Addresses are
/// consumed only to arm the next exact controller callback and are never
/// serialized.
pub fn discovered_controllers() -> Option<(usize, usize)> {
    let agency = LAST_AGENCY_CONTROLLER.load(Ordering::Acquire);
    let cipher = LAST_CIPHER_CONTROLLER.load(Ordering::Acquire);
    (agency != 0 && cipher != 0).then_some((agency, cipher))
}

pub fn capture_complete() -> bool {
    let Ok(queue) = queue().try_lock() else { return false };
    queue.iter().filter(|row| row.family == ControllerEvidenceFamily::AgencyUpgrade).count() == 1
        && queue.iter().filter(|row| row.family == ControllerEvidenceFamily::CipherCountry).count() == 1
}

pub fn arm_agency_refresh(arm: AgencyRefreshArm) -> Result<(), String> {
    validate_agency_arm(&arm)?;
    if !installed() || TEARING_DOWN.load(Ordering::Acquire) {
        return Err("A30 controller hooks are not installed or are tearing down".into());
    }
    let mut armed = armed().lock().map_err(|_| "A30 arm lock poisoned")?;
    if armed.agency.is_some() {
        return Err("A30 agency refresh is already armed".into());
    }
    armed.agency = Some(arm);
    Ok(())
}

pub fn arm_cipher_refresh(arm: CipherRefreshArm) -> Result<(), String> {
    validate_cipher_arm(&arm)?;
    if !installed() || TEARING_DOWN.load(Ordering::Acquire) {
        return Err("A30 controller hooks are not installed or are tearing down".into());
    }
    let mut armed = armed().lock().map_err(|_| "A30 arm lock poisoned")?;
    if armed.cipher.is_some() {
        return Err("A30 cipher refresh is already armed".into());
    }
    armed.cipher = Some(arm);
    Ok(())
}

pub fn install_fixture_hooks() -> Result<InstalledHookAddresses, String> {
    if !evidence_activation_enabled() {
        return Err(format!("A30 evidence hook requires {EVIDENCE_ACTIVATION_ENV}=1"));
    }
    install_hooks(true)
}

/// Install the exact byte-gated observer for the ordinary compact-domain 53
/// scheduler. No environment flag or fixture receipt is an activation input.
pub fn install_production_hooks() -> Result<InstalledHookAddresses, String> {
    install_hooks(false)
}

fn install_hooks(evidence_install: bool) -> Result<InstalledHookAddresses, String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A30 install lock poisoned")?;
    if installed() {
        if EVIDENCE_INSTALL.load(Ordering::Acquire) != evidence_install {
            return Err("A30 controller hooks already belong to the other install mode".into());
        }
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    verify_current_process_executable()?;
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A30 GetModuleHandleW(NULL) returned null".into());
    }
    let sites = addresses(base);
    for (address, sentinel) in site_sentinels(sites) {
        verify_sentinel(address, sentinel)?;
    }
    let epoch = NEXT_HOOK_EPOCH.fetch_add(1, Ordering::AcqRel);
    if epoch == 0 {
        return Err("A30 hook epoch exhausted".into());
    }
    reset_state(epoch)?;
    MODULE_BASE.store(base, Ordering::Release);
    let hooks = [
        (sites.agency_owner_resolver, hook_agency_owner_resolver as *mut c_void, &ORIGINAL_AGENCY_OWNER_RESOLVER),
        (sites.agency_outer, hook_agency_outer as *mut c_void, &ORIGINAL_AGENCY_OUTER),
        (sites.agency_materializer, hook_agency_materializer as *mut c_void, &ORIGINAL_AGENCY_MATERIALIZER),
        (sites.agency_filter, hook_agency_filter as *mut c_void, &ORIGINAL_AGENCY_FILTER),
        (sites.cipher_outer, hook_cipher_outer as *mut c_void, &ORIGINAL_CIPHER_OUTER),
        (sites.cipher_row, hook_cipher_row as *mut c_void, &ORIGINAL_CIPHER_ROW),
    ];
    let mut created = Vec::new();
    for (target, detour, original_slot) in hooks {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(original) => {
                original_slot.store(original as usize, Ordering::Release);
                created.push(target);
            }
            Err(status) => {
                if let Err(cleanup) = rollback_created(&created) {
                    INSTALLED.store(true, Ordering::Release);
                    return Err(format!(
                        "A30 hook create failed at 0x{target:X}: {status}; rollback unsafe: {cleanup}"
                    ));
                }
                clear_hook_state();
                return Err(format!("A30 hook create failed at 0x{target:X}: {status}"));
            }
        }
    }
    // Nested observers become active first; an outer hook is never able to
    // publish a record while its inner observation boundary is absent.
    for target in [sites.agency_owner_resolver, sites.agency_filter, sites.agency_materializer, sites.cipher_row, sites.agency_outer, sites.cipher_outer] {
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            if let Err(cleanup) = rollback_created(&created) {
                INSTALLED.store(true, Ordering::Release);
                return Err(format!(
                    "A30 hook enable failed at 0x{target:X}: {status}; rollback unsafe: {cleanup}"
                ));
            }
            clear_hook_state();
            return Err(format!("A30 hook enable failed at 0x{target:X}: {status}"));
        }
    }
    EVIDENCE_INSTALL.store(evidence_install, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

pub(crate) fn verify_current_process_executable() -> Result<(), String> {
    CURRENT_EXECUTABLE_VERIFICATION
        .get_or_init(verify_current_process_executable_uncached)
        .clone()
}

fn verify_current_process_executable_uncached() -> Result<(), String> {
    let mut path = [0u16; 32_768];
    let len = unsafe { GetModuleFileNameW(ptr::null_mut(), path.as_mut_ptr(), path.len() as u32) }
        as usize;
    if len == 0 || len >= path.len() {
        return Err("A30 current executable path is unavailable".into());
    }
    let path = PathBuf::from(OsString::from_wide(&path[..len]));
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("A30 current executable open failed: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("A30 current executable read failed: {error}"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if format!("{:x}", digest.finalize()) != EXECUTABLE_SHA256 {
        return Err("A30 current executable SHA-256 differs from exact a729".into());
    }
    Ok(())
}

pub fn drain() -> Result<HookDrain, String> {
    let records = queue()
        .lock()
        .map_err(|_| "A30 record queue lock poisoned")?
        .drain(..)
        .collect::<Vec<_>>();
    Ok(HookDrain {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        module_base: MODULE_BASE.load(Ordering::Acquire) as u64,
        evidence_install: EVIDENCE_INSTALL.load(Ordering::Acquire),
        diagnostics: diagnostics(records.len() as u64),
        records,
    })
}

pub fn receipt_from_drain(drain: &HookDrain) -> ControllerEvidenceReceipt {
    let sites = addresses(drain.module_base as usize);
    let strict_sequence = drain.records.first().is_none_or(|record| record.sequence != 0)
        && drain.records.windows(2).all(|pair| {
            pair[0].sequence.checked_add(1) == Some(pair[1].sequence)
        });
    let domain53_multiplex_ready = drain.hook_epoch != 0
        && drain.module_base != 0
        && !drain.records.is_empty()
        && !drain.diagnostics.completeness_poisoned
        && strict_sequence
        && drain.records.iter().all(|record| record.validate_carrier().is_ok());
    ControllerEvidenceReceipt {
        schema: "hoi4.a30.controller_evidence.v1".into(),
        executable_sha256: EXECUTABLE_SHA256.into(),
        image_base: IMAGE_BASE as u64,
        agency_job_id: AGENCY_JOB_ID.into(),
        agency_manifest_sha256: AGENCY_MANIFEST_SHA256.into(),
        cipher_controller_job_id: CIPHER_CONTROLLER_JOB_ID.into(),
        cipher_controller_manifest_sha256: CIPHER_CONTROLLER_MANIFEST_SHA256.into(),
        cipher_row_job_id: CIPHER_ROW_JOB_ID.into(),
        cipher_row_manifest_sha256: CIPHER_ROW_MANIFEST_SHA256.into(),
        sentinel_dump_job_id: SENTINEL_DUMP_JOB_ID.into(),
        sentinel_dump_identity_manifest_sha256: SENTINEL_DUMP_IDENTITY_MANIFEST_SHA256.into(),
        sentinel_dump_artifact_sha256: SENTINEL_DUMP_ARTIFACT_SHA256.into(),
        agency_owner_resolver_job_id: AGENCY_OWNER_RESOLVER_JOB_ID.into(),
        agency_owner_resolver_manifest_sha256: AGENCY_OWNER_RESOLVER_MANIFEST_SHA256.into(),
        process_identity: current_process_identity(),
        evidence_install: drain.evidence_install,
        hook_epoch: drain.hook_epoch,
        hook_sites: hook_site_receipts(sites, drain.module_base as usize),
        records: drain.records.iter().map(record_receipt).collect(),
        diagnostics: drain.diagnostics.clone(),
        domain53_multiplex_ready,
        production_scheduling_enabled: !drain.evidence_install,
    }
}

pub fn visibility_products_from_drain(
    drain: &HookDrain,
) -> Result<super::a30_visibility::VisibilityProductJoins, String> {
    let mut agency = None;
    let mut cipher = None;
    for record in &drain.records {
        match record.family {
            ControllerEvidenceFamily::AgencyUpgrade => {
                if agency.is_some() { return Err("A30 drain repeats agency refresh".into()); }
                agency = Some(crate::agency_upgrade_controller::AgencyUpgradeControllerRefresh::decode(&record.payload)
                    .map_err(|error| format!("A30 AUGC drain decode: {error:?}"))?);
            }
            ControllerEvidenceFamily::CipherCountry => {
                if cipher.is_some() { return Err("A30 drain repeats cipher refresh".into()); }
                cipher = Some(crate::cipher_controller::CipherControllerRefresh::decode(&record.payload)
                    .map_err(|error| format!("A30 CIPC drain decode: {error:?}"))?);
            }
        }
    }
    super::a30_visibility::VisibilityProductJoins::from_controller_refreshes(
        agency.as_ref().ok_or("A30 drain has no agency refresh")?,
        cipher.as_ref().ok_or("A30 drain has no cipher refresh")?,
    )
}

pub fn seal_drain_and_uninstall() -> Result<HookDrain, String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A30 teardown lock poisoned")?;
    if !installed() {
        return drain();
    }
    TEARING_DOWN.store(true, Ordering::Release);
    let sites = addresses(MODULE_BASE.load(Ordering::Acquire));
    // Stop new atomic refreshes before disabling their nested observers.
    disable(sites.agency_outer)?;
    disable(sites.cipher_outer)?;
    wait_for_callbacks()?;
    for target in [sites.agency_materializer, sites.agency_filter, sites.agency_owner_resolver, sites.cipher_row] {
        disable(target)?;
    }
    wait_for_callbacks()?;
    let mut armed = armed().lock().map_err(|_| "A30 arm lock poisoned at teardown")?;
    let unconsumed = u64::from(armed.agency.take().is_some())
        + u64::from(armed.cipher.take().is_some());
    drop(armed);
    if unconsumed != 0 {
        DROPPED_UNCONSUMED_ARM.fetch_add(unconsumed, Ordering::AcqRel);
        COMPLETENESS_POISONED.store(true, Ordering::Release);
    }
    let drained = drain()?;
    for target in [sites.cipher_outer, sites.agency_outer, sites.cipher_row, sites.agency_materializer, sites.agency_filter, sites.agency_owner_resolver] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A30 hook remove failed at 0x{target:X}: {status}")),
        }
    }
    INSTALLED.store(false, Ordering::Release);
    clear_hook_state();
    Ok(drained)
}

unsafe extern "system" fn hook_agency_outer(controller: usize) {
    let _active = CallbackGuard::enter();
    let Some(original) = original_outer(&ORIGINAL_AGENCY_OUTER) else { poison(&DROPPED_UNREADABLE); return };
    LAST_AGENCY_CONTROLLER.store(controller, Ordering::Release);
    if TEARING_DOWN.load(Ordering::Acquire) {
        unsafe { original(controller) };
        return;
    }
    if OPEN_AGENCY.with(|slot| slot.borrow().is_some()) {
        poison(&DROPPED_REENTRANT);
        unsafe { original(controller) };
        return;
    }
    let arm = take_agency_arm(controller);
    let Some(arm) = arm else {
        IGNORED_UNARMED_REFRESHES.fetch_add(1, Ordering::AcqRel);
        unsafe { original(controller) };
        return;
    };
    OPEN_AGENCY.with(|slot| *slot.borrow_mut() = Some(OpenAgency {
        builder: None,
        arm: arm.clone(),
        owner_resolved: false,
        expected_controller: controller,
        next_branch_index: 1,
        pending_branch: None,
        poisoned: false,
    }));
    unsafe { original(controller) };
    let open = OPEN_AGENCY.with(|slot| slot.borrow_mut().take());
    let Some(mut open) = open else { poison(&DROPPED_UNREADABLE); return };
    if open.expected_controller != controller
        || !open.owner_resolved
        || open.poisoned
        || open.pending_branch.is_some()
    {
        poison(&DROPPED_UNREADABLE);
        return;
    }
    let Some(builder) = open.builder.take() else { poison(&DROPPED_UNREADABLE); return };
    match builder.finish().and_then(|refresh| refresh.encode()) {
        Ok(payload) => retain(ControllerEvidenceFamily::AgencyUpgrade, arm.frame_hour, payload),
        Err(_) => poison(&DROPPED_UNREADABLE),
    }
}

/// CCountry resolver. This closes the previous trust gap where the arm's human
/// country could be accepted without proving that the active agency controller
/// resolved the same country.
unsafe extern "system" fn hook_agency_owner_resolver(country_id: usize) -> usize {
    let _active = CallbackGuard::enter();
    let address = ORIGINAL_AGENCY_OWNER_RESOLVER.load(Ordering::Acquire);
    if address == 0 {
        poison(&DROPPED_UNREADABLE);
        return 0;
    }
    let original: AgencyOwnerResolverFn = unsafe { mem::transmute(address) };
    let result = unsafe { original(country_id) };
    OPEN_AGENCY.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(open) = slot.as_mut() else { return };
        if open.owner_resolved || open.builder.is_some() {
            open.poisoned = true;
            return;
        }
        let observed_owner = read_stable::<u32>(country_id);
        if observed_owner != Some(open.arm.owner_tag_db_id) {
            open.poisoned = true;
            return;
        }
        let memory = InProcessMemory;
        match AgencyUpgradeRefreshBuilder::begin(
            &memory,
            open.arm.frame_hour,
            open.arm.owner_tag_db_id,
            &open.arm.valid_country_tag_db_ids,
            open.arm.producer_record_identity,
        ) {
            Ok(builder) => {
                open.builder = Some(builder);
                open.owner_resolved = true;
            }
            Err(_) => open.poisoned = true,
        }
    });
    result
}

unsafe extern "system" fn hook_agency_materializer(a1: usize, a2: usize, branch: usize, a4: usize, a5: usize) -> usize {
    let _active = CallbackGuard::enter();
    let address = ORIGINAL_AGENCY_MATERIALIZER.load(Ordering::Acquire);
    if address == 0 { poison(&DROPPED_UNREADABLE); return 0; }
    let original: AgencyMaterializerFn = unsafe { mem::transmute(address) };
    OPEN_AGENCY.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(open) = slot.as_mut() else { return };
        if open.pending_branch.is_some() || branch == 0 {
            open.poisoned = true;
        } else {
            open.pending_branch = Some((open.next_branch_index, branch as u64));
        }
    });
    let result = unsafe { original(a1, a2, branch, a4, a5) };
    OPEN_AGENCY.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(open) = slot.as_mut() {
            if open.pending_branch.is_some() { open.poisoned = true; }
        }
    });
    result
}

unsafe extern "system" fn hook_agency_filter(source: usize, output: usize, predicate: usize) -> usize {
    let _active = CallbackGuard::enter();
    let address = ORIGINAL_AGENCY_FILTER.load(Ordering::Acquire);
    if address == 0 { poison(&DROPPED_UNREADABLE); return 0; }
    let original: AgencyFilterFn = unsafe { mem::transmute(address) };
    let result = unsafe { original(source, output, predicate) };
    OPEN_AGENCY.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(open) = slot.as_mut() else { return };
        let Some((branch_index, branch_definition)) = open.pending_branch.take() else { return };
        let memory = InProcessMemory;
        let vector = memory.read_u64(output as u64);
        let count = output.checked_add(12).and_then(read_stable::<u32>);
        let observed = match (open.builder.as_mut(), vector, count) {
            (Some(builder), Ok(vector), Some(count)) => builder.observe_filtered_branch(
                &memory, branch_index, branch_definition, vector, count,
            ).is_ok(),
            _ => false,
        };
        if observed { open.next_branch_index = open.next_branch_index.saturating_add(1); }
        else { open.poisoned = true; }
    });
    result
}

unsafe extern "system" fn hook_cipher_outer(controller: usize) {
    let _active = CallbackGuard::enter();
    let Some(original) = original_outer(&ORIGINAL_CIPHER_OUTER) else { poison(&DROPPED_UNREADABLE); return };
    LAST_CIPHER_CONTROLLER.store(controller, Ordering::Release);
    if TEARING_DOWN.load(Ordering::Acquire) {
        unsafe { original(controller) };
        return;
    }
    if OPEN_CIPHER.with(|slot| slot.borrow().is_some()) {
        poison(&DROPPED_REENTRANT);
        unsafe { original(controller) };
        return;
    }
    let arm = take_cipher_arm(controller);
    let Some(arm) = arm else {
        IGNORED_UNARMED_REFRESHES.fetch_add(1, Ordering::AcqRel);
        unsafe { original(controller) };
        return;
    };
    let builder = CipherControllerRefreshBuilder::begin(
        arm.frame_hour, arm.owner_tag_db_id, arm.selected_target_tag_db_id,
        arm.producer_record_identity, &arm.valid_country_tag_db_ids,
    ).ok();
    let poisoned = builder.is_none();
    OPEN_CIPHER.with(|slot| *slot.borrow_mut() = Some(OpenCipher {
        builder, expected_controller: controller, poisoned,
    }));
    unsafe { original(controller) };
    let open = OPEN_CIPHER.with(|slot| slot.borrow_mut().take());
    let Some(mut open) = open else { poison(&DROPPED_UNREADABLE); return };
    if open.expected_controller != controller || open.poisoned {
        poison(&DROPPED_UNREADABLE);
        return;
    }
    let Some(builder) = open.builder.take() else { poison(&DROPPED_UNREADABLE); return };
    match builder.finish().and_then(|refresh| refresh.encode()) {
        Ok(payload) => retain(ControllerEvidenceFamily::CipherCountry, arm.frame_hour, payload),
        Err(_) => poison(&DROPPED_UNREADABLE),
    }
}

unsafe extern "system" fn hook_cipher_row(row: usize, target_cid: usize, selected: u8) {
    let _active = CallbackGuard::enter();
    let address = ORIGINAL_CIPHER_ROW.load(Ordering::Acquire);
    if address == 0 { poison(&DROPPED_UNREADABLE); return; }
    let original: CipherRowFn = unsafe { mem::transmute(address) };
    OPEN_CIPHER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(open) = slot.as_mut() else { return };
        let cid = read_stable::<u32>(target_cid);
        if cid.is_none_or(|cid| open.builder.as_mut().is_none_or(|builder| builder.observe_bound_target(cid, selected).is_err())) {
            open.poisoned = true;
        }
    });
    unsafe { original(row, target_cid, selected) }
}

fn take_agency_arm(controller: usize) -> Option<AgencyRefreshArm> {
    match armed().try_lock() {
        Ok(mut armed) if armed.agency.as_ref().is_some_and(|arm| arm.expected_controller == controller) => armed.agency.take(),
        Ok(_) => None,
        Err(_) => { poison(&DROPPED_LOCK_BUSY); None }
    }
}

fn take_cipher_arm(controller: usize) -> Option<CipherRefreshArm> {
    match armed().try_lock() {
        Ok(mut armed) if armed.cipher.as_ref().is_some_and(|arm| arm.expected_controller == controller) => armed.cipher.take(),
        Ok(_) => None,
        Err(_) => { poison(&DROPPED_LOCK_BUSY); None }
    }
}

fn validate_agency_arm(arm: &AgencyRefreshArm) -> Result<(), String> {
    if arm.expected_controller == 0 || arm.owner_tag_db_id == 0
        || !arm.valid_country_tag_db_ids.contains(&arm.owner_tag_db_id)
        || arm.producer_record_identity.iter().all(|byte| *byte == 0)
    { return Err("A30 agency arm is incomplete".into()); }
    Ok(())
}

fn validate_cipher_arm(arm: &CipherRefreshArm) -> Result<(), String> {
    if arm.expected_controller == 0 || arm.owner_tag_db_id == 0
        || !arm.valid_country_tag_db_ids.contains(&arm.owner_tag_db_id)
        || (arm.selected_target_tag_db_id != 0 && !arm.valid_country_tag_db_ids.contains(&arm.selected_target_tag_db_id))
        || arm.producer_record_identity.iter().all(|byte| *byte == 0)
    { return Err("A30 cipher arm is incomplete".into()); }
    Ok(())
}

fn retain(family: ControllerEvidenceFamily, frame_hour: u64, payload: Vec<u8>) {
    let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if sequence == 0 { poison(&DROPPED_OVERFLOW); return; }
    let record = Domain53ControllerEvidenceRecord {
        sequence, thread_id: unsafe { GetCurrentThreadId() }, family, frame_hour, payload,
    };
    match queue().try_lock() {
        Ok(mut queue) if queue.len() < MAX_PENDING_RECORDS => queue.push_back(record),
        Ok(_) => poison(&DROPPED_OVERFLOW),
        Err(_) => poison(&DROPPED_LOCK_BUSY),
    }
}

fn armed() -> &'static Mutex<ArmedRefreshes> {
    ARMED.get_or_init(|| Mutex::new(ArmedRefreshes::default()))
}
fn queue() -> &'static Mutex<VecDeque<Domain53ControllerEvidenceRecord>> {
    QUEUE.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_RECORDS)))
}

fn reset_state(epoch: u64) -> Result<(), String> {
    *armed().lock().map_err(|_| "A30 arm lock poisoned before install")? = ArmedRefreshes::default();
    queue().lock().map_err(|_| "A30 queue lock poisoned before install")?.clear();
    ACTIVE_HOOK_EPOCH.store(epoch, Ordering::Release);
    NEXT_SEQUENCE.store(1, Ordering::Release);
    TEARING_DOWN.store(false, Ordering::Release);
    for counter in [&IGNORED_UNARMED_REFRESHES, &DROPPED_REENTRANT, &DROPPED_UNREADABLE, &DROPPED_LOCK_BUSY, &DROPPED_OVERFLOW, &DROPPED_UNCONSUMED_ARM] {
        counter.store(0, Ordering::Release);
    }
    COMPLETENESS_POISONED.store(false, Ordering::Release);
    LAST_AGENCY_CONTROLLER.store(0, Ordering::Release);
    LAST_CIPHER_CONTROLLER.store(0, Ordering::Release);
    Ok(())
}

fn diagnostics(retained_records: u64) -> ControllerEvidenceDiagnostics {
    ControllerEvidenceDiagnostics {
        retained_records,
        ignored_unarmed_refreshes: IGNORED_UNARMED_REFRESHES.load(Ordering::Acquire),
        dropped_reentrant: DROPPED_REENTRANT.load(Ordering::Acquire),
        dropped_unreadable: DROPPED_UNREADABLE.load(Ordering::Acquire),
        dropped_lock_busy: DROPPED_LOCK_BUSY.load(Ordering::Acquire),
        dropped_overflow: DROPPED_OVERFLOW.load(Ordering::Acquire),
        dropped_unconsumed_arm: DROPPED_UNCONSUMED_ARM.load(Ordering::Acquire),
        completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
    }
}

fn poison(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::AcqRel);
    COMPLETENESS_POISONED.store(true, Ordering::Release);
}

fn original_outer(slot: &AtomicUsize) -> Option<ControllerOuterFn> {
    let address = slot.load(Ordering::Acquire);
    (address != 0).then(|| unsafe { mem::transmute(address) })
}

pub(crate) struct InProcessMemory;
impl Domain53Memory for InProcessMemory {
    fn module_base(&self) -> u64 { MODULE_BASE.load(Ordering::Acquire) as u64 }
    fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
        let first = read_bytes(address as usize, len)
            .ok_or(ExtractError::MemoryRead { address, len })?;
        let second = read_bytes(address as usize, len)
            .ok_or(ExtractError::MemoryRead { address, len })?;
        if first != second {
            return Err(ExtractError::Validation(
                "a30_controller_evidence_torn_read".into(),
            ));
        }
        Ok(first)
    }
}

fn read_stable<T: Copy + PartialEq>(address: usize) -> Option<T> {
    let first = read_value(address)?;
    let second = read_value(address)?;
    (first == second).then_some(first)
}
fn read_value<T: Copy>(address: usize) -> Option<T> {
    let bytes = read_bytes(address, mem::size_of::<T>())?;
    Some(unsafe { ptr::read_unaligned(bytes.as_ptr().cast::<T>()) })
}
fn read_bytes(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 || len == 0 { return None; }
    let mut bytes = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), address as *const c_void, bytes.as_mut_ptr().cast(), len, &mut read) };
    (ok != 0 && read == len).then_some(bytes)
}

fn addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        agency_outer: base + A729_AGENCY_UPGRADE_REFRESH_RVA,
        agency_materializer: base + A729_AGENCY_UPGRADE_BRANCH_MATERIALIZER_RVA,
        agency_filter: base + A729_AGENCY_UPGRADE_FILTER_RVA,
        cipher_outer: base + A729_CIPHER_CONTROLLER_RVA,
        cipher_row: base + A729_CIPHER_ROW_BIND_RVA,
        agency_owner_resolver: base + AGENCY_OWNER_RESOLVER_RVA,
    }
}
fn site_sentinels(sites: InstalledHookAddresses) -> [(usize, &'static [u8]); 6] {
    [
        (sites.agency_owner_resolver, &AGENCY_OWNER_RESOLVER_SENTINEL),
        (sites.agency_outer, &AGENCY_OUTER_SENTINEL),
        (sites.agency_materializer, &AGENCY_MATERIALIZER_SENTINEL),
        (sites.agency_filter, &AGENCY_FILTER_SENTINEL),
        (sites.cipher_outer, &CIPHER_OUTER_SENTINEL),
        (sites.cipher_row, &CIPHER_ROW_SENTINEL),
    ]
}
fn verify_sentinel(address: usize, expected: &[u8]) -> Result<(), String> {
    let actual = read_bytes(address, expected.len()).ok_or_else(|| format!("A30 sentinel unreadable at 0x{address:X}"))?;
    if actual != expected { return Err(format!("A30 exact-a729 sentinel mismatch at 0x{address:X}")); }
    Ok(())
}
fn disable(target: usize) -> Result<(), String> {
    match unsafe { MinHook::disable_hook(target as *mut c_void) } {
        Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => Ok(()),
        Err(status) => Err(format!("A30 hook disable failed at 0x{target:X}: {status}")),
    }
}
fn rollback_created(created: &[usize]) -> Result<(), String> {
    for target in created.iter().rev().copied() { disable(target)?; }
    wait_for_callbacks()?;
    for target in created.iter().rev().copied() {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A30 rollback remove failed at 0x{target:X}: {status}")),
        }
    }
    Ok(())
}
fn wait_for_callbacks() -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline { return Err("A30 callbacks did not quiesce within 10 seconds".into()); }
        std::thread::yield_now();
    }
    Ok(())
}
fn clear_hook_state() {
    for original in [&ORIGINAL_AGENCY_OUTER, &ORIGINAL_AGENCY_MATERIALIZER, &ORIGINAL_AGENCY_FILTER, &ORIGINAL_CIPHER_OUTER, &ORIGINAL_CIPHER_ROW, &ORIGINAL_AGENCY_OWNER_RESOLVER] {
        original.store(0, Ordering::Release);
    }
    MODULE_BASE.store(0, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    TEARING_DOWN.store(false, Ordering::Release);
    EVIDENCE_INSTALL.store(false, Ordering::Release);
}

fn current_process_identity() -> ProcessIdentityReceipt {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let process = unsafe { GetCurrentProcess() };
    let has_times = unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) } != 0;
    let create_time_100ns = if has_times { (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime) } else { 0 };
    ProcessIdentityReceipt { pid: unsafe { GetCurrentProcessId() }, create_time_100ns, executable_sha256: EXECUTABLE_SHA256.into() }
}
fn sha256(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}
fn record_receipt(record: &Domain53ControllerEvidenceRecord) -> EncodedRecordReceipt {
    let (owner_tag_db_id, selected_target_tag_db_id, producer_record_identity) = match record.family {
        ControllerEvidenceFamily::AgencyUpgrade => {
            match crate::agency_upgrade_controller::AgencyUpgradeControllerRefresh::decode(&record.payload) {
                Ok(refresh) => (Some(refresh.owner_tag_db_id), None, Some(hex(&refresh.producer_record_identity))),
                Err(_) => (None, None, None),
            }
        }
        ControllerEvidenceFamily::CipherCountry => {
            match crate::cipher_controller::CipherControllerRefresh::decode(&record.payload) {
                Ok(refresh) => (
                    Some(refresh.owner_tag_db_id),
                    Some(refresh.selected_target_tag_db_id),
                    Some(hex(&refresh.producer_record_identity)),
                ),
                Err(_) => (None, None, None),
            }
        }
    };
    EncodedRecordReceipt {
        sequence: record.sequence,
        thread_id: record.thread_id,
        family: record.family,
        frame_hour: record.frame_hour,
        payload_bytes: record.payload.len() as u64,
        payload_sha256: sha256(&record.payload),
        owner_tag_db_id,
        selected_target_tag_db_id,
        producer_record_identity,
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn hook_site_receipts(sites: InstalledHookAddresses, module_base: usize) -> Vec<HookSiteReceipt> {
    let artifact_hashes = [
        AGENCY_OWNER_RESOLVER_INSTRUCTIONS_SHA256,
        "792c01c7522c6ea3fa40b7ba26897a98de27d9d8a3d244441c1cb6157ba1c756",
        "60236d63cdeb51f62b73c89bd7fc773a354f06741b683ce8651d483f83b46934",
        "34e50c09f850eb6a6733be2f987c5929fc7d6ceb8240f1776a953e3e8e70e54c",
        "6368de5e6359edf502702a93093e20a6ddb69bccd1b7bc7894ab185231b50f73",
        "00a567e6bd65a98ae4e3ebcd865f71bd14b85d984c134fdec97f21c2a8469842",
    ];
    site_sentinels(sites).into_iter().zip(artifact_hashes).map(|((address, sentinel), artifact_sha256)| HookSiteReceipt {
        rva: address.saturating_sub(module_base) as u32,
        installed_address: address as u64,
        sentinel_sha256: sha256(sentinel),
        artifact_sha256: artifact_sha256.into(),
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher_record(frame_hour: u64) -> Domain53ControllerEvidenceRecord {
        let countries = BTreeSet::from([1, 2, 3]);
        let mut builder = CipherControllerRefreshBuilder::begin(
            frame_hour,
            1,
            2,
            [0x5a; 32],
            &countries,
        )
        .unwrap();
        builder.observe_bound_target(2, 1).unwrap();
        builder.observe_bound_target(3, 0).unwrap();
        Domain53ControllerEvidenceRecord {
            sequence: 1,
            thread_id: 7,
            family: ControllerEvidenceFamily::CipherCountry,
            frame_hour,
            payload: builder.finish().unwrap().encode().unwrap(),
        }
    }

    #[test]
    fn pointer_free_carrier_reaches_domain53_h4cr_with_owner_sequence() {
        let record = cipher_record(900);
        let h4cr = record.encode_h4cr(41).unwrap();
        assert_eq!(&h4cr[..4], b"H4CR");
        assert_eq!(u64::from_le_bytes(h4cr[8..16].try_into().unwrap()), 41);
        assert_eq!(i32::from_le_bytes(h4cr[16..20].try_into().unwrap()), 900);
        assert_eq!(u16::from_le_bytes(h4cr[20..22].try_into().unwrap()), crate::DOMAIN_ID);
        assert_eq!(&h4cr[32..], record.payload.as_slice());
    }

    #[test]
    fn evidence_receipt_is_authenticated_but_cannot_enable_production_scheduling() {
        let record = cipher_record(900);
        let drain = HookDrain {
            hook_epoch: 3,
            module_base: IMAGE_BASE as u64,
            evidence_install: true,
            diagnostics: ControllerEvidenceDiagnostics {
                retained_records: 1,
                ..ControllerEvidenceDiagnostics::default()
            },
            records: vec![record],
        };
        let receipt = receipt_from_drain(&drain);
        assert!(receipt.domain53_multiplex_ready);
        assert!(!receipt.production_scheduling_enabled);
        assert_eq!(receipt.hook_sites.len(), 6);
        assert_eq!(receipt.records[0].owner_tag_db_id, Some(1));
        assert_eq!(receipt.records[0].selected_target_tag_db_id, Some(2));
        let expected_record_identity = "5a".repeat(32);
        assert_eq!(
            receipt.records[0].producer_record_identity.as_deref(),
            Some(expected_record_identity.as_str())
        );
    }

    #[test]
    fn envelope_hour_mismatch_fails_before_h4cr_framing() {
        let mut record = cipher_record(900);
        record.frame_hour = 901;
        assert!(record.encode_h4cr(41).is_err());
    }
}
