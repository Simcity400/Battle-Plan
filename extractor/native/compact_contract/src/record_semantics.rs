macro_rules! compact_record_type_roster {
    ($(
        $(#[$attribute:meta])*
        $variant:ident => $constant:ident = $value:literal, $canonical_name:literal;
    )+) => {
        $(
            $(#[$attribute])*
            pub const $constant: u16 = $value;
        )+

        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        pub enum KnownRecordType {
            $(
                $(#[$attribute])*
                $variant = $value,
            )+
        }

        impl KnownRecordType {
            pub const fn code(self) -> u16 {
                self as u16
            }

            pub const fn canonical_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $canonical_name,)+
                }
            }
        }

        impl TryFrom<u16> for KnownRecordType {
            type Error = UnknownRecordType;

            fn try_from(value: u16) -> Result<Self, Self::Error> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(UnknownRecordType(value)),
                }
            }
        }
    };
}

compact_record_type_roster! {
DomainPerf => COMPACT_RECORD_TYPE_DOMAIN_PERF = 1, "domain_perf";
FrameEnd => COMPACT_RECORD_TYPE_FRAME_END = 2, "frame_end";
DomainData => COMPACT_RECORD_TYPE_DOMAIN_DATA = 3, "domain_data";
StreamHeader => COMPACT_RECORD_TYPE_STREAM_HEADER = 4, "stream_header";
DomainGap => COMPACT_RECORD_TYPE_DOMAIN_GAP = 5, "domain_gap";
FrameRefused => COMPACT_RECORD_TYPE_FRAME_REFUSED = 6, "frame_refused";
FrameDegraded => COMPACT_RECORD_TYPE_FRAME_DEGRADED = 7, "frame_degraded";
CaptureTerminated => COMPACT_RECORD_TYPE_CAPTURE_TERMINATED = 8, "capture_terminated";
ClockUnreadableInterval => COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL = 9, "clock_unreadable_interval";
UntrustedClockRecovery => COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY = 10, "untrusted_clock_recovery";
FreezeCensus => COMPACT_RECORD_TYPE_FREEZE_CENSUS = 11, "freeze_census";
ContentIdentityCensus => COMPACT_RECORD_TYPE_CONTENT_IDENTITY_CENSUS = 12, "content_identity_census";
StateTopologyCensus => COMPACT_RECORD_TYPE_STATE_TOPOLOGY_CENSUS = 13, "state_topology_census";
/// A DOMAIN_DATA hour whose payload, minus the domain's declared volatile
/// prefix ([`domain_declared_volatile_prefix_len`]), is byte-identical to the
/// previous PUBLISHED payload for that domain. The payload carries the fresh
/// volatile prefix followed by a SHA-256 digest of the unchanged remainder;
/// the fold reconstructs by overwriting the prefix on its stored baseline and
/// refuses by name unless that digest verifies. The owning hour stays COVERED:
/// this is a data record of a new kind, not a gap.
DomainDataRepeat => COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT = 14, "domain_data_repeat";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownRecordType(pub u16);

impl core::fmt::Display for UnknownRecordType {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "unknown compact record type {}", self.0)
    }
}

pub const COMPACT_DOMAIN_FRAME: u16 = 1000;

pub const COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2: u32 = 1 << 31;
pub const COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER: u32 = 1 << 30;
/// This DOMAIN_DATA payload is a `PSD1` envelope
/// ([`crate::payload_span_delta`]), not the domain's own grammar.
///
/// THE FLAG IS THE SIGNAL, and it is a header bit rather than a magic sniffed
/// out of the payload because a domain's DOMAIN_DATA records are not always one
/// grammar: d53 carries both its own payload and, from
/// `insert_a30_payloads_before_frame_end`, the `ICCP` companion carrier, whose
/// first bytes are its own magic and not the `elapsed_wall_ms` the delta basis
/// is defined against. A consumer that guessed from payload bytes would have to
/// be right about every such grammar forever; a consumer that reads this bit is
/// told.
pub const COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA: u32 = 1 << 29;

pub const COMPACT_DOMAIN_PERF_BASE_LEN: usize = 40;
pub const COMPACT_FRAME_END_PAYLOAD_LEN: usize = 32;
pub const COMPACT_DOMAIN_GAP_PAYLOAD_LEN: usize = 72;
pub const COMPACT_DOMAIN_GAP_MAGIC: &[u8; 4] = b"H4DG";
pub const COMPACT_DOMAIN_GAP_VERSION: u16 = 1;
pub const DOMAIN_GAP_REASON_TORN: u16 = 1;
pub const DOMAIN_GAP_REASON_OVERRUN: u16 = 2;
pub const DOMAIN_GAP_REASON_BACKPRESSURE: u16 = 3;
pub const DOMAIN_GAP_REASON_CLOCK_UNREADABLE: u16 = 4;
pub const DOMAIN_GAP_REASON_WORKER_PANIC: u16 = 5;
pub const DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE: u16 = 6;
pub const DOMAIN_GAP_REASON_READER_SILENT: u16 = 7;
pub const DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH: u16 = 8;
pub const DOMAIN_GAP_PART_STATE: u16 = 0;
pub const DOMAIN_GAP_PART_EVENT: u16 = 1;
pub const DOMAIN_GAP_PART_BOTH: u16 = 2;

pub const DOMAIN_GAP_WITNESS_PROVENANCE_UNSTATED: u16 = 0;
pub const DOMAIN_GAP_WITNESS_PROVENANCE_BRACKET_OBSERVED: u16 = 1;
pub const DOMAIN_GAP_WITNESS_PROVENANCE_UNOBSERVED: u16 = 2;
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC: &[u8; 4] = b"PDU1";
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION: u16 = 1;
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2: u16 = 2;

