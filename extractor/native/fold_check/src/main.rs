use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
use compact_contract::frame_refusal::FrameRefusalRecordV2;
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode, StreamValidationError,
};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance,
    compact_status_for_global_interpretation, compact_status_is_failure, lane_local_status_mask,
    parse_compact_record_header, validate_compact_record_semantics, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_CAPTURE_TERMINATED, COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_GAP,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_DEGRADED,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_TYPE_FRAME_REFUSED,
    COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY,
};
use replay_decoder::{DecodeStats, OpenOptions, Source};
use serde_json::{json, Map, Value};
use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::{to_hex, Sha256};

const FOLD_CHECK_GAME_ID: &str = "fold_check";

fn fold_check_opts() -> OpenOptions {
    OpenOptions {
        game_id: Some(FOLD_CHECK_GAME_ID.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let rc = match args.get(1).map(String::as_str) {
        Some("hash") => cmd_hash(&args[2..]),
        Some("compare") => cmd_compare(&args[2..]),
        Some("-h") | Some("--help") | None => {
            print_usage();
            Ok(0)
        }
        Some(other) => {
            eprintln!("fold_check: unknown subcommand {other:?}\n");
            print_usage();
            Ok(2)
        }
    };
    match rc {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("fold_check: error: {e}");
            ExitCode::from(1)
        }
    }
}

fn print_usage() {
    eprintln!(
        "fold_check — fold-equivalence gate (G1)\n\
         \n\
         USAGE:\n\
         \x20 fold_check hash    <input> --expected-profile production [--identity-mode legacy|identity-v1] [--out report.json]\n\
         \x20 fold_check compare <a> <b> --expected-profile production [--identity-mode legacy|identity-v1] [--out report.json] [--sample N]\n\
         \n\
         <input> is a collector_stream.bin OR a .battleplan container.\n\
         hash    prints a deterministic per-hour hash report.\n\
         compare exits 0 iff the two inputs fold identically.\n\
         --expected-profile is mandatory: certification never trusts the\n\
         stream's own FRAME_END counts as its coverage authority.\n\
         --identity-mode defaults to legacy. identity-v1 explicitly requires\n\
         one placement-valid CID1 sequence-zero header before ordinary records."
    );
}

fn stream_identity_mode_name(mode: StreamIdentityMode) -> &'static str {
    match mode {
        StreamIdentityMode::Legacy => "legacy",
        StreamIdentityMode::IdentityV1 => "identity-v1",
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DomainCompleteness {
    data_records: u64,
    perf_records: u64,
    gap_records: u64,
    gap_reason_mask: u32,
    gap_part_mask: u32,
    data_payload_status_flags_or: u32,
    normalized_status_flags_or: u32,
    lane_local_status_flags_or: u32,
    non_tolerated_status_flags: u32,
    raw_perf_status_flags_or: u32,
    non_tolerated_data_payload_status_flags: u32,
    excluded_data_records: u64,
    excluded_data_reasons: std::collections::BTreeSet<String>,
}

impl DomainCompleteness {
    fn terminal_outcomes(&self) -> u64 {
        self.perf_records.saturating_add(self.gap_records)
    }

    /// module warns is worse than the deadlock it was built to solve.
    fn state(&self, domain_id: u16) -> &'static str {
        if self.gap_records != 0 {
            return "absent";
        }
        if self.excluded_data_records != 0 {
            return "excluded";
        }
        if self.non_tolerated_status_flags != 0 || self.non_tolerated_data_payload_status_flags != 0
        {
            return "degraded_unknown";
        }
        if matches!(
            compact_contract::frame_degraded::compact_domain_completeness(
                domain_id,
                self.normalized_status_flags_or,
            ),
            compact_contract::frame_degraded::CompactDomainCompleteness::ExpectedPartial
        ) {
            return "expected_partial";
        }
        "complete"
    }

    fn is_complete(&self, domain_id: u16) -> bool {
        self.state(domain_id) == "complete"
    }
}

fn gap_reason_names(mask: u32) -> Vec<&'static str> {
    [
        (DomainGapReason::Torn, "Torn"),
        (DomainGapReason::Overrun, "Overrun"),
        (DomainGapReason::Backpressure, "Backpressure"),
        (DomainGapReason::ClockUnreadable, "ClockUnreadable"),
        (DomainGapReason::WorkerPanic, "WorkerPanic"),
        (DomainGapReason::SourceUnavailable, "SourceUnavailable"),
        (DomainGapReason::ReaderSilent, "ReaderSilent"),
        (DomainGapReason::HourLabelMismatch, "HourLabelMismatch"),
    ]
    .into_iter()
    .filter(|(reason, _)| mask & (1u32 << reason.code()) != 0)
    .map(|(_, name)| name)
    .collect()
}

#[test]
fn every_gap_reason_has_a_report_name() {
    for reason in DomainGapReason::ALL {
        assert_eq!(
            gap_reason_names(1u32 << reason.code()).len(),
            1,
            "{reason:?} has no name in the report table, so it would vanish from every \
             fold_check verdict while still setting its bit in the mask"
        );
    }
}

