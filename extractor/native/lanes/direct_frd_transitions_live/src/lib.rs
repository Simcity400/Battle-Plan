#![deny(unsafe_op_in_unsafe_fn)]

use std::fmt;

#[cfg(windows)]
pub mod a729_hook;

pub const STATIC_EXACT20_ROOT_AUTHORITY: bool = true;
pub const LIVE_INVOCATION_AUTHORITY: bool = false;
pub const PRODUCTION_INSTALL_AUTHORIZED: bool = STATIC_EXACT20_ROOT_AUTHORITY;
pub const PRODUCTION_AUTHORITY: bool = PRODUCTION_INSTALL_AUTHORIZED;
pub const PRODUCTION_ACTIVATION: bool = PRODUCTION_AUTHORITY;

pub const AUTHORITY_BINARY_VERSION: &str = "hoi4.exe 1.19.2.0.a729";
pub const AUTHORITY_IMAGE_BASE: u64 = 0x0000_0001_4000_0000;
pub const AUTHORITY_BINARY_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const AUTHORITY_EXACT20_JOB: &str = "20260720T020813Z-w3-focus-direct-hooks-d987b89a";
pub const AUTHORITY_EXACT20_SHARD: &str = "shard-15";
pub const AUTHORITY_EXACT20_READY_SHARD_COUNT: usize = 20;
pub const AUTHORITY_EXACT20_MANIFEST_SHA256: &str =
    "a9855541571204160bc4ac78dcf6f1dbdfa112496234e84540406a71c37d2fb3";
pub const AUTHORITY_EXACT20_ROOT_COUNT: usize = 6;
pub const X10_AUTHORITY_JOB: &str =
    "20260720T140344Z-x10-generic-on-action-and-bridges-root-o-a2a0ea16";
pub const X10_AUTHORITY_MANIFEST_SHA256: &str =
    "8012d37ac03a47187bc2744e509acac0dbec1e6a7a91ffd20d45d58903e0e3da";
pub const X10_TERMINAL_AUTHORITY_JOB: &str =
    "20260720T135537Z-x10-direct-and-decision-terminal-mutator-e7b8e477";
pub const X10_TERMINAL_AUTHORITY_MANIFEST_SHA256: &str =
    "213944e845d02896efd26314003a7880384e9d1eddf4a2b799d4449632da8227";
pub const X10_CLUSTER_EXHAUSTION_AUTHORITY_JOB: &str =
    "20260720T214321Z-w5-x10-on-action-cluster-and-bridges-fin-3733bf4e";
pub const X10_CLUSTER_EXHAUSTION_MANIFEST_SHA256: &str =
    "d94eae0f339ee53f6ce69ecc921925519f48138da756ebef353304e31a8d9603";

pub const D45_V5_SCHEMA_VERSION: u16 = 5;
pub const D45_V5_DIRECT_EVENT_TRAILER_MAGIC: [u8; 4] = *b"D45E";
pub const D45_V5_DIRECT_EVENT_TRAILER_VERSION: u16 = 3;
pub const D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN: usize = 56;
pub const D45_V5_DIRECT_EVENT_ROW_LEN: usize = 120;
pub const DEFAULT_MAX_PENDING_ROWS: usize = 4096;

pub const D45_V5_DIRECT_EVENT_TRAILER_FLAG_POINTERS_ERASED: u16 = 1;
pub const D45_V5_DIRECT_EVENT_TRAILER_FLAG_PRODUCER_COMPLETE: u16 = 1 << 1;
pub const D45_V5_DIRECT_EVENT_TRAILER_FLAG_STATIC_AUTHORITY_BOUND: u16 = 1 << 2;
pub const D45_V5_DIRECT_EVENT_TRAILER_FLAG_EVIDENCE_FIXTURE: u16 = 1 << 3;
pub const D45_V5_DIRECT_EVENT_TRAILER_KNOWN_FLAGS: u16 =
    D45_V5_DIRECT_EVENT_TRAILER_FLAG_POINTERS_ERASED
        | D45_V5_DIRECT_EVENT_TRAILER_FLAG_PRODUCER_COMPLETE
        | D45_V5_DIRECT_EVENT_TRAILER_FLAG_STATIC_AUTHORITY_BOUND
        | D45_V5_DIRECT_EVENT_TRAILER_FLAG_EVIDENCE_FIXTURE;
pub const D45_V5_DIRECT_EVENT_REQUIRED_ROOT_MASK: u64 = (1 << 12) - 1;

pub const ROOT_RVA_FOCUS_COMPLETED_A729: u32 = 0x002c_ef60;
pub const ROOT_RVA_RESEARCH_COMPLETED_A729: u32 = 0x00ec_e360;
pub const ROOT_RVA_ORDINARY_DECISION_TAKEN_A729: u32 = 0x0072_8bc0;
pub const ROOT_RVA_TARGETED_DECISION_TAKEN_A729: u32 = 0x0072_8ce0;
pub const ROOT_RVA_TIMED_DECISION_ACTIVATED_A729: u32 = 0x0072_8fb0;
pub const ROOT_RVA_DECISION_LIFECYCLE_CORRELATION_A729: u32 = 0x0072_3060;
pub const ROOT_RVA_ON_ACTION_INVOKED_A729: u32 = 0x00a6_aae0;
pub const ROOT_RVA_DECISION_CANCEL_EFFECT_A729: u32 = 0x0071_b350;
pub const ROOT_RVA_DECISION_REMOVE_EFFECT_A729: u32 = 0x0071_b590;
pub const ROOT_RVA_DECISION_TIMEOUT_EFFECT_A729: u32 = 0x0071_c490;
pub const ROOT_RVA_TARGETED_DECISION_CANCEL_EFFECT_A729: u32 = 0x0071_b7d0;
pub const ROOT_RVA_TARGETED_DECISION_REMOVE_EFFECT_A729: u32 = 0x0071_be10;
pub const ROOT_RVA_TARGETED_DECISION_TIMEOUT_EFFECT_A729: u32 = 0x0071_c130;