/// Byte length of the SHA-256 digest a `DOMAIN_DATA_REPEAT` payload carries
/// over the unchanged tail of the domain's prior published payload.
pub const COMPACT_DOMAIN_DATA_REPEAT_DIGEST_LEN: usize = 32;

/// How many consecutive published hours one domain may ride on repeats before
/// it must publish a full record again, so a consumer that lost a record
/// resyncs. Owner-ruled standard cadence (2026-08-23), matching the shipped
/// d100 XSM2 keyframe interval.
pub const COMPACT_DOMAIN_DATA_REPEAT_KEYFRAME_CADENCE_HOURS: u32 = 64;

/// The per-domain DECLARED volatile prefix of a DOMAIN_DATA payload: leading
/// bytes that are recorder telemetry rather than engine state, written fresh
/// into every record and therefore excluded from both halves of the
/// repeat-gate comparison.
///
/// The declaration is CONTRACT, not measurement: each nonzero entry names a
/// grammar whose documented layout opens with those bytes of recorder
/// telemetry, cited below. A wrong or missing entry can only move WHERE the
/// gate fires, never whether reconstruction is exact — the repeat record
/// always carries the fresh prefix bytes, and the digest covers everything
/// after them — so extending this table is a yield decision, not a safety one.
///
/// `elapsed_wall_ms` is a u64 of recorder wall-clock stamped at offset +0 of
/// these grammars (674 write sites in the collector; see PN's
/// `runs/pn_structural/MEASUREMENT.md` §3):
///
/// * wire_format grammars documenting `elapsed_wall_ms` u64 at `+0`:
///   d10 `fuel_live`, d11 `production_live`, d13 `tech_live`, d16
///   `decisions_live`, d17 `diplomacy_live`, d21 `land_combats_live`,
///   d22 `map_control_live`, d23 `map_detail_live`, d24 `weather_live`,
///   d26 `division_equipment_live`, d27 `army_orders_live`, d28
///   `oob_command_live`, d29 `ships_live`, d31 `air_region_activity_live`,
///   d32 `air_wing_loadouts_live`, d34 `air_base_status_live`, d35
///   `air_variant_stats`, d36 `air_variant_reserves`, d38
///   `air_superiority_inputs`, d46 `air_variant_archetypes_live`, d51
///   `scripted_state_flags_variables_live`, d52
///   `occupation_resistance_compliance_live`, d54
///   `army_battleplans_depth_live`, d56 `naval_missions_status_live`, d61
///   `air_loss_stats_live`, d62 `hub_motorization_live`, d63
///   `country_core_scalars_live`, d64 `delayed_events_live`, d65
///   `special_project_programs_live`, d68 `session_header_live`, d39+d40
///   `variant_upgrades` (one shared grammar);
/// * lane/collector encoders writing `elapsed_wall_ms` first: d12
///   `focus_live` (`attach_log.rs::compact_focus_data_payload`), d14
///   `politics_live`, d15 `politics_residuals`, d18 `country_identity`
///   (`compact_country_identity_data_payload`), d20 `divisions_live`
///   (`domain20_compact.rs`), d42 `air_combat_losses_live`
///   (`domain42_compact.rs`), d43 `market_trade_live`
///   (`market_trade_live/src/lib.rs::encode_payload`), d44
///   `supply_logistics_attrition_live`, d53 `diplomacy_intel_operatives_live`
///   snapshot (`diplomacy_intel_operatives_live/domain44_compact.rs::encode_payload` — the largest single
///   repeat-gate win), d60 `capitulation_war_state_live`
///   (`capitulation_war_state_live/src/compact.rs`), d66
///   `occupation_state_extras_live` and d69 `tech_sharing_groups_live`
///   (their authority decoders read the row/group count at `+8`, so `+0..8`
///   is the same telemetry u64).
///
/// Two grammars carry MORE than eight leading telemetry bytes, proven from
/// their own documented layouts:
///
/// * d49 `naval_combat_lifecycle_live` and d50 `naval_combat_members_live`:
///   schema_version u16 at `+0` (constant 2) with padding to `+4`, then
///   `elapsed_wall_ms` u64 at `+4` (wire_format `d49.rs`/`d50.rs` headers) —
///   declared 12 so the whole elapsed field leaves the compared tail;
/// * d67 `country_dynamic_modifiers_live`: `DMI2` magic at `+0`, version at
///   `+4`, `elapsed_wall_ms` u64 at `+8` (wire_format `d67.rs` header table)
///   — declared 16.
pub const fn domain_declared_volatile_prefix_len(domain_id: u16) -> usize {
    match domain_id {
        10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 20 | 21 | 22 | 23 | 24 | 26 | 27 | 28 | 29
        | 31 | 32 | 34 | 35 | 36 | 38 | 39 | 40 | 42 | 43 | 44 | 51 | 52 | 53 | 54 | 56 | 60
        | 61 | 62 | 63 | 64 | 65 | 66 | 68 | 69 => 8,
        49 | 50 => 12,
        67 => 16,
        _ => 0,
    }
}

/// Exact payload length of a `DOMAIN_DATA_REPEAT` record for `domain_id`:
/// the fresh declared volatile prefix plus the digest of the unchanged tail.
pub const fn domain_data_repeat_payload_len(domain_id: u16) -> usize {
    domain_declared_volatile_prefix_len(domain_id) + COMPACT_DOMAIN_DATA_REPEAT_DIGEST_LEN
}

