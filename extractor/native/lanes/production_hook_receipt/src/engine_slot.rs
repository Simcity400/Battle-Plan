//! Build-agnostic admission contract for engine-owned function-pointer and
//! vtable slots.
//!
//! The PE64 import-address-slot sites the crate already installs get their
//! provenance for free: an import thunk names the DLL and symbol it dispatches
//! to, the loader is the only writer, and the loader's own IAT is by
//! construction the complete set of slots through which an imported function is
//! reached from this image. None of that holds for a function the engine owns.
//! An internal function has no thunk naming it, its pointer can be replicated
//! into any number of engine-owned tables, and it can also be reached by a
//! direct `CALL rel32` that never loads a pointer at all.
//!
//! That last case is the one this module exists for. A pointer-slot hook
//! observes exactly the calls that dispatch *through* the slot. Calls that
//! reach the same function directly are invisible to it, and an observer that
//! silently misses them under-reports without ever failing. So a site is
//! admitted only against an enumeration that reached a fixed point over *every*
//! incoming edge, and only when that enumeration found no direct-call bypass to
//! be blind to. An enumeration that did not converge, or that converged on a
//! target which is also called directly, is refused here rather than installed
//! and quietly trusted.
//!
//! Nothing in this module is PE-import-specific and nothing it produces may be
//! attested as a PE import. The two genuine import sites keep their own
//! `ExpectedPe64ImportAddressSlot` front and their own `pe64.…` primitive id;
//! this front carries `ENGINE_OWNED_FUNCTION_POINTER_SLOT_PRIMITIVE_ID`.

use sha2::{Digest, Sha256};

/// Primitive id for an engine-owned function-pointer or vtable slot redirected
/// by a naturally aligned atomic compare/exchange whose dispatch shim,
/// callback, and unwind allocations are retained until process exit.
pub const ENGINE_OWNED_FUNCTION_POINTER_SLOT_PRIMITIVE_ID: &str =
    "engine-owned-function-pointer-slot.aligned-usize-cas.process-lifetime.v1";

/// Teardown policy id: the slot returns to the engine's own pointer by an
/// atomic compare/exchange, and every allocation a target thread could still be
/// executing inside stays mapped for the life of the process.
pub const ENGINE_OWNED_FUNCTION_POINTER_SLOT_PROCESS_LIFETIME_POLICY_ID: &str =
    "engine-owned-function-pointer-slot.external-dispatch.process-lifetime.v1";

/// Width of one slot. The primitive is 64-bit only: it exchanges exactly one
/// naturally aligned pointer-sized word.
pub const ENGINE_OWNED_SLOT_WIDTH_BYTES: u64 = 8;

/// What kind of structure owns the slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum OwningStructureKind {
    /// A C++ virtual function table. `base_rva` is the address an object's
    /// first quadword points at, which is what makes the slot's identity
    /// checkable independently of the slot address itself.
    Vtable,
    /// An engine-owned array of function pointers that is not a vtable.
    FunctionPointerTable,
}

/// Identity of the structure that owns the slot, so a receipt names *what* was
/// redirected and not merely *where*. A bare address cannot be reviewed; a
/// vtable base plus an entry index can, because the derived slot address must
/// reproduce the recorded one exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct OwningStructureIdentity {
    pub kind: OwningStructureKind,
    pub base_rva: u64,
    pub entry_count: u32,
    pub entry_index: u32,
}

impl OwningStructureIdentity {
    /// The slot address this owner identity implies, or `None` on overflow.
    pub fn derived_slot_rva(&self) -> Option<u64> {
        u64::from(self.entry_index)
            .checked_mul(ENGINE_OWNED_SLOT_WIDTH_BYTES)
            .and_then(|offset| self.base_rva.checked_add(offset))
    }
}

/// One incoming edge that reaches the target *without* loading an admitted
/// slot. A pointer-slot observer cannot see these, which is why recording one
/// refuses the site instead of annotating it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DirectCallBypass {
    pub caller_function_rva: u64,
    pub call_site_rva: u64,
}

/// Whether the incoming-edge enumeration reached a fixed point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotEnumerationCompleteness {
    /// Every reference to the target entry was recovered and classified as a
    /// dispatch slot, a direct call, or non-dispatch metadata.
    Complete,
    /// The enumeration did not converge. A site resting on it is refused.
    Incomplete,
}

