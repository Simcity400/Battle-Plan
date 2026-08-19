use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const PREFERRED_IMAGE_BASE: u64 = 0x140000000;
pub const CAPTURE_ASSIGNMENT_DISCRIMINATOR: u32 = 0;
pub const LEADER_ASSIGNMENT_OFFSET: usize = 0x1080;
pub const LEADER_CAPTURER_COUNTRY_ID_OFFSET: usize = 0xFB0;
pub const LEADER_CAPTURE_END_DATE_OFFSET: usize = 0xFC0;
pub const RECRUITMENT_VTABLE_RVA: usize = 0x29C2400;
pub const RECRUITMENT_OBJECT_SIZE: usize = 0x58;
pub const RECRUITMENT_OWNER_COUNTRY_POINTER_OFFSET: usize = 0x08;
pub const COUNTRY_RECRUITMENT_WRAPPER_OFFSET: usize = 0xFC0;
pub const RECRUITMENT_OBJECT_OFFSET_IN_WRAPPER: usize = 0x08;
pub const RECRUITMENT_MAIN_VECTOR_OFFSET: usize = 0x28;
pub const RECRUITMENT_NOT_TO_SPY_MASTER_VECTOR_OFFSET: usize = 0x40;
pub const COUNTRY_RECRUITMENT_WRAPPER_ACCESSOR_RVA: usize = 0x06C2CB0;
pub const RECRUITABLE_VECTOR_SOURCE_RVA: usize = 0x1567CB0;

pub const CAPTURE_PRODUCER_JOB_ID: &str =
    "20260721T082430Z-wave6-a30-captured-operative-producers-35d33761";
pub const CAPTURE_PRODUCER_MANIFEST_SHA256: &str =
    "e33a74c6e0fb507b62cf38f5a0817263e24ad97d1cc75701870a7454bd33f162";
pub const RECRUITMENT_VTABLE_JOB_ID: &str =
    "20260721T092209Z-wave6-a30-operative-recruitment-vtable-4d3d2faf";
pub const RECRUITMENT_VTABLE_ARTIFACT_SHA256: &str =
    "7402b9e321881fec939ae85489d9193b5727c28ec1b16943bb3ddd175aa4521c";
pub const RECRUITMENT_SERIALIZER_JOB_ID: &str =
    "20260721T092224Z-wave6-a30-recruitment-vslot-1-3f0deb76";
pub const RECRUITMENT_SERIALIZER_MANIFEST_SHA256: &str =
    "a7a0073f50149772c678512dfca3fa9de0a86f8f36889949bc9f3af8bf7f80c0";
pub const RECRUITMENT_ITEM_CODEC_JOB_ID: &str =
    "20260721T092255Z-wave6-a30-recruitment-container-0-5e97cef5";
pub const RECRUITMENT_ITEM_CODEC_MANIFEST_SHA256: &str =
    "1e12452c61956000ea3af1b71bf7d095da1d6fb811b0e8d54a113e426803b272";
pub const RECRUITMENT_OWNER_PATH_JOB_ID: &str =
    "20260721T094102Z-wave6-a30-recruitment-country-resolver-2ae84e74";
pub const RECRUITMENT_OWNER_PATH_MANIFEST_SHA256: &str =
    "d8bfc5aa0347c0eac443328566d572203a91fb2f44db94c8479e1d4899a4e229";
pub const RECRUITABLE_VECTOR_SOURCE_JOB_ID: &str =
    "20260721T094024Z-wave6-a30-recruitable-vector-source-c30688d8";
