use sha2::{Digest, Sha256};

pub const ENGINE_OWNED_FUNCTION_POINTER_SLOT_PRIMITIVE_ID: &str =
    "engine-owned-function-pointer-slot.aligned-usize-cas.process-lifetime.v1";

pub const ENGINE_OWNED_FUNCTION_POINTER_SLOT_PROCESS_LIFETIME_POLICY_ID: &str =
    "engine-owned-function-pointer-slot.external-dispatch.process-lifetime.v1";

pub const ENGINE_OWNED_SLOT_WIDTH_BYTES: u64 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum OwningStructureKind {
    Vtable,
    FunctionPointerTable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct OwningStructureIdentity {
    pub kind: OwningStructureKind,
    pub base_rva: u64,
    pub entry_count: u32,
    pub entry_index: u32,
}

impl OwningStructureIdentity {
    pub fn derived_slot_rva(&self) -> Option<u64> {
        u64::from(self.entry_index)
            .checked_mul(ENGINE_OWNED_SLOT_WIDTH_BYTES)
            .and_then(|offset| self.base_rva.checked_add(offset))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DirectCallBypass {
    pub caller_function_rva: u64,
    pub call_site_rva: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotEnumerationCompleteness {
    Complete,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlotEnumerationEvidence {
    pub evidence_job_id: &'static str,
    pub evidence_artifact_sha256: [u8; 32],
    pub target_function_rva: u64,
    pub slot_rvas: &'static [u64],
    pub direct_call_bypasses: &'static [DirectCallBypass],
    pub unwind_metadata_rvas: &'static [u64],
    pub completeness: SlotEnumerationCompleteness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpectedEngineOwnedFunctionPointerSlot {
    pub logical_name: &'static str,
    pub module_sha256: [u8; 32],
    pub slot_rva: u64,
    pub owner: OwningStructureIdentity,
    pub expected_original_target_rva: u64,
    pub enumeration: &'static SlotEnumerationEvidence,
}

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
    SlotRvaIsNotTheOwningStructureEntry {
        slot_rva: u64,
        derived_slot_rva: Option<u64>,
    },
    EnumerationIncomplete,
    EnumerationJobIdMissing,
    EnumerationArtifactDigestMissing,
    EnumerationTargetMismatch {
        enumeration_target_rva: u64,
        expected_original_target_rva: u64,
    },
    EnumerationSlotRosterEmpty,
    EnumerationSlotRosterNotStrictlyAscending {
        index: usize,
    },
    EnumerationSlotRosterEntryNotPointerAligned {
        slot_rva: u64,
    },
    EnumerationSlotRosterExcludesThisSlot {
        slot_rva: u64,
    },
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
    SiteTargetMismatch {
        slot_rva: u64,
        site_target_rva: u64,
        roster_target_rva: u64,
    },
    SiteEnumerationsDisagree {
        slot_rva: u64,
    },
    SiteModuleIdentitiesDisagree {
        slot_rva: u64,
    },
    DuplicateSlot {
        slot_rva: u64,
    },
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

    pub fn roster_sha256(&self) -> [u8; 32] {
        self.roster_sha256
    }

    pub fn roster_slot_count(&self) -> usize {
        self.roster_slot_count
    }
}

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

pub fn engine_owned_slot_spec_sha256(expected: ExpectedEngineOwnedFunctionPointerSlot) -> [u8; 32] {
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
            return Err(
                EngineOwnedSlotRosterAdmissionError::SiteEnumerationsDisagree {
                    slot_rva: site.slot_rva,
                },
            );
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
            return Err(
                EngineOwnedSlotRosterAdmissionError::SlotCoverageIncomplete {
                    uncovered_slot_rva: *enumerated,
                },
            );
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
            Err(
                EngineOwnedSlotRosterAdmissionError::SlotCoverageIncomplete {
                    uncovered_slot_rva: SECOND_SLOT
                }
            )
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
