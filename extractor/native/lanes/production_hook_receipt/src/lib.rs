//! Pointer-free production hook custody receipts.
//!
//! Legacy inline-hook callers use this crate to observe already-installed
//! target bytes and retain teardown custody. New pointer-slot callers use its
//! core-owned, non-suspending atomic CAS and sealed process-lifetime dispatch
//! shims. Every path fails closed unless the loaded binary and primitive match
//! their admitted identities.

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
pub const D3D11_CREATE_DEVICE_COUNTED_SHIM_TEMPLATE_ID: &str =
    "d3d11-create-device.pe64-iat-counted-shim-unwind.v1";
pub const D3D_COMPILE_COUNTED_SHIM_TEMPLATE_ID: &str = "d3dcompile.pe64-iat-counted-shim-unwind.v1";
pub const WIN64_REGISTER_ARGS_COUNTED_SHIM_TEMPLATE_ID: &str =
    "win64-register-args.counted-shim-unwind.v1";
pub const D3D11_CREATE_DEVICE_CALLBACK_SEMANTIC_ID: &str =
    "render-bootstrap.d3d11-create-device.callback.v1";
pub const D3D11_CREATE_DEVICE_CALLBACK_EXPORT_NAME: &str =
    "hoi4_bootstrap_d3d11_create_device_callback_v1";