/// The recovered incoming-edge census for one target function.
///
/// This is evidence, not configuration: `evidence_job_id` and
/// `evidence_artifact_sha256` name the exact read-only analysis artifact the
/// three lists were transcribed from, so a reviewer can re-derive them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlotEnumerationEvidence {
    pub evidence_job_id: &'static str,
    pub evidence_artifact_sha256: [u8; 32],
    /// Entry RVA of the function the slots dispatch to.
    pub target_function_rva: u64,
    /// Every engine-owned pointer slot holding the target, ascending.
    pub slot_rvas: &'static [u64],
    /// Every incoming edge that is *not* a slot load. Must be empty.
    pub direct_call_bypasses: &'static [DirectCallBypass],
    /// References classified as unwind/`.pdata` metadata. Recorded so that the
    /// classification of every non-slot reference is auditable. These are never
    /// dispatch and are never patched.
    pub unwind_metadata_rvas: &'static [u64],
    pub completeness: SlotEnumerationCompleteness,
}

/// One requested engine-owned slot site, pinned to an exact module image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpectedEngineOwnedFunctionPointerSlot {
    pub logical_name: &'static str,
    /// SHA-256 of the exact module file this slot address was recovered from.
    pub module_sha256: [u8; 32],
    pub slot_rva: u64,
    pub owner: OwningStructureIdentity,
    /// Entry RVA of the function the slot is expected to hold *before*
    /// installation. The CAS compares against this and never blind-stores.
    pub expected_original_target_rva: u64,
    pub enumeration: &'static SlotEnumerationEvidence,
}

/// Every slot for one target, declared together.
///
/// Declaring the slots one at a time is what makes partial coverage possible,
/// so the primitive does not offer that shape: only a roster mints installable
/// sites, and a roster is admitted only when it covers its enumeration exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineOwnedSlotSiteRoster {
    pub target_logical_name: &'static str,
    pub target_function_rva: u64,
    pub sites: &'static [ExpectedEngineOwnedFunctionPointerSlot],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineOwnedSlotAdmissionError {
    EmptyLogicalName,
    LogicalNameIsNotAscii,
    ModuleIdentityNotPinned,
    SlotRvaIsZero,
    SlotRvaNotPointerAligned {
        slot_rva: u64,
    },
    TargetRvaIsZero,
    OwningStructureBaseIsZero,
    OwningStructureBaseNotPointerAligned {
        base_rva: u64,
    },
    OwningStructureEntryCountIsZero,
    OwningStructureEntryIndexOutOfRange {
        entry_index: u32,
        entry_count: u32,
    },
    /// The declared owner does not derive the declared slot. Either the slot
    /// address or the structure identity is wrong, and a receipt naming both
    /// would be internally inconsistent.
    SlotRvaIsNotTheOwningStructureEntry {
        slot_rva: u64,
        derived_slot_rva: Option<u64>,
    },
    /// The incoming-edge census did not converge, so the slot roster below it
    /// is a lower bound rather than a census.
    EnumerationIncomplete,
    EnumerationJobIdMissing,
    EnumerationArtifactDigestMissing,
    EnumerationTargetMismatch {
        enumeration_target_rva: u64,
        expected_original_target_rva: u64,
    },
    EnumerationSlotRosterEmpty,
    /// Ascending order with no repeats: a census with a duplicate has not been
    /// deduplicated and its length is not a slot count.
    EnumerationSlotRosterNotStrictlyAscending {
        index: usize,
    },
    EnumerationSlotRosterEntryNotPointerAligned {
        slot_rva: u64,
    },
    /// This site names a slot its own enumeration does not list.
    EnumerationSlotRosterExcludesThisSlot {
        slot_rva: u64,
    },
    /// The target is also reached without loading a slot. A pointer-slot
    /// observer is structurally blind to these calls, so the site is refused
    /// rather than installed and silently under-reported.
    UncoveredDirectCallBypass {
        bypass_count: usize,
        first_call_site_rva: u64,
    },
}