/// Per-domain reconstruction baselines for stream walkers that decode
/// DOMAIN_DATA payloads and must therefore see through DOMAIN_DATA_REPEAT
/// records to the original bytes.
///
/// `observe` mirrors the collector gate and production fold rule exactly:
/// every full record refreshes the domain's slot; a repeat is verified
/// against the slot (exact length, baseline present, SHA-256 over the tail)
/// and resolves to the original payload `prefix ++ stored_tail`, which then
/// becomes the new slot content. Any violation is a named error — never a
/// guess — so callers refuse the stream rather than authenticate unverified
/// bytes.
#[derive(Default)]
pub struct DomainRepeatBaselines {
    slots: std::collections::BTreeMap<u16, Vec<u8>>,
}

impl DomainRepeatBaselines {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one framed record. Returns `Ok(None)` for non-data records and
    /// full DOMAIN_DATA records; `Ok(Some(reconstructed))` for a verified
    /// DOMAIN_DATA_REPEAT (callers then decode `reconstructed` through the
    /// domain grammar); `Err(reason)` for any violation.
    pub fn observe(
        &mut self,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) -> Result<Option<Vec<u8>>, String> {
        if record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT {
            let span = domain_declared_volatile_prefix_len(domain_id);
            let expected = domain_data_repeat_payload_len(domain_id);
            if payload.len() != expected {
                return Err(format!(
                    "DOMAIN_DATA_REPEAT for domain {domain_id} is {} byte(s), not {expected} (declared prefix {span}, digest {})",
                    payload.len(),
                    COMPACT_DOMAIN_DATA_REPEAT_DIGEST_LEN
                ));
            }
            let Some(baseline) = self.slots.get(&domain_id) else {
                return Err(format!(
                    "DOMAIN_DATA_REPEAT for domain {domain_id} has no stored baseline: a repeat may only follow a published full record"
                ));
            };
            if baseline.len() <= span {
                return Err(format!(
                    "DOMAIN_DATA_REPEAT baseline for domain {domain_id} carries no tail beyond its declared prefix ({span})"
                ));
            }
            let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
            <sha2::Sha256 as sha2::Digest>::update(&mut hasher, &baseline[span..]);
            let digest = <sha2::Sha256 as sha2::Digest>::finalize(hasher);
            if digest.as_slice() != &payload[span..] {
                return Err(format!(
                    "DOMAIN_DATA_REPEAT for domain {domain_id} fails its baseline digest; the recorded tail does not match the last ingested payload"
                ));
            }
            let mut reconstructed = payload[..span].to_vec();
            reconstructed.extend_from_slice(&baseline[span..]);
            self.slots.insert(domain_id, reconstructed.clone());
            Ok(Some(reconstructed))
        } else if record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            match self.slots.get_mut(&domain_id) {
                Some(stored) => {
                    stored.clear();
                    stored.extend_from_slice(payload);
                }
                None => {
                    self.slots.insert(domain_id, payload.to_vec());
                }
            }
            Ok(None)
        } else {
            Ok(None)
        }
    }
}

pub const fn compact_record_counts_toward_terminal_outcome(record_type: u16) -> bool {
    matches!(
        record_type,
        COMPACT_RECORD_TYPE_DOMAIN_PERF | COMPACT_RECORD_TYPE_DOMAIN_GAP
    )
}

pub const fn compact_record_counts_toward_perf_cardinality(record_type: u16) -> bool {
    record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF
}

pub const fn compact_record_allowed_flags(domain_id: u16, record_type: u16) -> u32 {
    let mut allowed = 0;
    if record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF {
        allowed |= COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER;
    }
    // A repeat replaces a DATA record one-for-one and carries that record's
    // flag word, so it admits exactly the bits its domain's DATA records
    // admit.
    if matches!(
        record_type,
        COMPACT_RECORD_TYPE_DOMAIN_DATA | COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
    ) && matches!(domain_id, 45 | 67)
    {
        allowed |= COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2;
    }
    if record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
        && crate::payload_span_delta::is_delta_enabled_domain(domain_id)
    {
        allowed |= COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA;
    }
    allowed
}

pub const fn compact_record_rejected_flags(
    domain_id: u16,
    record_type: u16,
    record_flags: u32,
) -> u32 {
    record_flags & !compact_record_allowed_flags(domain_id, record_type)
}