pub const D3D_COMPILE_CALLBACK_SEMANTIC_ID: &str = "render-bootstrap.d3dcompile.callback.v1";
pub const D3D_COMPILE_CALLBACK_EXPORT_NAME: &str = "hoi4_bootstrap_d3d_compile_callback_v1";

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
static SEALED_SHIM_CUSTODY: OnceLock<Mutex<BTreeMap<usize, SealedShimBinding>>> = OnceLock::new();
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalCallbackBinaryIdentity {
    pub module_file_sha256: [u8; 32],
    pub module_pe_identity_sha256: [u8; 32],
    pub module_size_of_image: u32,
    pub callback_rva: u64,
    pub entry_prefix_len: u16,
    pub entry_prefix_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpectedExternalCallbackManifest {
    pub semantic_id: &'static str,
    pub export_name: &'static str,
    pub binary_identity: ExternalCallbackBinaryIdentity,
    callback_class: ExternalCallbackManifestClass,
    authenticity_seal: u64,
    authenticity_digest_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExternalCallbackManifestClass {
    D3D11CreateDevice,
    D3DCompile,
    Win64RegisterArgsOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalCallbackManifestAuthority {
    pub semantic_id: &'static str,
    pub export_name: &'static str,
    pub binary_identity: ExternalCallbackBinaryIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanonicalPe64IatShimTemplate {
    Win64RegisterArgsOnly,
    D3D11CreateDevice,
    D3DCompile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SealedExternalDispatchShim {
    pub identity: ExternalDispatchShimIdentity,
    pub template_id: &'static str,
    pub callback_semantic_id: &'static str,
    pub callback_export_name: &'static str,
    pub callback_binary_identity: ExternalCallbackBinaryIdentity,
    pub callback_manifest_sha256: [u8; 32],
    pub normalized_template_sha256: [u8; 32],
    pub unwind_handler_sha256: [u8; 32],
    pub unwind_info_sha256: [u8; 32],
    pub runtime_function_sha256: [u8; 32],
    pub original_address: usize,
    pub callback_address: usize,
    unwind_registered: bool,
    authenticity_seal: u64,
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
    pub shim_template_id: &'static str,
    pub shim_callback_semantic_id: &'static str,
    pub shim_callback_export_name: &'static str,
    pub shim_callback_binary_identity: ExternalCallbackBinaryIdentity,
    pub shim_callback_manifest_sha256: [u8; 32],
    /// Process-local diagnostic only. Never serialize this field.
    pub shim_callback_address: usize,
    pub shim_normalized_template_sha256: [u8; 32],
    pub shim_unwind_handler_sha256: [u8; 32],
    pub shim_unwind_info_sha256: [u8; 32],
    pub shim_runtime_function_sha256: [u8; 32],
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pe64ImportAddressSlotTerminalPermit {
    logical_name: &'static str,
    slot_rva: u64,
    teardown_owner_token: u64,
    external_shim: ExternalDispatchShimIdentity,
    observed_terminal: ObservedExternalDispatchTerminal,
    authenticity_seal: u64,
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
    pub shim_template_id: &'static str,
    pub shim_callback_semantic_id: &'static str,
    pub shim_callback_export_name: &'static str,
    pub shim_callback_binary_identity: ExternalCallbackBinaryIdentity,
    pub shim_callback_manifest_sha256: [u8; 32],
    /// Process-local diagnostic only. Never serialize this field.
    pub shim_callback_address: usize,
    pub shim_normalized_template_sha256: [u8; 32],
    pub shim_unwind_handler_sha256: [u8; 32],
    pub shim_unwind_info_sha256: [u8; 32],
    pub shim_runtime_function_sha256: [u8; 32],
    pub dispatch_slot_sha256: [u8; 32],
    pub observed_terminal_dispatch_address: usize,
    pub active_callback_counter_sha256: [u8; 32],
    pub observed_active_callback_count: u64,
    pub custody_digest_sha256: [u8; 32],
    pub promotion_digest_sha256: [u8; 32],
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
struct ProvisionalPe64ImportSlotBinding {
    expected: ExpectedPe64ImportAddressSlot,
    external_shim: ExternalDispatchShimIdentity,
    teardown_owner_token: u64,
    slot_address: usize,
    protection_before: u32,
    authenticity_seal: u64,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalPe64ImportSlotBinding {
    installed: InstalledPe64ImportAddressSlotReceipt,
    receipt: RestoredPe64ImportAddressSlotCustodyReceipt,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalizingPe64ImportSlotBinding {
    installed: InstalledPe64ImportAddressSlotReceipt,
    permit: Pe64ImportAddressSlotTerminalPermit,
    observed_terminal: ObservedExternalDispatchTerminal,
    protection_before_restore: u32,
    authenticity_seal: u64,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct QuiescingPe64ImportSlotBinding {
    installed: InstalledPe64ImportAddressSlotReceipt,
    permit: Option<Pe64ImportAddressSlotTerminalPermit>,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
enum Pe64ImportSlotCustodyState {
    Provisional(ProvisionalPe64ImportSlotBinding),
    Active(ActivePe64ImportSlotBinding),
    Quiescing(QuiescingPe64ImportSlotBinding),
    Terminalizing(TerminalizingPe64ImportSlotBinding),
    Terminal(TerminalPe64ImportSlotBinding),
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct SealedShimBinding {
    shim: SealedExternalDispatchShim,
    phase: SealedShimLeasePhase,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SealedShimLeasePhase {
    Available,
    Pe64ImportSlotInstalling {
        teardown_owner_token: u64,
        slot_rva: u64,
    },
    Pe64ImportSlotActive {
        teardown_owner_token: u64,
        slot_rva: u64,
    },
    Pe64ImportSlotTerminalizing {
        teardown_owner_token: u64,
        slot_rva: u64,
    },
    Pe64ImportSlotTerminal {
        teardown_owner_token: u64,
        slot_rva: u64,
    },
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

#[cfg(windows)]
pub fn admit_canonical_pe64_iat_callback_manifest(
    template: CanonicalPe64IatShimTemplate,
    admitted_callback_module_sha256: [u8; 32],
    callback_address: usize,
) -> Result<ExpectedExternalCallbackManifest, String> {
    let (semantic_id, export_name) = match template {
        CanonicalPe64IatShimTemplate::D3D11CreateDevice => (
            D3D11_CREATE_DEVICE_CALLBACK_SEMANTIC_ID,
            D3D11_CREATE_DEVICE_CALLBACK_EXPORT_NAME,
        ),
        CanonicalPe64IatShimTemplate::D3DCompile => (
            D3D_COMPILE_CALLBACK_SEMANTIC_ID,
            D3D_COMPILE_CALLBACK_EXPORT_NAME,
        ),
        CanonicalPe64IatShimTemplate::Win64RegisterArgsOnly => {
            return Err(
                "register-args callbacks require an exact external callback authority".into(),
            );
        }
    };
    if admitted_callback_module_sha256 == [0; 32] {
        return Err("admitted callback module SHA-256 is zero".into());
    }
    let binary_identity =
        observe_external_callback_binary_identity(callback_address, export_name, 64)?;
    if binary_identity.module_file_sha256 != admitted_callback_module_sha256 {
        return Err("loaded callback module differs from the admitted collector SHA-256".into());
    }
    let authenticity_seal = next_receipt_seal()?;
    let mut manifest = ExpectedExternalCallbackManifest {
        semantic_id,
        export_name,
        binary_identity,
        callback_class: match template {
            CanonicalPe64IatShimTemplate::D3D11CreateDevice => {
                ExternalCallbackManifestClass::D3D11CreateDevice
            }
            CanonicalPe64IatShimTemplate::D3DCompile => ExternalCallbackManifestClass::D3DCompile,
            CanonicalPe64IatShimTemplate::Win64RegisterArgsOnly => unreachable!(),
        },
        authenticity_seal,
        authenticity_digest_sha256: [0; 32],
    };
    manifest.authenticity_digest_sha256 = authenticated_external_callback_manifest_sha256(manifest);
    Ok(manifest)
}

#[cfg(windows)]
pub fn admit_win64_register_args_callback_manifest(
    authority: ExternalCallbackManifestAuthority,
    callback_address: usize,
) -> Result<ExpectedExternalCallbackManifest, String> {
    if authority.semantic_id.is_empty()
        || authority.semantic_id.len() > 256
        || !authority.semantic_id.is_ascii()
        || authority.export_name.is_empty()
        || authority.export_name.len() > 256
        || !authority.export_name.is_ascii()
        || authority.export_name.as_bytes().contains(&0)
        || authority.binary_identity.module_file_sha256 == [0; 32]
        || authority.binary_identity.module_pe_identity_sha256 == [0; 32]
        || authority.binary_identity.module_size_of_image == 0
        || authority.binary_identity.callback_rva
            >= u64::from(authority.binary_identity.module_size_of_image)
        || !(16..=256).contains(&authority.binary_identity.entry_prefix_len)
        || authority.binary_identity.entry_prefix_sha256 == [0; 32]
    {
        return Err("register-args callback authority is not exact".into());
    }
    let observed = observe_external_callback_binary_identity(
        callback_address,
        authority.export_name,
        authority.binary_identity.entry_prefix_len,
    )?;
    if observed != authority.binary_identity {
        return Err("register-args callback differs from its exact external authority".into());
    }
    let authenticity_seal = next_receipt_seal()?;
    let mut manifest = ExpectedExternalCallbackManifest {
        semantic_id: authority.semantic_id,
        export_name: authority.export_name,
        binary_identity: authority.binary_identity,
        callback_class: ExternalCallbackManifestClass::Win64RegisterArgsOnly,
        authenticity_seal,
        authenticity_digest_sha256: [0; 32],
    };
    manifest.authenticity_digest_sha256 = authenticated_external_callback_manifest_sha256(manifest);
    Ok(manifest)
}

#[cfg(windows)]
pub fn allocate_canonical_pe64_iat_shim(
    template: CanonicalPe64IatShimTemplate,
    expected_callback: ExpectedExternalCallbackManifest,
    callback_address: usize,
    original_address: usize,
) -> Result<SealedExternalDispatchShim, String> {
    use windows_sys::Win32::System::Memory::{
        VirtualAlloc, VirtualProtect, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_READWRITE,
    };
    if callback_address == 0 || original_address == 0 || callback_address == original_address {
        return Err("canonical IAT shim callback/original identity is invalid".into());
    }
    if expected_callback.authenticity_seal == 0
        || expected_callback.authenticity_digest_sha256
            != authenticated_external_callback_manifest_sha256(expected_callback)
        || expected_callback.semantic_id.is_empty()
        || expected_callback.semantic_id.len() > 256
        || !expected_callback.semantic_id.is_ascii()
        || expected_callback.export_name.is_empty()
        || expected_callback.export_name.len() > 256
        || !expected_callback.export_name.is_ascii()
        || expected_callback.export_name.as_bytes().contains(&0)
    {
        return Err("canonical IAT shim callback manifest identity is invalid".into());
    }
    match template {
        CanonicalPe64IatShimTemplate::D3D11CreateDevice
            if expected_callback.callback_class
                == ExternalCallbackManifestClass::D3D11CreateDevice
                && expected_callback.semantic_id == D3D11_CREATE_DEVICE_CALLBACK_SEMANTIC_ID
                && expected_callback.export_name == D3D11_CREATE_DEVICE_CALLBACK_EXPORT_NAME => {}
        CanonicalPe64IatShimTemplate::D3DCompile
            if expected_callback.callback_class == ExternalCallbackManifestClass::D3DCompile
                && expected_callback.semantic_id == D3D_COMPILE_CALLBACK_SEMANTIC_ID
                && expected_callback.export_name == D3D_COMPILE_CALLBACK_EXPORT_NAME => {}
        CanonicalPe64IatShimTemplate::Win64RegisterArgsOnly
            if expected_callback.callback_class
                == ExternalCallbackManifestClass::Win64RegisterArgsOnly => {}
        _ => {
            return Err(
                "canonical IAT shim template has no exact admitted callback mapping".into(),
            );
        }
    }
    let callback_binary_identity = observe_external_callback_binary_identity(
        callback_address,
        expected_callback.export_name,
        expected_callback.binary_identity.entry_prefix_len,
    )?;
    if callback_binary_identity != expected_callback.binary_identity {
        return Err("canonical IAT shim callback differs from the admitted manifest".into());
    }
    let callback_manifest_sha256 = external_callback_manifest_sha256(expected_callback);
    let (template_id, stack_argument_count, expected_len) = match template {
        CanonicalPe64IatShimTemplate::Win64RegisterArgsOnly => (
            WIN64_REGISTER_ARGS_COUNTED_SHIM_TEMPLATE_ID,
            0usize,
            0x1fusize,
        ),
        CanonicalPe64IatShimTemplate::D3D11CreateDevice => (
            D3D11_CREATE_DEVICE_COUNTED_SHIM_TEMPLATE_ID,
            6usize,
            0x6dusize,
        ),
        CanonicalPe64IatShimTemplate::D3DCompile => {
            (D3D_COMPILE_COUNTED_SHIM_TEMPLATE_ID, 7usize, 0x7ausize)
        }
    };
    let base = unsafe {
        VirtualAlloc(
            std::ptr::null(),
            0x2000,
            MEM_RESERVE | MEM_COMMIT,
            PAGE_READWRITE,
        )
    } as usize;
    if base == 0 {
        return Err("canonical IAT shim VirtualAlloc failed".into());
    }
    let dispatch_address = base + 0x1000;
    let counter_address = base + 0x1008;
    let mut code = Vec::with_capacity(expected_len);
    code.extend_from_slice(&[0xf0, 0x48, 0xff, 0x05]);
    push_rip_disp32(&mut code, base, counter_address)?;
    code.extend_from_slice(&[0x48, 0x83, 0xec, 0x58]);
    for index in 0..stack_argument_count {
        code.extend_from_slice(&[0x48, 0x8b, 0x84, 0x24]);
        code.extend_from_slice(&(0x80u32 + index as u32 * 8).to_le_bytes());
        code.extend_from_slice(&[0x48, 0x89, 0x44, 0x24, 0x20 + index as u8 * 8]);
    }
    code.extend_from_slice(&[0xff, 0x15]);
    push_rip_disp32(&mut code, base, dispatch_address)?;
    code.extend_from_slice(&[0xf0, 0x48, 0xff, 0x0d]);
    push_rip_disp32(&mut code, base, counter_address)?;
    code.extend_from_slice(&[0x48, 0x83, 0xc4, 0x58, 0xc3]);
    if code.len() != expected_len {
        return Err(format!(
            "{template_id} code length mismatch: expected 0x{expected_len:X}, got 0x{:X}",
            code.len()
        ));
    }
    let entry_sha256: [u8; 32] = Sha256::digest(&code).into();
    let admitted_entry_sha256 = match template {
        CanonicalPe64IatShimTemplate::Win64RegisterArgsOnly => Some(hex_digest(
            "d1d8825314f716d1388d358f49f4554084c67e6d4c8c562c3b777da84038b10d",
        )),
        CanonicalPe64IatShimTemplate::D3D11CreateDevice => Some(hex_digest(
            "272f0c711fdc64bc425b25ea559fc94e66beef11cfecdfba1e6d35c0b30ffc05",
        )),
        CanonicalPe64IatShimTemplate::D3DCompile => Some(hex_digest(
            "c931ce51ba48d3f94a6bcdbadc568eb4999b158f61e089eca1c74ac37d8ad475",
        )),
    };
    if admitted_entry_sha256.is_some_and(|expected| expected != entry_sha256) {
        return Err(format!(
            "{template_id} canonical entry bytes differ from authority"
        ));
    }
    let handler = [
        0xf7u8, 0x41, 0x04, 0x66, 0x00, 0x00, 0x00, 0x74, 0x08, 0xf0, 0x48, 0xff, 0x0d, 0xf7, 0x0e,
        0x00, 0x00, 0xb8, 0x01, 0x00, 0x00, 0x00, 0xc3,
    ];
    let unwind = [
        0x11u8, 0x0c, 0x01, 0x00, 0x0c, 0xa2, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
    ];
    let mut runtime_function = Vec::with_capacity(12);
    runtime_function.extend_from_slice(&0u32.to_le_bytes());
    runtime_function.extend_from_slice(&(expected_len as u32).to_le_bytes());
    runtime_function.extend_from_slice(&0x200u32.to_le_bytes());
    unsafe {
        std::ptr::copy_nonoverlapping(code.as_ptr(), base as *mut u8, code.len());
        std::ptr::copy_nonoverlapping(handler.as_ptr(), (base + 0x100) as *mut u8, handler.len());
        std::ptr::copy_nonoverlapping(unwind.as_ptr(), (base + 0x200) as *mut u8, unwind.len());
        std::ptr::copy_nonoverlapping(
            runtime_function.as_ptr(),
            (base + 0x210) as *mut u8,
            runtime_function.len(),
        );
        (dispatch_address as *mut AtomicUsize).write(AtomicUsize::new(callback_address));
        (counter_address as *mut AtomicU64).write(AtomicU64::new(0));
    }
    let authenticity_seal = next_receipt_seal()?;
    let normalized_template_sha256 = canonical_shim_normalized_digest(template, &code);
    let identity = ExternalDispatchShimIdentity {
        entry_address: base,
        entry_len: code.len(),
        entry_bytes_sha256: entry_sha256,
        allocation_base: base,
        allocation_len: 0x2000,
        dispatch_slot_address: dispatch_address,
        active_callback_counter_address: counter_address,
    };
    let mut shim = SealedExternalDispatchShim {
        identity,
        template_id,
        callback_semantic_id: expected_callback.semantic_id,
        callback_export_name: expected_callback.export_name,
        callback_binary_identity,
        callback_manifest_sha256,
        normalized_template_sha256,
        unwind_handler_sha256: Sha256::digest(handler).into(),
        unwind_info_sha256: Sha256::digest(unwind).into(),
        runtime_function_sha256: Sha256::digest(&runtime_function).into(),
        original_address,
        callback_address,
        unwind_registered: false,
        authenticity_seal,
    };
    let registry = SEALED_SHIM_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| "sealed shim custody registry is poisoned")?;
    registry.insert(
        base,
        SealedShimBinding {
            shim: shim.clone(),
            phase: SealedShimLeasePhase::Available,
        },
    );
    let table = (base + 0x210)
        as *const windows_sys::Win32::System::Diagnostics::Debug::IMAGE_RUNTIME_FUNCTION_ENTRY;
    if !unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::RtlAddFunctionTable(table, 1, base as u64)
    } {
        return Err(format!(
            "{template_id} unwind registration failed; allocation retained in provisional custody"
        ));
    }
    shim.unwind_registered = true;
    registry.insert(
        base,
        SealedShimBinding {
            shim: shim.clone(),
            phase: SealedShimLeasePhase::Available,
        },
    );
    let mut previous = 0u32;
    if unsafe {
        VirtualProtect(
            base as *const core::ffi::c_void,
            0x1000,
            PAGE_EXECUTE_READ,
            &mut previous,
        )
    } == 0
        || previous != PAGE_READWRITE
    {
        return Err(format!(
            "{template_id} code protection failed; registered allocation retained"
        ));
    }
    if unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            base as *const core::ffi::c_void,
            0x1000,
        )
    } == 0
    {
        return Err(format!(
            "{template_id} instruction-cache flush failed; registered allocation retained"
        ));
    }
    let mut lookup_base = 0u64;
    let lookup = unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::RtlLookupFunctionEntry(
            base as u64,
            &mut lookup_base,
            std::ptr::null_mut(),
        )
    };
    if lookup != table as *mut _ || lookup_base != base as u64 {
        return Err(format!(
            "{template_id} unwind lookup did not return the registered table/base"
        ));
    }
    registry.insert(
        base,
        SealedShimBinding {
            shim: shim.clone(),
            phase: SealedShimLeasePhase::Available,
        },
    );
    drop(registry);
    validate_sealed_external_dispatch_shim(&shim)?;
    Ok(shim)
}

fn push_rip_disp32(code: &mut Vec<u8>, base: usize, destination: usize) -> Result<(), String> {
    let next = base
        .checked_add(code.len())
        .and_then(|value| value.checked_add(4))
        .ok_or("canonical shim RIP address overflow")?;
    let displacement = (destination as i128) - (next as i128);
    let displacement = i32::try_from(displacement)
        .map_err(|_| "canonical shim RIP displacement does not fit i32")?;
    code.extend_from_slice(&displacement.to_le_bytes());
    Ok(())
}

fn canonical_shim_normalized_digest(
    template: CanonicalPe64IatShimTemplate,
    code: &[u8],
) -> [u8; 32] {
    let mut normalized = code.to_vec();
    let displacement_offsets: &[usize] = match template {
        CanonicalPe64IatShimTemplate::Win64RegisterArgsOnly => &[4, 0x0e, 0x16],
        CanonicalPe64IatShimTemplate::D3D11CreateDevice => &[4, 0x5c, 0x64],
        CanonicalPe64IatShimTemplate::D3DCompile => &[4, 0x69, 0x71],
    };
    for offset in displacement_offsets {
        if let Some(bytes) = normalized.get_mut(*offset..*offset + 4) {
            bytes.fill(0);
        }
    }
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.canonical_counted_shim.v1");
    digest.update([template as u8]);
    digest.update(normalized);
    digest.finalize().into()
}

/// Install one exact PE64 import-address-slot redirection with a naturally
/// aligned atomic pointer compare/exchange. This function never changes code
/// bytes and never suspends a target thread.
#[cfg(windows)]
pub fn install_pe64_import_address_slot(
    expected: ExpectedPe64ImportAddressSlot,
    sealed_shim: &SealedExternalDispatchShim,
    teardown_owner_token: u64,
) -> Result<InstalledPe64ImportAddressSlotReceipt, String> {
    validate_expected_pe64_import_slot(expected)?;
    if teardown_owner_token == 0 {
        return Err(format!(
            "{} teardown owner token is zero",
            expected.logical_name
        ));
    }
    validate_sealed_external_dispatch_shim(sealed_shim)?;
    if sealed_shim.original_address != expected.expected_original_address {
        return Err(format!(
            "{} sealed shim original export mismatch",
            expected.logical_name
        ));
    }
    let external_shim = sealed_shim.identity;
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
        match state {
            Pe64ImportSlotCustodyState::Provisional(binding) => {
                binding.slot_address == slot_address
            }
            Pe64ImportSlotCustodyState::Active(binding) => {
                binding.receipt.observed_module_sha256 == module.sha256
                    && binding.receipt.observed_pe_identity_sha256 == module.pe_identity.sha256
                    && binding.receipt.slot_address == slot_address
            }
            Pe64ImportSlotCustodyState::Quiescing(binding) => {
                binding.installed.observed_module_sha256 == module.sha256
                    && binding.installed.observed_pe_identity_sha256 == module.pe_identity.sha256
                    && binding.installed.slot_address == slot_address
            }
            Pe64ImportSlotCustodyState::Terminalizing(binding) => {
                binding.installed.observed_module_sha256 == module.sha256
                    && binding.installed.observed_pe_identity_sha256 == module.pe_identity.sha256
                    && binding.installed.slot_address == slot_address
            }
            Pe64ImportSlotCustodyState::Terminal(binding) => {
                binding.installed.observed_module_sha256 == module.sha256
                    && binding.installed.observed_pe_identity_sha256 == module.pe_identity.sha256
                    && binding.installed.slot_address == slot_address
            }
        }
    }) {
        return Err(format!(
            "{} import slot already has a different active or terminal lease",
            expected.logical_name
        ));
    }
    match registry.get(&key) {
        Some(Pe64ImportSlotCustodyState::Provisional(binding)) => {
            if binding.expected != expected
                || binding.external_shim != external_shim
                || binding.teardown_owner_token != teardown_owner_token
                || binding.slot_address != slot_address
            {
                return Err(format!(
                    "{} provisional import-slot custody changed",
                    expected.logical_name
                ));
            }
        }
        Some(Pe64ImportSlotCustodyState::Active(binding)) => {
            validate_installed_pe64_import_slot_binding(expected, &binding.receipt)?;
            if binding.receipt.external_shim_address != external_shim_address
                || binding.receipt.external_shim != external_shim
                || binding.receipt.shim_callback_semantic_id != sealed_shim.callback_semantic_id
                || binding.receipt.shim_callback_export_name != sealed_shim.callback_export_name
                || binding.receipt.shim_callback_binary_identity
                    != sealed_shim.callback_binary_identity
                || binding.receipt.shim_callback_manifest_sha256
                    != sealed_shim.callback_manifest_sha256
                || binding.receipt.shim_callback_address != sealed_shim.callback_address
            {
                return Err(format!(
                    "{} install retry changed external shim identity",
                    expected.logical_name
                ));
            }
            validate_sealed_shim_lease_and_live_dispatch(
                sealed_shim,
                SealedShimLeasePhase::Pe64ImportSlotActive {
                    teardown_owner_token,
                    slot_rva: expected.slot_rva,
                },
                sealed_shim.callback_address,
                false,
            )?;
            let observed = atomic_load_pointer_slot(slot_address)?;
            if observed != external_shim_address {
                return Err(format!(
                    "{} active import slot changed before install retry",
                    expected.logical_name
                ));
            }
            return Ok(binding.receipt.clone());
        }
        Some(Pe64ImportSlotCustodyState::Quiescing(_)) => {
            return Err(format!(
                "{} import-slot lease is already quiescing",
                expected.logical_name
            ));
        }
        Some(Pe64ImportSlotCustodyState::Terminalizing(_)) => {
            return Err(format!(
                "{} import-slot lease is already terminalizing",
                expected.logical_name
            ));
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
    let (authenticity_seal, protection_before) = match registry.get(&key) {
        Some(Pe64ImportSlotCustodyState::Provisional(binding)) => {
            validate_sealed_shim_lease_and_live_dispatch(
                sealed_shim,
                SealedShimLeasePhase::Pe64ImportSlotInstalling {
                    teardown_owner_token,
                    slot_rva: expected.slot_rva,
                },
                sealed_shim.callback_address,
                false,
            )?;
            (binding.authenticity_seal, binding.protection_before)
        }
        Some(_) => unreachable!("active and terminal custody returned above"),
        None => {
            let authenticity_seal = next_receipt_seal()?;
            let protection_before = pointer_slot_protection(slot_address)?;
            transition_sealed_shim_lease(
                sealed_shim,
                SealedShimLeasePhase::Available,
                SealedShimLeasePhase::Pe64ImportSlotInstalling {
                    teardown_owner_token,
                    slot_rva: expected.slot_rva,
                },
                sealed_shim.callback_address,
                true,
            )?;
            registry.insert(
                key,
                Pe64ImportSlotCustodyState::Provisional(ProvisionalPe64ImportSlotBinding {
                    expected,
                    external_shim,
                    teardown_owner_token,
                    slot_address,
                    protection_before,
                    authenticity_seal,
                }),
            );
            (authenticity_seal, protection_before)
        }
    };
    let current = atomic_load_pointer_slot(slot_address)?;
    let cas = if current == external_shim_address {
        let current_protection = pointer_slot_protection(slot_address)?;
        if current_protection != protection_before {
            restore_pointer_slot_protection(slot_address, current_protection, protection_before)?;
        }
        AtomicPointerCasObservation {
            observed_before: expected.expected_original_address,
            protection_before,
            protection_temporary: pointer_slot_temporary_protection(protection_before),
            protection_after: pointer_slot_protection(slot_address)?,
        }
    } else {
        atomic_compare_exchange_pointer_slot(
            slot_address,
            expected.expected_original_address,
            external_shim_address,
        )?
    };
    if cas.protection_before != protection_before || cas.protection_after != protection_before {
        return Err(format!(
            "{} provisional import-slot page protection remains unresolved",
            expected.logical_name
        ));
    }
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
        shim_template_id: sealed_shim.template_id,
        shim_callback_semantic_id: sealed_shim.callback_semantic_id,
        shim_callback_export_name: sealed_shim.callback_export_name,
        shim_callback_binary_identity: sealed_shim.callback_binary_identity,
        shim_callback_manifest_sha256: sealed_shim.callback_manifest_sha256,
        shim_callback_address: sealed_shim.callback_address,
        shim_normalized_template_sha256: sealed_shim.normalized_template_sha256,
        shim_unwind_handler_sha256: sealed_shim.unwind_handler_sha256,
        shim_unwind_info_sha256: sealed_shim.unwind_info_sha256,
        shim_runtime_function_sha256: sealed_shim.runtime_function_sha256,
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
    transition_sealed_shim_lease(
        sealed_shim,
        SealedShimLeasePhase::Pe64ImportSlotInstalling {
            teardown_owner_token,
            slot_rva: expected.slot_rva,
        },
        SealedShimLeasePhase::Pe64ImportSlotActive {
            teardown_owner_token,
            slot_rva: expected.slot_rva,
        },
        sealed_shim.callback_address,
        false,
    )?;
    registry.insert(
        key,
        Pe64ImportSlotCustodyState::Active(ActivePe64ImportSlotBinding {
            receipt: receipt.clone(),
        }),
    );
    Ok(receipt)
}

#[cfg(windows)]
pub fn verify_installed_pe64_import_address_slot(
    expected: ExpectedPe64ImportAddressSlot,
    installed: &InstalledPe64ImportAddressSlotReceipt,
    sealed_shim: &SealedExternalDispatchShim,
) -> Result<(), String> {
    validate_expected_pe64_import_slot(expected)?;
    validate_installed_pe64_import_slot_binding(expected, installed)?;
    validate_sealed_external_dispatch_shim(sealed_shim)?;
    if sealed_shim.identity != installed.external_shim
        || sealed_shim.template_id != installed.shim_template_id
        || sealed_shim.callback_semantic_id != installed.shim_callback_semantic_id
        || sealed_shim.callback_export_name != installed.shim_callback_export_name
        || sealed_shim.callback_binary_identity != installed.shim_callback_binary_identity
        || sealed_shim.callback_manifest_sha256 != installed.shim_callback_manifest_sha256
        || sealed_shim.callback_address != installed.shim_callback_address
        || sealed_shim.original_address != installed.expected_original_address
    {
        return Err(format!(
            "{} active verification changed sealed shim authority",
            expected.logical_name
        ));
    }
    let module = observe_main_module()?;
    let slot_address = validate_pe64_import_slot(module, expected)?;
    if slot_address != installed.slot_address
        || atomic_load_pointer_slot(slot_address)? != installed.external_shim_address
    {
        return Err(format!(
            "{} active import-slot live pointer changed",
            expected.logical_name
        ));
    }
    let registry = PE64_IMPORT_SLOT_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let registry = registry
        .lock()
        .map_err(|_| "PE64 import-slot custody registry is poisoned")?;
    let key = (installed.teardown_owner_token, installed.slot_rva);
    match registry.get(&key) {
        Some(Pe64ImportSlotCustodyState::Active(binding)) if binding.receipt == *installed => {}
        _ => {
            return Err(format!(
                "{} is not the crate-owned active import-slot lease",
                expected.logical_name
            ));
        }
    }
    validate_sealed_shim_lease_and_live_dispatch(
        sealed_shim,
        SealedShimLeasePhase::Pe64ImportSlotActive {
            teardown_owner_token: installed.teardown_owner_token,
            slot_rva: installed.slot_rva,
        },
        sealed_shim.callback_address,
        false,
    )
}

/// Atomically cut one active import-slot shim over to its original export and
/// mint a private terminal permit only after the collector callback count is
/// observed at zero. A timed-out call retains resumable crate custody with the
/// dispatch already pointing at the original.
#[cfg(windows)]
pub fn quiesce_pe64_import_address_slot(
    expected: ExpectedPe64ImportAddressSlot,
    installed: &InstalledPe64ImportAddressSlotReceipt,
    sealed_shim: &SealedExternalDispatchShim,
    max_yields: u32,
) -> Result<Pe64ImportAddressSlotTerminalPermit, String> {
    validate_expected_pe64_import_slot(expected)?;
    validate_installed_pe64_import_slot_binding(expected, installed)?;
    validate_sealed_external_dispatch_shim(sealed_shim)?;
    validate_installed_sealed_shim_authority(installed, sealed_shim)?;
    let module = observe_main_module()?;
    let slot_address = validate_pe64_import_slot(module, expected)?;
    if slot_address != installed.slot_address
        || atomic_load_pointer_slot(slot_address)? != installed.external_shim_address
    {
        return Err(format!(
            "{} cannot quiesce after the active import slot changed",
            expected.logical_name
        ));
    }
    let key = (installed.teardown_owner_token, installed.slot_rva);
    let registry = PE64_IMPORT_SLOT_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| "PE64 import-slot custody registry is poisoned")?;
    match registry.get(&key) {
        Some(Pe64ImportSlotCustodyState::Active(binding)) if binding.receipt == *installed => {
            transition_sealed_shim_lease(
                sealed_shim,
                SealedShimLeasePhase::Pe64ImportSlotActive {
                    teardown_owner_token: installed.teardown_owner_token,
                    slot_rva: installed.slot_rva,
                },
                SealedShimLeasePhase::Pe64ImportSlotTerminalizing {
                    teardown_owner_token: installed.teardown_owner_token,
                    slot_rva: installed.slot_rva,
                },
                sealed_shim.callback_address,
                false,
            )?;
            registry.insert(
                key,
                Pe64ImportSlotCustodyState::Quiescing(QuiescingPe64ImportSlotBinding {
                    installed: installed.clone(),
                    permit: None,
                }),
            );
        }
        Some(Pe64ImportSlotCustodyState::Quiescing(binding)) if binding.installed == *installed => {
        }
        Some(Pe64ImportSlotCustodyState::Quiescing(_)) => {
            return Err(format!(
                "{} quiescence retry changed installed custody",
                expected.logical_name
            ));
        }
        _ => {
            return Err(format!(
                "{} is not an active or quiescing import-slot lease",
                expected.logical_name
            ));
        }
    }
    if let Some(Pe64ImportSlotCustodyState::Quiescing(binding)) = registry.get(&key) {
        if let Some(permit) = &binding.permit {
            validate_pe64_import_terminal_permit(installed, sealed_shim, permit)?;
            return Ok(permit.clone());
        }
    }
    atomic_cutover_external_dispatch_to_original(sealed_shim)?;
    let mut observed_zero = false;
    for attempt in 0..=max_yields {
        let (_, active_count) = observe_sealed_shim_dispatch(sealed_shim)?;
        if active_count == 0 {
            observed_zero = true;
            break;
        }
        if attempt != max_yields {
            std::thread::yield_now();
        }
    }
    if !observed_zero {
        return Err(format!(
            "{} collector callbacks did not quiesce within {max_yields} yields; cutover custody retained",
            expected.logical_name
        ));
    }
    let observed_terminal = observe_external_dispatch_terminal_live(
        installed.expected_original_address,
        installed.external_shim,
        true,
    )?;
    let authenticity_seal = next_receipt_seal()? ^ installed.authenticity_seal;
    if authenticity_seal == 0 {
        return Err(format!(
            "{} terminal permit authenticity seal is zero",
            expected.logical_name
        ));
    }
    let permit = Pe64ImportAddressSlotTerminalPermit {
        logical_name: installed.logical_name,
        slot_rva: installed.slot_rva,
        teardown_owner_token: installed.teardown_owner_token,
        external_shim: installed.external_shim,
        observed_terminal,
        authenticity_seal,
    };
    match registry.get_mut(&key) {
        Some(Pe64ImportSlotCustodyState::Quiescing(binding))
            if binding.installed == *installed && binding.permit.is_none() =>
        {
            binding.permit = Some(permit.clone());
        }
        _ => {
            return Err(format!(
                "{} quiescence custody changed before permit publication",
                expected.logical_name
            ));
        }
    }
    Ok(permit)
}

/// Restore one installed PE64 import slot with an atomic pointer CAS after the
/// external shim has switched to the original export and all DLL callbacks are
/// quiescent. The shim allocation remains process-lifetime custody because a
/// target thread may already have loaded its old pointer from the import slot.
#[cfg(windows)]
pub fn restore_pe64_import_address_slot_process_lifetime(
    expected: ExpectedPe64ImportAddressSlot,
    installed: &InstalledPe64ImportAddressSlotReceipt,
    sealed_shim: &SealedExternalDispatchShim,
    permit: &Pe64ImportAddressSlotTerminalPermit,
) -> Result<RestoredPe64ImportAddressSlotCustodyReceipt, String> {
    validate_expected_pe64_import_slot(expected)?;
    validate_installed_pe64_import_slot_binding(expected, installed)?;
    validate_sealed_external_dispatch_shim(sealed_shim)?;
    if sealed_shim.identity != installed.external_shim
        || sealed_shim.template_id != installed.shim_template_id
        || sealed_shim.callback_semantic_id != installed.shim_callback_semantic_id
        || sealed_shim.callback_export_name != installed.shim_callback_export_name
        || sealed_shim.callback_binary_identity != installed.shim_callback_binary_identity
        || sealed_shim.callback_manifest_sha256 != installed.shim_callback_manifest_sha256
        || sealed_shim.callback_address != installed.shim_callback_address
        || sealed_shim.normalized_template_sha256 != installed.shim_normalized_template_sha256
        || sealed_shim.unwind_handler_sha256 != installed.shim_unwind_handler_sha256
        || sealed_shim.unwind_info_sha256 != installed.shim_unwind_info_sha256
        || sealed_shim.runtime_function_sha256 != installed.shim_runtime_function_sha256
    {
        return Err(format!(
            "{} restore changed sealed shim authority",
            expected.logical_name
        ));
    }
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
    let terminalizing = match registry.get(&key) {
        None => {
            return Err(format!(
                "{} has no crate-owned import-slot custody",
                expected.logical_name
            ));
        }
        Some(Pe64ImportSlotCustodyState::Provisional(_)) => {
            return Err(format!(
                "{} import-slot install remains provisional",
                expected.logical_name
            ));
        }
        Some(Pe64ImportSlotCustodyState::Active(_)) => {
            return Err(format!(
                "{} must obtain a core-owned terminal permit before restore",
                expected.logical_name
            ));
        }
        Some(Pe64ImportSlotCustodyState::Quiescing(binding)) => {
            if &binding.installed != installed || binding.permit.as_ref() != Some(permit) {
                return Err(format!(
                    "{} restore changed the core-owned terminal permit",
                    expected.logical_name
                ));
            }
            validate_pe64_import_terminal_permit(installed, sealed_shim, permit)?;
            let authenticity_seal = next_receipt_seal()? ^ installed.authenticity_seal;
            if authenticity_seal == 0 {
                return Err(format!(
                    "{} terminal authenticity seal is zero",
                    expected.logical_name
                ));
            }
            let protection_before_restore = pointer_slot_protection(slot_address)?;
            let terminalizing = TerminalizingPe64ImportSlotBinding {
                installed: installed.clone(),
                permit: permit.clone(),
                observed_terminal: permit.observed_terminal,
                protection_before_restore,
                authenticity_seal,
            };
            registry.insert(
                key,
                Pe64ImportSlotCustodyState::Terminalizing(terminalizing.clone()),
            );
            terminalizing
        }
        Some(Pe64ImportSlotCustodyState::Terminalizing(binding)) => {
            if &binding.installed != installed || &binding.permit != permit {
                return Err(format!(
                    "{} terminalizing retry changed custody inputs",
                    expected.logical_name
                ));
            }
            validate_sealed_shim_lease_and_live_dispatch(
                sealed_shim,
                SealedShimLeasePhase::Pe64ImportSlotTerminalizing {
                    teardown_owner_token: installed.teardown_owner_token,
                    slot_rva: installed.slot_rva,
                },
                installed.expected_original_address,
                false,
            )?;
            binding.clone()
        }
        Some(Pe64ImportSlotCustodyState::Terminal(binding)) => {
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
    let current = atomic_load_pointer_slot(slot_address)?;
    let cas = if current == installed.expected_original_address {
        let current_protection = pointer_slot_protection(slot_address)?;
        if current_protection != terminalizing.protection_before_restore {
            restore_pointer_slot_protection(
                slot_address,
                current_protection,
                terminalizing.protection_before_restore,
            )?;
        }
        AtomicPointerCasObservation {
            observed_before: installed.external_shim_address,
            protection_before: terminalizing.protection_before_restore,
            protection_temporary: pointer_slot_temporary_protection(
                terminalizing.protection_before_restore,
            ),
            protection_after: pointer_slot_protection(slot_address)?,
        }
    } else {
        atomic_compare_exchange_pointer_slot(
            slot_address,
            installed.external_shim_address,
            installed.expected_original_address,
        )?
    };
    if cas.protection_before != terminalizing.protection_before_restore
        || cas.protection_after != terminalizing.protection_before_restore
    {
        return Err(format!(
            "{} terminalizing import-slot protection remains unresolved",
            expected.logical_name
        ));
    }
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
        callbacks_quiesced: true,
        slot_restored: true,
        retained_until_process_exit: true,
        policy_id: PE64_IMPORT_ADDRESS_SLOT_PROCESS_LIFETIME_POLICY_ID,
        external_shim: terminalizing.permit.external_shim,
        shim_template_id: installed.shim_template_id,
        shim_callback_semantic_id: installed.shim_callback_semantic_id,
        shim_callback_export_name: installed.shim_callback_export_name,
        shim_callback_binary_identity: installed.shim_callback_binary_identity,
        shim_callback_manifest_sha256: installed.shim_callback_manifest_sha256,
        shim_callback_address: installed.shim_callback_address,
        shim_normalized_template_sha256: installed.shim_normalized_template_sha256,
        shim_unwind_handler_sha256: installed.shim_unwind_handler_sha256,
        shim_unwind_info_sha256: installed.shim_unwind_info_sha256,
        shim_runtime_function_sha256: installed.shim_runtime_function_sha256,
        dispatch_slot_sha256: terminalizing.observed_terminal.dispatch_slot_sha256,
        observed_terminal_dispatch_address: terminalizing.observed_terminal.dispatch_address,
        active_callback_counter_sha256: terminalizing.observed_terminal.active_counter_sha256,
        observed_active_callback_count: terminalizing.observed_terminal.active_count,
        custody_digest_sha256: [0; 32],
        promotion_digest_sha256: [0; 32],
        authenticity_seal: terminalizing.authenticity_seal,
    };
    receipt.custody_digest_sha256 = restored_pe64_import_slot_custody_digest(&receipt);
    receipt.promotion_digest_sha256 = restored_pe64_import_slot_promotion_digest(&receipt);
    transition_sealed_shim_lease(
        sealed_shim,
        SealedShimLeasePhase::Pe64ImportSlotTerminalizing {
            teardown_owner_token: installed.teardown_owner_token,
            slot_rva: installed.slot_rva,
        },
        SealedShimLeasePhase::Pe64ImportSlotTerminal {
            teardown_owner_token: installed.teardown_owner_token,
            slot_rva: installed.slot_rva,
        },
        installed.expected_original_address,
        false,
    )?;
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
    if restored_pe64_import_slot_promotion_digest(receipt) != receipt.promotion_digest_sha256 {
        return Err(format!(
            "{} terminal import-slot promotion digest mismatch",
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
    digest.update(b"hoi4.production_hook.pe64_import_slot_terminal.v2");
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
    digest_len_prefixed(&mut digest, receipt.shim_template_id.as_bytes());
    digest_len_prefixed(&mut digest, receipt.shim_callback_semantic_id.as_bytes());
    digest_len_prefixed(&mut digest, receipt.shim_callback_export_name.as_bytes());
    digest_external_callback_binary_identity(&mut digest, receipt.shim_callback_binary_identity);
    digest.update(receipt.shim_callback_manifest_sha256);
    digest.update((receipt.shim_callback_address as u64).to_le_bytes());
    digest.update(receipt.shim_normalized_template_sha256);
    digest.update(receipt.shim_unwind_handler_sha256);
    digest.update(receipt.shim_unwind_info_sha256);
    digest.update(receipt.shim_runtime_function_sha256);
    digest.update(receipt.dispatch_slot_sha256);
    digest.update((receipt.observed_terminal_dispatch_address as u64).to_le_bytes());
    digest.update(receipt.active_callback_counter_sha256);
    digest.update(receipt.observed_active_callback_count.to_le_bytes());
    digest.update(receipt.authenticity_seal.to_le_bytes());
    digest.finalize().into()
}

pub fn restored_pe64_import_slot_promotion_digest(
    receipt: &RestoredPe64ImportAddressSlotCustodyReceipt,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.pe64_import_slot_promotion.v2");
    digest_len_prefixed(&mut digest, receipt.logical_name.as_bytes());
    digest.update(receipt.observed_module_sha256);
    digest.update(receipt.observed_pe_identity_sha256);
    digest.update(receipt.observed_pe_timestamp.to_le_bytes());
    digest.update(receipt.observed_size_of_image.to_le_bytes());
    digest_len_prefixed(&mut digest, receipt.imported_dll.as_bytes());
    digest_len_prefixed(&mut digest, receipt.imported_symbol.as_bytes());
    digest.update(receipt.slot_rva.to_le_bytes());
    digest.update(receipt.slot_width.to_le_bytes());
    digest.update(receipt.slot_alignment.to_le_bytes());
    digest_len_prefixed(&mut digest, receipt.primitive_id.as_bytes());
    digest.update([receipt.callbacks_quiesced as u8]);
    digest.update([receipt.slot_restored as u8]);
    digest.update([receipt.retained_until_process_exit as u8]);
    digest_len_prefixed(&mut digest, receipt.policy_id.as_bytes());
    digest_len_prefixed(&mut digest, receipt.shim_template_id.as_bytes());
    digest_len_prefixed(&mut digest, receipt.shim_callback_semantic_id.as_bytes());
    digest_len_prefixed(&mut digest, receipt.shim_callback_export_name.as_bytes());
    digest_external_callback_binary_identity(&mut digest, receipt.shim_callback_binary_identity);
    digest.update(receipt.shim_callback_manifest_sha256);
    digest.update(receipt.shim_normalized_template_sha256);
    digest.update(receipt.shim_unwind_handler_sha256);
    digest.update(receipt.shim_unwind_info_sha256);
    digest.update(receipt.shim_runtime_function_sha256);
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

fn digest_external_callback_binary_identity(
    digest: &mut Sha256,
    identity: ExternalCallbackBinaryIdentity,
) {
    digest.update(identity.module_file_sha256);
    digest.update(identity.module_pe_identity_sha256);
    digest.update(identity.module_size_of_image.to_le_bytes());
    digest.update(identity.callback_rva.to_le_bytes());
    digest.update(identity.entry_prefix_len.to_le_bytes());
    digest.update(identity.entry_prefix_sha256);
}

pub fn external_callback_manifest_sha256(manifest: ExpectedExternalCallbackManifest) -> [u8; 32] {
    external_callback_manifest_fields_sha256(
        manifest.semantic_id,
        manifest.export_name,
        manifest.binary_identity,
    )
}

fn external_callback_manifest_fields_sha256(
    semantic_id: &str,
    export_name: &str,
    binary_identity: ExternalCallbackBinaryIdentity,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.external_callback_manifest.v1");
    digest_len_prefixed(&mut digest, semantic_id.as_bytes());
    digest_len_prefixed(&mut digest, export_name.as_bytes());
    digest_external_callback_binary_identity(&mut digest, binary_identity);
    digest.finalize().into()
}

fn authenticated_external_callback_manifest_sha256(
    manifest: ExpectedExternalCallbackManifest,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.external_callback_manifest.authenticated.v1");
    digest.update(external_callback_manifest_sha256(manifest));
    digest.update([match manifest.callback_class {
        ExternalCallbackManifestClass::D3D11CreateDevice => 1,
        ExternalCallbackManifestClass::D3DCompile => 2,
        ExternalCallbackManifestClass::Win64RegisterArgsOnly => 3,
    }]);
    digest.update(manifest.authenticity_seal.to_le_bytes());
    digest.finalize().into()
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
        || receipt.shim_callback_address == 0
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
    let shim_registry = SEALED_SHIM_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let shim = {
        let shim_registry = shim_registry
            .lock()
            .map_err(|_| "sealed shim custody registry is poisoned")?;
        match shim_registry.get(&receipt.external_shim.allocation_base) {
            Some(binding)
                if binding.shim.identity == receipt.external_shim
                    && binding.shim.template_id == receipt.shim_template_id
                    && binding.shim.callback_semantic_id == receipt.shim_callback_semantic_id
                    && binding.shim.callback_export_name == receipt.shim_callback_export_name
                    && binding.shim.callback_binary_identity
                        == receipt.shim_callback_binary_identity
                    && binding.shim.callback_manifest_sha256
                        == receipt.shim_callback_manifest_sha256
                    && binding.shim.callback_address == receipt.shim_callback_address
                    && binding.shim.original_address == receipt.expected_original_address
                    && binding.shim.normalized_template_sha256
                        == receipt.shim_normalized_template_sha256
                    && binding.shim.unwind_handler_sha256 == receipt.shim_unwind_handler_sha256
                    && binding.shim.unwind_info_sha256 == receipt.shim_unwind_info_sha256
                    && binding.shim.runtime_function_sha256
                        == receipt.shim_runtime_function_sha256 =>
            {
                binding.shim.clone()
            }
            _ => {
                return Err(format!(
                    "{} installed receipt lost sealed shim custody",
                    expected.logical_name
                ));
            }
        }
    };
    validate_sealed_external_dispatch_shim(&shim)
}

#[cfg(windows)]
fn validate_installed_sealed_shim_authority(
    installed: &InstalledPe64ImportAddressSlotReceipt,
    sealed_shim: &SealedExternalDispatchShim,
) -> Result<(), String> {
    if sealed_shim.identity != installed.external_shim
        || sealed_shim.template_id != installed.shim_template_id
        || sealed_shim.callback_semantic_id != installed.shim_callback_semantic_id
        || sealed_shim.callback_export_name != installed.shim_callback_export_name
        || sealed_shim.callback_binary_identity != installed.shim_callback_binary_identity
        || sealed_shim.callback_manifest_sha256 != installed.shim_callback_manifest_sha256
        || sealed_shim.callback_address != installed.shim_callback_address
        || sealed_shim.original_address != installed.expected_original_address
        || sealed_shim.normalized_template_sha256 != installed.shim_normalized_template_sha256
        || sealed_shim.unwind_handler_sha256 != installed.shim_unwind_handler_sha256
        || sealed_shim.unwind_info_sha256 != installed.shim_unwind_info_sha256
        || sealed_shim.runtime_function_sha256 != installed.shim_runtime_function_sha256
    {
        return Err(format!(
            "{} sealed shim authority differs from installed custody",
            installed.logical_name
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn validate_pe64_import_terminal_permit(
    installed: &InstalledPe64ImportAddressSlotReceipt,
    sealed_shim: &SealedExternalDispatchShim,
    permit: &Pe64ImportAddressSlotTerminalPermit,
) -> Result<(), String> {
    validate_installed_sealed_shim_authority(installed, sealed_shim)?;
    if permit.logical_name != installed.logical_name
        || permit.slot_rva != installed.slot_rva
        || permit.teardown_owner_token != installed.teardown_owner_token
        || permit.external_shim != installed.external_shim
        || permit.observed_terminal.dispatch_address != installed.expected_original_address
        || permit.observed_terminal.active_count != 0
        || permit.authenticity_seal == 0
    {
        return Err(format!(
            "{} terminal permit does not bind installed custody",
            installed.logical_name
        ));
    }
    validate_sealed_shim_lease_and_live_dispatch(
        sealed_shim,
        SealedShimLeasePhase::Pe64ImportSlotTerminalizing {
            teardown_owner_token: installed.teardown_owner_token,
            slot_rva: installed.slot_rva,
        },
        installed.expected_original_address,
        false,
    )
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
        || receipt.shim_template_id != installed.shim_template_id
        || receipt.shim_callback_semantic_id != installed.shim_callback_semantic_id
        || receipt.shim_callback_export_name != installed.shim_callback_export_name
        || receipt.shim_callback_binary_identity != installed.shim_callback_binary_identity
        || receipt.shim_callback_manifest_sha256 != installed.shim_callback_manifest_sha256
        || receipt.shim_callback_address != installed.shim_callback_address
        || receipt.shim_normalized_template_sha256 != installed.shim_normalized_template_sha256
        || receipt.shim_unwind_handler_sha256 != installed.shim_unwind_handler_sha256
        || receipt.shim_unwind_info_sha256 != installed.shim_unwind_info_sha256
        || receipt.shim_runtime_function_sha256 != installed.shim_runtime_function_sha256
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
fn observe_external_dispatch_terminal_live(
    expected_original_address: usize,
    external_shim: ExternalDispatchShimIdentity,
    require_quiescent_cutover: bool,
) -> Result<ObservedExternalDispatchTerminal, String> {
    validate_external_dispatch_shim_identity(external_shim)?;
    let dispatch_bytes =
        read_process_bytes(external_shim.dispatch_slot_address, size_of::<usize>())?;
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
        external_shim.active_callback_counter_address,
        size_of::<u64>(),
    )?;
    let active_count = u64::from_le_bytes(
        counter_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "external active callback counter width changed")?,
    );
    if require_quiescent_cutover && active_count != 0 {
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
    let shim = {
        let registry = SEALED_SHIM_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
        let registry = registry
            .lock()
            .map_err(|_| "sealed shim custody registry is poisoned")?;
        match registry.get(&receipt.external_shim.allocation_base) {
            Some(binding)
                if binding.shim.identity == receipt.external_shim
                    && binding.shim.template_id == receipt.shim_template_id
                    && binding.shim.callback_semantic_id == receipt.shim_callback_semantic_id
                    && binding.shim.callback_export_name == receipt.shim_callback_export_name
                    && binding.shim.callback_binary_identity
                        == receipt.shim_callback_binary_identity
                    && binding.shim.callback_manifest_sha256
                        == receipt.shim_callback_manifest_sha256
                    && binding.shim.callback_address == receipt.shim_callback_address
                    && binding.shim.original_address == receipt.expected_original_address
                    && binding.shim.normalized_template_sha256
                        == receipt.shim_normalized_template_sha256
                    && binding.shim.unwind_handler_sha256 == receipt.shim_unwind_handler_sha256
                    && binding.shim.unwind_info_sha256 == receipt.shim_unwind_info_sha256
                    && binding.shim.runtime_function_sha256
                        == receipt.shim_runtime_function_sha256 =>
            {
                binding.shim.clone()
            }
            _ => {
                return Err(format!(
                    "{} terminal receipt lost sealed shim custody",
                    receipt.logical_name
                ));
            }
        }
    };
    validate_sealed_shim_lease_and_live_dispatch(
        &shim,
        SealedShimLeasePhase::Pe64ImportSlotTerminal {
            teardown_owner_token: receipt.teardown_owner_token,
            slot_rva: receipt.slot_rva,
        },
        receipt.expected_original_address,
        false,
    )?;
    if atomic_load_pointer_slot(receipt.slot_address)? != receipt.expected_original_address {
        return Err(format!(
            "{} restored import slot changed",
            receipt.logical_name
        ));
    }
    let observed = observe_external_dispatch_terminal_live(
        receipt.expected_original_address,
        receipt.external_shim,
        false,
    )?;
    if observed.dispatch_slot_sha256 != receipt.dispatch_slot_sha256
        || observed.dispatch_address != receipt.observed_terminal_dispatch_address
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
    characteristics: u32,
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
fn observe_external_callback_binary_identity(
    callback_address: usize,
    export_name: &str,
    entry_prefix_len: u16,
) -> Result<ExternalCallbackBinaryIdentity, String> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetProcAddress};
    use windows_sys::Win32::System::Memory::{
        VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_IMAGE, PAGE_EXECUTE,
        PAGE_EXECUTE_READ, PAGE_GUARD, PAGE_NOACCESS,
    };

    if !(16..=256).contains(&entry_prefix_len)
        || export_name.is_empty()
        || export_name.len() > 256
        || !export_name.is_ascii()
        || export_name.as_bytes().contains(&0)
    {
        return Err("external callback export manifest is invalid".into());
    }
    let mut info = MEMORY_BASIC_INFORMATION::default();
    let queried = unsafe {
        VirtualQuery(
            callback_address as *const core::ffi::c_void,
            &mut info,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    let executable = matches!(info.Protect & 0xff, PAGE_EXECUTE | PAGE_EXECUTE_READ);
    let module_base = info.AllocationBase as usize;
    if callback_address == 0
        || queried != size_of::<MEMORY_BASIC_INFORMATION>()
        || info.State != MEM_COMMIT
        || info.Type != MEM_IMAGE
        || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
        || !executable
        || module_base == 0
        || callback_address < module_base
    {
        return Err("external callback is not executable image memory".into());
    }
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe {
        GetModuleFileNameW(
            module_base as *mut core::ffi::c_void,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    };
    if length == 0 || length as usize >= buffer.len() {
        return Err("external callback module path is unavailable".into());
    }
    let export_name_c = std::ffi::CString::new(export_name)
        .map_err(|_| "external callback export contains an interior NUL")?;
    let export = unsafe {
        GetProcAddress(
            module_base as *mut core::ffi::c_void,
            export_name_c.as_ptr().cast(),
        )
    }
    .map(|function| function as usize)
    .unwrap_or(0);
    if export != callback_address {
        return Err("external callback address is not the admitted exact PE export".into());
    }
    buffer.truncate(length as usize);
    let path = String::from_utf16(&buffer)
        .map_err(|_| "external callback module path is not valid UTF-16")?;
    let mut file = std::fs::File::open(&path)
        .map_err(|error| format!("external callback module open failed: {error}"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("external callback module read failed: {error}"))?;
    let module_file_sha256: [u8; 32] = Sha256::digest(&bytes).into();
    let (pe_identity, sections, _) = parse_file_pe_layout(&bytes)?;
    if observe_mapped_pe_identity(module_base)? != pe_identity {
        return Err("external callback mapped/file PE identity mismatch".into());
    }
    let callback_rva = callback_address - module_base;
    let prefix_len = entry_prefix_len as usize;
    let callback_end_rva = callback_rva
        .checked_add(prefix_len)
        .ok_or("external callback prefix range overflow")?;
    let section = sections.iter().find(|section| {
        callback_rva >= section.virtual_address
            && callback_end_rva <= section.virtual_address.saturating_add(section.raw_size)
            && section.characteristics & 0x2000_0020 == 0x2000_0020
            && section.characteristics & 0x8000_0000 == 0
    });
    let Some(section) = section else {
        return Err("external callback is not exact file-backed module code".into());
    };
    if callback_end_rva > pe_identity.size_of_image as usize {
        return Err("external callback prefix exceeds the loaded image".into());
    }
    let file_offset = section
        .raw_offset
        .checked_add(callback_rva - section.virtual_address)
        .ok_or("external callback file offset overflow")?;
    let file_end = file_offset
        .checked_add(prefix_len)
        .ok_or("external callback file range overflow")?;
    let file_prefix = bytes
        .get(file_offset..file_end)
        .ok_or("external callback file prefix is truncated")?;
    let mapped_prefix = read_process_bytes(callback_address, prefix_len)?;
    if mapped_prefix != file_prefix {
        return Err("external callback mapped entry differs from exact file bytes".into());
    }
    Ok(ExternalCallbackBinaryIdentity {
        module_file_sha256,
        module_pe_identity_sha256: pe_identity.sha256,
        module_size_of_image: pe_identity.size_of_image,
        callback_rva: callback_rva as u64,
        entry_prefix_len,
        entry_prefix_sha256: Sha256::digest(file_prefix).into(),
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
fn atomic_cutover_external_dispatch_to_original(
    sealed_shim: &SealedExternalDispatchShim,
) -> Result<(), String> {
    validate_sealed_external_dispatch_shim(sealed_shim)?;
    let dispatch = unsafe { &*(sealed_shim.identity.dispatch_slot_address as *const AtomicUsize) };
    let current = dispatch.load(Ordering::SeqCst);
    if current == sealed_shim.callback_address {
        let observed = match dispatch.compare_exchange(
            sealed_shim.callback_address,
            sealed_shim.original_address,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(previous) | Err(previous) => previous,
        };
        if observed != sealed_shim.callback_address {
            return Err(format!(
                "sealed external dispatch cutover observed 0x{observed:X}, not callback 0x{:X}",
                sealed_shim.callback_address
            ));
        }
    } else if current != sealed_shim.original_address {
        return Err(format!(
            "sealed external dispatch targets unowned address 0x{current:X}"
        ));
    }
    unsafe { windows_sys::Win32::System::Threading::FlushProcessWriteBuffers() };
    if dispatch.load(Ordering::SeqCst) != sealed_shim.original_address {
        return Err("sealed external dispatch did not retain the original target".into());
    }
    Ok(())
}

#[cfg(windows)]
fn atomic_compare_exchange_pointer_slot(
    address: usize,
    expected: usize,
    replacement: usize,
) -> Result<AtomicPointerCasObservation, String> {
    use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_READWRITE};
    validate_atomic_pointer_slot_memory(address)?;
    let protection_before = pointer_slot_protection(address)?;
    let writable = pointer_protection_is_writable(protection_before);
    let protection_temporary = pointer_slot_temporary_protection(protection_before);
    if !writable {
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
    }
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
fn restore_pointer_slot_protection(
    address: usize,
    observed: u32,
    expected: u32,
) -> Result<(), String> {
    use windows_sys::Win32::System::Memory::VirtualProtect;
    let mut previous = 0u32;
    let changed = unsafe {
        VirtualProtect(
            address as *const core::ffi::c_void,
            size_of::<usize>(),
            expected,
            &mut previous,
        )
    };
    if changed == 0 || previous != observed || pointer_slot_protection(address)? != expected {
        return Err("atomic pointer slot protection recovery remains unresolved".into());
    }
    Ok(())
}

#[cfg(windows)]
fn pointer_protection_is_writable(protection: u32) -> bool {
    use windows_sys::Win32::System::Memory::{
        PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_READWRITE, PAGE_WRITECOPY,
    };
    matches!(
        protection & 0xff,
        PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY
    )
}

#[cfg(windows)]
fn pointer_slot_temporary_protection(protection_before: u32) -> u32 {
    if pointer_protection_is_writable(protection_before) {
        protection_before
    } else {
        windows_sys::Win32::System::Memory::PAGE_READWRITE
    }
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
fn validate_sealed_external_dispatch_shim(shim: &SealedExternalDispatchShim) -> Result<(), String> {
    if !shim.unwind_registered
        || shim.authenticity_seal == 0
        || shim.callback_semantic_id.is_empty()
        || shim.callback_semantic_id.len() > 256
        || !shim.callback_semantic_id.is_ascii()
        || shim.callback_export_name.is_empty()
        || shim.callback_export_name.len() > 256
        || !shim.callback_export_name.is_ascii()
        || shim.callback_export_name.as_bytes().contains(&0)
        || shim.callback_address == 0
        || shim.original_address == 0
        || shim.callback_address == shim.original_address
        || shim.callback_binary_identity.module_file_sha256 == [0; 32]
        || shim.callback_binary_identity.module_pe_identity_sha256 == [0; 32]
        || shim.callback_binary_identity.module_size_of_image == 0
        || shim.callback_binary_identity.callback_rva
            >= u64::from(shim.callback_binary_identity.module_size_of_image)
        || !(16..=256).contains(&shim.callback_binary_identity.entry_prefix_len)
        || shim.callback_binary_identity.entry_prefix_sha256 == [0; 32]
        || shim.callback_manifest_sha256
            != external_callback_manifest_fields_sha256(
                shim.callback_semantic_id,
                shim.callback_export_name,
                shim.callback_binary_identity,
            )
    {
        return Err("sealed external shim lacks unwind/authenticity custody".into());
    }
    let template = match shim.template_id {
        WIN64_REGISTER_ARGS_COUNTED_SHIM_TEMPLATE_ID => {
            CanonicalPe64IatShimTemplate::Win64RegisterArgsOnly
        }
        D3D11_CREATE_DEVICE_COUNTED_SHIM_TEMPLATE_ID => {
            CanonicalPe64IatShimTemplate::D3D11CreateDevice
        }
        D3D_COMPILE_COUNTED_SHIM_TEMPLATE_ID => CanonicalPe64IatShimTemplate::D3DCompile,
        _ => return Err("sealed external shim template ID is not admitted".into()),
    };
    validate_external_dispatch_shim_identity(shim.identity)?;
    validate_committed_range_in_allocation(
        shim.identity.allocation_base + 0x100,
        23,
        shim.identity.allocation_base,
        shim.identity.allocation_len,
        true,
        false,
    )?;
    if pointer_slot_protection(shim.identity.allocation_base)?
        != windows_sys::Win32::System::Memory::PAGE_EXECUTE_READ
        || pointer_slot_protection(shim.identity.allocation_base + 0x1000)?
            != windows_sys::Win32::System::Memory::PAGE_READWRITE
    {
        return Err("sealed external shim page protections changed".into());
    }
    let code = read_process_bytes(shim.identity.entry_address, shim.identity.entry_len)?;
    if canonical_shim_normalized_digest(template, &code) != shim.normalized_template_sha256 {
        return Err("sealed external shim normalized template changed".into());
    }
    let handler = read_process_bytes(shim.identity.allocation_base + 0x100, 23)?;
    let unwind = read_process_bytes(shim.identity.allocation_base + 0x200, 12)?;
    let runtime = read_process_bytes(shim.identity.allocation_base + 0x210, 12)?;
    if Sha256::digest(handler).as_slice() != shim.unwind_handler_sha256
        || shim.unwind_handler_sha256
            != hex_digest("1aa6940862cf632a41f0b52b8eb3884237cdc78c68d5f5a0004d9e0d45bec655")
        || Sha256::digest(unwind).as_slice() != shim.unwind_info_sha256
        || Sha256::digest(runtime).as_slice() != shim.runtime_function_sha256
    {
        return Err("sealed external shim unwind metadata changed".into());
    }
    let expected_table = (shim.identity.allocation_base + 0x210)
        as *mut windows_sys::Win32::System::Diagnostics::Debug::IMAGE_RUNTIME_FUNCTION_ENTRY;
    let mut lookup_base = 0u64;
    let lookup = unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::RtlLookupFunctionEntry(
            shim.identity.entry_address as u64,
            &mut lookup_base,
            std::ptr::null_mut(),
        )
    };
    if lookup != expected_table || lookup_base != shim.identity.allocation_base as u64 {
        return Err("sealed external shim unwind registration changed".into());
    }
    let registry = SEALED_SHIM_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let registry = registry
        .lock()
        .map_err(|_| "sealed shim custody registry is poisoned")?;
    match registry.get(&shim.identity.allocation_base) {
        Some(binding) if binding.shim == *shim => Ok(()),
        _ => Err("sealed external shim is not core-owned custody".into()),
    }
}

#[cfg(windows)]
fn observe_sealed_shim_dispatch(shim: &SealedExternalDispatchShim) -> Result<(usize, u64), String> {
    let dispatch = atomic_load_pointer_slot(shim.identity.dispatch_slot_address)?;
    let counter = unsafe {
        (&*(shim.identity.active_callback_counter_address as *const AtomicU64))
            .load(Ordering::SeqCst)
    };
    Ok((dispatch, counter))
}

#[cfg(windows)]
fn validate_sealed_shim_lease_and_live_dispatch(
    shim: &SealedExternalDispatchShim,
    expected_phase: SealedShimLeasePhase,
    expected_dispatch: usize,
    require_zero_counter: bool,
) -> Result<(), String> {
    validate_sealed_external_dispatch_shim(shim)?;
    let (dispatch, counter) = observe_sealed_shim_dispatch(shim)?;
    if dispatch != expected_dispatch || (require_zero_counter && counter != 0) {
        return Err(format!(
            "sealed external shim live phase mismatch: dispatch=0x{dispatch:X}, counter={counter}"
        ));
    }
    if expected_dispatch == shim.callback_address
        && observe_external_callback_binary_identity(
            shim.callback_address,
            shim.callback_export_name,
            shim.callback_binary_identity.entry_prefix_len,
        )? != shim.callback_binary_identity
    {
        return Err("sealed external shim callback binary identity changed".into());
    }
    let registry = SEALED_SHIM_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let registry = registry
        .lock()
        .map_err(|_| "sealed shim custody registry is poisoned")?;
    match registry.get(&shim.identity.allocation_base) {
        Some(binding) if binding.shim == *shim && binding.phase == expected_phase => Ok(()),
        _ => Err("sealed external shim lease phase changed".into()),
    }
}

#[cfg(windows)]
fn transition_sealed_shim_lease(
    shim: &SealedExternalDispatchShim,
    expected_phase: SealedShimLeasePhase,
    next_phase: SealedShimLeasePhase,
    expected_dispatch: usize,
    require_zero_counter: bool,
) -> Result<(), String> {
    validate_sealed_external_dispatch_shim(shim)?;
    let (dispatch, counter) = observe_sealed_shim_dispatch(shim)?;
    if dispatch != expected_dispatch || (require_zero_counter && counter != 0) {
        return Err(format!(
            "sealed external shim transition mismatch: dispatch=0x{dispatch:X}, counter={counter}"
        ));
    }
    if expected_dispatch == shim.callback_address
        && observe_external_callback_binary_identity(
            shim.callback_address,
            shim.callback_export_name,
            shim.callback_binary_identity.entry_prefix_len,
        )? != shim.callback_binary_identity
    {
        return Err("sealed external shim callback binary identity changed".into());
    }
    let registry = SEALED_SHIM_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| "sealed shim custody registry is poisoned")?;
    match registry.get_mut(&shim.identity.allocation_base) {
        Some(binding) if binding.shim == *shim && binding.phase == expected_phase => {
            binding.phase = next_phase;
            Ok(())
        }
        _ => Err("sealed external shim lease transition is not exact".into()),
    }
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
        let characteristics = read_u32(bytes, offset + 36, "PE section characteristics")?;
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
            characteristics,
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