fn gap_part_names(mask: u32) -> Vec<&'static str> {
    [
        (DomainGapPart::State, "State"),
        (DomainGapPart::Event, "Event"),
        (DomainGapPart::Both, "Both"),
    ]
    .into_iter()
    .filter(|(part, _)| mask & (1u32 << part.code()) != 0)
    .map(|(_, name)| name)
    .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RefusalOffender {
    domain_id: u16,
    status_flags: u32,
    non_tolerated_flags: u32,
    reason: String,
    detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FrameFinding {
    predicate: String,
    detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HourRefusal {
    wire_version: u16,
    offenders: Vec<RefusalOffender>,
    frame_findings: Vec<FrameFinding>,
    discarded_terminal_outcome_domains: Vec<u16>,
    discarded_data_records: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HourCompleteness {
    domains: BTreeMap<u16, DomainCompleteness>,
    frame_emitted_domains: u32,
    frame_scheduled_domains: u32,
    normalized_frame_status_flags: u32,
    normalized_status_flags_or: u32,
    declared_degraded_domains: BTreeMap<u16, (u32, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedDomain {
    wire_name: String,
    requires_data_record: bool,
    mandatory: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedRoster {
    profile: String,
    domains: BTreeMap<u16, ExpectedDomain>,
}

impl ExpectedRoster {
    fn production() -> Self {
        let domains = compact_contract::passive_capture_roster()
            .into_iter()
            .map(|descriptor| {
                (
                    descriptor.id,
                    ExpectedDomain {
                        wire_name: descriptor.wire_name.to_string(),
                        requires_data_record: descriptor.requires_data_record,
                        mandatory: !compact_contract::PASSIVE_CAPTURE_ONLY_DOMAIN_IDS
                            .contains(&descriptor.id),
                    },
                )
            })
            .collect();
        Self {
            profile: "production".to_string(),
            domains,
        }
    }

    #[cfg(test)]
    fn test(domains: &[(u16, bool)]) -> Self {
        Self {
            profile: "test".to_string(),
            domains: domains
                .iter()
                .map(|(domain_id, requires_data_record)| {
                    (
                        *domain_id,
                        ExpectedDomain {
                            wire_name: compact_contract::wire_name_for_id(*domain_id)
                                .unwrap_or("test_domain")
                                .to_string(),
                            requires_data_record: *requires_data_record,
                            mandatory: true,
                        },
                    )
                })
                .collect(),
        }
    }
}

fn verify_expected_roster(
    hour_index: i64,
    completeness: &HourCompleteness,
    expected: &ExpectedRoster,
) -> Result<(), String> {
    let expected_count = u32::try_from(expected.domains.len())
        .map_err(|_| "expected domain roster is larger than u32".to_string())?;
    if completeness.frame_scheduled_domains != expected_count {
        return Err(format!(
            "hour {hour_index} FRAME_END scheduled-domain count {} does not match externally authoritative {} profile roster count {expected_count}",
            completeness.frame_scheduled_domains, expected.profile
        ));
    }
    if completeness.frame_emitted_domains != expected_count {
        return Err(format!(
            "hour {hour_index} FRAME_END emitted-domain count {} does not match externally authoritative {} profile roster count {expected_count}",
            completeness.frame_emitted_domains, expected.profile
        ));
    }

    for (domain_id, expected_domain) in &expected.domains {
        let observed = completeness.domains.get(domain_id);
        let perf_records = observed.map_or(0, |domain| domain.perf_records);
        let gap_records = observed.map_or(0, |domain| domain.gap_records);
        if gap_records != 0 {
            if !expected_domain.mandatory {
                continue;
            }
            let (reason_mask, part_mask) = observed
                .map(|domain| (domain.gap_reason_mask, domain.gap_part_mask))
                .unwrap_or((0, 0));
            return Err(format!(
                "hour {hour_index} {} profile domain {} ({domain_id}) terminal outcome is {gap_records} DOMAIN_GAP record(s) (reason {}; part {}), an accounted absence rather than an extracted outcome",
                expected.profile,
                expected_domain.wire_name,
                gap_reason_names(reason_mask).join("|"),
                gap_part_names(part_mask).join("|"),
            ));
        }
        if perf_records != 1 {
            return Err(format!(
                "hour {hour_index} expected {} profile domain {} ({domain_id}) PERF record count 1, observed {perf_records}",
                expected.profile, expected_domain.wire_name
            ));
        }
        let data_records = observed.map_or(0, |domain| domain.data_records);
        let expected_data_records = u64::from(expected_domain.requires_data_record);
        if data_records != expected_data_records {
            return Err(format!(
                "hour {hour_index} expected {} profile domain {} ({domain_id}) DATA record count {expected_data_records}, observed {data_records}",
                expected.profile, expected_domain.wire_name
            ));
        }
    }

    for (domain_id, observed) in &completeness.domains {
        if (observed.data_records != 0 || observed.terminal_outcomes() != 0)
            && !expected.domains.contains_key(domain_id)
        {
            return Err(format!(
                "hour {hour_index} observed domain {domain_id} outside externally authoritative {} profile roster (DATA={}, PERF={}, GAP={})",
                expected.profile,
                observed.data_records,
                observed.perf_records,
                observed.gap_records
            ));
        }
    }
    Ok(())
}

#[derive(Default)]
struct CompletenessAudit {
    /// This walker's own `PSD1` baselines
    /// ([`compact_contract::payload_span_delta`]).
    ///
    /// The completeness audit is an INDEPENDENT walker of the same spool -- the
    /// fold, the seal walk and the manifest summary are the others -- so it
    /// keeps its own store and reconstructs for itself. Without it every
    /// enveloped record's `domain_data_payload_status_at` would return `None`
    /// and the audit would quietly stop accounting every delta-enabled domain's
    /// (d43/d47/d53 today) declared
    /// status flags: not an error, just those domains missing from a
    /// completeness answer, which is the shape of defect this audit exists to
    /// prevent.
    span_delta: compact_contract::payload_span_delta::PayloadSpanDeltaConsumer,
    hours: BTreeMap<i64, HourCompleteness>,
    refusals: BTreeMap<i64, HourRefusal>,
    current_domains: BTreeMap<u16, DomainCompleteness>,
    current_declared_degraded: BTreeMap<u16, (u32, u32)>,
}

impl CompletenessAudit {
    fn observe(
        &mut self,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        sequence: u64,
        stream_offset: u64,
        payload: &[u8],
    ) -> Result<(), String> {
        validate_compact_record_semantics(domain_id, record_type, record_flags, payload)?;
        // UNWRAP BEFORE ANYTHING LOOKS AT THE PAYLOAD, and clear the envelope
        // flag downstream: the flag describes the payload being handed over, and
        // after this it is the domain's own grammar.
        // A RECONSTRUCTION FAILURE EXCLUDES AND NAMES THE RECORD; IT DOES NOT
        // ABORT THE AUDIT (Codex review round 1, finding 3). The first draft
        // propagated the error with `?`, and the caller turns an `Err` here into
        // a whole-run failure -- so one unreadable record would have destroyed
        // the completeness answer for every hour of the capture, while the
        // audit's own `excluded_data_records` / `excluded_data_reasons`
        // machinery, which exists for exactly this, was never reached. It is
        // also the opposite of what an ordinary malformed DOMAIN_DATA payload
        // does below, where `domain_data_payload_status_at(...).unwrap_or(None)`
        // records an unreadable status and carries on. Partial data still seals.
        let reconstructed = match compact_contract::payload_span_delta::reconstruct_if_enveloped(
            &mut self.span_delta,
            record_flags,
            compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
                domain_id,
                source_sequence: sequence,
                game_hour_raw,
            },
            payload,
        ) {
            Ok(reconstructed) => reconstructed,
            Err(_reason) => {
                // THE EXCLUSION IS COUNTED BY THE LEDGER, NOT HERE (Codex review
                // round 5, finding 1). The production fold has already recorded
                // this same record in its `ExcludedDomainDataRecord` ledger, and
                // `attribute_exclusion_ledger` below increments
                // `excluded_data_records` and inserts the reason when it walks
                // that ledger for this hour -- so counting it a second time here
                // reported one lost record as two, a false loss count in a
                // completeness answer.
                //
                // What this arm must still do is REGISTER THE DOMAIN for the
                // hour, because the ledger attribution refuses an entry naming a
                // domain the audit did not see. That is exactly what an ordinary
                // unreadable payload does two calls below, where
                // `domain_data_payload_status_at(...).unwrap_or(None)` records a
                // data record with no readable status and carries on.
                //
                // The two cannot disagree about WHICH records fail: this audit
                // and the fold walk the same records of the same stream in the
                // same order, each advancing its own baseline store on every
                // enveloped record -- the same reason the eight parallel fold
                // workers stay in lockstep with one another.
                self.current_domains.entry(domain_id).or_default().data_records += 1;
                return Ok(());
            }
        };
        let record_flags = if reconstructed.is_some() {
            record_flags & !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
        } else {
            record_flags
        };
        let payload = reconstructed.as_deref().unwrap_or(payload);
        let record_type = compact_contract::KnownRecordType::try_from(record_type)
            .map_err(|unknown| format!("unknown compact record type {}", unknown.0))?;
        match record_type {
            compact_contract::KnownRecordType::DomainData => {
                let payload_status = replay_decoder::domain_data_payload_status_at(
                    domain_id,
                    payload,
                    record_flags,
                    sequence,
                    game_hour_raw,
                    stream_offset,
                )
                .unwrap_or(None);
                let residual = payload_status.map_or(0, |status_flags| {
                    compact_status_failure_flags_after_expected_partial_tolerance(
                        domain_id,
                        status_flags,
                    )
                });
                if residual != 0 && compact_contract::assigned_descriptor_by_id(domain_id).is_none()
                {
                    let status_flags = payload_status.unwrap_or(0);
                    return Err(format!(
                        "domain {domain_id} DOMAIN_DATA payload status {status_flags:#010x} has non-tolerated failure flags {residual:#010x}; the domain is outside the assigned roster, so no H4FD notice can account for it"
                    ));
                }
                let domain = self.current_domains.entry(domain_id).or_default();
                domain.data_records += 1;
                if let Some(status_flags) = payload_status {
                    domain.non_tolerated_data_payload_status_flags |= residual;
                    domain.data_payload_status_flags_or |=
                        compact_status_for_global_interpretation(domain_id, status_flags);
                    domain.normalized_status_flags_or |=
                        compact_status_for_global_interpretation(domain_id, status_flags);
                    domain.lane_local_status_flags_or |=
                        status_flags & lane_local_status_mask(domain_id);
                }
            }
            compact_contract::KnownRecordType::DomainPerf => {
                let status_flags = read_u32_at(payload, 32, "DOMAIN_PERF status")?;
                let normalized = compact_status_for_global_interpretation(domain_id, status_flags);
                let residual = compact_status_failure_flags_after_expected_partial_tolerance(
                    domain_id,
                    status_flags,
                );
                if compact_contract::assigned_descriptor_by_id(domain_id).is_some() {
                    let domain = self.current_domains.entry(domain_id).or_default();
                    domain.perf_records += 1;
                    domain.normalized_status_flags_or |= normalized;
                    domain.lane_local_status_flags_or |=
                        status_flags & lane_local_status_mask(domain_id);
                    domain.non_tolerated_status_flags |= residual;
                    domain.raw_perf_status_flags_or |= status_flags;
                } else if residual != 0 {
                    return Err(format!(
                        "domain {domain_id} PERF status {status_flags:#010x} has non-tolerated failure flags {residual:#010x}"
                    ));
                }
            }
            compact_contract::KnownRecordType::FrameDegraded => {
                let record =
                    compact_contract::frame_degraded::FrameDegradedRecordV2::decode(payload)
                        .map_err(|reason| {
                            format!("FRAME_DEGRADED payload is unreadable: {reason}")
                        })?;
                for domain in &record.domains {
                    if self
                        .current_declared_degraded
                        .insert(
                            domain.domain_id,
                            (domain.status_flags, domain.non_tolerated_flags),
                        )
                        .is_some()
                    {
                        return Err(format!(
                            "hour {game_hour_raw} declares domain {} degraded more than once",
                            domain.domain_id
                        ));
                    }
                }
            }
            compact_contract::KnownRecordType::FrameEnd => {
                let emitted = read_u32_at(payload, 16, "FRAME_END emitted-domain count")?;
                let scheduled = read_u32_at(payload, 20, "FRAME_END scheduled-domain count")?;
                let status_flags = read_u32_at(payload, 24, "FRAME_END status")?;
                let normalized = compact_status_for_global_interpretation(domain_id, status_flags);
                if compact_status_is_failure(normalized) {
                    return Err(format!(
                        "FRAME_END status {status_flags:#010x} contains failure flags"
                    ));
                }
                if emitted != scheduled {
                    return Err(format!(
                        "FRAME_END emitted-domain count {emitted} does not match scheduled-domain count {scheduled}"
                    ));
                }
                let hour_index = compact_contract::calendar::hour_index_from_raw(game_hour_raw)
                    .ok_or_else(|| format!("FRAME_END game hour {game_hour_raw} is invalid"))?;

                let observed_degraded: BTreeMap<u16, (u32, u32)> = self
                    .current_domains
                    .iter()
                    .filter(|(_, domain)| domain.non_tolerated_status_flags != 0)
                    .map(|(domain_id, domain)| {
                        (
                            *domain_id,
                            (
                                domain.raw_perf_status_flags_or,
                                domain.non_tolerated_status_flags,
                            ),
                        )
                    })
                    .collect();
                let declared_degraded = std::mem::take(&mut self.current_declared_degraded);
                if observed_degraded != declared_degraded {
                    for (domain_id, (raw, residual)) in &observed_degraded {
                        match declared_degraded.get(domain_id) {
                            Some((declared_raw, declared)) if declared == residual => {
                                if declared_raw != raw {
                                    return Err(format!(
                                        "domain {domain_id} PERF status is {raw:#010x} but the \
                                         hour's H4FD notice declares it degraded from status \
                                         {declared_raw:#010x}"
                                    ));
                                }
                            }
                            Some((_, declared)) => {
                                return Err(format!(
                                    "domain {domain_id} PERF status has non-tolerated failure flags \
                                     {residual:#010x} but the hour's H4FD notice declares it \
                                     degraded with {declared:#010x}"
                                ));
                            }
                            None => {
                                return Err(format!(
                                    "domain {domain_id} PERF status has non-tolerated failure flags \
                                     {residual:#010x} and the frame declares no H4FD degraded \
                                     notice for it"
                                ));
                            }
                        }
                    }
                    for domain_id in declared_degraded.keys() {
                        if !observed_degraded.contains_key(domain_id) {
                            return Err(format!(
                                "hour {hour_index} H4FD notice declares domain {domain_id} degraded \
                                 but its PERF status carries no non-tolerated failure flags"
                            ));
                        }
                    }
                    return Err(format!(
                        "hour {hour_index} observed degraded domains {observed_degraded:?} do not \
                         match the H4FD declaration {declared_degraded:?}"
                    ));
                }

                for (domain_id, domain) in &self.current_domains {
                    let payload_residual = domain.non_tolerated_data_payload_status_flags;
                    if payload_residual == 0 {
                        continue;
                    }
                    match declared_degraded.get(domain_id) {
                        Some((_, declared)) if (payload_residual & !declared) == 0 => {}
                        Some((_, declared)) => {
                            return Err(format!(
                                "domain {domain_id} DOMAIN_DATA payload status has non-tolerated \
                                 failure flags {payload_residual:#010x} that the hour's H4FD notice \
                                 does not declare; it declares it degraded with {declared:#010x}"
                            ));
                        }
                        None => {
                            return Err(format!(
                                "domain {domain_id} DOMAIN_DATA payload status has non-tolerated \
                                 failure flags {payload_residual:#010x} and the frame declares no \
                                 H4FD degraded notice for it"
                            ));
                        }
                    }
                }

                let aggregate = self
                    .current_domains
                    .values()
                    .fold(normalized, |status, domain| {
                        status | domain.normalized_status_flags_or
                    });
                let completeness = HourCompleteness {
                    domains: std::mem::take(&mut self.current_domains),
                    frame_emitted_domains: emitted,
                    frame_scheduled_domains: scheduled,
                    normalized_frame_status_flags: normalized,
                    normalized_status_flags_or: aggregate,
                    declared_degraded_domains: declared_degraded,
                };
                if self.refusals.contains_key(&hour_index) {
                    return Err(format!(
                        "hour {hour_index} is both refused by a FRAME_REFUSED and published by a FRAME_END"
                    ));
                }
                if self.hours.insert(hour_index, completeness).is_some() {
                    return Err(format!(
                        "more than one FRAME_END completeness record for hour {hour_index}"
                    ));
                }
            }
            compact_contract::KnownRecordType::DomainGap => {
                let gap = DomainGapRecordV1::decode(payload)
                    .map_err(|error| format!("domain {domain_id} DOMAIN_GAP payload: {error}"))?;
                if gap.target_hour_raw != game_hour_raw {
                    return Err(format!(
                        "domain {domain_id} DOMAIN_GAP is framed for hour {game_hour_raw} but its payload targets hour {}",
                        gap.target_hour_raw
                    ));
                }
                let domain = self.current_domains.entry(domain_id).or_default();
                domain.gap_records += 1;
                domain.gap_reason_mask |= 1u32 << gap.reason.code();
                domain.gap_part_mask |= 1u32 << gap.part.code();
            }
            compact_contract::KnownRecordType::FrameRefused => {
                let record = FrameRefusalRecordV2::decode(payload)
                    .map_err(|error| format!("FRAME_REFUSED payload: {error}"))?;
                let wire_version =
                    compact_contract::frame_refusal::declared_frame_refusal_version(payload)
                        .unwrap_or(compact_contract::frame_refusal::FRAME_REFUSAL_VERSION);
                let hour_index = compact_contract::calendar::hour_index_from_raw(game_hour_raw)
                    .ok_or_else(|| format!("FRAME_REFUSED game hour {game_hour_raw} is invalid"))?;
                let stale_declaration = std::mem::take(&mut self.current_declared_degraded);
                if !stale_declaration.is_empty() {
                    return Err(format!(
                        "hour {hour_index} carries an H4FD degraded notice for domains {:?} and \
                         then closes with FRAME_REFUSED; a notice annotates a published hour and \
                         a refusal closes an hour that published nothing",
                        stale_declaration.keys().collect::<Vec<_>>()
                    ));
                }
                let discarded = std::mem::take(&mut self.current_domains);
                let refusal = HourRefusal {
                    wire_version,
                    offenders: record
                        .offenders
                        .iter()
                        .map(|offender| RefusalOffender {
                            domain_id: offender.domain_id,
                            status_flags: offender.status_flags,
                            non_tolerated_flags: offender.non_tolerated_flags,
                            reason: offender.reason.clone(),
                            detail: Some(offender.render()),
                        })
                        .collect(),
                    frame_findings: record
                        .frame_findings
                        .iter()
                        .map(|finding| FrameFinding {
                            predicate: finding.predicate.name.clone(),
                            detail: finding.render(),
                        })
                        .collect(),
                    discarded_terminal_outcome_domains: discarded
                        .iter()
                        .filter(|(_, domain)| domain.terminal_outcomes() != 0)
                        .map(|(domain_id, _)| *domain_id)
                        .collect(),
                    discarded_data_records: discarded
                        .values()
                        .map(|domain| domain.data_records)
                        .sum(),
                };
                if self.hours.contains_key(&hour_index) {
                    return Err(format!(
                        "hour {hour_index} is both published by a FRAME_END and refused by a FRAME_REFUSED"
                    ));
                }
                if self.refusals.insert(hour_index, refusal).is_some() {
                    return Err(format!(
                        "more than one FRAME_REFUSED record for hour {hour_index}"
                    ));
                }
            }
            compact_contract::KnownRecordType::CaptureTerminated => {
                let termination =
                    compact_contract::capture_termination::CaptureTerminationRecordV1::decode(
                        payload,
                    )
                    .map_err(|error| format!("CAPTURE_TERMINATED payload: {error}"))?;
                return Err(format!(
                    "capture terminated abnormally after {} ms: reasons={:?}; detail={}",
                    termination.elapsed_wall_ms,
                    termination.reasons.names(),
                    termination.detail
                ));
            }
            compact_contract::KnownRecordType::ClockUnreadableInterval => {
                compact_contract::clock_unreadable_interval::
                    ClockUnreadableIntervalRecordV1::decode(payload)
                    .map_err(|error| format!("CLOCK_UNREADABLE_INTERVAL payload: {error}"))?;
            }
            compact_contract::KnownRecordType::UntrustedClockRecovery => {
                compact_contract::untrusted_clock_recovery::UntrustedClockRecoveryRecordV1::decode(
                    payload,
                )
                .map_err(|error| format!("UNTRUSTED_CLOCK_RECOVERY payload: {error}"))?;
            }
            compact_contract::KnownRecordType::FreezeCensus
            | compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StateTopologyCensus
            | compact_contract::KnownRecordType::StreamHeader => {}
        }
        Ok(())
    }
}

fn read_u32_at(payload: &[u8], offset: usize, label: &str) -> Result<u32, String> {
    let bytes = payload
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{label} is truncated"))?;
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("four-byte status/count slice"),
    ))
}

#[derive(Default)]
struct AuditResult {
    hours: BTreeMap<i64, HourCompleteness>,
    refusals: BTreeMap<i64, HourRefusal>,
}

fn audit_compact_reader_with_identity_mode(
    mut reader: impl Read,
    identity_mode: StreamIdentityMode,
) -> Result<AuditResult, String> {
    const CHUNK: usize = 1024 * 1024;
    let mut audit = CompletenessAudit::default();
    let mut stream_validator = match identity_mode {
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
    };
    let mut carry = Vec::with_capacity(CHUNK * 2);
    let mut chunk = vec![0u8; CHUNK];
    let mut offset = 0u64;
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| format!("read compact stream for completeness audit: {e}"))?;
        if n != 0 {
            carry.extend_from_slice(&chunk[..n]);
        }
        let mut cursor = 0usize;
        while carry.len().saturating_sub(cursor) >= COMPACT_RECORD_HEADER_LEN {
            let header = parse_compact_record_header(&carry[cursor..]).map_err(|e| {
                format!(
                    "completeness framing at byte {}: {e}",
                    offset + cursor as u64
                )
            })?;
            let record_len = header.record_len();
            if carry.len().saturating_sub(cursor) < record_len {
                break;
            }
            let payload_start = cursor + COMPACT_RECORD_HEADER_LEN;
            let payload = &carry[payload_start..cursor + record_len];
            let accepted = stream_validator
                .validate_record(&header, payload)
                .map_err(|error| {
                    let semantics_context = if identity_mode == StreamIdentityMode::Legacy
                        && matches!(&error, StreamValidationError::ReservedDomainZero)
                    {
                        validate_compact_record_semantics(
                            header.domain_id,
                            header.record_type,
                            header.record_flags,
                            payload,
                        )
                        .err()
                        .map(|error| format!("; current record semantics also reject it: {error}"))
                        .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    format!(
                        "{} stream validation at byte {}: {error}{semantics_context}",
                        stream_identity_mode_name(identity_mode),
                        offset + cursor as u64
                    )
                })?;
            if accepted == AcceptedRecordKind::CampaignIdentity {
                cursor += record_len;
                continue;
            }
            audit
                .observe(
                    header.game_hour_raw,
                    header.domain_id,
                    header.record_type,
                    header.record_flags,
                    header.sequence,
                    offset + cursor as u64,
                    payload,
                )
                .map_err(|e| {
                    format!(
                        "completeness record at byte {}: {e}",
                        offset + cursor as u64
                    )
                })?;
            cursor += record_len;
        }
        if cursor != 0 {
            carry.drain(..cursor);
            offset += cursor as u64;
        }
        if n == 0 {
            break;
        }
    }
    if !carry.is_empty() {
        return Err(format!(
            "truncated/corrupt tail: completeness audit ended with {} truncated byte(s) at byte {offset}; stream boundary is not closed",
            carry.len()
        ));
    }
    if identity_mode == StreamIdentityMode::IdentityV1 && stream_validator.identity().is_none() {
        return Err("identity-v1 stream ended before its mandatory CID1 header".to_string());
    }
    Ok(AuditResult {
        hours: audit.hours,
        refusals: audit.refusals,
    })
}

fn audit_source(source: &Source, identity_mode: StreamIdentityMode) -> Result<AuditResult, String> {
    match source {
        Source::StreamFile(path) => {
            let reader = compact_stream::open(path)
                .map_err(|e| format!("open {} for completeness audit: {e}", path.display()))?;
            audit_compact_reader_with_identity_mode(reader, identity_mode)
        }
        Source::Container(path) => {
            let container = Container::open(path, &ExpectedIdentity::default())
                .map_err(|e| format!("open {} for completeness audit: {e}", path.display()))?;
            let entries = container.entries_by_role(replay_decoder::STREAM_ROLE);
            let entry = match entries.as_slice() {
                [entry] => entry,
                [] => {
                    return Err(match snapshot::container_raw_stream_packaging(&container) {
                        Ok(snapshot::RawStreamPackaging::Omitted) => format!(
                            "{}: {}",
                            path.display(),
                            snapshot::raw_stream_unavailable_message(
                                "audit raw record completeness"
                            ),
                        ),
                        _ => format!(
                            "{} has no entry with role {:?}",
                            path.display(),
                            replay_decoder::STREAM_ROLE
                        ),
                    })
                }
                many => {
                    return Err(format!(
                        "{} has {} entries with role {:?}; expected exactly one",
                        path.display(),
                        many.len(),
                        replay_decoder::STREAM_ROLE
                    ))
                }
            };
            let bytes = container
                .read_entry_bytes(&entry.path)
                .map_err(|e| format!("read {} for completeness audit: {e}", entry.path))?;
            audit_compact_reader_with_identity_mode(std::io::Cursor::new(bytes), identity_mode)
        }
    }
}

fn attribute_exclusion_ledger(
    hour_index: i64,
    completeness: &mut HourCompleteness,
    ledger: &[replay_decoder::ExcludedDomainDataRecord],
    next: &mut usize,
) -> Result<u64, String> {
    let mut attributed = 0u64;
    while let Some(entry) = ledger.get(*next) {
        match compact_contract::calendar::hour_index_from_raw(entry.game_hour_raw) {
            Some(entry_hour) if entry_hour > hour_index => break,
            Some(entry_hour) if entry_hour == hour_index => {
                let domain = completeness
                    .domains
                    .get_mut(&entry.domain_id)
                    .ok_or_else(|| {
                        format!(
                            "hour {hour_index} exclusion ledger names domain {} at stream offset \
                             {} but the completeness audit recorded no such domain for the hour",
                            entry.domain_id, entry.stream_offset
                        )
                    })?;
                domain.excluded_data_records = domain.excluded_data_records.saturating_add(1);
                domain
                    .excluded_data_reasons
                    .insert(entry.validation_reason.clone());
                attributed += 1;
            }
            _ => {}
        }
        *next += 1;
    }
    Ok(attributed)
}

fn verify_exclusion_ledger_attribution(
    ledger: &[replay_decoder::ExcludedDomainDataRecord],
    attributed: u64,
    published_hours: &std::collections::BTreeSet<i64>,
    refusals: &BTreeMap<i64, HourRefusal>,
) -> Result<(), String> {
    let mut expected_attributed = 0u64;
    let mut unaccounted: Vec<String> = Vec::new();
    for entry in ledger {
        match compact_contract::calendar::hour_index_from_raw(entry.game_hour_raw) {
            Some(hour) if published_hours.contains(&hour) => expected_attributed += 1,
            Some(hour) if refusals.contains_key(&hour) => {}
            _ => unaccounted.push(format!(
                "domain {} at stream offset {} (game hour {})",
                entry.domain_id, entry.stream_offset, entry.game_hour_raw
            )),
        }
    }
    if !unaccounted.is_empty() {
        return Err(format!(
            "exclusion ledger names {} excluded DOMAIN_DATA record(s) belonging to neither a \
             published nor a refused hour, so their loss reaches no hour's completeness: {}",
            unaccounted.len(),
            unaccounted.join("; ")
        ));
    }
    if attributed != expected_attributed {
        return Err(format!(
            "exclusion ledger attribution is short: {expected_attributed} excluded record(s) \
             belong to published hours but only {attributed} reached a per-hour completeness \
             record"
        ));
    }
    Ok(())
}

fn verify_fold_counts(
    hour_index: i64,
    folded: &BTreeMap<u16, u64>,
    completeness: &HourCompleteness,
) -> Result<(), String> {
    let audited: BTreeMap<u16, u64> = completeness
        .domains
        .iter()
        .filter_map(|(domain_id, domain)| {
            (domain.data_records != 0).then_some((*domain_id, domain.data_records))
        })
        .collect();
    if &audited == folded {
        return Ok(());
    }
    let mut per_domain: Vec<String> = Vec::new();
    let domains: std::collections::BTreeSet<u16> =
        folded.keys().chain(audited.keys()).copied().collect();
    for domain_id in domains {
        let fold_count = folded.get(&domain_id).copied().unwrap_or(0);
        let audit_count = audited.get(&domain_id).copied().unwrap_or(0);
        if fold_count != audit_count {
            per_domain.push(format!(
                "domain {domain_id} production fold {fold_count} vs completeness audit {audit_count}"
            ));
        }
    }
    Err(format!(
        "hour {hour_index} DOMAIN_DATA counts disagree ({}); production fold {folded:?}, completeness audit {audited:?}",
        per_domain.join("; ")
    ))
}

struct HourReport {
    hour_index: i64,
    date: String,
    snapshot_hash: String,
    #[cfg(test)]
    world_hash: String,
    snapshot_bytes: usize,
    domain_data_counts: BTreeMap<u16, u64>,
    completeness: HourCompleteness,
}

#[derive(Default)]
struct Totals {
    stream_bytes: u64,
    bytes_folded: u64,
    truncated_tail_bytes: u64,
    records_total: u64,
    data_records: u64,
    malformed_payload_records: u64,
    excluded_domain_data_records: Vec<replay_decoder::ExcludedDomainDataRecord>,
    frame_end_records: u64,
    perf_records: u64,
    other_records: u64,
    hours_emitted: u64,
    per_domain_data_records: BTreeMap<u16, u64>,
    stop_reason: Option<String>,
    boundary_error: Option<String>,
    ends_at_frame_end: bool,
}

struct FoldRun {
    hours: Vec<HourReport>,
    refusals: BTreeMap<i64, HourRefusal>,
    totals: Totals,
}

fn totals_from_stats(s: &DecodeStats) -> Totals {
    Totals {
        stream_bytes: s.stream_bytes,
        bytes_folded: s.bytes_folded,
        truncated_tail_bytes: s.truncated_tail_bytes,
        records_total: s.records_total,
        data_records: s.data_records,
        malformed_payload_records: s.malformed_payload_records,
        excluded_domain_data_records: s.excluded_domain_data_records.clone(),
        frame_end_records: s.frame_end_records,
        perf_records: s.perf_records,
        other_records: s.other_records,
        hours_emitted: s.hours_emitted,
        per_domain_data_records: s.per_domain_data_records.clone(),
        stop_reason: s.stop_reason.clone(),
        boundary_error: s.boundary_error.clone(),
        ends_at_frame_end: s.ends_at_frame_end,
    }
}

/// as each game-hour closes so the caller can hash-and-release. The decoder folds
fn fold_input(
    path: &Path,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
    mut on_hour: impl FnMut(i64, &str, &[u8], &BTreeMap<u16, u64>, &HourCompleteness),
) -> Result<(Totals, &'static str, BTreeMap<i64, HourRefusal>), String> {
    let source = Source::detect(path).map_err(|e| e.to_string())?;
    let kind = match source {
        Source::StreamFile(_) => "raw_stream",
        Source::Container(_) => "container",
    };
    let audit = audit_source(&source, identity_mode)?;
    let mut completeness = audit.hours;
    let refusals = audit.refusals;
    let mut cursor = source
        .open_cursor_with_identity_mode(&fold_check_opts(), identity_mode)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut published_hours = 0usize;
    let mut published_hour_indexes: std::collections::BTreeSet<i64> =
        std::collections::BTreeSet::new();
    let mut attributed_exclusions = 0u64;
    let mut ledger_position = 0usize;
    while let Some(hour) = cursor.next_hour() {
        let hour = hour.map_err(|e| format!("fold {}: {e}", path.display()))?;
        let audited = completeness.get_mut(&hour.hour_index).ok_or_else(|| {
            format!(
                "completeness audit has no closed record boundary for folded hour {}",
                hour.hour_index
            )
        })?;
        attributed_exclusions += attribute_exclusion_ledger(
            hour.hour_index,
            audited,
            &cursor.stats().excluded_domain_data_records,
            &mut ledger_position,
        )?;
        let audited = &*audited;
        verify_fold_counts(hour.hour_index, &hour.domain_data_counts, audited)?;
        verify_expected_roster(hour.hour_index, audited, expected_roster)?;
        on_hour(
            hour.hour_index,
            &hour.date,
            &hour.snapshot,
            &hour.domain_data_counts,
            audited,
        );
        completeness.remove(&hour.hour_index);
        published_hours += 1;
        published_hour_indexes.insert(hour.hour_index);
    }
    cursor
        .stats()
        .certification_verdict()
        .map_err(|reason| format!("certify {}: {reason}", path.display()))?;
    verify_exclusion_ledger_attribution(
        &cursor.stats().excluded_domain_data_records,
        attributed_exclusions,
        &published_hour_indexes,
        &refusals,
    )
    .map_err(|reason| format!("certify {}: {reason}", path.display()))?;
    if let Some(hour) = completeness.keys().next() {
        return Err(format!(
            "completeness audit produced hour {hour} that the production fold did not emit"
        ));
    }
    verify_published_coverage(path, published_hours, &refusals, expected_roster)?;
    Ok((totals_from_stats(cursor.stats()), kind, refusals))
}

fn verify_published_coverage(
    path: &Path,
    published_hours: usize,
    refusals: &BTreeMap<i64, HourRefusal>,
    expected_roster: &ExpectedRoster,
) -> Result<(), String> {
    if published_hours != 0 {
        return Ok(());
    }
    let refused = refusals.len();
    let detail = match (refusals.keys().next(), refusals.keys().next_back()) {
        (Some(first), Some(last)) => {
            let mut offenders: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
            for refusal in refusals.values() {
                offenders.extend(refusal.offenders.iter().map(|offender| offender.domain_id));
            }
            let (predicate_count, predicates) = frame_finding_predicates(refusals);
            let frame_detail = if predicate_count == 0 {
                String::new()
            } else {
                let elided = predicate_count - predicates.len();
                let elision = if elided == 0 {
                    String::new()
                } else {
                    format!(" and {elided} more")
                };
                format!(" and {predicate_count} frame-level finding predicate(s) {predicates:?}{elision}")
            };
            format!(
                "; {refused} hour(s) {first}..{last} were attempted and refused, naming {} offending domain(s) {:?}{frame_detail}",
                offenders.len(),
                offenders
            )
        }
        _ => String::new(),
    };
    Err(format!(
        "certify {}: the stream publishes no game hour, so it certifies no {} profile coverage{detail}",
        path.display(),
        expected_roster.profile
    ))
}

#[cfg(test)]
fn world_only_hash(bytes: &[u8]) -> String {
    let mut doc: Value = serde_json::from_slice(bytes)
        .expect("a published hour snapshot is JSON; the fold serialized it");
    doc.as_object_mut()
        .expect("a published hour snapshot is a JSON object")
        .remove("provenance")
        .expect("every published hour carries a provenance member");
    to_hex(&Sha256::digest(
        serde_json::to_vec(&doc)
            .expect("re-serializing a parsed document cannot fail")
            .as_slice(),
    ))
}

fn push_hour(
    hours: &mut Vec<HourReport>,
    hour_index: i64,
    date: &str,
    bytes: &[u8],
    counts: &BTreeMap<u16, u64>,
    completeness: &HourCompleteness,
) {
    hours.push(HourReport {
        hour_index,
        date: date.to_string(),
        snapshot_hash: to_hex(&Sha256::digest(bytes)),
        #[cfg(test)]
        world_hash: world_only_hash(bytes),
        snapshot_bytes: bytes.len(),
        domain_data_counts: counts.clone(),
        completeness: completeness.clone(),
    });
}

#[cfg(test)]
fn run_input(
    path: &Path,
    expected_roster: &ExpectedRoster,
) -> Result<(FoldRun, &'static str), String> {
    run_input_with_identity_mode(path, expected_roster, StreamIdentityMode::Legacy)
}

fn run_input_with_identity_mode(
    path: &Path,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
) -> Result<(FoldRun, &'static str), String> {
    let mut hours: Vec<HourReport> = Vec::new();
    let (totals, kind, refusals) = fold_input(
        path,
        expected_roster,
        identity_mode,
        |hour_index, date, bytes, counts, completeness| {
            push_hour(&mut hours, hour_index, date, bytes, counts, completeness)
        },
    )?;
    Ok((
        FoldRun {
            hours,
            refusals,
            totals,
        },
        kind,
    ))
}

#[cfg(test)]
fn collect_run(reader: std::io::Cursor<Vec<u8>>, total_len: u64) -> Result<FoldRun, String> {
    collect_run_with_expected_roster(reader, total_len, None)
}

#[cfg(test)]
fn collect_identity_v1_run(
    reader: std::io::Cursor<Vec<u8>>,
    total_len: u64,
) -> Result<FoldRun, String> {
    collect_run_with_identity_mode_and_expected_roster(
        reader,
        total_len,
        StreamIdentityMode::IdentityV1,
        None,
    )
}

#[cfg(test)]
fn collect_certified_run(
    reader: std::io::Cursor<Vec<u8>>,
    total_len: u64,
    expected_roster: &ExpectedRoster,
) -> Result<FoldRun, String> {
    collect_run_with_expected_roster(reader, total_len, Some(expected_roster))
}

#[cfg(test)]
fn collect_run_with_expected_roster(
    reader: std::io::Cursor<Vec<u8>>,
    total_len: u64,
    expected_roster: Option<&ExpectedRoster>,
) -> Result<FoldRun, String> {
    collect_run_with_identity_mode_and_expected_roster(
        reader,
        total_len,
        StreamIdentityMode::Legacy,
        expected_roster,
    )
}

#[cfg(test)]
fn collect_run_with_identity_mode_and_expected_roster(
    reader: std::io::Cursor<Vec<u8>>,
    total_len: u64,
    identity_mode: StreamIdentityMode,
    expected_roster: Option<&ExpectedRoster>,
) -> Result<FoldRun, String> {
    let mut hours: Vec<HourReport> = Vec::new();
    let audit = audit_compact_reader_with_identity_mode(
        std::io::Cursor::new(reader.get_ref().clone()),
        identity_mode,
    )?;
    let mut completeness = audit.hours;
    let refusals = audit.refusals;
    let mut cursor = replay_decoder::open_reader_with_identity_mode(
        reader,
        total_len,
        FOLD_CHECK_GAME_ID,
        identity_mode,
    );
    let mut published_hour_indexes: std::collections::BTreeSet<i64> =
        std::collections::BTreeSet::new();
    let mut attributed_exclusions = 0u64;
    let mut ledger_position = 0usize;
    while let Some(hour) = cursor.next_hour() {
        let hour = hour.map_err(|e| e.to_string())?;
        let audited = completeness.get_mut(&hour.hour_index).ok_or_else(|| {
            format!(
                "completeness audit has no closed record boundary for folded hour {}",
                hour.hour_index
            )
        })?;
        attributed_exclusions += attribute_exclusion_ledger(
            hour.hour_index,
            audited,
            &cursor.stats().excluded_domain_data_records,
            &mut ledger_position,
        )?;
        let audited = &*audited;
        verify_fold_counts(hour.hour_index, &hour.domain_data_counts, audited)?;
        if let Some(expected_roster) = expected_roster {
            verify_expected_roster(hour.hour_index, audited, expected_roster)?;
        }
        push_hour(
            &mut hours,
            hour.hour_index,
            &hour.date,
            &hour.snapshot,
            &hour.domain_data_counts,
            audited,
        );
        completeness.remove(&hour.hour_index);
        published_hour_indexes.insert(hour.hour_index);
    }
    cursor
        .stats()
        .certification_verdict()
        .map_err(|reason| format!("certify in-memory stream: {reason}"))?;
    verify_exclusion_ledger_attribution(
        &cursor.stats().excluded_domain_data_records,
        attributed_exclusions,
        &published_hour_indexes,
        &refusals,
    )
    .map_err(|reason| format!("certify in-memory stream: {reason}"))?;
    if let Some(hour) = completeness.keys().next() {
        return Err(format!(
            "completeness audit produced hour {hour} that the production fold did not emit"
        ));
    }
    Ok(FoldRun {
        hours,
        refusals,
        totals: totals_from_stats(cursor.stats()),
    })
}

fn chain_hash(hours: &[HourReport]) -> String {
    let mut h = Sha256::new();
    for hr in hours {
        h.update(hr.hour_index.to_le_bytes().as_slice());
        h.update(hr.snapshot_hash.as_bytes());
        h.update(
            hr.completeness
                .frame_emitted_domains
                .to_le_bytes()
                .as_slice(),
        );
        h.update(
            hr.completeness
                .frame_scheduled_domains
                .to_le_bytes()
                .as_slice(),
        );
        h.update(
            hr.completeness
                .normalized_frame_status_flags
                .to_le_bytes()
                .as_slice(),
        );
        h.update(
            hr.completeness
                .normalized_status_flags_or
                .to_le_bytes()
                .as_slice(),
        );
        h.update(
            (hr.completeness.domains.len() as u64)
                .to_le_bytes()
                .as_slice(),
        );
        for (domain_id, domain) in &hr.completeness.domains {
            h.update(domain_id.to_le_bytes().as_slice());
            h.update(domain.data_records.to_le_bytes().as_slice());
            h.update(domain.perf_records.to_le_bytes().as_slice());
            h.update(domain.gap_records.to_le_bytes().as_slice());
            h.update(domain.gap_reason_mask.to_le_bytes().as_slice());
            h.update(domain.gap_part_mask.to_le_bytes().as_slice());
            h.update(domain.data_payload_status_flags_or.to_le_bytes().as_slice());
            h.update(domain.normalized_status_flags_or.to_le_bytes().as_slice());
            h.update(domain.lane_local_status_flags_or.to_le_bytes().as_slice());
        }
        h.update(b"\n");
    }
    to_hex(&h.finalize())
}

fn cmd_hash(args: &[String]) -> Result<u8, String> {
    let expected_roster = parse_expected_profile(args)?;
    let identity_mode = parse_stream_identity_mode(args)?;
    let (positional, out) = parse_common(args)?;
    if positional.len() != 1 {
        return Err("hash takes exactly one <input>".into());
    }
    let path = Path::new(&positional[0]);
    let (run, kind) = run_input_with_identity_mode(path, &expected_roster, identity_mode)?;
    let report = hash_report(path, kind, &run, &expected_roster, identity_mode);
    emit(&report, out.as_deref())?;
    Ok(0)
}

fn hash_report(
    path: &Path,
    kind: &str,
    run: &FoldRun,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
) -> Value {
    let mut per_domain = Map::new();
    for (dom, n) in &run.totals.per_domain_data_records {
        per_domain.insert(dom.to_string(), json!({ "data_records": n }));
    }
    let hours: Vec<Value> = run
        .hours
        .iter()
        .map(|hr| {
            let mut doms = Map::new();
            for (d, c) in &hr.domain_data_counts {
                doms.insert(d.to_string(), json!(c));
            }
            json!({
                "hour_index": hr.hour_index,
                "date": hr.date,
                "snapshot_hash": hr.snapshot_hash,
                "snapshot_bytes": hr.snapshot_bytes,
                "domain_data_counts": Value::Object(doms),
                "normalized_completeness": completeness_json(&hr.completeness),
            })
        })
        .collect();
    let range = hour_range(&run.hours);
    json!({
        "tool": "fold_check",
        "mode": "hash",
        "report_version": 5,
        "input": path.display().to_string(),
        "input_kind": kind,
        "identity_mode": stream_identity_mode_name(identity_mode),
        "game_id": FOLD_CHECK_GAME_ID,
        "expected_roster": expected_roster_json(expected_roster),
        "totals": totals_json(&run.totals),
        "hour_range": range,
        "per_domain": Value::Object(per_domain),
        "hours": hours,
        "refused_hours": refusals_json(&run.refusals),
        "chain_hash": chain_hash(&run.hours),
    })
}

const REFUSAL_DETAIL_LIMIT: usize = 64;

const FRAME_FINDING_PREDICATE_LIMIT: usize = 32;

fn frame_finding_predicates(refusals: &BTreeMap<i64, HourRefusal>) -> (usize, Vec<&str>) {
    let mut names: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for refusal in refusals.values() {
        names.extend(
            refusal
                .frame_findings
                .iter()
                .map(|finding| finding.predicate.as_str()),
        );
    }
    let total = names.len();
    (
        total,
        names
            .into_iter()
            .take(FRAME_FINDING_PREDICATE_LIMIT)
            .collect(),
    )
}

fn refusals_json(refusals: &BTreeMap<i64, HourRefusal>) -> Value {
    let mut offending_domains: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
    for refusal in refusals.values() {
        offending_domains.extend(refusal.offenders.iter().map(|offender| offender.domain_id));
    }
    let (frame_predicate_count, frame_predicates) = frame_finding_predicates(refusals);
    let entries: Vec<Value> = refusals
        .iter()
        .take(REFUSAL_DETAIL_LIMIT)
        .map(|(hour_index, refusal)| {
            let offenders: Vec<Value> = refusal
                .offenders
                .iter()
                .map(|offender| {
                    json!({
                        "domain_id": offender.domain_id,
                        "status_flags": offender.status_flags,
                        "non_tolerated_flags": offender.non_tolerated_flags,
                        "reason": offender.reason,
                        "detail": offender.detail,
                    })
                })
                .collect();
            let frame_findings: Vec<Value> = refusal
                .frame_findings
                .iter()
                .map(|finding| {
                    json!({
                        "predicate": finding.predicate,
                        "detail": finding.detail,
                    })
                })
                .collect();
            json!({
                "hour_index": hour_index,
                "wire_version": refusal.wire_version,
                "offenders": offenders,
                "frame_findings": frame_findings,
                "discarded_terminal_outcome_domains": refusal.discarded_terminal_outcome_domains,
                "discarded_data_records": refusal.discarded_data_records,
            })
        })
        .collect();
    json!({
        "count": refusals.len(),
        "hour_range": match (refusals.keys().next(), refusals.keys().next_back()) {
            (Some(first), Some(last)) => json!([first, last]),
            _ => Value::Null,
        },
        "offending_domains": offending_domains,
        "frame_finding_predicate_count": frame_predicate_count,
        "frame_finding_predicates": frame_predicates,
        "frame_finding_predicate_limit": FRAME_FINDING_PREDICATE_LIMIT,
        "hours_detailed": entries.len(),
        "hours_detail_limit": REFUSAL_DETAIL_LIMIT,
        "hours": entries,
    })
}

fn expected_roster_json(expected: &ExpectedRoster) -> Value {
    let domains: Vec<Value> = expected
        .domains
        .iter()
        .map(|(domain_id, domain)| {
            json!({
                "domain_id": domain_id,
                "wire_name": domain.wire_name,
                "requires_data_record": domain.requires_data_record,
                "requires_perf_record": true,
            })
        })
        .collect();
    json!({
        "profile": expected.profile,
        "domain_count": expected.domains.len(),
        "domains": domains,
    })
}

fn completeness_json(completeness: &HourCompleteness) -> Value {
    let mut domains = Map::new();
    for (domain_id, domain) in &completeness.domains {
        domains.insert(
            domain_id.to_string(),
            json!({
                "state": domain.state(*domain_id),
                "complete": domain.is_complete(*domain_id),
                "data_records": domain.data_records,
                "perf_records": domain.perf_records,
                "gap_records": domain.gap_records,
                "gap_reasons": gap_reason_names(domain.gap_reason_mask),
                "gap_parts": gap_part_names(domain.gap_part_mask),
                "excluded_data_records": domain.excluded_data_records,
                "excluded_data_reasons": domain.excluded_data_reasons,
                "data_payload_status_flags_or": domain.data_payload_status_flags_or,
                "normalized_status_flags_or": domain.normalized_status_flags_or,
                "lane_local_status_flags_or": domain.lane_local_status_flags_or,
            }),
        );
    }
    let declared_degraded: Map<String, Value> = completeness
        .declared_degraded_domains
        .iter()
        .map(|(domain_id, (status_flags, non_tolerated_flags))| {
            (
                domain_id.to_string(),
                json!({
                    "status_flags": status_flags,
                    "non_tolerated_flags": non_tolerated_flags,
                    "non_tolerated_flag_names":
                        compact_contract::compact_status_flag_names(*non_tolerated_flags),
                }),
            )
        })
        .collect();
    json!({
        "frame_emitted_domains": completeness.frame_emitted_domains,
        "frame_scheduled_domains": completeness.frame_scheduled_domains,
        "normalized_frame_status_flags": completeness.normalized_frame_status_flags,
        "normalized_status_flags_or": completeness.normalized_status_flags_or,
        "declared_degraded_domains": Value::Object(declared_degraded),
        "domains": Value::Object(domains),
    })
}

fn totals_json(t: &Totals) -> Value {
    let mut v = json!({
        "stream_bytes": t.stream_bytes,
        "bytes_folded": t.bytes_folded,
        "truncated_tail_bytes": t.truncated_tail_bytes,
        "records_total": t.records_total,
        "data_records": t.data_records,
        "malformed_payload_records": t.malformed_payload_records,
        "excluded_domain_data": replay_decoder::excluded_domain_data_json(
            &t.excluded_domain_data_records,
        ),
        "frame_end_records": t.frame_end_records,
        "perf_records_ignored": t.perf_records,
        "other_records_ignored": t.other_records,
        "hours_emitted": t.hours_emitted,
        "ends_at_frame_end": t.ends_at_frame_end,
    });
    if let Some(reason) = &t.stop_reason {
        v.as_object_mut()
            .unwrap()
            .insert("framing_stopped".into(), json!(reason));
    }
    if let Some(reason) = &t.boundary_error {
        v.as_object_mut()
            .unwrap()
            .insert("boundary_error".into(), json!(reason));
    }
    v
}

fn hour_range(hours: &[HourReport]) -> Value {
    match (hours.first(), hours.last()) {
        (Some(f), Some(l)) => json!([f.hour_index, l.hour_index]),
        _ => Value::Null,
    }
}

fn cmd_compare(args: &[String]) -> Result<u8, String> {
    let expected_roster = parse_expected_profile(args)?;
    let identity_mode = parse_stream_identity_mode(args)?;
    let (positional, out) = parse_common(args)?;
    let sample = parse_sample(args)?.unwrap_or(24);
    if positional.len() != 2 {
        return Err("compare takes exactly two inputs <a> <b>".into());
    }
    let path_a = Path::new(&positional[0]);
    let path_b = Path::new(&positional[1]);
    let (run_a, kind_a) = run_input_with_identity_mode(path_a, &expected_roster, identity_mode)?;
    let (run_b, kind_b) = run_input_with_identity_mode(path_b, &expected_roster, identity_mode)?;

    let first_divergence = first_divergence(
        &run_a,
        &run_b,
        path_a,
        path_b,
        sample,
        &expected_roster,
        identity_mode,
    )?;

    let identical = first_divergence.is_none();
    let report = json!({
        "tool": "fold_check",
        "mode": "compare",
        "report_version": 5,
        "identical": identical,
        "identity_mode": stream_identity_mode_name(identity_mode),
        "expected_roster": expected_roster_json(&expected_roster),
        "a": side_summary(path_a, kind_a, &run_a),
        "b": side_summary(path_b, kind_b, &run_b),
        "hour_count_match": run_a.hours.len() == run_b.hours.len(),
        "first_divergence": first_divergence.unwrap_or(Value::Null),
    });
    emit(&report, out.as_deref())?;
    Ok(if identical { 0 } else { 1 })
}

fn first_divergence(
    run_a: &FoldRun,
    run_b: &FoldRun,
    path_a: &Path,
    path_b: &Path,
    sample: usize,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
) -> Result<Option<Value>, String> {
    let map_a: BTreeMap<i64, &HourReport> = run_a.hours.iter().map(|h| (h.hour_index, h)).collect();
    let map_b: BTreeMap<i64, &HourReport> = run_b.hours.iter().map(|h| (h.hour_index, h)).collect();

    let mut all_hours: Vec<i64> = map_a.keys().chain(map_b.keys()).copied().collect();
    all_hours.sort_unstable();
    all_hours.dedup();

    let mut first_divergence: Option<Value> = None;
    for h in &all_hours {
        match (map_a.get(h), map_b.get(h)) {
            (Some(a), Some(b)) => {
                if a.snapshot_hash != b.snapshot_hash {
                    first_divergence = Some(divergence_hash_mismatch(
                        *h,
                        a,
                        b,
                        path_a,
                        path_b,
                        sample,
                        expected_roster,
                        identity_mode,
                    )?);
                    break;
                } else if a.completeness != b.completeness {
                    first_divergence = Some(json!({
                        "hour_index": h,
                        "kind": "normalized_completeness_mismatch",
                        "a_hash": a.snapshot_hash,
                        "b_hash": b.snapshot_hash,
                        "a_normalized_completeness": completeness_json(&a.completeness),
                        "b_normalized_completeness": completeness_json(&b.completeness),
                    }));
                    break;
                }
            }
            (Some(a), None) => {
                first_divergence = Some(json!({
                    "hour_index": h,
                    "kind": "missing_in_b",
                    "a_hash": a.snapshot_hash,
                    "b_hash": Value::Null,
                }));
                break;
            }
            (None, Some(b)) => {
                first_divergence = Some(json!({
                    "hour_index": h,
                    "kind": "missing_in_a",
                    "a_hash": Value::Null,
                    "b_hash": b.snapshot_hash,
                }));
                break;
            }
            (None, None) => {}
        }
    }

    if first_divergence.is_none() {
        first_divergence = refusal_divergence(&run_a.refusals, &run_b.refusals);
    }
    Ok(first_divergence)
}

fn refusal_divergence(
    a: &BTreeMap<i64, HourRefusal>,
    b: &BTreeMap<i64, HourRefusal>,
) -> Option<Value> {
    if a == b {
        return None;
    }
    let only_a: Vec<i64> = a
        .keys()
        .filter(|hour| !b.contains_key(hour))
        .copied()
        .collect();
    let only_b: Vec<i64> = b
        .keys()
        .filter(|hour| !a.contains_key(hour))
        .copied()
        .collect();
    Some(json!({
        "kind": "refused_hour_mismatch",
        "a_refused_hours": a.len(),
        "b_refused_hours": b.len(),
        "refused_only_in_a": only_a,
        "refused_only_in_b": only_b,
    }))
}

fn side_summary(path: &Path, kind: &str, run: &FoldRun) -> Value {
    json!({
        "input": path.display().to_string(),
        "input_kind": kind,
        "hours": run.hours.len(),
        "hour_range": hour_range(&run.hours),
        "refused_hours": refusals_json(&run.refusals),
        "chain_hash": chain_hash(&run.hours),
        "totals": totals_json(&run.totals),
    })
}

fn divergence_hash_mismatch(
    hour: i64,
    a: &HourReport,
    b: &HourReport,
    path_a: &Path,
    path_b: &Path,
    sample: usize,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
) -> Result<Value, String> {
    let bytes_a = capture_hour(path_a, hour, expected_roster, identity_mode)?;
    let bytes_b = capture_hour(path_b, hour, expected_roster, identity_mode)?;
    let (section_diffs, sections) = match (bytes_a, bytes_b) {
        (Some(ba), Some(bb)) => {
            let va: Value = serde_json::from_slice(&ba)
                .map_err(|e| format!("parse snapshot a hour {hour}: {e}"))?;
            let vb: Value = serde_json::from_slice(&bb)
                .map_err(|e| format!("parse snapshot b hour {hour}: {e}"))?;
            let mut diffs: Vec<Value> = Vec::new();
            diff_values(&va, &vb, "", &mut diffs, sample);
            (Value::Array(diffs), section_summary(&va, &vb))
        }
        _ => (Value::Null, Value::Null),
    };

    let mut domain_diffs = Map::new();
    let doms: std::collections::BTreeSet<u16> = a
        .domain_data_counts
        .keys()
        .chain(b.domain_data_counts.keys())
        .copied()
        .collect();
    for d in doms {
        let ca = a.domain_data_counts.get(&d).copied().unwrap_or(0);
        let cb = b.domain_data_counts.get(&d).copied().unwrap_or(0);
        if ca != cb {
            domain_diffs.insert(d.to_string(), json!({ "a": ca, "b": cb }));
        }
    }

    Ok(json!({
        "hour_index": hour,
        "kind": "hash_mismatch",
        "date": a.date,
        "a_hash": a.snapshot_hash,
        "b_hash": b.snapshot_hash,
        "a_snapshot_bytes": a.snapshot_bytes,
        "b_snapshot_bytes": b.snapshot_bytes,
        "a_normalized_completeness": completeness_json(&a.completeness),
        "b_normalized_completeness": completeness_json(&b.completeness),
        "domain_data_count_diffs": Value::Object(domain_diffs),
        "sections": sections,
        "section_diffs_truncated_at": sample,
        "section_diffs": section_diffs,
    }))
}

fn capture_hour(
    path: &Path,
    target: i64,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
) -> Result<Option<Vec<u8>>, String> {
    let mut captured: Option<Vec<u8>> = None;
    fold_input(
        path,
        expected_roster,
        identity_mode,
        |hour_index, _date, bytes, _counts, _completeness| {
            if hour_index == target && captured.is_none() {
                captured = Some(bytes.to_vec());
            }
        },
    )?;
    Ok(captured)
}

fn section_summary(a: &Value, b: &Value) -> Value {
    let empty = Map::new();
    let am = a.as_object().unwrap_or(&empty);
    let bm = b.as_object().unwrap_or(&empty);
    let keys: std::collections::BTreeSet<&String> = am.keys().chain(bm.keys()).collect();
    let mut out = Map::new();
    for k in keys {
        let av = am.get(k);
        let bv = bm.get(k);
        let equal = av == bv;
        if !equal {
            out.insert(
                k.clone(),
                json!({
                    "a_rows": value_len(av),
                    "b_rows": value_len(bv),
                    "equal": false,
                }),
            );
        }
    }
    Value::Object(out)
}

fn value_len(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => json!(0),
        Some(Value::Array(a)) => json!(a.len()),
        Some(Value::Object(o)) => json!(o.len()),
        Some(_) => json!(1),
    }
}

fn diff_values(a: &Value, b: &Value, path: &str, out: &mut Vec<Value>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            let keys: std::collections::BTreeSet<&String> = am.keys().chain(bm.keys()).collect();
            for k in keys {
                if out.len() >= limit {
                    return;
                }
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}/{k}")
                };
                match (am.get(k), bm.get(k)) {
                    (Some(x), Some(y)) => diff_values(x, y, &child, out, limit),
                    (Some(x), None) => out.push(json!({
                        "path": child, "kind": "only_in_a", "a": preview(x), "b": Value::Null,
                    })),
                    (None, Some(y)) => out.push(json!({
                        "path": child, "kind": "only_in_b", "a": Value::Null, "b": preview(y),
                    })),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(aa), Value::Array(ba)) => {
            if aa.len() != ba.len() {
                out.push(json!({
                    "path": path, "kind": "array_len", "a": aa.len(), "b": ba.len(),
                }));
            }
            let n = aa.len().min(ba.len());
            for i in 0..n {
                if out.len() >= limit {
                    return;
                }
                diff_values(&aa[i], &ba[i], &format!("{path}[{i}]"), out, limit);
            }
        }
        _ => {
            if a != b {
                out.push(json!({
                    "path": path, "kind": "value", "a": preview(a), "b": preview(b),
                }));
            }
        }
    }
}