pub fn validate_compact_record_semantics(
    domain_id: u16,
    record_type: u16,
    record_flags: u32,
    payload: &[u8],
) -> Result<(), String> {
    let rejected_flags = compact_record_rejected_flags(domain_id, record_type, record_flags);
    if rejected_flags != 0 {
        return Err(format!(
            "record flags {record_flags:#010x} contain rejected bits {rejected_flags:#010x} for domain {domain_id} record type {record_type}"
        ));
    }

    let known = KnownRecordType::try_from(record_type).map_err(|error| error.to_string())?;
    match known {
        KnownRecordType::DomainPerf => validate_domain_perf(record_flags, payload),
        KnownRecordType::FrameEnd => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "FRAME_END domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            if payload.len() != COMPACT_FRAME_END_PAYLOAD_LEN {
                return Err(format!(
                    "FRAME_END payload length {} is not {COMPACT_FRAME_END_PAYLOAD_LEN}",
                    payload.len()
                ));
            }
            Ok(())
        }
        KnownRecordType::DomainData => Ok(()),
        KnownRecordType::DomainDataRepeat => {
            if domain_id == 0 || domain_id == COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "DOMAIN_DATA_REPEAT envelope affected domain {domain_id} is invalid"
                ));
            }
            // Flag admissibility is already enforced above: a repeat admits
            // exactly the flag bits its domain's DATA records admit, because
            // it replaces that record one-for-one and carries its flag word.
            let expected = domain_data_repeat_payload_len(domain_id);
            if payload.len() != expected {
                return Err(format!(
                    "DOMAIN_DATA_REPEAT payload length {} is not {expected} for domain {domain_id} (declared volatile prefix {}, digest {})",
                    payload.len(),
                    domain_declared_volatile_prefix_len(domain_id),
                    COMPACT_DOMAIN_DATA_REPEAT_DIGEST_LEN
                ));
            }
            Ok(())
        }
        KnownRecordType::StreamHeader => {
            Err("STREAM_HEADER is valid only through the CID1 sequence-zero authority".to_string())
        }
        KnownRecordType::FrameRefused => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "FRAME_REFUSED domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::frame_refusal::validate_frame_refusal_payload(payload)
        }
        KnownRecordType::FrameDegraded => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "FRAME_DEGRADED domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::frame_degraded::validate_frame_degraded_payload(payload)
        }
        KnownRecordType::CaptureTerminated => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "CAPTURE_TERMINATED domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::capture_termination::CaptureTerminationRecordV1::decode(payload).map(|_| ())
        }
        KnownRecordType::ClockUnreadableInterval => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "CLOCK_UNREADABLE_INTERVAL domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::clock_unreadable_interval::validate_clock_unreadable_interval_payload(payload)
        }
        KnownRecordType::UntrustedClockRecovery => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "UNTRUSTED_CLOCK_RECOVERY domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::untrusted_clock_recovery::validate_untrusted_clock_recovery_payload(payload)
        }
        KnownRecordType::FreezeCensus => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "FREEZE_CENSUS domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::freeze_census_record::FreezeCensusRecordV1::decode(payload).map(|_| ())
        }
        KnownRecordType::ContentIdentityCensus => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "CONTENT_IDENTITY_CENSUS domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::content_identity_census::ContentIdentityCensusV1::decode(payload).map(|_| ())
        }
        KnownRecordType::StateTopologyCensus => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "STATE_TOPOLOGY_CENSUS domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::state_topology_census::StateTopologyCensusV1::decode(payload).map(|_| ())
        }
        KnownRecordType::DomainGap => {
            if domain_id == 0 || domain_id == COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "DOMAIN_GAP envelope affected domain {domain_id} is invalid"
                ));
            }
            validate_domain_gap_payload(payload)
        }
    }
}

