//! Bounded, frame-independent freeze-census payload (`H4FC`).
//!
//! The enclosing H4CR record closes no game hour and claims no domain
//! coverage. This payload carries only rows that were already observed after a
//! freeze ended. Encoding may allocate, but it is never called under a freeze.
//!
//! The encoder distinguishes contradictory input from bounded observation:
//! counter regression, impossible rows, and invalid ordering return `Err`;
//! more than 256 rows, an individual label longer than 1,024 bytes, or a label
//! blob longer than 16,384 bytes cuts the retained row prefix and sets
//! [`FreezeCensusQuality::encoder_bound_truncated`]. Thus a bounded payload
//! still seals without pretending that its retained rows are complete.

use core::fmt;

use crate::record_semantics::KnownRecordType;

pub const FREEZE_CENSUS_MAGIC: &[u8; 4] = b"H4FC";
pub const FREEZE_CENSUS_VERSION: u16 = 1;
pub const FREEZE_CENSUS_HEADER_LEN: usize = 176;
pub const FREEZE_CENSUS_ROW_LEN: usize = 40;
pub const FREEZE_CENSUS_MAX_ROWS: usize = 256;
pub const FREEZE_CENSUS_MAX_LABEL_BLOB_LEN: usize = 16_384;
pub const FREEZE_CENSUS_MAX_LABEL_LEN: usize = 1_024;
pub const FREEZE_CENSUS_MAX_PAYLOAD_LEN: usize = FREEZE_CENSUS_HEADER_LEN
    + FREEZE_CENSUS_MAX_ROWS * FREEZE_CENSUS_ROW_LEN
    + FREEZE_CENSUS_MAX_LABEL_BLOB_LEN;

/// Shared release-epilogue policy authority.
///
/// Flipping this is an OWNER RULING selecting option (a), a bounded H4CR
/// record after the last frame boundary. It is false while the owner may
/// instead select option (b), an authenticated container receipt, or option
/// (c), a pre-release flush that cannot observe teardown freezes.
pub const RELEASE_FREEZE_CENSUS_EPILOGUE_ENABLED: bool = false;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezeCensusPlacement {
    InstallPreamble,
    ReleaseEpilogue,
}

/// Pure placement authority compiled for both the shipped policy and the
/// owner-ruled epilogue policy in tests.
pub fn validate_freeze_census_placement_policy(
    release_epilogue_enabled: bool,
    phase: FreezeCensusPhase,
    campaign_identity_present: bool,
    sequence: u64,
    ordinary_record_count: u64,
    previous_record_type: Option<KnownRecordType>,
    release_epilogue_already_seen: bool,
) -> Result<FreezeCensusPlacement, String> {
    match phase {
        FreezeCensusPhase::Install => {
            if !campaign_identity_present
                || sequence != 1
                || ordinary_record_count != 0
                || previous_record_type.is_some()
            {
                return Err(
                    "install FREEZE_CENSUS must be sequence 1 immediately after an admitted CID1"
                        .to_string(),
                );
            }
            Ok(FreezeCensusPlacement::InstallPreamble)
        }
        FreezeCensusPhase::Release => {
            if !release_epilogue_enabled {
                return Err(
                    "release FREEZE_CENSUS epilogue is disabled pending an owner ruling"
                        .to_string(),
                );
            }
            if release_epilogue_already_seen {
                return Err("release FREEZE_CENSUS epilogue is duplicated".to_string());
            }
            let expected_sequence = ordinary_record_count
                .checked_add(1)
                .ok_or_else(|| "release FREEZE_CENSUS sequence is exhausted".to_string())?;
            if sequence != expected_sequence {
                return Err(format!(
                    "release FREEZE_CENSUS sequence {sequence} is not contiguous {expected_sequence}"
                ));
            }
            if ordinary_record_count == 0
                || !matches!(
                    previous_record_type,
                    Some(KnownRecordType::FrameEnd | KnownRecordType::FrameRefused)
                )
            {
                return Err(
                    "release FREEZE_CENSUS must immediately follow a canonical FRAME_END or FRAME_REFUSED"
                        .to_string(),
                );
            }
            Ok(FreezeCensusPlacement::ReleaseEpilogue)
        }
    }
}

/// Pure EOF side of the bounded epilogue grammar: once a release census was
/// accepted, no further physical H4CR record is permitted.
pub fn validate_record_allowed_after_release_epilogue(
    release_epilogue_seen: bool,
) -> Result<(), String> {
    if release_epilogue_seen {
        Err("release FREEZE_CENSUS must be the last physical record before EOF".to_string())
    } else {
        Ok(())
    }
}

/// Pure physical-EOF policy shared by whole-stream consumers. A release row
/// closes the stream only once, immediately after a clean world boundary.
pub const fn release_epilogue_closes_stream_policy(
    release_epilogue_enabled: bool,
    previous_record_closed_world: bool,
    phase: FreezeCensusPhase,
    release_epilogue_already_seen: bool,
) -> bool {
    release_epilogue_enabled
        && previous_record_closed_world
        && matches!(phase, FreezeCensusPhase::Release)
        && !release_epilogue_already_seen
}

const QUALITY_ATTEMPTS_UNREPRESENTED: u8 = 1 << 0;
const QUALITY_CENSUS_OVERFLOWED_BEFORE_PHASE: u8 = 1 << 1;
const QUALITY_CENSUS_DROPPED_DURING_PHASE: u8 = 1 << 2;
const QUALITY_PROCESS_THREAD_ATTEMPTS_DIFFER: u8 = 1 << 3;
const QUALITY_COUNTER_RECONCILIATION: u8 = 1 << 4;
// The accepted design required encoder-bound cuts to be named but left bits
// 5..=7 unassigned. Bit 5 is the smallest wire correction that makes that
// requirement representable without changing any field offset or width.
const QUALITY_ENCODER_BOUND_TRUNCATED: u8 = 1 << 5;
const QUALITY_KNOWN_MASK: u8 = QUALITY_ATTEMPTS_UNREPRESENTED
    | QUALITY_CENSUS_OVERFLOWED_BEFORE_PHASE
    | QUALITY_CENSUS_DROPPED_DURING_PHASE
    | QUALITY_PROCESS_THREAD_ATTEMPTS_DIFFER
    | QUALITY_COUNTER_RECONCILIATION
    | QUALITY_ENCODER_BOUND_TRUNCATED;

const ROW_FLAG_REFUSED: u8 = 1 << 0;
const ROW_FLAG_KNOWN_MASK: u8 = ROW_FLAG_REFUSED;
const REFUSED_ENTRY_POINT_SENTINEL: u8 = 0xff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FreezeCensusPhase {
    Install = 1,
    Release = 2,
}

impl FreezeCensusPhase {
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Release => "release",
        }
    }
}

impl TryFrom<u8> for FreezeCensusPhase {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Install),
            2 => Ok(Self::Release),
            _ => Err(format!("H4FC phase {value} is unassigned")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FreezeCensusEntryPoint {
    Direct = 0,
    Enable = 1,
    ApplyQueued = 2,
    EnableBatch = 3,
    Disable = 4,
    DisableBatch = 5,
    Remove = 6,
    RemoveBatch = 7,
}

impl FreezeCensusEntryPoint {
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Direct => "freeze_process",
            Self::Enable => "enable",
            Self::ApplyQueued => "apply_queued",
            Self::EnableBatch => "enable_batch",
            Self::Disable => "disable",
            Self::DisableBatch => "disable_batch",
            Self::Remove => "remove",
            Self::RemoveBatch => "remove_batch",
        }
    }
}

impl TryFrom<u8> for FreezeCensusEntryPoint {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Direct),
            1 => Ok(Self::Enable),
            2 => Ok(Self::ApplyQueued),
            3 => Ok(Self::EnableBatch),
            4 => Ok(Self::Disable),
            5 => Ok(Self::DisableBatch),
            6 => Ok(Self::Remove),
            7 => Ok(Self::RemoveBatch),
            _ => Err(format!("H4FC entry-point code {value} is unassigned")),
        }
    }
}

