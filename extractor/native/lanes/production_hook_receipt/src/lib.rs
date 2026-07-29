//! Pointer-free production hook custody receipts.
//!
//! Legacy inline-hook callers use this crate to observe already-installed
//! target bytes and retain teardown custody. New pointer-slot callers use its
//! core-owned, non-suspending atomic CAS and sealed process-lifetime dispatch
//! shims. Every path fails closed unless the loaded binary and primitive match
//! their admitted identities.

mod custody_record;
pub mod engine_slot;

use custody_record::{perform_recorded_cas, reconcile_recorded_cas};
pub use engine_slot::{
    admit_engine_owned_function_pointer_slot_site, admit_engine_owned_slot_site_roster,
    engine_owned_slot_spec_sha256, slot_enumeration_sha256, AdmittedEngineOwnedSlotRoster,
    AdmittedEngineOwnedSlotSite, DirectCallBypass, EngineOwnedSlotAdmissionError,
    EngineOwnedSlotRosterAdmissionError, EngineOwnedSlotSiteRoster,
    ExpectedEngineOwnedFunctionPointerSlot, OwningStructureIdentity, OwningStructureKind,
    RosterAdmittedEngineOwnedSlotSite, SlotEnumerationCompleteness, SlotEnumerationEvidence,
    ENGINE_OWNED_FUNCTION_POINTER_SLOT_PRIMITIVE_ID,
    ENGINE_OWNED_FUNCTION_POINTER_SLOT_PROCESS_LIFETIME_POLICY_ID, ENGINE_OWNED_SLOT_WIDTH_BYTES,
};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
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
static POINTER_SLOT_CUSTODY: OnceLock<Mutex<BTreeMap<(u64, u64), PointerSlotCustodyState>>> =
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

/// Where a pointer slot comes from, and therefore what may be attested about
/// it.
///
/// The lifecycle below -- aligned compare/exchange, sealed counted shim, atomic
/// disarm, process-lifetime retention -- is identical for both variants. What
/// differs is how the slot's provenance is proved and what a receipt is allowed
/// to claim. A PE import slot is proved from the image's own import directory
/// and attests a DLL and a symbol. An engine-owned slot has no thunk naming
/// itself, so it is proved from its owning structure and a complete
/// incoming-edge census, and it must never be attested as an import.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotProvenance {
    /// A PE64 import address table entry the loader itself wrote.
    Pe64Import {
        imported_dll: &'static str,
        imported_symbol: &'static str,
    },
    /// An engine-owned function-pointer or vtable slot inside the target image.
    ///
    /// The payload is deliberately opaque. An enum variant's fields are as
    /// public as the enum, so leaving them inline made the whole attestation
    /// forgeable by any caller willing to write four plausible-looking values,
    /// and a validator can only ever check that such values are non-zero --
    /// which is not a proof of anything. Wrapping them in a type whose fields
    /// are private means the only way to obtain this variant is to have gone
    /// through the roster admission that produced the digests.
    EngineOwnedFunctionPointer(EngineOwnedSlotAttestation),
}

/// Proof that a slot came from an admitted roster.
///
/// Constructible only inside this crate, and inside it only from a
/// `RosterAdmittedEngineOwnedSlotSite`. Callers read it through the accessors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineOwnedSlotAttestation {
    owner: OwningStructureIdentity,
    enumeration_sha256: [u8; 32],
    roster_sha256: [u8; 32],
    roster_slot_count: usize,
}

impl EngineOwnedSlotAttestation {
    pub(crate) fn from_roster_admitted_site(site: &RosterAdmittedEngineOwnedSlotSite) -> Self {
        Self {
            owner: site.expected().owner,
            enumeration_sha256: site.enumeration_sha256(),
            roster_sha256: site.roster_sha256(),
            roster_slot_count: site.roster_slot_count(),
        }
    }

    pub fn owner(&self) -> OwningStructureIdentity {
        self.owner
    }

    /// Digest of the incoming-edge census this slot rests on, produced by
    /// `engine_slot::slot_enumeration_sha256`.
    pub fn enumeration_sha256(&self) -> [u8; 32] {
        self.enumeration_sha256
    }

    /// Digest of the admitted roster that proved this target's slot coverage
    /// complete. Every slot of one target carries the same value.
    pub fn roster_sha256(&self) -> [u8; 32] {
        self.roster_sha256
    }

    pub fn roster_slot_count(&self) -> usize {
        self.roster_slot_count
    }
}

impl SlotProvenance {
    /// Primitive id this provenance is entitled to attest.
    pub fn primitive_id(&self) -> &'static str {
        match self {
            Self::Pe64Import { .. } => PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS_PRIMITIVE_ID,
            Self::EngineOwnedFunctionPointer(_) => ENGINE_OWNED_FUNCTION_POINTER_SLOT_PRIMITIVE_ID,
        }
    }

    /// Process-lifetime retention policy id this provenance is entitled to
    /// attest.
    pub fn policy_id(&self) -> &'static str {
        match self {
            Self::Pe64Import { .. } => PE64_IMPORT_ADDRESS_SLOT_PROCESS_LIFETIME_POLICY_ID,
            Self::EngineOwnedFunctionPointer(_) => {
                ENGINE_OWNED_FUNCTION_POINTER_SLOT_PROCESS_LIFETIME_POLICY_ID
            }
        }
    }

    /// True only for a genuine PE64 import address slot. Callers that render or
    /// serialize an import DLL/symbol must gate on this rather than assuming.
    pub fn is_pe64_import(&self) -> bool {
        matches!(self, Self::Pe64Import { .. })
    }
}

/// One naturally aligned, pointer-sized dispatch slot admitted for redirection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpectedPointerSlot {
    pub logical_name: &'static str,
    pub slot_rva: u64,
    pub provenance: SlotProvenance,
    /// Exact process-local pointer expected in the slot before installation.
    /// The compare/exchange is against this value; there is no blind store.
    pub expected_original_address: usize,
}

/// PE64 import-address-slot front. The two genuine import sites keep this shape
/// so their receipts continue to attest a DLL and a symbol; nothing else in the
/// crate may construct it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpectedPe64ImportAddressSlot {
    pub logical_name: &'static str,
    pub slot_rva: u64,
    pub imported_dll: &'static str,
    pub imported_symbol: &'static str,
    /// Exact current-process export pointer expected before installation.
    pub expected_original_address: usize,
}

impl ExpectedPe64ImportAddressSlot {
    /// Widen to the provenance-parameterised form the lifecycle operates on.
    pub fn generalize(self) -> ExpectedPointerSlot {
        ExpectedPointerSlot {
            logical_name: self.logical_name,
            slot_rva: self.slot_rva,
            provenance: SlotProvenance::Pe64Import {
                imported_dll: self.imported_dll,
                imported_symbol: self.imported_symbol,
            },
            expected_original_address: self.expected_original_address,
        }
    }
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
pub enum CanonicalCountedShimTemplate {
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
pub struct InstalledPointerSlotReceipt {
    pub logical_name: &'static str,
    pub observed_module_name: &'static str,
    pub observed_module_sha256: [u8; 32],
    pub observed_pe_identity_sha256: [u8; 32],
    pub observed_pe_timestamp: u32,
    pub observed_size_of_image: u32,
    /// How the slot's provenance was proved, and therefore what this receipt
    /// is entitled to claim. Only `SlotProvenance::Pe64Import` names a DLL and
    /// a symbol; an engine-owned slot names its owning structure instead and is
    /// never rendered as an import.
    pub provenance: SlotProvenance,
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
pub struct PointerSlotTerminalPermit {
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
pub struct RestoredPointerSlotCustodyReceipt {
    pub logical_name: &'static str,
    pub observed_module_name: &'static str,
    pub observed_module_sha256: [u8; 32],
    pub observed_pe_identity_sha256: [u8; 32],
    pub observed_pe_timestamp: u32,
    pub observed_size_of_image: u32,
    /// How the slot's provenance was proved, and therefore what this receipt
    /// is entitled to claim. Only `SlotProvenance::Pe64Import` names a DLL and
    /// a symbol; an engine-owned slot names its owning structure instead and is
    /// never rendered as an import.
    pub provenance: SlotProvenance,
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

/// Exact accounting of the lane-retained observations a production install
/// abort discarded.
///
/// An install abort is the only transition allowed to consume producer custody
/// without a sink publication, and it exists solely because a partially
/// installed production roster can never produce an H4RD frame to publish
/// into. Every lane abort entry point returns this so the discard is recorded
/// and reported rather than silently swallowed: a non-zero `retained_rows`
/// means real observations were dropped, which is correct for an aborted
/// install and is never correct for a capture that actually ran.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InstallAbortDiscard {
    /// Producer rows/records/events retained by the lane and discarded here.
    pub retained_rows: u64,
    /// Rows that were already inside an uncommitted reservation.
    pub reserved_rows: u64,
    /// Non-zero drop/poison diagnostic counters discarded with the rows.
    pub discarded_diagnostics: u64,
    /// The lane held a sticky producer error or poisoned completeness latch.
    pub poisoned: bool,
}

impl InstallAbortDiscard {
    /// Const-constructible zero, so a lane can hold a running discard total in
    /// a `static Mutex<InstallAbortDiscard>` without a lazy initializer.
    pub const ZERO: Self = Self {
        retained_rows: 0,
        reserved_rows: 0,
        discarded_diagnostics: 0,
        poisoned: false,
    };

    pub const fn is_empty(&self) -> bool {
        self.retained_rows == 0
            && self.reserved_rows == 0
            && self.discarded_diagnostics == 0
            && !self.poisoned
    }