impl std::fmt::Display for EngineOwnedSlotAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyLogicalName => write!(formatter, "engine-owned slot has no logical name"),
            Self::LogicalNameIsNotAscii => {
                write!(formatter, "engine-owned slot logical name is not ASCII")
            }
            Self::ModuleIdentityNotPinned => write!(
                formatter,
                "engine-owned slot is not pinned to an exact module SHA-256"
            ),
            Self::SlotRvaIsZero => write!(formatter, "engine-owned slot RVA is zero"),
            Self::SlotRvaNotPointerAligned { slot_rva } => write!(
                formatter,
                "engine-owned slot RVA 0x{slot_rva:X} is not naturally pointer-aligned"
            ),
            Self::TargetRvaIsZero => {
                write!(formatter, "engine-owned slot target function RVA is zero")
            }
            Self::OwningStructureBaseIsZero => {
                write!(formatter, "engine-owned slot owning structure RVA is zero")
            }
            Self::OwningStructureBaseNotPointerAligned { base_rva } => write!(
                formatter,
                "engine-owned slot owning structure RVA 0x{base_rva:X} is not pointer-aligned"
            ),
            Self::OwningStructureEntryCountIsZero => write!(
                formatter,
                "engine-owned slot owning structure declares no entries"
            ),
            Self::OwningStructureEntryIndexOutOfRange {
                entry_index,
                entry_count,
            } => write!(
                formatter,
                "engine-owned slot entry index {entry_index} is outside its {entry_count}-entry owner"
            ),
            Self::SlotRvaIsNotTheOwningStructureEntry {
                slot_rva,
                derived_slot_rva,
            } => write!(
                formatter,
                "engine-owned slot RVA 0x{slot_rva:X} is not entry-derived 0x{:X}",
                derived_slot_rva.unwrap_or_default()
            ),
            Self::EnumerationIncomplete => write!(
                formatter,
                "engine-owned slot enumeration is incomplete; the slot roster is a lower bound"
            ),
            Self::EnumerationJobIdMissing => write!(
                formatter,
                "engine-owned slot enumeration names no evidence job"
            ),
            Self::EnumerationArtifactDigestMissing => write!(
                formatter,
                "engine-owned slot enumeration names no evidence artifact digest"
            ),
            Self::EnumerationTargetMismatch {
                enumeration_target_rva,
                expected_original_target_rva,
            } => write!(
                formatter,
                "engine-owned slot enumeration covers 0x{enumeration_target_rva:X}, not 0x{expected_original_target_rva:X}"
            ),
            Self::EnumerationSlotRosterEmpty => write!(
                formatter,
                "engine-owned slot enumeration lists no dispatch slot"
            ),
            Self::EnumerationSlotRosterNotStrictlyAscending { index } => write!(
                formatter,
                "engine-owned slot enumeration entry {index} is out of ascending order or repeated"
            ),
            Self::EnumerationSlotRosterEntryNotPointerAligned { slot_rva } => write!(
                formatter,
                "engine-owned slot enumeration entry 0x{slot_rva:X} is not pointer-aligned"
            ),
            Self::EnumerationSlotRosterExcludesThisSlot { slot_rva } => write!(
                formatter,
                "engine-owned slot 0x{slot_rva:X} is absent from its own enumeration"
            ),
            Self::UncoveredDirectCallBypass {
                bypass_count,
                first_call_site_rva,
            } => write!(
                formatter,
                "engine-owned slot target has {bypass_count} direct-call bypass(es) a slot hook cannot observe, first at 0x{first_call_site_rva:X}"
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineOwnedSlotRosterAdmissionError {
    EmptyRosterName,
    RosterHasNoSites,
    RosterTargetRvaIsZero,
    Site {
        slot_rva: u64,
        error: EngineOwnedSlotAdmissionError,
    },
    /// A site in the roster covers a different target function.
    SiteTargetMismatch {
        slot_rva: u64,
        site_target_rva: u64,
        roster_target_rva: u64,
    },
    /// Two sites cite different incoming-edge censuses for one target, so at
    /// most one of them is current.
    SiteEnumerationsDisagree {
        slot_rva: u64,
    },
    SiteModuleIdentitiesDisagree {
        slot_rva: u64,
    },
    DuplicateSlot {
        slot_rva: u64,
    },
    /// The enumeration lists a slot no site in the roster covers. Installing
    /// the rest would leave a live dispatch path unobserved.
    SlotCoverageIncomplete {
        uncovered_slot_rva: u64,
    },
}

impl std::fmt::Display for EngineOwnedSlotRosterAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyRosterName => write!(formatter, "engine-owned slot roster has no name"),
            Self::RosterHasNoSites => write!(formatter, "engine-owned slot roster declares no site"),
            Self::RosterTargetRvaIsZero => {
                write!(formatter, "engine-owned slot roster target RVA is zero")
            }
            Self::Site { slot_rva, error } => {
                write!(formatter, "engine-owned slot 0x{slot_rva:X}: {error}")
            }
            Self::SiteTargetMismatch {
                slot_rva,
                site_target_rva,
                roster_target_rva,
            } => write!(
                formatter,
                "engine-owned slot 0x{slot_rva:X} targets 0x{site_target_rva:X}, roster targets 0x{roster_target_rva:X}"
            ),
            Self::SiteEnumerationsDisagree { slot_rva } => write!(
                formatter,
                "engine-owned slot 0x{slot_rva:X} cites a different enumeration than its roster"
            ),
            Self::SiteModuleIdentitiesDisagree { slot_rva } => write!(
                formatter,
                "engine-owned slot 0x{slot_rva:X} cites a different module identity than its roster"
            ),
            Self::DuplicateSlot { slot_rva } => write!(
                formatter,
                "engine-owned slot 0x{slot_rva:X} is declared twice in one roster"
            ),
            Self::SlotCoverageIncomplete { uncovered_slot_rva } => write!(
                formatter,
                "engine-owned slot 0x{uncovered_slot_rva:X} is enumerated but not covered by the roster"
            ),
        }
    }
}