pub const AUTHORITY_EXACT20_ROOT_RVAS_A729: [u32; AUTHORITY_EXACT20_ROOT_COUNT] = [
    ROOT_RVA_FOCUS_COMPLETED_A729,
    ROOT_RVA_RESEARCH_COMPLETED_A729,
    ROOT_RVA_ORDINARY_DECISION_TAKEN_A729,
    ROOT_RVA_TARGETED_DECISION_TAKEN_A729,
    ROOT_RVA_TIMED_DECISION_ACTIVATED_A729,
    ROOT_RVA_DECISION_LIFECYCLE_CORRELATION_A729,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum DirectEventKind {
    FocusCompleted = 1,
    ResearchCompleted = 2,
    OrdinaryDecisionTaken = 3,
    TargetedDecisionTaken = 4,
    TimedDecisionActivated = 5,
    OnActionInvoked = 6,
    DecisionCancelEffectInvoked = 7,
    DecisionRemoveEffectInvoked = 8,
    DecisionTimeoutEffectInvoked = 9,
    TargetedDecisionCancelEffectInvoked = 10,
    TargetedDecisionRemoveEffectInvoked = 11,
    TargetedDecisionTimeoutEffectInvoked = 12,
}

compact_contract::wire_coded_codes! {
    DirectEventKind: u32,
    code = pub code,
    Ok = from_wire(value) -> Result<Self, CarrierError> { Err(CarrierError::InvalidKind(value)) };

    FocusCompleted = 1,
    ResearchCompleted = 2,
    OrdinaryDecisionTaken = 3,
    TargetedDecisionTaken = 4,
    TimedDecisionActivated = 5,
    OnActionInvoked = 6,
    DecisionCancelEffectInvoked = 7,
    DecisionRemoveEffectInvoked = 8,
    DecisionTimeoutEffectInvoked = 9,
    TargetedDecisionCancelEffectInvoked = 10,
    TargetedDecisionRemoveEffectInvoked = 11,
    TargetedDecisionTimeoutEffectInvoked = 12,
}

impl DirectEventKind {
    pub const fn expected_root_rva_a729(self) -> u32 {
        match self {
            Self::FocusCompleted => ROOT_RVA_FOCUS_COMPLETED_A729,
            Self::ResearchCompleted => ROOT_RVA_RESEARCH_COMPLETED_A729,
            Self::OrdinaryDecisionTaken => ROOT_RVA_ORDINARY_DECISION_TAKEN_A729,
            Self::TargetedDecisionTaken => ROOT_RVA_TARGETED_DECISION_TAKEN_A729,
            Self::TimedDecisionActivated => ROOT_RVA_TIMED_DECISION_ACTIVATED_A729,
            Self::OnActionInvoked => ROOT_RVA_ON_ACTION_INVOKED_A729,
            Self::DecisionCancelEffectInvoked => ROOT_RVA_DECISION_CANCEL_EFFECT_A729,
            Self::DecisionRemoveEffectInvoked => ROOT_RVA_DECISION_REMOVE_EFFECT_A729,
            Self::DecisionTimeoutEffectInvoked => ROOT_RVA_DECISION_TIMEOUT_EFFECT_A729,
            Self::TargetedDecisionCancelEffectInvoked => {
                ROOT_RVA_TARGETED_DECISION_CANCEL_EFFECT_A729
            }
            Self::TargetedDecisionRemoveEffectInvoked => {
                ROOT_RVA_TARGETED_DECISION_REMOVE_EFFECT_A729
            }
            Self::TargetedDecisionTimeoutEffectInvoked => {
                ROOT_RVA_TARGETED_DECISION_TIMEOUT_EFFECT_A729
            }
        }
    }

    pub const fn is_entry_observation(self) -> bool {
        matches!(
            self,
            Self::OnActionInvoked
                | Self::DecisionCancelEffectInvoked
                | Self::DecisionRemoveEffectInvoked
                | Self::DecisionTimeoutEffectInvoked
                | Self::TargetedDecisionCancelEffectInvoked
                | Self::TargetedDecisionRemoveEffectInvoked
                | Self::TargetedDecisionTimeoutEffectInvoked
        )
    }

    pub const fn requires_target_argument(self) -> bool {
        matches!(
            self,
            Self::TargetedDecisionTaken
                | Self::TargetedDecisionCancelEffectInvoked
                | Self::TargetedDecisionRemoveEffectInvoked
                | Self::TargetedDecisionTimeoutEffectInvoked
        )
    }

    pub const fn root_mask_bit(self) -> u64 {
        1 << ((self as u32) - 1)
    }

    pub const fn requires_owner_key(self) -> bool {
        matches!(
            self,
            Self::FocusCompleted
                | Self::OrdinaryDecisionTaken
                | Self::TargetedDecisionTaken
                | Self::TimedDecisionActivated
                | Self::DecisionCancelEffectInvoked
                | Self::DecisionRemoveEffectInvoked
                | Self::DecisionTimeoutEffectInvoked
                | Self::TargetedDecisionCancelEffectInvoked
                | Self::TargetedDecisionRemoveEffectInvoked
                | Self::TargetedDecisionTimeoutEffectInvoked
        )
    }

    pub const fn requires_subject_token_hash(self) -> bool {
        !matches!(self, Self::ResearchCompleted)
    }

    pub const fn requires_subject_key(self) -> bool {
        matches!(self, Self::ResearchCompleted)
    }

}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct OpaqueInstanceId(u64);

impl OpaqueInstanceId {
    pub const NONE: Self = Self(0);

    fn from_nonzero(value: u64) -> Result<Self, CarrierError> {
        if value == 0 {
            return Err(CarrierError::MissingInstanceId);
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn is_present(self) -> bool {
        self.0 != 0
    }
}

#[derive(Debug)]
struct OpaqueInstanceIdAllocator {
    next: u64,
}

impl Default for OpaqueInstanceIdAllocator {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl OpaqueInstanceIdAllocator {
    pub fn allocate(&mut self) -> Result<OpaqueInstanceId, CarrierError> {
        let allocated = OpaqueInstanceId::from_nonzero(self.next)?;
        self.next = self
            .next
            .checked_add(1)
            .ok_or(CarrierError::InstanceIdSpaceExhausted)?;
        Ok(allocated)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirectEventStateSummary {
    pub state_bits: u64,
    pub progress_raw: i32,
    pub remaining_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectEventRow {
    pub sequence: u64,
    pub kind: DirectEventKind,
    pub thread_id: u32,
    pub game_hour_raw: i32,
    pub root_rva: u32,
    pub owner_instance_id: OpaqueInstanceId,
    pub subject_instance_id: OpaqueInstanceId,
    pub argument_instance_id: OpaqueInstanceId,
    /// Stable engine owner key where the exact ABI exposes one (for example a
    pub owner_key: u64,
    pub subject_token_hash: u64,
    pub subject_key: u64,
    pub input_bits: u64,
    pub return_bits: u64,
    pub before: DirectEventStateSummary,
    pub after: DirectEventStateSummary,
}

impl DirectEventRow {
    pub fn validate(&self) -> Result<(), CarrierError> {
        if self.sequence == 0 {
            return Err(CarrierError::ZeroSequence);
        }
        if self.thread_id == 0 {
            return Err(CarrierError::ZeroThreadId);
        }
        if !self.owner_instance_id.is_present() {
            return Err(CarrierError::MissingOwnerInstanceId);
        }
        if !self.subject_instance_id.is_present() {
            return Err(CarrierError::MissingSubjectInstanceId);
        }
        if self.kind.requires_target_argument() && !self.argument_instance_id.is_present() {
            return Err(CarrierError::MissingTargetArgumentInstanceId);
        }
        if self.kind.requires_owner_key() && self.owner_key == 0 {
            return Err(CarrierError::MissingPortableOwnerKey);
        }
        if self.kind.requires_subject_token_hash() && self.subject_token_hash == 0 {
            return Err(CarrierError::MissingPortableSubjectTokenHash);
        }
        if self.kind.requires_subject_key() && self.subject_key == 0 {
            return Err(CarrierError::MissingPortableSubjectKey);
        }
        let expected_rva = self.kind.expected_root_rva_a729();
        if self.root_rva != expected_rva {
            return Err(CarrierError::RootRvaMismatch {
                kind: self.kind,
                expected: expected_rva,
                actual: self.root_rva,
            });
        }
        if self.kind.is_entry_observation() {
            if self.return_bits != 0
                || self.before != DirectEventStateSummary::default()
                || self.after.state_bits != 1 << 1
                || self.after.progress_raw != 0
                || self.after.remaining_raw != 0
            {
                return Err(CarrierError::InvalidInvocationObservation);
            }
        } else if self.return_bits != 1
            || self.after.state_bits != (self.before.state_bits | 1 << 63)
            || self.after.progress_raw != self.before.progress_raw
            || self.after.remaining_raw != self.before.remaining_raw
        {
            return Err(CarrierError::InvalidReturnObservation);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsequencedDirectEventRow {
    pub kind: DirectEventKind,
    pub thread_id: u32,
    pub game_hour_raw: i32,
    pub root_rva: u32,
    pub owner_instance_id: OpaqueInstanceId,
    pub subject_instance_id: OpaqueInstanceId,
    pub argument_instance_id: OpaqueInstanceId,
    pub owner_key: u64,
    pub subject_token_hash: u64,
    pub subject_key: u64,
    pub input_bits: u64,
    pub return_bits: u64,
    pub before: DirectEventStateSummary,
    pub after: DirectEventStateSummary,
}

#[derive(Debug)]
pub struct DirectEventCarrier {
    next_sequence: u64,
    instance_ids: OpaqueInstanceIdAllocator,
    max_pending_rows: usize,
    pending: Vec<DirectEventRow>,
    poisoned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectEventBatch {
    pub hook_epoch: u64,
    pub drain_sequence: u64,
    pub installed_root_mask: u64,
    pub observed_kind_mask: u64,
    pub sequence_cursor: u64,
    pub producer_complete: bool,
    pub evidence_fixture: bool,
    pub rows: Vec<DirectEventRow>,
}

impl DirectEventBatch {
    pub fn validate(&self) -> Result<(), CarrierError> {
        if self.hook_epoch == 0 {
            return Err(CarrierError::ZeroHookEpoch);
        }
        if self.drain_sequence == 0 {
            return Err(CarrierError::ZeroDrainSequence);
        }
        if self.installed_root_mask != D45_V5_DIRECT_EVENT_REQUIRED_ROOT_MASK {
            return Err(CarrierError::IncompleteInstalledRootMask(
                self.installed_root_mask,
            ));
        }
        if self.observed_kind_mask & !D45_V5_DIRECT_EVENT_REQUIRED_ROOT_MASK != 0
            || self
                .rows
                .iter()
                .any(|row| self.observed_kind_mask & row.kind.root_mask_bit() == 0)
        {
            return Err(CarrierError::InvalidObservedKindMask(
                self.observed_kind_mask,
            ));
        }
        if self.sequence_cursor == 0 {
            return Err(CarrierError::ZeroSequence);
        }
        if !self.producer_complete {
            return Err(CarrierError::ProducerIncomplete);
        }
        let expected_first = self
            .rows
            .first()
            .map(|row| row.sequence)
            .unwrap_or(self.sequence_cursor);
        validate_sequence(&self.rows, expected_first)?;
        let expected_cursor = self
            .rows
            .last()
            .map(|row| {
                row.sequence
                    .checked_add(1)
                    .ok_or(CarrierError::SequenceSpaceExhausted)
            })
            .transpose()?
            .unwrap_or(self.sequence_cursor);
        if expected_cursor != self.sequence_cursor {
            return Err(CarrierError::SequenceCursorMismatch {
                expected: expected_cursor,
                actual: self.sequence_cursor,
            });
        }
        Ok(())
    }
}

impl Default for DirectEventCarrier {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectEventCarrier {
    pub fn new() -> Self {
        Self::with_max_pending_rows(DEFAULT_MAX_PENDING_ROWS)
            .expect("default D45 direct-event row capacity is nonzero")
    }

    pub fn with_max_pending_rows(max_pending_rows: usize) -> Result<Self, CarrierError> {
        if max_pending_rows == 0 {
            return Err(CarrierError::QueueCapacityZero);
        }
        Ok(Self {
            next_sequence: 1,
            instance_ids: OpaqueInstanceIdAllocator::default(),
            max_pending_rows,
            pending: Vec::with_capacity(max_pending_rows),
            poisoned: false,
        })
    }

    pub fn allocate_instance_id(&mut self) -> Result<OpaqueInstanceId, CarrierError> {
        if self.poisoned {
            return Err(CarrierError::CarrierPoisoned);
        }
        match self.instance_ids.allocate() {
            Ok(id) => Ok(id),
            Err(error) => {
                self.poisoned = true;
                Err(error)
            }
        }
    }

    pub fn capture(&mut self, input: UnsequencedDirectEventRow) -> Result<u64, CarrierError> {
        if self.poisoned {
            return Err(CarrierError::CarrierPoisoned);
        }
        if self.pending.len() >= self.max_pending_rows {
            self.poisoned = true;
            return Err(CarrierError::PendingQueueFull {
                capacity: self.max_pending_rows,
            });
        }
        let sequence = self.next_sequence;
        let Some(next_sequence) = sequence.checked_add(1) else {
            self.poisoned = true;
            return Err(CarrierError::SequenceSpaceExhausted);
        };
        let row = DirectEventRow {
            sequence,
            kind: input.kind,
            thread_id: input.thread_id,
            game_hour_raw: input.game_hour_raw,
            root_rva: input.root_rva,
            owner_instance_id: input.owner_instance_id,
            subject_instance_id: input.subject_instance_id,
            argument_instance_id: input.argument_instance_id,
            owner_key: input.owner_key,
            subject_token_hash: input.subject_token_hash,
            subject_key: input.subject_key,
            input_bits: input.input_bits,
            return_bits: input.return_bits,
            before: input.before,
            after: input.after,
        };
        if let Err(error) = row.validate() {
            self.poisoned = true;
            return Err(error);
        }
        self.pending.push(row);
        self.next_sequence = next_sequence;
        Ok(sequence)
    }

    pub fn pending_rows(&self) -> &[DirectEventRow] {
        &self.pending
    }

    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    pub fn drain_rows(&mut self) -> Result<Vec<DirectEventRow>, CarrierError> {
        if self.poisoned {
            return Err(CarrierError::CarrierPoisoned);
        }
        let mut rows = Vec::with_capacity(self.max_pending_rows);
        std::mem::swap(&mut rows, &mut self.pending);
        Ok(rows)
    }

    pub fn drain_trailer(&mut self) -> Result<Vec<u8>, CarrierError> {
        if self.poisoned {
            return Err(CarrierError::CarrierPoisoned);
        }
        let encoded = encode_trailer(&self.pending)?;
        self.pending.clear();
        Ok(encoded)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    InvalidTrailerMagic,
    InvalidTrailerVersion(u16),
    InvalidTrailerHeaderLength(u16),
    InvalidTrailerRowLength(u16),
    InvalidTrailerFlags(u16),
    ZeroHookEpoch,
    ZeroDrainSequence,
    ProducerIncomplete,
    IncompleteInstalledRootMask(u64),
    InvalidObservedKindMask(u64),
    SequenceCursorMismatch {
        expected: u64,
        actual: u64,
    },
    TrailerLengthOverflow,
    TrailerLengthMismatch {
        expected: usize,
        actual: usize,
    },
    InvalidKind(u32),
    MissingInstanceId,
    MissingOwnerInstanceId,
    MissingSubjectInstanceId,
    MissingTargetArgumentInstanceId,
    MissingPortableOwnerKey,
    MissingPortableSubjectTokenHash,
    MissingPortableSubjectKey,
    ZeroSequence,
    ZeroThreadId,
    RootRvaMismatch {
        kind: DirectEventKind,
        expected: u32,
        actual: u32,
    },
    InvalidInvocationObservation,
    InvalidReturnObservation,
    UnexpectedInitialSequence {
        expected: u64,
        actual: u64,
    },
    SequenceDiscontinuity {
        previous: u64,
        actual: u64,
    },
    SequenceSpaceExhausted,
    InstanceIdSpaceExhausted,
    CarrierPoisoned,
    QueueCapacityZero,
    PendingQueueFull {
        capacity: usize,
    },
}

impl fmt::Display for CarrierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CarrierError {}

pub fn encode_row(row: &DirectEventRow) -> Result<[u8; D45_V5_DIRECT_EVENT_ROW_LEN], CarrierError> {
    row.validate()?;
    let mut bytes = [0u8; D45_V5_DIRECT_EVENT_ROW_LEN];
    bytes[0..8].copy_from_slice(&row.sequence.to_le_bytes());
    bytes[8..12].copy_from_slice(&row.kind.code().to_le_bytes());
    bytes[12..16].copy_from_slice(&row.thread_id.to_le_bytes());
    bytes[16..20].copy_from_slice(&row.game_hour_raw.to_le_bytes());
    bytes[20..24].copy_from_slice(&row.root_rva.to_le_bytes());
    bytes[24..32].copy_from_slice(&row.owner_instance_id.get().to_le_bytes());
    bytes[32..40].copy_from_slice(&row.subject_instance_id.get().to_le_bytes());
    bytes[40..48].copy_from_slice(&row.argument_instance_id.get().to_le_bytes());
    bytes[48..56].copy_from_slice(&row.owner_key.to_le_bytes());
    bytes[56..64].copy_from_slice(&row.subject_token_hash.to_le_bytes());
    bytes[64..72].copy_from_slice(&row.subject_key.to_le_bytes());
    bytes[72..80].copy_from_slice(&row.input_bits.to_le_bytes());
    bytes[80..88].copy_from_slice(&row.return_bits.to_le_bytes());
    encode_summary(row.before, &mut bytes[88..104]);
    encode_summary(row.after, &mut bytes[104..120]);
    Ok(bytes)
}

pub fn decode_row(bytes: &[u8]) -> Result<DirectEventRow, CarrierError> {
    if bytes.len() != D45_V5_DIRECT_EVENT_ROW_LEN {
        return Err(CarrierError::TrailerLengthMismatch {
            expected: D45_V5_DIRECT_EVENT_ROW_LEN,
            actual: bytes.len(),
        });
    }
    let row = DirectEventRow {
        sequence: read_u64(bytes, 0),
        kind: DirectEventKind::from_wire(read_u32(bytes, 8))?,
        thread_id: read_u32(bytes, 12),
        game_hour_raw: read_i32(bytes, 16),
        root_rva: read_u32(bytes, 20),
        owner_instance_id: OpaqueInstanceId(read_u64(bytes, 24)),
        subject_instance_id: OpaqueInstanceId(read_u64(bytes, 32)),
        argument_instance_id: OpaqueInstanceId(read_u64(bytes, 40)),
        owner_key: read_u64(bytes, 48),
        subject_token_hash: read_u64(bytes, 56),
        subject_key: read_u64(bytes, 64),
        input_bits: read_u64(bytes, 72),
        return_bits: read_u64(bytes, 80),
        before: decode_summary(bytes, 88),
        after: decode_summary(bytes, 104),
    };
    row.validate()?;
    Ok(row)
}

pub fn encode_trailer(rows: &[DirectEventRow]) -> Result<Vec<u8>, CarrierError> {
    let sequence_cursor = rows
        .last()
        .map(|row| {
            row.sequence
                .checked_add(1)
                .ok_or(CarrierError::SequenceSpaceExhausted)
        })
        .transpose()?
        .unwrap_or(1);
    encode_authoritative_trailer(&DirectEventBatch {
        hook_epoch: 1,
        drain_sequence: 1,
        installed_root_mask: D45_V5_DIRECT_EVENT_REQUIRED_ROOT_MASK,
        observed_kind_mask: rows
            .iter()
            .fold(0, |mask, row| mask | row.kind.root_mask_bit()),
        sequence_cursor,
        producer_complete: true,
        evidence_fixture: false,
        rows: rows.to_vec(),
    })
}

pub fn encode_authoritative_trailer(batch: &DirectEventBatch) -> Result<Vec<u8>, CarrierError> {
    batch.validate()?;
    let rows = &batch.rows;
    let row_count = u32::try_from(rows.len()).map_err(|_| CarrierError::TrailerLengthOverflow)?;
    let row_bytes = rows
        .len()
        .checked_mul(D45_V5_DIRECT_EVENT_ROW_LEN)
        .ok_or(CarrierError::TrailerLengthOverflow)?;
    let capacity = D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN
        .checked_add(row_bytes)
        .ok_or(CarrierError::TrailerLengthOverflow)?;
    let mut trailer = Vec::with_capacity(capacity);
    trailer.extend_from_slice(&D45_V5_DIRECT_EVENT_TRAILER_MAGIC);
    trailer.extend_from_slice(&D45_V5_DIRECT_EVENT_TRAILER_VERSION.to_le_bytes());
    trailer.extend_from_slice(&(D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN as u16).to_le_bytes());
    trailer.extend_from_slice(&(D45_V5_DIRECT_EVENT_ROW_LEN as u16).to_le_bytes());
    let mut flags = D45_V5_DIRECT_EVENT_TRAILER_FLAG_POINTERS_ERASED
        | D45_V5_DIRECT_EVENT_TRAILER_FLAG_PRODUCER_COMPLETE
        | D45_V5_DIRECT_EVENT_TRAILER_FLAG_STATIC_AUTHORITY_BOUND;
    if batch.evidence_fixture {
        flags |= D45_V5_DIRECT_EVENT_TRAILER_FLAG_EVIDENCE_FIXTURE;
    }
    trailer.extend_from_slice(&flags.to_le_bytes());
    trailer.extend_from_slice(&row_count.to_le_bytes());
    trailer.extend_from_slice(&batch.hook_epoch.to_le_bytes());
    trailer.extend_from_slice(&batch.installed_root_mask.to_le_bytes());
    trailer.extend_from_slice(&batch.sequence_cursor.to_le_bytes());
    trailer.extend_from_slice(&batch.drain_sequence.to_le_bytes());
    trailer.extend_from_slice(&batch.observed_kind_mask.to_le_bytes());
    for row in rows {
        trailer.extend_from_slice(&encode_row(row)?);
    }
    Ok(trailer)
}

pub fn decode_trailer(
    trailer: &[u8],
    expected_first_sequence: u64,
) -> Result<Vec<DirectEventRow>, CarrierError> {
    Ok(decode_authoritative_trailer(trailer, expected_first_sequence)?.rows)
}

pub fn decode_authoritative_trailer(
    trailer: &[u8],
    expected_first_sequence: u64,
) -> Result<DirectEventBatch, CarrierError> {
    if expected_first_sequence == 0 {
        return Err(CarrierError::ZeroSequence);
    }
    if trailer.len() < D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN {
        return Err(CarrierError::TrailerLengthMismatch {
            expected: D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN,
            actual: trailer.len(),
        });
    }
    if trailer[0..4] != D45_V5_DIRECT_EVENT_TRAILER_MAGIC {
        return Err(CarrierError::InvalidTrailerMagic);
    }
    let version = read_u16(trailer, 4);
    if version != D45_V5_DIRECT_EVENT_TRAILER_VERSION {
        return Err(CarrierError::InvalidTrailerVersion(version));
    }
    let header_len = read_u16(trailer, 6);
    if usize::from(header_len) != D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN {
        return Err(CarrierError::InvalidTrailerHeaderLength(header_len));
    }
    let row_len = read_u16(trailer, 8);
    if usize::from(row_len) != D45_V5_DIRECT_EVENT_ROW_LEN {
        return Err(CarrierError::InvalidTrailerRowLength(row_len));
    }
    let flags = read_u16(trailer, 10);
    if flags & !D45_V5_DIRECT_EVENT_TRAILER_KNOWN_FLAGS != 0
        || flags & D45_V5_DIRECT_EVENT_TRAILER_FLAG_POINTERS_ERASED == 0
        || flags & D45_V5_DIRECT_EVENT_TRAILER_FLAG_PRODUCER_COMPLETE == 0
        || flags & D45_V5_DIRECT_EVENT_TRAILER_FLAG_STATIC_AUTHORITY_BOUND == 0
    {
        return Err(CarrierError::InvalidTrailerFlags(flags));
    }
    let row_count = read_u32(trailer, 12) as usize;
    let hook_epoch = read_u64(trailer, 16);
    let installed_root_mask = read_u64(trailer, 24);
    let sequence_cursor = read_u64(trailer, 32);
    let drain_sequence = read_u64(trailer, 40);
    let observed_kind_mask = read_u64(trailer, 48);
    let expected_len = row_count
        .checked_mul(D45_V5_DIRECT_EVENT_ROW_LEN)
        .and_then(|bytes| bytes.checked_add(D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN))
        .ok_or(CarrierError::TrailerLengthOverflow)?;
    if trailer.len() != expected_len {
        return Err(CarrierError::TrailerLengthMismatch {
            expected: expected_len,
            actual: trailer.len(),
        });
    }
    let mut rows = Vec::with_capacity(row_count);
    let mut cursor = D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN;
    for _ in 0..row_count {
        rows.push(decode_row(
            &trailer[cursor..cursor + D45_V5_DIRECT_EVENT_ROW_LEN],
        )?);
        cursor += D45_V5_DIRECT_EVENT_ROW_LEN;
    }
    if let Some(first) = rows.first() {
        if first.sequence != expected_first_sequence {
            return Err(CarrierError::UnexpectedInitialSequence {
                expected: expected_first_sequence,
                actual: first.sequence,
            });
        }
    } else if sequence_cursor != expected_first_sequence {
        return Err(CarrierError::SequenceCursorMismatch {
            expected: expected_first_sequence,
            actual: sequence_cursor,
        });
    }
    let batch = DirectEventBatch {
        hook_epoch,
        drain_sequence,
        installed_root_mask,
        observed_kind_mask,
        sequence_cursor,
        producer_complete: true,
        evidence_fixture: flags & D45_V5_DIRECT_EVENT_TRAILER_FLAG_EVIDENCE_FIXTURE != 0,
        rows,
    };
    batch.validate()?;
    Ok(batch)
}

fn validate_sequence(rows: &[DirectEventRow], expected_first: u64) -> Result<(), CarrierError> {
    let mut previous: Option<u64> = None;
    for row in rows {
        match previous {
            None if row.sequence != expected_first => {
                return Err(CarrierError::UnexpectedInitialSequence {
                    expected: expected_first,
                    actual: row.sequence,
                });
            }
            Some(value) if value.checked_add(1) != Some(row.sequence) => {
                return Err(CarrierError::SequenceDiscontinuity {
                    previous: value,
                    actual: row.sequence,
                });
            }
            _ => {}
        }
        previous = Some(row.sequence);
    }
    Ok(())
}

fn encode_summary(summary: DirectEventStateSummary, destination: &mut [u8]) {
    destination[0..8].copy_from_slice(&summary.state_bits.to_le_bytes());
    destination[8..12].copy_from_slice(&summary.progress_raw.to_le_bytes());
    destination[12..16].copy_from_slice(&summary.remaining_raw.to_le_bytes());
}

fn decode_summary(bytes: &[u8], offset: usize) -> DirectEventStateSummary {
    DirectEventStateSummary {
        state_bits: read_u64(bytes, offset),
        progress_raw: read_i32(bytes, offset + 8),
        remaining_raw: read_i32(bytes, offset + 12),
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("fixed row"))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("fixed row"))
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("fixed row"))
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("fixed row"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        kind: DirectEventKind,
        owner: OpaqueInstanceId,
        subject: OpaqueInstanceId,
        argument: OpaqueInstanceId,
    ) -> UnsequencedDirectEventRow {
        UnsequencedDirectEventRow {
            kind,
            thread_id: 77,
            game_hour_raw: 1_234,
            root_rva: kind.expected_root_rva_a729(),
            owner_instance_id: owner,
            subject_instance_id: subject,
            argument_instance_id: argument,
            owner_key: if kind.requires_owner_key() { 1 } else { 0 },
            subject_token_hash: if kind.requires_subject_token_hash() {
                1
            } else {
                0
            },
            subject_key: if kind.requires_subject_key() { 1 } else { 0 },
            input_bits: 0,
            return_bits: 1,
            before: DirectEventStateSummary::default(),
            after: DirectEventStateSummary {
                state_bits: 1 << 63,
                progress_raw: 0,
                remaining_raw: 0,
            },
        }
    }

    #[test]
    fn legacy_five_kinds_share_one_pointer_free_sequence() {
        let mut carrier = DirectEventCarrier::new();
        let owner = carrier.allocate_instance_id().unwrap();
        let subject = carrier.allocate_instance_id().unwrap();
        let target = carrier.allocate_instance_id().unwrap();
        let kinds = [
            DirectEventKind::FocusCompleted,
            DirectEventKind::ResearchCompleted,
            DirectEventKind::OrdinaryDecisionTaken,
            DirectEventKind::TargetedDecisionTaken,
            DirectEventKind::TimedDecisionActivated,
        ];
        for kind in kinds {
            let argument = if kind == DirectEventKind::TargetedDecisionTaken {
                target
            } else {
                OpaqueInstanceId::NONE
            };
            carrier
                .capture(input(kind, owner, subject, argument))
                .unwrap();
        }
        let trailer = carrier.drain_trailer().unwrap();
        let decoded = decode_trailer(&trailer, 1).unwrap();
        assert_eq!(
            decoded.iter().map(|row| row.sequence).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        assert_eq!(
            decoded.iter().map(|row| row.kind).collect::<Vec<_>>(),
            kinds.to_vec()
        );
        assert_eq!(
            trailer.len(),
            D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN + 5 * D45_V5_DIRECT_EVENT_ROW_LEN
        );
        assert!(STATIC_EXACT20_ROOT_AUTHORITY);
        assert_eq!(AUTHORITY_EXACT20_READY_SHARD_COUNT, 20);
        assert_eq!(AUTHORITY_EXACT20_ROOT_RVAS_A729.len(), 6);
        assert!(AUTHORITY_EXACT20_ROOT_RVAS_A729
            .contains(&ROOT_RVA_DECISION_LIFECYCLE_CORRELATION_A729));
        assert!(!LIVE_INVOCATION_AUTHORITY);
        assert!(PRODUCTION_INSTALL_AUTHORIZED);
        assert!(PRODUCTION_AUTHORITY);
        assert!(PRODUCTION_ACTIVATION);
    }

    #[test]
    fn x10_entry_kinds_are_pointer_free_and_phase_distinct() {
        let mut carrier = DirectEventCarrier::new();
        let owner = carrier.allocate_instance_id().unwrap();
        let subject = carrier.allocate_instance_id().unwrap();
        let target = carrier.allocate_instance_id().unwrap();
        let kinds = [
            DirectEventKind::OnActionInvoked,
            DirectEventKind::DecisionCancelEffectInvoked,
            DirectEventKind::DecisionRemoveEffectInvoked,
            DirectEventKind::DecisionTimeoutEffectInvoked,
            DirectEventKind::TargetedDecisionCancelEffectInvoked,
            DirectEventKind::TargetedDecisionRemoveEffectInvoked,
            DirectEventKind::TargetedDecisionTimeoutEffectInvoked,
        ];
        for kind in kinds {
            let mut row = input(
                kind,
                owner,
                subject,
                if kind.requires_target_argument() {
                    target
                } else {
                    OpaqueInstanceId::NONE
                },
            );
            row.owner_key = 1;
            row.subject_token_hash = 0x9e37_79b9_7f4a_7c15;
            row.return_bits = 0;
            row.after = DirectEventStateSummary {
                state_bits: 1 << 1,
                ..DirectEventStateSummary::default()
            };
            carrier.capture(row).unwrap();
        }
        let decoded = decode_trailer(&carrier.drain_trailer().unwrap(), 1).unwrap();
        assert_eq!(
            decoded.iter().map(|row| row.kind).collect::<Vec<_>>(),
            kinds.to_vec()
        );
        assert!(decoded.iter().all(|row| row.kind.is_entry_observation()));
        assert!(decoded.iter().all(|row| row.return_bits == 0));
        assert!(decoded
            .iter()
            .all(|row| row.subject_token_hash == 0x9e37_79b9_7f4a_7c15));
    }

    #[test]
    fn root_rva_and_target_argument_are_fail_closed() {
        let mut wrong_root_carrier = DirectEventCarrier::new();
        let owner = wrong_root_carrier.allocate_instance_id().unwrap();
        let subject = wrong_root_carrier.allocate_instance_id().unwrap();
        let mut wrong_root = input(
            DirectEventKind::FocusCompleted,
            owner,
            subject,
            OpaqueInstanceId::NONE,
        );
        wrong_root.root_rva = ROOT_RVA_RESEARCH_COMPLETED_A729;
        assert!(matches!(
            wrong_root_carrier.capture(wrong_root),
            Err(CarrierError::RootRvaMismatch { .. })
        ));

        let mut missing_target_carrier = DirectEventCarrier::new();
        let owner = missing_target_carrier.allocate_instance_id().unwrap();
        let subject = missing_target_carrier.allocate_instance_id().unwrap();
        assert_eq!(
            missing_target_carrier.capture(input(
                DirectEventKind::TargetedDecisionTaken,
                owner,
                subject,
                OpaqueInstanceId::NONE,
            )),
            Err(CarrierError::MissingTargetArgumentInstanceId)
        );
    }

    #[test]
    fn bounded_drains_preserve_global_sequence() {
        let mut carrier = DirectEventCarrier::with_max_pending_rows(1).unwrap();
        let owner = carrier.allocate_instance_id().unwrap();
        let subject = carrier.allocate_instance_id().unwrap();
        carrier
            .capture(input(
                DirectEventKind::FocusCompleted,
                owner,
                subject,
                OpaqueInstanceId::NONE,
            ))
            .unwrap();
        let first = carrier.drain_rows().unwrap();
        assert_eq!(first[0].sequence, 1);
        carrier
            .capture(input(
                DirectEventKind::ResearchCompleted,
                owner,
                subject,
                OpaqueInstanceId::NONE,
            ))
            .unwrap();
        let second = carrier.drain_trailer().unwrap();
        assert_eq!(decode_trailer(&second, 2).unwrap()[0].sequence, 2);
    }

    #[test]
    fn overflow_poison_is_sticky_and_cannot_emit_partial_authority() {
        let mut carrier = DirectEventCarrier::with_max_pending_rows(1).unwrap();
        let owner = carrier.allocate_instance_id().unwrap();
        let subject = carrier.allocate_instance_id().unwrap();
        carrier
            .capture(input(
                DirectEventKind::FocusCompleted,
                owner,
                subject,
                OpaqueInstanceId::NONE,
            ))
            .unwrap();
        assert_eq!(
            carrier.capture(input(
                DirectEventKind::ResearchCompleted,
                owner,
                subject,
                OpaqueInstanceId::NONE,
            )),
            Err(CarrierError::PendingQueueFull { capacity: 1 })
        );
        assert!(carrier.is_poisoned());
        assert_eq!(carrier.drain_trailer(), Err(CarrierError::CarrierPoisoned));
    }

    #[test]
    fn trailer_rejects_sequence_gap_and_truncation() {
        let mut carrier = DirectEventCarrier::new();
        let owner = carrier.allocate_instance_id().unwrap();
        let subject = carrier.allocate_instance_id().unwrap();
        let row = DirectEventRow {
            sequence: 2,
            kind: DirectEventKind::FocusCompleted,
            thread_id: 1,
            game_hour_raw: 1,
            root_rva: ROOT_RVA_FOCUS_COMPLETED_A729,
            owner_instance_id: owner,
            subject_instance_id: subject,
            argument_instance_id: OpaqueInstanceId::NONE,
            owner_key: 1,
            subject_token_hash: 1,
            subject_key: 0,
            input_bits: 0,
            return_bits: 1,
            before: DirectEventStateSummary::default(),
            after: DirectEventStateSummary {
                state_bits: 1 << 63,
                ..DirectEventStateSummary::default()
            },
        };
        let trailer = encode_trailer(&[row]).unwrap();
        assert_eq!(
            decode_trailer(&trailer, 1),
            Err(CarrierError::UnexpectedInitialSequence {
                expected: 1,
                actual: 2,
            })
        );
        assert!(matches!(
            decode_trailer(&trailer[..trailer.len() - 1], 2),
            Err(CarrierError::TrailerLengthMismatch { .. })
        ));
    }
}