fn preview(v: &Value) -> Value {
    match v {
        Value::Array(a) => json!(format!("<array len {}>", a.len())),
        Value::Object(o) => json!(format!("<object len {}>", o.len())),
        Value::String(s) if s.len() > 120 => json!(format!("{}…({} chars)", &s[..117], s.len())),
        other => other.clone(),
    }
}

fn parse_common(args: &[String]) -> Result<(Vec<String>, Option<String>), String> {
    let mut positional = Vec::new();
    let mut out = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out = Some(args.get(i).ok_or("--out requires a file path")?.clone());
            }
            "--sample" => {
                i += 1;
            }
            "--expected-profile" => {
                i += 1;
            }
            "--identity-mode" => {
                i += 1;
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {other}"));
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    Ok((positional, out))
}

fn parse_stream_identity_mode(args: &[String]) -> Result<StreamIdentityMode, String> {
    let mut selected: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--identity-mode" {
            let value = args
                .get(i + 1)
                .ok_or("--identity-mode requires legacy or identity-v1")?;
            if selected.replace(value.as_str()).is_some() {
                return Err("--identity-mode may be specified only once".to_string());
            }
            i += 1;
        }
        i += 1;
    }
    match selected {
        None | Some("legacy") => Ok(StreamIdentityMode::Legacy),
        Some("identity-v1") => Ok(StreamIdentityMode::IdentityV1),
        Some(other) => Err(format!(
            "unsupported stream identity mode {other:?}; expected legacy or identity-v1"
        )),
    }
}