/// A site that passed the per-site admission gate.
///
/// This is deliberately *not* installable on its own. Complete slot coverage is
/// a property of the roster, not of any one site, so only
/// [`admit_engine_owned_slot_site_roster`] mints the installable form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmittedEngineOwnedSlotSite {
    expected: ExpectedEngineOwnedFunctionPointerSlot,
    spec_sha256: [u8; 32],
    enumeration_sha256: [u8; 32],
}

impl AdmittedEngineOwnedSlotSite {
    pub fn expected(&self) -> ExpectedEngineOwnedFunctionPointerSlot {
        self.expected
    }

    pub fn spec_sha256(&self) -> [u8; 32] {
        self.spec_sha256
    }

    pub fn enumeration_sha256(&self) -> [u8; 32] {
        self.enumeration_sha256
    }
}

/// One site of an admitted roster: the only installable engine-owned slot form.
///
/// The private `roster_sha256` cannot be produced outside this module, so an
/// install path that accepts this type accepts only sites whose whole target
/// roster was proved complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RosterAdmittedEngineOwnedSlotSite {
    site: AdmittedEngineOwnedSlotSite,
    roster_sha256: [u8; 32],
    roster_slot_count: usize,
}

impl RosterAdmittedEngineOwnedSlotSite {
    pub fn expected(&self) -> ExpectedEngineOwnedFunctionPointerSlot {
        self.site.expected
    }

    pub fn spec_sha256(&self) -> [u8; 32] {
        self.site.spec_sha256
    }

    pub fn enumeration_sha256(&self) -> [u8; 32] {
        self.site.enumeration_sha256
    }

    /// Digest over the whole admitted roster. Every site of one roster carries
    /// the same value, so a receipt names the coverage proof it rests on.
    pub fn roster_sha256(&self) -> [u8; 32] {
        self.roster_sha256
    }

    pub fn roster_slot_count(&self) -> usize {
        self.roster_slot_count
    }
}

/// A whole target's slot coverage, proved complete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmittedEngineOwnedSlotRoster {
    target_logical_name: &'static str,
    target_function_rva: u64,
    roster_sha256: [u8; 32],
    sites: Vec<RosterAdmittedEngineOwnedSlotSite>,
}

impl AdmittedEngineOwnedSlotRoster {
    pub fn target_logical_name(&self) -> &'static str {
        self.target_logical_name
    }

    pub fn target_function_rva(&self) -> u64 {
        self.target_function_rva
    }

    pub fn roster_sha256(&self) -> [u8; 32] {
        self.roster_sha256
    }

    pub fn sites(&self) -> &[RosterAdmittedEngineOwnedSlotSite] {
        &self.sites
    }
}

fn digest_len_prefixed(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

/// Digest of one incoming-edge census, computed here and never transcribed.
pub fn slot_enumeration_sha256(evidence: &SlotEnumerationEvidence) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.engine_owned_slot_enumeration.v1");
    digest_len_prefixed(&mut digest, evidence.evidence_job_id.as_bytes());
    digest.update(evidence.evidence_artifact_sha256);
    digest.update(evidence.target_function_rva.to_le_bytes());
    digest.update([match evidence.completeness {
        SlotEnumerationCompleteness::Complete => 1,
        SlotEnumerationCompleteness::Incomplete => 2,
    }]);
    digest.update((evidence.slot_rvas.len() as u64).to_le_bytes());
    for slot_rva in evidence.slot_rvas {
        digest.update(slot_rva.to_le_bytes());
    }
    digest.update((evidence.direct_call_bypasses.len() as u64).to_le_bytes());
    for bypass in evidence.direct_call_bypasses {
        digest.update(bypass.caller_function_rva.to_le_bytes());
        digest.update(bypass.call_site_rva.to_le_bytes());
    }
    digest.update((evidence.unwind_metadata_rvas.len() as u64).to_le_bytes());
    for metadata_rva in evidence.unwind_metadata_rvas {
        digest.update(metadata_rva.to_le_bytes());
    }
    digest.finalize().into()
}