pub const RECRUITABLE_VECTOR_SOURCE_MANIFEST_SHA256: &str =
    "07f911d109aead3e6c139fb37951ef535b789ddbb4ac34c03970e25c7a8b888a";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecruitmentVectorKind {
    RecruitableOperatives,
    RecruitableOperativesNotToSpyMaster,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureProducerObservation {
    pub frame_hour: u64,
    pub producer_record_identity: String,
    pub owner_tag_db_id: u32,
    pub operative_id: u32,
    pub assignment_discriminator: u32,
    pub captured_by_tag_db_id: Option<u32>,
    pub capture_end_date_raw: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecruitmentProducerObservation {
    pub frame_hour: u64,
    pub producer_record_identity: String,
    pub owner_tag_db_id: u32,
    pub operative_id: u32,
    pub vector_kind: RecruitmentVectorKind,
    pub source_ordinal: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperativeCaptureRecruitableSemantic {
    pub operative_id: u32,
    pub captured: bool,
    pub captured_by_tag_db_id: Option<u32>,
    pub capture_end_date_raw: Option<i64>,
    pub recruitable_pool_member: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRecruitableJoinReceipt {
    pub schema: String,
    pub executable_sha256: String,
    pub frame_hour: u64,
    pub producer_record_identity: String,
    pub owner_tag_db_id: u32,
    pub rows: Vec<OperativeCaptureRecruitableSemantic>,
    pub not_to_spy_master_operatives: Vec<u32>,
    pub capture_producer_job_id: String,
    pub capture_producer_manifest_sha256: String,
    pub recruitment_vtable_job_id: String,
    pub recruitment_vtable_artifact_sha256: String,
    pub recruitment_serializer_job_id: String,
    pub recruitment_serializer_manifest_sha256: String,
    pub recruitment_item_codec_job_id: String,
    pub recruitment_item_codec_manifest_sha256: String,
    pub recruitment_owner_path_job_id: String,
    pub recruitment_owner_path_manifest_sha256: String,
    pub recruitable_vector_source_job_id: String,
    pub recruitable_vector_source_manifest_sha256: String,
}

pub fn seal_capture_recruitable_join(
    frame_hour: u64,
    producer_record_identity: &str,
    owner_tag_db_id: u32,
    active_operative_ids: &[u32],
    capture_rows: &[CaptureProducerObservation],
    recruitment_rows: &[RecruitmentProducerObservation],
) -> Result<CaptureRecruitableJoinReceipt, String> {
    if frame_hour == 0 || !lower_sha256(producer_record_identity) || owner_tag_db_id == 0 {
        return Err("A30 capture/recruitable join has an empty owner or producer identity".into());
    }
    let active = active_operative_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if active.is_empty() || active.len() != active_operative_ids.len() || active.contains(&0) {
        return Err("A30 active operative order is empty or ambiguous".into());
    }

    let mut capture_by_id = BTreeMap::new();
    for row in capture_rows {
        if row.frame_hour != frame_hour
            || row.producer_record_identity != producer_record_identity
            || row.owner_tag_db_id != owner_tag_db_id
            || row.operative_id == 0
            || capture_by_id.insert(row.operative_id, row).is_some()
        {
            return Err("A30 capture producer identity, timing, or uniqueness mismatch".into());
        }
        let captured = row.assignment_discriminator == CAPTURE_ASSIGNMENT_DISCRIMINATOR;
        let capture_payload_is_exact = if captured {
            row.captured_by_tag_db_id.is_some_and(|id| id != 0)
                && row.capture_end_date_raw.is_some()
        } else {
            row.captured_by_tag_db_id.is_none() && row.capture_end_date_raw.is_none()
        };
        if !capture_payload_is_exact {
            return Err("A30 capture branch lacks its exact captured-by/date payload".into());
        }
    }
    if capture_by_id.keys().copied().collect::<BTreeSet<_>>() != active {
        return Err(
            "A30 capture observations do not exactly cover the active controller order".into(),
        );
    }

    let mut main_members = BTreeSet::new();
    let mut not_to_spy_master = BTreeSet::new();
    let mut ordered_vectors = BTreeMap::new();
    for row in recruitment_rows {
        if row.frame_hour != frame_hour
            || row.producer_record_identity != producer_record_identity
            || row.owner_tag_db_id != owner_tag_db_id
            || row.operative_id == 0
            || ordered_vectors
                .insert((row.vector_kind, row.source_ordinal), row.operative_id)
                .is_some()
        {
            return Err("A30 recruitment producer identity, timing, or order mismatch".into());
        }
        let inserted = match row.vector_kind {
            RecruitmentVectorKind::RecruitableOperatives => main_members.insert(row.operative_id),
            RecruitmentVectorKind::RecruitableOperativesNotToSpyMaster => {
                not_to_spy_master.insert(row.operative_id)
            }
        };
        if !inserted {
            return Err("A30 recruitment producer contains a duplicate operative".into());
        }
    }
    if !main_members.is_disjoint(&not_to_spy_master) {
        return Err("A30 main and not-to-spy-master recruitment vectors overlap".into());
    }

    for kind in [
        RecruitmentVectorKind::RecruitableOperatives,
        RecruitmentVectorKind::RecruitableOperativesNotToSpyMaster,
    ] {
        let ordinals = ordered_vectors
            .range((kind, 0)..=(kind, u32::MAX))
            .map(|((_, ordinal), _)| *ordinal)
            .collect::<Vec<_>>();
        if ordinals
            .iter()
            .copied()
            .enumerate()
            .any(|(expected, actual)| usize::try_from(actual).ok() != Some(expected))
        {
            return Err("A30 recruitment producer ordinals are not contiguous".into());
        }
    }

    let mut ordered_ids = active_operative_ids.to_vec();
    ordered_ids.extend(
        ordered_vectors
            .range(
                (RecruitmentVectorKind::RecruitableOperatives, 0)
                    ..=(RecruitmentVectorKind::RecruitableOperatives, u32::MAX),
            )
            .map(|(_, operative_id)| *operative_id)
            .filter(|operative_id| !active.contains(operative_id)),
    );
    let rows = ordered_ids
        .into_iter()
        .map(|operative_id| {
            let capture = capture_by_id.get(&operative_id).copied();
            let captured = capture.is_some_and(|row| {
                row.assignment_discriminator == CAPTURE_ASSIGNMENT_DISCRIMINATOR
            });
            OperativeCaptureRecruitableSemantic {
                operative_id,
                captured,
                captured_by_tag_db_id: capture.and_then(|row| row.captured_by_tag_db_id),
                capture_end_date_raw: capture.and_then(|row| row.capture_end_date_raw),
                recruitable_pool_member: main_members.contains(&operative_id),
            }
        })
        .collect();
    let not_to_spy_master_operatives = ordered_vectors
        .range(
            (
                RecruitmentVectorKind::RecruitableOperativesNotToSpyMaster,
                0,
            )
                ..=(
                    RecruitmentVectorKind::RecruitableOperativesNotToSpyMaster,
                    u32::MAX,
                ),
        )
        .map(|(_, operative_id)| *operative_id)
        .collect();

    Ok(CaptureRecruitableJoinReceipt {
        schema: "hoi4.a30.capture_recruitable_join.v1".into(),
        executable_sha256: EXECUTABLE_SHA256.into(),
        frame_hour,
        producer_record_identity: producer_record_identity.into(),
        owner_tag_db_id,
        rows,
        not_to_spy_master_operatives,
        capture_producer_job_id: CAPTURE_PRODUCER_JOB_ID.into(),
        capture_producer_manifest_sha256: CAPTURE_PRODUCER_MANIFEST_SHA256.into(),
        recruitment_vtable_job_id: RECRUITMENT_VTABLE_JOB_ID.into(),
        recruitment_vtable_artifact_sha256: RECRUITMENT_VTABLE_ARTIFACT_SHA256.into(),
        recruitment_serializer_job_id: RECRUITMENT_SERIALIZER_JOB_ID.into(),
        recruitment_serializer_manifest_sha256: RECRUITMENT_SERIALIZER_MANIFEST_SHA256.into(),
        recruitment_item_codec_job_id: RECRUITMENT_ITEM_CODEC_JOB_ID.into(),
        recruitment_item_codec_manifest_sha256: RECRUITMENT_ITEM_CODEC_MANIFEST_SHA256.into(),
        recruitment_owner_path_job_id: RECRUITMENT_OWNER_PATH_JOB_ID.into(),
        recruitment_owner_path_manifest_sha256: RECRUITMENT_OWNER_PATH_MANIFEST_SHA256.into(),
        recruitable_vector_source_job_id: RECRUITABLE_VECTOR_SOURCE_JOB_ID.into(),
        recruitable_vector_source_manifest_sha256: RECRUITABLE_VECTOR_SOURCE_MANIFEST_SHA256.into(),
    })
}

pub fn validate_capture_recruitable_join_receipt(
    receipt: &CaptureRecruitableJoinReceipt,
    frame_hour: u64,
    producer_record_identity: &str,
    owner_tag_db_id: u32,
    active_operative_ids: &[u32],
) -> Result<(), String> {
    if receipt.schema != "hoi4.a30.capture_recruitable_join.v1"
        || receipt.executable_sha256 != EXECUTABLE_SHA256
        || receipt.frame_hour != frame_hour
        || receipt.producer_record_identity != producer_record_identity
        || receipt.owner_tag_db_id != owner_tag_db_id
        || receipt.capture_producer_job_id != CAPTURE_PRODUCER_JOB_ID
        || receipt.capture_producer_manifest_sha256 != CAPTURE_PRODUCER_MANIFEST_SHA256
        || receipt.recruitment_vtable_job_id != RECRUITMENT_VTABLE_JOB_ID
        || receipt.recruitment_vtable_artifact_sha256 != RECRUITMENT_VTABLE_ARTIFACT_SHA256
        || receipt.recruitment_serializer_job_id != RECRUITMENT_SERIALIZER_JOB_ID
        || receipt.recruitment_serializer_manifest_sha256 != RECRUITMENT_SERIALIZER_MANIFEST_SHA256
        || receipt.recruitment_item_codec_job_id != RECRUITMENT_ITEM_CODEC_JOB_ID
        || receipt.recruitment_item_codec_manifest_sha256 != RECRUITMENT_ITEM_CODEC_MANIFEST_SHA256
        || receipt.recruitment_owner_path_job_id != RECRUITMENT_OWNER_PATH_JOB_ID
        || receipt.recruitment_owner_path_manifest_sha256 != RECRUITMENT_OWNER_PATH_MANIFEST_SHA256
        || receipt.recruitable_vector_source_job_id != RECRUITABLE_VECTOR_SOURCE_JOB_ID
        || receipt.recruitable_vector_source_manifest_sha256
            != RECRUITABLE_VECTOR_SOURCE_MANIFEST_SHA256
        || !lower_sha256(&receipt.producer_record_identity)
    {
        return Err("A30 capture/recruitable receipt authority or identity mismatch".into());
    }
    let active = active_operative_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if active.is_empty()
        || active.len() != active_operative_ids.len()
        || receipt.rows.len() < active_operative_ids.len()
        || receipt
            .rows
            .iter()
            .take(active_operative_ids.len())
            .map(|row| row.operative_id)
            .ne(active_operative_ids.iter().copied())
    {
        return Err("A30 capture/recruitable receipt does not preserve controller order".into());
    }
    let mut row_ids = BTreeSet::new();
    for row in &receipt.rows {
        let capture_payload_is_exact = if row.captured {
            row.captured_by_tag_db_id.is_some_and(|id| id != 0)
                && row.capture_end_date_raw.is_some()
        } else {
            row.captured_by_tag_db_id.is_none() && row.capture_end_date_raw.is_none()
        };
        if row.operative_id == 0 || !row_ids.insert(row.operative_id) || !capture_payload_is_exact {
            return Err(
                "A30 capture/recruitable receipt has duplicate or half-populated rows".into(),
            );
        }
        if !active.contains(&row.operative_id) && !row.recruitable_pool_member {
            return Err("A30 non-controller row lacks main recruitable-vector membership".into());
        }
    }
    let excluded = receipt
        .not_to_spy_master_operatives
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if excluded.len() != receipt.not_to_spy_master_operatives.len()
        || excluded.contains(&0)
        || receipt
            .rows
            .iter()
            .any(|row| row.recruitable_pool_member && excluded.contains(&row.operative_id))
    {
        return Err("A30 recruitment vector identity or exclusion invariant failed".into());
    }
    Ok(())
}

fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