pub fn validate_domain_gap_payload(payload: &[u8]) -> Result<(), String> {
    if payload.len() != COMPACT_DOMAIN_GAP_PAYLOAD_LEN {
        return Err(format!(
            "DOMAIN_GAP payload length {} is not {COMPACT_DOMAIN_GAP_PAYLOAD_LEN}",
            payload.len()
        ));
    }
    if &payload[..4] != COMPACT_DOMAIN_GAP_MAGIC {
        return Err("DOMAIN_GAP payload magic is invalid".to_string());
    }
    let version = u16::from_le_bytes([payload[4], payload[5]]);
    if version != COMPACT_DOMAIN_GAP_VERSION {
        return Err(format!(
            "DOMAIN_GAP payload version {version} is not {COMPACT_DOMAIN_GAP_VERSION}"
        ));
    }
    let declared_len = usize::from(u16::from_le_bytes([payload[6], payload[7]]));
    if declared_len != COMPACT_DOMAIN_GAP_PAYLOAD_LEN {
        return Err(format!(
            "DOMAIN_GAP declared length {declared_len} is not {COMPACT_DOMAIN_GAP_PAYLOAD_LEN}"
        ));
    }
    let reason = u16::from_le_bytes([payload[8], payload[9]]);
    if !matches!(
        reason,
        DOMAIN_GAP_REASON_TORN
            | DOMAIN_GAP_REASON_OVERRUN
            | DOMAIN_GAP_REASON_BACKPRESSURE
            | DOMAIN_GAP_REASON_CLOCK_UNREADABLE
            | DOMAIN_GAP_REASON_WORKER_PANIC
            | DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE
            | DOMAIN_GAP_REASON_READER_SILENT
            | DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH
    ) {
        return Err(format!("DOMAIN_GAP reason {reason} is unknown"));
    }
    let part = u16::from_le_bytes([payload[10], payload[11]]);
    if !matches!(
        part,
        DOMAIN_GAP_PART_STATE | DOMAIN_GAP_PART_EVENT | DOMAIN_GAP_PART_BOTH
    ) {
        return Err(format!("DOMAIN_GAP part {part} is unknown"));
    }
    let witness_provenance = u16::from_le_bytes([payload[12], payload[13]]);
    if !matches!(
        witness_provenance,
        DOMAIN_GAP_WITNESS_PROVENANCE_UNSTATED
            | DOMAIN_GAP_WITNESS_PROVENANCE_BRACKET_OBSERVED
            | DOMAIN_GAP_WITNESS_PROVENANCE_UNOBSERVED
    ) {
        return Err(format!(
            "DOMAIN_GAP witness provenance {witness_provenance} is unknown"
        ));
    }
    if u16::from_le_bytes([payload[14], payload[15]]) != 0
        || u32::from_le_bytes(payload[28..32].try_into().expect("fixed reserved slice")) != 0
    {
        return Err("DOMAIN_GAP reserved field is nonzero".to_string());
    }
    let target_hour = i32::from_le_bytes(payload[16..20].try_into().expect("fixed target slice"));
    let range_start =
        i32::from_le_bytes(payload[20..24].try_into().expect("fixed range-start slice"));
    let range_end = i32::from_le_bytes(payload[24..28].try_into().expect("fixed range-end slice"));
    for (name, hour) in [
        ("target", target_hour),
        ("range_start", range_start),
        ("range_end", range_end),
    ] {
        if crate::calendar::hour_index_from_raw(hour).is_none() {
            return Err(format!("DOMAIN_GAP {name} hour {hour} is invalid"));
        }
    }
    if range_start > range_end || target_hour < range_start || target_hour > range_end {
        return Err(format!(
            "DOMAIN_GAP target/range witness {target_hour} not within {range_start}..={range_end}"
        ));
    }
    let inclusive_range_width = u64::try_from(
        i64::from(range_end)
            .checked_sub(i64::from(range_start))
            .and_then(|width| width.checked_add(1))
            .ok_or_else(|| "DOMAIN_GAP inclusive range width overflows".to_string())?,
    )
    .map_err(|_| "DOMAIN_GAP inclusive range width is negative".to_string())?;
    let tick_before =
        u64::from_le_bytes(payload[32..40].try_into().expect("fixed tick-before slice"));
    let tick_after =
        u64::from_le_bytes(payload[40..48].try_into().expect("fixed tick-after slice"));
    let queue_capacity = u32::from_le_bytes(
        payload[48..52]
            .try_into()
            .expect("fixed queue-capacity slice"),
    );
    let queue_depth =
        u32::from_le_bytes(payload[52..56].try_into().expect("fixed queue-depth slice"));
    let queue_dropped = u64::from_le_bytes(
        payload[56..64]
            .try_into()
            .expect("fixed queue-dropped slice"),
    );
    let outcome_ordinal = u64::from_le_bytes(
        payload[64..72]
            .try_into()
            .expect("fixed outcome-ordinal slice"),
    );
    if outcome_ordinal == u64::MAX {
        return Err("DOMAIN_GAP outcome ordinal is exhausted".to_string());
    }
    match reason {
        DOMAIN_GAP_REASON_TORN
            if range_start == range_end
                && target_hour == range_start
                && tick_before != tick_after
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        DOMAIN_GAP_REASON_OVERRUN
            if target_hour == range_end
                && range_start < range_end
                && tick_after.checked_sub(tick_before) == Some(inclusive_range_width)
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        DOMAIN_GAP_REASON_BACKPRESSURE
            if target_hour == range_end
                && tick_before == tick_after
                && queue_capacity > 0
                && queue_depth == queue_capacity
                && queue_dropped == inclusive_range_width => {}
        DOMAIN_GAP_REASON_CLOCK_UNREADABLE
            if range_start == range_end
                && target_hour == range_start
                && tick_before == 0
                && tick_after == 0
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        DOMAIN_GAP_REASON_WORKER_PANIC
            if range_start == range_end
                && target_hour == range_start
                && tick_before == tick_after
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE
            if range_start == range_end
                && target_hour == range_start
                && tick_before == tick_after
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        DOMAIN_GAP_REASON_READER_SILENT
            if range_start == range_end
                && target_hour == range_start
                && tick_before == 0
                && tick_after == 0
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH
            if range_start == range_end
                && target_hour == range_start
                && tick_before == tick_after
                && tick_before != 0
                && u64::try_from(target_hour).map(|label| label != tick_before) == Ok(true)
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        _ => {
            return Err(format!(
                "DOMAIN_GAP witnesses do not prove closed reason {reason}"
            ))
        }
    }
    match witness_provenance {
        DOMAIN_GAP_WITNESS_PROVENANCE_BRACKET_OBSERVED if tick_before == 0 || tick_after == 0 => {
            return Err(
                "DOMAIN_GAP claims a bracket-observed witness on the wire's no-witness zero"
                    .to_string(),
            )
        }
        DOMAIN_GAP_WITNESS_PROVENANCE_UNOBSERVED if tick_before != 0 || tick_after != 0 => {
            return Err(
                "DOMAIN_GAP claims no observation while carrying a nonzero tick witness"
                    .to_string(),
            )
        }
        _ => {}
    }
    Ok(())
}

pub fn domain_gap_target_hour_raw(payload: &[u8]) -> Result<i32, String> {
    validate_domain_gap_payload(payload)?;
    Ok(i32::from_le_bytes(
        payload[16..20]
            .try_into()
            .expect("validated DOMAIN_GAP target slice"),
    ))
}

