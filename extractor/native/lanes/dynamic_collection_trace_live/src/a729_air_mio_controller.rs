use crate::{
    validate_frame, CollectionTrace, ExactControllerTraceSource, MaterializedSlotTrace, RectI32,
    SourceItemTrace, TraceFrame,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const A729_BINARY_ID: &str = "hoi4.exe 1.19.2.0.a729";
pub const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const FINAL_FAMILY_JOB_ID: &str =
    "20260722T071816Z-a28-mio-history-and-bonus-final-exhausti-08b8a7f4";
pub const FINAL_FAMILY_MANIFEST_SHA256: &str =
    "a182725f84e611185fb3c4284130d1db27bae22763dd4b9f11470df8c6ef76ab";
pub const PAIR_TO_OBJECT_RESOLVER_RVA: u32 = 0x0220_A3D0;
pub const REFERENCE_TABLE_LOOKUP_RVA: u32 = 0x0220_83E0;
pub const PAIR_TO_OBJECT_RESOLVER_JOB_ID: &str =
    "20260722T014524Z-a28-reference-table-lookup-final-ef179ecf";
pub const PAIR_TO_OBJECT_RESOLVER_MANIFEST_SHA256: &str =
    "614e00757bf393f235f297fbf9519f33d038521c9ca9a97b1ef8553b4a86086c";
pub const AIR_FOUR_BUCKET_JOB_ID: &str =
    "20260722T013755Z-a28-air-upstream-four-bucket-authority-285d9be0";
pub const AIR_FOUR_BUCKET_MANIFEST_SHA256: &str =
    "839b5f4f9d184dfe4edcf0941797f56fd09e10753f5c8457ae748d21ad2a376e";
pub const AIR_PASSIVE_HOOK_JOB_ID: &str =
    "20260722T070946Z-a28-air-passive-controller-final-hook-c1cb9bb8";
pub const AIR_PASSIVE_HOOK_MANIFEST_SHA256: &str =
    "c3fc4dedb2f30acd3d862ae364b2b9ae844aa5d4941d4d24b896a063c7845c6d";

pub const AIR_COMMON_MASK_ZERO_RVA: u32 = 0x0195_261B;
pub const AIR_COMMON_EXCLUDED_MASK_RVA: u32 = 0x0195_2624;
pub const AIR_COMMON_BIT_33_RVA: u32 = 0x0195_2631;
pub const AIR_COMMON_ARCHETYPE_GATE_RVA: u32 = 0x00E5_05E0;
pub const AIR_BUCKET_1_BYTE_5B4_RVA: u32 = 0x0195_2837;
pub const AIR_BUCKET_2_BIT_20_RVA: u32 = 0x0195_2704;
pub const AIR_BUCKET_3_TYPE_3_VALUE_RVA: u32 = 0x0100_5DD0;
pub const AIR_SELECTION_PASSIVE_HOOK_BOUND: bool = true;
pub const MIO_HISTORY_PASSIVE_HOOK_BOUND: bool = true;
pub const MIO_BONUS_PASSIVE_HOOK_BOUND: bool = true;
pub const AIR_MIO_RUNTIME_AUTHORITY_BOUND: bool = false;

pub const AIR_COLLECTION_ID: &str = "interface/airselectionview.gui#collection-0001";
pub const MIO_BONUS_COLLECTION_ID: &str =
    "interface/military_industrial_organization/industrial_organization_detail.gui#collection-0001";
pub const MIO_VARIANT_COLLECTION_ID: &str =
    "interface/military_industrial_organization/industrial_organization_detail.gui#collection-0003";
pub const MIO_PRODUCTION_COLLECTION_ID: &str =
    "interface/military_industrial_organization/industrial_organization_detail.gui#collection-0004";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AirMioControllerKind {
    AirSelection,
    MioAggregatedBonus,
    MioVariantHistory,
    MioProductionHistory,
}

impl AirMioControllerKind {
    const fn wire_prefix(self) -> &'static str {
        match self {
            Self::AirSelection => "air-selection",
            Self::MioAggregatedBonus => "mio-bonus",
            Self::MioVariantHistory => "mio-variant-history",
            Self::MioProductionHistory => "mio-production-history",
        }
    }

    pub const fn collection_id(self) -> &'static str {
        match self {
            Self::AirSelection => AIR_COLLECTION_ID,
            Self::MioAggregatedBonus => MIO_BONUS_COLLECTION_ID,
            Self::MioVariantHistory => MIO_VARIANT_COLLECTION_ID,
            Self::MioProductionHistory => MIO_PRODUCTION_COLLECTION_ID,
        }
    }

    pub const fn controller_template(self) -> &'static str {
        match self {
            Self::AirSelection => "CPlaneTypeSelectionEntry",
            Self::MioAggregatedBonus => "industrial_organisation_detail_mixed_rows",
            Self::MioVariantHistory | Self::MioProductionHistory => {
                "industrial_organisation_history_item"
            }
        }
    }

    pub const fn producer_rvas(self) -> &'static [u32] {
        match self {
            Self::AirSelection => &[
                0x01F4_A980,
                0x01CD_B770,
                0x0195_1BA0,
                0x0195_1A10,
                0x0195_2420,
                0x00E5_05E0,
                0x0100_5DD0,
                0x01FC_49D0,
            ],
            Self::MioAggregatedBonus => &[0x01F0_5960, 0x01F0_6630, 0x01F0_5CD0, 0x01FA_4B90],
            Self::MioVariantHistory => &[0x00DA_1420, 0x01F0_6360, 0x01FA_5390, 0x01FA_6630],
            Self::MioProductionHistory => &[0x00DA_1680, 0x01F0_6360, 0x01FA_5390, 0x01FA_6630],
        }
    }

    fn rejection_predicate_rvas(self) -> &'static [u32] {
        match self {
            Self::AirSelection => &[
                AIR_COMMON_MASK_ZERO_RVA,
                AIR_COMMON_EXCLUDED_MASK_RVA,
                AIR_COMMON_BIT_33_RVA,
                AIR_COMMON_ARCHETYPE_GATE_RVA,
                AIR_BUCKET_1_BYTE_5B4_RVA,
                AIR_BUCKET_2_BIT_20_RVA,
                AIR_BUCKET_3_TYPE_3_VALUE_RVA,
            ],
            Self::MioAggregatedBonus => &[0x01F0_5960, 0x01F0_6630, 0x01F0_5CD0],
            Self::MioVariantHistory => &[0x00DA_1420],
            Self::MioProductionHistory => &[0x00DA_1680],
        }
    }

    const fn function_job_id(self) -> &'static str {
        match self {
            Self::AirSelection => AIR_PASSIVE_HOOK_JOB_ID,
            _ => FINAL_FAMILY_JOB_ID,
        }
    }

    const fn function_manifest_sha256(self) -> &'static str {
        match self {
            Self::AirSelection => AIR_PASSIVE_HOOK_MANIFEST_SHA256,
            _ => FINAL_FAMILY_MANIFEST_SHA256,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ControllerSourceLane {
    AirBucket0,
    AirBucket1,
    AirBucket2,
    AirBucket3,
    MioFixedBonus,
    MioEquipmentGroup,
    MioVariantTable,
    MioProductionLine,
    MioProductionTable,
}

impl ControllerSourceLane {
    const fn wire(self) -> &'static str {
        match self {
            Self::AirBucket0 => "air-bucket-0",
            Self::AirBucket1 => "air-bucket-1",
            Self::AirBucket2 => "air-bucket-2",
            Self::AirBucket3 => "air-bucket-3",
            Self::MioFixedBonus => "mio-fixed-bonus",
            Self::MioEquipmentGroup => "mio-equipment-group",
            Self::MioVariantTable => "mio-variant-table",
            Self::MioProductionLine => "mio-production-line",
            Self::MioProductionTable => "mio-production-table",
        }
    }

    const fn belongs_to(self, kind: AirMioControllerKind) -> bool {
        match kind {
            AirMioControllerKind::AirSelection => matches!(
                self,
                Self::AirBucket0 | Self::AirBucket1 | Self::AirBucket2 | Self::AirBucket3
            ),
            AirMioControllerKind::MioAggregatedBonus => {
                matches!(self, Self::MioFixedBonus | Self::MioEquipmentGroup)
            }
            AirMioControllerKind::MioVariantHistory => matches!(self, Self::MioVariantTable),
            AirMioControllerKind::MioProductionHistory => {
                matches!(self, Self::MioProductionLine | Self::MioProductionTable)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PortableItemIdentity {
    ReferencedObject {
        object_type: u32,
        stable_id: u32,
    },
    ControllerAtom {
        producer_rva: u32,
        semantic_key: u32,
        subject_type: u32,
        subject_id: u32,
    },
}

impl PortableItemIdentity {
    fn wire_id(self, kind: AirMioControllerKind) -> Result<String, String> {
        let prefix = kind.wire_prefix();
        match self {
            Self::ReferencedObject {
                object_type,
                stable_id,
            } => {
                if object_type == 0 || stable_id == 0 {
                    return Err("A28 referenced-object identity contains zero".into());
                }
                Ok(format!("{prefix}:object:{object_type}:{stable_id}"))
            }
            Self::ControllerAtom {
                producer_rva,
                semantic_key,
                subject_type,
                subject_id,
            } => {
                if kind != AirMioControllerKind::MioAggregatedBonus
                    || !kind.producer_rvas().contains(&producer_rva)
                    || semantic_key == 0
                    || ((subject_type == 0) != (subject_id == 0))
                {
                    return Err("A28 controller-atom identity is not authority-bound".into());
                }
                Ok(format!(
                    "{prefix}:atom:{producer_rva:08x}:{semantic_key}:{subject_type}:{subject_id}"
                ))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ControllerCandidateKey {
    pub source_lane: ControllerSourceLane,
    pub source_instance_key: u64,
    pub identity: PortableItemIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResolverInputSource {
    AirOptionEquipmentSelfReference,
    MioBonusOrganisationSubject,
    MioBonusObjectReference,
    MioVariantHistoryEntryReference,
    MioProductionHistoryEntryReference,
}

impl ResolverInputSource {
    const fn wire(self) -> &'static str {
        match self {
            Self::AirOptionEquipmentSelfReference => "air-option-equipment-self-reference",
            Self::MioBonusOrganisationSubject => "mio-bonus-organisation-subject",
            Self::MioBonusObjectReference => "mio-bonus-object-reference",
            Self::MioVariantHistoryEntryReference => "mio-variant-history-entry-reference",
            Self::MioProductionHistoryEntryReference => "mio-production-history-entry-reference",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortableObjectResolverRecord {
    pub source: ResolverInputSource,
    pub resolver_rva: u32,
    pub lookup_rva: u32,
    pub resolver_job_id: String,
    pub resolver_manifest_sha256: String,
    pub raw_object_type: u32,
    pub raw_stable_id: u32,
    pub resolved_object_type: u32,
    pub resolved_stable_id: u32,
    pub resolved_non_null: bool,
    pub round_trip_self_reference_matches: bool,
}

impl ControllerCandidateKey {
    fn wire_id(self, kind: AirMioControllerKind) -> Result<String, String> {
        if !self.source_lane.belongs_to(kind) || self.source_instance_key == 0 {
            return Err("A28 candidate source lane/key is not authority-bound".into());
        }
        let identity = self.identity.wire_id(kind)?;
        let prefix = kind.wire_prefix();
        let suffix = identity
            .strip_prefix(prefix)
            .ok_or_else(|| "A28 candidate identity prefix differs".to_string())?;
        Ok(format!(
            "{prefix}:candidate:{}:{}{suffix}",
            self.source_lane.wire(),
            self.source_instance_key,
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectionEvidence {
    pub reason_code: String,
    pub predicate_rva: u32,
    pub predicate_record_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerCandidate {
    pub key: ControllerCandidateKey,
    pub identity_resolver: PortableObjectResolverRecord,
    pub identity_record_sha256: String,
    pub source_ordinal: u32,
    pub rejection: Option<RejectionEvidence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeRowTemplate {
    AirPlaneTypeSelectionEntry,
    MioDetailModifierItem,
    MioDetailBonusItem,
    MioEquipmentTypeWindow,
    MioDetailSeparator,
    MioHistoryItem,
}

impl RuntimeRowTemplate {
    fn wire(self) -> &'static str {
        match self {
            Self::AirPlaneTypeSelectionEntry => "CPlaneTypeSelectionEntry",
            Self::MioDetailModifierItem => "industrial_organisation_detail_modifier_item",
            Self::MioDetailBonusItem => "industrial_organisation_detail_bonus_item",
            Self::MioEquipmentTypeWindow => "industrial_organisation_equipment_type_window",
            Self::MioDetailSeparator => "industrial_organisation_detail_separator",
            Self::MioHistoryItem => "industrial_organisation_history_item",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerSlot {
    pub stable_slot_id: u32,
    pub bound_candidate: ControllerCandidateKey,
    pub template: RuntimeRowTemplate,
    pub rect: RectI32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedListState {
    pub clip_rect: RectI32,
    pub content_extent_pixels: (i32, i32),
    pub viewport_extent_pixels: (i32, i32),
    pub scroll_min_fixed: i64,
    pub scroll_max_fixed: i64,
    pub scroll_current_fixed: i64,
    pub scroll_target_fixed: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirMioAuthorityReceipt {
    pub binary_id: String,
    pub executable_sha256: String,
    pub function_job_id: String,
    pub function_manifest_sha256: String,
    pub producer_rvas: Vec<u32>,
    pub transition_sequence: u64,
    pub observation_qpc_ticks: u64,
    pub captured_game_hour_raw: i32,
    pub owner_instance_key: u64,
    pub producer_record_sha256: String,
    pub post_return: bool,
    pub candidate_universe_complete: bool,
    pub rejection_predicates_complete: bool,
    pub stable_identity_complete: bool,
    pub list_state_complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirMioControllerObservation {
    pub kind: AirMioControllerKind,
    pub authority: AirMioAuthorityReceipt,
    pub candidates: Vec<ControllerCandidate>,
    pub accepted_order: Vec<ControllerCandidateKey>,
    pub slots: Vec<ControllerSlot>,
    pub list_state: ResolvedListState,
}

#[derive(Clone, Debug)]
pub struct A729AirMioControllerSource {
    frame_hour: u64,
    collections: Vec<CollectionTrace>,
}

impl A729AirMioControllerSource {
    pub fn try_admit(
        frame_hour: u64,
        observations: Vec<AirMioControllerObservation>,
    ) -> Result<Self, String> {
        if !AIR_MIO_RUNTIME_AUTHORITY_BOUND {
            return Err("A28 air/MIO runtime receipt authority is not bound".into());
        }
        let collections = validate_fixture_observations(frame_hour, observations)?;
        Ok(Self {
            frame_hour,
            collections,
        })
    }

    pub(crate) fn admit_hook_owned_air(
        frame_hour: u64,
        observations: Vec<AirMioControllerObservation>,
    ) -> Result<Self, String> {
        if !AIR_SELECTION_PASSIVE_HOOK_BOUND
            || observations.is_empty()
            || observations
                .iter()
                .any(|observation| observation.kind != AirMioControllerKind::AirSelection)
        {
            return Err("A28 hook-owned air observation family is not bound".into());
        }
        let collections = validate_fixture_observations(frame_hour, observations)?;
        Ok(Self {
            frame_hour,
            collections,
        })
    }

    pub(crate) fn admit_hook_owned_mio_history(
        frame_hour: u64,
        observations: Vec<AirMioControllerObservation>,
    ) -> Result<Self, String> {
        let kinds = observations
            .iter()
            .map(|observation| observation.kind)
            .collect::<BTreeSet<_>>();
        if !MIO_HISTORY_PASSIVE_HOOK_BOUND
            || observations.len() != 2
            || kinds
                != BTreeSet::from([
                    AirMioControllerKind::MioVariantHistory,
                    AirMioControllerKind::MioProductionHistory,
                ])
        {
            return Err("A28 hook-owned MIO history pair is not bound".into());
        }
        let first = &observations[0].authority;
        if first.owner_instance_key == 0
            || observations.iter().any(|observation| {
                observation.authority.transition_sequence != first.transition_sequence
                    || observation.authority.observation_qpc_ticks != first.observation_qpc_ticks
                    || observation.authority.captured_game_hour_raw != first.captured_game_hour_raw
                    || observation.authority.owner_instance_key != first.owner_instance_key
            })
        {
            return Err(
                "A28 hook-owned MIO histories do not share one transition and organization".into(),
            );
        }
        let collections = validate_fixture_observations(frame_hour, observations)?;
        Ok(Self {
            frame_hour,
            collections,
        })
    }

    pub(crate) fn admit_hook_owned_mio_bonus(
        frame_hour: u64,
        observations: Vec<AirMioControllerObservation>,
    ) -> Result<Self, String> {
        if !MIO_BONUS_PASSIVE_HOOK_BOUND
            || observations.len() != 1
            || observations[0].kind != AirMioControllerKind::MioAggregatedBonus
            || observations[0].authority.owner_instance_key == 0
        {
            return Err("A28 hook-owned MIO aggregate-bonus observation is not bound".into());
        }
        let collections = validate_fixture_observations(frame_hour, observations)?;
        Ok(Self {
            frame_hour,
            collections,
        })
    }

    #[cfg(windows)]
    pub fn admit_hook_owned_inactive(
        frame_hour: u64,
        proof: &crate::a729_headless_inactive::A729InactiveControllerProof,
    ) -> Result<Self, String> {
        if !proof.matches_hour(frame_hour) {
            return Err("A28 inactive-controller proof hour differs".into());
        }
        Ok(Self {
            frame_hour,
            collections: Vec::new(),
        })
    }

    pub fn combine_hook_owned(
        frame_hour: u64,
        sources: impl IntoIterator<Item = Self>,
    ) -> Result<Self, String> {
        let mut collections = Vec::new();
        let mut collection_ids = BTreeSet::new();
        for source in sources {
            if source.frame_hour != frame_hour {
                return Err(format!(
                    "A28 hook-owned source hour {} differs from combined hour {frame_hour}",
                    source.frame_hour
                ));
            }
            for collection in source.collections {
                if !collection_ids.insert(collection.collection_instance_id.clone()) {
                    return Err(format!(
                        "A28 hook-owned source repeats collection {}",
                        collection.collection_instance_id
                    ));
                }
                collections.push(collection);
            }
        }
        if collections.is_empty() {
            return Err("A28 hook-owned combined source is empty".into());
        }
        let frame = TraceFrame {
            elapsed_wall_ms: 0,
            status_flags: 0,
            frame_hour,
            collections: collections.clone(),
        };
        validate_frame(&frame).map_err(|error| {
            format!("A28 combined hook-owned DCT1 validation failed: {error:?}")
        })?;
        Ok(Self {
            frame_hour,
            collections,
        })
    }
}

pub fn validate_fixture_observations(
    frame_hour: u64,
    observations: Vec<AirMioControllerObservation>,
) -> Result<Vec<CollectionTrace>, String> {
    if observations.is_empty() {
        return Err("A28 air/MIO source has no controller observations".into());
    }
    let mut kinds = BTreeSet::new();
    let mut collections = Vec::with_capacity(observations.len());
    for observation in observations {
        if !kinds.insert(observation.kind) {
            return Err("A28 air/MIO source repeats a controller family in one frame".into());
        }
        collections.push(observation.into_collection(frame_hour)?);
    }
    let frame = TraceFrame {
        elapsed_wall_ms: 0,
        status_flags: 0,
        frame_hour,
        collections: collections.clone(),
    };
    validate_frame(&frame)
        .map_err(|error| format!("A28 air/MIO DCT1 validation failed: {error:?}"))?;
    Ok(collections)
}

impl crate::source_seal::Sealed for A729AirMioControllerSource {}

impl ExactControllerTraceSource for A729AirMioControllerSource {
    fn is_exact_a729_controller_source(&self) -> bool {
        true
    }

    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<CollectionTrace>, String> {
        if frame_hour != self.frame_hour {
            return Err(format!(
                "A28 air/MIO observation hour {} differs from requested hour {frame_hour}",
                self.frame_hour
            ));
        }
        Ok(self.collections.clone())
    }
}

impl AirMioControllerObservation {
    fn into_collection(self, frame_hour: u64) -> Result<CollectionTrace, String> {
        validate_authority(self.kind, &self.authority)?;
        validate_state(&self.list_state)?;
        let mut candidate_keys = BTreeSet::new();
        let mut source_ids = BTreeSet::new();
        let mut included_order = Vec::new();
        let mut candidate_to_wire = BTreeMap::new();
        for (ordinal, candidate) in self.candidates.iter().enumerate() {
            if candidate.source_ordinal as usize != ordinal {
                return Err("A28 controller source ordinals are not contiguous".into());
            }
            if !candidate_keys.insert(candidate.key) {
                return Err("A28 controller source repeats a candidate key".into());
            }
            let base_id = candidate.key.wire_id(self.kind)?;
            validate_identity_resolver(self.kind, candidate)?;
            let id = format!("{base_id}:resolver:{}", candidate.identity_record_sha256);
            if !source_ids.insert(id.clone()) {
                return Err("A28 controller source contains duplicate stable candidate key".into());
            }
            if let Some(rejection) = &candidate.rejection {
                validate_rejection(self.kind, candidate.key.source_lane, rejection)?;
            } else {
                included_order.push(id.clone());
            }
            candidate_to_wire.insert(candidate.key, id);
        }
        let accepted_order = self
            .accepted_order
            .iter()
            .map(|candidate| {
                candidate_to_wire.get(candidate).cloned().ok_or_else(|| {
                    "A28 accepted candidate is absent from source ledger".to_string()
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let exact_order_required = self.kind == AirMioControllerKind::MioProductionHistory;
        if accepted_order.len() != included_order.len()
            || accepted_order.iter().collect::<BTreeSet<_>>()
                != included_order.iter().collect::<BTreeSet<_>>()
            || (exact_order_required && accepted_order != included_order)
        {
            return Err("A28 accepted order differs from included source set".into());
        }
        if self.slots.len() != accepted_order.len() {
            return Err("A28 accepted row count differs from materialized slot count".into());
        }
        if self.slots.len() > crate::MAX_ITEMS_PER_COLLECTION {
            return Err("A28 materialized slot count exceeds bound".into());
        }
        let mut slot_ids = BTreeSet::new();
        let mut materialized_slots = Vec::with_capacity(self.slots.len());
        for (slot, expected_id) in self.slots.iter().zip(&accepted_order) {
            if slot.stable_slot_id == 0 || !slot_ids.insert(slot.stable_slot_id) {
                return Err("A28 controller slot identity is zero or duplicated".into());
            }
            validate_template(self.kind, slot.template)?;
            let bound_id = candidate_to_wire
                .get(&slot.bound_candidate)
                .cloned()
                .ok_or_else(|| "A28 materialized slot candidate is absent".to_string())?;
            if bound_id != *expected_id {
                return Err("A28 materialized slot traversal differs from accepted order".into());
            }
            if slot.rect.width <= 0 || slot.rect.height <= 0 {
                return Err("A28 materialized row rectangle is nonpositive".into());
            }
            materialized_slots.push(MaterializedSlotTrace {
                slot_id: format!("{}/slot:{}", self.kind.collection_id(), slot.stable_slot_id),
                bound_item_id: bound_id,
                row_template_id: slot.template.wire().into(),
                rect: slot.rect,
            });
        }
        let source_items = self
            .candidates
            .into_iter()
            .map(|candidate| {
                let stable_item_id = candidate_to_wire[&candidate.key].clone();
                SourceItemTrace {
                    stable_item_id,
                    source_ordinal: candidate.source_ordinal,
                    included: candidate.rejection.is_none(),
                    filter_reason: candidate.rejection.map(|evidence| evidence.reason_code),
                }
            })
            .collect();
        Ok(CollectionTrace {
            collection_instance_id: self.kind.collection_id().into(),
            frame_hour,
            source_items,
            ordered_item_ids: accepted_order,
            group_boundaries: Vec::new(),
            row_template_id: self.kind.controller_template().into(),
            materialized_slots,
            clip_rect: self.list_state.clip_rect,
            content_extent_pixels: self.list_state.content_extent_pixels,
            viewport_extent_pixels: self.list_state.viewport_extent_pixels,
            scroll_min_fixed: self.list_state.scroll_min_fixed,
            scroll_max_fixed: self.list_state.scroll_max_fixed,
            scroll_current_fixed: self.list_state.scroll_current_fixed,
            scroll_target_fixed: self.list_state.scroll_target_fixed,
        })
    }
}

pub fn compute_identity_record_sha256(
    kind: AirMioControllerKind,
    record: &PortableObjectResolverRecord,
) -> String {
    let mut canonical = Vec::with_capacity(256);
    canonical.extend_from_slice(b"hoi4-a28-pair-to-object-resolver-v1\0");
    canonical.extend_from_slice(kind.wire_prefix().as_bytes());
    canonical.push(0);
    canonical.extend_from_slice(record.source.wire().as_bytes());
    canonical.push(0);
    canonical.extend_from_slice(&record.resolver_rva.to_le_bytes());
    canonical.extend_from_slice(&record.lookup_rva.to_le_bytes());
    canonical.extend_from_slice(&record.raw_object_type.to_le_bytes());
    canonical.extend_from_slice(&record.raw_stable_id.to_le_bytes());
    canonical.extend_from_slice(&record.resolved_object_type.to_le_bytes());
    canonical.extend_from_slice(&record.resolved_stable_id.to_le_bytes());
    canonical.push(u8::from(record.resolved_non_null));
    canonical.push(u8::from(record.round_trip_self_reference_matches));
    format!("{:x}", Sha256::digest(canonical))
}

fn validate_identity_resolver(
    kind: AirMioControllerKind,
    candidate: &ControllerCandidate,
) -> Result<(), String> {
    let record = &candidate.identity_resolver;
    let expected_source = match (kind, candidate.key.identity) {
        (AirMioControllerKind::AirSelection, PortableItemIdentity::ReferencedObject { .. }) => {
            ResolverInputSource::AirOptionEquipmentSelfReference
        }
        (AirMioControllerKind::MioAggregatedBonus, PortableItemIdentity::ControllerAtom { .. }) => {
            ResolverInputSource::MioBonusOrganisationSubject
        }
        (
            AirMioControllerKind::MioAggregatedBonus,
            PortableItemIdentity::ReferencedObject { .. },
        ) => ResolverInputSource::MioBonusObjectReference,
        (
            AirMioControllerKind::MioVariantHistory,
            PortableItemIdentity::ReferencedObject { .. },
        ) => ResolverInputSource::MioVariantHistoryEntryReference,
        (
            AirMioControllerKind::MioProductionHistory,
            PortableItemIdentity::ReferencedObject { .. },
        ) => ResolverInputSource::MioProductionHistoryEntryReference,
        _ => return Err("A28 identity kind has no frozen resolver source".into()),
    };
    if record.source != expected_source
        || record.resolver_rva != PAIR_TO_OBJECT_RESOLVER_RVA
        || record.lookup_rva != REFERENCE_TABLE_LOOKUP_RVA
        || record.resolver_job_id != PAIR_TO_OBJECT_RESOLVER_JOB_ID
        || record.resolver_manifest_sha256 != PAIR_TO_OBJECT_RESOLVER_MANIFEST_SHA256
        || record.raw_object_type == 0
        || record.raw_stable_id == 0
        || record.raw_object_type != record.resolved_object_type
        || record.raw_stable_id != record.resolved_stable_id
        || !record.resolved_non_null
        || !record.round_trip_self_reference_matches
    {
        return Err("A28 pair-to-object resolver record rejected".into());
    }
    match candidate.key.identity {
        PortableItemIdentity::ReferencedObject {
            object_type,
            stable_id,
        } => {
            if (object_type, stable_id) != (record.raw_object_type, record.raw_stable_id) {
                return Err("A28 referenced-object identity differs from resolver input".into());
            }
        }
        PortableItemIdentity::ControllerAtom {
            subject_type,
            subject_id,
            ..
        } => {
            if (subject_type, subject_id) != (record.raw_object_type, record.raw_stable_id) {
                return Err("A28 controller atom subject differs from resolver input".into());
            }
        }
    }
    if !lower_hex_sha256(&candidate.identity_record_sha256)
        || candidate.identity_record_sha256 != compute_identity_record_sha256(kind, record)
    {
        return Err("A28 candidate resolver digest differs from canonical record".into());
    }
    Ok(())
}

fn validate_authority(
    kind: AirMioControllerKind,
    receipt: &AirMioAuthorityReceipt,
) -> Result<(), String> {
    if receipt.binary_id != A729_BINARY_ID
        || receipt.executable_sha256 != A729_EXECUTABLE_SHA256
        || receipt.function_job_id != kind.function_job_id()
        || receipt.function_manifest_sha256 != kind.function_manifest_sha256()
        || receipt.producer_rvas.as_slice() != kind.producer_rvas()
        || receipt.transition_sequence == 0
        || receipt.observation_qpc_ticks == 0
        || !receipt.post_return
        || !receipt.candidate_universe_complete
        || !receipt.rejection_predicates_complete
        || !receipt.stable_identity_complete
        || !receipt.list_state_complete
        || !lower_hex_sha256(&receipt.producer_record_sha256)
    {
        return Err("A28 air/MIO authority receipt rejected".into());
    }
    Ok(())
}

fn validate_rejection(
    kind: AirMioControllerKind,
    source_lane: ControllerSourceLane,
    evidence: &RejectionEvidence,
) -> Result<(), String> {
    if evidence.reason_code.trim() != evidence.reason_code
        || evidence.reason_code.is_empty()
        || evidence.reason_code.len() > crate::MAX_STRING_BYTES
        || !kind
            .rejection_predicate_rvas()
            .contains(&evidence.predicate_rva)
        || !lower_hex_sha256(&evidence.predicate_record_sha256)
    {
        return Err("A28 rejected candidate lacks authenticated predicate evidence".into());
    }
    if kind == AirMioControllerKind::AirSelection {
        let expected_reason = match evidence.predicate_rva {
            AIR_COMMON_MASK_ZERO_RVA => "common_air_mask_zero",
            AIR_COMMON_EXCLUDED_MASK_RVA => "common_excluded_mask_nonzero",
            AIR_COMMON_BIT_33_RVA => "common_bit_33_set",
            AIR_COMMON_ARCHETYPE_GATE_RVA => "common_archetype_gate_false",
            AIR_BUCKET_1_BYTE_5B4_RVA => "bucket_1_byte_5b4_zero",
            AIR_BUCKET_2_BIT_20_RVA => "bucket_2_bit_20_set",
            AIR_BUCKET_3_TYPE_3_VALUE_RVA => "bucket_3_type_3_value_nonpositive",
            _ => return Err("A28 air rejection predicate is not in the frozen law".into()),
        };
        let lane_allows_predicate = match source_lane {
            ControllerSourceLane::AirBucket0 => matches!(
                evidence.predicate_rva,
                AIR_COMMON_MASK_ZERO_RVA
                    | AIR_COMMON_EXCLUDED_MASK_RVA
                    | AIR_COMMON_BIT_33_RVA
                    | AIR_COMMON_ARCHETYPE_GATE_RVA
            ),
            ControllerSourceLane::AirBucket1 => {
                evidence.predicate_rva == AIR_BUCKET_1_BYTE_5B4_RVA
                    || matches!(
                        evidence.predicate_rva,
                        AIR_COMMON_MASK_ZERO_RVA
                            | AIR_COMMON_EXCLUDED_MASK_RVA
                            | AIR_COMMON_BIT_33_RVA
                            | AIR_COMMON_ARCHETYPE_GATE_RVA
                    )
            }
            ControllerSourceLane::AirBucket2 => {
                evidence.predicate_rva == AIR_BUCKET_2_BIT_20_RVA
                    || matches!(
                        evidence.predicate_rva,
                        AIR_COMMON_MASK_ZERO_RVA
                            | AIR_COMMON_EXCLUDED_MASK_RVA
                            | AIR_COMMON_BIT_33_RVA
                            | AIR_COMMON_ARCHETYPE_GATE_RVA
                    )
            }
            ControllerSourceLane::AirBucket3 => {
                evidence.predicate_rva == AIR_BUCKET_3_TYPE_3_VALUE_RVA
                    || matches!(
                        evidence.predicate_rva,
                        AIR_COMMON_MASK_ZERO_RVA
                            | AIR_COMMON_EXCLUDED_MASK_RVA
                            | AIR_COMMON_BIT_33_RVA
                            | AIR_COMMON_ARCHETYPE_GATE_RVA
                    )
            }
            _ => false,
        };
        if evidence.reason_code != expected_reason || !lane_allows_predicate {
            return Err("A28 air rejection reason/predicate does not belong to its bucket".into());
        }
    }
    Ok(())
}

fn validate_template(
    kind: AirMioControllerKind,
    template: RuntimeRowTemplate,
) -> Result<(), String> {
    let valid = match kind {
        AirMioControllerKind::AirSelection => {
            template == RuntimeRowTemplate::AirPlaneTypeSelectionEntry
        }
        AirMioControllerKind::MioAggregatedBonus => matches!(
            template,
            RuntimeRowTemplate::MioDetailModifierItem
                | RuntimeRowTemplate::MioDetailBonusItem
                | RuntimeRowTemplate::MioEquipmentTypeWindow
                | RuntimeRowTemplate::MioDetailSeparator
        ),
        AirMioControllerKind::MioVariantHistory | AirMioControllerKind::MioProductionHistory => {
            template == RuntimeRowTemplate::MioHistoryItem
        }
    };
    if valid {
        Ok(())
    } else {
        Err("A28 runtime row template differs from controller family".into())
    }
}

fn validate_state(state: &ResolvedListState) -> Result<(), String> {
    if state.clip_rect.width <= 0
        || state.clip_rect.height <= 0
        || state.content_extent_pixels.0 < 0
        || state.content_extent_pixels.1 < 0
        || state.viewport_extent_pixels.0 <= 0
        || state.viewport_extent_pixels.1 <= 0
        || state.scroll_max_fixed < state.scroll_min_fixed
        || !(state.scroll_min_fixed..=state.scroll_max_fixed).contains(&state.scroll_current_fixed)
        || !(state.scroll_min_fixed..=state.scroll_max_fixed).contains(&state.scroll_target_fixed)
    {
        return Err("A28 resolved list geometry/scroll state rejected".into());
    }
    Ok(())
}

fn lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
