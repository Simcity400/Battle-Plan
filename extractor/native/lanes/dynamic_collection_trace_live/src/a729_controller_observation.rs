use crate::{validate_frame, CollectionTrace, ExactControllerTraceSource, TraceFrame};

pub const A729_BINARY_ID: &str = "hoi4.exe 1.19.2.0.a729";
pub const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

pub const EQUIPMENT_GRID_JOB_ID: &str =
    "20260720T133905Z-w5-a28-equipment-grid-owner-geometry-7d8900a4";
pub const EQUIPMENT_GRID_MANIFEST_SHA256: &str =
    "96ad7c23413611dcc1524db1fac91a80c94324fc44f04b0e07430f927d8e76a4";

pub const EQUIPMENT_GRID_PRODUCER_RVA: u32 = 0x0177_D780;
pub const EQUIPMENT_GRID_CALLBACK_RVA: u32 = 0x0177_97A0;
pub const STANDARD_GRID_APPEND_RVA: u32 = 0x002D_E850;
pub const STANDARD_GRID_FINALIZE_RVA: u32 = 0x022B_2F10;
pub const CONTAINER_SCROLL_SYNC_RVA: u32 = 0x022A_FA30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CollectionFamily {
    StandardGrid,
    StandardList,
    SmoothList,
    Overlap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ControllerKind {
    EquipmentVariantGrid,
    TechnologySpecializationBadgeGrid,
    AirSelectionList,
    FacilitySpecializationSmoothList,
    MioAggregatedBonusSmoothList,
    MioVariantHistorySmoothList,
    MioProductionHistorySmoothList,
    NamedOverlap,
}

impl ControllerKind {
    pub const fn family(self) -> CollectionFamily {
        match self {
            Self::EquipmentVariantGrid | Self::TechnologySpecializationBadgeGrid => {
                CollectionFamily::StandardGrid
            }
            Self::AirSelectionList => CollectionFamily::StandardList,
            Self::FacilitySpecializationSmoothList
            | Self::MioAggregatedBonusSmoothList
            | Self::MioVariantHistorySmoothList
            | Self::MioProductionHistorySmoothList => CollectionFamily::SmoothList,
            Self::NamedOverlap => CollectionFamily::Overlap,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClosureDisposition {
    OwnerRunnable,
    MissingOuterGeometryAndScroll,
    MissingRejectedSourceAccounting,
    MissingRuntimeIdentityAndGeometry,
    MissingControllerSpecificJoin,
}

pub const fn closure_disposition(kind: ControllerKind) -> ClosureDisposition {
    match kind {
        ControllerKind::EquipmentVariantGrid
        | ControllerKind::TechnologySpecializationBadgeGrid
        | ControllerKind::FacilitySpecializationSmoothList => ClosureDisposition::OwnerRunnable,
        ControllerKind::AirSelectionList
        | ControllerKind::MioAggregatedBonusSmoothList
        | ControllerKind::MioVariantHistorySmoothList
        | ControllerKind::MioProductionHistorySmoothList => {
            ClosureDisposition::MissingRejectedSourceAccounting
        }
        ControllerKind::NamedOverlap => ClosureDisposition::MissingControllerSpecificJoin,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationPhase {
    FunctionEntry,
    PostReturn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessAuthorityReceipt {
    pub binary_id: String,
    pub executable_sha256: String,
    pub function_job_id: String,
    pub function_manifest_sha256: String,
    pub producer_rva: u32,
    pub callback_rva: u32,
    pub append_rva: u32,
    pub finalize_rva: u32,
    pub scroll_sync_rva: u32,
    pub phase: ObservationPhase,
    pub transition_sequence: u64,
    pub observation_qpc_ticks: u64,
    pub producer_record_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessControllerObservation {
    pub kind: ControllerKind,
    pub source_complete: bool,
    pub disposition_complete: bool,
    pub final_order_complete: bool,
    pub materialization_complete: bool,
    pub clip_viewport_complete: bool,
    pub scroll_complete: bool,
    pub collection: CollectionTrace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    ForeignAuthority(&'static str),
    UnsupportedFamily {
        kind: ControllerKind,
        disposition: ClosureDisposition,
    },
    IncompleteObservation(&'static str),
    InvalidCollection(String),
}

#[derive(Clone, Debug)]
pub struct ExactA729ControllerObservationSource {
    frame_hour: u64,
    collections: Vec<CollectionTrace>,
}

impl ExactA729ControllerObservationSource {
    pub fn try_admit(
        frame_hour: u64,
        receipt: HarnessAuthorityReceipt,
        observations: Vec<HarnessControllerObservation>,
    ) -> Result<Self, AdmissionError> {
        validate_equipment_receipt(&receipt)?;
        if observations.is_empty() {
            return Err(AdmissionError::IncompleteObservation(
                "owner-runnable receipt contains no collection observations",
            ));
        }

        let mut collections = Vec::with_capacity(observations.len());
        for observation in observations {
            let disposition = closure_disposition(observation.kind);
            if disposition != ClosureDisposition::OwnerRunnable {
                return Err(AdmissionError::UnsupportedFamily {
                    kind: observation.kind,
                    disposition,
                });
            }
            if observation.kind.family() != CollectionFamily::StandardGrid {
                return Err(AdmissionError::UnsupportedFamily {
                    kind: observation.kind,
                    disposition,
                });
            }
            require_complete(&observation)?;
            validate_equipment_grid_collection(frame_hour, &observation.collection)?;
            collections.push(observation.collection);
        }

        let frame = TraceFrame {
            elapsed_wall_ms: 0,
            status_flags: 0,
            frame_hour,
            collections: collections.clone(),
        };
        validate_frame(&frame)
            .map_err(|error| AdmissionError::InvalidCollection(format!("{error:?}")))?;
        Ok(Self {
            frame_hour,
            collections,
        })
    }
}

impl crate::source_seal::Sealed for ExactA729ControllerObservationSource {}

impl ExactControllerTraceSource for ExactA729ControllerObservationSource {
    fn is_exact_a729_controller_source(&self) -> bool {
        true
    }

    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<CollectionTrace>, String> {
        if frame_hour != self.frame_hour {
            return Err(format!(
                "A28 observation hour {} differs from requested hour {frame_hour}",
                self.frame_hour
            ));
        }
        Ok(self.collections.clone())
    }
}

fn validate_equipment_receipt(receipt: &HarnessAuthorityReceipt) -> Result<(), AdmissionError> {
    if receipt.binary_id != A729_BINARY_ID {
        return Err(AdmissionError::ForeignAuthority("binary id"));
    }
    if receipt.executable_sha256 != A729_EXECUTABLE_SHA256 {
        return Err(AdmissionError::ForeignAuthority("executable SHA-256"));
    }
    if receipt.function_job_id != EQUIPMENT_GRID_JOB_ID {
        return Err(AdmissionError::ForeignAuthority("function job id"));
    }
    if receipt.function_manifest_sha256 != EQUIPMENT_GRID_MANIFEST_SHA256 {
        return Err(AdmissionError::ForeignAuthority(
            "function manifest SHA-256",
        ));
    }
    if receipt.producer_rva != EQUIPMENT_GRID_PRODUCER_RVA
        || receipt.callback_rva != EQUIPMENT_GRID_CALLBACK_RVA
        || receipt.append_rva != STANDARD_GRID_APPEND_RVA
        || receipt.finalize_rva != STANDARD_GRID_FINALIZE_RVA
        || receipt.scroll_sync_rva != CONTAINER_SCROLL_SYNC_RVA
    {
        return Err(AdmissionError::ForeignAuthority(
            "producer/callback/helper RVA set",
        ));
    }
    if receipt.phase != ObservationPhase::PostReturn {
        return Err(AdmissionError::ForeignAuthority(
            "observation is not post-return",
        ));
    }
    if receipt.transition_sequence == 0 || receipt.observation_qpc_ticks == 0 {
        return Err(AdmissionError::ForeignAuthority(
            "transition sequence or QPC timestamp",
        ));
    }
    if !lower_hex_sha256(&receipt.producer_record_sha256) {
        return Err(AdmissionError::ForeignAuthority("producer record SHA-256"));
    }
    Ok(())
}

fn require_complete(observation: &HarnessControllerObservation) -> Result<(), AdmissionError> {
    let fields = [
        (observation.source_complete, "source accounting"),
        (
            observation.disposition_complete,
            "inclusion/exclusion disposition",
        ),
        (observation.final_order_complete, "final order"),
        (
            observation.materialization_complete,
            "materialized slot binding",
        ),
        (observation.clip_viewport_complete, "resolved clip/viewport"),
        (observation.scroll_complete, "fixed scroll state"),
    ];
    for (complete, label) in fields {
        if !complete {
            return Err(AdmissionError::IncompleteObservation(label));
        }
    }
    Ok(())
}

fn validate_equipment_grid_collection(
    frame_hour: u64,
    collection: &CollectionTrace,
) -> Result<(), AdmissionError> {
    if collection.frame_hour != frame_hour {
        return Err(invalid("collection/frame hour mismatch"));
    }
    if !collection.group_boundaries.is_empty() {
        return Err(invalid(
            "equipment variant grid has no controller grouping operation",
        ));
    }
    for (ordinal, item) in collection.source_items.iter().enumerate() {
        if item.source_ordinal as usize != ordinal {
            return Err(invalid(
                "equipment source ordinals are not contiguous source order",
            ));
        }
        portable_atom(&item.stable_item_id, "stable item id")?;
        if let Some(reason) = &item.filter_reason {
            portable_atom(reason, "filter reason")?;
        }
    }
    if collection.materialized_slots.len() != collection.ordered_item_ids.len() {
        return Err(invalid(
            "equipment grid must materialize every accepted row at commit",
        ));
    }
    for (ordered_id, slot) in collection
        .ordered_item_ids
        .iter()
        .zip(&collection.materialized_slots)
    {
        if slot.bound_item_id != *ordered_id {
            return Err(invalid(
                "materialized grid traversal differs from accepted order",
            ));
        }
        portable_atom(&slot.slot_id, "slot id")?;
        portable_atom(&slot.bound_item_id, "bound item id")?;
        portable_atom(&slot.row_template_id, "slot template id")?;
    }
    portable_atom(&collection.collection_instance_id, "collection instance id")?;
    portable_atom(&collection.row_template_id, "collection template id")?;
    Ok(())
}

fn portable_atom(value: &str, label: &'static str) -> Result<(), AdmissionError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed != value
        || trimmed.len() > crate::MAX_STRING_BYTES
        || trimmed.contains('\0')
        || trimmed.contains('\\')
        || trimmed.contains("0x")
        || trimmed.contains("0X")
    {
        return Err(invalid(label));
    }
    Ok(())
}

fn lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid(message: impl Into<String>) -> AdmissionError {
    AdmissionError::InvalidCollection(message.into())
}