fn validate_domain_perf(record_flags: u32, payload: &[u8]) -> Result<(), String> {
    if payload.len() < COMPACT_DOMAIN_PERF_BASE_LEN {
        return Err(format!(
            "DOMAIN_PERF payload length {} is shorter than {COMPACT_DOMAIN_PERF_BASE_LEN}",
            payload.len()
        ));
    }
    if u32::from_le_bytes(payload[36..40].try_into().expect("fixed PERF base slice")) != 0 {
        return Err("DOMAIN_PERF reserved base word is nonzero".to_string());
    }

    let has_trailer = record_flags & COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER != 0;
    if !has_trailer {
        return if payload.len() == COMPACT_DOMAIN_PERF_BASE_LEN {
            Ok(())
        } else {
            Err(format!(
                "DOMAIN_PERF payload has {} unannounced trailer byte(s)",
                payload.len() - COMPACT_DOMAIN_PERF_BASE_LEN
            ))
        };
    }

    const TRAILER_HEADER_LEN: usize = 8;
    const UNIT_HEADER_LEN: usize = 12;
    let trailer_start = COMPACT_DOMAIN_PERF_BASE_LEN;
    if payload.len() < trailer_start + TRAILER_HEADER_LEN {
        return Err("DOMAIN_PERF diagnostic trailer header is truncated".to_string());
    }
    if &payload[trailer_start..trailer_start + 4] != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC {
        return Err("DOMAIN_PERF diagnostic trailer magic is invalid".to_string());
    }
    let version = u16::from_le_bytes(
        payload[trailer_start + 4..trailer_start + 6]
            .try_into()
            .expect("fixed PERF trailer version slice"),
    );
    if version != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION
        && version != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2
    {
        return Err(format!(
            "DOMAIN_PERF diagnostic trailer version {version} is neither \
             {COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION} nor \
             {COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2}"
        ));
    }
    let unit_count = u16::from_le_bytes(
        payload[trailer_start + 6..trailer_start + 8]
            .try_into()
            .expect("fixed PERF trailer count slice"),
    ) as usize;
    let mut cursor = trailer_start + TRAILER_HEADER_LEN;
    for unit_index in 0..unit_count {
        if payload.len().saturating_sub(cursor) < UNIT_HEADER_LEN {
            return Err(format!(
                "DOMAIN_PERF diagnostic unit {unit_index} header is truncated"
            ));
        }
        let name_len = u16::from_le_bytes(
            payload[cursor..cursor + 2]
                .try_into()
                .expect("fixed PERF unit name-length slice"),
        ) as usize;
        let reserved = u16::from_le_bytes(
            payload[cursor + 2..cursor + 4]
                .try_into()
                .expect("fixed PERF unit reserved slice"),
        );
        if version == COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION {
            if reserved != 0 {
                return Err(format!(
                    "DOMAIN_PERF diagnostic unit {unit_index} reserved field is nonzero"
                ));
            }
        } else {
            let role = (reserved & 0x00FF) as u8;
            let presence = (reserved >> 8) as u8;
            if crate::diagnostic_channel::UnitRole::from_code(role).is_none() {
                return Err(format!(
                    "DOMAIN_PERF diagnostic unit {unit_index} has undefined role code {role}"
                ));
            }
            if crate::diagnostic_channel::UnitPresence::from_code(presence).is_none() {
                return Err(format!(
                    "DOMAIN_PERF diagnostic unit {unit_index} has undefined presence code \
                     {presence}"
                ));
            }
        }
        cursor += UNIT_HEADER_LEN;
        if payload.len().saturating_sub(cursor) < name_len {
            return Err(format!(
                "DOMAIN_PERF diagnostic unit {unit_index} name is truncated"
            ));
        }
        let name = std::str::from_utf8(&payload[cursor..cursor + name_len])
            .map_err(|_| format!("DOMAIN_PERF diagnostic unit {unit_index} name is not UTF-8"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(format!(
                "DOMAIN_PERF diagnostic unit {unit_index} name {name:?} is invalid"
            ));
        }
        cursor += name_len;
    }
    if cursor != payload.len() {
        return Err(format!(
            "DOMAIN_PERF diagnostic trailer has {} trailing byte(s)",
            payload.len() - cursor
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::DATE_EPOCH_CONST;

    fn base_perf() -> Vec<u8> {
        vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN]
    }

    fn perf_with_trailer(name: Option<&[u8]>, reserved: u16) -> Vec<u8> {
        let mut payload = base_perf();
        payload.extend_from_slice(COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes());
        payload.extend_from_slice(&(name.is_some() as u16).to_le_bytes());
        if let Some(name) = name {
            payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
            payload.extend_from_slice(&reserved.to_le_bytes());
            payload.extend_from_slice(&7u64.to_le_bytes());
            payload.extend_from_slice(name);
        }
        payload
    }

    fn perf_result(flags: u32, payload: &[u8]) -> Result<(), String> {
        validate_compact_record_semantics(18, COMPACT_RECORD_TYPE_DOMAIN_PERF, flags, payload)
    }

    fn gap_payload(reason: u16) -> Vec<u8> {
        let mut payload = vec![0u8; COMPACT_DOMAIN_GAP_PAYLOAD_LEN];
        payload[..4].copy_from_slice(COMPACT_DOMAIN_GAP_MAGIC);
        payload[4..6].copy_from_slice(&COMPACT_DOMAIN_GAP_VERSION.to_le_bytes());
        payload[6..8].copy_from_slice(&(COMPACT_DOMAIN_GAP_PAYLOAD_LEN as u16).to_le_bytes());
        payload[8..10].copy_from_slice(&reason.to_le_bytes());
        let hour = DATE_EPOCH_CONST + 1936 * 365 * 24;
        payload[16..20].copy_from_slice(&hour.to_le_bytes());
        payload[20..24].copy_from_slice(&hour.to_le_bytes());
        payload[24..28].copy_from_slice(&hour.to_le_bytes());
        payload[32..40].copy_from_slice(&10u64.to_le_bytes());
        payload[40..48].copy_from_slice(&11u64.to_le_bytes());
        payload
    }

    #[test]
    fn record_flag_policy_is_domain_and_type_specific() {
        assert_eq!(
            compact_record_allowed_flags(45, COMPACT_RECORD_TYPE_DOMAIN_DATA),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
        );
        assert_eq!(
            compact_record_allowed_flags(67, COMPACT_RECORD_TYPE_DOMAIN_DATA),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
        );
        assert_eq!(
            compact_record_allowed_flags(18, COMPACT_RECORD_TYPE_DOMAIN_PERF),
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER
        );
        assert_eq!(
            compact_record_allowed_flags(18, COMPACT_RECORD_TYPE_DOMAIN_DATA),
            0
        );
        assert_eq!(
            compact_record_rejected_flags(
                67,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            ),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
        );
        assert_ne!(
            compact_record_rejected_flags(
                18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            ),
            0
        );
    }

    #[test]
    fn domain_gap_is_terminal_outcome_but_not_perf_or_data_cardinality() {
        assert!(compact_record_counts_toward_terminal_outcome(
            COMPACT_RECORD_TYPE_DOMAIN_GAP
        ));
        assert!(!compact_record_counts_toward_perf_cardinality(
            COMPACT_RECORD_TYPE_DOMAIN_GAP
        ));
        assert!(compact_record_counts_toward_terminal_outcome(
            COMPACT_RECORD_TYPE_DOMAIN_PERF
        ));
        assert!(compact_record_counts_toward_perf_cardinality(
            COMPACT_RECORD_TYPE_DOMAIN_PERF
        ));
        assert!(!compact_record_counts_toward_terminal_outcome(
            COMPACT_RECORD_TYPE_DOMAIN_DATA
        ));
    }

    #[test]
    fn domain_gap_header_owns_affected_domain_and_witnesses_are_closed() {
        let torn = gap_payload(DOMAIN_GAP_REASON_TORN);
        validate_compact_record_semantics(81, COMPACT_RECORD_TYPE_DOMAIN_GAP, 0, &torn).unwrap();
        for invalid_domain in [0, COMPACT_DOMAIN_FRAME] {
            assert!(validate_compact_record_semantics(
                invalid_domain,
                COMPACT_RECORD_TYPE_DOMAIN_GAP,
                0,
                &torn
            )
            .is_err());
        }
        assert!(
            validate_compact_record_semantics(81, COMPACT_RECORD_TYPE_DOMAIN_GAP, 1, &torn)
                .is_err()
        );

        let mut queue_on_torn = torn.clone();
        queue_on_torn[48..52].copy_from_slice(&1u32.to_le_bytes());
        assert!(validate_compact_record_semantics(
            81,
            COMPACT_RECORD_TYPE_DOMAIN_GAP,
            0,
            &queue_on_torn
        )
        .is_err());

        let mut backpressure = gap_payload(DOMAIN_GAP_REASON_BACKPRESSURE);
        backpressure[40..48].copy_from_slice(&10u64.to_le_bytes());
        backpressure[48..52].copy_from_slice(&8u32.to_le_bytes());
        backpressure[52..56].copy_from_slice(&8u32.to_le_bytes());
        backpressure[56..64].copy_from_slice(&1u64.to_le_bytes());
        validate_compact_record_semantics(81, COMPACT_RECORD_TYPE_DOMAIN_GAP, 0, &backpressure)
            .unwrap();
    }

    #[test]
    fn accepts_exact_base_perf_without_trailer() {
        perf_result(0, &base_perf()).expect("exact base PERF must be valid");
    }

    #[test]
    fn announced_perf_trailer_must_be_present() {
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &base_perf())
            .expect_err("bit 30 without a trailer must fail");
        assert!(err.contains("trailer header is truncated"), "{err}");
    }

    #[test]
    fn perf_trailer_must_be_announced() {
        let payload = perf_with_trailer(None, 0);
        let err = perf_result(0, &payload).expect_err("unannounced trailer must fail");
        assert!(err.contains("unannounced trailer"), "{err}");
    }

    #[test]
    fn accepts_empty_and_nonempty_announced_pdu1_trailers() {
        for payload in [
            perf_with_trailer(None, 0),
            perf_with_trailer(Some(b"read_us"), 0),
        ] {
            perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &payload)
                .expect("canonical PDU1 trailer must be valid");
        }
    }

    #[test]
    fn rejects_bad_pdu1_magic_and_undefined_version() {
        let mut bad_magic = perf_with_trailer(None, 0);
        bad_magic[COMPACT_DOMAIN_PERF_BASE_LEN] = b'X';
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &bad_magic)
            .expect_err("bad PDU magic must fail");
        assert!(err.contains("magic is invalid"), "{err}");

        for version in [0u16, 3, 9, u16::MAX] {
            let mut bad_version = perf_with_trailer(None, 0);
            bad_version[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
                .copy_from_slice(&version.to_le_bytes());
            let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &bad_version)
                .expect_err("an undefined trailer version must fail");
            assert!(
                err.contains(&format!("version {version} is neither")),
                "{err}"
            );
        }

        for version in [
            COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION,
            COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2,
        ] {
            let mut good = perf_with_trailer(None, 0);
            good[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
                .copy_from_slice(&version.to_le_bytes());
            perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &good)
                .unwrap_or_else(|error| panic!("version {version} must validate: {error}"));
        }
    }

    #[test]
    fn a_version_two_unit_must_declare_a_defined_role_and_presence() {
        use crate::diagnostic_channel::{UnitPresence, UnitRole};
        let declaration =
            |role: u8, presence: u8| -> u16 { u16::from(role) | (u16::from(presence) << 8) };
        let mut good = perf_with_trailer(
            Some(b"d44_country_supply_failure_stage"),
            declaration(UnitRole::Deciding.code(), UnitPresence::Observed.code()),
        );
        good[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
            .copy_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2.to_le_bytes());
        perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &good)
            .expect("a fully declared version 2 unit must validate");

        for (role, presence, expected) in [
            (0u8, UnitPresence::Observed.code(), "undefined role code 0"),
            (99, UnitPresence::Observed.code(), "undefined role code 99"),
            (UnitRole::Deciding.code(), 0, "undefined presence code 0"),
            (UnitRole::Deciding.code(), 99, "undefined presence code 99"),
        ] {
            let mut bad = perf_with_trailer(Some(b"x"), declaration(role, presence));
            bad[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
                .copy_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2.to_le_bytes());
            let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &bad)
                .expect_err("an undeclared version 2 unit must fail");
            assert!(err.contains(expected), "{err}");
        }

        let mut legacy = perf_with_trailer(Some(b"x"), 1);
        legacy[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
            .copy_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes());
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &legacy)
            .expect_err("version 1 reserved bytes must stay zero");
        assert!(err.contains("reserved field is nonzero"), "{err}");
    }

    #[test]
    fn rejects_reserved_perf_words() {
        let mut bad_base = base_perf();
        bad_base[36..40].copy_from_slice(&1u32.to_le_bytes());
        let err = perf_result(0, &bad_base).expect_err("nonzero base reserved word must fail");
        assert!(err.contains("reserved base word"), "{err}");

        let bad_unit = perf_with_trailer(Some(b"read_us"), 1);
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &bad_unit)
            .expect_err("nonzero unit reserved word must fail");
        assert!(err.contains("reserved field"), "{err}");
    }

    #[test]
    fn rejects_invalid_perf_unit_names() {
        for name in [b"ReadUs".as_slice(), b"".as_slice(), &[0xff][..]] {
            let payload = perf_with_trailer(Some(name), 0);
            assert!(
                perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &payload).is_err(),
                "invalid diagnostic name {name:?} was accepted"
            );
        }
    }

    #[test]
    fn rejects_trailing_pdu1_bytes() {
        let mut payload = perf_with_trailer(Some(b"read_us"), 0);
        payload.push(0);
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &payload)
            .expect_err("trailing PDU1 bytes must fail");
        assert!(err.contains("trailing byte"), "{err}");
    }

    #[test]
    fn frame_end_domain_payload_and_flags_are_exact() {
        validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        )
        .expect("canonical FRAME_END must be valid");

        let wrong_domain = validate_compact_record_semantics(
            41,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        )
        .expect_err("wrong FRAME_END domain must fail");
        assert!(wrong_domain.contains("FRAME_END domain"), "{wrong_domain}");

        let wrong_len = validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN - 1],
        )
        .expect_err("wrong FRAME_END length must fail");
        assert!(wrong_len.contains("payload length"), "{wrong_len}");

        let wrong_flags = validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        )
        .expect_err("FRAME_END flags must fail");
        assert!(wrong_flags.contains("rejected bits"), "{wrong_flags}");
    }

    #[test]
    fn freeze_census_envelope_is_frame_domain_zero_flags_and_no_coverage() {
        use crate::freeze_census_record::{
            FreezeCensusCounterSnapshot, FreezeCensusPhase, FreezeCensusRecordV1,
        };

        let payload = FreezeCensusRecordV1::encode_bounded(
            FreezeCensusPhase::Install,
            FreezeCensusCounterSnapshot::default(),
            FreezeCensusCounterSnapshot::default(),
            &[],
        )
        .unwrap();
        validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            0,
            &payload,
        )
        .unwrap();
        assert!(validate_compact_record_semantics(
            18,
            COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            0,
            &payload,
        )
        .is_err());
        assert!(validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            1,
            &payload,
        )
        .is_err());
        assert_eq!(
            compact_record_allowed_flags(COMPACT_DOMAIN_FRAME, COMPACT_RECORD_TYPE_FREEZE_CENSUS),
            0
        );
        assert!(!compact_record_counts_toward_terminal_outcome(
            COMPACT_RECORD_TYPE_FREEZE_CENSUS
        ));
        assert!(!compact_record_counts_toward_perf_cardinality(
            COMPACT_RECORD_TYPE_FREEZE_CENSUS
        ));
    }

    #[test]
    fn raw_unknown_record_type_fifteen_rejects() {
        // 14 became DOMAIN_DATA_REPEAT (lane PV); 15 is the first unassigned type.
        let error = validate_compact_record_semantics(COMPACT_DOMAIN_FRAME, 15, 0, &[])
            .expect_err("raw type 15 is not assigned");
        assert_eq!(error, "unknown compact record type 15");

        let topology_error = validate_compact_record_semantics(COMPACT_DOMAIN_FRAME, 13, 0, &[])
            .expect_err("an empty payload is not a state topology census");
        assert!(
            topology_error.contains("state topology census"),
            "type 13 is assigned and validated by its own grammar: {topology_error}"
        );
        let topology = crate::state_topology_census::StateTopologyCensusV1 {
            state_ids: (1..=1_081).collect(),
        }
        .encode()
        .expect("the topology census encodes");
        validate_compact_record_semantics(COMPACT_DOMAIN_FRAME, 13, 0, &topology)
            .expect("a well-formed topology census is admitted on the frame control domain");
        validate_compact_record_semantics(48, 13, 0, &topology)
            .expect_err("a topology census may not claim a domain");

        let assigned = validate_compact_record_semantics(COMPACT_DOMAIN_FRAME, 12, 0, &[])
            .expect_err("an empty payload is not a census");
        assert!(
            assigned.contains("content identity census"),
            "type 12 is assigned and validated by its own grammar: {assigned}"
        );
        let census = crate::content_identity_census::ContentIdentityCensusV1 {
            buildings: vec![crate::content_identity_census::ContentIdentityRow {
                token_id: 0x4E2D,
                token: "industrial_complex".to_string(),
            }],
            resources: Vec::new(),
            buildings_declared: 1,
            resources_declared: 0,
            status: 0,
        }
        .encode()
        .expect("the census encodes");
        validate_compact_record_semantics(COMPACT_DOMAIN_FRAME, 12, 0, &census)
            .expect("a well-formed census is admitted on the frame control domain");
        validate_compact_record_semantics(18, 12, 0, &census)
            .expect_err("a census may not claim a domain");
    }

    #[test]
    fn explicitly_accepts_data_and_rejects_unknown_record_types() {
        validate_compact_record_semantics(18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &[])
            .expect("DOMAIN_DATA envelope semantics must be accepted");
        let misplaced = validate_compact_record_semantics(
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &[],
        )
        .expect_err("misplaced PERF flag on DOMAIN_DATA must fail");
        assert!(misplaced.contains("rejected bits"), "{misplaced}");
        let err = validate_compact_record_semantics(18, 99, 0, &[])
            .expect_err("unknown record type must fail");
        assert!(err.contains("unknown compact record type 99"), "{err}");
    }
}