/// Digest of one engine-owned slot specification.
pub fn engine_owned_slot_spec_sha256(
    expected: ExpectedEngineOwnedFunctionPointerSlot,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.engine_owned_function_pointer_slot_spec.v1");
    digest_len_prefixed(&mut digest, expected.logical_name.as_bytes());
    digest.update(expected.module_sha256);
    digest.update(expected.slot_rva.to_le_bytes());
    digest.update([match expected.owner.kind {
        OwningStructureKind::Vtable => 1,
        OwningStructureKind::FunctionPointerTable => 2,
    }]);
    digest.update(expected.owner.base_rva.to_le_bytes());
    digest.update(expected.owner.entry_count.to_le_bytes());
    digest.update(expected.owner.entry_index.to_le_bytes());
    digest.update(expected.expected_original_target_rva.to_le_bytes());
    digest.update(slot_enumeration_sha256(expected.enumeration));
    digest.finalize().into()
}

fn roster_sha256(
    roster: EngineOwnedSlotSiteRoster,
    sites: &[AdmittedEngineOwnedSlotSite],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.production_hook.engine_owned_slot_roster.v1");
    digest_len_prefixed(&mut digest, roster.target_logical_name.as_bytes());
    digest.update(roster.target_function_rva.to_le_bytes());
    digest.update((sites.len() as u64).to_le_bytes());
    for site in sites {
        digest.update(site.spec_sha256);
    }
    digest.finalize().into()
}

/// Admit one engine-owned slot site.
///
/// The result is intentionally not installable; see
/// [`admit_engine_owned_slot_site_roster`].
pub fn admit_engine_owned_function_pointer_slot_site(
    expected: ExpectedEngineOwnedFunctionPointerSlot,
) -> Result<AdmittedEngineOwnedSlotSite, EngineOwnedSlotAdmissionError> {
    if expected.logical_name.is_empty() {
        return Err(EngineOwnedSlotAdmissionError::EmptyLogicalName);
    }
    if !expected.logical_name.is_ascii() {
        return Err(EngineOwnedSlotAdmissionError::LogicalNameIsNotAscii);
    }
    if expected.module_sha256 == [0; 32] {
        return Err(EngineOwnedSlotAdmissionError::ModuleIdentityNotPinned);
    }
    if expected.slot_rva == 0 {
        return Err(EngineOwnedSlotAdmissionError::SlotRvaIsZero);
    }
    if expected.slot_rva % ENGINE_OWNED_SLOT_WIDTH_BYTES != 0 {
        return Err(EngineOwnedSlotAdmissionError::SlotRvaNotPointerAligned {
            slot_rva: expected.slot_rva,
        });
    }
    if expected.expected_original_target_rva == 0 {
        return Err(EngineOwnedSlotAdmissionError::TargetRvaIsZero);
    }
    if expected.owner.base_rva == 0 {
        return Err(EngineOwnedSlotAdmissionError::OwningStructureBaseIsZero);
    }
    if expected.owner.base_rva % ENGINE_OWNED_SLOT_WIDTH_BYTES != 0 {
        return Err(
            EngineOwnedSlotAdmissionError::OwningStructureBaseNotPointerAligned {
                base_rva: expected.owner.base_rva,
            },
        );
    }
    if expected.owner.entry_count == 0 {
        return Err(EngineOwnedSlotAdmissionError::OwningStructureEntryCountIsZero);
    }
    if expected.owner.entry_index >= expected.owner.entry_count {
        return Err(
            EngineOwnedSlotAdmissionError::OwningStructureEntryIndexOutOfRange {
                entry_index: expected.owner.entry_index,
                entry_count: expected.owner.entry_count,
            },
        );
    }
    let derived_slot_rva = expected.owner.derived_slot_rva();
    if derived_slot_rva != Some(expected.slot_rva) {
        return Err(
            EngineOwnedSlotAdmissionError::SlotRvaIsNotTheOwningStructureEntry {
                slot_rva: expected.slot_rva,
                derived_slot_rva,
            },
        );
    }
    let enumeration = expected.enumeration;
    if enumeration.completeness != SlotEnumerationCompleteness::Complete {
        return Err(EngineOwnedSlotAdmissionError::EnumerationIncomplete);
    }
    if enumeration.evidence_job_id.is_empty() {
        return Err(EngineOwnedSlotAdmissionError::EnumerationJobIdMissing);
    }
    if enumeration.evidence_artifact_sha256 == [0; 32] {
        return Err(EngineOwnedSlotAdmissionError::EnumerationArtifactDigestMissing);
    }
    if enumeration.target_function_rva != expected.expected_original_target_rva {
        return Err(EngineOwnedSlotAdmissionError::EnumerationTargetMismatch {
            enumeration_target_rva: enumeration.target_function_rva,
            expected_original_target_rva: expected.expected_original_target_rva,
        });
    }
    if enumeration.slot_rvas.is_empty() {
        return Err(EngineOwnedSlotAdmissionError::EnumerationSlotRosterEmpty);
    }
    for (index, slot_rva) in enumeration.slot_rvas.iter().enumerate() {
        if *slot_rva % ENGINE_OWNED_SLOT_WIDTH_BYTES != 0 {
            return Err(
                EngineOwnedSlotAdmissionError::EnumerationSlotRosterEntryNotPointerAligned {
                    slot_rva: *slot_rva,
                },
            );
        }
        if index > 0 && *slot_rva <= enumeration.slot_rvas[index - 1] {
            return Err(
                EngineOwnedSlotAdmissionError::EnumerationSlotRosterNotStrictlyAscending { index },
            );
        }
    }
    if !enumeration.slot_rvas.contains(&expected.slot_rva) {
        return Err(
            EngineOwnedSlotAdmissionError::EnumerationSlotRosterExcludesThisSlot {
                slot_rva: expected.slot_rva,
            },
        );
    }
    if let Some(first) = enumeration.direct_call_bypasses.first() {
        return Err(EngineOwnedSlotAdmissionError::UncoveredDirectCallBypass {
            bypass_count: enumeration.direct_call_bypasses.len(),
            first_call_site_rva: first.call_site_rva,
        });
    }
    Ok(AdmittedEngineOwnedSlotSite {
        expected,
        spec_sha256: engine_owned_slot_spec_sha256(expected),
        enumeration_sha256: slot_enumeration_sha256(enumeration),
    })
}