/// One atomic-counter snapshot taken at a phase boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FreezeCensusCounterSnapshot {
    pub process_attempts: u64,
    pub producer_thread_attempts: u64,
    pub process_refusals: u64,
    pub census_recorded: u64,
    pub census_dropped: u64,
}

/// Borrowed producer-side row. The encoder copies its label into the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreezeCensusRowRef<'a> {
    pub ordinal: u64,
    pub refused: bool,
    pub entry_point: Option<FreezeCensusEntryPoint>,
    pub label: &'a str,
    pub target_count: u32,
    pub window_us: u32,
    pub acquire_us: u32,
    pub minhook_us: u32,
    pub passes: u32,
}

/// Owned consumer-side row. Its label is carried by the sealed payload rather
/// than borrowed from the producer's process-static roster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreezeCensusRow {
    pub ordinal: u64,
    pub refused: bool,
    pub entry_point: Option<FreezeCensusEntryPoint>,
    pub label: String,
    pub target_count: u32,
    pub window_us: u32,
    pub acquire_us: u32,
    pub minhook_us: u32,
    pub passes: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FreezeCensusQuality(u8);

impl fmt::Debug for FreezeCensusQuality {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FreezeCensusQuality")
            .field("bits", &format_args!("{:#04x}", self.0))
            .field("attempts_unrepresented", &self.attempts_unrepresented())
            .field(
                "census_overflowed_before_phase",
                &self.census_overflowed_before_phase(),
            )
            .field(
                "census_dropped_during_phase",
                &self.census_dropped_during_phase(),
            )
            .field(
                "process_thread_attempts_differ",
                &self.process_thread_attempts_differ(),
            )
            .field(
                "counter_reconciliation_issue",
                &self.counter_reconciliation_issue(),
            )
            .field("encoder_bound_truncated", &self.encoder_bound_truncated())
            .finish()
    }
}

impl FreezeCensusQuality {
    const fn from_validated_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn attempts_unrepresented(self) -> bool {
        self.0 & QUALITY_ATTEMPTS_UNREPRESENTED != 0
    }

    pub const fn census_overflowed_before_phase(self) -> bool {
        self.0 & QUALITY_CENSUS_OVERFLOWED_BEFORE_PHASE != 0
    }

    pub const fn census_dropped_during_phase(self) -> bool {
        self.0 & QUALITY_CENSUS_DROPPED_DURING_PHASE != 0
    }

    pub const fn process_thread_attempts_differ(self) -> bool {
        self.0 & QUALITY_PROCESS_THREAD_ATTEMPTS_DIFFER != 0
    }

    pub const fn counter_reconciliation_issue(self) -> bool {
        self.0 & QUALITY_COUNTER_RECONCILIATION != 0
    }