    pub fn merge(&mut self, other: Self) {
        self.retained_rows = self.retained_rows.saturating_add(other.retained_rows);
        self.reserved_rows = self.reserved_rows.saturating_add(other.reserved_rows);
        self.discarded_diagnostics = self
            .discarded_diagnostics
            .saturating_add(other.discarded_diagnostics);
        self.poisoned |= other.poisoned;
    }
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
struct ActivePointerSlotBinding {
    receipt: InstalledPointerSlotReceipt,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct ProvisionalPointerSlotBinding {
    expected: ExpectedPointerSlot,
    external_shim: ExternalDispatchShimIdentity,
    teardown_owner_token: u64,
    slot_address: usize,
    protection_before: u32,
    /// The install compare-exchange this crate itself performed, recorded the
    /// instant the hardware exchange returned and before any fallible cleanup.
    /// A retry that finds the slot already reading the external shim replays
    /// these values and, if the record is not yet reconciled, finishes the page
    /// cleanup first. A retry with no record here has no evidence of a
    /// transition and must not mint a receipt.
    ///
    /// The cell is handed to the ordering helper directly. There is no accessor
    /// method and no storage trait: the helper writes into this field, and the
    /// only way to put a record into the field is to have performed the
    /// exchange that produced it.
    install_cas_custody: InstallCasCustody,
    authenticity_seal: u64,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalPointerSlotBinding {
    installed: InstalledPointerSlotReceipt,
    receipt: RestoredPointerSlotCustodyReceipt,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalizingPointerSlotBinding {
    installed: InstalledPointerSlotReceipt,
    permit: PointerSlotTerminalPermit,
    observed_terminal: ObservedExternalDispatchTerminal,
    protection_before_restore: u32,
    /// The restore compare-exchange this crate itself performed, recorded the
    /// instant the hardware exchange returned and before any fallible cleanup.
    /// A retry that finds the slot already reading the original replays these
    /// values and, if the record is not yet reconciled, finishes the page
    /// cleanup first. A retry with no record here has no evidence of a
    /// transition and must not mint a receipt.
    ///
    /// The cell is handed to the ordering helper directly. There is no accessor
    /// method and no storage trait: the helper writes into this field, and the
    /// only way to put a record into the field is to have performed the
    /// exchange that produced it.
    restore_cas_custody: RestoreCasCustody,
    authenticity_seal: u64,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct QuiescingPointerSlotBinding {
    installed: InstalledPointerSlotReceipt,
    permit: Option<PointerSlotTerminalPermit>,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
enum PointerSlotCustodyState {
    Provisional(ProvisionalPointerSlotBinding),
    Active(ActivePointerSlotBinding),
    Quiescing(QuiescingPointerSlotBinding),
    Terminalizing(TerminalizingPointerSlotBinding),
    Terminal(TerminalPointerSlotBinding),
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
    PointerSlotInstalling {
        teardown_owner_token: u64,
        slot_rva: u64,
    },
    PointerSlotActive {
        teardown_owner_token: u64,
        slot_rva: u64,
    },
    PointerSlotTerminalizing {
        teardown_owner_token: u64,
        slot_rva: u64,
    },
    PointerSlotTerminal {
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
    template: CanonicalCountedShimTemplate,
    admitted_callback_module_sha256: [u8; 32],
    callback_address: usize,
) -> Result<ExpectedExternalCallbackManifest, String> {
    let (semantic_id, export_name) = match template {
        CanonicalCountedShimTemplate::D3D11CreateDevice => (
            D3D11_CREATE_DEVICE_CALLBACK_SEMANTIC_ID,
            D3D11_CREATE_DEVICE_CALLBACK_EXPORT_NAME,
        ),
        CanonicalCountedShimTemplate::D3DCompile => (
            D3D_COMPILE_CALLBACK_SEMANTIC_ID,
            D3D_COMPILE_CALLBACK_EXPORT_NAME,
        ),
        CanonicalCountedShimTemplate::Win64RegisterArgsOnly => {
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
            CanonicalCountedShimTemplate::D3D11CreateDevice => {
                ExternalCallbackManifestClass::D3D11CreateDevice
            }
            CanonicalCountedShimTemplate::D3DCompile => ExternalCallbackManifestClass::D3DCompile,
            CanonicalCountedShimTemplate::Win64RegisterArgsOnly => unreachable!(),
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
pub fn allocate_canonical_counted_shim(
    template: CanonicalCountedShimTemplate,
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
        CanonicalCountedShimTemplate::D3D11CreateDevice
            if expected_callback.callback_class
                == ExternalCallbackManifestClass::D3D11CreateDevice
                && expected_callback.semantic_id == D3D11_CREATE_DEVICE_CALLBACK_SEMANTIC_ID
                && expected_callback.export_name == D3D11_CREATE_DEVICE_CALLBACK_EXPORT_NAME => {}
        CanonicalCountedShimTemplate::D3DCompile
            if expected_callback.callback_class == ExternalCallbackManifestClass::D3DCompile
                && expected_callback.semantic_id == D3D_COMPILE_CALLBACK_SEMANTIC_ID
                && expected_callback.export_name == D3D_COMPILE_CALLBACK_EXPORT_NAME => {}
        CanonicalCountedShimTemplate::Win64RegisterArgsOnly
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
        CanonicalCountedShimTemplate::Win64RegisterArgsOnly => (
            WIN64_REGISTER_ARGS_COUNTED_SHIM_TEMPLATE_ID,
            0usize,
            0x1fusize,
        ),
        CanonicalCountedShimTemplate::D3D11CreateDevice => (
            D3D11_CREATE_DEVICE_COUNTED_SHIM_TEMPLATE_ID,
            6usize,
            0x6dusize,
        ),
        CanonicalCountedShimTemplate::D3DCompile => {
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
        CanonicalCountedShimTemplate::Win64RegisterArgsOnly => Some(hex_digest(
            "d1d8825314f716d1388d358f49f4554084c67e6d4c8c562c3b777da84038b10d",
        )),
        CanonicalCountedShimTemplate::D3D11CreateDevice => Some(hex_digest(
            "272f0c711fdc64bc425b25ea559fc94e66beef11cfecdfba1e6d35c0b30ffc05",
        )),
        CanonicalCountedShimTemplate::D3DCompile => Some(hex_digest(
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
    template: CanonicalCountedShimTemplate,
    code: &[u8],
) -> [u8; 32] {
    let mut normalized = code.to_vec();
    let displacement_offsets: &[usize] = match template {
        CanonicalCountedShimTemplate::Win64RegisterArgsOnly => &[4, 0x0e, 0x16],
        CanonicalCountedShimTemplate::D3D11CreateDevice => &[4, 0x5c, 0x64],
        CanonicalCountedShimTemplate::D3DCompile => &[4, 0x69, 0x71],
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
/// Re-read the provisional binding out of the registry and require it to hold
/// the record the exchange just produced.
///
/// `perform_recorded_cas` writes through the `&mut` it was handed, and safe Rust
/// cannot state that such a borrow points into the registry entry this receipt
/// will be minted from. A redirected write -- say a leaked throwaway cell --
/// would leave the registry empty and strand the lease silently on the next
/// attempt as `UnwitnessedForeignInstall`, blaming a foreign agent for this
/// crate's own lost evidence. This makes that case say what it is, here, where
/// it happened.
#[cfg(windows)]
fn confirm_provisional_install_custody_retained(
    registry: &BTreeMap<(u64, u64), PointerSlotCustodyState>,
    key: &(u64, u64),
    expected: ExpectedPointerSlot,
    external_shim: ExternalDispatchShimIdentity,
    teardown_owner_token: u64,
    slot_address: usize,
    performed: UnconfirmedInstallCas,
) -> Result<RecordedInstallCas, String> {
    let retained = match registry.get(key) {
        Some(PointerSlotCustodyState::Provisional(binding))
            if binding.expected == expected
                && binding.external_shim == external_shim
                && binding.teardown_owner_token == teardown_owner_token
                && binding.slot_address == slot_address =>
        {
            binding.install_cas_custody.recorded()
        }
        _ => {
            return Err(format!(
                "{} provisional import-slot custody changed after the install CAS",
                expected.logical_name
            ));
        }
    };
    release_install_cas_if_custody_matches(retained, performed)
        .map_err(|error| format!("{} {error}", expected.logical_name))
}

/// The restore direction of [`confirm_provisional_install_custody_retained`].
#[cfg(windows)]
fn confirm_terminalizing_restore_custody_retained(
    registry: &BTreeMap<(u64, u64), PointerSlotCustodyState>,
    key: &(u64, u64),
    expected: ExpectedPointerSlot,
    installed: &InstalledPointerSlotReceipt,
    permit: &PointerSlotTerminalPermit,
    performed: UnconfirmedRestoreCas,
) -> Result<RecordedRestoreCas, String> {
    let retained = match registry.get(key) {
        Some(PointerSlotCustodyState::Terminalizing(binding))
            if binding.installed == *installed && &binding.permit == permit =>
        {
            binding.restore_cas_custody.recorded()
        }
        _ => {
            return Err(format!(
                "{} terminalizing custody changed after the restore CAS",
                expected.logical_name
            ));
        }
    };
    release_restore_cas_if_custody_matches(retained, performed)
        .map_err(|error| format!("{} {error}", expected.logical_name))
}

/// Install one exact PE64 import-address-slot redirection.
///
/// Kept as its own front so the two genuine import sites continue to state a
/// DLL and a symbol at the call site as well as in the receipt. Everything past
/// the provenance widening is the shared lifecycle.
#[cfg(windows)]
pub fn install_pe64_import_address_slot(
    expected: ExpectedPe64ImportAddressSlot,
    sealed_shim: &SealedExternalDispatchShim,
    teardown_owner_token: u64,
) -> Result<InstalledPointerSlotReceipt, String> {
    install_pointer_slot(expected.generalize(), sealed_shim, teardown_owner_token)
}

/// Install one engine-owned slot of a roster.
///
/// Deliberately not public. Admitting a roster proves the target's slot
/// coverage complete, but installing one site of that roster is still a partial
/// redirection: calls through the sibling slots keep reaching the target
/// unobserved, and nothing about a single-site receipt says so. Completeness is
/// therefore enforced where it lives -- across the roster -- by
/// [`EngineOwnedSlotRosterInstallation`], which is the only public route in.
#[cfg(windows)]
fn install_engine_owned_function_pointer_slot(
    site: &RosterAdmittedEngineOwnedSlotSite,
    module_base: usize,
    sealed_shim: &SealedExternalDispatchShim,
    teardown_owner_token: u64,
) -> Result<InstalledPointerSlotReceipt, String> {
    install_pointer_slot(
        engine_owned_expected_pointer_slot(site, module_base)?,
        sealed_shim,
        teardown_owner_token,
    )
}

/// An engine-owned slot roster part-way through installation.
///
/// A target whose census lists two dispatch slots is observed completely only
/// when both are redirected. This type makes that the unit of success: receipts
/// are handed out only by [`Self::finish`], and only once every enumerated slot
/// has been redirected under one teardown owner token.
///
/// A run that fails part-way does not silently degrade to a partial observer.
/// `finish` returns [`IncompleteEngineOwnedSlotRosterInstallation`], which
/// carries the receipts for the slots that *were* redirected precisely so the
/// caller can tear them down -- they are live redirections and must not be
/// leaked -- while making it impossible to mistake them for a working observer.
#[cfg(windows)]
#[derive(Debug)]
pub struct EngineOwnedSlotRosterInstallation {
    target_logical_name: &'static str,
    target_function_rva: u64,
    roster_sha256: [u8; 32],
    module_base: usize,
    teardown_owner_token: u64,
    required_slot_rvas: Vec<u64>,
    unresolved_slot_rvas: Vec<u64>,
    installed: Vec<InstalledPointerSlotReceipt>,
}

/// A roster whose every slot is redirected. The only form an engine-owned
/// observer may treat as live.
///
/// The fields are private for the same reason
/// `EngineOwnedSlotAttestation`'s are: a type whose whole meaning is "this
/// proof was carried out" is worth nothing if anyone can write one down. The
/// only constructor is [`EngineOwnedSlotRosterInstallation::finish`], and it
/// only returns this when every enumerated slot of the roster has a receipt.
#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompleteEngineOwnedSlotRosterInstallation {
    target_logical_name: &'static str,
    target_function_rva: u64,
    roster_sha256: [u8; 32],
    teardown_owner_token: u64,
    receipts: Vec<InstalledPointerSlotReceipt>,
}

#[cfg(windows)]
impl CompleteEngineOwnedSlotRosterInstallation {
    pub fn target_logical_name(&self) -> &'static str {
        self.target_logical_name
    }

    pub fn target_function_rva(&self) -> u64 {
        self.target_function_rva
    }

    /// Digest of the admitted roster whose coverage this installation
    /// completed.
    pub fn roster_sha256(&self) -> [u8; 32] {
        self.roster_sha256
    }

    pub fn teardown_owner_token(&self) -> u64 {
        self.teardown_owner_token
    }

    /// One receipt per enumerated slot of the roster.
    pub fn receipts(&self) -> &[InstalledPointerSlotReceipt] {
        &self.receipts
    }

    /// Consume the proof and take the receipts, for teardown.
    pub fn into_receipts(self) -> Vec<InstalledPointerSlotReceipt> {
        self.receipts
    }
}

/// A roster that did not reach full coverage.
///
/// `installed` is live redirection that still needs the quiesce/restore path;
/// `uncovered_slot_rvas` names what was never redirected and therefore what an
/// observer built on this would have missed.
#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncompleteEngineOwnedSlotRosterInstallation {
    pub target_logical_name: &'static str,
    pub roster_sha256: [u8; 32],
    pub teardown_owner_token: u64,
    pub installed: Vec<InstalledPointerSlotReceipt>,
    pub uncovered_slot_rvas: Vec<u64>,
    /// Slots whose install attempt returned an error.
    ///
    /// An error is not proof that nothing happened: the exchange is recorded
    /// before the page-protection cleanup that can fail after it, so one of
    /// these slots may already be redirected with no receipt yet minted for it.
    /// The recovery is to retry `install_site` with the same site and shim,
    /// which replays the recorded exchange and mints the receipt. Until that
    /// succeeds these slots must be treated as possibly live, and a caller that
    /// abandons the installation here abandons a redirection it cannot yet tear
    /// down. Always a subset of `uncovered_slot_rvas`.
    pub unresolved_slot_rvas: Vec<u64>,
}

#[cfg(windows)]
impl EngineOwnedSlotRosterInstallation {
    /// Begin installing every slot of an admitted roster.
    ///
    /// `module_base` is the process-local base the slot RVAs resolve against;
    /// it derives the expected original pointer, which the shared lifecycle
    /// then re-proves against both the file image and the live slot.
    pub fn begin(
        roster: &AdmittedEngineOwnedSlotRoster,
        module_base: usize,
        teardown_owner_token: u64,
    ) -> Result<Self, String> {
        if teardown_owner_token == 0 {
            return Err(format!(
                "{} engine-owned roster teardown owner token is zero",
                roster.target_logical_name()
            ));
        }
        if roster.sites().is_empty() {
            return Err(format!(
                "{} engine-owned roster has no admitted site",
                roster.target_logical_name()
            ));
        }
        Ok(Self {
            target_logical_name: roster.target_logical_name(),
            target_function_rva: roster.target_function_rva(),
            roster_sha256: roster.roster_sha256(),
            module_base,
            teardown_owner_token,
            required_slot_rvas: roster
                .sites()
                .iter()
                .map(|site| site.expected().slot_rva)
                .collect(),
            unresolved_slot_rvas: Vec::new(),
            installed: Vec::with_capacity(roster.sites().len()),
        })
    }

    /// Slots of this roster that are not yet redirected.
    pub fn uncovered_slot_rvas(&self) -> &[u64] {
        &self.required_slot_rvas
    }

    /// Slots whose install attempt failed and which may already be redirected
    /// without a receipt. See
    /// [`IncompleteEngineOwnedSlotRosterInstallation::unresolved_slot_rvas`].
    pub fn unresolved_slot_rvas(&self) -> &[u64] {
        &self.unresolved_slot_rvas
    }

    /// Redirect one slot of this roster.
    ///
    /// Refuses a site belonging to a different roster, and refuses a repeat of
    /// a slot already redirected under this installation.
    pub fn install_site(
        &mut self,
        site: &RosterAdmittedEngineOwnedSlotSite,
        sealed_shim: &SealedExternalDispatchShim,
    ) -> Result<(), String> {
        if site.roster_sha256() != self.roster_sha256 {
            return Err(format!(
                "{} site belongs to a different admitted roster",
                self.target_logical_name
            ));
        }
        let slot_rva = site.expected().slot_rva;
        let position = self
            .required_slot_rvas
            .iter()
            .position(|required| *required == slot_rva)
            .ok_or_else(|| {
                format!(
                    "{} slot 0x{slot_rva:X} is not an outstanding slot of this roster",
                    self.target_logical_name
                )
            })?;
        // Record the attempt before making it. An error return does not mean
        // the slot is untouched, so the attempt has to be visible even if this
        // call never returns normally.
        if !self.unresolved_slot_rvas.contains(&slot_rva) {
            self.unresolved_slot_rvas.push(slot_rva);
        }
        let receipt = install_engine_owned_function_pointer_slot(
            site,
            self.module_base,
            sealed_shim,
            self.teardown_owner_token,
        )?;
        self.required_slot_rvas.remove(position);
        self.unresolved_slot_rvas.retain(|pending| *pending != slot_rva);
        self.installed.push(receipt);
        Ok(())
    }

    /// Complete the installation, or report what was left uncovered.
    pub fn finish(
        self,
    ) -> Result<
        CompleteEngineOwnedSlotRosterInstallation,
        IncompleteEngineOwnedSlotRosterInstallation,
    > {
        if !self.required_slot_rvas.is_empty() {
            return Err(IncompleteEngineOwnedSlotRosterInstallation {
                target_logical_name: self.target_logical_name,
                roster_sha256: self.roster_sha256,
                teardown_owner_token: self.teardown_owner_token,
                installed: self.installed,
                uncovered_slot_rvas: self.required_slot_rvas,
                unresolved_slot_rvas: self.unresolved_slot_rvas,
            });
        }
        Ok(CompleteEngineOwnedSlotRosterInstallation {
            target_logical_name: self.target_logical_name,
            target_function_rva: self.target_function_rva,
            roster_sha256: self.roster_sha256,
            teardown_owner_token: self.teardown_owner_token,
            receipts: self.installed,
        })
    }
}

/// Widen one roster-admitted engine-owned site into the shared lifecycle's
/// specification form.
///
/// Public because the quiesce and restore paths take the same specification and
/// a caller must be able to reproduce exactly the one it installed with.
pub fn engine_owned_expected_pointer_slot(
    site: &RosterAdmittedEngineOwnedSlotSite,
    module_base: usize,
) -> Result<ExpectedPointerSlot, String> {
    let expected = site.expected();
    if module_base == 0 {
        return Err(format!(
            "{} engine-owned slot module base is zero",
            expected.logical_name
        ));
    }
    if expected.module_sha256 != A729_EXE_SHA256 {
        return Err(format!(
            "{} engine-owned slot is pinned to a module this crate does not admit",
            expected.logical_name
        ));
    }
    let expected_original_address = usize::try_from(expected.expected_original_target_rva)
        .ok()
        .and_then(|rva| module_base.checked_add(rva))
        .ok_or_else(|| {
            format!(
                "{} engine-owned slot target address overflow",
                expected.logical_name
            )
        })?;
    Ok(ExpectedPointerSlot {
        logical_name: expected.logical_name,
        slot_rva: expected.slot_rva,
        provenance: SlotProvenance::EngineOwnedFunctionPointer(
            EngineOwnedSlotAttestation::from_roster_admitted_site(site),
        ),
        expected_original_address,
    })
}

/// The shared install lifecycle.
///
/// Deliberately private. Publishing it would re-open single-slot installation
/// of a multi-slot engine-owned target by the back door: a legitimately
/// roster-admitted site widens into a perfectly valid `ExpectedPointerSlot`,
/// and handing that straight to a generic installer produces a receipt for one
/// slot of a target whose other slots are still dispatching unobserved. The two
/// public routes in -- `install_pe64_import_address_slot` and
/// `EngineOwnedSlotRosterInstallation` -- are the two shapes for which a
/// single-call install is actually complete.
#[cfg(windows)]
fn install_pointer_slot(
    expected: ExpectedPointerSlot,
    sealed_shim: &SealedExternalDispatchShim,
    teardown_owner_token: u64,
) -> Result<InstalledPointerSlotReceipt, String> {
    validate_expected_pointer_slot(expected)?;
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
    let slot_address = validate_pointer_slot(module, expected)?;
    validate_atomic_pointer_slot_memory(slot_address)?;
    let key = (teardown_owner_token, expected.slot_rva);
    let registry = POINTER_SLOT_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| "pointer-slot custody registry is poisoned")?;
    if registry.iter().any(|(existing_key, state)| {
        if *existing_key == key {
            return false;
        }
        match state {
            PointerSlotCustodyState::Provisional(binding) => {
                binding.slot_address == slot_address
            }
            PointerSlotCustodyState::Active(binding) => {
                binding.receipt.observed_module_sha256 == module.sha256
                    && binding.receipt.observed_pe_identity_sha256 == module.pe_identity.sha256
                    && binding.receipt.slot_address == slot_address
            }
            PointerSlotCustodyState::Quiescing(binding) => {
                binding.installed.observed_module_sha256 == module.sha256
                    && binding.installed.observed_pe_identity_sha256 == module.pe_identity.sha256
                    && binding.installed.slot_address == slot_address
            }
            PointerSlotCustodyState::Terminalizing(binding) => {
                binding.installed.observed_module_sha256 == module.sha256
                    && binding.installed.observed_pe_identity_sha256 == module.pe_identity.sha256
                    && binding.installed.slot_address == slot_address
            }
            PointerSlotCustodyState::Terminal(binding) => {
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
        Some(PointerSlotCustodyState::Provisional(binding)) => {
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
        Some(PointerSlotCustodyState::Active(binding)) => {
            validate_installed_pointer_slot_binding(expected, &binding.receipt)?;
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
                SealedShimLeasePhase::PointerSlotActive {
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
        Some(PointerSlotCustodyState::Quiescing(_)) => {
            return Err(format!(
                "{} import-slot lease is already quiescing",
                expected.logical_name
            ));
        }
        Some(PointerSlotCustodyState::Terminalizing(_)) => {
            return Err(format!(
                "{} import-slot lease is already terminalizing",
                expected.logical_name
            ));
        }
        Some(PointerSlotCustodyState::Terminal(_)) => {
            return Err(format!(
                "{} import-slot lease is irreversible terminal custody",
                expected.logical_name
            ));
        }
        None => {}
    }
    let slot_spec_sha256 = pointer_slot_spec_sha256(expected);
    let (authenticity_seal, protection_before, recorded_install_cas) = match registry.get(&key) {
        Some(PointerSlotCustodyState::Provisional(binding)) => {
            validate_sealed_shim_lease_and_live_dispatch(
                sealed_shim,
                SealedShimLeasePhase::PointerSlotInstalling {
                    teardown_owner_token,
                    slot_rva: expected.slot_rva,
                },
                sealed_shim.callback_address,
                false,
            )?;
            (
                binding.authenticity_seal,
                binding.protection_before,
                binding.install_cas_custody.recorded(),
            )
        }
        Some(_) => unreachable!("active and terminal custody returned above"),
        None => {
            let authenticity_seal = next_receipt_seal()?;
            let protection_before = pointer_slot_protection(slot_address)?;
            transition_sealed_shim_lease(
                sealed_shim,
                SealedShimLeasePhase::Available,
                SealedShimLeasePhase::PointerSlotInstalling {
                    teardown_owner_token,
                    slot_rva: expected.slot_rva,
                },
                sealed_shim.callback_address,
                true,
            )?;
            registry.insert(
                key,
                PointerSlotCustodyState::Provisional(ProvisionalPointerSlotBinding {
                    expected,
                    external_shim,
                    teardown_owner_token,
                    slot_address,
                    protection_before,
                    install_cas_custody: InstallCasCustody::unrecorded(),
                    authenticity_seal,
                }),
            );
            (authenticity_seal, protection_before, None)
        }
    };
    let current = atomic_load_pointer_slot(slot_address)?;
    // The arm is chosen by what this crate recorded, not by what the slot
    // happens to read. A slot already holding the external shim with no
    // recorded compare-exchange is somebody else's write and is refused
    // outright -- synthesizing `observed_before` from `expected` there would
    // attest an installation this crate never performed, and would make the
    // `observed_before != expected.expected_original_address` check below a
    // tautology on exactly the path where it is the only remaining evidence.
    let decision = decide_pointer_slot_install_cas(
        current,
        expected.expected_original_address,
        external_shim_address,
        recorded_install_cas,
    )
    .map_err(|error| format!("{} {error}", expected.logical_name))?;
    let cas = match decision {
        PointerSlotInstallCasDecision::PerformCas => {
            // Resolve the registry slot to write into *before* the exchange.
            // After it, recording is a plain field write on a borrow that
            // already exists, so no early return can stand between the
            // exchange and its record.
            let binding = match registry.get_mut(&key) {
                Some(PointerSlotCustodyState::Provisional(binding))
                    if binding.expected == expected
                        && binding.external_shim == external_shim
                        && binding.teardown_owner_token == teardown_owner_token
                        && binding.slot_address == slot_address =>
                {
                    binding
                }
                _ => {
                    return Err(format!(
                        "{} provisional import-slot custody changed before the install CAS",
                        expected.logical_name
                    ));
                }
            };
            // The ordering lives in `custody_record::perform_recorded_cas`: the
            // record is written between the exchange and the fallible cleanup,
            // so a cleanup failure leaves a resumable record instead of losing
            // the evidence that the exchange happened.
            let (performed, protection_after) = perform_recorded_cas(
                || {
                    let prepared = begin_atomic_compare_exchange_pointer_slot(slot_address)?;
                    Ok((
                        prepared,
                        prepared.protection_before,
                        prepared.protection_temporary,
                    ))
                },
                |prepared| {
                    exchange_pointer_slot(
                        prepared,
                        expected.expected_original_address,
                        external_shim_address,
                    )
                },
                &mut binding.install_cas_custody,
                finish_atomic_compare_exchange_pointer_slot,
            )?;
            // `performed` is an `UnconfirmedInstallCas`, so simply deleting
            // this call does not compile -- the receipt below has nothing to
            // form itself from. That makes the re-read the shortest path, not
            // the only one; `custody_record`'s module docs name the two ways
            // around it that still compile.
            let reconciled = confirm_provisional_install_custody_retained(
                &registry,
                &key,
                expected,
                external_shim,
                teardown_owner_token,
                slot_address,
                performed,
            )?;
            complete_recorded_install_cas_observation(reconciled, protection_after)
                .map_err(|error| format!("{} {error}", expected.logical_name))?
        }
        PointerSlotInstallCasDecision::ReconcileProtectionThenReplay(recorded) => {
            // Resolve the registry slot before the reconciliation runs, for the
            // same reason the `PerformCas` arm does: the write that follows must
            // not be able to fail to find its home.
            let binding = match registry.get_mut(&key) {
                Some(PointerSlotCustodyState::Provisional(binding))
                    if binding.expected == expected
                        && binding.external_shim == external_shim
                        && binding.teardown_owner_token == teardown_owner_token
                        && binding.slot_address == slot_address =>
                {
                    binding
                }
                _ => {
                    return Err(format!(
                        "{} provisional import-slot custody changed before the install CAS",
                        expected.logical_name
                    ));
                }
            };
            let (performed, protection_after) = reconcile_recorded_cas(
                recorded,
                &mut binding.install_cas_custody,
                |protection_before| {
                    reconcile_pointer_slot_protection(slot_address, protection_before)
                },
            )?;
            let reconciled = confirm_provisional_install_custody_retained(
                &registry,
                &key,
                expected,
                external_shim,
                teardown_owner_token,
                slot_address,
                performed,
            )?;
            complete_recorded_install_cas_observation(reconciled, protection_after)
                .map_err(|error| format!("{} {error}", expected.logical_name))?
        }
        PointerSlotInstallCasDecision::ReplayRecordedCas(recorded) => {
            let protection_after =
                reconcile_pointer_slot_protection(slot_address, protection_before)?;
            complete_recorded_install_cas_observation(recorded, protection_after)
                .map_err(|error| format!("{} {error}", expected.logical_name))?
        }
    };
    // Both comparisons are against the protection observed when the provisional
    // binding was created, not against the record's own. KNOWN, DELIBERATE, and
    // deliberately kept: see `verify_install_cas_matches_lease_protection` for
    // why a permanently stranded lease is the right outcome here and what each
    // typed refusal means.
    verify_install_cas_matches_lease_protection(protection_before, cas)
        .map_err(|error| format!("{} {error}", expected.logical_name))?;
    // Non-tautological now: `observed_before` is only ever a value some real
    // compare-exchange returned, replayed verbatim from the record if this is a
    // retry. Before the fix the already-shim arm defined it to equal the value
    // this line compares it against.
    let observed_before = cas.observed_before;
    if observed_before != expected.expected_original_address {
        return Err(format!(
            "{} import slot CAS expected 0x{:X}, observed 0x{observed_before:X}",
            expected.logical_name, expected.expected_original_address
        ));
    }
    // KNOWN, DELIBERATE, and deliberately kept: if an external writer puts the
    // slot back to the original between the exchange and this load, the call
    // fails here and the retry is refused as `RecordedInstallCasSlotDiverged`
    // rather than driving a second exchange. The pre-change code recovered from
    // this only by synthesizing the observation, which is the defect this file's
    // custody records remove; a record whose slot no longer holds the shim
    // genuinely no longer describes the live slot, so re-exchanging under the
    // same record would attest a transition against evidence from a different
    // one. Re-exchanging under a *replacement* record would be honest about the
    // second exchange but silently drop the fact that a third party wrote the
    // slot in between -- an installation receipt says this invocation held the
    // slot from original to shim, and after a contested write it did not.
    let observed_after = atomic_load_pointer_slot(slot_address)?;
    if observed_after != external_shim_address {
        return Err(format!(
            "{} import slot did not retain the external shim after CAS",
            expected.logical_name
        ));
    }
    let receipt = InstalledPointerSlotReceipt {
        logical_name: expected.logical_name,
        observed_module_name: BATTLE_PLAN_MODULE_NAME,
        observed_module_sha256: module.sha256,
        observed_pe_identity_sha256: module.pe_identity.sha256,
        observed_pe_timestamp: module.pe_identity.timestamp,
        observed_size_of_image: module.pe_identity.size_of_image,
        provenance: expected.provenance,
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
        primitive_id: expected.provenance.primitive_id(),
        slot_spec_sha256,
        teardown_owner_token,
        installed: true,
        authenticity_seal,
    };
    transition_sealed_shim_lease(
        sealed_shim,
        SealedShimLeasePhase::PointerSlotInstalling {
            teardown_owner_token,
            slot_rva: expected.slot_rva,
        },
        SealedShimLeasePhase::PointerSlotActive {
            teardown_owner_token,
            slot_rva: expected.slot_rva,
        },
        sealed_shim.callback_address,
        false,
    )?;
    registry.insert(
        key,
        PointerSlotCustodyState::Active(ActivePointerSlotBinding {
            receipt: receipt.clone(),
        }),
    );
    Ok(receipt)
}

#[cfg(windows)]
pub fn verify_installed_pointer_slot(
    expected: ExpectedPointerSlot,
    installed: &InstalledPointerSlotReceipt,
    sealed_shim: &SealedExternalDispatchShim,
) -> Result<(), String> {
    validate_expected_pointer_slot(expected)?;
    validate_installed_pointer_slot_binding(expected, installed)?;
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
    let slot_address = validate_pointer_slot(module, expected)?;
    if slot_address != installed.slot_address
        || atomic_load_pointer_slot(slot_address)? != installed.external_shim_address
    {
        return Err(format!(
            "{} active import-slot live pointer changed",
            expected.logical_name
        ));
    }
    let registry = POINTER_SLOT_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let registry = registry
        .lock()
        .map_err(|_| "pointer-slot custody registry is poisoned")?;
    let key = (installed.teardown_owner_token, installed.slot_rva);
    match registry.get(&key) {
        Some(PointerSlotCustodyState::Active(binding)) if binding.receipt == *installed => {}
        _ => {
            return Err(format!(
                "{} is not the crate-owned active import-slot lease",
                expected.logical_name
            ));
        }
    }
    validate_sealed_shim_lease_and_live_dispatch(
        sealed_shim,
        SealedShimLeasePhase::PointerSlotActive {
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
pub fn quiesce_pointer_slot(
    expected: ExpectedPointerSlot,
    installed: &InstalledPointerSlotReceipt,
    sealed_shim: &SealedExternalDispatchShim,
    max_yields: u32,
) -> Result<PointerSlotTerminalPermit, String> {
    validate_expected_pointer_slot(expected)?;
    validate_installed_pointer_slot_binding(expected, installed)?;
    validate_sealed_external_dispatch_shim(sealed_shim)?;
    validate_installed_sealed_shim_authority(installed, sealed_shim)?;
    let max_yields = validate_pointer_slot_quiesce_yields(max_yields)
        .map_err(|error| format!("{} {error}", expected.logical_name))?;
    let module = observe_main_module()?;
    let slot_address = validate_pointer_slot(module, expected)?;
    if slot_address != installed.slot_address
        || atomic_load_pointer_slot(slot_address)? != installed.external_shim_address
    {
        return Err(format!(
            "{} cannot quiesce after the active import slot changed",
            expected.logical_name
        ));
    }
    let key = (installed.teardown_owner_token, installed.slot_rva);
    let registry = POINTER_SLOT_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    // Publish `Quiescing` under the guard, then drop it. The cutover and the
    // yield loop below run unlocked: they are the caller-bounded part of the
    // operation and every other import-slot custody call needs this same
    // process-wide registry. `Quiescing` is exactly the resumable state that
    // makes releasing the guard safe -- re-entry is handled above and the
    // re-acquire below re-verifies the binding before publishing anything.
    let published_permit = {
        let mut registry = registry
            .lock()
            .map_err(|_| "pointer-slot custody registry is poisoned")?;
        match registry.get(&key) {
            Some(PointerSlotCustodyState::Active(binding)) if binding.receipt == *installed => {
                transition_sealed_shim_lease(
                    sealed_shim,
                    SealedShimLeasePhase::PointerSlotActive {
                        teardown_owner_token: installed.teardown_owner_token,
                        slot_rva: installed.slot_rva,
                    },
                    SealedShimLeasePhase::PointerSlotTerminalizing {
                        teardown_owner_token: installed.teardown_owner_token,
                        slot_rva: installed.slot_rva,
                    },
                    sealed_shim.callback_address,
                    false,
                )?;
                registry.insert(
                    key,
                    PointerSlotCustodyState::Quiescing(QuiescingPointerSlotBinding {
                        installed: installed.clone(),
                        permit: None,
                    }),
                );
                None
            }
            Some(PointerSlotCustodyState::Quiescing(binding))
                if binding.installed == *installed =>
            {
                binding.permit.clone()
            }
            Some(PointerSlotCustodyState::Quiescing(_)) => {
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
    };
    if let Some(permit) = published_permit {
        validate_pointer_slot_terminal_permit(installed, sealed_shim, &permit)?;
        return Ok(permit);
    }
    atomic_cutover_external_dispatch_to_original(sealed_shim)?;
    let started = std::time::Instant::now();
    let outcome = drive_pointer_slot_quiesce_loop(
        max_yields,
        POINTER_SLOT_QUIESCE_TIME_BUDGET,
        || observe_sealed_shim_dispatch(sealed_shim).map(|(_, active_count)| active_count),
        std::thread::yield_now,
        || started.elapsed(),
    )?;
    if !outcome.observed_zero() {
        return Err(render_pointer_slot_quiesce_timeout(
            expected.logical_name,
            &outcome,
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
    let permit = PointerSlotTerminalPermit {
        logical_name: installed.logical_name,
        slot_rva: installed.slot_rva,
        teardown_owner_token: installed.teardown_owner_token,
        external_shim: installed.external_shim,
        observed_terminal,
        authenticity_seal,
    };
    // Re-acquire and re-verify. A racing caller may have finished the same
    // quiescence while the guard was down; adopting its permit is the same
    // outcome the re-entry fast path above gives, so validate and return it
    // instead of erroring. Anything other than `Quiescing` with this exact
    // `installed` stays a hard error.
    let (published, adopted_racing_permit) = {
        let mut registry = registry
            .lock()
            .map_err(|_| "pointer-slot custody registry is poisoned")?;
        match registry.get_mut(&key) {
            Some(PointerSlotCustodyState::Quiescing(binding))
                if binding.installed == *installed =>
            {
                match &binding.permit {
                    Some(racing) => (racing.clone(), true),
                    None => {
                        binding.permit = Some(permit.clone());
                        (permit, false)
                    }
                }
            }
            _ => {
                return Err(format!(
                    "{} quiescence custody changed before permit publication",
                    expected.logical_name
                ));
            }
        }
    };
    if adopted_racing_permit {
        validate_pointer_slot_terminal_permit(installed, sealed_shim, &published)?;
    }
    Ok(published)
}

/// Restore one installed PE64 import slot with an atomic pointer CAS after the
/// external shim has switched to the original export and all DLL callbacks are
/// quiescent. The shim allocation remains process-lifetime custody because a
/// target thread may already have loaded its old pointer from the import slot.
#[cfg(windows)]
pub fn restore_pointer_slot_process_lifetime(
    expected: ExpectedPointerSlot,
    installed: &InstalledPointerSlotReceipt,
    sealed_shim: &SealedExternalDispatchShim,
    permit: &PointerSlotTerminalPermit,
) -> Result<RestoredPointerSlotCustodyReceipt, String> {
    validate_expected_pointer_slot(expected)?;
    validate_installed_pointer_slot_binding(expected, installed)?;
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
    let slot_address = validate_pointer_slot(module, expected)?;
    if slot_address != installed.slot_address {
        return Err(format!(
            "{} import slot address changed",
            expected.logical_name
        ));
    }
    validate_atomic_pointer_slot_memory(slot_address)?;
    let key = (installed.teardown_owner_token, installed.slot_rva);
    let registry = POINTER_SLOT_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| "pointer-slot custody registry is poisoned")?;
    let terminalizing = match registry.get(&key) {
        None => {
            return Err(format!(
                "{} has no crate-owned import-slot custody",
                expected.logical_name
            ));
        }
        Some(PointerSlotCustodyState::Provisional(_)) => {
            return Err(format!(
                "{} import-slot install remains provisional",
                expected.logical_name
            ));
        }
        Some(PointerSlotCustodyState::Active(_)) => {
            return Err(format!(
                "{} must obtain a core-owned terminal permit before restore",
                expected.logical_name
            ));
        }
        Some(PointerSlotCustodyState::Quiescing(binding)) => {
            if &binding.installed != installed || binding.permit.as_ref() != Some(permit) {
                return Err(format!(
                    "{} restore changed the core-owned terminal permit",
                    expected.logical_name
                ));
            }
            validate_pointer_slot_terminal_permit(installed, sealed_shim, permit)?;
            let authenticity_seal = next_receipt_seal()? ^ installed.authenticity_seal;
            if authenticity_seal == 0 {
                return Err(format!(
                    "{} terminal authenticity seal is zero",
                    expected.logical_name
                ));
            }
            let protection_before_restore = pointer_slot_protection(slot_address)?;
            let terminalizing = TerminalizingPointerSlotBinding {
                installed: installed.clone(),
                permit: permit.clone(),
                observed_terminal: permit.observed_terminal,
                protection_before_restore,
                restore_cas_custody: RestoreCasCustody::unrecorded(),
                authenticity_seal,
            };
            registry.insert(
                key,
                PointerSlotCustodyState::Terminalizing(terminalizing.clone()),
            );
            terminalizing
        }
        Some(PointerSlotCustodyState::Terminalizing(binding)) => {
            if &binding.installed != installed || &binding.permit != permit {
                return Err(format!(
                    "{} terminalizing retry changed custody inputs",
                    expected.logical_name
                ));
            }
            validate_sealed_shim_lease_and_live_dispatch(
                sealed_shim,
                SealedShimLeasePhase::PointerSlotTerminalizing {
                    teardown_owner_token: installed.teardown_owner_token,
                    slot_rva: installed.slot_rva,
                },
                installed.expected_original_address,
                false,
            )?;
            binding.clone()
        }
        Some(PointerSlotCustodyState::Terminal(binding)) => {
            if &binding.installed != installed {
                return Err(format!(
                    "{} terminal retry changed installed receipt",
                    expected.logical_name
                ));
            }
            validate_terminal_pointer_slot_live(&binding.receipt)?;
            return Ok(binding.receipt.clone());
        }
    };
    let current = atomic_load_pointer_slot(slot_address)?;
    // The arm is chosen by what this crate recorded, not by what the slot
    // happens to read. A slot already holding the original with no recorded
    // compare-exchange is somebody else's restore and is refused outright --
    // synthesizing `observed_before` from `installed` there would attest a
    // transition this crate never witnessed.
    let decision = decide_pointer_slot_restore_cas(
        current,
        installed.expected_original_address,
        installed.external_shim_address,
        terminalizing.restore_cas_custody.recorded(),
    )
    .map_err(|error| format!("{} {error}", expected.logical_name))?;
    let cas = match decision {
        PointerSlotRestoreCasDecision::PerformCas => {
            // Resolve the registry slot to write into *before* the exchange.
            // After it, recording is a plain field write on a borrow that
            // already exists, so no early return can stand between the
            // exchange and its record.
            let binding = match registry.get_mut(&key) {
                Some(PointerSlotCustodyState::Terminalizing(binding))
                    if binding.installed == *installed && &binding.permit == permit =>
                {
                    binding
                }
                _ => {
                    return Err(format!(
                        "{} terminalizing custody changed before the restore CAS",
                        expected.logical_name
                    ));
                }
            };
            // The ordering lives in `custody_record::perform_recorded_cas`: the
            // record below is written between the exchange and the fallible
            // cleanup, so a cleanup failure leaves a resumable record instead of
            // losing the evidence that the exchange happened.
            let (performed, protection_after) = perform_recorded_cas(
                || {
                    let prepared = begin_atomic_compare_exchange_pointer_slot(slot_address)?;
                    Ok((
                        prepared,
                        prepared.protection_before,
                        prepared.protection_temporary,
                    ))
                },
                |prepared| {
                    exchange_pointer_slot(
                        prepared,
                        installed.external_shim_address,
                        installed.expected_original_address,
                    )
                },
                &mut binding.restore_cas_custody,
                finish_atomic_compare_exchange_pointer_slot,
            )?;
            // `performed` is an `UnconfirmedRestoreCas`, so simply deleting
            // this call does not compile -- the receipt below has nothing to
            // form itself from. That makes the re-read the shortest path, not
            // the only one; `custody_record`'s module docs name the two ways
            // around it that still compile.
            let reconciled = confirm_terminalizing_restore_custody_retained(
                &registry,
                &key,
                expected,
                installed,
                permit,
                performed,
            )?;
            complete_recorded_restore_cas_observation(reconciled, protection_after)
                .map_err(|error| format!("{} {error}", expected.logical_name))?
        }
        PointerSlotRestoreCasDecision::ReconcileProtectionThenReplay(recorded) => {
            // Resolve the registry slot before the reconciliation runs, for the
            // same reason the `PerformCas` arm does: the write that follows must
            // not be able to fail to find its home.
            let binding = match registry.get_mut(&key) {
                Some(PointerSlotCustodyState::Terminalizing(binding))
                    if binding.installed == *installed && &binding.permit == permit =>
                {
                    binding
                }
                _ => {
                    return Err(format!(
                        "{} terminalizing custody changed before the restore CAS",
                        expected.logical_name
                    ));
                }
            };
            let (performed, protection_after) = reconcile_recorded_cas(
                recorded,
                &mut binding.restore_cas_custody,
                |protection_before| {
                    reconcile_pointer_slot_protection(slot_address, protection_before)
                },
            )?;
            let reconciled = confirm_terminalizing_restore_custody_retained(
                &registry,
                &key,
                expected,
                installed,
                permit,
                performed,
            )?;
            complete_recorded_restore_cas_observation(reconciled, protection_after)
                .map_err(|error| format!("{} {error}", expected.logical_name))?
        }
        PointerSlotRestoreCasDecision::ReplayRecordedCas(recorded) => {
            let protection_after = reconcile_pointer_slot_protection(
                slot_address,
                terminalizing.protection_before_restore,
            )?;
            complete_recorded_restore_cas_observation(recorded, protection_after)
                .map_err(|error| format!("{} {error}", expected.logical_name))?
        }
    };
    // Against the protection observed when the terminalizing binding was
    // created, not against the record's own. KNOWN, DELIBERATE, and deliberately
    // kept -- identical to the install path; see
    // `verify_restore_cas_matches_lease_protection`.
    verify_restore_cas_matches_lease_protection(terminalizing.protection_before_restore, cas)
        .map_err(|error| format!("{} {error}", expected.logical_name))?;
    let observed_before = cas.observed_before;
    if observed_before != installed.external_shim_address {
        return Err(format!(
            "{} restore CAS expected shim 0x{:X}, observed 0x{observed_before:X}",
            expected.logical_name, installed.external_shim_address
        ));
    }
    // KNOWN, DELIBERATE, and deliberately kept -- the restore direction of the
    // property the install path states above its own retention check. If an
    // external writer puts the slot back to the shim between the exchange and
    // this load, the call fails here and the retry is refused as
    // `RecordedRestoreCasSlotDiverged` rather than driving a second exchange:
    // the record on hand describes an exchange whose result a third party has
    // since overwritten, so replaying it would attest a transition against
    // evidence from a different one, and re-exchanging under a replacement
    // record would drop the fact that custody of the slot was contested.
    let observed_after = atomic_load_pointer_slot(slot_address)?;
    if observed_after != installed.expected_original_address {
        return Err(format!(
            "{} import slot did not retain the original export after restore",
            expected.logical_name
        ));
    }
    let mut receipt = RestoredPointerSlotCustodyReceipt {
        logical_name: installed.logical_name,
        observed_module_name: installed.observed_module_name,
        observed_module_sha256: installed.observed_module_sha256,
        observed_pe_identity_sha256: installed.observed_pe_identity_sha256,
        observed_pe_timestamp: installed.observed_pe_timestamp,
        observed_size_of_image: installed.observed_size_of_image,
        provenance: installed.provenance,
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
        policy_id: installed.provenance.policy_id(),
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
    receipt.custody_digest_sha256 = restored_pointer_slot_custody_digest(&receipt);
    receipt.promotion_digest_sha256 = restored_pointer_slot_promotion_digest(&receipt);
    transition_sealed_shim_lease(
        sealed_shim,
        SealedShimLeasePhase::PointerSlotTerminalizing {
            teardown_owner_token: installed.teardown_owner_token,
            slot_rva: installed.slot_rva,
        },
        SealedShimLeasePhase::PointerSlotTerminal {
            teardown_owner_token: installed.teardown_owner_token,
            slot_rva: installed.slot_rva,
        },
        installed.expected_original_address,
        false,
    )?;
    registry.insert(
        key,
        PointerSlotCustodyState::Terminal(TerminalPointerSlotBinding {
            installed: installed.clone(),
            receipt: receipt.clone(),
        }),
    );
    Ok(receipt)
}

#[cfg(windows)]
pub fn verify_restored_pointer_slot_custody(
    installed: &InstalledPointerSlotReceipt,
    receipt: &RestoredPointerSlotCustodyReceipt,
) -> Result<(), String> {
    validate_terminal_receipt_binding(installed, receipt)?;
    if restored_pointer_slot_custody_digest(receipt) != receipt.custody_digest_sha256 {
        return Err(format!(
            "{} terminal import-slot custody digest mismatch",
            receipt.logical_name
        ));
    }
    if restored_pointer_slot_promotion_digest(receipt) != receipt.promotion_digest_sha256 {
        return Err(format!(
            "{} terminal import-slot promotion digest mismatch",
            receipt.logical_name
        ));
    }
    let registry = POINTER_SLOT_CUSTODY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let registry = registry
        .lock()
        .map_err(|_| "pointer-slot custody registry is poisoned")?;
    let key = (installed.teardown_owner_token, installed.slot_rva);
    match registry.get(&key) {
        Some(PointerSlotCustodyState::Terminal(binding))
            if binding.installed == *installed && binding.receipt == *receipt => {}
        _ => {
            return Err(format!(
                "{} is not the crate-owned terminal import-slot custody",
                receipt.logical_name
            ));
        }
    }
    validate_terminal_pointer_slot_live(receipt)
}

pub fn restored_pointer_slot_custody_digest(
    receipt: &RestoredPointerSlotCustodyReceipt,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.pointer_slot_terminal.v3");
    digest_len_prefixed(&mut digest, receipt.logical_name.as_bytes());
    digest.update(receipt.observed_module_sha256);
    digest.update(receipt.observed_pe_identity_sha256);
    digest.update(receipt.observed_pe_timestamp.to_le_bytes());
    digest.update(receipt.observed_size_of_image.to_le_bytes());
    digest_slot_provenance(&mut digest, receipt.provenance);
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

pub fn restored_pointer_slot_promotion_digest(
    receipt: &RestoredPointerSlotCustodyReceipt,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.pointer_slot_promotion.v3");
    digest_len_prefixed(&mut digest, receipt.logical_name.as_bytes());
    digest.update(receipt.observed_module_sha256);
    digest.update(receipt.observed_pe_identity_sha256);
    digest.update(receipt.observed_pe_timestamp.to_le_bytes());
    digest.update(receipt.observed_size_of_image.to_le_bytes());
    digest_slot_provenance(&mut digest, receipt.provenance);
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

fn validate_expected_pointer_slot(expected: ExpectedPointerSlot) -> Result<(), String> {
    if expected.logical_name.is_empty()
        || expected.slot_rva == 0
        || expected.slot_rva % ENGINE_OWNED_SLOT_WIDTH_BYTES != 0
        || expected.expected_original_address == 0
    {
        return Err(format!(
            "{} pointer-slot expected specification is not admitted",
            expected.logical_name
        ));
    }
    match expected.provenance {
        SlotProvenance::Pe64Import {
            imported_dll,
            imported_symbol,
        } => {
            if imported_dll.is_empty()
                || imported_symbol.is_empty()
                || !imported_dll.is_ascii()
                || !imported_symbol.is_ascii()
            {
                return Err("PE64 import-slot expected specification is not admitted".into());
            }
        }
        SlotProvenance::EngineOwnedFunctionPointer(attestation) => {
            // Reaching here at all means the attestation came from a roster
            // admission: `EngineOwnedSlotAttestation` has no public
            // constructor. These are the checks that still have content --
            // that the specification's slot address was not edited away from
            // the owner the attestation carries after the fact.
            if attestation.enumeration_sha256() == [0; 32]
                || attestation.roster_sha256() == [0; 32]
                || attestation.roster_slot_count() == 0
                || attestation.owner().derived_slot_rva() != Some(expected.slot_rva)
                || attestation.owner().entry_index >= attestation.owner().entry_count
            {
                return Err(format!(
                    "{} engine-owned slot specification is not roster-admitted",
                    expected.logical_name
                ));
            }
        }
    }
    Ok(())
}

fn digest_slot_provenance(digest: &mut Sha256, provenance: SlotProvenance) {
    match provenance {
        SlotProvenance::Pe64Import {
            imported_dll,
            imported_symbol,
        } => {
            digest.update([1u8]);
            digest_len_prefixed(digest, imported_dll.as_bytes());
            digest_len_prefixed(digest, imported_symbol.as_bytes());
        }
        SlotProvenance::EngineOwnedFunctionPointer(attestation) => {
            digest.update([2u8]);
            digest.update([match attestation.owner().kind {
                OwningStructureKind::Vtable => 1u8,
                OwningStructureKind::FunctionPointerTable => 2u8,
            }]);
            digest.update(attestation.owner().base_rva.to_le_bytes());
            digest.update(attestation.owner().entry_count.to_le_bytes());
            digest.update(attestation.owner().entry_index.to_le_bytes());
            digest.update(attestation.enumeration_sha256());
            digest.update(attestation.roster_sha256());
            digest.update((attestation.roster_slot_count() as u64).to_le_bytes());
        }
    }
}

fn pointer_slot_spec_sha256(expected: ExpectedPointerSlot) -> [u8; 32] {
    let mut digest = Sha256::new();
    // Domain-separated per provenance, and `digest_slot_provenance` also
    // prepends a variant tag, so no engine-owned specification can collide with
    // an import specification however its fields are chosen. Both digests are
    // process-local receipt content computed by this encoder on every call --
    // no admitted value is transcribed anywhere against them.
    match expected.provenance {
        SlotProvenance::Pe64Import { .. } => {
            digest.update(b"hoi4.production_hook.pe64_import_slot_spec.v1")
        }
        SlotProvenance::EngineOwnedFunctionPointer(_) => {
            digest.update(b"hoi4.production_hook.engine_owned_function_pointer_slot.v1")
        }
    }
    digest_len_prefixed(&mut digest, expected.logical_name.as_bytes());
    digest.update(expected.slot_rva.to_le_bytes());
    digest_slot_provenance(&mut digest, expected.provenance);
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
fn validate_installed_pointer_slot_binding(
    expected: ExpectedPointerSlot,
    receipt: &InstalledPointerSlotReceipt,
) -> Result<(), String> {
    if receipt.logical_name != expected.logical_name
        || receipt.observed_module_name != BATTLE_PLAN_MODULE_NAME
        || receipt.observed_module_sha256 != A729_EXE_SHA256
        || receipt.provenance != expected.provenance
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
        || receipt.primitive_id != expected.provenance.primitive_id()
        || receipt.slot_spec_sha256 != pointer_slot_spec_sha256(expected)
        || receipt.teardown_owner_token == 0
        || !receipt.installed
        || receipt.authenticity_seal == 0
    {
        return Err(format!(
            "{} installed pointer-slot receipt is invalid",
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
    installed: &InstalledPointerSlotReceipt,
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
fn validate_pointer_slot_terminal_permit(
    installed: &InstalledPointerSlotReceipt,
    sealed_shim: &SealedExternalDispatchShim,
    permit: &PointerSlotTerminalPermit,
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
        SealedShimLeasePhase::PointerSlotTerminalizing {
            teardown_owner_token: installed.teardown_owner_token,
            slot_rva: installed.slot_rva,
        },
        installed.expected_original_address,
        false,
    )
}

#[cfg(windows)]
fn validate_terminal_receipt_binding(
    installed: &InstalledPointerSlotReceipt,
    receipt: &RestoredPointerSlotCustodyReceipt,
) -> Result<(), String> {
    if receipt.logical_name != installed.logical_name
        || receipt.observed_module_name != installed.observed_module_name
        || receipt.observed_module_sha256 != installed.observed_module_sha256
        || receipt.observed_pe_identity_sha256 != installed.observed_pe_identity_sha256
        || receipt.observed_pe_timestamp != installed.observed_pe_timestamp
        || receipt.observed_size_of_image != installed.observed_size_of_image
        || receipt.provenance != installed.provenance
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
        || receipt.policy_id != installed.provenance.policy_id()
        || receipt.external_shim.entry_address != installed.external_shim_address
        || receipt.observed_terminal_dispatch_address != installed.expected_original_address
        || receipt.observed_active_callback_count != 0
        || receipt.authenticity_seal == 0
    {
        return Err(format!(
            "{} terminal pointer-slot receipt is invalid",
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
fn validate_terminal_pointer_slot_live(
    receipt: &RestoredPointerSlotCustodyReceipt,
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
        SealedShimLeasePhase::PointerSlotTerminal {
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
    /// `VirtualSize`: the section's extent once loaded. Not interchangeable
    /// with `raw_size`, which is the initialized on-disk size and may be larger
    /// after file alignment padding or smaller before zero-fill. Containment in
    /// the loaded image is a question about this field.
    virtual_size: usize,
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

/// Resolve one admitted slot to a process-local address, proving its
/// provenance against the loaded image first.
#[cfg(windows)]
fn validate_pointer_slot(
    module: &ObservedMainModule,
    expected: ExpectedPointerSlot,
) -> Result<usize, String> {
    match expected.provenance {
        SlotProvenance::Pe64Import {
            imported_dll,
            imported_symbol,
        } => validate_pe64_import_slot(module, expected, imported_dll, imported_symbol),
        SlotProvenance::EngineOwnedFunctionPointer(attestation) => {
            validate_engine_owned_pointer_slot(module, expected, attestation.owner())
        }
    }
}

/// Prove an engine-owned function-pointer or vtable slot from the image alone.
///
/// There is no thunk to name the target, so every fact a receipt will carry is
/// re-derived here from the pinned file image and the live mapping:
///
/// * the slot is naturally aligned and inside an initialized, file-backed
///   section of this module;
/// * the slot is **not** inside the PE import directory or any import address
///   table, so an engine-owned receipt can never be minted over a real import
///   (and, symmetrically, an import can never be laundered as engine-owned);
/// * the file image's own static initializer at the slot already holds the
///   declared target, which is what makes the owning-structure claim checkable
///   rather than asserted;
/// * the target RVA lies inside an executable section;
/// * the live mapped slot still holds exactly `expected_original_address`.
#[cfg(windows)]
fn validate_engine_owned_pointer_slot(
    module: &ObservedMainModule,
    expected: ExpectedPointerSlot,
    owner: OwningStructureIdentity,
) -> Result<usize, String> {
    if owner.derived_slot_rva() != Some(expected.slot_rva) {
        return Err(format!(
            "{} engine-owned slot is not its owning structure's declared entry",
            expected.logical_name
        ));
    }
    let target_rva = expected
        .expected_original_address
        .checked_sub(module.base)
        .ok_or_else(|| {
            format!(
                "{} engine-owned slot target is below the module base",
                expected.logical_name
            )
        })? as u64;
    if !module_rva_is_executable(module, target_rva) {
        return Err(format!(
            "{} engine-owned slot target RVA is not inside an executable section",
            expected.logical_name
        ));
    }
    reject_pe64_import_table_overlap(module, expected)?;
    let file_slot = module_file_bytes_at_rva(module, expected.slot_rva as usize, size_of::<u64>())?;
    let file_value = u64::from_le_bytes(file_slot.try_into().unwrap());
    let expected_file_value = file_preferred_image_base(module)?
        .checked_add(target_rva)
        .ok_or("engine-owned slot file value overflow")?;
    if file_value != expected_file_value {
        return Err(format!(
            "{} engine-owned slot file image holds 0x{file_value:X}, not 0x{expected_file_value:X}",
            expected.logical_name
        ));
    }
    // The owning structure has to be a real, wholly in-image structure too: a
    // base plus an entry count that runs off the end of the section is not an
    // identity anything can be reviewed against.
    let owner_span = u64::from(owner.entry_count)
        .checked_mul(ENGINE_OWNED_SLOT_WIDTH_BYTES)
        .ok_or("engine-owned slot owning structure span overflow")?;
    let owner_span = usize::try_from(owner_span)
        .map_err(|_| "engine-owned slot owning structure span does not fit usize")?;
    module_file_bytes_at_rva(module, owner.base_rva as usize, owner_span)?;
    let slot_address = checked_image_address(module, expected.slot_rva, size_of::<usize>())?;
    if slot_address % std::mem::align_of::<usize>() != 0 {
        return Err(format!(
            "{} engine-owned slot is not naturally pointer-aligned",
            expected.logical_name
        ));
    }
    // No live-slot comparison here, deliberately. `perform_recorded_cas`
    // records the exchange before its fallible page-protection cleanup, so an
    // install can legitimately fail *after* the slot already holds the shim;
    // recovery is a retry that replays the recorded exchange. A pre-check
    // demanding the slot still read the original would reject exactly those
    // retries before `decide_pointer_slot_install_cas` ever sees them, turning
    // a recoverable partial install into a redirection with no obtainable
    // receipt and therefore no teardown path. Whether the live slot is
    // acceptable is the custody state machine's question, and it distinguishes
    // a replayable record from an unwitnessed foreign write -- which this
    // check could not. The PE64 import path likewise leaves it alone.
    Ok(slot_address)
}

/// True when `rva` lands inside a section carrying `IMAGE_SCN_MEM_EXECUTE`.
#[cfg(windows)]
fn module_rva_is_executable(module: &ObservedMainModule, rva: u64) -> bool {
    rva_is_in_executable_section(&module.sections, rva)
}

#[cfg(windows)]
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;

/// Containment against the section's *loaded* extent.
///
/// `VirtualSize` and `SizeOfRawData` are different quantities and neither
/// bounds the other: file alignment pads the raw size upwards, `.bss`-style
/// zero-fill makes the virtual size the larger one. Only `VirtualSize`
/// describes what is mapped, so only it answers "is this RVA inside the
/// section". In a729's `.text` the raw size is 0x1D8 bytes longer than the
/// virtual size, so using it would accept RVAs past the end of the section.
#[cfg(windows)]
fn rva_is_in_executable_section(sections: &[PeSection], rva: u64) -> bool {
    let rva = match usize::try_from(rva) {
        Ok(value) => value,
        Err(_) => return false,
    };
    sections.iter().any(|section| {
        section.characteristics & IMAGE_SCN_MEM_EXECUTE != 0
            && rva >= section.virtual_address
            && rva < section.virtual_address.saturating_add(section.virtual_size)
    })
}

/// The image base the file's own optional header prefers.
///
/// The static initializer stored in an engine-owned slot is an absolute address
/// relative to *this* base, not to wherever the loader mapped the module, so a
/// file-image check has to rebase against it.
#[cfg(windows)]
fn file_preferred_image_base(module: &ObservedMainModule) -> Result<u64, String> {
    let pe_offset = read_u32(&module.file_bytes, 0x3c, "file PE header offset")? as usize;
    let optional = pe_offset
        .checked_add(24)
        .ok_or("PE64 optional-header offset overflow")?;
    let field = optional
        .checked_add(24)
        .ok_or("PE64 image base offset overflow")?;
    let bytes = module
        .file_bytes
        .get(field..field + 8)
        .ok_or("PE64 image base field is truncated")?;
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

/// Refuse an engine-owned admission whose slot lands in the import directory or
/// in any import address table.
///
/// This is the guard that keeps the two attestations from being confused. An
/// import slot has a loader-owned lifetime and a DLL/symbol identity that an
/// engine-owned receipt does not carry; redirecting one through the
/// engine-owned front would attest an owning structure that does not exist and
/// would omit the import identity that does.
#[cfg(windows)]
fn reject_pe64_import_table_overlap(
    module: &ObservedMainModule,
    expected: ExpectedPointerSlot,
) -> Result<(), String> {
    let slot_rva =
        usize::try_from(expected.slot_rva).map_err(|_| "engine-owned slot RVA does not fit usize")?;
    let slot_end = slot_rva
        .checked_add(size_of::<u64>())
        .ok_or("engine-owned slot RVA range overflow")?;
    let pe_offset = read_u32(&module.file_bytes, 0x3c, "file PE header offset")? as usize;
    let optional = pe_offset
        .checked_add(24)
        .ok_or("PE64 optional-header offset overflow")?;
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
    if import_directory_rva != 0
        && import_directory_size != 0
        && ranges_overlap(
            slot_rva,
            slot_end - slot_rva,
            import_directory_rva,
            import_directory_size,
        )?
    {
        return Err(format!(
            "{} engine-owned slot overlaps the PE64 import directory",
            expected.logical_name
        ));
    }
    let iat_rva = read_u32(&module.file_bytes, optional + 112 + 96, "PE64 IAT RVA")? as usize;
    let iat_size = read_u32(&module.file_bytes, optional + 112 + 100, "PE64 IAT size")? as usize;
    if iat_rva != 0
        && iat_size != 0
        && ranges_overlap(slot_rva, slot_end - slot_rva, iat_rva, iat_size)?
    {
        return Err(format!(
            "{} engine-owned slot overlaps the PE64 import address table",
            expected.logical_name
        ));
    }
    if import_directory_rva == 0 || import_directory_size < 20 {
        return Ok(());
    }
    // The IAT data directory is advisory: walk the descriptors and refuse an
    // overlap with any real first-thunk array as well.
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
        if first_thunk == 0 {
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
            let entry_rva = first_thunk
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
            if entry_rva == slot_rva {
                return Err(format!(
                    "{} engine-owned slot is a PE64 import thunk and must not be attested as engine-owned",
                    expected.logical_name
                ));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn validate_pe64_import_slot(
    module: &ObservedMainModule,
    expected: ExpectedPointerSlot,
    imported_dll: &'static str,
    imported_symbol: &'static str,
) -> Result<usize, String> {
    validate_file_pe64_import_slot(module, expected, imported_dll, imported_symbol)?;
    let slot_address = checked_image_address(module, expected.slot_rva, size_of::<usize>())?;
    if slot_address % std::mem::align_of::<usize>() != 0 {
        return Err(format!(
            "{} import slot is not naturally pointer-aligned",
            expected.logical_name
        ));
    }
    let mut dll_wide: Vec<u16> = imported_dll.encode_utf16().collect();
    dll_wide.push(0);
    let dll =
        unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(dll_wide.as_ptr()) };
    if dll.is_null() {
        return Err(format!(
            "{} imported DLL {} is not loaded",
            expected.logical_name, imported_dll
        ));
    }
    let symbol = std::ffi::CString::new(imported_symbol)
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
    expected: ExpectedPointerSlot,
    imported_dll: &'static str,
    imported_symbol: &'static str,
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
        if !dll.eq_ignore_ascii_case(imported_dll) {
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
            if symbol != imported_symbol {
                return Err(format!(
                    "{} import symbol mismatch: expected {}, observed {symbol}",
                    expected.logical_name, imported_symbol
                ));
            }
            return Ok(());
        }
    }
    Err(format!(
        "{} slot RVA is not the exact {}!{} PE64 import",
        expected.logical_name, imported_dll, imported_symbol
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

/// One atomic pointer-slot compare-exchange, as observed when it ran.
///
/// Recorded verbatim into the terminalizing binding so a retry can replay the
/// values this crate actually saw rather than reconstruct them from what the
/// caller expected to see.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtomicPointerCasObservation {
    pub observed_before: usize,
    pub protection_before: u32,
    pub protection_temporary: u32,
    pub protection_after: u32,
}

// ---------------------------------------------------------------------------
// PE64 import-slot custody decisions.
//
// These are deliberately pure: they take values the `#[cfg(windows)]` paths
// have already observed and return a decision or a typed refusal. The entry
// points that call them are gated behind `observe_main_module()` and cannot be
// driven from a test process; the decisions themselves are the part that has
// to be right, so they live here where they are directly exercisable.
// ---------------------------------------------------------------------------

/// Why a PE64 import-slot restore refused to attest a shim -> original
/// transition. Typed rather than stringly so a caller -- or a test -- can pin
/// *which* refusal fired instead of only that one did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerSlotRestoreCustodyError {
    /// The slot already reads the original export, but this crate never
    /// recorded a compare-exchange for it. Some agent outside this crate
    /// performed the transition (a competing hook engine unhooking, a shim
    /// that never installed, a rewritten slot). Custody is unresolvable: the
    /// crate cannot attest a transition it did not perform and must not adopt
    /// someone else's restore.
    UnwitnessedForeignRestore { observed_slot_value: usize },
    /// A prior attempt of this crate recorded a completed compare-exchange,
    /// but the slot no longer reads the original export that attempt left in
    /// it. The recorded observation no longer describes the live slot.
    RecordedRestoreCasSlotDiverged {
        observed_slot_value: usize,
        expected_original_address: usize,
    },
    /// The external shim and the original export share one address, so "the
    /// slot reads the shim" and "the slot reads the original" are the same
    /// observation and no transition is attestable in either direction.
    IndistinguishableShimAndOriginalAddress { address: usize },
    /// A recorded compare-exchange was completed, but the page protection it
    /// left behind has still not been observed back at the value the exchange
    /// started from. The transition happened; the custody of the page has not
    /// been resolved, so no receipt may form yet.
    RecordedRestoreProtectionUnreconciled {
        protection_before: u32,
        protection_after: u32,
    },
    /// The exchange ran but the record this crate holds for the slot is not the
    /// one it just performed. The write went somewhere other than the custody
    /// the receipt is minted against, so the evidence a retry needs is not
    /// where it has to be.
    RecordedRestoreCasNotRetained {
        retained: Option<RecordedRestoreCas>,
        performed: RecordedRestoreCas,
    },
    /// The page protection the exchange observed before it ran is not the
    /// protection observed when the lease over the slot was taken -- something
    /// outside this crate re-protected the page in between.
    RestoreCasProtectionBeforeDivergedFromLease {
        lease_protection: u32,
        cas_protection_before: u32,
    },
    /// The page protection observed after the exchange is not the protection
    /// observed when the lease was taken.
    RestoreCasProtectionAfterDivergedFromLease {
        lease_protection: u32,
        cas_protection_after: u32,
    },
}

impl std::fmt::Display for PointerSlotRestoreCustodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnwitnessedForeignRestore {
                observed_slot_value,
            } => write!(
                f,
                "import slot already reads 0x{observed_slot_value:X} with no restore CAS recorded by this crate; custody of the transition is unresolvable"
            ),
            Self::RecordedRestoreCasSlotDiverged {
                observed_slot_value,
                expected_original_address,
            } => write!(
                f,
                "recorded restore CAS expected the slot to retain original 0x{expected_original_address:X}, observed 0x{observed_slot_value:X}"
            ),
            Self::IndistinguishableShimAndOriginalAddress { address } => write!(
                f,
                "external shim and original export share address 0x{address:X}; no import-slot transition is attestable"
            ),
            Self::RecordedRestoreProtectionUnreconciled {
                protection_before,
                protection_after,
            } => write!(
                f,
                "recorded restore CAS left page protection at 0x{protection_after:X}, not the 0x{protection_before:X} it started from"
            ),
            Self::RecordedRestoreCasNotRetained {
                retained,
                performed,
            } => write!(
                f,
                "restore CAS custody did not retain the record the exchange produced: performed {performed:?}, custody holds {retained:?}"
            ),
            Self::RestoreCasProtectionBeforeDivergedFromLease {
                lease_protection,
                cas_protection_before,
            } => write!(
                f,
                "restore CAS observed page protection 0x{cas_protection_before:X} before the exchange; the lease was taken over 0x{lease_protection:X}"
            ),
            Self::RestoreCasProtectionAfterDivergedFromLease {
                lease_protection,
                cas_protection_after,
            } => write!(
                f,
                "restore CAS left page protection at 0x{cas_protection_after:X}; the lease was taken over 0x{lease_protection:X}"
            ),
        }
    }
}

/// What this crate recorded about a restore compare-exchange it performed.
///
/// Written the instant the hardware `compare_exchange` returns and *before* any
/// fallible page-protection cleanup, so a failure during that cleanup leaves
/// evidence of the transition behind instead of losing it.
/// `protection_reconciled` distinguishes "exchange done, page protection still
/// owed" from "exchange done and the page is back where it started"; only the
/// latter can form a receipt.
///
/// Constructible only inside [`crate::custody_record`], and there only by
/// performing an exchange. A synthesized `observed_before` -- the defect this
/// whole mechanism exists to prevent -- is a compile error outside that module
/// rather than something a test has to catch.
pub type RecordedRestoreCas = custody_record::RecordedCas<custody_record::Restore>;

/// The durable slot a restore record lives in, owned by the terminalizing
/// binding the custody registry holds.
pub type RestoreCasCustody = custody_record::CasCustody<custody_record::Restore>;

/// A restore record whose landing in the terminalizing binding has not been
/// checked yet. [`release_restore_cas_if_custody_matches`] turns one into a
/// [`RecordedRestoreCas`].
pub type UnconfirmedRestoreCas = custody_record::UnconfirmedCas<custody_record::Restore>;

/// Which arm the restore path takes for one import slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerSlotRestoreCasDecision {
    /// Drive a real compare-exchange and record what it returns.
    PerformCas,
    /// Replay the recorded exchange. Its page protection was already
    /// reconciled, so only the live protection has to be re-observed.
    ReplayRecordedCas(RecordedRestoreCas),
    /// Replay the recorded exchange, but finish the page-protection
    /// reconciliation the earlier attempt failed to complete before returning.
    /// This is the resumable state an exchange followed by a failed protection
    /// restore leaves behind.
    ReconcileProtectionThenReplay(RecordedRestoreCas),
}

/// Choose the restore arm from recorded custody, never from the slot alone.
///
/// `recorded_prior_cas` is `Some` only when this crate's own exchange returned
/// for this `(teardown_owner_token, slot_rva)`, so neither replay arm is
/// reachable without that evidence.
pub fn decide_pointer_slot_restore_cas(
    current_slot_value: usize,
    expected_original_address: usize,
    external_shim_address: usize,
    recorded_prior_cas: Option<RecordedRestoreCas>,
) -> Result<PointerSlotRestoreCasDecision, PointerSlotRestoreCustodyError> {
    if external_shim_address == expected_original_address {
        return Err(
            PointerSlotRestoreCustodyError::IndistinguishableShimAndOriginalAddress {
                address: external_shim_address,
            },
        );
    }
    match recorded_prior_cas {
        Some(recorded) => {
            if current_slot_value != expected_original_address {
                return Err(
                    PointerSlotRestoreCustodyError::RecordedRestoreCasSlotDiverged {
                        observed_slot_value: current_slot_value,
                        expected_original_address,
                    },
                );
            }
            if recorded.protection_reconciled() {
                Ok(PointerSlotRestoreCasDecision::ReplayRecordedCas(recorded))
            } else {
                Ok(PointerSlotRestoreCasDecision::ReconcileProtectionThenReplay(recorded))
            }
        }
        None => {
            if current_slot_value == expected_original_address {
                return Err(
                    PointerSlotRestoreCustodyError::UnwitnessedForeignRestore {
                        observed_slot_value: current_slot_value,
                    },
                );
            }
            Ok(PointerSlotRestoreCasDecision::PerformCas)
        }
    }
}

/// Release the record an exchange just produced, if the custody the caller
/// observed holds it.
///
/// `custody_record::perform_recorded_cas` writes through the `&mut` it is handed
/// and cannot tell whether that borrow points into the registry entry this
/// receipt will be minted from -- safe Rust has no way to state the provenance
/// of a `&mut`. So the call site re-reads the entry afterwards and passes what it
/// found here. What this establishes is only that the record matches the
/// `observed_custody` it was given; the value of that depends entirely on the
/// caller having re-read its real storage, which is why
/// [`confirm_terminalizing_restore_custody_retained`] does the re-read itself
/// rather than trusting a caller to.
///
/// A record that did not land is fail-closed either way -- the next attempt
/// finds no evidence and refuses the transition as unwitnessed -- so this turns
/// a silent liveness loss into a named refusal at the point it happens.
pub fn release_restore_cas_if_custody_matches(
    observed_custody: Option<RecordedRestoreCas>,
    performed: UnconfirmedRestoreCas,
) -> Result<RecordedRestoreCas, PointerSlotRestoreCustodyError> {
    performed
        .release_if_custody_matches(observed_custody)
        .map_err(|mismatch| {
            PointerSlotRestoreCustodyError::RecordedRestoreCasNotRetained {
                retained: mismatch.retained,
                performed: mismatch.performed,
            }
        })
}

/// Refuse unless the page protection the exchange saw, and left, is the
/// protection observed when the lease over this slot was taken.
///
/// Both comparisons are against the lease's observation, not the record's own.
/// KNOWN, DELIBERATE, and re-affirmed rather than changed: if an external agent
/// re-protects the page between the lease observation and the exchange, a
/// successful exchange records the newer value, this refuses, and no retry can
/// satisfy both this and [`complete_recorded_restore_cas_observation`] at once
/// -- the lease stays terminalizing for the process lifetime. That is a
/// liveness cost, never a forged receipt, and the alternative is worse: an
/// external re-protection of a page this crate holds custody of is exactly the
/// kind of event a custody receipt must not silently absorb. The typed variants
/// exist so an operator reading the log can tell that case from a cleanup that
/// failed to put the page back.
pub fn verify_restore_cas_matches_lease_protection(
    lease_protection: u32,
    cas: AtomicPointerCasObservation,
) -> Result<(), PointerSlotRestoreCustodyError> {
    if cas.protection_before != lease_protection {
        return Err(
            PointerSlotRestoreCustodyError::RestoreCasProtectionBeforeDivergedFromLease {
                lease_protection,
                cas_protection_before: cas.protection_before,
            },
        );
    }
    if cas.protection_after != lease_protection {
        return Err(
            PointerSlotRestoreCustodyError::RestoreCasProtectionAfterDivergedFromLease {
                lease_protection,
                cas_protection_after: cas.protection_after,
            },
        );
    }
    Ok(())
}

/// Render the message a timed-out quiescence reports.
///
/// This is the production renderer, not a test-side reconstruction of it: the
/// call site passes the outcome straight through, so every number in the text
/// is one the loop counted rather than one recomputed from `max_yields`. Kept
/// pure and separate precisely so a test can assert the *rendered* string and
/// therefore catch a call site that starts quoting the wrong field.
pub fn render_pointer_slot_quiesce_timeout(
    logical_name: &str,
    outcome: &Pe64ImportSlotQuiesceOutcome,
) -> String {
    format!(
        "{logical_name} collector callbacks did not quiesce: stopped on {} after {} yields and {} dispatch observations in {:?}; cutover custody retained",
        outcome.stopped_on, outcome.yields_performed, outcome.observations, outcome.elapsed
    )
}

/// Turn a recorded exchange plus the live page protection into the complete
/// observation a receipt is minted from.
///
/// Refuses unless the page is back at the protection the exchange started from,
/// so an unreconciled record can never reach a receipt no matter which arm
/// produced it.
pub fn complete_recorded_restore_cas_observation(
    recorded: RecordedRestoreCas,
    protection_after: u32,
) -> Result<AtomicPointerCasObservation, PointerSlotRestoreCustodyError> {
    if !recorded.protection_reconciled() || protection_after != recorded.protection_before() {
        return Err(
            PointerSlotRestoreCustodyError::RecordedRestoreProtectionUnreconciled {
                protection_before: recorded.protection_before(),
                protection_after,
            },
        );
    }
    Ok(AtomicPointerCasObservation {
        observed_before: recorded.observed_before(),
        protection_before: recorded.protection_before(),
        protection_temporary: recorded.protection_temporary(),
        protection_after,
    })
}

// ---------------------------------------------------------------------------
// PE64 import-slot install custody decisions.
//
// The install direction of the custody problem the restore helpers above solve,
// deliberately kept as its own record type, decision, storage trait and error
// rather than sharing theirs. The two directions swap the roles of
// `expected_original_address` and `external_shim_address`, so a record of one
// direction is not evidence for the other; keeping the types disjoint means the
// type system, not a convention, prevents one being replayed as the other.
// ---------------------------------------------------------------------------

/// Why a PE64 import-slot install refused to attest an original -> shim
/// transition. Typed rather than stringly so a caller -- or a test -- can pin
/// *which* refusal fired instead of only that one did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerSlotInstallCustodyError {
    /// The slot already reads the external shim, but this crate never recorded
    /// a compare-exchange for it. Whatever put that value there did so outside
    /// this crate's custody (a competing hook engine, a previous process
    /// lifetime's leftovers, a rewritten slot). Custody is unresolvable: the
    /// crate cannot attest an installation it did not perform, and an
    /// installation receipt is exactly that attestation.
    UnwitnessedForeignInstall { observed_slot_value: usize },
    /// A prior attempt of this crate recorded a completed compare-exchange, but
    /// the slot no longer reads the external shim that attempt left in it. The
    /// recorded observation no longer describes the live slot.
    RecordedInstallCasSlotDiverged {
        observed_slot_value: usize,
        external_shim_address: usize,
    },
    /// The external shim and the original export share one address, so "the
    /// slot reads the original" and "the slot reads the shim" are the same
    /// observation and no transition is attestable in either direction.
    IndistinguishableShimAndOriginalAddress { address: usize },
    /// A recorded compare-exchange was completed, but the page protection it
    /// left behind has still not been observed back at the value the exchange
    /// started from. The transition happened; the custody of the page has not
    /// been resolved, so no receipt may form yet.
    RecordedInstallProtectionUnreconciled {
        protection_before: u32,
        protection_after: u32,
    },
    /// The exchange ran but the record this crate holds for the slot is not the
    /// one it just performed. The write went somewhere other than the custody
    /// the receipt is minted against, so the evidence a retry needs is not
    /// where it has to be.
    RecordedInstallCasNotRetained {
        retained: Option<RecordedInstallCas>,
        performed: RecordedInstallCas,
    },
    /// The page protection the exchange observed before it ran is not the
    /// protection observed when the lease over the slot was taken -- something
    /// outside this crate re-protected the page in between.
    InstallCasProtectionBeforeDivergedFromLease {
        lease_protection: u32,
        cas_protection_before: u32,
    },
    /// The page protection observed after the exchange is not the protection
    /// observed when the lease was taken.
    InstallCasProtectionAfterDivergedFromLease {
        lease_protection: u32,
        cas_protection_after: u32,
    },
}

impl std::fmt::Display for PointerSlotInstallCustodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnwitnessedForeignInstall {
                observed_slot_value,
            } => write!(
                f,
                "import slot already reads 0x{observed_slot_value:X} with no install CAS recorded by this crate; custody of the transition is unresolvable"
            ),
            Self::RecordedInstallCasSlotDiverged {
                observed_slot_value,
                external_shim_address,
            } => write!(
                f,
                "recorded install CAS expected the slot to retain external shim 0x{external_shim_address:X}, observed 0x{observed_slot_value:X}"
            ),
            Self::IndistinguishableShimAndOriginalAddress { address } => write!(
                f,
                "external shim and original export share address 0x{address:X}; no import-slot transition is attestable"
            ),
            Self::RecordedInstallProtectionUnreconciled {
                protection_before,
                protection_after,
            } => write!(
                f,
                "recorded install CAS left page protection at 0x{protection_after:X}, not the 0x{protection_before:X} it started from"
            ),
            Self::RecordedInstallCasNotRetained {
                retained,
                performed,
            } => write!(
                f,
                "install CAS custody did not retain the record the exchange produced: performed {performed:?}, custody holds {retained:?}"
            ),
            Self::InstallCasProtectionBeforeDivergedFromLease {
                lease_protection,
                cas_protection_before,
            } => write!(
                f,
                "install CAS observed page protection 0x{cas_protection_before:X} before the exchange; the lease was taken over 0x{lease_protection:X}"
            ),
            Self::InstallCasProtectionAfterDivergedFromLease {
                lease_protection,
                cas_protection_after,
            } => write!(
                f,
                "install CAS left page protection at 0x{cas_protection_after:X}; the lease was taken over 0x{lease_protection:X}"
            ),
        }
    }
}

/// What this crate recorded about an install compare-exchange it performed.
///
/// Written the instant the hardware `compare_exchange` returns and *before* any
/// fallible page-protection cleanup, so a failure during that cleanup leaves
/// evidence of the transition behind instead of losing it.
/// `protection_reconciled` distinguishes "exchange done, page protection still
/// owed" from "exchange done and the page is back where it started"; only the
/// latter can form a receipt.
///
/// A distinct type from [`RecordedRestoreCas`], not an alias of it: the two
/// directions swap the roles of the original and the shim address, so a record
/// of one is not evidence for the other and the compiler refuses to substitute
/// them. Constructible only inside [`crate::custody_record`], and there only by
/// performing an exchange.
pub type RecordedInstallCas = custody_record::RecordedCas<custody_record::Install>;

/// The durable slot an install record lives in, owned by the provisional
/// binding the custody registry holds.
pub type InstallCasCustody = custody_record::CasCustody<custody_record::Install>;

/// An install record whose landing in the provisional binding has not been
/// checked yet. [`release_install_cas_if_custody_matches`] turns one into a
/// [`RecordedInstallCas`].
pub type UnconfirmedInstallCas = custody_record::UnconfirmedCas<custody_record::Install>;

/// Which arm the install path takes for one import slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerSlotInstallCasDecision {
    /// Drive a real compare-exchange and record what it returns.
    PerformCas,
    /// Replay the recorded exchange. Its page protection was already
    /// reconciled, so only the live protection has to be re-observed.
    ReplayRecordedCas(RecordedInstallCas),
    /// Replay the recorded exchange, but finish the page-protection
    /// reconciliation the earlier attempt failed to complete before returning.
    /// This is the resumable state an exchange followed by a failed protection
    /// restore leaves behind.
    ReconcileProtectionThenReplay(RecordedInstallCas),
}

/// Choose the install arm from recorded custody, never from the slot alone.
///
/// `recorded_prior_cas` is `Some` only when this crate's own exchange returned
/// for this `(teardown_owner_token, slot_rva)`, so neither replay arm is
/// reachable without that evidence. A first attempt that finds the slot already
/// reading the shim is refused outright rather than adopted: an installation
/// receipt attests that *this* invocation performed the original -> shim
/// transition, and there is nothing here to support that claim.
pub fn decide_pointer_slot_install_cas(
    current_slot_value: usize,
    expected_original_address: usize,
    external_shim_address: usize,
    recorded_prior_cas: Option<RecordedInstallCas>,
) -> Result<PointerSlotInstallCasDecision, PointerSlotInstallCustodyError> {
    if external_shim_address == expected_original_address {
        return Err(
            PointerSlotInstallCustodyError::IndistinguishableShimAndOriginalAddress {
                address: external_shim_address,
            },
        );
    }
    match recorded_prior_cas {
        Some(recorded) => {
            if current_slot_value != external_shim_address {
                return Err(
                    PointerSlotInstallCustodyError::RecordedInstallCasSlotDiverged {
                        observed_slot_value: current_slot_value,
                        external_shim_address,
                    },
                );
            }
            if recorded.protection_reconciled() {
                Ok(PointerSlotInstallCasDecision::ReplayRecordedCas(recorded))
            } else {
                Ok(PointerSlotInstallCasDecision::ReconcileProtectionThenReplay(recorded))
            }
        }
        None => {
            if current_slot_value == external_shim_address {
                return Err(
                    PointerSlotInstallCustodyError::UnwitnessedForeignInstall {
                        observed_slot_value: current_slot_value,
                    },
                );
            }
            Ok(PointerSlotInstallCasDecision::PerformCas)
        }
    }
}

/// Release the record an exchange just produced, if the custody the caller
/// observed holds it. The install direction of
/// [`release_restore_cas_if_custody_matches`]; see it for what this does and
/// does not establish.
pub fn release_install_cas_if_custody_matches(
    observed_custody: Option<RecordedInstallCas>,
    performed: UnconfirmedInstallCas,
) -> Result<RecordedInstallCas, PointerSlotInstallCustodyError> {
    performed
        .release_if_custody_matches(observed_custody)
        .map_err(|mismatch| {
            PointerSlotInstallCustodyError::RecordedInstallCasNotRetained {
                retained: mismatch.retained,
                performed: mismatch.performed,
            }
        })
}

/// Refuse unless the page protection the exchange saw, and left, is the
/// protection observed when the lease over this slot was taken.
///
/// The install direction of [`verify_restore_cas_matches_lease_protection`],
/// with the identical deliberate liveness cost. The two are kept in step by
/// being the same shape over the same fields.
pub fn verify_install_cas_matches_lease_protection(
    lease_protection: u32,
    cas: AtomicPointerCasObservation,
) -> Result<(), PointerSlotInstallCustodyError> {
    if cas.protection_before != lease_protection {
        return Err(
            PointerSlotInstallCustodyError::InstallCasProtectionBeforeDivergedFromLease {
                lease_protection,
                cas_protection_before: cas.protection_before,
            },
        );
    }
    if cas.protection_after != lease_protection {
        return Err(
            PointerSlotInstallCustodyError::InstallCasProtectionAfterDivergedFromLease {
                lease_protection,
                cas_protection_after: cas.protection_after,
            },
        );
    }
    Ok(())
}

/// Turn a recorded install exchange plus the live page protection into the
/// complete observation a receipt is minted from.
///
/// Refuses unless the page is back at the protection the exchange started from,
/// so an unreconciled record can never reach a receipt no matter which arm
/// produced it.
pub fn complete_recorded_install_cas_observation(
    recorded: RecordedInstallCas,
    protection_after: u32,
) -> Result<AtomicPointerCasObservation, PointerSlotInstallCustodyError> {
    if !recorded.protection_reconciled() || protection_after != recorded.protection_before() {
        return Err(
            PointerSlotInstallCustodyError::RecordedInstallProtectionUnreconciled {
                protection_before: recorded.protection_before(),
                protection_after,
            },
        );
    }
    Ok(AtomicPointerCasObservation {
        observed_before: recorded.observed_before(),
        protection_before: recorded.protection_before(),
        protection_temporary: recorded.protection_temporary(),
        protection_after,
    })
}

/// Wall-clock budget for the quiescence loop. **This is an operational policy
/// limit, not a derivation** -- nothing about the loop bounds how long a
/// collector callback takes to leave the shim.
///
/// What the loop waits for is callbacks already inside the sealed shim to
/// return. It cannot make that happen: `std::thread::yield_now` is
/// `SwitchToThread`, which offers a scheduling opportunity to a ready thread on
/// the current processor, guarantees no fairness, and does not imply any
/// callback completes; a callback that is blocked stays active across any
/// number of yields. So iterations bound only a proxy, and this budget bounds
/// the quantity that actually matters.
///
/// The policy the 2 seconds encodes: teardown runs on a caller thread and holds
/// the sealed-shim lease for this slot, so a stalled quiescence blocks any
/// later install for the same slot. Long enough that a callback merely
/// descheduled several times still finishes; short enough that a teardown never
/// reads as hung. Expiry is not data loss -- the function returns its resumable
/// timeout error with the dispatch already cut over to the original, so the
/// caller may re-drive it.
const POINTER_SLOT_QUIESCE_TIME_BUDGET_NANOS: u64 = 2_000_000_000;

/// See [`POINTER_SLOT_QUIESCE_TIME_BUDGET_NANOS`].
pub const POINTER_SLOT_QUIESCE_TIME_BUDGET: Duration =
    Duration::from_nanos(POINTER_SLOT_QUIESCE_TIME_BUDGET_NANOS);

/// Cost this policy assumes one loop iteration cannot go below: a `VirtualQuery`
/// plus a `SwitchToThread`. It is used only to size the iteration backstop below
/// so that the backstop sits above what the time budget can admit -- it is not a
/// claim about how long an iteration takes in practice.
const POINTER_SLOT_QUIESCE_ASSUMED_MIN_ITERATION_NANOS: u64 = 1_000;

/// Ceiling on the caller-supplied yield budget of
/// [`quiesce_pointer_slot`].
///
/// A **backstop**, not the operative bound. [`POINTER_SLOT_QUIESCE_TIME_BUDGET`]
/// is what stops a contended loop; this catches a caller asking for an absurd
/// iteration count and a clock that has stalled or gone backwards. It is derived
/// from the two policy numbers above -- the iterations the time budget could
/// admit if every one of them cost only the assumed minimum -- so it sits above
/// the clock rather than pre-empting it, and it moves when the budget moves.
pub const MAX_POINTER_SLOT_QUIESCE_YIELDS: u32 = {
    let iterations = POINTER_SLOT_QUIESCE_TIME_BUDGET_NANOS
        / POINTER_SLOT_QUIESCE_ASSUMED_MIN_ITERATION_NANOS;
    if iterations > u32::MAX as u64 {
        u32::MAX
    } else {
        iterations as u32
    }
};

/// Why a quiescence yield budget was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pe64ImportSlotQuiesceBoundError {
    YieldBudgetAboveCeiling { requested: u32, ceiling: u32 },
}

impl std::fmt::Display for Pe64ImportSlotQuiesceBoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::YieldBudgetAboveCeiling { requested, ceiling } => write!(
                f,
                "quiescence yield budget {requested} exceeds the {ceiling} ceiling"
            ),
        }
    }
}

/// Validate a caller-supplied quiescence yield budget against the ceiling.
pub fn validate_pointer_slot_quiesce_yields(
    max_yields: u32,
) -> Result<u32, Pe64ImportSlotQuiesceBoundError> {
    if max_yields > MAX_POINTER_SLOT_QUIESCE_YIELDS {
        return Err(Pe64ImportSlotQuiesceBoundError::YieldBudgetAboveCeiling {
            requested: max_yields,
            ceiling: MAX_POINTER_SLOT_QUIESCE_YIELDS,
        });
    }
    Ok(max_yields)
}

/// Why the quiescence loop stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pe64ImportSlotQuiesceStop {
    /// An observation returned an active callback count of zero.
    Quiesced,
    /// The caller-supplied yield budget ran out.
    YieldBudgetExhausted,
    /// [`POINTER_SLOT_QUIESCE_TIME_BUDGET`] ran out.
    TimeBudgetExhausted,
}

impl std::fmt::Display for Pe64ImportSlotQuiesceStop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Quiesced => write!(f, "a zero active-callback observation"),
            Self::YieldBudgetExhausted => write!(f, "the yield budget"),
            Self::TimeBudgetExhausted => write!(f, "the time budget"),
        }
    }
}

/// What the quiescence loop actually did, counted by the loop itself.
///
/// The timeout message quotes these fields rather than recomputing them from
/// `max_yields`, so the message cannot drift from the loop's behaviour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pe64ImportSlotQuiesceOutcome {
    pub stopped_on: Pe64ImportSlotQuiesceStop,
    pub observations: u64,
    pub yields_performed: u32,
    pub elapsed: Duration,
}

impl Pe64ImportSlotQuiesceOutcome {
    pub fn observed_zero(&self) -> bool {
        matches!(self.stopped_on, Pe64ImportSlotQuiesceStop::Quiesced)
    }
}

/// The quiescence loop itself.
///
/// This is the loop `quiesce_pointer_slot` runs -- the observation,
/// the yield, and both bounds are here, not duplicated at the call site -- so a
/// test that drives it with its own closures exercises the real control flow.
/// The `#[cfg(windows)]` caller supplies `observe_active_callback_count`,
/// `yield_now` and `elapsed`; nothing else about the loop is platform-specific.
///
/// One observation precedes any yield and one follows each yield, so a run that
/// stops on the yield budget has performed `max_yields` yields and
/// `max_yields + 1` observations. The time budget is checked after an
/// observation shows callbacks still active and before the yield that would
/// follow it, so an already-quiesced slot is never failed for lateness.
pub fn drive_pointer_slot_quiesce_loop<E>(
    max_yields: u32,
    time_budget: Duration,
    mut observe_active_callback_count: impl FnMut() -> Result<u64, E>,
    mut yield_now: impl FnMut(),
    mut elapsed: impl FnMut() -> Duration,
) -> Result<Pe64ImportSlotQuiesceOutcome, E> {
    let mut observations: u64 = 0;
    let mut yields_performed: u32 = 0;
    loop {
        let active_count = observe_active_callback_count()?;
        observations += 1;
        if active_count == 0 {
            return Ok(Pe64ImportSlotQuiesceOutcome {
                stopped_on: Pe64ImportSlotQuiesceStop::Quiesced,
                observations,
                yields_performed,
                elapsed: elapsed(),
            });
        }
        if yields_performed >= max_yields {
            return Ok(Pe64ImportSlotQuiesceOutcome {
                stopped_on: Pe64ImportSlotQuiesceStop::YieldBudgetExhausted,
                observations,
                yields_performed,
                elapsed: elapsed(),
            });
        }
        let spent = elapsed();
        if spent >= time_budget {
            return Ok(Pe64ImportSlotQuiesceOutcome {
                stopped_on: Pe64ImportSlotQuiesceStop::TimeBudgetExhausted,
                observations,
                yields_performed,
                elapsed: spent,
            });
        }
        yield_now();
        yields_performed += 1;
    }
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

/// A pointer-slot compare-exchange that has been prepared but not yet performed.
///
/// Splitting the operation into prepare / exchange / finish is what lets the
/// restore path persist the exchange's result with no fallible call standing
/// between the two: everything that can fail is either before the exchange
/// (`begin_`) or after the record has been written (`finish_`).
#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PreparedPointerSlotCas {
    address: usize,
    protection_before: u32,
    protection_temporary: u32,
    was_writable: bool,
}

/// Everything fallible that must happen *before* the exchange: validate the
/// slot memory, observe the page protection, and make the page writable.
#[cfg(windows)]
fn begin_atomic_compare_exchange_pointer_slot(
    address: usize,
) -> Result<PreparedPointerSlotCas, String> {
    use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_READWRITE};
    validate_atomic_pointer_slot_memory(address)?;
    let protection_before = pointer_slot_protection(address)?;
    let was_writable = pointer_protection_is_writable(protection_before);
    let protection_temporary = pointer_slot_temporary_protection(protection_before);
    if !was_writable {
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
    Ok(PreparedPointerSlotCas {
        address,
        protection_before,
        protection_temporary,
        was_writable,
    })
}

/// The exchange itself. Infallible by construction: the caller must be able to
/// persist what it returns without a `?` in between.
#[cfg(windows)]
fn exchange_pointer_slot(
    prepared: &PreparedPointerSlotCas,
    expected: usize,
    replacement: usize,
) -> usize {
    let slot = unsafe { &*(prepared.address as *const AtomicUsize) };
    let observed_before =
        match slot.compare_exchange(expected, replacement, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(previous) | Err(previous) => previous,
        };
    unsafe { windows_sys::Win32::System::Threading::FlushProcessWriteBuffers() };
    observed_before
}

/// Everything fallible that must happen *after* the exchange: put the page
/// protection back and confirm it landed. Returns the observed protection.
#[cfg(windows)]
fn finish_atomic_compare_exchange_pointer_slot(
    prepared: PreparedPointerSlotCas,
) -> Result<u32, String> {
    use windows_sys::Win32::System::Memory::VirtualProtect;
    if !prepared.was_writable {
        let mut temporary = 0u32;
        let restored = unsafe {
            VirtualProtect(
                prepared.address as *const core::ffi::c_void,
                size_of::<usize>(),
                prepared.protection_before,
                &mut temporary,
            )
        };
        if restored == 0 || temporary != prepared.protection_temporary {
            return Err(
                "atomic pointer slot page protection restoration failed; custody unresolved".into(),
            );
        }
    }
    let protection_after = pointer_slot_protection(prepared.address)?;
    if protection_after != prepared.protection_before {
        return Err("atomic pointer slot page protection was not exactly restored".into());
    }
    Ok(protection_after)
}

// The "prepare, exchange and finish in one call" wrapper that used to live here
// was deleted with the install path's last non-recording caller. Both remaining
// compare-exchange call sites have to persist the exchange result before the
// fallible cleanup runs, which is precisely what the wrapper could not do; a
// second sequencing of the same three steps is a drift hazard with no user.

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

/// Put a slot's page protection back at `expected` if it has drifted, then
/// report what it actually reads. Used by the replay arms of both the install
/// and the restore path -- the operation is the same in either direction -- so
/// a retry finishes the cleanup a failed attempt left owed.
#[cfg(windows)]
fn reconcile_pointer_slot_protection(
    address: usize,
    expected: u32,
) -> Result<u32, String> {
    let observed = pointer_slot_protection(address)?;
    if observed != expected {
        restore_pointer_slot_protection(address, observed, expected)?;
    }
    pointer_slot_protection(address)
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
            CanonicalCountedShimTemplate::Win64RegisterArgsOnly
        }
        D3D11_CREATE_DEVICE_COUNTED_SHIM_TEMPLATE_ID => {
            CanonicalCountedShimTemplate::D3D11CreateDevice
        }
        D3D_COMPILE_COUNTED_SHIM_TEMPLATE_ID => CanonicalCountedShimTemplate::D3DCompile,
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
        let virtual_size = read_u32(bytes, offset + 8, "PE section virtual size")? as usize;
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
            virtual_size,
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

#[cfg(test)]
mod pe64_import_slot_custody_decision_tests {
    use super::*;
    use std::time::Duration;

    /// Stand-in addresses. They only have to be distinct and non-zero: the
    /// decision helper compares them, it never dereferences them.
    const ORIGINAL: usize = 0x7FFA_1111_0000;
    const SHIM: usize = 0x0000_2222_3000;
    const FOREIGN: usize = 0x0000_3333_4000;

    /// `observed_before` here is deliberately *not* `SHIM`. The pre-fix code
    /// synthesized `observed_before` as `installed.external_shim_address`, so
    /// any test whose recorded value equals `SHIM` cannot tell a replay from a
    /// reconstruction. This sentinel can.
    const RECORDED_SENTINEL_BEFORE: usize = 0x0000_5555_6000;

    /// Page protections. Distinct so a test can tell which field was replayed.
    const PROTECTION_BEFORE: u32 = 0x11;
    const PROTECTION_TEMPORARY: u32 = 0x22;

    /// A recorded exchange whose page cleanup completed.
    ///
    /// Built by actually driving `custody_record::perform_recorded_cas`, because
    /// there is no other way: `RecordedRestoreCas` has private fields and no
    /// constructor outside `custody_record`, so a test cannot hand-assemble one
    /// any more than production can. That is what this lane changed -- the
    /// forged record these helpers used to be written as is now a compile error.
    fn recorded_cas() -> RecordedRestoreCas {
        let mut custody = RestoreCasCustody::unrecorded();
        let (performed, _) = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Ok(PROTECTION_BEFORE),
        )
        .expect("an injected cleanup returning the starting protection cannot fail");
        performed
            .release_if_custody_matches(custody.recorded())
            .expect("the helper writes into the cell it was handed")
    }

    /// A recorded exchange that completed but whose page cleanup then failed --
    /// the state Finding 1 is about. Read back out of the custody cell, because
    /// that path returns `Err` rather than a record.
    fn recorded_cas_pending_protection() -> RecordedRestoreCas {
        let mut custody = RestoreCasCustody::unrecorded();
        let outcome = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Err("injected page protection restoration failure"),
        );
        assert_eq!(outcome, Err("injected page protection restoration failure"));
        custody
            .recorded()
            .expect("the exchange is recorded before the cleanup runs")
    }

    // ---- Defect A: restore arm selection --------------------------------

    // FALSIFIABILITY: delete the `current_slot_value == expected_original_address`
    // refusal in the `None` branch of `decide_pointer_slot_restore_cas` --
    // i.e. restore the pre-fix behaviour where an already-original slot silently
    // took the synthesized-observation arm -- and this test fails: the call
    // returns `Ok(PerformCas)` instead of `Err(UnwitnessedForeignRestore)`.
    //
    // This is the forged-attestation case. On a first attempt the crate has
    // recorded no CAS; if the slot already reads the original then some agent
    // outside this crate performed the transition and no receipt may be minted.
    #[test]
    fn first_attempt_with_slot_already_original_and_no_record_is_refused() {
        let outcome = decide_pointer_slot_restore_cas(ORIGINAL, ORIGINAL, SHIM, None);
        assert_eq!(
            outcome,
            Err(
                PointerSlotRestoreCustodyError::UnwitnessedForeignRestore {
                    observed_slot_value: ORIGINAL
                }
            )
        );
    }

    // FALSIFIABILITY: make the `None` branch refuse unconditionally, or invert
    // its comparison to `current_slot_value != expected_original_address`, and
    // this test fails: the genuine first attempt no longer reaches `PerformCas`.
    // This is the negative arm of the test above -- without it the refusal could
    // be a blanket rejection and still look correct.
    #[test]
    fn first_attempt_with_slot_holding_the_shim_performs_a_real_cas() {
        let outcome = decide_pointer_slot_restore_cas(SHIM, ORIGINAL, SHIM, None);
        assert_eq!(outcome, Ok(PointerSlotRestoreCasDecision::PerformCas));
    }

    // FALSIFIABILITY: change the `Some(recorded)` branch to rebuild the
    // observation from the caller-supplied addresses -- e.g. return
    // `ReplayRecordedCas(AtomicPointerCasObservation { observed_before:
    // external_shim_address, .. })`, which is exactly what the pre-fix code did
    // -- and this test fails on `observed_before`, because the replayed value
    // would become `SHIM` instead of the recorded sentinel.
    //
    // This is property A2: the retry arm reports what the earlier CAS actually
    // returned, never a value reconstructed from `installed`.
    #[test]
    fn retry_replays_the_recorded_observation_verbatim() {
        // The sentinel is load-bearing test data, so pin it here. Set it equal
        // to `SHIM` and the assertion below passes against the pre-fix
        // reconstruction as well -- the test would go vacuous with nothing
        // going red anywhere.
        assert_ne!(RECORDED_SENTINEL_BEFORE, SHIM);
        assert_ne!(RECORDED_SENTINEL_BEFORE, ORIGINAL);
        assert_ne!(SHIM, ORIGINAL);

        let recorded = recorded_cas();
        let outcome = decide_pointer_slot_restore_cas(ORIGINAL, ORIGINAL, SHIM, Some(recorded));
        // Field by field rather than against `recorded_cas()`: comparing the
        // decision to the same helper that produced its input would pass for any
        // implementation that echoes its argument, including one that rebuilt the
        // record from the addresses. These four values are the ones a receipt
        // copies.
        let Ok(PointerSlotRestoreCasDecision::ReplayRecordedCas(replayed)) = outcome else {
            panic!("a reconciled record must select the plain replay arm: {outcome:?}");
        };
        assert_eq!(replayed.observed_before(), RECORDED_SENTINEL_BEFORE);
        assert_eq!(replayed.protection_before(), PROTECTION_BEFORE);
        assert_eq!(replayed.protection_temporary(), PROTECTION_TEMPORARY);
        assert!(replayed.protection_reconciled());
        // ...and the value that reaches the receipt is that same recorded one.
        assert_eq!(
            complete_recorded_restore_cas_observation(recorded, PROTECTION_BEFORE),
            Ok(AtomicPointerCasObservation {
                observed_before: RECORDED_SENTINEL_BEFORE,
                protection_before: PROTECTION_BEFORE,
                protection_temporary: PROTECTION_TEMPORARY,
                protection_after: PROTECTION_BEFORE,
            })
        );
    }

    // ---- Finding 1: a completed exchange must survive a failed cleanup ----

    // FALSIFIABILITY: make the `Some(recorded)` branch return `ReplayRecordedCas`
    // regardless of `protection_reconciled` -- i.e. collapse the two replay
    // variants back into one -- and this test fails. It is the sequencing
    // decision for the state a successful exchange followed by a failed page
    // cleanup leaves behind: the exchange happened and must be replayed, but the
    // page cleanup is still owed and has to run before a receipt can form.
    #[test]
    fn retry_after_a_cleanup_failure_resumes_the_protection_reconciliation() {
        let outcome = decide_pointer_slot_restore_cas(
            ORIGINAL,
            ORIGINAL,
            SHIM,
            Some(recorded_cas_pending_protection()),
        );
        assert_eq!(
            outcome,
            Ok(
                PointerSlotRestoreCasDecision::ReconcileProtectionThenReplay(
                    recorded_cas_pending_protection()
                )
            )
        );
    }

    // FALSIFIABILITY: delete the record-back between `exchange_pointer_slot` and
    // `finish_atomic_compare_exchange_pointer_slot` in the `PerformCas` arm --
    // i.e. go back to recording only after the fallible cleanup returned -- and
    // the state this test describes becomes `None` instead, which
    // `first_attempt_with_slot_already_original_and_no_record_is_refused` shows
    // is refused as `UnwitnessedForeignRestore`. This test names the outcome
    // that refusal would wrongly produce for a caller who genuinely did perform
    // the transition.
    #[test]
    fn a_recorded_exchange_is_never_mistaken_for_a_foreign_restore() {
        for recorded in [recorded_cas(), recorded_cas_pending_protection()] {
            let outcome =
                decide_pointer_slot_restore_cas(ORIGINAL, ORIGINAL, SHIM, Some(recorded));
            assert!(
                !matches!(
                    outcome,
                    Err(PointerSlotRestoreCustodyError::UnwitnessedForeignRestore { .. })
                ),
                "a recorded exchange ({recorded:?}) was refused as foreign: {outcome:?}"
            );
        }
    }

    // FALSIFIABILITY: move the `custody.record = Some(recorded)` write in
    // `custody_record::perform_recorded_cas` to after `finish(prepared)?` --
    // exactly the ordering Finding 1 identified, where the record is written
    // only once the fallible page cleanup has returned -- and this test fails:
    // the custody cell comes back empty. That empty state is what leaves a
    // caller who genuinely did perform the transition refused as
    // `UnwitnessedForeignRestore`, and the last assertion here is the retry it
    // must get instead.
    #[test]
    fn a_cleanup_failure_after_the_exchange_still_leaves_the_exchange_recorded() {
        let mut custody = RestoreCasCustody::unrecorded();
        let outcome = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Err("page protection restoration failed"),
        );
        assert_eq!(outcome, Err("page protection restoration failed"));
        // This is the same cell type production hands in -- the field of the
        // registry binding -- so this pins the write production actually
        // performs rather than one the test supplied.
        let stored = custody
            .recorded()
            .expect("the exchange must be recorded before the cleanup runs");
        assert_eq!(stored.observed_before(), RECORDED_SENTINEL_BEFORE);
        assert_eq!(stored.protection_before(), PROTECTION_BEFORE);
        assert_eq!(stored.protection_temporary(), PROTECTION_TEMPORARY);
        assert!(!stored.protection_reconciled());
        assert_eq!(
            decide_pointer_slot_restore_cas(ORIGINAL, ORIGINAL, SHIM, custody.recorded()),
            Ok(
                PointerSlotRestoreCasDecision::ReconcileProtectionThenReplay(stored)
            )
        );
    }

    // FALSIFIABILITY: drop the second `custody.record = Some(reconciled)` write
    // in `custody_record::perform_recorded_cas` and this test fails -- the cell
    // would still hold the unreconciled record, so a fully completed restore
    // would stay marked as owing page cleanup forever and
    // `complete_recorded_restore_cas_observation` would refuse it. The
    // companion property -- that the *first* write lands before the fallible
    // cleanup -- is pinned by
    // `a_cleanup_failure_after_the_exchange_still_leaves_the_exchange_recorded`,
    // which is the only path on which that intermediate write is observable.
    //
    // The exchange and the cleanup here are injected closures, so what this pins
    // is the ordering contract -- not that a hardware compare-exchange swapped
    // anything.
    #[test]
    fn a_restore_exchange_whose_injected_cleanup_succeeds_is_recorded_as_reconciled() {
        let mut custody = RestoreCasCustody::unrecorded();
        let outcome = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Ok(PROTECTION_BEFORE),
        );
        let (performed, protection_after) =
            outcome.expect("a cleanup returning the starting protection cannot fail");
        assert_eq!(protection_after, PROTECTION_BEFORE);
        let reconciled =
        performed
            .release_if_custody_matches(custody.recorded())
            .expect("the helper writes into the cell it was handed");
        assert_eq!(reconciled.observed_before(), RECORDED_SENTINEL_BEFORE);
        assert_eq!(reconciled.protection_before(), PROTECTION_BEFORE);
        assert_eq!(reconciled.protection_temporary(), PROTECTION_TEMPORARY);
        assert!(reconciled.protection_reconciled());
        assert_eq!(custody.recorded(), Some(reconciled));
    }

    // FALSIFIABILITY: change `protection_reconciled: protection_after ==
    // protection_before` in `custody_record::perform_recorded_cas` back to a
    // bare `true` -- the shape both paths shipped with -- and this test fails:
    // the record would claim the page was observed back at its starting
    // protection when the cleanup returned something else. The negative arm is
    // the sibling test above, where a cleanup that does return the starting
    // protection must still reconcile.
    #[test]
    fn a_cleanup_returning_the_wrong_protection_is_not_recorded_as_reconciled() {
        assert_ne!(PROTECTION_BEFORE, PROTECTION_TEMPORARY);
        let mut custody = RestoreCasCustody::unrecorded();
        let (performed, protection_after) = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| RECORDED_SENTINEL_BEFORE,
            &mut custody,
            // Returns `Ok`, but not at the protection the exchange started from.
            |_: ()| Ok(PROTECTION_TEMPORARY),
        )
        .expect("an Ok cleanup must not turn into an error");
        assert_eq!(protection_after, PROTECTION_TEMPORARY);
        let recorded =
        performed
            .release_if_custody_matches(custody.recorded())
            .expect("the helper writes into the cell it was handed");
        assert!(!recorded.protection_reconciled());
        assert_eq!(custody.recorded(), Some(recorded));
        assert_eq!(
            complete_recorded_restore_cas_observation(recorded, protection_after),
            Err(
                PointerSlotRestoreCustodyError::RecordedRestoreProtectionUnreconciled {
                    protection_before: PROTECTION_BEFORE,
                    protection_after: PROTECTION_TEMPORARY,
                }
            )
        );
    }

    // FALSIFIABILITY: drop the `?` on `prepare()` in
    // `custody_record::perform_recorded_cas` and this test panics in the
    // `exchange` closure -- the slot would be exchanged after the page could not
    // be made writable. The panicking closures are the negative arm: they assert
    // the failed prepare short-circuits rather than merely that an error came
    // back.
    #[test]
    fn a_failure_before_the_exchange_records_nothing() {
        let mut custody = RestoreCasCustody::unrecorded();
        let outcome = perform_recorded_cas::<custody_record::Restore, (), &str>(
            || Err("page protection change failed"),
            |_| panic!("must not exchange after a failed prepare"),
            &mut custody,
            |_| panic!("must not run cleanup after a failed prepare"),
        );
        assert_eq!(outcome, Err("page protection change failed"));
        assert_eq!(custody.recorded(), None);
    }

    // FALSIFIABILITY: drop the `custody.record = Some(reconciled)` write in
    // `custody_record::reconcile_recorded_cas` and the last assertion fails: the
    // cell would still hold the unreconciled record, so the resumed attempt
    // would owe the page cleanup forever. Make `reconcile_recorded_cas` set
    // `protection_reconciled: true` unconditionally and
    // `a_reconciliation_that_lands_elsewhere_stays_unreconciled` fails instead.
    #[test]
    fn a_resumed_reconciliation_marks_the_recorded_exchange_reconciled() {
        let mut custody = RestoreCasCustody::unrecorded();
        let owed = recorded_cas_pending_protection();
        let (performed, protection_after) =
            reconcile_recorded_cas(owed, &mut custody, |protection_before| {
                // The helper must hand the reconciliation the protection the
                // exchange started from, not some value of the caller's.
                assert_eq!(protection_before, PROTECTION_BEFORE);
                Ok::<_, &str>(PROTECTION_BEFORE)
            })
            .expect("an injected reconciliation returning Ok cannot fail");
        assert_eq!(protection_after, PROTECTION_BEFORE);
        let reconciled =
        performed
            .release_if_custody_matches(custody.recorded())
            .expect("the helper writes into the cell it was handed");
        // `observed_before` is carried through: reconciling the page must not be
        // able to change what transition the record attests.
        assert_eq!(reconciled.observed_before(), RECORDED_SENTINEL_BEFORE);
        assert_eq!(reconciled.protection_temporary(), PROTECTION_TEMPORARY);
        assert!(reconciled.protection_reconciled());
        assert_eq!(custody.recorded(), Some(reconciled));
    }

    // FALSIFIABILITY: set `protection_reconciled: true` unconditionally in
    // `custody_record::reconcile_recorded_cas` -- the shape the production
    // `ReconcileProtectionThenReplay` arms shipped with -- and this test fails:
    // a page that came back at some other protection would be recorded as
    // reconciled, and a later retry would take the plain replay arm believing
    // the cleanup was done.
    #[test]
    fn a_reconciliation_that_lands_elsewhere_stays_unreconciled() {
        assert_ne!(PROTECTION_BEFORE, PROTECTION_TEMPORARY);
        let mut custody = RestoreCasCustody::unrecorded();
        let (performed, protection_after) = reconcile_recorded_cas(
            recorded_cas_pending_protection(),
            &mut custody,
            |_| Ok::<_, &str>(PROTECTION_TEMPORARY),
        )
        .expect("an Ok reconciliation must not turn into an error");
        assert_eq!(protection_after, PROTECTION_TEMPORARY);
        let still_owed =
        performed
            .release_if_custody_matches(custody.recorded())
            .expect("the helper writes into the cell it was handed");
        assert!(!still_owed.protection_reconciled());
        assert_eq!(custody.recorded(), Some(still_owed));
        assert_eq!(
            complete_recorded_restore_cas_observation(still_owed, protection_after),
            Err(
                PointerSlotRestoreCustodyError::RecordedRestoreProtectionUnreconciled {
                    protection_before: PROTECTION_BEFORE,
                    protection_after: PROTECTION_TEMPORARY,
                }
            )
        );
    }

    // FALSIFIABILITY: make `custody_record::reconcile_recorded_cas` absorb the
    // failure instead of propagating it -- `let protection_after =
    // reconcile(recorded.protection_before()).unwrap_or(recorded.protection_before)`,
    // the "recover by assuming what we wanted to see" shape that produced both
    // of the forged-attestation defects on this file -- and this test fails
    // twice over: the call returns `Ok` and custody comes back holding a record
    // marked reconciled.
    #[test]
    fn a_failed_reconciliation_leaves_the_recorded_exchange_still_owing_the_page() {
        // Custody in the state production is in when this arm runs: the exchange
        // recorded, the page cleanup still owed.
        let mut custody = RestoreCasCustody::unrecorded();
        let _ = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Err("injected page protection restoration failure"),
        );
        let owed = custody
            .recorded()
            .expect("the exchange must be recorded before the cleanup runs");
        let outcome = reconcile_recorded_cas(owed, &mut custody, |_| {
            Err::<u32, &str>("page protection recovery remains unresolved")
        });
        assert_eq!(outcome, Err("page protection recovery remains unresolved"));
        assert_eq!(custody.recorded(), Some(owed));
        assert!(!owed.protection_reconciled());
    }

    // FALSIFIABILITY: drop the `!recorded.protection_reconciled` clause from
    // `complete_recorded_restore_cas_observation` and the first assertion fails:
    // an exchange whose page cleanup never completed would mint a receipt.
    // Drop the `protection_after != recorded.protection_before` clause and the
    // second fails.
    #[test]
    fn an_unreconciled_record_cannot_form_a_receipt_observation() {
        assert_eq!(
            complete_recorded_restore_cas_observation(
                recorded_cas_pending_protection(),
                PROTECTION_BEFORE
            ),
            Err(
                PointerSlotRestoreCustodyError::RecordedRestoreProtectionUnreconciled {
                    protection_before: PROTECTION_BEFORE,
                    protection_after: PROTECTION_BEFORE,
                }
            )
        );
        assert_eq!(
            complete_recorded_restore_cas_observation(recorded_cas(), PROTECTION_TEMPORARY),
            Err(
                PointerSlotRestoreCustodyError::RecordedRestoreProtectionUnreconciled {
                    protection_before: PROTECTION_BEFORE,
                    protection_after: PROTECTION_TEMPORARY,
                }
            )
        );
    }

    // FALSIFIABILITY: delete the `current_slot_value != expected_original_address`
    // check in the `Some(recorded)` branch and this test fails: a slot rewritten
    // back to the shim since the recorded CAS would replay the stale observation
    // and return `Ok`.
    #[test]
    fn retry_with_record_but_slot_no_longer_original_is_refused() {
        let outcome =
            decide_pointer_slot_restore_cas(SHIM, ORIGINAL, SHIM, Some(recorded_cas()));
        assert_eq!(
            outcome,
            Err(
                PointerSlotRestoreCustodyError::RecordedRestoreCasSlotDiverged {
                    observed_slot_value: SHIM,
                    expected_original_address: ORIGINAL,
                }
            )
        );
    }

    // FALSIFIABILITY: remove the `external_shim_address == expected_original_address`
    // guard at the top of `decide_pointer_slot_restore_cas` and this test
    // fails: with the two addresses equal the `None` case falls through to
    // `UnwitnessedForeignRestore` and the `Some` case to a replay, neither of
    // which is an attestable transition.
    #[test]
    fn shim_sharing_the_original_address_is_refused_with_and_without_a_record() {
        let expected = Err(
            PointerSlotRestoreCustodyError::IndistinguishableShimAndOriginalAddress {
                address: ORIGINAL,
            },
        );
        assert_eq!(
            decide_pointer_slot_restore_cas(ORIGINAL, ORIGINAL, ORIGINAL, None),
            expected
        );
        assert_eq!(
            decide_pointer_slot_restore_cas(ORIGINAL, ORIGINAL, ORIGINAL, Some(recorded_cas())),
            expected
        );
    }

    // FALSIFIABILITY: broaden the `None` branch to refuse anything other than
    // the shim -- e.g. `if current_slot_value != external_shim_address { Err(..) }`
    // -- and this test fails. The refusal must be specific to the already-original
    // case; a third-party value is still a case the real CAS resolves, and it
    // fails there with the existing "restore CAS expected shim" message.
    #[test]
    fn first_attempt_with_a_third_party_slot_value_still_performs_the_cas() {
        let outcome = decide_pointer_slot_restore_cas(FOREIGN, ORIGINAL, SHIM, None);
        assert_eq!(outcome, Ok(PointerSlotRestoreCasDecision::PerformCas));
    }

    // FALSIFIABILITY: any edit that lets the no-record path yield a replay --
    // for instance defaulting `recorded_prior_cas` to a synthesized observation
    // when it is `None` -- makes this test fail. This is property A1 stated
    // directly: the replay arm is reachable only from recorded evidence.
    #[test]
    fn replay_arm_is_unreachable_without_a_recorded_cas() {
        for current in [SHIM, ORIGINAL, FOREIGN, 0usize, usize::MAX] {
            let outcome = decide_pointer_slot_restore_cas(current, ORIGINAL, SHIM, None);
            // BOTH replay variants, not just the plain one. Rejecting only
            // `ReplayRecordedCas` left `ReconcileProtectionThenReplay` free to
            // be fabricated from `None`, which reaches a receipt through the
            // same completion path and is the identical defect.
            assert!(
                !matches!(
                    outcome,
                    Ok(PointerSlotRestoreCasDecision::ReplayRecordedCas(_))
                        | Ok(PointerSlotRestoreCasDecision::ReconcileProtectionThenReplay(_))
                ),
                "slot value 0x{current:X} reached a replay arm with no recorded CAS: {outcome:?}"
            );
        }
    }

    // ---- Install path: the same defect in the original -> shim direction --
    //
    // `install_pointer_slot` used to synthesize its CAS observation
    // whenever the slot already read the external shim:
    //
    //     let cas = if current == external_shim_address {
    //         AtomicPointerCasObservation { observed_before: expected.expected_original_address, .. }
    //     } else { atomic_compare_exchange_pointer_slot(..) };
    //
    // and then checked `cas.observed_before != expected.expected_original_address`
    // -- a tautology on that arm, because the value had just been *defined* to
    // be the value it was checked against. An installation receipt could
    // therefore be minted with no evidence that this crate ever performed the
    // installing exchange.

    /// `observed_before` here is deliberately *not* `ORIGINAL`. The pre-fix
    /// install code synthesized `observed_before` as
    /// `expected.expected_original_address`, so any test whose recorded value
    /// equals `ORIGINAL` cannot tell a replay from a reconstruction. This
    /// sentinel can. Held separate from the restore side's sentinel so editing
    /// one cannot silently change what the other proves.
    const INSTALL_RECORDED_SENTINEL_BEFORE: usize = 0x0000_7777_8000;

    /// An install exchange this crate performed whose page cleanup completed.
    ///
    /// As on the restore side, built by driving the real ordering helper: an
    /// install record cannot be written down by hand any more.
    fn recorded_install_cas() -> RecordedInstallCas {
        let mut custody = InstallCasCustody::unrecorded();
        let (performed, _) = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| INSTALL_RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Ok(PROTECTION_BEFORE),
        )
        .expect("an injected cleanup returning the starting protection cannot fail");
        performed
            .release_if_custody_matches(custody.recorded())
            .expect("the helper writes into the cell it was handed")
    }

    /// An install exchange that completed but whose page cleanup then failed.
    fn recorded_install_cas_pending_protection() -> RecordedInstallCas {
        let mut custody = InstallCasCustody::unrecorded();
        let outcome = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| INSTALL_RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Err("injected page protection restoration failure"),
        );
        assert_eq!(outcome, Err("injected page protection restoration failure"));
        custody
            .recorded()
            .expect("the exchange is recorded before the cleanup runs")
    }

    // FALSIFIABILITY: delete the `current_slot_value == external_shim_address`
    // refusal in the `None` branch of `decide_pointer_slot_install_cas` --
    // i.e. restore the pre-fix behaviour where an already-shim slot silently
    // took the synthesized-observation arm -- and this test fails: the call
    // returns `Ok(PerformCas)` instead of `Err(UnwitnessedForeignInstall)`.
    //
    // This is the forged-attestation case. On a first attempt the crate has
    // recorded no CAS; if the slot already reads the shim then something
    // outside this crate put it there, and no installation receipt may be
    // minted for a transition this crate did not perform.
    #[test]
    fn install_first_attempt_with_slot_already_shim_and_no_record_is_refused() {
        let outcome = decide_pointer_slot_install_cas(SHIM, ORIGINAL, SHIM, None);
        assert_eq!(
            outcome,
            Err(
                PointerSlotInstallCustodyError::UnwitnessedForeignInstall {
                    observed_slot_value: SHIM
                }
            )
        );
    }

    // FALSIFIABILITY: make the `None` branch refuse unconditionally, or invert
    // its comparison to `current_slot_value != external_shim_address`, and this
    // test fails: the genuine first attempt no longer reaches `PerformCas`.
    // This is the negative arm of the test above -- without it the refusal could
    // be a blanket rejection and still look correct.
    #[test]
    fn install_first_attempt_with_slot_holding_the_original_selects_a_real_cas() {
        let outcome = decide_pointer_slot_install_cas(ORIGINAL, ORIGINAL, SHIM, None);
        assert_eq!(outcome, Ok(PointerSlotInstallCasDecision::PerformCas));
    }

    // FALSIFIABILITY: change the `Some(recorded)` branch to rebuild the
    // observation from the caller-supplied addresses -- e.g. return an
    // observation with `observed_before: expected_original_address`, which is
    // exactly what the pre-fix install code did -- and this test fails on
    // `observed_before`, because the replayed value would become `ORIGINAL`
    // instead of the recorded sentinel.
    //
    // This is the property the whole lane exists for: the retry arm reports what
    // the earlier CAS actually returned, never a value reconstructed from
    // `expected`.
    #[test]
    fn install_retry_replays_the_recorded_observation_verbatim() {
        // The sentinel is load-bearing test data, so pin it here. Set it equal
        // to `ORIGINAL` and the assertions below pass against the pre-fix
        // reconstruction as well -- the test would go vacuous with nothing
        // going red anywhere.
        assert_ne!(INSTALL_RECORDED_SENTINEL_BEFORE, ORIGINAL);
        assert_ne!(INSTALL_RECORDED_SENTINEL_BEFORE, SHIM);
        assert_ne!(SHIM, ORIGINAL);

        let recorded = recorded_install_cas();
        let outcome = decide_pointer_slot_install_cas(SHIM, ORIGINAL, SHIM, Some(recorded));
        // Field by field rather than against `recorded_install_cas()`: comparing
        // the decision to the same helper that produced its input would pass for
        // any implementation that echoes its argument, including one that
        // rebuilt the record from the addresses.
        let Ok(PointerSlotInstallCasDecision::ReplayRecordedCas(replayed)) = outcome else {
            panic!("a reconciled record must select the plain replay arm: {outcome:?}");
        };
        assert_eq!(replayed.observed_before(), INSTALL_RECORDED_SENTINEL_BEFORE);
        assert_eq!(replayed.protection_before(), PROTECTION_BEFORE);
        assert_eq!(replayed.protection_temporary(), PROTECTION_TEMPORARY);
        assert!(replayed.protection_reconciled());
        // ...and the value that reaches the receipt is that same recorded one.
        assert_eq!(
            complete_recorded_install_cas_observation(recorded, PROTECTION_BEFORE),
            Ok(AtomicPointerCasObservation {
                observed_before: INSTALL_RECORDED_SENTINEL_BEFORE,
                protection_before: PROTECTION_BEFORE,
                protection_temporary: PROTECTION_TEMPORARY,
                protection_after: PROTECTION_BEFORE,
            })
        );
    }

    // FALSIFIABILITY: make the `Some(recorded)` branch return `ReplayRecordedCas`
    // regardless of `protection_reconciled` -- i.e. collapse the two replay
    // variants into one -- and this test fails. It is the sequencing decision
    // for the state a successful exchange followed by a failed page cleanup
    // leaves behind: the exchange happened and must be replayed, but the page
    // cleanup is still owed and has to run before a receipt can form.
    #[test]
    fn install_retry_after_a_cleanup_failure_resumes_the_protection_reconciliation() {
        let outcome = decide_pointer_slot_install_cas(
            SHIM,
            ORIGINAL,
            SHIM,
            Some(recorded_install_cas_pending_protection()),
        );
        assert_eq!(
            outcome,
            Ok(
                PointerSlotInstallCasDecision::ReconcileProtectionThenReplay(
                    recorded_install_cas_pending_protection()
                )
            )
        );
    }

    // FALSIFIABILITY: delete the record-back between `exchange_pointer_slot` and
    // `finish_atomic_compare_exchange_pointer_slot` in the install `PerformCas`
    // arm -- i.e. record only after the fallible cleanup returned -- and the
    // state this test describes becomes `None` instead, which
    // `install_first_attempt_with_slot_already_shim_and_no_record_is_refused`
    // shows is refused as `UnwitnessedForeignInstall`. This test names the
    // outcome that refusal would wrongly produce for a caller who genuinely did
    // perform the transition, i.e. requirement 3: idempotent retry survives.
    #[test]
    fn a_recorded_install_exchange_is_never_mistaken_for_a_foreign_install() {
        for recorded in [
            recorded_install_cas(),
            recorded_install_cas_pending_protection(),
        ] {
            let outcome = decide_pointer_slot_install_cas(SHIM, ORIGINAL, SHIM, Some(recorded));
            assert!(
                !matches!(
                    outcome,
                    Err(PointerSlotInstallCustodyError::UnwitnessedForeignInstall { .. })
                ),
                "a recorded install exchange ({recorded:?}) was refused as foreign: {outcome:?}"
            );
        }
    }

    // FALSIFIABILITY: move the first `custody.record = Some(recorded)` write in
    // `custody_record::perform_recorded_cas` to after `finish(prepared)?` and
    // this test fails: the custody cell comes back `None`. That empty state is
    // what would leave a caller who genuinely did perform the installing
    // exchange refused as `UnwitnessedForeignInstall` on the retry, and the last
    // assertion here is the retry it must get instead.
    //
    // The install and restore directions share one implementation, so this and
    // its restore twin fail together; they are kept separate because each pins
    // its own direction's decision function on the last assertion.
    #[test]
    fn a_cleanup_failure_after_the_install_exchange_still_leaves_the_exchange_recorded() {
        let mut custody = InstallCasCustody::unrecorded();
        let outcome = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| INSTALL_RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Err("page protection restoration failed"),
        );
        assert_eq!(outcome, Err("page protection restoration failed"));
        // This is the same cell type production hands in -- the field of the
        // provisional binding held in the custody registry -- so this pins the
        // write production actually performs.
        let stored = custody
            .recorded()
            .expect("the exchange must be recorded before the cleanup runs");
        assert_eq!(stored.observed_before(), INSTALL_RECORDED_SENTINEL_BEFORE);
        assert_eq!(stored.protection_before(), PROTECTION_BEFORE);
        assert_eq!(stored.protection_temporary(), PROTECTION_TEMPORARY);
        assert!(!stored.protection_reconciled());
        assert_eq!(
            decide_pointer_slot_install_cas(SHIM, ORIGINAL, SHIM, custody.recorded()),
            Ok(PointerSlotInstallCasDecision::ReconcileProtectionThenReplay(stored))
        );
    }

    // FALSIFIABILITY: drop the second `custody.record = Some(reconciled)` write
    // in `custody_record::perform_recorded_cas` and this test fails -- the cell
    // would still hold the unreconciled record, so a fully completed install
    // would stay marked as owing page cleanup forever and
    // `complete_recorded_install_cas_observation` would refuse it. The companion
    // property -- that the *first* write lands before the fallible cleanup -- is
    // pinned by
    // `a_cleanup_failure_after_the_install_exchange_still_leaves_the_exchange_recorded`,
    // the only path on which that intermediate write is observable.
    //
    // The exchange and the cleanup here are injected closures, so what this
    // pins is the ordering contract -- not that a hardware compare-exchange
    // swapped anything. The name says so.
    #[test]
    fn an_install_exchange_whose_injected_cleanup_succeeds_is_recorded_as_reconciled() {
        let mut custody = InstallCasCustody::unrecorded();
        let (performed, protection_after) = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| INSTALL_RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Ok(PROTECTION_BEFORE),
        )
        .expect("a cleanup returning the starting protection cannot fail");
        assert_eq!(protection_after, PROTECTION_BEFORE);
        let reconciled =
        performed
            .release_if_custody_matches(custody.recorded())
            .expect("the helper writes into the cell it was handed");
        assert_eq!(reconciled.observed_before(), INSTALL_RECORDED_SENTINEL_BEFORE);
        assert_eq!(reconciled.protection_before(), PROTECTION_BEFORE);
        assert_eq!(reconciled.protection_temporary(), PROTECTION_TEMPORARY);
        assert!(reconciled.protection_reconciled());
        assert_eq!(custody.recorded(), Some(reconciled));
    }

    // FALSIFIABILITY: drop the `?` on `prepare()` in
    // `custody_record::perform_recorded_cas` and this test panics in the
    // `exchange` closure -- the slot would be exchanged after the page could not
    // be made writable. The panicking closures are the negative arm: they assert
    // the failed prepare short-circuits rather than merely that an error came
    // back.
    #[test]
    fn a_failure_before_the_install_exchange_records_nothing() {
        let mut custody = InstallCasCustody::unrecorded();
        let outcome = perform_recorded_cas::<custody_record::Install, (), &str>(
            || Err("page protection change failed"),
            |_| panic!("must not exchange after a failed prepare"),
            &mut custody,
            |_| panic!("must not run cleanup after a failed prepare"),
        );
        assert_eq!(outcome, Err("page protection change failed"));
        assert_eq!(custody.recorded(), None);
    }

    // FALSIFIABILITY: delete the `custody.record = Some(reconciled)` write in
    // `custody_record::reconcile_recorded_cas` and this test fails -- an install
    // whose resumed page reconciliation succeeded would keep a record saying the
    // page is still owed, and every later retry would redo the reconciliation.
    #[test]
    fn a_resumed_install_reconciliation_marks_the_recorded_exchange_reconciled() {
        let mut custody = InstallCasCustody::unrecorded();
        let (performed, protection_after) = reconcile_recorded_cas(
            recorded_install_cas_pending_protection(),
            &mut custody,
            |protection_before| {
                assert_eq!(protection_before, PROTECTION_BEFORE);
                Ok::<_, &str>(PROTECTION_BEFORE)
            },
        )
        .expect("an injected reconciliation returning Ok cannot fail");
        assert_eq!(protection_after, PROTECTION_BEFORE);
        let reconciled =
        performed
            .release_if_custody_matches(custody.recorded())
            .expect("the helper writes into the cell it was handed");
        assert_eq!(reconciled.observed_before(), INSTALL_RECORDED_SENTINEL_BEFORE);
        assert!(reconciled.protection_reconciled());
        assert_eq!(custody.recorded(), Some(reconciled));
    }

    // ---- The redirected-write backstop -----------------------------------
    //
    // `perform_recorded_cas` writes through the `&mut CasCustody` it is handed.
    // Safe Rust cannot express "this borrow points into the registry entry the
    // receipt will be minted from", so a call site that handed over a leaked
    // throwaway cell would compile and silently lose the evidence. The call
    // sites therefore re-read the registry afterwards and pass what they found
    // here.

    /// One exchange, returning both the unconfirmed record and the cell it was
    /// written into. A fresh pair each call, because confirming consumes the
    /// unconfirmed record -- which is the property that makes the confirmation
    /// impossible to skip.
    fn performed_install_cas() -> (UnconfirmedInstallCas, InstallCasCustody) {
        let mut custody = InstallCasCustody::unrecorded();
        let (performed, _) = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| INSTALL_RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Ok(PROTECTION_BEFORE),
        )
        .expect("an injected cleanup returning the starting protection cannot fail");
        (performed, custody)
    }

    /// The restore twin of `performed_install_cas`.
    fn performed_restore_cas() -> (UnconfirmedRestoreCas, RestoreCasCustody) {
        let mut custody = RestoreCasCustody::unrecorded();
        let (performed, _) = perform_recorded_cas(
            || Ok::<_, &str>(((), PROTECTION_BEFORE, PROTECTION_TEMPORARY)),
            |_: &()| RECORDED_SENTINEL_BEFORE,
            &mut custody,
            |_: ()| Ok(PROTECTION_BEFORE),
        )
        .expect("an injected cleanup returning the starting protection cannot fail");
        (performed, custody)
    }

    // FALSIFIABILITY: delete the `observed_custody != Some(self.0)` refusal in
    // `custody_record::UnconfirmedCas::release_if_custody_matches` and the first two
    // assertions return `Ok`. The `None` case is the one a leaked custody cell
    // produces: the exchange happened, the cell the receipt is minted against
    // holds nothing, and without this the next attempt would refuse as
    // `UnwitnessedForeignInstall` -- blaming a foreign agent for this crate's
    // own lost write.
    #[test]
    fn install_custody_that_did_not_retain_the_performed_record_is_refused() {
        let record = recorded_install_cas();

        // What a redirected write leaves behind: the cell the receipt is minted
        // against never received anything.
        let (performed, _) = performed_install_cas();
        assert_eq!(
            release_install_cas_if_custody_matches(None, performed),
            Err(
                PointerSlotInstallCustodyError::RecordedInstallCasNotRetained {
                    retained: None,
                    performed: record,
                }
            )
        );

        // A cell holding some other record is refused just as hard.
        let stale = recorded_install_cas_pending_protection();
        assert_ne!(stale, record);
        let (performed, _) = performed_install_cas();
        assert_eq!(
            release_install_cas_if_custody_matches(Some(stale), performed),
            Err(
                PointerSlotInstallCustodyError::RecordedInstallCasNotRetained {
                    retained: Some(stale),
                    performed: record,
                }
            )
        );

        // Negative arm: the honest case must pass and must hand back the record
        // itself, or the check would be a blanket refusal no production path
        // could satisfy.
        let (performed, custody) = performed_install_cas();
        assert_eq!(
            release_install_cas_if_custody_matches(custody.recorded(), performed),
            Ok(record)
        );
    }

    // FALSIFIABILITY: the restore twin of the test above; the two directions
    // share `release_if_custody_matches`, so both go red together.
    #[test]
    fn restore_custody_that_did_not_retain_the_performed_record_is_refused() {
        let record = recorded_cas();

        let (performed, _) = performed_restore_cas();
        assert_eq!(
            release_restore_cas_if_custody_matches(None, performed),
            Err(
                PointerSlotRestoreCustodyError::RecordedRestoreCasNotRetained {
                    retained: None,
                    performed: record,
                }
            )
        );

        let stale = recorded_cas_pending_protection();
        assert_ne!(stale, record);
        let (performed, _) = performed_restore_cas();
        assert_eq!(
            release_restore_cas_if_custody_matches(Some(stale), performed),
            Err(
                PointerSlotRestoreCustodyError::RecordedRestoreCasNotRetained {
                    retained: Some(stale),
                    performed: record,
                }
            )
        );

        let (performed, custody) = performed_restore_cas();
        assert_eq!(
            release_restore_cas_if_custody_matches(custody.recorded(), performed),
            Ok(record)
        );
    }

    // ---- Lease protection, and the states that stay stranded -------------

    const LEASE_PROTECTION: u32 = PROTECTION_BEFORE;
    const EXTERNALLY_REPROTECTED: u32 = 0x33;

    /// The observation a completed exchange produces, parameterised so a test
    /// can state the page protections it wants without hand-building a record.
    fn cas_observation(protection_before: u32, protection_after: u32) -> AtomicPointerCasObservation {
        AtomicPointerCasObservation {
            observed_before: INSTALL_RECORDED_SENTINEL_BEFORE,
            protection_before,
            protection_temporary: PROTECTION_TEMPORARY,
            protection_after,
        }
    }

    // FALSIFIABILITY: relax either comparison in
    // `verify_install_cas_matches_lease_protection` to compare the CAS against
    // its own `protection_before` instead of against the lease's -- the change
    // that would make the externally-re-protected page retryable -- and the
    // first assertion returns `Ok`. That is the change this lane examined and
    // declined: it would let a receipt form over a page some other agent
    // re-protected while this crate held the lease.
    #[test]
    fn an_install_cas_over_an_externally_reprotected_page_is_refused() {
        assert_ne!(LEASE_PROTECTION, EXTERNALLY_REPROTECTED);
        assert_eq!(
            verify_install_cas_matches_lease_protection(
                LEASE_PROTECTION,
                cas_observation(EXTERNALLY_REPROTECTED, EXTERNALLY_REPROTECTED),
            ),
            Err(
                PointerSlotInstallCustodyError::InstallCasProtectionBeforeDivergedFromLease {
                    lease_protection: LEASE_PROTECTION,
                    cas_protection_before: EXTERNALLY_REPROTECTED,
                }
            )
        );
        // The after-clause carries its own variant so a log reader can tell a
        // page that was re-protected before the exchange from one the cleanup
        // failed to put back. Defence in depth: with the current
        // `complete_recorded_*_cas_observation` a CAS observation reaching here
        // always has `protection_after == protection_before`, so only a change
        // there could make production produce this one.
        assert_eq!(
            verify_install_cas_matches_lease_protection(
                LEASE_PROTECTION,
                cas_observation(LEASE_PROTECTION, EXTERNALLY_REPROTECTED),
            ),
            Err(
                PointerSlotInstallCustodyError::InstallCasProtectionAfterDivergedFromLease {
                    lease_protection: LEASE_PROTECTION,
                    cas_protection_after: EXTERNALLY_REPROTECTED,
                }
            )
        );
        // Negative arm: the ordinary case must pass, or every install would be
        // refused and the two assertions above would prove nothing.
        assert_eq!(
            verify_install_cas_matches_lease_protection(
                LEASE_PROTECTION,
                cas_observation(LEASE_PROTECTION, LEASE_PROTECTION),
            ),
            Ok(())
        );
    }

    // FALSIFIABILITY: the restore twin of the test above; relax either
    // comparison in `verify_restore_cas_matches_lease_protection`. Held
    // separately because the two directions must refuse the same states -- a
    // change to one alone shows up as one of these two tests going red.
    #[test]
    fn a_restore_cas_over_an_externally_reprotected_page_is_refused() {
        assert_ne!(LEASE_PROTECTION, EXTERNALLY_REPROTECTED);
        assert_eq!(
            verify_restore_cas_matches_lease_protection(
                LEASE_PROTECTION,
                cas_observation(EXTERNALLY_REPROTECTED, EXTERNALLY_REPROTECTED),
            ),
            Err(
                PointerSlotRestoreCustodyError::RestoreCasProtectionBeforeDivergedFromLease {
                    lease_protection: LEASE_PROTECTION,
                    cas_protection_before: EXTERNALLY_REPROTECTED,
                }
            )
        );
        assert_eq!(
            verify_restore_cas_matches_lease_protection(
                LEASE_PROTECTION,
                cas_observation(LEASE_PROTECTION, EXTERNALLY_REPROTECTED),
            ),
            Err(
                PointerSlotRestoreCustodyError::RestoreCasProtectionAfterDivergedFromLease {
                    lease_protection: LEASE_PROTECTION,
                    cas_protection_after: EXTERNALLY_REPROTECTED,
                }
            )
        );
        assert_eq!(
            verify_restore_cas_matches_lease_protection(
                LEASE_PROTECTION,
                cas_observation(LEASE_PROTECTION, LEASE_PROTECTION),
            ),
            Ok(())
        );
    }

    // FALSIFIABILITY: change the `Some(recorded)` branch of either decision
    // function to fall through to `PerformCas` when the slot has diverged --
    // "retry by exchanging again" -- and this test fails. The two directions
    // must stay in step: the state is unretryable on both, deliberately, because
    // the record on hand describes an exchange whose result a third party has
    // since overwritten.
    #[test]
    fn a_diverged_slot_is_refused_rather_than_re_exchanged_in_both_directions() {
        assert_eq!(
            decide_pointer_slot_install_cas(
                ORIGINAL,
                ORIGINAL,
                SHIM,
                Some(recorded_install_cas())
            ),
            Err(
                PointerSlotInstallCustodyError::RecordedInstallCasSlotDiverged {
                    observed_slot_value: ORIGINAL,
                    external_shim_address: SHIM,
                }
            )
        );
        assert_eq!(
            decide_pointer_slot_restore_cas(SHIM, ORIGINAL, SHIM, Some(recorded_cas())),
            Err(
                PointerSlotRestoreCustodyError::RecordedRestoreCasSlotDiverged {
                    observed_slot_value: SHIM,
                    expected_original_address: ORIGINAL,
                }
            )
        );
    }

    // FALSIFIABILITY: drop the `!recorded.protection_reconciled` clause from
    // `complete_recorded_install_cas_observation` and the first assertion fails:
    // an exchange whose page cleanup never completed would mint a receipt. Drop
    // the `protection_after != recorded.protection_before` clause and the second
    // fails.
    #[test]
    fn an_unreconciled_install_record_cannot_form_a_receipt_observation() {
        // The second assertion is only a test of the protection comparison if
        // the two constants differ; equal, it would duplicate the first.
        assert_ne!(PROTECTION_BEFORE, PROTECTION_TEMPORARY);
        assert_eq!(
            complete_recorded_install_cas_observation(
                recorded_install_cas_pending_protection(),
                PROTECTION_BEFORE
            ),
            Err(
                PointerSlotInstallCustodyError::RecordedInstallProtectionUnreconciled {
                    protection_before: PROTECTION_BEFORE,
                    protection_after: PROTECTION_BEFORE,
                }
            )
        );
        assert_eq!(
            complete_recorded_install_cas_observation(
                recorded_install_cas(),
                PROTECTION_TEMPORARY
            ),
            Err(
                PointerSlotInstallCustodyError::RecordedInstallProtectionUnreconciled {
                    protection_before: PROTECTION_BEFORE,
                    protection_after: PROTECTION_TEMPORARY,
                }
            )
        );
    }

    // FALSIFIABILITY: delete the `current_slot_value != external_shim_address`
    // check in the `Some(recorded)` branch and this test fails: a slot rewritten
    // back to the original since the recorded CAS would replay the stale
    // observation and return `Ok`.
    #[test]
    fn install_retry_with_record_but_slot_no_longer_shim_is_refused() {
        let outcome = decide_pointer_slot_install_cas(
            ORIGINAL,
            ORIGINAL,
            SHIM,
            Some(recorded_install_cas()),
        );
        assert_eq!(
            outcome,
            Err(
                PointerSlotInstallCustodyError::RecordedInstallCasSlotDiverged {
                    observed_slot_value: ORIGINAL,
                    external_shim_address: SHIM,
                }
            )
        );
    }

    // FALSIFIABILITY: remove the `external_shim_address == expected_original_address`
    // guard at the top of `decide_pointer_slot_install_cas` and this test
    // fails: with the two addresses equal the `None` case falls through to
    // `UnwitnessedForeignInstall` and the `Some` case to a replay, neither of
    // which is an attestable transition.
    #[test]
    fn install_shim_sharing_the_original_address_is_refused_with_and_without_a_record() {
        let expected = Err(
            PointerSlotInstallCustodyError::IndistinguishableShimAndOriginalAddress {
                address: ORIGINAL,
            },
        );
        assert_eq!(
            decide_pointer_slot_install_cas(ORIGINAL, ORIGINAL, ORIGINAL, None),
            expected
        );
        assert_eq!(
            decide_pointer_slot_install_cas(
                ORIGINAL,
                ORIGINAL,
                ORIGINAL,
                Some(recorded_install_cas())
            ),
            expected
        );
    }

    // FALSIFIABILITY: broaden the `None` branch to refuse anything other than
    // the original -- e.g. `if current_slot_value != expected_original_address
    // { Err(..) }` -- and this test fails. The refusal must be specific to the
    // already-shim case; a third-party value is still a case the real CAS
    // resolves, and it fails there with the existing "import slot CAS expected"
    // message.
    #[test]
    fn install_first_attempt_with_a_third_party_slot_value_still_selects_a_real_cas() {
        let outcome = decide_pointer_slot_install_cas(FOREIGN, ORIGINAL, SHIM, None);
        assert_eq!(outcome, Ok(PointerSlotInstallCasDecision::PerformCas));
    }

    // FALSIFIABILITY: any edit that lets the no-record path yield a replay --
    // for instance defaulting `recorded_prior_cas` to a synthesized observation
    // when it is `None` -- makes this test fail. This states the lane's core
    // property directly: an installation receipt's `observed_before` can only
    // come from a real exchange, and the only arm that does not run one is
    // unreachable without recorded evidence.
    #[test]
    fn install_replay_arm_is_unreachable_without_a_recorded_cas() {
        for current in [SHIM, ORIGINAL, FOREIGN, 0usize, usize::MAX] {
            let outcome = decide_pointer_slot_install_cas(current, ORIGINAL, SHIM, None);
            // BOTH replay variants: rejecting only `ReplayRecordedCas` would
            // leave `ReconcileProtectionThenReplay` free to be fabricated from
            // `None`, which reaches a receipt through the same completion path
            // and is the identical defect.
            assert!(
                !matches!(
                    outcome,
                    Ok(PointerSlotInstallCasDecision::ReplayRecordedCas(_))
                        | Ok(PointerSlotInstallCasDecision::ReconcileProtectionThenReplay(_))
                ),
                "slot value 0x{current:X} reached an install replay arm with no recorded CAS: {outcome:?}"
            );
        }
    }

    // ---- the timeout message production actually emits -------------------

    // FALSIFIABILITY: change the call site to quote `max_yields` instead of
    // `outcome.yields_performed`, and this test fails -- a time-budget timeout
    // after 10 yields would otherwise report the full budget. The earlier loop
    // test proves the counters are *accurate*; only this one proves the message
    // quotes THOSE counters. Drop the stop reason from the format string and
    // the first assertion fails; drop the elapsed time and the last fails.
    #[test]
    fn the_timeout_message_quotes_the_outcome_counters_not_the_budget() {
        let outcome = Pe64ImportSlotQuiesceOutcome {
            stopped_on: Pe64ImportSlotQuiesceStop::TimeBudgetExhausted,
            observations: 11,
            yields_performed: 10,
            elapsed: Duration::from_millis(100),
        };
        let rendered = render_pointer_slot_quiesce_timeout("d90 convoy", &outcome);
        assert!(
            rendered.contains(&Pe64ImportSlotQuiesceStop::TimeBudgetExhausted.to_string()),
            "stop reason missing from {rendered:?}"
        );
        assert!(
            rendered.contains("after 10 yields"),
            "expected the 10 yields the loop performed, got {rendered:?}"
        );
        assert!(
            rendered.contains("11 dispatch observations"),
            "expected the 11 observations the loop made, got {rendered:?}"
        );
        assert!(rendered.contains("d90 convoy"), "logical name missing");
        assert!(
            rendered.contains(&format!("{:?}", Duration::from_millis(100))),
            "elapsed time missing from {rendered:?}"
        );
    }

    // FALSIFIABILITY: make the renderer quote anything other than the outcome's
    // own counters -- a constant, or `max_yields` -- and this fails.
    //
    // The two fixtures are identical in stop reason, elapsed time and logical
    // name, and differ ONLY in `observations` and `yields_performed`. That
    // isolation is the point: an earlier version of this test also varied the
    // stop reason and the elapsed time, so a renderer that ignored the counters
    // entirely and rendered only the stop reason still passed it.
    #[test]
    fn outcomes_differing_only_in_their_counters_render_differently() {
        let base = Pe64ImportSlotQuiesceOutcome {
            stopped_on: Pe64ImportSlotQuiesceStop::TimeBudgetExhausted,
            observations: 4,
            yields_performed: 3,
            elapsed: Duration::from_millis(100),
        };
        let more_iterations = Pe64ImportSlotQuiesceOutcome {
            observations: 900,
            yields_performed: 899,
            ..base
        };
        assert_eq!(base.stopped_on, more_iterations.stopped_on);
        assert_eq!(base.elapsed, more_iterations.elapsed);
        assert_ne!(
            render_pointer_slot_quiesce_timeout("lane", &base),
            render_pointer_slot_quiesce_timeout("lane", &more_iterations)
        );
    }

    // ---- Defect B1: the yield-budget ceiling ----------------------------

    // FALSIFIABILITY: change the comparison in
    // `validate_pointer_slot_quiesce_yields` from `>` to `>=` and this test
    // fails -- a budget exactly at the ceiling would be refused.
    #[test]
    fn yield_budget_at_the_ceiling_is_accepted_unchanged() {
        assert_eq!(
            validate_pointer_slot_quiesce_yields(MAX_POINTER_SLOT_QUIESCE_YIELDS),
            Ok(MAX_POINTER_SLOT_QUIESCE_YIELDS)
        );
    }

    // FALSIFIABILITY: delete the ceiling check in
    // `validate_pointer_slot_quiesce_yields` -- the pre-fix state, where
    // `max_yields` was never validated at all -- and this test fails: the call
    // returns `Ok` instead of `Err(YieldBudgetAboveCeiling)`.
    #[test]
    fn yield_budget_one_above_the_ceiling_is_refused() {
        assert_eq!(
            validate_pointer_slot_quiesce_yields(MAX_POINTER_SLOT_QUIESCE_YIELDS + 1),
            Err(Pe64ImportSlotQuiesceBoundError::YieldBudgetAboveCeiling {
                requested: MAX_POINTER_SLOT_QUIESCE_YIELDS + 1,
                ceiling: MAX_POINTER_SLOT_QUIESCE_YIELDS,
            })
        );
    }

    // FALSIFIABILITY: raise `MAX_POINTER_SLOT_QUIESCE_YIELDS` to `u32::MAX`
    // -- the unbounded state this lane exists to close -- and this test fails.
    // It pins the ceiling from above independently of the constant's own value.
    #[test]
    fn a_full_u32_yield_budget_is_refused() {
        assert_eq!(
            validate_pointer_slot_quiesce_yields(u32::MAX),
            Err(Pe64ImportSlotQuiesceBoundError::YieldBudgetAboveCeiling {
                requested: u32::MAX,
                ceiling: MAX_POINTER_SLOT_QUIESCE_YIELDS,
            })
        );
    }

    // FALSIFIABILITY: lower `MAX_POINTER_SLOT_QUIESCE_YIELDS` below the
    // iterations a tenth of the time budget can admit and this test fails. The
    // number is not free-standing: it is a tenth of the time budget divided by
    // the same assumed per-iteration cost the ceiling itself is derived from, so
    // a caller asking to wait a fraction of the budget must not be pre-empted by
    // the backstop.
    #[test]
    fn a_budget_covering_a_fraction_of_the_time_budget_is_accepted() {
        let tenth_of_the_budget = (POINTER_SLOT_QUIESCE_TIME_BUDGET_NANOS
            / 10
            / POINTER_SLOT_QUIESCE_ASSUMED_MIN_ITERATION_NANOS)
            as u32;
        assert_eq!(tenth_of_the_budget, 200_000);
        for budget in [0u32, 1, tenth_of_the_budget] {
            assert_eq!(
                validate_pointer_slot_quiesce_yields(budget),
                Ok(budget),
                "budget {budget} was refused"
            );
        }
    }

    // FALSIFIABILITY: replace `MAX_POINTER_SLOT_QUIESCE_YIELDS` with a
    // hand-picked literal, or change either policy constant without the ceiling
    // following, and this test fails. The ceiling is a backstop sized from the
    // time budget, not a number chosen on its own, and this pins that
    // relationship rather than the value -- asserting the constant equals itself
    // would prove nothing.
    #[test]
    fn the_yield_ceiling_is_derived_from_the_time_budget() {
        assert_eq!(
            u64::from(MAX_POINTER_SLOT_QUIESCE_YIELDS),
            POINTER_SLOT_QUIESCE_TIME_BUDGET_NANOS
                / POINTER_SLOT_QUIESCE_ASSUMED_MIN_ITERATION_NANOS
        );
        assert_eq!(
            POINTER_SLOT_QUIESCE_TIME_BUDGET,
            Duration::from_nanos(POINTER_SLOT_QUIESCE_TIME_BUDGET_NANOS)
        );
    }

    // FALSIFIABILITY: move `POINTER_SLOT_QUIESCE_TIME_BUDGET` outside the
    // band and this test fails. The budget is a policy choice, so what is pinned
    // is the property the policy has to satisfy, not the literal: below 250 ms a
    // callback that was merely descheduled a few times gets failed as a timeout,
    // above 10 s a teardown reads as hung to whoever is waiting on it.
    #[test]
    fn the_quiesce_time_budget_stays_inside_its_policy_band() {
        assert!(POINTER_SLOT_QUIESCE_TIME_BUDGET >= Duration::from_millis(250));
        assert!(POINTER_SLOT_QUIESCE_TIME_BUDGET <= Duration::from_secs(10));
    }

    // ---- Defect B2/B3: the real loop, driven directly --------------------

    /// Drive the production loop -- `drive_pointer_slot_quiesce_loop`, the
    /// same function `quiesce_pointer_slot` calls -- with scripted
    /// active-callback counts, a counted yield, and a clock this harness
    /// advances. Nothing here re-implements the loop; the control flow under
    /// test is the production control flow.
    ///
    /// Returns the outcome plus independently counted invocations of the two
    /// closures, so the loop's self-reported numbers can be checked against what
    /// it actually did rather than against a recomputation.
    fn run_real_quiesce_loop(
        max_yields: u32,
        time_budget: Duration,
        active_counts: &[u64],
        clock_step_per_yield: Duration,
    ) -> (Pe64ImportSlotQuiesceOutcome, u64, u32) {
        use std::cell::Cell;
        let observe_calls = Cell::new(0u64);
        let yield_calls = Cell::new(0u32);
        let clock = Cell::new(Duration::ZERO);
        let outcome = drive_pointer_slot_quiesce_loop(
            max_yields,
            time_budget,
            || {
                let index = observe_calls.get() as usize;
                observe_calls.set(observe_calls.get() + 1);
                let count = active_counts
                    .get(index)
                    .copied()
                    .unwrap_or_else(|| *active_counts.last().expect("non-empty script"));
                Ok::<u64, std::convert::Infallible>(count)
            },
            || {
                yield_calls.set(yield_calls.get() + 1);
                clock.set(clock.get() + clock_step_per_yield);
            },
            || clock.get(),
        )
        .expect("scripted observations never fail");
        (outcome, observe_calls.get(), yield_calls.get())
    }

    // FALSIFIABILITY: change the loop's `yields_performed >= max_yields` stop to
    // `>` and `yields_performed` becomes `max_yields + 1`, failing the second
    // assertion; move the `observations += 1` so it no longer counts every
    // observation and the fourth fails, because it compares the number the
    // timeout message quotes against the closure invocations this harness
    // counted independently. The old version of this test re-implemented the
    // loop and could not see either edit.
    #[test]
    fn the_real_loop_reports_the_counts_the_timeout_message_quotes() {
        for max_yields in [0u32, 1, 2, 7, 1_000] {
            let (outcome, observe_calls, yield_calls) = run_real_quiesce_loop(
                max_yields,
                Duration::from_secs(3_600),
                &[1],
                Duration::ZERO,
            );
            assert_eq!(
                outcome.stopped_on,
                Pe64ImportSlotQuiesceStop::YieldBudgetExhausted,
                "max_yields={max_yields}"
            );
            assert_eq!(
                outcome.yields_performed, max_yields,
                "yield count at max_yields={max_yields}"
            );
            assert_eq!(
                outcome.observations,
                u64::from(max_yields) + 1,
                "observation count at max_yields={max_yields}"
            );
            assert_eq!(
                outcome.observations, observe_calls,
                "reported observations disagree with the observations performed at max_yields={max_yields}"
            );
            assert_eq!(
                outcome.yields_performed, yield_calls,
                "reported yields disagree with the yields performed at max_yields={max_yields}"
            );
            assert!(!outcome.observed_zero());
        }
    }

    // FALSIFIABILITY: move the time-budget check above the `active_count == 0`
    // check in `drive_pointer_slot_quiesce_loop` and this test fails. The
    // budget here is already exhausted at entry, so a slot that is quiescent on
    // its first observation must still be reported as quiesced -- the deadline
    // may not fail an operation that has already succeeded.
    #[test]
    fn the_real_loop_reports_quiescence_even_with_the_time_budget_already_spent() {
        let (outcome, observe_calls, yield_calls) =
            run_real_quiesce_loop(1_000, Duration::ZERO, &[0], Duration::ZERO);
        assert_eq!(outcome.stopped_on, Pe64ImportSlotQuiesceStop::Quiesced);
        assert!(outcome.observed_zero());
        assert_eq!(outcome.observations, 1);
        assert_eq!(outcome.yields_performed, 0);
        assert_eq!(observe_calls, 1);
        assert_eq!(yield_calls, 0);
    }

    // FALSIFIABILITY: delete the time-budget check from
    // `drive_pointer_slot_quiesce_loop` and this test fails -- the loop runs
    // to the million-yield budget instead of stopping at 10 yields. This is the
    // Finding-3 property: iterations are a proxy, wall clock is the bound that
    // actually holds, and the yield ceiling must not be able to override it.
    #[test]
    fn the_real_loop_stops_on_the_time_budget_long_before_the_yield_budget() {
        let (outcome, observe_calls, yield_calls) = run_real_quiesce_loop(
            1_000_000,
            Duration::from_millis(100),
            &[1],
            Duration::from_millis(10),
        );
        assert_eq!(
            outcome.stopped_on,
            Pe64ImportSlotQuiesceStop::TimeBudgetExhausted
        );
        assert_eq!(outcome.yields_performed, 10);
        assert_eq!(outcome.observations, 11);
        assert_eq!(outcome.elapsed, Duration::from_millis(100));
        assert_eq!(observe_calls, 11);
        assert_eq!(yield_calls, 10);
    }

    // FALSIFIABILITY: remove the `break`-equivalent early return on a zero
    // active-callback count and this test fails -- the loop would keep yielding
    // past the observation that already showed quiescence.
    #[test]
    fn the_real_loop_stops_at_the_observation_that_sees_zero() {
        let (outcome, observe_calls, yield_calls) = run_real_quiesce_loop(
            1_000,
            Duration::from_secs(3_600),
            &[3, 2, 1, 0],
            Duration::ZERO,
        );
        assert_eq!(outcome.stopped_on, Pe64ImportSlotQuiesceStop::Quiesced);
        assert_eq!(outcome.observations, 4);
        assert_eq!(outcome.yields_performed, 3);
        assert_eq!(observe_calls, 4);
        assert_eq!(yield_calls, 3);
    }

    // FALSIFIABILITY: swallow the observation error in
    // `drive_pointer_slot_quiesce_loop` -- treat a failed dispatch
    // observation as a non-zero count and keep looping -- and this test fails.
    // A dispatch observation that cannot be taken is not evidence of activity.
    #[test]
    fn the_real_loop_propagates_a_failed_dispatch_observation() {
        let outcome = drive_pointer_slot_quiesce_loop(
            5,
            Duration::from_secs(1),
            || Err::<u64, &str>("dispatch observation failed"),
            || panic!("must not yield after a failed observation"),
            || Duration::ZERO,
        );
        assert_eq!(outcome, Err("dispatch observation failed"));
    }

    // ---- Refusal messages ------------------------------------------------

    // FALSIFIABILITY: fold any of these refusals into an existing message
    // string, or give two of them the same text, and this test fails. Property
    // A3 requires the unwitnessed-restore case to carry its own message rather
    // than reusing the generic "restore CAS expected shim" one, which is what a
    // reader of the receipt log has to distinguish them by.
    #[test]
    fn each_custody_refusal_renders_a_distinct_message() {
        let unwitnessed = PointerSlotRestoreCustodyError::UnwitnessedForeignRestore {
            observed_slot_value: ORIGINAL,
        }
        .to_string();
        let diverged = PointerSlotRestoreCustodyError::RecordedRestoreCasSlotDiverged {
            observed_slot_value: SHIM,
            expected_original_address: ORIGINAL,
        }
        .to_string();
        let indistinguishable =
            PointerSlotRestoreCustodyError::IndistinguishableShimAndOriginalAddress {
                address: ORIGINAL,
            }
            .to_string();
        let unreconciled =
            PointerSlotRestoreCustodyError::RecordedRestoreProtectionUnreconciled {
                protection_before: PROTECTION_BEFORE,
                protection_after: PROTECTION_TEMPORARY,
            }
            .to_string();
        let not_retained = PointerSlotRestoreCustodyError::RecordedRestoreCasNotRetained {
            retained: None,
            performed: recorded_cas(),
        }
        .to_string();
        let reprotected_before =
            PointerSlotRestoreCustodyError::RestoreCasProtectionBeforeDivergedFromLease {
                lease_protection: LEASE_PROTECTION,
                cas_protection_before: EXTERNALLY_REPROTECTED,
            }
            .to_string();
        let reprotected_after =
            PointerSlotRestoreCustodyError::RestoreCasProtectionAfterDivergedFromLease {
                lease_protection: LEASE_PROTECTION,
                cas_protection_after: EXTERNALLY_REPROTECTED,
            }
            .to_string();
        let bound = Pe64ImportSlotQuiesceBoundError::YieldBudgetAboveCeiling {
            requested: 9,
            ceiling: MAX_POINTER_SLOT_QUIESCE_YIELDS,
        }
        .to_string();

        assert!(
            not_retained.contains("did not retain")
                && not_retained.contains(&format!("0x{RECORDED_SENTINEL_BEFORE:X}")),
            "not-retained message must name the record the exchange produced: {not_retained}"
        );
        assert!(
            reprotected_before.contains("before the exchange")
                && reprotected_before.contains(&format!("0x{EXTERNALLY_REPROTECTED:X}"))
                && reprotected_before.contains(&format!("0x{LEASE_PROTECTION:X}")),
            "re-protected-before message must report both protections: {reprotected_before}"
        );
        assert!(
            reprotected_after.contains(&format!("0x{EXTERNALLY_REPROTECTED:X}")),
            "re-protected-after message lost the observed protection: {reprotected_after}"
        );
        assert!(
            unwitnessed.contains("no restore CAS recorded by this crate"),
            "unwitnessed message lost its distinguishing text: {unwitnessed}"
        );
        assert!(
            !unwitnessed.contains("restore CAS expected shim"),
            "unwitnessed message was folded into the generic CAS-mismatch text: {unwitnessed}"
        );
        assert!(
            diverged.contains("recorded restore CAS"),
            "diverged message lost its distinguishing text: {diverged}"
        );
        assert!(
            indistinguishable.contains("share address"),
            "indistinguishable message lost its distinguishing text: {indistinguishable}"
        );
        assert!(
            unreconciled.contains("page protection"),
            "unreconciled message lost its distinguishing text: {unreconciled}"
        );
        assert!(
            bound.contains('9')
                && bound.contains(&MAX_POINTER_SLOT_QUIESCE_YIELDS.to_string()),
            "bound message must report both the request and the ceiling: {bound}"
        );

        let rendered = [
            &unwitnessed,
            &diverged,
            &indistinguishable,
            &unreconciled,
            &not_retained,
            &reprotected_before,
            &reprotected_after,
            &bound,
        ];
        for i in 0..rendered.len() {
            for j in (i + 1)..rendered.len() {
                assert_ne!(rendered[i], rendered[j], "two refusals render identically");
            }
        }
    }

    // FALSIFIABILITY: give two `Pe64ImportSlotQuiesceStop` variants the same
    // rendering, or drop the reason from the timeout message's format string,
    // and this test fails. The timeout message says which bound fired, and a
    // yield-budget timeout and a time-budget timeout call for different
    // responses from the caller.
    #[test]
    fn each_quiesce_stop_reason_renders_distinctly() {
        let rendered = [
            Pe64ImportSlotQuiesceStop::Quiesced.to_string(),
            Pe64ImportSlotQuiesceStop::YieldBudgetExhausted.to_string(),
            Pe64ImportSlotQuiesceStop::TimeBudgetExhausted.to_string(),
        ];
        assert!(rendered[1].contains("yield"), "{}", rendered[1]);
        assert!(rendered[2].contains("time"), "{}", rendered[2]);
        for i in 0..rendered.len() {
            for j in (i + 1)..rendered.len() {
                assert_ne!(rendered[i], rendered[j], "two stop reasons render identically");
            }
        }
    }
}

#[cfg(test)]
mod pointer_slot_provenance_tests {
    use super::*;
    use crate::engine_slot::{
        admit_engine_owned_slot_site_roster, EngineOwnedSlotSiteRoster,
        ExpectedEngineOwnedFunctionPointerSlot, SlotEnumerationCompleteness,
        SlotEnumerationEvidence,
    };

    const TARGET_RVA: u64 = 0x00C6_E270;
    const VTABLE_RVA: u64 = 0x0294_3AD0;
    const SLOT_RVA: u64 = 0x0294_3C40;
    const MODULE_BASE: usize = 0x1_4000_0000;

    static CENSUS: SlotEnumerationEvidence = SlotEnumerationEvidence {
        evidence_job_id: "provenance-test",
        evidence_artifact_sha256: [0x5E; 32],
        target_function_rva: TARGET_RVA,
        slot_rvas: &[SLOT_RVA],
        direct_call_bypasses: &[],
        unwind_metadata_rvas: &[],
        completeness: SlotEnumerationCompleteness::Complete,
    };

    static SITES: [ExpectedEngineOwnedFunctionPointerSlot; 1] =
        [ExpectedEngineOwnedFunctionPointerSlot {
            logical_name: "provenance.test.slot",
            module_sha256: A729_EXE_SHA256,
            slot_rva: SLOT_RVA,
            owner: OwningStructureIdentity {
                kind: OwningStructureKind::Vtable,
                base_rva: VTABLE_RVA,
                entry_count: 59,
                entry_index: 46,
            },
            expected_original_target_rva: TARGET_RVA,
            enumeration: &CENSUS,
        }];

    fn engine_owned_expected() -> ExpectedPointerSlot {
        let roster = admit_engine_owned_slot_site_roster(EngineOwnedSlotSiteRoster {
            target_logical_name: "provenance.test.target",
            target_function_rva: TARGET_RVA,
            sites: &SITES,
        })
        .expect("the single-site roster covers its census exactly");
        engine_owned_expected_pointer_slot(&roster.sites()[0], MODULE_BASE)
            .expect("a roster-admitted site widens")
    }

    const IMPORT_EXPECTED: ExpectedPe64ImportAddressSlot = ExpectedPe64ImportAddressSlot {
        logical_name: "provenance.test.import",
        slot_rva: SLOT_RVA,
        imported_dll: "d3d11.dll",
        imported_symbol: "D3D11CreateDevice",
        expected_original_address: 0x7FFA_1111_0000,
    };

    // FALSIFIABILITY: make `SlotProvenance::primitive_id` return the PE64
    // constant for both variants and this fails. It is the "do not attest an
    // engine-owned slot as a PE import" rule expressed as a test: the two
    // provenances must not be able to claim each other's primitive or policy
    // id, and only the import variant may report `is_pe64_import`.
    #[test]
    fn an_engine_owned_slot_never_attests_a_pe_import_identity() {
        let engine_owned = engine_owned_expected();
        assert!(!engine_owned.provenance.is_pe64_import());
        assert_eq!(
            engine_owned.provenance.primitive_id(),
            ENGINE_OWNED_FUNCTION_POINTER_SLOT_PRIMITIVE_ID
        );
        assert_eq!(
            engine_owned.provenance.policy_id(),
            ENGINE_OWNED_FUNCTION_POINTER_SLOT_PROCESS_LIFETIME_POLICY_ID
        );
        assert_ne!(
            engine_owned.provenance.primitive_id(),
            PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS_PRIMITIVE_ID
        );
        assert_ne!(
            engine_owned.provenance.policy_id(),
            PE64_IMPORT_ADDRESS_SLOT_PROCESS_LIFETIME_POLICY_ID
        );

        let import = IMPORT_EXPECTED.generalize();
        assert!(import.provenance.is_pe64_import());
        assert_eq!(
            import.provenance.primitive_id(),
            PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS_PRIMITIVE_ID
        );
        assert_eq!(
            import.provenance.policy_id(),
            PE64_IMPORT_ADDRESS_SLOT_PROCESS_LIFETIME_POLICY_ID
        );
    }

    // The forged-attestation case is closed by construction rather than by a
    // check: `EngineOwnedSlotAttestation`'s fields are private and its only
    // constructor is `from_roster_admitted_site`, so no code outside this crate
    // can build `SlotProvenance::EngineOwnedFunctionPointer` at all. This test
    // therefore cannot be written from outside; it stands in for the remaining
    // in-crate hazard, which is a specification edited *after* admission.
    //
    // FALSIFIABILITY: delete the `owner().derived_slot_rva()` and
    // `entry_index >= entry_count` arms of `validate_expected_pointer_slot`'s
    // `EngineOwnedFunctionPointer` match and this fails -- a specification
    // whose slot address no longer matches the owner its attestation names
    // FALSIFIABILITY: make `EngineOwnedSlotRosterInstallation::finish` return
    // `Ok` regardless of `required_slot_rvas` -- i.e. hand out receipts for a
    // partially redirected roster -- and this fails. A two-slot target with one
    // slot redirected is an observer that misses every call through the other
    // slot, so no usable receipt may exist until coverage is complete.
    #[cfg(windows)]
    #[test]
    fn a_partially_installed_roster_yields_no_receipts() {
        const SECOND_SLOT: u64 = 0x0294_3C48;
        static TWO_SLOTS: SlotEnumerationEvidence = SlotEnumerationEvidence {
            evidence_job_id: "provenance-test",
            evidence_artifact_sha256: [0x5E; 32],
            target_function_rva: TARGET_RVA,
            slot_rvas: &[SLOT_RVA, SECOND_SLOT],
            direct_call_bypasses: &[],
            unwind_metadata_rvas: &[],
            completeness: SlotEnumerationCompleteness::Complete,
        };
        static PAIR: [ExpectedEngineOwnedFunctionPointerSlot; 2] = [
            ExpectedEngineOwnedFunctionPointerSlot {
                enumeration: &TWO_SLOTS,
                ..SITES[0]
            },
            ExpectedEngineOwnedFunctionPointerSlot {
                logical_name: "provenance.test.slot.second",
                slot_rva: SECOND_SLOT,
                owner: OwningStructureIdentity {
                    entry_index: 47,
                    ..SITES[0].owner
                },
                enumeration: &TWO_SLOTS,
                ..SITES[0]
            },
        ];
        let roster = admit_engine_owned_slot_site_roster(EngineOwnedSlotSiteRoster {
            target_logical_name: "provenance.test.target",
            target_function_rva: TARGET_RVA,
            sites: &PAIR,
        })
        .expect("the pair covers its census exactly");

        let installation = EngineOwnedSlotRosterInstallation::begin(&roster, MODULE_BASE, 7)
            .expect("a two-site roster begins");
        assert_eq!(installation.uncovered_slot_rvas(), &[SLOT_RVA, SECOND_SLOT]);
        let incomplete = installation
            .finish()
            .expect_err("no slot was redirected, so there is no complete installation");
        assert!(incomplete.installed.is_empty());
        assert_eq!(incomplete.uncovered_slot_rvas, vec![SLOT_RVA, SECOND_SLOT]);
        assert_eq!(incomplete.roster_sha256, roster.roster_sha256());

        // A zero teardown token can never own a redirection.
        assert!(EngineOwnedSlotRosterInstallation::begin(&roster, MODULE_BASE, 0).is_err());
    }

    // FALSIFIABILITY: delete the `unresolved_slot_rvas.push(slot_rva)` that
    // `install_site` performs *before* attempting the install, and this fails.
    // The exchange is recorded before the page-protection cleanup that can fail
    // after it, so an `Err` from `install_site` does not prove the slot is
    // untouched. Reporting such a slot as merely "uncovered" would tell a
    // caller nothing happened when a live redirection may exist with no receipt
    // and therefore no teardown path.
    #[cfg(windows)]
    #[test]
    fn a_failed_install_attempt_is_reported_as_unresolved_not_merely_uncovered() {
        let roster = admit_engine_owned_slot_site_roster(EngineOwnedSlotSiteRoster {
            target_logical_name: "provenance.test.target",
            target_function_rva: TARGET_RVA,
            sites: &SITES,
        })
        .expect("the single-site roster is admitted");
        let mut installation = EngineOwnedSlotRosterInstallation::begin(&roster, MODULE_BASE, 11)
            .expect("the roster begins");
        assert!(installation.unresolved_slot_rvas().is_empty());

        // A shim that fails its own seal check, so nothing is exchanged -- but
        // the bookkeeping cannot know that, and must not assume it.
        installation
            .install_site(&roster.sites()[0], &foreign_shim())
            .expect_err("an unsealed shim cannot install");
        assert_eq!(installation.unresolved_slot_rvas(), &[SLOT_RVA]);
        assert_eq!(installation.uncovered_slot_rvas(), &[SLOT_RVA]);

        let incomplete = installation.finish().expect_err("nothing was redirected");
        assert_eq!(incomplete.unresolved_slot_rvas, vec![SLOT_RVA]);
        assert_eq!(incomplete.uncovered_slot_rvas, vec![SLOT_RVA]);
        assert!(incomplete.installed.is_empty());
        // Every unresolved slot is by construction still uncovered.
        assert!(incomplete
            .unresolved_slot_rvas
            .iter()
            .all(|slot| incomplete.uncovered_slot_rvas.contains(slot)));
    }

    // FALSIFIABILITY: delete the `site.roster_sha256() != self.roster_sha256`
    // refusal in `install_site` and a site admitted under one target's coverage
    // proof counts towards a different target's, which would let a roster
    // "complete" without its own slots ever being redirected.
    #[cfg(windows)]
    #[test]
    fn a_site_from_another_roster_cannot_satisfy_this_one() {
        let roster = admit_engine_owned_slot_site_roster(EngineOwnedSlotSiteRoster {
            target_logical_name: "provenance.test.target",
            target_function_rva: TARGET_RVA,
            sites: &SITES,
        })
        .expect("the single-site roster is admitted");
        let other = admit_engine_owned_slot_site_roster(EngineOwnedSlotSiteRoster {
            target_logical_name: "provenance.test.other-target",
            target_function_rva: TARGET_RVA,
            sites: &SITES,
        })
        .expect("a differently named roster over the same census is admitted");
        assert_ne!(roster.roster_sha256(), other.roster_sha256());

        let mut installation = EngineOwnedSlotRosterInstallation::begin(&roster, MODULE_BASE, 9)
            .expect("the roster begins");
        // Refused on roster identity, before any process memory is touched.
        let error = installation
            .install_site(&other.sites()[0], &foreign_shim())
            .expect_err("a foreign site cannot count towards this roster");
        assert!(error.contains("different admitted roster"), "{error}");
        assert_eq!(installation.uncovered_slot_rvas(), &[SLOT_RVA]);
    }

    /// A shim value the roster-identity check must reject before ever reading
    /// it. Never dereferenced: `install_site` returns on the roster mismatch
    /// above it.
    #[cfg(windows)]
    fn foreign_shim() -> SealedExternalDispatchShim {
        SealedExternalDispatchShim {
            identity: ExternalDispatchShimIdentity {
                entry_address: 0,
                entry_len: 0,
                entry_bytes_sha256: [0; 32],
                allocation_base: 0,
                allocation_len: 0,
                dispatch_slot_address: 0,
                active_callback_counter_address: 0,
            },
            template_id: WIN64_REGISTER_ARGS_COUNTED_SHIM_TEMPLATE_ID,
            callback_semantic_id: "provenance.test.callback",
            callback_export_name: "provenance_test_callback",
            callback_binary_identity: ExternalCallbackBinaryIdentity {
                module_file_sha256: [0; 32],
                module_pe_identity_sha256: [0; 32],
                module_size_of_image: 0,
                callback_rva: 0,
                entry_prefix_len: 0,
                entry_prefix_sha256: [0; 32],
            },
            callback_manifest_sha256: [0; 32],
            normalized_template_sha256: [0; 32],
            unwind_handler_sha256: [0; 32],
            unwind_info_sha256: [0; 32],
            runtime_function_sha256: [0; 32],
            original_address: 0,
            callback_address: 0,
            unwind_registered: false,
            authenticity_seal: 0,
        }
    }


    // FALSIFIABILITY: change `rva_is_in_executable_section` back to bounding
    // with `raw_size` and this fails. The two sizes are independent: a729's
    // `.text` pads its raw size 0x1D8 bytes past its virtual size (so raw_size
    // over-accepts), while a zero-filled section runs the other way (so
    // raw_size under-accepts and would reject a real code RVA).
    #[cfg(windows)]
    #[test]
    fn executable_containment_uses_the_loaded_extent_not_the_disk_extent() {
        // a729 `.text`: VirtualSize 0x026EFC28, SizeOfRawData 0x026EFE00.
        let padded = PeSection {
            virtual_address: 0x1000,
            virtual_size: 0x026E_FC28,
            raw_size: 0x026E_FE00,
            raw_offset: 0x400,
            characteristics: IMAGE_SCN_MEM_EXECUTE,
        };
        let sections = [padded];
        assert!(rva_is_in_executable_section(
            &sections,
            0x1000 + 0x026E_FC27
        ));
        // Inside the raw extent, past the loaded extent: not in the section.
        assert!(!rva_is_in_executable_section(
            &sections,
            0x1000 + 0x026E_FC28
        ));
        assert!(!rva_is_in_executable_section(
            &sections,
            0x1000 + 0x026E_FDFF
        ));

        // Zero-filled section: the loaded extent is the larger one.
        let zero_filled = PeSection {
            virtual_address: 0x2000,
            virtual_size: 0x4000,
            raw_size: 0x1000,
            raw_offset: 0x400,
            characteristics: IMAGE_SCN_MEM_EXECUTE,
        };
        let sections = [zero_filled];
        assert!(rva_is_in_executable_section(&sections, 0x2000 + 0x3FFF));

        // A non-executable section never satisfies the predicate.
        let data = PeSection {
            characteristics: 0,
            ..padded
        };
        assert!(!rva_is_in_executable_section(&[data], 0x1000));
    }

    // would reach the compare-exchange, redirecting a slot nothing admitted.
    #[test]
    fn an_engine_owned_specification_edited_after_admission_is_refused() {
        let admitted = engine_owned_expected();
        assert!(validate_expected_pointer_slot(admitted).is_ok());

        // Same attestation, slot address moved one entry along.
        let mut moved = admitted;
        moved.slot_rva += ENGINE_OWNED_SLOT_WIDTH_BYTES;
        assert!(validate_expected_pointer_slot(moved).is_err());

        // An attestation whose own owner index is outside its structure.
        let mut out_of_range = admitted;
        out_of_range.provenance =
            SlotProvenance::EngineOwnedFunctionPointer(EngineOwnedSlotAttestation {
                owner: OwningStructureIdentity {
                    entry_count: 46,
                    ..SITES[0].owner
                },
                enumeration_sha256: [0x11; 32],
                roster_sha256: [0x22; 32],
                roster_slot_count: 1,
            });
        assert!(validate_expected_pointer_slot(out_of_range).is_err());

        // Zeroed digests and an empty roster never come out of a roster
        // admission; each is refused on its own.
        for forged in [
            EngineOwnedSlotAttestation {
                owner: SITES[0].owner,
                enumeration_sha256: [0x11; 32],
                roster_sha256: [0; 32],
                roster_slot_count: 1,
            },
            EngineOwnedSlotAttestation {
                owner: SITES[0].owner,
                enumeration_sha256: [0; 32],
                roster_sha256: [0x22; 32],
                roster_slot_count: 1,
            },
            EngineOwnedSlotAttestation {
                owner: SITES[0].owner,
                enumeration_sha256: [0x11; 32],
                roster_sha256: [0x22; 32],
                roster_slot_count: 0,
            },
        ] {
            let mut spec = admitted;
            spec.provenance = SlotProvenance::EngineOwnedFunctionPointer(forged);
            assert!(validate_expected_pointer_slot(spec).is_err());
        }
    }

    // FALSIFIABILITY: drop the per-provenance domain string and the variant tag
    // from `pointer_slot_spec_sha256`/`digest_slot_provenance` and these two
    // specifications -- same name, same slot, same original address -- digest
    // identically, so one receipt could stand in for the other.
    #[test]
    fn the_slot_spec_digest_separates_the_two_provenances() {
        let engine_owned = engine_owned_expected();
        let mut import = IMPORT_EXPECTED.generalize();
        import.logical_name = engine_owned.logical_name;
        import.slot_rva = engine_owned.slot_rva;
        import.expected_original_address = engine_owned.expected_original_address;
        assert_ne!(
            pointer_slot_spec_sha256(engine_owned),
            pointer_slot_spec_sha256(import)
        );
    }

    // FALSIFIABILITY: delete the `expected.module_sha256 != A729_EXE_SHA256`
    // refusal in `engine_owned_expected_pointer_slot` and a site pinned to a
    // different build widens into an installable specification against the
    // build this crate is compiled for.
    #[test]
    fn widening_refuses_a_site_pinned_to_another_build() {
        static OTHER_BUILD: [ExpectedEngineOwnedFunctionPointerSlot; 1] =
            [ExpectedEngineOwnedFunctionPointerSlot {
                module_sha256: [0x5A; 32],
                ..SITES[0]
            }];
        let roster = admit_engine_owned_slot_site_roster(EngineOwnedSlotSiteRoster {
            target_logical_name: "provenance.test.target",
            target_function_rva: TARGET_RVA,
            sites: &OTHER_BUILD,
        })
        .expect("admission is build-agnostic; the build pin is checked at widening");
        assert!(engine_owned_expected_pointer_slot(&roster.sites()[0], MODULE_BASE).is_err());
        assert!(engine_owned_expected_pointer_slot(&roster.sites()[0], 0).is_err());
    }

    // FALSIFIABILITY: drop the `slot_rva % ENGINE_OWNED_SLOT_WIDTH_BYTES`
    // refusal from `validate_expected_pointer_slot` and an unaligned slot
    // reaches the compare/exchange, where a straddling word is not atomic at
    // all.
    #[test]
    fn an_unaligned_slot_specification_is_refused() {
        let mut unaligned = IMPORT_EXPECTED.generalize();
        unaligned.slot_rva = SLOT_RVA + 4;
        assert!(validate_expected_pointer_slot(unaligned).is_err());
    }
}