/// Admit a whole target's slots and prove the coverage complete.
///
/// Refuses unless the roster's slot set is exactly the enumeration's slot set:
/// a target with two dispatch slots and one declared site is a live unobserved
/// path, which is the same under-reporting failure as a direct-call bypass.
pub fn admit_engine_owned_slot_site_roster(
    roster: EngineOwnedSlotSiteRoster,
) -> Result<AdmittedEngineOwnedSlotRoster, EngineOwnedSlotRosterAdmissionError> {
    if roster.target_logical_name.is_empty() {
        return Err(EngineOwnedSlotRosterAdmissionError::EmptyRosterName);
    }
    if roster.target_function_rva == 0 {
        return Err(EngineOwnedSlotRosterAdmissionError::RosterTargetRvaIsZero);
    }
    if roster.sites.is_empty() {
        return Err(EngineOwnedSlotRosterAdmissionError::RosterHasNoSites);
    }
    let reference = roster.sites[0];
    let mut admitted = Vec::with_capacity(roster.sites.len());
    let mut covered: Vec<u64> = Vec::with_capacity(roster.sites.len());
    for site in roster.sites {
        let site = *site;
        if site.expected_original_target_rva != roster.target_function_rva {
            return Err(EngineOwnedSlotRosterAdmissionError::SiteTargetMismatch {
                slot_rva: site.slot_rva,
                site_target_rva: site.expected_original_target_rva,
                roster_target_rva: roster.target_function_rva,
            });
        }
        if site.enumeration != reference.enumeration {
            return Err(EngineOwnedSlotRosterAdmissionError::SiteEnumerationsDisagree {
                slot_rva: site.slot_rva,
            });
        }
        if site.module_sha256 != reference.module_sha256 {
            return Err(
                EngineOwnedSlotRosterAdmissionError::SiteModuleIdentitiesDisagree {
                    slot_rva: site.slot_rva,
                },
            );
        }
        if covered.contains(&site.slot_rva) {
            return Err(EngineOwnedSlotRosterAdmissionError::DuplicateSlot {
                slot_rva: site.slot_rva,
            });
        }
        let site = admit_engine_owned_function_pointer_slot_site(site).map_err(|error| {
            EngineOwnedSlotRosterAdmissionError::Site {
                slot_rva: site.slot_rva,
                error,
            }
        })?;
        covered.push(site.expected.slot_rva);
        admitted.push(site);
    }
    for enumerated in reference.enumeration.slot_rvas {
        if !covered.contains(enumerated) {
            return Err(EngineOwnedSlotRosterAdmissionError::SlotCoverageIncomplete {
                uncovered_slot_rva: *enumerated,
            });
        }
    }
    let roster_sha256 = roster_sha256(roster, &admitted);
    let roster_slot_count = admitted.len();
    Ok(AdmittedEngineOwnedSlotRoster {
        target_logical_name: roster.target_logical_name,
        target_function_rva: roster.target_function_rva,
        roster_sha256,
        sites: admitted
            .into_iter()
            .map(|site| RosterAdmittedEngineOwnedSlotSite {
                site,
                roster_sha256,
                roster_slot_count,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE: [u8; 32] = [0xA7; 32];
    const ARTIFACT: [u8; 32] = [0x5E; 32];
    const TARGET_RVA: u64 = 0x0C6_E270;
    const VTABLE_RVA: u64 = 0x294_3AD0;
    const SLOT_RVA: u64 = 0x294_3C40;

    static COMPLETE: SlotEnumerationEvidence = SlotEnumerationEvidence {
        evidence_job_id: "test-job",
        evidence_artifact_sha256: ARTIFACT,
        target_function_rva: TARGET_RVA,
        slot_rvas: &[SLOT_RVA],
        direct_call_bypasses: &[],
        unwind_metadata_rvas: &[0x363_7D30],
        completeness: SlotEnumerationCompleteness::Complete,
    };

    fn site() -> ExpectedEngineOwnedFunctionPointerSlot {
        ExpectedEngineOwnedFunctionPointerSlot {
            logical_name: "test.site",
            module_sha256: MODULE,
            slot_rva: SLOT_RVA,
            owner: OwningStructureIdentity {
                kind: OwningStructureKind::Vtable,
                base_rva: VTABLE_RVA,
                entry_count: 59,
                entry_index: 46,
            },
            expected_original_target_rva: TARGET_RVA,
            enumeration: &COMPLETE,
        }
    }

    // FALSIFIABILITY: delete the `derived_slot_rva != Some(expected.slot_rva)`
    // refusal and this test goes green with an owner that does not derive its
    // own slot, i.e. a receipt naming a structure it did not redirect.
    #[test]
    fn owner_identity_must_derive_the_slot_address() {
        assert!(admit_engine_owned_function_pointer_slot_site(site()).is_ok());
        let mut wrong = site();
        wrong.owner.entry_index = 45;
        assert_eq!(
            admit_engine_owned_function_pointer_slot_site(wrong),
            Err(
                EngineOwnedSlotAdmissionError::SlotRvaIsNotTheOwningStructureEntry {
                    slot_rva: SLOT_RVA,
                    derived_slot_rva: Some(SLOT_RVA - 8),
                }
            )
        );
    }

    // FALSIFIABILITY: delete the `completeness != Complete` refusal and an
    // enumeration that never converged is admitted as a census.
    #[test]
    fn incomplete_enumeration_is_refused() {
        static INCOMPLETE: SlotEnumerationEvidence = SlotEnumerationEvidence {
            completeness: SlotEnumerationCompleteness::Incomplete,
            ..COMPLETE
        };
        let mut expected = site();
        expected.enumeration = &INCOMPLETE;
        assert_eq!(
            admit_engine_owned_function_pointer_slot_site(expected),
            Err(EngineOwnedSlotAdmissionError::EnumerationIncomplete)
        );
    }

    // FALSIFIABILITY: delete the `direct_call_bypasses.first()` refusal and a
    // target that is also called directly is admitted, so the installed hook
    // silently observes only part of its own traffic.
    #[test]
    fn a_direct_call_bypass_refuses_the_site() {
        static BYPASSED: SlotEnumerationEvidence = SlotEnumerationEvidence {
            direct_call_bypasses: &[DirectCallBypass {
                caller_function_rva: 0x129_9610,
                call_site_rva: 0x129_96C4,
            }],
            ..COMPLETE
        };
        let mut expected = site();
        expected.enumeration = &BYPASSED;
        assert_eq!(
            admit_engine_owned_function_pointer_slot_site(expected),
            Err(EngineOwnedSlotAdmissionError::UncoveredDirectCallBypass {
                bypass_count: 1,
                first_call_site_rva: 0x129_96C4,
            })
        );
    }

    // FALSIFIABILITY: delete the `slot_rvas.contains(&expected.slot_rva)`
    // refusal and a site may name a slot its own census never found.
    #[test]
    fn a_slot_outside_its_own_enumeration_is_refused() {
        let mut expected = site();
        expected.slot_rva = SLOT_RVA + 8;
        expected.owner.entry_index = 47;
        assert_eq!(
            admit_engine_owned_function_pointer_slot_site(expected),
            Err(
                EngineOwnedSlotAdmissionError::EnumerationSlotRosterExcludesThisSlot {
                    slot_rva: SLOT_RVA + 8
                }
            )
        );
    }

    // FALSIFIABILITY: delete the strict-ascending check and a census listing
    // one slot twice reports a slot count of two.
    #[test]
    fn a_repeated_enumeration_entry_is_refused() {
        static REPEATED: SlotEnumerationEvidence = SlotEnumerationEvidence {
            slot_rvas: &[SLOT_RVA, SLOT_RVA],
            ..COMPLETE
        };
        let mut expected = site();
        expected.enumeration = &REPEATED;
        assert_eq!(
            admit_engine_owned_function_pointer_slot_site(expected),
            Err(
                EngineOwnedSlotAdmissionError::EnumerationSlotRosterNotStrictlyAscending {
                    index: 1
                }
            )
        );
    }

    // FALSIFIABILITY: delete the final `covered.contains(enumerated)` loop and
    // a roster covering one of two enumerated slots is admitted, leaving the
    // second slot's dispatch live and unobserved.
    #[test]
    fn a_roster_missing_an_enumerated_slot_is_refused() {
        const SECOND_SLOT: u64 = 0x29A_5920;
        const LIFECYCLE_RVA: u64 = 0x129_9850;
        static TWO_SLOTS: SlotEnumerationEvidence = SlotEnumerationEvidence {
            evidence_job_id: "test-job",
            evidence_artifact_sha256: ARTIFACT,
            target_function_rva: LIFECYCLE_RVA,
            slot_rvas: &[0x299_14C8, SECOND_SLOT],
            direct_call_bypasses: &[],
            unwind_metadata_rvas: &[],
            completeness: SlotEnumerationCompleteness::Complete,
        };
        static FIRST: ExpectedEngineOwnedFunctionPointerSlot =
            ExpectedEngineOwnedFunctionPointerSlot {
                logical_name: "test.first",
                module_sha256: MODULE,
                slot_rva: 0x299_14C8,
                owner: OwningStructureIdentity {
                    kind: OwningStructureKind::Vtable,
                    base_rva: 0x299_13C0,
                    entry_count: 34,
                    entry_index: 33,
                },
                expected_original_target_rva: LIFECYCLE_RVA,
                enumeration: &TWO_SLOTS,
            };
        static SECOND: ExpectedEngineOwnedFunctionPointerSlot =
            ExpectedEngineOwnedFunctionPointerSlot {
                logical_name: "test.second",
                module_sha256: MODULE,
                slot_rva: SECOND_SLOT,
                owner: OwningStructureIdentity {
                    kind: OwningStructureKind::Vtable,
                    base_rva: 0x29A_5818,
                    entry_count: 34,
                    entry_index: 33,
                },
                expected_original_target_rva: LIFECYCLE_RVA,
                enumeration: &TWO_SLOTS,
            };
        static PARTIAL: [ExpectedEngineOwnedFunctionPointerSlot; 1] = [FIRST];
        static COMPLETE_PAIR: [ExpectedEngineOwnedFunctionPointerSlot; 2] = [FIRST, SECOND];

        assert_eq!(
            admit_engine_owned_slot_site_roster(EngineOwnedSlotSiteRoster {
                target_logical_name: "test.target",
                target_function_rva: LIFECYCLE_RVA,
                sites: &PARTIAL,
            }),
            Err(EngineOwnedSlotRosterAdmissionError::SlotCoverageIncomplete {
                uncovered_slot_rva: SECOND_SLOT
            })
        );
        let admitted = admit_engine_owned_slot_site_roster(EngineOwnedSlotSiteRoster {
            target_logical_name: "test.target",
            target_function_rva: LIFECYCLE_RVA,
            sites: &COMPLETE_PAIR,
        })
        .expect("the complete pair covers its enumeration exactly");
        assert_eq!(admitted.sites().len(), 2);
        assert!(admitted
            .sites()
            .iter()
            .all(|site| site.roster_sha256() == admitted.roster_sha256()
                && site.roster_slot_count() == 2));
    }

    // FALSIFIABILITY: make `slot_enumeration_sha256` ignore
    // `direct_call_bypasses` and these two censuses digest identically, so a
    // receipt could no longer distinguish a bypassed target from a clean one.
    #[test]
    fn the_enumeration_digest_covers_the_bypass_list() {
        static BYPASSED: SlotEnumerationEvidence = SlotEnumerationEvidence {
            direct_call_bypasses: &[DirectCallBypass {
                caller_function_rva: 1,
                call_site_rva: 2,
            }],
            ..COMPLETE
        };
        assert_ne!(
            slot_enumeration_sha256(&COMPLETE),
            slot_enumeration_sha256(&BYPASSED)
        );
    }

    // FALSIFIABILITY: drop `module_sha256` from
    // `engine_owned_slot_spec_sha256` and one slot spec digests the same
    // against two different builds.
    #[test]
    fn the_spec_digest_covers_the_module_identity() {
        let mut other = site();
        other.module_sha256 = [0x5A; 32];
        assert_ne!(
            engine_owned_slot_spec_sha256(site()),
            engine_owned_slot_spec_sha256(other)
        );
    }
}