fn parse_expected_profile(args: &[String]) -> Result<ExpectedRoster, String> {
    let mut selected: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--expected-profile" {
            let value = args
                .get(i + 1)
                .ok_or("--expected-profile requires a profile name")?;
            if selected.replace(value.as_str()).is_some() {
                return Err("--expected-profile may be specified only once".to_string());
            }
            i += 1;
        }
        i += 1;
    }
    match selected {
        Some("production") => Ok(ExpectedRoster::production()),
        Some(other) => Err(format!(
            "unsupported expected profile {other:?}; only \"production\" is authoritative"
        )),
        None => Err(
            "missing mandatory --expected-profile production; FRAME_END self-reported counts are not a coverage authority"
                .to_string(),
        ),
    }
}

fn parse_sample(args: &[String]) -> Result<Option<usize>, String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--sample" {
            let v = args.get(i + 1).ok_or("--sample requires a count")?;
            return v
                .parse::<usize>()
                .map(Some)
                .map_err(|_| format!("--sample expects a non-negative integer, got {v:?}"));
        }
        i += 1;
    }
    Ok(None)
}

fn emit(report: &Value, out: Option<&str>) -> Result<(), String> {
    let mut text =
        serde_json::to_string_pretty(report).map_err(|e| format!("serialize report: {e}"))?;
    text.push('\n');
    match out {
        Some(path) => {
            File::create(path)
                .and_then(|mut f| f.write_all(text.as_bytes()))
                .map_err(|e| format!("write {path}: {e}"))?;
        }
        None => {
            io::stdout()
                .write_all(text.as_bytes())
                .map_err(|e| format!("write stdout: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
