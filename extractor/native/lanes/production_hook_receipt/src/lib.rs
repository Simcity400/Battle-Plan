//! Pointer-free production hook custody receipts.
//!
//! This crate does not install hooks.  It observes the already-installed
//! target bytes and the loaded main module, then fails closed unless they are
//! the exact a729 executable and admitted patch primitive.  Teardown receipts
//! are minted only after the original target prefix is restored.

use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
#[cfg(windows)]
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    mem::size_of,
    sync::{Mutex, OnceLock},
};

pub const A729_EXE_SHA256: [u8; 32] =
    hex_digest("870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const BATTLE_PLAN_MODULE_NAME: &str = "hoi4.exe";
pub const MINHOOK_NORMAL_REL5_PRIMITIVE_ID: &str = "minhook-0.9.0.normal-jmp-rel5";
pub const D90_REL32_CALLSITE_PRIMITIVE_ID: &str = "naval_combat.convoy_terminal.rel32_callsite.v1";
pub const PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS_PRIMITIVE_ID: &str =
    "pe64.import-address-slot.atomic-usize-cas.v1";
pub const PE64_IMPORT_ADDRESS_SLOT_PROCESS_LIFETIME_POLICY_ID: &str =
    "pe64.import-address-slot.external-dispatch.process-lifetime.v1";

const MAX_EXTERNAL_DISPATCH_SHIM_LEN: usize = 4 * 1024;
const MAX_EXTERNAL_DISPATCH_ALLOCATION_LEN: usize = 64 * 1024 * 1024;

static NEXT_OWNER_TOKEN: AtomicU64 = AtomicU64::new(1);
static NEXT_RECEIPT_SEAL: AtomicU64 = AtomicU64::new(1);

#[cfg(windows)]
static HOOK_CUSTODY: OnceLock<Mutex<BTreeMap<(u64, u64), HookCustodyState>>> = OnceLock::new();
#[cfg(windows)]
static PE64_IMPORT_SLOT_CUSTODY: OnceLock<Mutex<BTreeMap<(u64, u64), Pe64ImportSlotCustodyState>>> =
    OnceLock::new();
#[cfg(windows)]
static MAIN_MODULE_AUTHORITY: OnceLock<Result<ObservedMainModule, String>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookPatchPlacement {
    NormalTarget,
    PatchAboveSplit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpectedRvaHook {
    pub logical_name: &'static str,
    pub requested_rva: u64,
    pub target_patch_len: u16,
    pub primitive_id: &'static str,
    pub original_prefix: &'static [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpectedPe64ImportAddressSlot {
    pub logical_name: &'static str,
    pub slot_rva: u64,
    pub imported_dll: &'static str,
    pub imported_symbol: &'static str,
    /// Exact current-process export pointer expected before installation.
    pub expected_original_address: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalDispatchShimIdentity {
    pub entry_address: usize,
    pub entry_len: usize,
    pub entry_bytes_sha256: [u8; 32],
    pub allocation_base: usize,
    pub allocation_len: usize,
    pub dispatch_slot_address: usize,
    pub active_callback_counter_address: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledPe64ImportAddressSlotReceipt {
    pub logical_name: &'static str,
    pub observed_module_name: &'static str,
    pub observed_module_sha256: [u8; 32],
    pub observed_pe_identity_sha256: [u8; 32],
    pub observed_pe_timestamp: u32,
    pub observed_size_of_image: u32,
    pub imported_dll: &'static str,
    pub imported_symbol: &'static str,
    pub slot_rva: u64,
    /// Process-local diagnostic only. Never serialize this field.
    pub slot_address: usize,
    pub slot_width: u16,
    pub slot_alignment: u16,
    pub expected_original_address: usize,
    pub external_shim_address: usize,
    pub external_shim: ExternalDispatchShimIdentity,
    pub observed_before_address: usize,
    pub observed_after_address: usize,
    pub slot_protection_before: u32,
    pub slot_protection_temporary: u32,
    pub slot_protection_after: u32,
    pub primitive_id: &'static str,
    pub slot_spec_sha256: [u8; 32],
    pub teardown_owner_token: u64,
    pub installed: bool,
    authenticity_seal: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pe64ImportAddressSlotTerminalAttestation {
    pub callbacks_quiesced: bool,
    pub slot_dispatches_to_original: bool,
    pub retained_until_process_exit: bool,
    pub policy_id: &'static str,
    pub external_shim: ExternalDispatchShimIdentity,
}

/// Terminal custody for a restored PE64 import slot whose external dispatch
/// allocation remains resident because a thread may already have loaded the
/// old shim pointer. This is deliberately not `ReleasedRvaHookReceipt`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoredPe64ImportAddressSlotCustodyReceipt {
    pub logical_name: &'static str,
    pub observed_module_name: &'static str,
    pub observed_module_sha256: [u8; 32],
    pub observed_pe_identity_sha256: [u8; 32],
    pub observed_pe_timestamp: u32,
    pub observed_size_of_image: u32,
    pub imported_dll: &'static str,
    pub imported_symbol: &'static str,
    pub slot_rva: u64,
    /// Process-local diagnostic only. Never serialize this field.
    pub slot_address: usize,
    pub slot_width: u16,
    pub slot_alignment: u16,
    pub expected_original_address: usize,
    pub external_shim_address: usize,
    pub observed_restore_before_address: usize,
    pub observed_restore_after_address: usize,
    pub slot_protection_before_restore: u32,
    pub slot_protection_temporary_restore: u32,
    pub slot_protection_after_restore: u32,
    pub primitive_id: &'static str,
    pub slot_spec_sha256: [u8; 32],
    pub teardown_owner_token: u64,
    pub callbacks_quiesced: bool,
    pub slot_restored: bool,
    pub retained_until_process_exit: bool,
    pub policy_id: &'static str,
    pub external_shim: ExternalDispatchShimIdentity,
    pub dispatch_slot_sha256: [u8; 32],
    pub observed_terminal_dispatch_address: usize,
    pub active_callback_counter_sha256: [u8; 32],
    pub observed_active_callback_count: u64,
    pub custody_digest_sha256: [u8; 32],
    authenticity_seal: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstalledRvaHookReceipt {
    pub logical_name: &'static str,
    pub observed_module_name: &'static str,
    pub observed_module_sha256: [u8; 32],
    pub observed_pe_identity_sha256: [u8; 32],
    pub observed_pe_timestamp: u32,
    pub observed_size_of_image: u32,
    pub requested_rva: u64,
    pub observed_rva: u64,
    /// Process-local diagnostic only.  Never serialize this field.
    pub observed_runtime_address: usize,
    pub target_patch_len: u16,
    pub primitive_id: &'static str,
    pub hook_spec_sha256: [u8; 32],
    pub original_prefix_len: u16,
    pub original_prefix_sha256: [u8; 32],
    /// Process-local diagnostic only. Never serialize this field.
    pub observed_relay_address: usize,
    /// Process-local diagnostic only. Never serialize this field.
    pub observed_detour_address: usize,
    pub placement: HookPatchPlacement,
    pub patch_above: bool,
    pub enabled: bool,
    pub teardown_owner_token: u64,
    authenticity_seal: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HookTeardownAttestation {
    pub callbacks_quiesced: bool,
    pub hook_removed: bool,
    pub relay_freed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleasedRvaHookReceipt {
    pub logical_name: &'static str,
    pub observed_module_name: &'static str,
    pub observed_module_sha256: [u8; 32],
    pub observed_pe_identity_sha256: [u8; 32],
    pub observed_pe_timestamp: u32,
    pub observed_size_of_image: u32,
    pub observed_rva: u64,
    /// Process-local diagnostic only.  Never serialize this field.
    pub observed_runtime_address: usize,
    pub target_patch_len: u16,
    pub primitive_id: &'static str,
    pub hook_spec_sha256: [u8; 32],
    pub original_prefix_len: u16,
    pub original_prefix_sha256: [u8; 32],
    /// Process-local diagnostic only. Never serialize this field.
    pub observed_relay_address: usize,
    /// Process-local diagnostic only. Never serialize this field.
    pub observed_detour_address: usize,
    pub teardown_owner_token: u64,
    pub restored_prefix_sha256: [u8; 32],
    pub callbacks_quiesced: bool,
    pub hook_removed: bool,
    pub relay_freed: bool,
    pub released: bool,
    authenticity_seal: u64,
}

/// Dev-only receipt constructor for dependent lane tests. It is unavailable
/// from every default/shipped build and deliberately bypasses the process
/// registry so tests can exercise higher-level opaque custody flows without
/// patching the running test executable.
#[cfg(feature = "test-support")]
pub fn mint_test_released_receipt(
    expected: ExpectedRvaHook,
    teardown_owner_token: u64,
) -> ReleasedRvaHookReceipt {
    let original_prefix_sha256: [u8; 32] = Sha256::digest(expected.original_prefix).into();
    ReleasedRvaHookReceipt {
        logical_name: expected.logical_name,
        observed_module_name: BATTLE_PLAN_MODULE_NAME,
        observed_module_sha256: A729_EXE_SHA256,
        observed_pe_identity_sha256: [0xA7; 32],
        observed_pe_timestamp: 1,
        observed_size_of_image: 1,
        observed_rva: expected.requested_rva,
        observed_runtime_address: usize::try_from(expected.requested_rva).unwrap_or(1).max(1),
        target_patch_len: expected.target_patch_len,
        primitive_id: expected.primitive_id,
        hook_spec_sha256: hook_spec_sha256(expected),
        original_prefix_len: expected.original_prefix.len() as u16,
        original_prefix_sha256,
        observed_relay_address: 1,
        observed_detour_address: 1,
        teardown_owner_token,
        restored_prefix_sha256: original_prefix_sha256,
        callbacks_quiesced: true,
        hook_removed: true,
        relay_freed: true,
        released: true,
        authenticity_seal: 1,
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveHookBinding {
    logical_name: &'static str,
    observed_module_sha256: [u8; 32],
    observed_pe_identity_sha256: [u8; 32],
    requested_rva: u64,
    observed_runtime_address: usize,
    target_patch_len: u16,
    primitive_id: &'static str,
    original_prefix_len: usize,
    original_prefix_sha256: [u8; 32],
    hook_spec_sha256: [u8; 32],
    observed_relay_address: usize,
    observed_detour_address: usize,
    authenticity_seal: u64,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReleasedHookBinding {
    active: ActiveHookBinding,
    receipt: ReleasedRvaHookReceipt,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
enum HookCustodyState {
    Active(ActiveHookBinding),
    Released(ReleasedHookBinding),
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct ActivePe64ImportSlotBinding {
    receipt: InstalledPe64ImportAddressSlotReceipt,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalPe64ImportSlotBinding {
    installed: InstalledPe64ImportAddressSlotReceipt,
    receipt: RestoredPe64ImportAddressSlotCustodyReceipt,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
enum Pe64ImportSlotCustodyState {
    Active(ActivePe64ImportSlotBinding),
    Terminal(TerminalPe64ImportSlotBinding),
}

pub fn next_teardown_owner_token() -> Result<u64, String> {
    loop {
        let token = NEXT_OWNER_TOKEN.load(Ordering::Acquire);
        if token == 0 || token == u64::MAX {
            return Err("production hook teardown owner token space exhausted".into());
        }
        if NEXT_OWNER_TOKEN
            .compare_exchange_weak(token, token + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(token);
        }
    }
}

/// Install one exact PE64 import-address-slot redirection with a naturally
/// aligned atomic pointer compare/exchange. This function never changes code
/// bytes and never suspends a target thread.
#[cfg(windows)]
pub fn install_pe64_import_address_slot(
    expected: ExpectedPe64ImportAddressSlot,
    external_shim: ExternalDispatchShimIdentity,
    teardown_owner_token: u64,
) -> Result<InstalledPe64ImportAddressSlotReceipt, String> {
    validate_expected_pe64_import_slot(expected)?;
    if teardown_owner_token == 0 {
        return Err(format!(
            "{} teardown owner token is zero",
            expected.logical_name
        ));
    }
    validate_external_dispatch_shim_identity(external_shim)?;
    let external_shim_address = external_shim.entry_address;
    if external_shim_address == expected.expected_original_address {
        return Err(format!(
            "{} external shim address is invalid",
            expected.logical_name
        ));
    }
    let module = observe_main_module()?;
    let slot_address = validate_pe64_import_slot(module, expected)?;
    validate_atomic_pointer_slot_memory(slot_address)?;
    let key = (teardown_owner_token, expected.slot_rva);
    let registry = PE64_IMPORT_SLOT_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| "PE64 import-slot custody registry is poisoned")?;
    if registry.iter().any(|(existing_key, state)| {
        if *existing_key == key {
            return false;
        }
        let receipt = match state {
            Pe64ImportSlotCustodyState::Active(binding) => &binding.receipt,
            Pe64ImportSlotCustodyState::Terminal(binding) => &binding.installed,
        };
        receipt.observed_module_sha256 == module.sha256
            && receipt.observed_pe_identity_sha256 == module.pe_identity.sha256
            && receipt.slot_address == slot_address
    }) {
        return Err(format!(
            "{} import slot already has a different active or terminal lease",
            expected.logical_name
        ));
    }
    match registry.get(&key) {
        Some(Pe64ImportSlotCustodyState::Active(binding)) => {
            validate_installed_pe64_import_slot_binding(expected, &binding.receipt)?;
            if binding.receipt.external_shim_address != external_shim_address
                || binding.receipt.external_shim != external_shim
            {
                return Err(format!(
                    "{} install retry changed external shim identity",
                    expected.logical_name
                ));
            }
            validate_external_dispatch_shim_identity(binding.receipt.external_shim)?;
            let observed = atomic_load_pointer_slot(slot_address)?;
            if observed != external_shim_address {
                return Err(format!(
                    "{} active import slot changed before install retry",
                    expected.logical_name
                ));
            }
            return Ok(binding.receipt.clone());
        }
        Some(Pe64ImportSlotCustodyState::Terminal(_)) => {
            return Err(format!(
                "{} import-slot lease is irreversible terminal custody",
                expected.logical_name
            ));
        }
        None => {}
    }
    let slot_spec_sha256 = pe64_import_slot_spec_sha256(expected);
    let authenticity_seal = next_receipt_seal()?;
    let cas = atomic_compare_exchange_pointer_slot(
        slot_address,
        expected.expected_original_address,
        external_shim_address,
    )?;
    let observed_before = cas.observed_before;
    if observed_before != expected.expected_original_address {
        return Err(format!(
            "{} import slot CAS expected 0x{:X}, observed 0x{observed_before:X}",
            expected.logical_name, expected.expected_original_address
        ));
    }
    let observed_after = atomic_load_pointer_slot(slot_address)?;
    if observed_after != external_shim_address {
        return Err(format!(
            "{} import slot did not retain the external shim after CAS",
            expected.logical_name
        ));
    }
    let receipt = InstalledPe64ImportAddressSlotReceipt {
        logical_name: expected.logical_name,
        observed_module_name: BATTLE_PLAN_MODULE_NAME,
        observed_module_sha256: module.sha256,
        observed_pe_identity_sha256: module.pe_identity.sha256,
        observed_pe_timestamp: module.pe_identity.timestamp,
        observed_size_of_image: module.pe_identity.size_of_image,
        imported_dll: expected.imported_dll,
        imported_symbol: expected.imported_symbol,
        slot_rva: expected.slot_rva,
        slot_address,
        slot_width: usize::BITS as u16 / 8,
        slot_alignment: std::mem::align_of::<usize>() as u16,
        expected_original_address: expected.expected_original_address,
        external_shim_address,
        external_shim,
        observed_before_address: observed_before,
        observed_after_address: observed_after,
        slot_protection_before: cas.protection_before,
        slot_protection_temporary: cas.protection_temporary,
        slot_protection_after: cas.protection_after,
        primitive_id: PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS_PRIMITIVE_ID,
        slot_spec_sha256,
        teardown_owner_token,
        installed: true,
        authenticity_seal,
    };
    registry.insert(
        key,
        Pe64ImportSlotCustodyState::Active(ActivePe64ImportSlotBinding {
            receipt: receipt.clone(),
        }),
    );
    Ok(receipt)
}

/// Restore one installed PE64 import slot with an atomic pointer CAS after the
/// external shim has switched to the original export and all DLL callbacks are
/// quiescent. The shim allocation remains process-lifetime custody because a
/// target thread may already have loaded its old pointer from the import slot.
#[cfg(windows)]
pub fn restore_pe64_import_address_slot_process_lifetime(
    expected: ExpectedPe64ImportAddressSlot,
    installed: &InstalledPe64ImportAddressSlotReceipt,
    attestation: Pe64ImportAddressSlotTerminalAttestation,
) -> Result<RestoredPe64ImportAddressSlotCustodyReceipt, String> {
    validate_expected_pe64_import_slot(expected)?;
    validate_installed_pe64_import_slot_binding(expected, installed)?;
    let module = observe_main_module()?;
    let slot_address = validate_pe64_import_slot(module, expected)?;
    if slot_address != installed.slot_address {
        return Err(format!(
            "{} import slot address changed",
            expected.logical_name
        ));
    }
    validate_atomic_pointer_slot_memory(slot_address)?;
    let key = (installed.teardown_owner_token, installed.slot_rva);
    let registry = PE64_IMPORT_SLOT_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| "PE64 import-slot custody registry is poisoned")?;
    let state = registry.get(&key).ok_or_else(|| {
        format!(
            "{} has no crate-owned import-slot custody",
            expected.logical_name
        )
    })?;
    let active = match state {
        Pe64ImportSlotCustodyState::Active(binding) => &binding.receipt,
        Pe64ImportSlotCustodyState::Terminal(binding) => {
            if &binding.installed != installed {
                return Err(format!(
                    "{} terminal retry changed installed receipt",
                    expected.logical_name
                ));
            }
            validate_terminal_pe64_import_slot_live(&binding.receipt)?;
            return Ok(binding.receipt.clone());
        }
    };
    if active != installed {
        return Err(format!(
            "{} installed receipt is not the crate-owned import-slot lease",
            expected.logical_name
        ));
    }
    let observed_terminal = observe_external_dispatch_terminal(
        installed.expected_original_address,
        installed.external_shim_address,
        attestation,
    )?;
    if attestation.external_shim != installed.external_shim {
        return Err(format!(
            "{} terminal attestation changed external shim identity",
            expected.logical_name
        ));
    }
    let authenticity_seal = next_receipt_seal()? ^ installed.authenticity_seal;
    if authenticity_seal == 0 {
        return Err(format!(
            "{} terminal authenticity seal is zero",
            expected.logical_name
        ));
    }
    let cas = atomic_compare_exchange_pointer_slot(
        slot_address,
        installed.external_shim_address,
        installed.expected_original_address,
    )?;
    let observed_before = cas.observed_before;
    if observed_before != installed.external_shim_address {
        return Err(format!(
            "{} restore CAS expected shim 0x{:X}, observed 0x{observed_before:X}",
            expected.logical_name, installed.external_shim_address
        ));
    }
    let observed_after = atomic_load_pointer_slot(slot_address)?;
    if observed_after != installed.expected_original_address {
        return Err(format!(
            "{} import slot did not retain the original export after restore",
            expected.logical_name
        ));
    }
    let mut receipt = RestoredPe64ImportAddressSlotCustodyReceipt {
        logical_name: installed.logical_name,
        observed_module_name: installed.observed_module_name,
        observed_module_sha256: installed.observed_module_sha256,
        observed_pe_identity_sha256: installed.observed_pe_identity_sha256,
        observed_pe_timestamp: installed.observed_pe_timestamp,
        observed_size_of_image: installed.observed_size_of_image,
        imported_dll: installed.imported_dll,
        imported_symbol: installed.imported_symbol,
        slot_rva: installed.slot_rva,
        slot_address: installed.slot_address,
        slot_width: installed.slot_width,
        slot_alignment: installed.slot_alignment,
        expected_original_address: installed.expected_original_address,
        external_shim_address: installed.external_shim_address,
        observed_restore_before_address: observed_before,
        observed_restore_after_address: observed_after,
        slot_protection_before_restore: cas.protection_before,
        slot_protection_temporary_restore: cas.protection_temporary,
        slot_protection_after_restore: cas.protection_after,
        primitive_id: installed.primitive_id,
        slot_spec_sha256: installed.slot_spec_sha256,
        teardown_owner_token: installed.teardown_owner_token,
        callbacks_quiesced: attestation.callbacks_quiesced,
        slot_restored: true,
        retained_until_process_exit: attestation.retained_until_process_exit,
        policy_id: attestation.policy_id,
        external_shim: attestation.external_shim,
        dispatch_slot_sha256: observed_terminal.dispatch_slot_sha256,
        observed_terminal_dispatch_address: observed_terminal.dispatch_address,
        active_callback_counter_sha256: observed_terminal.active_counter_sha256,
        observed_active_callback_count: observed_terminal.active_count,
        custody_digest_sha256: [0; 32],
        authenticity_seal,
    };
    receipt.custody_digest_sha256 = restored_pe64_import_slot_custody_digest(&receipt);
    registry.insert(
        key,
        Pe64ImportSlotCustodyState::Terminal(TerminalPe64ImportSlotBinding {
            installed: installed.clone(),
            receipt: receipt.clone(),
        }),
    );
    Ok(receipt)
}

#[cfg(windows)]
pub fn verify_restored_pe64_import_address_slot_custody(
    installed: &InstalledPe64ImportAddressSlotReceipt,
    receipt: &RestoredPe64ImportAddressSlotCustodyReceipt,
) -> Result<(), String> {
    validate_terminal_receipt_binding(installed, receipt)?;
    if restored_pe64_import_slot_custody_digest(receipt) != receipt.custody_digest_sha256 {
        return Err(format!(
            "{} terminal import-slot custody digest mismatch",
            receipt.logical_name
        ));
    }
    let registry = PE64_IMPORT_SLOT_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let registry = registry
        .lock()
        .map_err(|_| "PE64 import-slot custody registry is poisoned")?;
    let key = (installed.teardown_owner_token, installed.slot_rva);
    match registry.get(&key) {
        Some(Pe64ImportSlotCustodyState::Terminal(binding))
            if binding.installed == *installed && binding.receipt == *receipt => {}
        _ => {
            return Err(format!(
                "{} is not the crate-owned terminal import-slot custody",
                receipt.logical_name
            ));
        }
    }
    validate_terminal_pe64_import_slot_live(receipt)
}

pub fn restored_pe64_import_slot_custody_digest(
    receipt: &RestoredPe64ImportAddressSlotCustodyReceipt,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.pe64_import_slot_terminal.v1");
    digest_len_prefixed(&mut digest, receipt.logical_name.as_bytes());
    digest.update(receipt.observed_module_sha256);
    digest.update(receipt.observed_pe_identity_sha256);
    digest.update(receipt.observed_pe_timestamp.to_le_bytes());
    digest.update(receipt.observed_size_of_image.to_le_bytes());
    digest_len_prefixed(&mut digest, receipt.imported_dll.as_bytes());
    digest_len_prefixed(&mut digest, receipt.imported_symbol.as_bytes());
    digest.update(receipt.slot_rva.to_le_bytes());
    digest.update((receipt.slot_address as u64).to_le_bytes());
    digest.update(receipt.slot_width.to_le_bytes());
    digest.update(receipt.slot_alignment.to_le_bytes());
    digest.update((receipt.expected_original_address as u64).to_le_bytes());
    digest.update((receipt.external_shim_address as u64).to_le_bytes());
    digest.update((receipt.observed_restore_before_address as u64).to_le_bytes());
    digest.update((receipt.observed_restore_after_address as u64).to_le_bytes());
    digest.update(receipt.slot_protection_before_restore.to_le_bytes());
    digest.update(receipt.slot_protection_temporary_restore.to_le_bytes());
    digest.update(receipt.slot_protection_after_restore.to_le_bytes());
    digest_len_prefixed(&mut digest, receipt.primitive_id.as_bytes());
    digest.update(receipt.slot_spec_sha256);
    digest.update(receipt.teardown_owner_token.to_le_bytes());
    digest.update([receipt.callbacks_quiesced as u8]);
    digest.update([receipt.slot_restored as u8]);
    digest.update([receipt.retained_until_process_exit as u8]);
    digest_len_prefixed(&mut digest, receipt.policy_id.as_bytes());
    digest_external_shim_identity(&mut digest, receipt.external_shim);
    digest.update(receipt.dispatch_slot_sha256);
    digest.update((receipt.observed_terminal_dispatch_address as u64).to_le_bytes());
    digest.update(receipt.active_callback_counter_sha256);
    digest.update(receipt.observed_active_callback_count.to_le_bytes());
    digest.update(receipt.authenticity_seal.to_le_bytes());
    digest.finalize().into()
}

/// Observe one enabled MinHook 0.9.0 normal five-byte JMP rel32 target.
///
/// `patchAbove` starts with `EB F9` at the target and is explicitly rejected.
#[cfg(windows)]
pub fn observe_enabled_minhook_rel5(
    expected: ExpectedRvaHook,
    target: usize,
    expected_detour: usize,
    teardown_owner_token: u64,
) -> Result<InstalledRvaHookReceipt, String> {
    validate_expected(expected, MINHOOK_NORMAL_REL5_PRIMITIVE_ID, 5)?;
    if expected_detour == 0 {
        return Err(format!(
            "{} expected detour address is zero",
            expected.logical_name
        ));
    }
    let module = observe_main_module()?;
    let observed_rva = validate_exact_target(module, expected, target)?;
    let patched = read_exact_five_byte_patch(module, expected, target)?;
    if patched.starts_with(&[0xeb, 0xf9]) {
        return Err(format!(
            "{} used forbidden MinHook patchAbove/split placement",
            expected.logical_name
        ));
    }
    if patched.first().copied() != Some(0xe9) {
        return Err(format!(
            "{} target does not contain an enabled normal JMP rel32 patch",
            expected.logical_name
        ));
    }
    let displacement = i32::from_le_bytes(
        patched[1..5]
            .try_into()
            .map_err(|_| "MinHook rel32 displacement is truncated")?,
    );
    let relay = target
        .checked_add(5)
        .and_then(|next| next.checked_add_signed(displacement as isize))
        .ok_or("MinHook relay destination overflow")?;
    let relay_stub = read_process_bytes(relay, 14)?;
    if relay_stub[..6] != [0xff, 0x25, 0x00, 0x00, 0x00, 0x00] {
        return Err(format!(
            "{} relay is not the exact FF25 absolute-indirect stub",
            expected.logical_name
        ));
    }
    let observed_detour_u64 = u64::from_le_bytes(
        relay_stub[6..14]
            .try_into()
            .map_err(|_| "MinHook relay absolute destination is truncated")?,
    );
    let observed_detour = usize::try_from(observed_detour_u64)
        .map_err(|_| "MinHook relay absolute destination does not fit usize")?;
    if observed_detour != expected_detour {
        return Err(format!(
            "{} relay destination mismatch: expected 0x{expected_detour:X}, observed 0x{observed_detour:X}",
            expected.logical_name
        ));
    }
    observe_common(
        module,
        expected,
        target,
        observed_rva,
        relay,
        observed_detour,
        teardown_owner_token,
    )
}

/// Observe D90's enabled five-byte CALL rel32 callsite patch and bind its
/// decoded destination to the installed detour.
#[cfg(windows)]
pub fn observe_enabled_rel32_callsite(
    expected: ExpectedRvaHook,
    target: usize,
    expected_detour: usize,
    teardown_owner_token: u64,
) -> Result<InstalledRvaHookReceipt, String> {
    validate_expected(expected, D90_REL32_CALLSITE_PRIMITIVE_ID, 5)?;
    if expected_detour == 0 {
        return Err(format!(
            "{} expected detour address is zero",
            expected.logical_name
        ));
    }
    let module = observe_main_module()?;
    let observed_rva = validate_exact_target(module, expected, target)?;
    let patched = read_exact_five_byte_patch(module, expected, target)?;
    if patched.first().copied() != Some(0xe8) {
        return Err(format!(
            "{} target does not contain an enabled CALL rel32 patch",
            expected.logical_name
        ));
    }
    let displacement = i32::from_le_bytes(
        patched[1..5]
            .try_into()
            .map_err(|_| "D90 rel32 displacement is truncated")?,
    );
    let observed_detour = target
        .checked_add(5)
        .and_then(|next| next.checked_add_signed(displacement as isize))
        .ok_or("D90 rel32 destination overflow")?;
    if observed_detour != expected_detour {
        return Err(format!(
            "{} rel32 destination mismatch: expected 0x{expected_detour:X}, observed 0x{observed_detour:X}",
            expected.logical_name
        ));
    }
    observe_common(
        module,
        expected,
        target,
        observed_rva,
        0,
        observed_detour,
        teardown_owner_token,
    )
}

/// Mint a release receipt only after the exact original target prefix has
/// returned and the receipt still identifies this exact a729 target.
#[cfg(windows)]
pub fn observe_released(
    expected: ExpectedRvaHook,
    installed: InstalledRvaHookReceipt,
    attestation: HookTeardownAttestation,
) -> Result<ReleasedRvaHookReceipt, String> {
    let [released] = observe_released_set([expected], [installed], [attestation])?;
    Ok(released)
}

/// Atomically transition a set of restored hooks from active custody to
/// released custody. A retry with the exact same opaque receipts returns the
/// already-minted receipts; it never mints a second release or reactivates a
/// consumed lease.
#[cfg(windows)]
pub fn observe_released_set<const N: usize>(
    expected: [ExpectedRvaHook; N],
    installed: [InstalledRvaHookReceipt; N],
    attestations: [HookTeardownAttestation; N],
) -> Result<[ReleasedRvaHookReceipt; N], String> {
    if N == 0 {
        return Err("production hook release set is empty".into());
    }
    let module = observe_main_module()?;
    let custody = HOOK_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut registry = custody
        .lock()
        .map_err(|_| "production hook custody registry is poisoned")?;
    let mut seen = BTreeSet::new();
    let mut planned = Vec::with_capacity(N);

    for index in 0..N {
        let expected = expected[index];
        let installed = installed[index];
        let attestation = attestations[index];
        if !attestation.callbacks_quiesced || !attestation.hook_removed || !attestation.relay_freed
        {
            return Err(format!(
                "{} teardown attestation is incomplete",
                expected.logical_name
            ));
        }
        validate_known_expected(expected)?;
        if !installed.enabled
            || installed.patch_above
            || installed.placement != HookPatchPlacement::NormalTarget
            || installed.logical_name != expected.logical_name
            || installed.requested_rva != expected.requested_rva
            || installed.observed_rva != expected.requested_rva
            || installed.target_patch_len != expected.target_patch_len
            || installed.primitive_id != expected.primitive_id
            || installed.teardown_owner_token == 0
        {
            return Err(format!(
                "{} teardown does not own its exact installed receipt",
                expected.logical_name
            ));
        }
        let key = (installed.teardown_owner_token, installed.requested_rva);
        if !seen.insert(key) {
            return Err(format!(
                "{} release set repeats one owner-token/RVA lease",
                expected.logical_name
            ));
        }
        let expected_prefix_sha256: [u8; 32] = Sha256::digest(expected.original_prefix).into();
        let expected_spec_sha256 = hook_spec_sha256(expected);
        let state = registry
            .get(&key)
            .ok_or_else(|| format!("{} has no crate-owned hook custody", expected.logical_name))?;
        let active = match state {
            HookCustodyState::Active(active) => *active,
            HookCustodyState::Released(released) => released.active,
        };
        validate_active_binding(
            active,
            module,
            expected,
            installed,
            expected_spec_sha256,
            expected_prefix_sha256,
        )?;
        validate_exact_target(module, expected, installed.observed_runtime_address)?;
        let restored = read_process_bytes(
            installed.observed_runtime_address,
            expected.original_prefix.len(),
        )?;
        if restored != expected.original_prefix {
            return Err(format!(
                "{} original target prefix was not restored",
                expected.logical_name
            ));
        }
        let released = match state {
            HookCustodyState::Released(released) => released.receipt,
            HookCustodyState::Active(_) => {
                let release_seal = next_receipt_seal()?;
                ReleasedRvaHookReceipt {
                    logical_name: installed.logical_name,
                    observed_module_name: installed.observed_module_name,
                    observed_module_sha256: installed.observed_module_sha256,
                    observed_pe_identity_sha256: installed.observed_pe_identity_sha256,
                    observed_pe_timestamp: installed.observed_pe_timestamp,
                    observed_size_of_image: installed.observed_size_of_image,
                    observed_rva: installed.observed_rva,
                    observed_runtime_address: installed.observed_runtime_address,
                    target_patch_len: installed.target_patch_len,
                    primitive_id: installed.primitive_id,
                    hook_spec_sha256: installed.hook_spec_sha256,
                    original_prefix_len: installed.original_prefix_len,
                    original_prefix_sha256: installed.original_prefix_sha256,
                    observed_relay_address: installed.observed_relay_address,
                    observed_detour_address: installed.observed_detour_address,
                    teardown_owner_token: installed.teardown_owner_token,
                    restored_prefix_sha256: Sha256::digest(restored).into(),
                    callbacks_quiesced: attestation.callbacks_quiesced,
                    hook_removed: attestation.hook_removed,
                    relay_freed: attestation.relay_freed,
                    released: true,
                    authenticity_seal: release_seal ^ active.authenticity_seal,
                }
            }
        };
        planned.push((key, active, released));
    }

    let receipts: [ReleasedRvaHookReceipt; N] = planned
        .iter()
        .map(|(_, _, receipt)| *receipt)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| "production hook release set cardinality changed")?;
    for (key, active, receipt) in planned {
        registry.insert(
            key,
            HookCustodyState::Released(ReleasedHookBinding { active, receipt }),
        );
    }
    Ok(receipts)
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObservedExternalDispatchTerminal {
    dispatch_slot_sha256: [u8; 32],
    dispatch_address: usize,
    active_counter_sha256: [u8; 32],
    active_count: u64,
}

fn validate_expected_pe64_import_slot(
    expected: ExpectedPe64ImportAddressSlot,
) -> Result<(), String> {
    if expected.logical_name.is_empty()
        || expected.slot_rva == 0
        || expected.imported_dll.is_empty()
        || expected.imported_symbol.is_empty()
        || !expected.imported_dll.is_ascii()
        || !expected.imported_symbol.is_ascii()
        || expected.expected_original_address == 0
    {
        return Err("PE64 import-slot expected specification is not admitted".into());
    }
    Ok(())
}

fn pe64_import_slot_spec_sha256(expected: ExpectedPe64ImportAddressSlot) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.pe64_import_slot_spec.v1");
    digest_len_prefixed(&mut digest, expected.logical_name.as_bytes());
    digest.update(expected.slot_rva.to_le_bytes());
    digest_len_prefixed(&mut digest, expected.imported_dll.as_bytes());
    digest_len_prefixed(&mut digest, expected.imported_symbol.as_bytes());
    digest.update((expected.expected_original_address as u64).to_le_bytes());
    digest.update(A729_EXE_SHA256);
    digest.finalize().into()
}

fn digest_len_prefixed(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

fn digest_external_shim_identity(digest: &mut Sha256, identity: ExternalDispatchShimIdentity) {
    digest.update((identity.entry_address as u64).to_le_bytes());
    digest.update((identity.entry_len as u64).to_le_bytes());
    digest.update(identity.entry_bytes_sha256);
    digest.update((identity.allocation_base as u64).to_le_bytes());
    digest.update((identity.allocation_len as u64).to_le_bytes());
    digest.update((identity.dispatch_slot_address as u64).to_le_bytes());
    digest.update((identity.active_callback_counter_address as u64).to_le_bytes());
}

#[cfg(windows)]
fn validate_installed_pe64_import_slot_binding(
    expected: ExpectedPe64ImportAddressSlot,
    receipt: &InstalledPe64ImportAddressSlotReceipt,
) -> Result<(), String> {
    if receipt.logical_name != expected.logical_name
        || receipt.observed_module_name != BATTLE_PLAN_MODULE_NAME
        || receipt.observed_module_sha256 != A729_EXE_SHA256
        || receipt.imported_dll != expected.imported_dll
        || receipt.imported_symbol != expected.imported_symbol
        || receipt.slot_rva != expected.slot_rva
        || receipt.slot_address == 0
        || receipt.slot_width != usize::BITS as u16 / 8
        || receipt.slot_alignment != std::mem::align_of::<usize>() as u16
        || receipt.slot_address % std::mem::align_of::<usize>() != 0
        || receipt.expected_original_address != expected.expected_original_address
        || receipt.external_shim_address == 0
        || receipt.external_shim.entry_address != receipt.external_shim_address
        || receipt.observed_before_address != expected.expected_original_address
        || receipt.observed_after_address != receipt.external_shim_address
        || receipt.slot_protection_before == 0
        || receipt.slot_protection_temporary == 0
        || receipt.slot_protection_after != receipt.slot_protection_before
        || receipt.primitive_id != PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS_PRIMITIVE_ID
        || receipt.slot_spec_sha256 != pe64_import_slot_spec_sha256(expected)
        || receipt.teardown_owner_token == 0
        || !receipt.installed
        || receipt.authenticity_seal == 0
    {
        return Err(format!(
            "{} installed PE64 import-slot receipt is invalid",
            expected.logical_name
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn validate_terminal_receipt_binding(
    installed: &InstalledPe64ImportAddressSlotReceipt,
    receipt: &RestoredPe64ImportAddressSlotCustodyReceipt,
) -> Result<(), String> {
    if receipt.logical_name != installed.logical_name
        || receipt.observed_module_name != installed.observed_module_name
        || receipt.observed_module_sha256 != installed.observed_module_sha256
        || receipt.observed_pe_identity_sha256 != installed.observed_pe_identity_sha256
        || receipt.observed_pe_timestamp != installed.observed_pe_timestamp
        || receipt.observed_size_of_image != installed.observed_size_of_image
        || receipt.imported_dll != installed.imported_dll
        || receipt.imported_symbol != installed.imported_symbol
        || receipt.slot_rva != installed.slot_rva
        || receipt.slot_address != installed.slot_address
        || receipt.slot_width != installed.slot_width
        || receipt.slot_alignment != installed.slot_alignment
        || receipt.expected_original_address != installed.expected_original_address
        || receipt.external_shim_address != installed.external_shim_address
        || receipt.external_shim != installed.external_shim
        || receipt.observed_restore_before_address != installed.external_shim_address
        || receipt.observed_restore_after_address != installed.expected_original_address
        || receipt.slot_protection_before_restore == 0
        || receipt.slot_protection_temporary_restore == 0
        || receipt.slot_protection_after_restore != receipt.slot_protection_before_restore
        || receipt.primitive_id != installed.primitive_id
        || receipt.slot_spec_sha256 != installed.slot_spec_sha256
        || receipt.teardown_owner_token != installed.teardown_owner_token
        || !receipt.callbacks_quiesced
        || !receipt.slot_restored
        || !receipt.retained_until_process_exit
        || receipt.policy_id != PE64_IMPORT_ADDRESS_SLOT_PROCESS_LIFETIME_POLICY_ID
        || receipt.external_shim.entry_address != installed.external_shim_address
        || receipt.observed_terminal_dispatch_address != installed.expected_original_address
        || receipt.observed_active_callback_count != 0
        || receipt.authenticity_seal == 0
    {
        return Err(format!(
            "{} terminal PE64 import-slot receipt is invalid",
            receipt.logical_name
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn observe_external_dispatch_terminal(
    expected_original_address: usize,
    external_shim_address: usize,
    attestation: Pe64ImportAddressSlotTerminalAttestation,
) -> Result<ObservedExternalDispatchTerminal, String> {
    if !attestation.callbacks_quiesced
        || !attestation.slot_dispatches_to_original
        || !attestation.retained_until_process_exit
        || attestation.policy_id != PE64_IMPORT_ADDRESS_SLOT_PROCESS_LIFETIME_POLICY_ID
        || attestation.external_shim.entry_address != external_shim_address
    {
        return Err("PE64 import-slot terminal attestation is incomplete".into());
    }
    validate_external_dispatch_shim_identity(attestation.external_shim)?;
    let dispatch_bytes = read_process_bytes(
        attestation.external_shim.dispatch_slot_address,
        size_of::<usize>(),
    )?;
    let dispatch_address = usize::from_le_bytes(
        dispatch_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "external dispatch slot width changed")?,
    );
    if dispatch_address != expected_original_address {
        return Err(format!(
            "external dispatch slot targets 0x{dispatch_address:X}, not original 0x{expected_original_address:X}"
        ));
    }
    let counter_bytes = read_process_bytes(
        attestation.external_shim.active_callback_counter_address,
        size_of::<u64>(),
    )?;
    let active_count = u64::from_le_bytes(
        counter_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "external active callback counter width changed")?,
    );
    if active_count != 0 {
        return Err(format!(
            "external dispatch shim still has {active_count} active callbacks"
        ));
    }
    Ok(ObservedExternalDispatchTerminal {
        dispatch_slot_sha256: Sha256::digest(dispatch_bytes).into(),
        dispatch_address,
        active_counter_sha256: Sha256::digest(counter_bytes).into(),
        active_count,
    })
}

#[cfg(windows)]
fn validate_terminal_pe64_import_slot_live(
    receipt: &RestoredPe64ImportAddressSlotCustodyReceipt,
) -> Result<(), String> {
    if atomic_load_pointer_slot(receipt.slot_address)? != receipt.expected_original_address {
        return Err(format!(
            "{} restored import slot changed",
            receipt.logical_name
        ));
    }
    let observed = observe_external_dispatch_terminal(
        receipt.expected_original_address,
        receipt.external_shim_address,
        Pe64ImportAddressSlotTerminalAttestation {
            callbacks_quiesced: receipt.callbacks_quiesced,
            slot_dispatches_to_original: receipt.slot_restored,
            retained_until_process_exit: receipt.retained_until_process_exit,
            policy_id: receipt.policy_id,
            external_shim: receipt.external_shim,
        },
    )?;
    if observed.dispatch_slot_sha256 != receipt.dispatch_slot_sha256
        || observed.dispatch_address != receipt.observed_terminal_dispatch_address
        || observed.active_counter_sha256 != receipt.active_callback_counter_sha256
        || observed.active_count != receipt.observed_active_callback_count
    {
        return Err(format!(
            "{} external dispatch terminal identity changed",
            receipt.logical_name
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn validate_active_binding(
    active: ActiveHookBinding,
    module: &ObservedMainModule,
    expected: ExpectedRvaHook,
    installed: InstalledRvaHookReceipt,
    expected_spec_sha256: [u8; 32],
    expected_prefix_sha256: [u8; 32],
) -> Result<(), String> {
    if installed.observed_module_name != BATTLE_PLAN_MODULE_NAME
        || installed.observed_module_sha256 != A729_EXE_SHA256
        || module.sha256 != installed.observed_module_sha256
        || module.pe_identity.sha256 != installed.observed_pe_identity_sha256
        || module.pe_identity.timestamp != installed.observed_pe_timestamp
        || module.pe_identity.size_of_image != installed.observed_size_of_image
        || active.logical_name != expected.logical_name
        || active.observed_module_sha256 != installed.observed_module_sha256
        || active.observed_pe_identity_sha256 != installed.observed_pe_identity_sha256
        || active.requested_rva != expected.requested_rva
        || active.observed_runtime_address != installed.observed_runtime_address
        || active.target_patch_len != expected.target_patch_len
        || active.primitive_id != expected.primitive_id
        || active.original_prefix_len != expected.original_prefix.len()
        || active.original_prefix_sha256 != expected_prefix_sha256
        || active.hook_spec_sha256 != expected_spec_sha256
        || active.observed_relay_address != installed.observed_relay_address
        || active.observed_detour_address != installed.observed_detour_address
        || installed.hook_spec_sha256 != expected_spec_sha256
        || usize::from(installed.original_prefix_len) != expected.original_prefix.len()
        || installed.original_prefix_sha256 != expected_prefix_sha256
        || active.authenticity_seal == 0
        || active.authenticity_seal != installed.authenticity_seal
    {
        return Err(format!(
            "{} installed receipt does not match crate-owned exact-build custody",
            expected.logical_name
        ));
    }
    Ok(())
}

fn validate_expected(
    expected: ExpectedRvaHook,
    admitted_primitive: &str,
    admitted_patch_len: u16,
) -> Result<(), String> {
    if expected.logical_name.is_empty()
        || expected.requested_rva == 0
        || expected.target_patch_len != admitted_patch_len
        || expected.primitive_id != admitted_primitive
        || expected.original_prefix.len() < admitted_patch_len as usize
        || expected.original_prefix.len() > 64
        || expected.original_prefix.len() > u16::MAX as usize
    {
        return Err("production hook expected specification is not admitted".into());
    }
    Ok(())
}

fn validate_known_expected(expected: ExpectedRvaHook) -> Result<(), String> {
    match expected.primitive_id {
        MINHOOK_NORMAL_REL5_PRIMITIVE_ID => {
            validate_expected(expected, MINHOOK_NORMAL_REL5_PRIMITIVE_ID, 5)
        }
        D90_REL32_CALLSITE_PRIMITIVE_ID => {
            validate_expected(expected, D90_REL32_CALLSITE_PRIMITIVE_ID, 5)
        }
        _ => Err("production hook expected primitive is not admitted".into()),
    }
}

fn hook_spec_sha256(expected: ExpectedRvaHook) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.expected_rva_spec.v1");
    digest.update((expected.logical_name.len() as u64).to_le_bytes());
    digest.update(expected.logical_name.as_bytes());
    digest.update(expected.requested_rva.to_le_bytes());
    digest.update(expected.target_patch_len.to_le_bytes());
    digest.update((expected.primitive_id.len() as u64).to_le_bytes());
    digest.update(expected.primitive_id.as_bytes());
    digest.update((expected.original_prefix.len() as u64).to_le_bytes());
    digest.update(expected.original_prefix);
    digest.update(BATTLE_PLAN_MODULE_NAME.as_bytes());
    digest.update(A729_EXE_SHA256);
    digest.finalize().into()
}

#[cfg(windows)]
fn observe_common(
    module: &ObservedMainModule,
    expected: ExpectedRvaHook,
    target: usize,
    observed_rva: u64,
    observed_relay_address: usize,
    observed_detour_address: usize,
    teardown_owner_token: u64,
) -> Result<InstalledRvaHookReceipt, String> {
    if teardown_owner_token == 0 {
        return Err(format!(
            "{} teardown owner token is zero",
            expected.logical_name
        ));
    }
    if observed_rva != expected.requested_rva || module.sha256 != A729_EXE_SHA256 {
        return Err(format!(
            "{} executable hash or RVA differs from exact a729 authority",
            expected.logical_name
        ));
    }
    let key = (teardown_owner_token, expected.requested_rva);
    let original_prefix_sha256: [u8; 32] = Sha256::digest(expected.original_prefix).into();
    let hook_spec_sha256 = hook_spec_sha256(expected);
    let registry = HOOK_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| "production hook active registry is poisoned")?;
    let candidate = ActiveHookBinding {
        logical_name: expected.logical_name,
        observed_module_sha256: module.sha256,
        observed_pe_identity_sha256: module.pe_identity.sha256,
        requested_rva: expected.requested_rva,
        observed_runtime_address: target,
        target_patch_len: expected.target_patch_len,
        primitive_id: expected.primitive_id,
        original_prefix_len: expected.original_prefix.len(),
        original_prefix_sha256,
        hook_spec_sha256,
        observed_relay_address,
        observed_detour_address,
        authenticity_seal: 0,
    };
    if registry.iter().any(|(existing_key, state)| {
        *existing_key != key
            && matches!(
                state,
                HookCustodyState::Active(active)
                    if active.observed_module_sha256 == candidate.observed_module_sha256
                        && active.observed_pe_identity_sha256
                            == candidate.observed_pe_identity_sha256
                        && active.requested_rva == candidate.requested_rva
                        && active.observed_runtime_address
                            == candidate.observed_runtime_address
            )
    }) {
        return Err(format!(
            "{} physical module target already has a different active lease",
            expected.logical_name
        ));
    }
    let authenticity_seal = match registry.get(&key) {
        Some(HookCustodyState::Active(active)) => {
            let mut comparable = candidate;
            comparable.authenticity_seal = active.authenticity_seal;
            if active != &comparable {
                return Err(format!(
                    "{} owner token and RVA are already bound to a different hook",
                    expected.logical_name
                ));
            }
            active.authenticity_seal
        }
        Some(HookCustodyState::Released(_)) => {
            return Err(format!(
                "{} owner token and RVA lease was already released",
                expected.logical_name
            ));
        }
        None => {
            let authenticity_seal = next_receipt_seal()?;
            let mut active = candidate;
            active.authenticity_seal = authenticity_seal;
            registry.insert(key, HookCustodyState::Active(active));
            authenticity_seal
        }
    };
    Ok(InstalledRvaHookReceipt {
        logical_name: expected.logical_name,
        observed_module_name: BATTLE_PLAN_MODULE_NAME,
        observed_module_sha256: module.sha256,
        observed_pe_identity_sha256: module.pe_identity.sha256,
        observed_pe_timestamp: module.pe_identity.timestamp,
        observed_size_of_image: module.pe_identity.size_of_image,
        requested_rva: expected.requested_rva,
        observed_rva,
        observed_runtime_address: target,
        target_patch_len: expected.target_patch_len,
        primitive_id: expected.primitive_id,
        hook_spec_sha256,
        original_prefix_len: expected.original_prefix.len() as u16,
        original_prefix_sha256,
        observed_relay_address,
        observed_detour_address,
        placement: HookPatchPlacement::NormalTarget,
        patch_above: false,
        enabled: true,
        teardown_owner_token,
        authenticity_seal,
    })
}

fn next_receipt_seal() -> Result<u64, String> {
    loop {
        let seal = NEXT_RECEIPT_SEAL.load(Ordering::Acquire);
        if seal == 0 || seal == u64::MAX {
            return Err("production hook authenticity seal space exhausted".into());
        }
        if NEXT_RECEIPT_SEAL
            .compare_exchange_weak(seal, seal + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(seal);
        }
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct ObservedMainModule {
    base: usize,
    size_of_image: usize,
    sha256: [u8; 32],
    pe_identity: PeIdentity,
    file_bytes: Vec<u8>,
    sections: Vec<PeSection>,
    size_of_headers: usize,
    _stable_file: std::fs::File,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PeIdentity {
    machine: u16,
    number_of_sections: u16,
    timestamp: u32,
    optional_header_size: u16,
    optional_header_magic: u16,
    size_of_image: u32,
    size_of_headers: u32,
    sha256: [u8; 32],
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
struct PeSection {
    virtual_address: usize,
    raw_size: usize,
    raw_offset: usize,
}

#[cfg(windows)]
fn observe_main_module() -> Result<&'static ObservedMainModule, String> {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

    let authority = MAIN_MODULE_AUTHORITY
        .get_or_init(initialize_main_module)
        .as_ref()
        .map_err(Clone::clone)?;
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    if module.is_null() || module as usize != authority.base {
        return Err("production hook main module base identity changed".into());
    }
    let mapped_identity = observe_mapped_pe_identity(authority.base)?;
    if mapped_identity != authority.pe_identity {
        return Err("production hook mapped/file PE identity mismatch".into());
    }
    Ok(authority)
}

#[cfg(windows)]
fn initialize_main_module() -> Result<ObservedMainModule, String> {
    use std::path::Path;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};

    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    if module.is_null() {
        return Err("production hook main module handle is unavailable".into());
    }
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return Err("production hook main module path is unavailable".into());
    }
    buffer.truncate(length as usize);
    let path = String::from_utf16(&buffer)
        .map_err(|_| "production hook main module path is not valid UTF-16")?;
    let file_name = Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("production hook main module filename is unavailable")?;
    if !file_name.eq_ignore_ascii_case(BATTLE_PLAN_MODULE_NAME) {
        return Err(format!(
            "production hook main module is {file_name}, not {BATTLE_PLAN_MODULE_NAME}"
        ));
    }
    let mut stable_file = std::fs::File::open(&path)
        .map_err(|error| format!("production hook main module open failed: {error}"))?;
    let mut bytes = Vec::new();
    stable_file
        .read_to_end(&mut bytes)
        .map_err(|error| format!("production hook main module read failed: {error}"))?;
    let sha256: [u8; 32] = Sha256::digest(&bytes).into();
    if sha256 != A729_EXE_SHA256 {
        return Err("production hook main module file is not exact a729".into());
    }
    let (pe_identity, sections, size_of_headers) = parse_file_pe_layout(&bytes)?;
    let mapped_identity = observe_mapped_pe_identity(module as usize)?;
    if mapped_identity != pe_identity {
        return Err("production hook mapped/file PE identity mismatch".into());
    }
    Ok(ObservedMainModule {
        base: module as usize,
        size_of_image: pe_identity.size_of_image as usize,
        sha256,
        pe_identity,
        file_bytes: bytes,
        sections,
        size_of_headers,
        _stable_file: stable_file,
    })
}

#[cfg(windows)]
fn checked_image_address(
    module: &ObservedMainModule,
    requested_rva: u64,
    observed_len: usize,
) -> Result<usize, String> {
    let requested_rva = usize::try_from(requested_rva)
        .map_err(|_| "production hook requested RVA does not fit usize")?;
    let range_end = requested_rva
        .checked_add(observed_len)
        .ok_or("production hook image range overflow")?;
    if observed_len == 0
        || requested_rva >= module.size_of_image
        || range_end > module.size_of_image
    {
        return Err("production hook requested range is outside the loaded image".into());
    }
    module
        .base
        .checked_add(requested_rva)
        .ok_or_else(|| "production hook image address overflow".into())
}

#[cfg(windows)]
fn validate_exact_target(
    module: &ObservedMainModule,
    expected: ExpectedRvaHook,
    target: usize,
) -> Result<u64, String> {
    let expected_target = checked_image_address(
        module,
        expected.requested_rva,
        expected.original_prefix.len(),
    )?;
    if target != expected_target {
        return Err(format!(
            "{} target mismatch: expected 0x{expected_target:X}, observed 0x{target:X}",
            expected.logical_name
        ));
    }
    let requested_rva = usize::try_from(expected.requested_rva)
        .map_err(|_| "production hook requested RVA does not fit usize")?;
    let file_prefix =
        module_file_bytes_at_rva(module, requested_rva, expected.original_prefix.len())?;
    if file_prefix != expected.original_prefix {
        return Err(format!(
            "{} expected original prefix does not match exact a729 file bytes",
            expected.logical_name
        ));
    }
    target
        .checked_sub(module.base)
        .and_then(|value| u64::try_from(value).ok())
        .filter(|observed_rva| *observed_rva == expected.requested_rva)
        .ok_or_else(|| "production hook observed RVA is invalid".into())
}

#[cfg(windows)]
fn read_exact_five_byte_patch(
    module: &ObservedMainModule,
    expected: ExpectedRvaHook,
    target: usize,
) -> Result<Vec<u8>, String> {
    let requested_rva = usize::try_from(expected.requested_rva)
        .map_err(|_| "production hook requested RVA does not fit usize")?;
    checked_image_address(module, expected.requested_rva, 6)?;
    let file_window = module_file_bytes_at_rva(module, requested_rva, 6)?;
    let observed_window = read_process_bytes(target, 6)?;
    if observed_window[5] != file_window[5] {
        return Err(format!(
            "{} patch changed bytes beyond the admitted five-byte target width",
            expected.logical_name
        ));
    }
    Ok(observed_window[..5].to_vec())
}

#[cfg(windows)]
fn module_file_bytes_at_rva<'a>(
    module: &'a ObservedMainModule,
    rva: usize,
    len: usize,
) -> Result<&'a [u8], String> {
    let end_rva = rva
        .checked_add(len)
        .ok_or("production hook file RVA range overflow")?;
    let file_offset = if end_rva <= module.size_of_headers {
        rva
    } else {
        let section = module
            .sections
            .iter()
            .find(|section| {
                rva >= section.virtual_address
                    && end_rva <= section.virtual_address.saturating_add(section.raw_size)
            })
            .ok_or("production hook RVA has no exact file-backed PE range")?;
        section
            .raw_offset
            .checked_add(rva - section.virtual_address)
            .ok_or("production hook file offset overflow")?
    };
    let file_end = file_offset
        .checked_add(len)
        .ok_or("production hook file range overflow")?;
    module
        .file_bytes
        .get(file_offset..file_end)
        .ok_or_else(|| "production hook file-backed prefix is truncated".into())
}

#[cfg(windows)]
fn validate_pe64_import_slot(
    module: &ObservedMainModule,
    expected: ExpectedPe64ImportAddressSlot,
) -> Result<usize, String> {
    validate_file_pe64_import_slot(module, expected)?;
    let slot_address = checked_image_address(module, expected.slot_rva, size_of::<usize>())?;
    if slot_address % std::mem::align_of::<usize>() != 0 {
        return Err(format!(
            "{} import slot is not naturally pointer-aligned",
            expected.logical_name
        ));
    }
    let mut dll_wide: Vec<u16> = expected.imported_dll.encode_utf16().collect();
    dll_wide.push(0);
    let dll =
        unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(dll_wide.as_ptr()) };
    if dll.is_null() {
        return Err(format!(
            "{} imported DLL {} is not loaded",
            expected.logical_name, expected.imported_dll
        ));
    }
    let symbol = std::ffi::CString::new(expected.imported_symbol)
        .map_err(|_| "PE64 imported symbol contains an interior NUL")?;
    let export = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(dll, symbol.as_ptr().cast())
    }
    .map(|function| function as usize)
    .unwrap_or(0);
    if export == 0 || export != expected.expected_original_address {
        return Err(format!(
            "{} import export pointer mismatch: expected 0x{:X}, observed 0x{export:X}",
            expected.logical_name, expected.expected_original_address
        ));
    }
    Ok(slot_address)
}

#[cfg(windows)]
fn validate_file_pe64_import_slot(
    module: &ObservedMainModule,
    expected: ExpectedPe64ImportAddressSlot,
) -> Result<(), String> {
    let pe_offset = read_u32(&module.file_bytes, 0x3c, "file PE header offset")? as usize;
    let optional = pe_offset
        .checked_add(24)
        .ok_or("PE64 optional-header offset overflow")?;
    let dll_characteristics = read_u16(
        &module.file_bytes,
        optional + 70,
        "PE64 DLL characteristics",
    )?;
    if dll_characteristics & 0x4000 != 0 {
        return Err(
            "PE64 image enables Control Flow Guard; slot target admission is absent".into(),
        );
    }
    let import_directory_rva = read_u32(
        &module.file_bytes,
        optional + 112 + 8,
        "PE64 import directory RVA",
    )? as usize;
    let import_directory_size = read_u32(
        &module.file_bytes,
        optional + 112 + 12,
        "PE64 import directory size",
    )? as usize;
    if import_directory_rva == 0 || import_directory_size < 20 {
        return Err("PE64 import directory is absent".into());
    }
    let descriptor_count = (import_directory_size / 20).min(4096);
    for descriptor_index in 0..descriptor_count {
        let descriptor_rva = import_directory_rva
            .checked_add(descriptor_index * 20)
            .ok_or("PE64 import descriptor RVA overflow")?;
        let descriptor = module_file_bytes_at_rva(module, descriptor_rva, 20)?;
        let original_first_thunk =
            u32::from_le_bytes(descriptor[0..4].try_into().unwrap()) as usize;
        let name_rva = u32::from_le_bytes(descriptor[12..16].try_into().unwrap()) as usize;
        let first_thunk = u32::from_le_bytes(descriptor[16..20].try_into().unwrap()) as usize;
        if original_first_thunk == 0 && name_rva == 0 && first_thunk == 0 {
            break;
        }
        if name_rva == 0 || first_thunk == 0 {
            continue;
        }
        let dll = file_c_string_at_rva(module, name_rva, 260)?;
        if !dll.eq_ignore_ascii_case(expected.imported_dll) {
            continue;
        }
        let lookup_base = if original_first_thunk != 0 {
            original_first_thunk
        } else {
            first_thunk
        };
        for thunk_index in 0..=65_535usize {
            let offset = thunk_index
                .checked_mul(size_of::<u64>())
                .ok_or("PE64 import thunk offset overflow")?;
            let lookup_rva = lookup_base
                .checked_add(offset)
                .ok_or("PE64 import lookup RVA overflow")?;
            let iat_rva = first_thunk
                .checked_add(offset)
                .ok_or("PE64 IAT RVA overflow")?;
            let lookup = u64::from_le_bytes(
                module_file_bytes_at_rva(module, lookup_rva, 8)?
                    .try_into()
                    .unwrap(),
            );
            if lookup == 0 {
                break;
            }
            if iat_rva as u64 != expected.slot_rva {
                continue;
            }
            if lookup & (1u64 << 63) != 0 {
                return Err(format!(
                    "{} exact import slot is ordinal-bound",
                    expected.logical_name
                ));
            }
            let import_by_name_rva = usize::try_from(lookup)
                .map_err(|_| "PE64 import-by-name RVA does not fit usize")?;
            let symbol = file_c_string_at_rva(module, import_by_name_rva + 2, 1024)?;
            if symbol != expected.imported_symbol {
                return Err(format!(
                    "{} import symbol mismatch: expected {}, observed {symbol}",
                    expected.logical_name, expected.imported_symbol
                ));
            }
            return Ok(());
        }
    }
    Err(format!(
        "{} slot RVA is not the exact {}!{} PE64 import",
        expected.logical_name, expected.imported_dll, expected.imported_symbol
    ))
}

#[cfg(windows)]
fn file_c_string_at_rva(
    module: &ObservedMainModule,
    rva: usize,
    max_len: usize,
) -> Result<String, String> {
    let mut bytes = Vec::new();
    for offset in 0..max_len {
        let byte = module_file_bytes_at_rva(module, rva + offset, 1)?[0];
        if byte == 0 {
            return String::from_utf8(bytes)
                .map_err(|_| "PE64 import string is not valid UTF-8".into());
        }
        if !byte.is_ascii() {
            return Err("PE64 import string is not ASCII".into());
        }
        bytes.push(byte);
    }
    Err("PE64 import string is unterminated".into())
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AtomicPointerCasObservation {
    observed_before: usize,
    protection_before: u32,
    protection_temporary: u32,
    protection_after: u32,
}

#[cfg(windows)]
fn atomic_load_pointer_slot(address: usize) -> Result<usize, String> {
    validate_atomic_pointer_slot_memory(address)?;
    let slot = unsafe { &*(address as *const AtomicUsize) };
    Ok(slot.load(Ordering::SeqCst))
}

#[cfg(windows)]
fn atomic_compare_exchange_pointer_slot(
    address: usize,
    expected: usize,
    replacement: usize,
) -> Result<AtomicPointerCasObservation, String> {
    use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_READWRITE, PAGE_WRITECOPY};
    validate_atomic_pointer_slot_memory(address)?;
    let protection_before = pointer_slot_protection(address)?;
    let writable = protection_before & (PAGE_READWRITE | PAGE_WRITECOPY) != 0;
    let protection_temporary = if writable {
        protection_before
    } else {
        let mut previous = 0u32;
        let changed = unsafe {
            VirtualProtect(
                address as *const core::ffi::c_void,
                size_of::<usize>(),
                PAGE_READWRITE,
                &mut previous,
            )
        };
        if changed == 0 {
            return Err("atomic pointer slot page protection change failed".into());
        }
        if previous != protection_before {
            let mut discarded = 0u32;
            let _ = unsafe {
                VirtualProtect(
                    address as *const core::ffi::c_void,
                    size_of::<usize>(),
                    protection_before,
                    &mut discarded,
                )
            };
            return Err(
                "atomic pointer slot page protection observation changed before CAS".into(),
            );
        }
        PAGE_READWRITE
    };
    let slot = unsafe { &*(address as *const AtomicUsize) };
    let observed_before =
        match slot.compare_exchange(expected, replacement, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(previous) | Err(previous) => previous,
        };
    unsafe { windows_sys::Win32::System::Threading::FlushProcessWriteBuffers() };
    if !writable {
        let mut temporary = 0u32;
        let restored = unsafe {
            VirtualProtect(
                address as *const core::ffi::c_void,
                size_of::<usize>(),
                protection_before,
                &mut temporary,
            )
        };
        if restored == 0 || temporary != protection_temporary {
            return Err(
                "atomic pointer slot page protection restoration failed; custody unresolved".into(),
            );
        }
    }
    let protection_after = pointer_slot_protection(address)?;
    if protection_after != protection_before {
        return Err("atomic pointer slot page protection was not exactly restored".into());
    }
    Ok(AtomicPointerCasObservation {
        observed_before,
        protection_before,
        protection_temporary,
        protection_after,
    })
}

#[cfg(windows)]
fn validate_atomic_pointer_slot_memory(address: usize) -> Result<(), String> {
    use windows_sys::Win32::System::Memory::{
        VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS,
    };
    if address == 0 || address % std::mem::align_of::<usize>() != 0 {
        return Err("atomic pointer slot is null or misaligned".into());
    }
    let mut info = MEMORY_BASIC_INFORMATION::default();
    let queried = unsafe {
        VirtualQuery(
            address as *const core::ffi::c_void,
            &mut info,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    let end = address
        .checked_add(size_of::<usize>())
        .ok_or("atomic pointer slot range overflow")?;
    let region_end = (info.BaseAddress as usize)
        .checked_add(info.RegionSize)
        .ok_or("atomic pointer slot region overflow")?;
    if queried != size_of::<MEMORY_BASIC_INFORMATION>()
        || info.State != MEM_COMMIT
        || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
        || address < info.BaseAddress as usize
        || end > region_end
    {
        return Err("atomic pointer slot is not one committed writable pointer word".into());
    }
    Ok(())
}

#[cfg(windows)]
fn pointer_slot_protection(address: usize) -> Result<u32, String> {
    use windows_sys::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION};
    let mut info = MEMORY_BASIC_INFORMATION::default();
    let queried = unsafe {
        VirtualQuery(
            address as *const core::ffi::c_void,
            &mut info,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if queried != size_of::<MEMORY_BASIC_INFORMATION>() {
        return Err("atomic pointer slot protection query failed".into());
    }
    Ok(info.Protect)
}

#[cfg(windows)]
fn validate_external_dispatch_shim_identity(
    identity: ExternalDispatchShimIdentity,
) -> Result<(), String> {
    if identity.entry_address == 0
        || identity.entry_len == 0
        || identity.entry_len > MAX_EXTERNAL_DISPATCH_SHIM_LEN
        || identity.allocation_base == 0
        || identity.allocation_len == 0
        || identity.allocation_len > MAX_EXTERNAL_DISPATCH_ALLOCATION_LEN
        || identity.dispatch_slot_address % std::mem::align_of::<usize>() != 0
        || identity.active_callback_counter_address % std::mem::align_of::<u64>() != 0
    {
        return Err("external dispatch shim identity is unbounded or misaligned".into());
    }
    validate_exact_virtual_allocation(identity.allocation_base, identity.allocation_len)?;
    validate_committed_range_in_allocation(
        identity.entry_address,
        identity.entry_len,
        identity.allocation_base,
        identity.allocation_len,
        true,
        false,
    )?;
    validate_committed_range_in_allocation(
        identity.dispatch_slot_address,
        size_of::<usize>(),
        identity.allocation_base,
        identity.allocation_len,
        false,
        true,
    )?;
    validate_committed_range_in_allocation(
        identity.active_callback_counter_address,
        size_of::<u64>(),
        identity.allocation_base,
        identity.allocation_len,
        false,
        true,
    )?;
    if ranges_overlap(
        identity.entry_address,
        identity.entry_len,
        identity.dispatch_slot_address,
        size_of::<usize>(),
    )? || ranges_overlap(
        identity.entry_address,
        identity.entry_len,
        identity.active_callback_counter_address,
        size_of::<u64>(),
    )? || ranges_overlap(
        identity.dispatch_slot_address,
        size_of::<usize>(),
        identity.active_callback_counter_address,
        size_of::<u64>(),
    )? {
        return Err("external dispatch shim code/data identities overlap".into());
    }
    let observed_hash: [u8; 32] = Sha256::digest(read_process_bytes(
        identity.entry_address,
        identity.entry_len,
    )?)
    .into();
    if observed_hash != identity.entry_bytes_sha256 {
        return Err("external dispatch shim bytes changed".into());
    }
    Ok(())
}

#[cfg(windows)]
fn validate_exact_virtual_allocation(base: usize, expected_len: usize) -> Result<(), String> {
    use windows_sys::Win32::System::Memory::{
        VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS,
    };
    let expected_end = base
        .checked_add(expected_len)
        .ok_or("retained allocation range overflow")?;
    let mut cursor = base;
    while cursor < expected_end {
        let mut info = MEMORY_BASIC_INFORMATION::default();
        let queried = unsafe {
            VirtualQuery(
                cursor as *const core::ffi::c_void,
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if queried != size_of::<MEMORY_BASIC_INFORMATION>()
            || info.AllocationBase as usize != base
            || info.State != MEM_COMMIT
            || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
            || info.BaseAddress as usize != cursor
            || info.RegionSize == 0
        {
            return Err("retained allocation identity is not exact committed memory".into());
        }
        cursor = cursor
            .checked_add(info.RegionSize)
            .ok_or("retained allocation extent overflow")?;
        if cursor > expected_end {
            return Err("retained allocation length ends inside a memory region".into());
        }
    }
    let mut next = MEMORY_BASIC_INFORMATION::default();
    let queried = unsafe {
        VirtualQuery(
            expected_end as *const core::ffi::c_void,
            &mut next,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if queried == size_of::<MEMORY_BASIC_INFORMATION>() && next.AllocationBase as usize == base {
        return Err("retained allocation length omits regions from the same allocation".into());
    }
    Ok(())
}

#[cfg(windows)]
fn validate_committed_range_in_allocation(
    address: usize,
    len: usize,
    allocation_base: usize,
    allocation_len: usize,
    require_executable: bool,
    require_writable: bool,
) -> Result<(), String> {
    use windows_sys::Win32::System::Memory::{
        VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_EXECUTE, PAGE_EXECUTE_READ,
        PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, PAGE_READWRITE,
        PAGE_WRITECOPY,
    };
    let end = address.checked_add(len).ok_or("retained range overflow")?;
    let allocation_end = allocation_base
        .checked_add(allocation_len)
        .ok_or("retained allocation range overflow")?;
    if len == 0 || address < allocation_base || end > allocation_end {
        return Err("retained range is outside its exact allocation".into());
    }
    let mut cursor = address;
    while cursor < end {
        let mut info = MEMORY_BASIC_INFORMATION::default();
        let queried = unsafe {
            VirtualQuery(
                cursor as *const core::ffi::c_void,
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        let executable = info.Protect
            & (PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY)
            != 0;
        let writable = info.Protect
            & (PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY)
            != 0;
        if queried != size_of::<MEMORY_BASIC_INFORMATION>()
            || info.AllocationBase as usize != allocation_base
            || info.State != MEM_COMMIT
            || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
            || (require_executable && !executable)
            || (require_writable && !writable)
        {
            return Err("retained range protection/commit identity is invalid".into());
        }
        let region_end = (info.BaseAddress as usize)
            .checked_add(info.RegionSize)
            .ok_or("retained region extent overflow")?;
        if region_end <= cursor {
            return Err("retained region did not advance".into());
        }
        cursor = region_end.min(end);
    }
    Ok(())
}

fn ranges_overlap(
    left: usize,
    left_len: usize,
    right: usize,
    right_len: usize,
) -> Result<bool, String> {
    let left_end = left.checked_add(left_len).ok_or("range overflow")?;
    let right_end = right.checked_add(right_len).ok_or("range overflow")?;
    Ok(left < right_end && right < left_end)
}

#[cfg(windows)]
fn observe_mapped_pe_identity(module_base: usize) -> Result<PeIdentity, String> {
    let dos = read_process_bytes(module_base, 0x40)?;
    if dos.get(..2) != Some(&[b'M', b'Z']) {
        return Err("production hook main module has no DOS header".into());
    }
    let pe_offset = u32::from_le_bytes(
        dos[0x3c..0x40]
            .try_into()
            .map_err(|_| "production hook DOS header is truncated")?,
    ) as usize;
    if !(0x40..=0x10_0000).contains(&pe_offset) {
        return Err("production hook mapped PE header offset is invalid".into());
    }
    let file_header = module_base
        .checked_add(pe_offset)
        .ok_or("production hook PE header address overflow")?;
    let prefix = read_process_bytes(file_header, 24)?;
    let optional_header_size = read_u16(&prefix, 20, "mapped PE optional-header size")? as usize;
    if optional_header_size < 64 || optional_header_size > 0x1000 {
        return Err("production hook mapped PE optional header has invalid size".into());
    }
    let header_len = pe_offset
        .checked_add(24)
        .and_then(|value| value.checked_add(optional_header_size))
        .ok_or("production hook mapped PE header length overflow")?;
    let mapped = read_process_bytes(module_base, header_len)?;
    parse_pe_identity(&mapped)
}

#[cfg(windows)]
fn parse_file_pe_layout(bytes: &[u8]) -> Result<(PeIdentity, Vec<PeSection>, usize), String> {
    let identity = parse_pe_identity(bytes)?;
    let pe_offset = read_u32(bytes, 0x3c, "file PE header offset")? as usize;
    let section_table = pe_offset
        .checked_add(24)
        .and_then(|value| value.checked_add(identity.optional_header_size as usize))
        .ok_or("production hook file PE section table overflow")?;
    let section_table_end = section_table
        .checked_add(identity.number_of_sections as usize * 40)
        .ok_or("production hook file PE section table range overflow")?;
    if section_table_end > identity.size_of_headers as usize
        || identity.size_of_headers as usize > bytes.len()
    {
        return Err("production hook file PE headers are truncated".into());
    }
    let mut sections = Vec::with_capacity(identity.number_of_sections as usize);
    for index in 0..identity.number_of_sections as usize {
        let offset = section_table + index * 40;
        let virtual_address = read_u32(bytes, offset + 12, "PE section RVA")? as usize;
        let raw_size = read_u32(bytes, offset + 16, "PE section raw size")? as usize;
        let raw_offset = read_u32(bytes, offset + 20, "PE section raw offset")? as usize;
        let raw_end = raw_offset
            .checked_add(raw_size)
            .ok_or("production hook PE section file range overflow")?;
        if raw_end > bytes.len() {
            return Err("production hook PE section exceeds the stable file".into());
        }
        sections.push(PeSection {
            virtual_address,
            raw_size,
            raw_offset,
        });
    }
    Ok((identity, sections, identity.size_of_headers as usize))
}

#[cfg(windows)]
fn parse_pe_identity(bytes: &[u8]) -> Result<PeIdentity, String> {
    if bytes.get(..2) != Some(&[b'M', b'Z']) {
        return Err("production hook PE file has no DOS signature".into());
    }
    let pe_offset = read_u32(bytes, 0x3c, "PE header offset")? as usize;
    if !(0x40..=0x10_0000).contains(&pe_offset) {
        return Err("production hook PE header offset is invalid".into());
    }
    if read_u32(bytes, pe_offset, "PE signature")? != 0x0000_4550 {
        return Err("production hook image has no PE signature".into());
    }
    let machine = read_u16(bytes, pe_offset + 4, "PE machine")?;
    let number_of_sections = read_u16(bytes, pe_offset + 6, "PE section count")?;
    let timestamp = read_u32(bytes, pe_offset + 8, "PE timestamp")?;
    let optional_header_size = read_u16(bytes, pe_offset + 20, "PE optional-header size")?;
    let optional_header = pe_offset
        .checked_add(24)
        .ok_or("production hook optional-header offset overflow")?;
    let optional_header_magic = read_u16(bytes, optional_header, "PE optional-header magic")?;
    let size_of_image = read_u32(bytes, optional_header + 56, "PE SizeOfImage")?;
    let size_of_headers = read_u32(bytes, optional_header + 60, "PE SizeOfHeaders")?;
    if machine != 0x8664
        || number_of_sections == 0
        || number_of_sections > 96
        || optional_header_size < 64
        || optional_header_size > 0x1000
        || optional_header_magic != 0x20b
        || size_of_image == 0
        || size_of_headers == 0
    {
        return Err("production hook image is not a valid bounded x64 PE32+ image".into());
    }
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.pe_identity.v1");
    digest.update(0x0000_4550u32.to_le_bytes());
    digest.update(machine.to_le_bytes());
    digest.update(number_of_sections.to_le_bytes());
    digest.update(timestamp.to_le_bytes());
    digest.update(optional_header_size.to_le_bytes());
    digest.update(optional_header_magic.to_le_bytes());
    digest.update(size_of_image.to_le_bytes());
    digest.update(size_of_headers.to_le_bytes());
    Ok(PeIdentity {
        machine,
        number_of_sections,
        timestamp,
        optional_header_size,
        optional_header_magic,
        size_of_image,
        size_of_headers,
        sha256: digest.finalize().into(),
    })
}

#[cfg(windows)]
fn read_u16(bytes: &[u8], offset: usize, field: &str) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or_else(|| format!("production hook {field} is truncated"))?;
    Ok(u16::from_le_bytes(value.try_into().map_err(|_| {
        format!("production hook {field} is truncated")
    })?))
}

#[cfg(windows)]
fn read_u32(bytes: &[u8], offset: usize, field: &str) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or_else(|| format!("production hook {field} is truncated"))?;
    Ok(u32::from_le_bytes(value.try_into().map_err(|_| {
        format!("production hook {field} is truncated")
    })?))
}

#[cfg(windows)]
fn read_process_bytes(address: usize, len: usize) -> Result<Vec<u8>, String> {
    use std::ffi::c_void;
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    if address == 0 || len == 0 {
        return Err("production hook memory observation range is empty".into());
    }
    let mut bytes = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            bytes.as_mut_ptr().cast(),
            len,
            &mut read,
        )
    };
    if ok == 0 || read != len {
        return Err(format!(
            "production hook memory observation failed at 0x{address:X}"
        ));
    }
    Ok(bytes)
}

const fn hex_digest(value: &str) -> [u8; 32] {
    let bytes = value.as_bytes();
    let mut out = [0u8; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = (hex_nibble(bytes[index * 2]) << 4) | hex_nibble(bytes[index * 2 + 1]);
        index += 1;
    }
    out
}

const fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        b'A'..=b'F' => value - b'A' + 10,
        _ => panic!("invalid hex digest"),
    }
}