    pub const fn encoder_bound_truncated(self) -> bool {
        self.0 & QUALITY_ENCODER_BOUND_TRUNCATED != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreezeCensusRecordV1 {
    phase: FreezeCensusPhase,
    quality: FreezeCensusQuality,
    attempt_ordinal_start_exclusive: u64,
    attempt_ordinal_end_inclusive: u64,
    process_attempts_total: u64,
    producer_thread_attempts_total: u64,
    process_refusals_before: u64,
    process_refusals_after: u64,
    process_refusals_total: u64,
    census_recorded_before: u64,
    census_recorded_after: u64,
    census_dropped_before: u64,
    census_dropped_after: u64,
    census_dropped_delta: u64,
    attempts_unrepresented_total: u64,
    present_window_us_sum: u64,
    present_acquire_us_sum: u64,
    present_minhook_us_sum: u64,
    present_refused_row_count: u64,
    rows: Vec<FreezeCensusRow>,
}

impl FreezeCensusRecordV1 {
    /// Total, bounded encoder. It rejects contradictory snapshots or rows, but
    /// seals a named prefix when the row/label storage bound is reached.
    pub fn encode_bounded(
        phase: FreezeCensusPhase,
        before: FreezeCensusCounterSnapshot,
        after: FreezeCensusCounterSnapshot,
        selected_rows: &[FreezeCensusRowRef<'_>],
    ) -> Result<Vec<u8>, String> {
        let process_attempts_total = checked_delta(
            "process attempts",
            before.process_attempts,
            after.process_attempts,
        )?;
        let producer_thread_attempts_total = checked_delta(
            "producer-thread attempts",
            before.producer_thread_attempts,
            after.producer_thread_attempts,
        )?;
        if producer_thread_attempts_total > process_attempts_total {
            return Err(format!(
                "H4FC producer-thread attempts {producer_thread_attempts_total} exceed process attempts {process_attempts_total}"
            ));
        }
        let process_refusals_total = checked_delta(
            "process refusals",
            before.process_refusals,
            after.process_refusals,
        )?;
        let census_dropped_delta = checked_delta(
            "census dropped",
            before.census_dropped,
            after.census_dropped,
        )?;

        let selected_count = u64::try_from(selected_rows.len())
            .map_err(|_| "H4FC selected row count does not fit u64".to_string())?;
        if selected_count > process_attempts_total {
            return Err(format!(
                "H4FC selected rows {selected_count} exceed process attempts {process_attempts_total}"
            ));
        }

        let mut rows_to_keep = 0usize;
        let mut label_blob_len = 0usize;
        let mut encoder_bound_truncated = false;
        for row in selected_rows {
            if rows_to_keep == FREEZE_CENSUS_MAX_ROWS
                || row.label.len() > FREEZE_CENSUS_MAX_LABEL_LEN
                || label_blob_len
                    .checked_add(row.label.len())
                    .is_none_or(|len| len > FREEZE_CENSUS_MAX_LABEL_BLOB_LEN)
            {
                encoder_bound_truncated = true;
                break;
            }
            label_blob_len += row.label.len();
            rows_to_keep += 1;
        }
        if rows_to_keep < selected_rows.len() {
            encoder_bound_truncated = true;
        }
        let retained_rows = &selected_rows[..rows_to_keep];
        // A row beyond the retained prefix is outside the payload by the named
        // bound and cannot make partial telemetry fatal. Contradictions inside
        // the retained prefix still reject.
        validate_selected_rows(
            before.process_attempts,
            after.process_attempts,
            retained_rows,
        )?;
        let rows_present = u64::try_from(rows_to_keep)
            .map_err(|_| "H4FC retained row count does not fit u64".to_string())?;
        let attempts_unrepresented_total = process_attempts_total
            .checked_sub(rows_present)
            .ok_or_else(|| "H4FC retained rows exceed process attempts".to_string())?;

        let (present_window_us_sum, present_acquire_us_sum, present_minhook_us_sum) = retained_rows
            .iter()
            .try_fold((0u64, 0u64, 0u64), |(window, acquire, minhook), row| {
                Ok::<_, String>((
                    window
                        .checked_add(u64::from(row.window_us))
                        .ok_or_else(|| "H4FC present window sum overflows".to_string())?,
                    acquire
                        .checked_add(u64::from(row.acquire_us))
                        .ok_or_else(|| "H4FC present acquire sum overflows".to_string())?,
                    minhook
                        .checked_add(u64::from(row.minhook_us))
                        .ok_or_else(|| "H4FC present MinHook sum overflows".to_string())?,
                ))
            })?;
        let present_refused_row_count =
            u64::try_from(retained_rows.iter().filter(|row| row.refused).count())
                .map_err(|_| "H4FC refused row count does not fit u64".to_string())?;

        let quality = canonical_quality_flags(
            process_attempts_total,
            producer_thread_attempts_total,
            before.census_recorded,
            after.census_recorded,
            before.census_dropped,
            census_dropped_delta,
            rows_present,
            attempts_unrepresented_total,
            encoder_bound_truncated,
        )?;

        let payload_len = FREEZE_CENSUS_HEADER_LEN
            .checked_add(
                rows_to_keep
                    .checked_mul(FREEZE_CENSUS_ROW_LEN)
                    .ok_or_else(|| "H4FC row-array length overflows".to_string())?,
            )
            .and_then(|len| len.checked_add(label_blob_len))
            .ok_or_else(|| "H4FC payload length overflows".to_string())?;
        if payload_len > FREEZE_CENSUS_MAX_PAYLOAD_LEN {
            return Err(format!(
                "H4FC payload length {payload_len} exceeds {FREEZE_CENSUS_MAX_PAYLOAD_LEN}"
            ));
        }

        let rows_present_u32 = u32::try_from(rows_to_keep)
            .map_err(|_| "H4FC retained row count does not fit u32".to_string())?;
        let label_blob_len_u32 = u32::try_from(label_blob_len)
            .map_err(|_| "H4FC label blob length does not fit u32".to_string())?;
        let payload_len_u32 = u32::try_from(payload_len)
            .map_err(|_| "H4FC payload length does not fit u32".to_string())?;

        let mut out = vec![0u8; payload_len];
        out[0..4].copy_from_slice(FREEZE_CENSUS_MAGIC);
        put_u16(&mut out, 4, FREEZE_CENSUS_VERSION);
        put_u16(&mut out, 6, FREEZE_CENSUS_HEADER_LEN as u16);
        out[8] = phase as u8;
        out[9] = quality;
        put_u16(&mut out, 10, FREEZE_CENSUS_ROW_LEN as u16);
        put_u32(&mut out, 12, payload_len_u32);
        put_u32(&mut out, 16, rows_present_u32);
        put_u32(&mut out, 20, label_blob_len_u32);
        put_u64(&mut out, 24, before.process_attempts);
        put_u64(&mut out, 32, after.process_attempts);
        put_u64(&mut out, 40, process_attempts_total);
        put_u64(&mut out, 48, producer_thread_attempts_total);
        put_u64(&mut out, 56, before.process_refusals);
        put_u64(&mut out, 64, after.process_refusals);
        put_u64(&mut out, 72, process_refusals_total);
        put_u64(&mut out, 80, before.census_recorded);
        put_u64(&mut out, 88, after.census_recorded);
        put_u64(&mut out, 96, before.census_dropped);
        put_u64(&mut out, 104, after.census_dropped);
        put_u64(&mut out, 112, census_dropped_delta);
        put_u64(&mut out, 120, attempts_unrepresented_total);
        put_u64(&mut out, 128, present_window_us_sum);
        put_u64(&mut out, 136, present_acquire_us_sum);
        put_u64(&mut out, 144, present_minhook_us_sum);
        put_u64(&mut out, 152, present_refused_row_count);

        let label_blob_start = FREEZE_CENSUS_HEADER_LEN + rows_to_keep * FREEZE_CENSUS_ROW_LEN;
        let mut label_cursor = 0usize;
        for (index, row) in retained_rows.iter().enumerate() {
            let row_start = FREEZE_CENSUS_HEADER_LEN + index * FREEZE_CENSUS_ROW_LEN;
            put_u64(&mut out, row_start, row.ordinal);
            out[row_start + 8] = row
                .entry_point
                .map_or(REFUSED_ENTRY_POINT_SENTINEL, |entry| entry as u8);
            out[row_start + 9] = if row.refused { ROW_FLAG_REFUSED } else { 0 };
            put_u16(
                &mut out,
                row_start + 10,
                u16::try_from(row.label.len())
                    .map_err(|_| "H4FC label length does not fit u16".to_string())?,
            );
            put_u32(
                &mut out,
                row_start + 12,
                u32::try_from(label_cursor)
                    .map_err(|_| "H4FC label offset does not fit u32".to_string())?,
            );
            put_u32(&mut out, row_start + 16, row.target_count);
            put_u32(&mut out, row_start + 20, row.window_us);
            put_u32(&mut out, row_start + 24, row.acquire_us);
            put_u32(&mut out, row_start + 28, row.minhook_us);
            put_u32(&mut out, row_start + 32, row.passes);
            let label_end = label_cursor + row.label.len();
            out[label_blob_start + label_cursor..label_blob_start + label_end]
                .copy_from_slice(row.label.as_bytes());
            label_cursor = label_end;
        }

        // The producer contract is deliberately checked through the same total
        // decoder consumers use. No successful encode can mint rejected bytes.
        Self::decode(&out)
            .map_err(|error| format!("H4FC encoder produced invalid bytes: {error}"))?;
        Ok(out)
    }

    /// Total decoder over arbitrary byte slices.
    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        if payload.len() < FREEZE_CENSUS_HEADER_LEN {
            return Err(format!(
                "H4FC payload has {} byte(s), fewer than header {FREEZE_CENSUS_HEADER_LEN}",
                payload.len()
            ));
        }
        if &payload[0..4] != FREEZE_CENSUS_MAGIC {
            return Err("H4FC magic is invalid".to_string());
        }
        let version = read_u16(payload, 4)?;
        if version != FREEZE_CENSUS_VERSION {
            return Err(format!(
                "H4FC version {version} is not {FREEZE_CENSUS_VERSION}"
            ));
        }
        let header_len = usize::from(read_u16(payload, 6)?);
        if header_len != FREEZE_CENSUS_HEADER_LEN {
            return Err(format!(
                "H4FC header length {header_len} is not {FREEZE_CENSUS_HEADER_LEN}"
            ));
        }
        let phase = FreezeCensusPhase::try_from(payload[8])?;
        let quality_bits = payload[9];
        let unknown_quality = quality_bits & !QUALITY_KNOWN_MASK;
        if unknown_quality != 0 {
            return Err(format!(
                "H4FC quality flags contain unassigned bits {unknown_quality:#04x}"
            ));
        }
        let row_len = usize::from(read_u16(payload, 10)?);
        if row_len != FREEZE_CENSUS_ROW_LEN {
            return Err(format!(
                "H4FC row length {row_len} is not {FREEZE_CENSUS_ROW_LEN}"
            ));
        }
        let declared_len = usize::try_from(read_u32(payload, 12)?)
            .map_err(|_| "H4FC declared length does not fit usize".to_string())?;
        if declared_len > FREEZE_CENSUS_MAX_PAYLOAD_LEN {
            return Err(format!(
                "H4FC declared length {declared_len} exceeds {FREEZE_CENSUS_MAX_PAYLOAD_LEN}"
            ));
        }
        if declared_len != payload.len() {
            return Err(format!(
                "H4FC declared length {declared_len} does not equal payload length {}",
                payload.len()
            ));
        }
        let rows_present = usize::try_from(read_u32(payload, 16)?)
            .map_err(|_| "H4FC rows_present does not fit usize".to_string())?;
        if rows_present > FREEZE_CENSUS_MAX_ROWS {
            return Err(format!(
                "H4FC rows_present {rows_present} exceeds {FREEZE_CENSUS_MAX_ROWS}"
            ));
        }
        let label_blob_len = usize::try_from(read_u32(payload, 20)?)
            .map_err(|_| "H4FC label_blob_len does not fit usize".to_string())?;
        if label_blob_len > FREEZE_CENSUS_MAX_LABEL_BLOB_LEN {
            return Err(format!(
                "H4FC label blob length {label_blob_len} exceeds {FREEZE_CENSUS_MAX_LABEL_BLOB_LEN}"
            ));
        }
        let row_bytes = rows_present
            .checked_mul(row_len)
            .ok_or_else(|| "H4FC row-array length overflows".to_string())?;
        let computed_len = header_len
            .checked_add(row_bytes)
            .and_then(|len| len.checked_add(label_blob_len))
            .ok_or_else(|| "H4FC computed payload length overflows".to_string())?;
        if computed_len != payload.len() {
            return Err(format!(
                "H4FC header/row/blob lengths compute {computed_len}, not payload length {}",
                payload.len()
            ));
        }
        if read_u64(payload, 160)? != 0 || read_u64(payload, 168)? != 0 {
            return Err("H4FC reserved header words are nonzero".to_string());
        }

        let attempt_ordinal_start_exclusive = read_u64(payload, 24)?;
        let attempt_ordinal_end_inclusive = read_u64(payload, 32)?;
        let process_attempts_total = read_u64(payload, 40)?;
        let expected_process_attempts = attempt_ordinal_end_inclusive
            .checked_sub(attempt_ordinal_start_exclusive)
            .ok_or_else(|| "H4FC attempt ordinal interval regresses".to_string())?;
        if process_attempts_total != expected_process_attempts {
            return Err(format!(
                "H4FC process_attempts_total {process_attempts_total} does not equal interval width {expected_process_attempts}"
            ));
        }
        let producer_thread_attempts_total = read_u64(payload, 48)?;
        if producer_thread_attempts_total > process_attempts_total {
            return Err(format!(
                "H4FC producer-thread attempts {producer_thread_attempts_total} exceed process attempts {process_attempts_total}"
            ));
        }
        let process_refusals_before = read_u64(payload, 56)?;
        let process_refusals_after = read_u64(payload, 64)?;
        let process_refusals_total = read_u64(payload, 72)?;
        let expected_refusals = process_refusals_after
            .checked_sub(process_refusals_before)
            .ok_or_else(|| "H4FC process refusal counter regresses".to_string())?;
        if process_refusals_total != expected_refusals {
            return Err(format!(
                "H4FC process_refusals_total {process_refusals_total} does not equal counter delta {expected_refusals}"
            ));
        }
        let census_recorded_before = read_u64(payload, 80)?;
        let census_recorded_after = read_u64(payload, 88)?;
        if census_recorded_after < census_recorded_before {
            return Err("H4FC census recorded cursor regresses".to_string());
        }
        let census_dropped_before = read_u64(payload, 96)?;
        let census_dropped_after = read_u64(payload, 104)?;
        let census_dropped_delta = read_u64(payload, 112)?;
        let expected_dropped = census_dropped_after
            .checked_sub(census_dropped_before)
            .ok_or_else(|| "H4FC census dropped counter regresses".to_string())?;
        if census_dropped_delta != expected_dropped {
            return Err(format!(
                "H4FC census_dropped_delta {census_dropped_delta} does not equal counter delta {expected_dropped}"
            ));
        }
        let rows_present_u64 = u64::try_from(rows_present)
            .map_err(|_| "H4FC rows_present does not fit u64".to_string())?;
        if rows_present_u64 > process_attempts_total {
            return Err(format!(
                "H4FC rows_present {rows_present} exceeds process attempts {process_attempts_total}"
            ));
        }
        let attempts_unrepresented_total = read_u64(payload, 120)?;
        let expected_unrepresented = process_attempts_total - rows_present_u64;
        if attempts_unrepresented_total != expected_unrepresented {
            return Err(format!(
                "H4FC attempts_unrepresented_total {attempts_unrepresented_total} does not equal {expected_unrepresented}"
            ));
        }
        let encoder_bound_truncated = quality_bits & QUALITY_ENCODER_BOUND_TRUNCATED != 0;
        let expected_quality = canonical_quality_flags(
            process_attempts_total,
            producer_thread_attempts_total,
            census_recorded_before,
            census_recorded_after,
            census_dropped_before,
            census_dropped_delta,
            rows_present_u64,
            attempts_unrepresented_total,
            encoder_bound_truncated,
        )?;
        if quality_bits != expected_quality {
            return Err(format!(
                "H4FC quality flags {quality_bits:#04x} do not equal canonical {expected_quality:#04x}"
            ));
        }

        let label_blob_start = header_len + row_bytes;
        let label_blob = &payload[label_blob_start..];
        let mut expected_label_offset = 0usize;
        let mut previous_ordinal = None;
        let mut rows = Vec::with_capacity(rows_present);
        for index in 0..rows_present {
            let row_start = header_len + index * row_len;
            let ordinal = read_u64(payload, row_start)?;
            if ordinal <= attempt_ordinal_start_exclusive || ordinal > attempt_ordinal_end_inclusive
            {
                return Err(format!(
                    "H4FC row {index} ordinal {ordinal} is outside ({attempt_ordinal_start_exclusive}, {attempt_ordinal_end_inclusive}]"
                ));
            }
            if previous_ordinal.is_some_and(|previous| ordinal <= previous) {
                return Err(format!(
                    "H4FC row {index} ordinal {ordinal} is not strictly increasing"
                ));
            }
            previous_ordinal = Some(ordinal);

            let entry_code = payload[row_start + 8];
            let row_flags = payload[row_start + 9];
            let unknown_row_flags = row_flags & !ROW_FLAG_KNOWN_MASK;
            if unknown_row_flags != 0 {
                return Err(format!(
                    "H4FC row {index} flags contain unassigned bits {unknown_row_flags:#04x}"
                ));
            }
            let refused = row_flags & ROW_FLAG_REFUSED != 0;
            let entry_point = if entry_code == REFUSED_ENTRY_POINT_SENTINEL {
                None
            } else {
                Some(FreezeCensusEntryPoint::try_from(entry_code)?)
            };
            if refused != entry_point.is_none() {
                return Err(format!(
                    "H4FC row {index} refused flag and entry-point sentinel disagree"
                ));
            }

            let label_len = usize::from(read_u16(payload, row_start + 10)?);
            if label_len > FREEZE_CENSUS_MAX_LABEL_LEN {
                return Err(format!(
                    "H4FC row {index} label length {label_len} exceeds {FREEZE_CENSUS_MAX_LABEL_LEN}"
                ));
            }
            let label_offset = usize::try_from(read_u32(payload, row_start + 12)?)
                .map_err(|_| format!("H4FC row {index} label offset does not fit usize"))?;
            if label_offset != expected_label_offset {
                return Err(format!(
                    "H4FC row {index} label offset {label_offset} is not canonical next offset {expected_label_offset}"
                ));
            }
            let label_end = label_offset
                .checked_add(label_len)
                .ok_or_else(|| format!("H4FC row {index} label range overflows"))?;
            if label_end > label_blob.len() {
                return Err(format!("H4FC row {index} label range is truncated"));
            }
            let label = std::str::from_utf8(&label_blob[label_offset..label_end])
                .map_err(|_| format!("H4FC row {index} label is not UTF-8"))?
                .to_string();
            expected_label_offset = label_end;

            let target_count = read_u32(payload, row_start + 16)?;
            let window_us = read_u32(payload, row_start + 20)?;
            let acquire_us = read_u32(payload, row_start + 24)?;
            let minhook_us = read_u32(payload, row_start + 28)?;
            let passes = read_u32(payload, row_start + 32)?;
            if read_u32(payload, row_start + 36)? != 0 {
                return Err(format!("H4FC row {index} reserved word is nonzero"));
            }
            if u64::from(acquire_us) + u64::from(minhook_us) > u64::from(window_us) {
                return Err(format!(
                    "H4FC row {index} acquire_us + minhook_us exceeds window_us"
                ));
            }
            if refused && (target_count != 0 || acquire_us != 0 || minhook_us != 0 || passes != 0) {
                return Err(format!(
                    "H4FC refused row {index} carries target/acquire/MinHook/pass values"
                ));
            }
            rows.push(FreezeCensusRow {
                ordinal,
                refused,
                entry_point,
                label,
                target_count,
                window_us,
                acquire_us,
                minhook_us,
                passes,
            });
        }
        if expected_label_offset != label_blob.len() {
            return Err(format!(
                "H4FC label blob has {} trailing byte(s)",
                label_blob.len() - expected_label_offset
            ));
        }

        let present_window_us_sum = read_u64(payload, 128)?;
        let present_acquire_us_sum = read_u64(payload, 136)?;
        let present_minhook_us_sum = read_u64(payload, 144)?;
        let present_refused_row_count = read_u64(payload, 152)?;
        let computed_window_sum = rows
            .iter()
            .try_fold(0u64, |sum, row| sum.checked_add(u64::from(row.window_us)))
            .ok_or_else(|| "H4FC decoded window sum overflows".to_string())?;
        let computed_acquire_sum = rows
            .iter()
            .try_fold(0u64, |sum, row| sum.checked_add(u64::from(row.acquire_us)))
            .ok_or_else(|| "H4FC decoded acquire sum overflows".to_string())?;
        let computed_minhook_sum = rows
            .iter()
            .try_fold(0u64, |sum, row| sum.checked_add(u64::from(row.minhook_us)))
            .ok_or_else(|| "H4FC decoded MinHook sum overflows".to_string())?;
        let computed_refused = u64::try_from(rows.iter().filter(|row| row.refused).count())
            .map_err(|_| "H4FC decoded refused count does not fit u64".to_string())?;
        if present_window_us_sum != computed_window_sum
            || present_acquire_us_sum != computed_acquire_sum
            || present_minhook_us_sum != computed_minhook_sum
            || present_refused_row_count != computed_refused
        {
            return Err("H4FC present-row header totals disagree with decoded rows".to_string());
        }

        Ok(Self {
            phase,
            quality: FreezeCensusQuality::from_validated_bits(quality_bits),
            attempt_ordinal_start_exclusive,
            attempt_ordinal_end_inclusive,
            process_attempts_total,
            producer_thread_attempts_total,
            process_refusals_before,
            process_refusals_after,
            process_refusals_total,
            census_recorded_before,
            census_recorded_after,
            census_dropped_before,
            census_dropped_after,
            census_dropped_delta,
            attempts_unrepresented_total,
            present_window_us_sum,
            present_acquire_us_sum,
            present_minhook_us_sum,
            present_refused_row_count,
            rows,
        })
    }

    pub const fn phase(&self) -> FreezeCensusPhase {
        self.phase
    }

    pub const fn quality(&self) -> FreezeCensusQuality {
        self.quality
    }

    pub const fn attempt_ordinal_start_exclusive(&self) -> u64 {
        self.attempt_ordinal_start_exclusive
    }

    pub const fn attempt_ordinal_end_inclusive(&self) -> u64 {
        self.attempt_ordinal_end_inclusive
    }

    pub const fn process_attempts_total(&self) -> u64 {
        self.process_attempts_total
    }

    pub const fn producer_thread_attempts_total(&self) -> u64 {
        self.producer_thread_attempts_total
    }

    pub const fn process_refusals_before(&self) -> u64 {
        self.process_refusals_before
    }

    pub const fn process_refusals_after(&self) -> u64 {
        self.process_refusals_after
    }

    pub const fn process_refusals_total(&self) -> u64 {
        self.process_refusals_total
    }

    pub const fn census_recorded_before(&self) -> u64 {
        self.census_recorded_before
    }

    pub const fn census_recorded_after(&self) -> u64 {
        self.census_recorded_after
    }

    pub const fn census_dropped_before(&self) -> u64 {
        self.census_dropped_before
    }

    pub const fn census_dropped_after(&self) -> u64 {
        self.census_dropped_after
    }

    pub const fn census_dropped_delta(&self) -> u64 {
        self.census_dropped_delta
    }

    pub const fn attempts_unrepresented_total(&self) -> u64 {
        self.attempts_unrepresented_total
    }

    pub const fn present_window_us_sum(&self) -> u64 {
        self.present_window_us_sum
    }

    pub const fn present_acquire_us_sum(&self) -> u64 {
        self.present_acquire_us_sum
    }

    pub const fn present_minhook_us_sum(&self) -> u64 {
        self.present_minhook_us_sum
    }

    pub const fn present_refused_row_count(&self) -> u64 {
        self.present_refused_row_count
    }

    pub fn rows(&self) -> &[FreezeCensusRow] {
        &self.rows
    }

    /// Report-only condition: missing labels are allowed because a row remains
    /// useful without an identifying roster string. `Direct` has no target
    /// roster, so its canonical empty label is not missing.
    pub fn has_missing_non_refused_labels(&self) -> bool {
        self.rows.iter().any(|row| {
            !row.refused
                && row.entry_point != Some(FreezeCensusEntryPoint::Direct)
                && row.label.is_empty()
        })
    }
}

fn validate_selected_rows(
    start_exclusive: u64,
    end_inclusive: u64,
    rows: &[FreezeCensusRowRef<'_>],
) -> Result<(), String> {
    let mut prior = None;
    for (index, row) in rows.iter().enumerate() {
        if row.ordinal <= start_exclusive || row.ordinal > end_inclusive {
            return Err(format!(
                "H4FC input row {index} ordinal {} is outside ({start_exclusive}, {end_inclusive}]",
                row.ordinal
            ));
        }
        if prior.is_some_and(|ordinal| row.ordinal <= ordinal) {
            return Err(format!(
                "H4FC input row {index} ordinal {} is not strictly increasing",
                row.ordinal
            ));
        }
        prior = Some(row.ordinal);
        if row.refused != row.entry_point.is_none() {
            return Err(format!(
                "H4FC input row {index} refused flag and entry point disagree"
            ));
        }
        if u64::from(row.acquire_us) + u64::from(row.minhook_us) > u64::from(row.window_us) {
            return Err(format!(
                "H4FC input row {index} acquire_us + minhook_us exceeds window_us"
            ));
        }
        if row.refused
            && (row.target_count != 0
                || row.acquire_us != 0
                || row.minhook_us != 0
                || row.passes != 0)
        {
            return Err(format!(
                "H4FC input refused row {index} carries target/acquire/MinHook/pass values"
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn canonical_quality_flags(
    process_attempts_total: u64,
    producer_thread_attempts_total: u64,
    census_recorded_before: u64,
    census_recorded_after: u64,
    census_dropped_before: u64,
    census_dropped_delta: u64,
    rows_present: u64,
    attempts_unrepresented_total: u64,
    encoder_bound_truncated: bool,
) -> Result<u8, String> {
    if encoder_bound_truncated && attempts_unrepresented_total == 0 {
        return Err("H4FC encoder-bound truncation flag has no unrepresented attempt".to_string());
    }
    let census_recorded_delta = census_recorded_after
        .checked_sub(census_recorded_before)
        .ok_or_else(|| "H4FC census recorded cursor regresses".to_string())?;
    let census_dropped_after = census_dropped_before
        .checked_add(census_dropped_delta)
        .ok_or_else(|| "H4FC census dropped counter overflows".to_string())?;
    let mut quality = 0u8;
    if attempts_unrepresented_total != 0 {
        quality |= QUALITY_ATTEMPTS_UNREPRESENTED;
    }
    if census_dropped_before != 0 {
        quality |= QUALITY_CENSUS_OVERFLOWED_BEFORE_PHASE;
    }
    if census_dropped_delta != 0 {
        quality |= QUALITY_CENSUS_DROPPED_DURING_PHASE;
    }
    if process_attempts_total != producer_thread_attempts_total {
        quality |= QUALITY_PROCESS_THREAD_ATTEMPTS_DIFFER;
    }
    if encoder_bound_truncated
        || census_dropped_before > census_recorded_before
        || census_dropped_after > census_recorded_after
        || rows_present.checked_add(census_dropped_delta) != Some(census_recorded_delta)
    {
        quality |= QUALITY_COUNTER_RECONCILIATION;
    }
    if encoder_bound_truncated {
        quality |= QUALITY_ENCODER_BOUND_TRUNCATED;
    }
    Ok(quality)
}

fn checked_delta(name: &str, before: u64, after: u64) -> Result<u64, String> {
    after
        .checked_sub(before)
        .ok_or_else(|| format!("H4FC {name} regresses from {before} to {after}"))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let slice = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or_else(|| format!("H4FC u16 at offset {offset} is truncated"))?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or_else(|| format!("H4FC u32 at offset {offset} is truncated"))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let slice = bytes
        .get(offset..offset.saturating_add(8))
        .ok_or_else(|| format!("H4FC u64 at offset {offset} is truncated"))?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshots_for(row_count: u64) -> (FreezeCensusCounterSnapshot, FreezeCensusCounterSnapshot) {
        (
            FreezeCensusCounterSnapshot::default(),
            FreezeCensusCounterSnapshot {
                process_attempts: row_count,
                producer_thread_attempts: row_count,
                process_refusals: 0,
                census_recorded: row_count,
                census_dropped: 0,
            },
        )
    }

    fn row<'a>(
        ordinal: u64,
        entry_point: FreezeCensusEntryPoint,
        label: &'a str,
    ) -> FreezeCensusRowRef<'a> {
        FreezeCensusRowRef {
            ordinal,
            refused: false,
            entry_point: Some(entry_point),
            label,
            target_count: 1,
            window_us: 100,
            acquire_us: 20,
            minhook_us: 70,
            passes: 2,
        }
    }

    fn refused_row(ordinal: u64) -> FreezeCensusRowRef<'static> {
        FreezeCensusRowRef {
            ordinal,
            refused: true,
            entry_point: None,
            label: "",
            target_count: 0,
            window_us: 50,
            acquire_us: 0,
            minhook_us: 0,
            passes: 0,
        }
    }

    fn encode_rows(rows: &[FreezeCensusRowRef<'_>]) -> Vec<u8> {
        let (before, after) = snapshots_for(rows.len() as u64);
        FreezeCensusRecordV1::encode_bounded(FreezeCensusPhase::Install, before, after, rows)
            .unwrap()
    }

    fn put_test_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_test_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    /// kills: any H4FC offset, width, byte order, magic, version, row packing,
    /// or label-blob order changing without an intentional version change. The
    /// explicit array is independent of source-file line endings.
    #[test]
    fn exact_byte_golden_pins_header_rows_and_label_blob() {
        let before = FreezeCensusCounterSnapshot {
            process_attempts: 10,
            producer_thread_attempts: 20,
            process_refusals: 1,
            census_recorded: 5,
            census_dropped: 0,
        };
        let after = FreezeCensusCounterSnapshot {
            process_attempts: 12,
            producer_thread_attempts: 22,
            process_refusals: 2,
            census_recorded: 7,
            census_dropped: 0,
        };
        let rows = [
            row(11, FreezeCensusEntryPoint::Enable, "A"),
            refused_row(12),
        ];
        let expected: [u8; 257] = [
            0x48, 0x34, 0x46, 0x43, 0x01, 0x00, 0xb0, 0x00, 0x01, 0x00, 0x28, 0x00, 0x01, 0x01,
            0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x96, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x46, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x64, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x46, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xff, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x32, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x41,
        ];
        assert_eq!(
            FreezeCensusRecordV1::encode_bounded(FreezeCensusPhase::Install, before, after, &rows,)
                .unwrap(),
            expected
        );
    }

    /// kills: reducing either advertised maximum, overflowing payload-length
    /// arithmetic, or accepting fewer than 256 rows / 16,384 label bytes at
    /// the exact bound.
    #[test]
    fn maximum_size_payload_round_trips_at_exact_bounds() {
        let labels = vec!["x".repeat(64); FREEZE_CENSUS_MAX_ROWS];
        let rows: Vec<_> = labels
            .iter()
            .enumerate()
            .map(|(index, label)| row(index as u64 + 1, FreezeCensusEntryPoint::EnableBatch, label))
            .collect();
        let payload = encode_rows(&rows);
        assert_eq!(payload.len(), FREEZE_CENSUS_MAX_PAYLOAD_LEN);
        let decoded = FreezeCensusRecordV1::decode(&payload).unwrap();
        assert_eq!(decoded.rows().len(), FREEZE_CENSUS_MAX_ROWS);
        assert!(!decoded.quality().encoder_bound_truncated());
        assert_eq!(
            decoded
                .rows()
                .iter()
                .map(|row| row.label.len())
                .sum::<usize>(),
            FREEZE_CENSUS_MAX_LABEL_BLOB_LEN
        );
    }

    /// kills: silently dropping bound overflow without a wire-visible reason,
    /// rejecting honest partial telemetry, or retaining more than the bounded
    /// row/label prefix. These are boundary examples, not byte mutation loops.
    #[test]
    fn encoder_bounds_cut_a_prefix_and_name_the_truncation() {
        let rows: Vec<_> = (1..=257)
            .map(|ordinal| row(ordinal, FreezeCensusEntryPoint::Enable, ""))
            .collect();
        let decoded = FreezeCensusRecordV1::decode(&encode_rows(&rows)).unwrap();
        assert_eq!(decoded.rows().len(), FREEZE_CENSUS_MAX_ROWS);
        assert_eq!(decoded.attempts_unrepresented_total(), 1);
        assert!(decoded.quality().attempts_unrepresented());
        assert!(decoded.quality().encoder_bound_truncated());

        let mut malformed_tail = rows.clone();
        malformed_tail[FREEZE_CENSUS_MAX_ROWS].entry_point = None;
        let decoded = FreezeCensusRecordV1::decode(&encode_rows(&malformed_tail)).unwrap();
        assert_eq!(decoded.rows().len(), FREEZE_CENSUS_MAX_ROWS);
        assert!(decoded.quality().encoder_bound_truncated());

        let labels = vec!["y".repeat(FREEZE_CENSUS_MAX_LABEL_LEN); 17];
        let rows: Vec<_> = labels
            .iter()
            .enumerate()
            .map(|(index, label)| row(index as u64 + 1, FreezeCensusEntryPoint::Enable, label))
            .collect();
        let decoded = FreezeCensusRecordV1::decode(&encode_rows(&rows)).unwrap();
        assert_eq!(decoded.rows().len(), 16);
        assert_eq!(decoded.attempts_unrepresented_total(), 1);
        assert!(decoded.quality().encoder_bound_truncated());

        let overlong = "z".repeat(FREEZE_CENSUS_MAX_LABEL_LEN + 1);
        let rows = [row(1, FreezeCensusEntryPoint::Enable, &overlong)];
        let decoded = FreezeCensusRecordV1::decode(&encode_rows(&rows)).unwrap();
        assert!(decoded.rows().is_empty());
        assert_eq!(decoded.attempts_unrepresented_total(), 1);
        assert!(decoded.quality().encoder_bound_truncated());
    }

    /// kills: changing any phase or entry-point discriminant and thereby
    /// re-attributing install/release work or MinHook entry points.
    #[test]
    fn every_phase_and_entry_point_round_trips() {
        let entries = [
            FreezeCensusEntryPoint::Direct,
            FreezeCensusEntryPoint::Enable,
            FreezeCensusEntryPoint::ApplyQueued,
            FreezeCensusEntryPoint::EnableBatch,
            FreezeCensusEntryPoint::Disable,
            FreezeCensusEntryPoint::DisableBatch,
            FreezeCensusEntryPoint::Remove,
            FreezeCensusEntryPoint::RemoveBatch,
        ];
        for phase in [FreezeCensusPhase::Install, FreezeCensusPhase::Release] {
            for entry in entries {
                let rows = [row(1, entry, entry.canonical_name())];
                let (before, after) = snapshots_for(1);
                let decoded = FreezeCensusRecordV1::decode(
                    &FreezeCensusRecordV1::encode_bounded(phase, before, after, &rows).unwrap(),
                )
                .unwrap();
                assert_eq!(decoded.phase(), phase);
                assert_eq!(decoded.rows()[0].entry_point, Some(entry));
            }
        }
    }

    /// kills: accepting a phase, quality bit, row bit, or entry-point code that
    /// has no assigned meaning. Each mutation targets a complete one-byte
    /// vocabulary; changing unrelated bytes cannot evade the closed masks.
    #[test]
    fn unassigned_discriminants_and_bits_reject() {
        let base = encode_rows(&[row(1, FreezeCensusEntryPoint::Enable, "x")]);
        let mut phase = base.clone();
        phase[8] = 3;
        assert!(FreezeCensusRecordV1::decode(&phase).is_err());
        let mut quality = base.clone();
        quality[9] |= 1 << 6;
        assert!(FreezeCensusRecordV1::decode(&quality).is_err());
        let mut row_flags = base.clone();
        row_flags[FREEZE_CENSUS_HEADER_LEN + 9] |= 1 << 1;
        assert!(FreezeCensusRecordV1::decode(&row_flags).is_err());
        let mut entry = base;
        entry[FREEZE_CENSUS_HEADER_LEN + 8] = 8;
        assert!(FreezeCensusRecordV1::decode(&entry).is_err());
    }

    /// kills: trusting assigned quality claims that contradict the counters or
    /// retained rows. Each bit is derived by one closed arithmetic predicate,
    /// so representative false-positive and false-negative claims prove the
    /// rule without mutating unrelated payload bytes.
    #[test]
    fn assigned_quality_flags_must_match_their_wire_facts() {
        let complete = encode_rows(&[]);
        let mut false_unrepresented = complete.clone();
        false_unrepresented[9] |= QUALITY_ATTEMPTS_UNREPRESENTED;
        assert!(FreezeCensusRecordV1::decode(&false_unrepresented).is_err());
        let mut false_prior_overflow = complete.clone();
        false_prior_overflow[9] |= QUALITY_CENSUS_OVERFLOWED_BEFORE_PHASE;
        assert!(FreezeCensusRecordV1::decode(&false_prior_overflow).is_err());
        let mut false_phase_drop = complete.clone();
        false_phase_drop[9] |= QUALITY_CENSUS_DROPPED_DURING_PHASE;
        assert!(FreezeCensusRecordV1::decode(&false_phase_drop).is_err());
        let mut false_thread_difference = complete.clone();
        false_thread_difference[9] |= QUALITY_PROCESS_THREAD_ATTEMPTS_DIFFER;
        assert!(FreezeCensusRecordV1::decode(&false_thread_difference).is_err());
        let mut false_reconciliation = complete.clone();
        false_reconciliation[9] |= QUALITY_COUNTER_RECONCILIATION;
        assert!(FreezeCensusRecordV1::decode(&false_reconciliation).is_err());
        let mut false_bound = complete;
        false_bound[9] |= QUALITY_ENCODER_BOUND_TRUNCATED;
        assert!(FreezeCensusRecordV1::decode(&false_bound).is_err());

        let before = FreezeCensusCounterSnapshot {
            process_attempts: 10,
            producer_thread_attempts: 20,
            process_refusals: 0,
            census_recorded: 258,
            census_dropped: 2,
        };
        let after = FreezeCensusCounterSnapshot {
            process_attempts: 12,
            producer_thread_attempts: 21,
            process_refusals: 0,
            census_recorded: 260,
            census_dropped: 3,
        };
        let partial = FreezeCensusRecordV1::encode_bounded(
            FreezeCensusPhase::Install,
            before,
            after,
            &[row(11, FreezeCensusEntryPoint::Enable, "")],
        )
        .unwrap();
        for flag in [
            QUALITY_ATTEMPTS_UNREPRESENTED,
            QUALITY_CENSUS_OVERFLOWED_BEFORE_PHASE,
            QUALITY_CENSUS_DROPPED_DURING_PHASE,
            QUALITY_PROCESS_THREAD_ATTEMPTS_DIFFER,
        ] {
            let mut missing = partial.clone();
            missing[9] &= !flag;
            assert!(FreezeCensusRecordV1::decode(&missing).is_err());
        }

        let rows: Vec<_> = (1..=257)
            .map(|ordinal| row(ordinal, FreezeCensusEntryPoint::Enable, ""))
            .collect();
        let bounded = encode_rows(&rows);
        let mut missing_reconciliation = bounded.clone();
        missing_reconciliation[9] &= !QUALITY_COUNTER_RECONCILIATION;
        assert!(FreezeCensusRecordV1::decode(&missing_reconciliation).is_err());
        let mut dishonest_complete = bounded;
        dishonest_complete[9] &= !QUALITY_ATTEMPTS_UNREPRESENTED;
        assert!(FreezeCensusRecordV1::decode(&dishonest_complete).is_err());
    }

    /// kills: spending reserved header or row words without a versioned
    /// grammar. Both reserved regions are checked directly, so no byte sweep is
    /// needed to prove the rule.
    #[test]
    fn reserved_words_reject() {
        let base = encode_rows(&[row(1, FreezeCensusEntryPoint::Enable, "x")]);
        let mut header_reserved = base.clone();
        put_test_u64(&mut header_reserved, 160, 1);
        assert!(FreezeCensusRecordV1::decode(&header_reserved).is_err());
        let mut second_header_reserved = base.clone();
        put_test_u64(&mut second_header_reserved, 168, 1);
        assert!(FreezeCensusRecordV1::decode(&second_header_reserved).is_err());
        let mut row_reserved = base;
        put_test_u32(&mut row_reserved, FREEZE_CENSUS_HEADER_LEN + 36, 1);
        assert!(FreezeCensusRecordV1::decode(&row_reserved).is_err());
    }

    /// kills: trusting `rows_present` or `label_blob_len` without proving that
    /// header + fixed rows + blob equals the exact supplied payload.
    #[test]
    fn row_count_and_blob_length_must_describe_the_exact_payload() {
        let base = encode_rows(&[
            row(1, FreezeCensusEntryPoint::Enable, "ab"),
            row(2, FreezeCensusEntryPoint::Enable, "cd"),
        ]);
        let mut wrong_rows = base.clone();
        put_test_u32(&mut wrong_rows, 16, 1);
        assert!(FreezeCensusRecordV1::decode(&wrong_rows).is_err());
        let mut wrong_blob = base;
        put_test_u32(&mut wrong_blob, 20, 3);
        assert!(FreezeCensusRecordV1::decode(&wrong_blob).is_err());
    }

    /// kills: ambiguous label ownership through invalid UTF-8, overlaps,
    /// aliases, gaps, or unclaimed trailing bytes. Requiring every offset to
    /// equal the prior label end proves canonical packing analytically.
    #[test]
    fn labels_are_utf8_and_canonically_partition_the_blob() {
        let base = encode_rows(&[
            row(1, FreezeCensusEntryPoint::Enable, "ab"),
            row(2, FreezeCensusEntryPoint::Enable, "cd"),
        ]);
        let blob_start = FREEZE_CENSUS_HEADER_LEN + 2 * FREEZE_CENSUS_ROW_LEN;
        let second_offset = FREEZE_CENSUS_HEADER_LEN + FREEZE_CENSUS_ROW_LEN + 12;

        let mut invalid_utf8 = base.clone();
        invalid_utf8[blob_start] = 0xff;
        assert!(FreezeCensusRecordV1::decode(&invalid_utf8).is_err());
        let mut overlap = base.clone();
        put_test_u32(&mut overlap, second_offset, 1);
        assert!(FreezeCensusRecordV1::decode(&overlap).is_err());
        let mut alias = base.clone();
        put_test_u32(&mut alias, second_offset, 0);
        assert!(FreezeCensusRecordV1::decode(&alias).is_err());
        let mut gap = base.clone();
        put_test_u32(&mut gap, second_offset, 3);
        assert!(FreezeCensusRecordV1::decode(&gap).is_err());

        let mut trailing = base;
        trailing.push(b'x');
        let trailing_len = trailing.len() as u32;
        put_test_u32(&mut trailing, 12, trailing_len);
        put_test_u32(&mut trailing, 20, 5);
        assert!(FreezeCensusRecordV1::decode(&trailing).is_err());
    }

    /// kills: accepting duplicate, reordered, or out-of-phase ordinals. One
    /// strict comparison against the prior row plus the closed interval proves
    /// the property for every row, so no per-byte mutation battery is needed.
    #[test]
    fn ordinals_are_strictly_increasing_and_inside_the_phase_interval() {
        let base = encode_rows(&[
            row(1, FreezeCensusEntryPoint::Enable, ""),
            row(2, FreezeCensusEntryPoint::Enable, ""),
        ]);
        let second_ordinal = FREEZE_CENSUS_HEADER_LEN + FREEZE_CENSUS_ROW_LEN;
        let mut duplicate = base.clone();
        put_test_u64(&mut duplicate, second_ordinal, 1);
        assert!(FreezeCensusRecordV1::decode(&duplicate).is_err());
        let mut below = base.clone();
        put_test_u64(&mut below, FREEZE_CENSUS_HEADER_LEN, 0);
        assert!(FreezeCensusRecordV1::decode(&below).is_err());
        let mut above = base;
        put_test_u64(&mut above, second_ordinal, 3);
        assert!(FreezeCensusRecordV1::decode(&above).is_err());
    }

    /// kills: a header lying about attempt, refusal, cursor, or drop deltas.
    /// Each arithmetic identity has one representative counter mutation; the
    /// decoder applies the same identity to the full-width value.
    #[test]
    fn header_counter_deltas_are_exact_and_monotonic() {
        let base = encode_rows(&[row(1, FreezeCensusEntryPoint::Enable, "")]);
        let mut attempts = base.clone();
        put_test_u64(&mut attempts, 40, 0);
        assert!(FreezeCensusRecordV1::decode(&attempts).is_err());
        let mut thread = base.clone();
        put_test_u64(&mut thread, 48, 2);
        assert!(FreezeCensusRecordV1::decode(&thread).is_err());
        let mut refusals = base.clone();
        put_test_u64(&mut refusals, 72, 1);
        assert!(FreezeCensusRecordV1::decode(&refusals).is_err());
        let mut recorded_regression = base.clone();
        put_test_u64(&mut recorded_regression, 80, 2);
        assert!(FreezeCensusRecordV1::decode(&recorded_regression).is_err());
        let mut dropped_regression = base.clone();
        put_test_u64(&mut dropped_regression, 96, 1);
        assert!(FreezeCensusRecordV1::decode(&dropped_regression).is_err());
        let mut dropped_delta = base.clone();
        put_test_u64(&mut dropped_delta, 112, 1);
        assert!(FreezeCensusRecordV1::decode(&dropped_delta).is_err());
        let mut unrepresented = base;
        put_test_u64(&mut unrepresented, 120, 1);
        assert!(FreezeCensusRecordV1::decode(&unrepresented).is_err());
    }

    /// kills: reporting duration or refusal totals not recomputed from the
    /// decoded rows. The four fields cover every present-row aggregate.
    #[test]
    fn header_present_sums_must_equal_the_rows() {
        let base = encode_rows(&[row(1, FreezeCensusEntryPoint::Enable, ""), refused_row(2)]);
        for offset in [128usize, 136, 144, 152] {
            let mut bad = base.clone();
            let current = read_u64(&bad, offset).unwrap();
            put_test_u64(&mut bad, offset, current + 1);
            assert!(FreezeCensusRecordV1::decode(&bad).is_err());
        }
    }

    /// kills: accepting a row whose acquisition and MinHook shares add to more
    /// than the whole freeze-call upper bound.
    #[test]
    fn row_parts_cannot_exceed_the_window() {
        let mut payload = encode_rows(&[row(1, FreezeCensusEntryPoint::Enable, "")]);
        put_test_u32(&mut payload, FREEZE_CENSUS_HEADER_LEN + 24, 60);
        put_test_u32(&mut payload, FREEZE_CENSUS_HEADER_LEN + 28, 50);
        assert!(FreezeCensusRecordV1::decode(&payload).is_err());
    }

    /// kills: a refused row claiming a real entry point or nonzero target,
    /// acquisition, MinHook, or enumeration work that never completed.
    #[test]
    fn refused_row_invariants_reject_contradictions() {
        let base = encode_rows(&[refused_row(1)]);
        let mut real_entry = base.clone();
        real_entry[FREEZE_CENSUS_HEADER_LEN + 8] = FreezeCensusEntryPoint::Enable as u8;
        assert!(FreezeCensusRecordV1::decode(&real_entry).is_err());
        for offset in [16usize, 24, 28, 32] {
            let mut claimed = base.clone();
            put_test_u32(&mut claimed, FREEZE_CENSUS_HEADER_LEN + offset, 1);
            assert!(FreezeCensusRecordV1::decode(&claimed).is_err());
        }
    }

    /// kills: turning honest incompleteness into either a decoder failure or a
    /// silent success. Every accepted partial condition is asserted through a
    /// consumer-facing accessor rather than inferred from private bytes.
    #[test]
    fn report_only_conditions_decode_and_are_observable() {
        let before = FreezeCensusCounterSnapshot {
            process_attempts: 10,
            producer_thread_attempts: 20,
            process_refusals: 3,
            census_recorded: 258,
            census_dropped: 2,
        };
        let after = FreezeCensusCounterSnapshot {
            process_attempts: 15,
            producer_thread_attempts: 24,
            process_refusals: 4,
            census_recorded: 260,
            census_dropped: 3,
        };
        let rows = [row(11, FreezeCensusEntryPoint::Enable, "")];
        let payload =
            FreezeCensusRecordV1::encode_bounded(FreezeCensusPhase::Install, before, after, &rows)
                .unwrap();
        let decoded = FreezeCensusRecordV1::decode(&payload).unwrap();
        assert_eq!(decoded.attempts_unrepresented_total(), 4);
        assert_eq!(decoded.census_dropped_delta(), 1);
        assert!(decoded.quality().attempts_unrepresented());
        assert!(decoded.quality().census_overflowed_before_phase());
        assert!(decoded.quality().census_dropped_during_phase());
        assert!(decoded.quality().process_thread_attempts_differ());
        assert!(!decoded.quality().counter_reconciliation_issue());
        assert!(decoded.has_missing_non_refused_labels());

        let mut direct_row = row(11, FreezeCensusEntryPoint::Direct, "");
        direct_row.target_count = 0;
        direct_row.minhook_us = 0;
        let direct = [direct_row];
        let direct_payload = FreezeCensusRecordV1::encode_bounded(
            FreezeCensusPhase::Install,
            before,
            after,
            &direct,
        )
        .unwrap();
        assert!(
            !FreezeCensusRecordV1::decode(&direct_payload)
                .unwrap()
                .has_missing_non_refused_labels(),
            "Direct has no target list, so its canonical empty label is not missing"
        );

        let mut inconsistent_after = after;
        inconsistent_after.census_recorded = 264;
        let payload = FreezeCensusRecordV1::encode_bounded(
            FreezeCensusPhase::Install,
            before,
            inconsistent_after,
            &rows,
        )
        .unwrap();
        assert!(FreezeCensusRecordV1::decode(&payload)
            .unwrap()
            .quality()
            .counter_reconciliation_issue());
    }

    /// kills: a decoder panic on adversarial/truncated bytes. The deterministic
    /// generator samples lengths through and beyond the maximum payload; this
    /// is the explicitly requested fuzz-style safety test, not a mutation
    /// battery claiming semantic coverage.
    #[test]
    fn arbitrary_payload_slices_never_panic() {
        let mut state = 0x58A4_32D1_9EC7_B605u64;
        for case in 0..16_384usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % (FREEZE_CENSUS_MAX_PAYLOAD_LEN + 513);
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            assert!(
                std::panic::catch_unwind(|| FreezeCensusRecordV1::decode(&bytes)).is_ok(),
                "decoder panicked for fuzz case {case}, len {len}"
            );
        }
    }

    /// kills: shipping release acceptance while the policy is false, or making
    /// option (a) impossible to activate with one ruling. Both boolean states
    /// compile and exercise the same pure policy function.
    #[test]
    fn release_epilogue_policy_is_compiled_for_false_and_true() {
        assert!(!RELEASE_FREEZE_CENSUS_EPILOGUE_ENABLED);
        let disabled = validate_freeze_census_placement_policy(
            false,
            FreezeCensusPhase::Release,
            true,
            9,
            8,
            Some(KnownRecordType::FrameEnd),
            false,
        );
        assert!(disabled.is_err());
        assert_eq!(
            validate_freeze_census_placement_policy(
                true,
                FreezeCensusPhase::Release,
                true,
                9,
                8,
                Some(KnownRecordType::FrameEnd),
                false,
            ),
            Ok(FreezeCensusPlacement::ReleaseEpilogue)
        );
        assert!(validate_freeze_census_placement_policy(
            true,
            FreezeCensusPhase::Release,
            true,
            9,
            8,
            Some(KnownRecordType::DomainData),
            false,
        )
        .is_err());
        assert!(validate_freeze_census_placement_policy(
            true,
            FreezeCensusPhase::Release,
            true,
            10,
            9,
            Some(KnownRecordType::FrameEnd),
            true,
        )
        .is_err());
        assert!(validate_freeze_census_placement_policy(
            true,
            FreezeCensusPhase::Release,
            true,
            10,
            8,
            Some(KnownRecordType::FrameEnd),
            false,
        )
        .is_err());
        assert_eq!(
            validate_record_allowed_after_release_epilogue(false),
            Ok(())
        );
        assert!(validate_record_allowed_after_release_epilogue(true).is_err());
        // kills: changing any consumer-facing EOF predicate so disabled policy
        // accepts release, duplicates are accepted, or the world boundary is optional.
        assert!(!release_epilogue_closes_stream_policy(
            false,
            true,
            FreezeCensusPhase::Release,
            false,
        ));
        assert!(release_epilogue_closes_stream_policy(
            true,
            true,
            FreezeCensusPhase::Release,
            false,
        ));
        assert!(!release_epilogue_closes_stream_policy(
            true,
            true,
            FreezeCensusPhase::Release,
            true,
        ));
    }
}
