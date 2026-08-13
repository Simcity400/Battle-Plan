//! `fold_check` — the fold-equivalence gate (G1) for the extraction pipeline.
//!
//! Given a collector compact record stream (`collector_stream.bin`) or a
//! `.battleplan` container, it decodes every record and drives the **existing
//! production fold** ([`recorder_host::world_fold::WorldFold`]) to produce the
//! per-game-hour world snapshots, then emits a canonical SHA-256 plus normalized
//! status/completeness evidence per hour.
//!
//! There is exactly ONE decode+fold in the codebase. This tool now reuses it
//! through the shared [`replay_decoder`] facade: `replay_decoder` frames the
//! `H4CR` records (via the single `compact_contract::parse_compact_record_header`
//! primitive), drives the production `recorder_host::world_fold::WorldFold`, and
//! yields each game-hour's canonical snapshot bytes in bounded memory. `fold_check`
//! just hashes those bytes. Folding under the fixed [`FOLD_CHECK_GAME_ID`] makes a
//! raw stream and the same stream packed in a container hash identically, so an
//! optimized collector can be certified byte-identical (Extraction Optimization
//! Plan, gate G1).
//!
//! Usage:
//!   fold_check hash    <input> --expected-profile production [--identity-mode legacy|identity-v1] [--out report.json]
//!   fold_check compare <a> <b> --expected-profile production [--identity-mode legacy|identity-v1] [--out report.json] [--sample N]
//!
//! `<input>` is auto-detected: a stream begins with the `H4CR` magic; a container
//! begins with the ZIP local-file magic and its `collector_compact_stream` entry
//! is extracted and folded.
//!
//! `hash` prints a deterministic JSON report (per-hour hash + per-domain record
//! counts + totals + a chain hash) to stdout. `compare` exits 0 iff both inputs
//! fold to the identical hour range with identical per-hour hashes and
//! authoritative completeness; on divergence it reports the first divergent
//! hour with a bounded per-section drill-down.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
use compact_contract::frame_refusal::FrameRefusalRecordV2;
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance,
    compact_status_for_global_interpretation, compact_status_is_failure, lane_local_status_mask,
    parse_compact_record_header, validate_compact_record_semantics, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_CAPTURE_TERMINATED, COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL,
    COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_GAP,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_DEGRADED,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_TYPE_FRAME_REFUSED,
};
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode, StreamValidationError,
};
use replay_decoder::{DecodeStats, OpenOptions, Source};
use serde_json::{json, Map, Value};
use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::{to_hex, Sha256};

/// Fixed synthetic game id so the folded snapshot's `game_id` field (and thus the
/// hash) depends only on the record stream, never on the file path. Two files
/// with identical stream bytes therefore hash identically — the whole point.
const FOLD_CHECK_GAME_ID: &str = "fold_check";

/// The decode options every `fold_check` run uses: fold under the fixed
/// [`FOLD_CHECK_GAME_ID`] regardless of source, so a raw stream and the same
/// stream packed in a container hash identically.
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

// ---------------------------------------------------------------------------
// Shared: fold-run accounting
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DomainCompleteness {
    data_records: u64,
    perf_records: u64,
    /// `DOMAIN_GAP` records for this domain in this hour. A gap is the domain's
    /// TERMINAL OUTCOME, exactly as a PERF is
    /// (`compact_record_counts_toward_terminal_outcome`) — it is an ACCOUNTED
    /// ABSENCE, not a missing record. Counting it separately is what lets the
    /// roster verdict say "this domain was accounted absent for reason R"
    /// instead of the false "expected PERF record count 1, observed 0".
    gap_records: u64,
    /// OR of `1 << DomainGapReason::code()` over this hour's gaps, and likewise
    /// for the affected part. Kept as bitmasks so the identity chain hash stays
    /// a fixed-width, order-independent function of the accounted absence.
    gap_reason_mask: u32,
    gap_part_mask: u32,
    /// Normalized OR of the status each DOMAIN_DATA payload carried INSIDE
    /// ITSELF. Kept beside — not merged into — the PERF-derived status, because
    /// the two answering differently is exactly the condition that let an hour
    /// certify while the fold withheld the domain's rows. The report shows both
    /// so a reader can see they agree.
    data_payload_status_flags_or: u32,
    normalized_status_flags_or: u32,
    lane_local_status_flags_or: u32,
    /// Failure-class bits no expected-partial tolerance covers, i.e. this
    /// domain is UNKNOWN for the hour. Nonzero here is only certifiable when
    /// the frame's `H4FD` notice declares this domain with exactly these bits;
    /// see `CompletenessAudit::observe`.
    ///
    /// PERF-derived, and kept that way. The collector mints its `H4FD` entries
    /// from the PERF status, so this is the word the declaration is checked
    /// against for EQUALITY.
    non_tolerated_status_flags: u32,
    /// The same classification applied to the status embedded in this domain's
    /// `DOMAIN_DATA` payload, kept SEPARATE from the PERF-derived word above.
    ///
    /// Merging the two loses the disagreement between them, and the merge is
    /// not conservative in the direction one would hope: a domain whose PERF
    /// says `partial_data` and whose payload says `source_unavailable` would
    /// present a union of `0x00000300` that a notice could then declare, so the
    /// hour would certify against a word NEITHER record carries. Separate fields
    /// let the reconciliation state the two rules it actually means -- the
    /// declaration EQUALS the PERF residual, and it COVERS the payload residual
    /// -- neither of which the union can express.
    non_tolerated_data_payload_status_flags: u32,
    /// `DOMAIN_DATA` records for this domain in this hour that the decoder's
    /// payload authority REJECTED, so `WorldFold` applied none of their rows.
    ///
    /// Joined in from the decoder's exclusion ledger by (hour, domain); it is
    /// never re-decided here. The ledger has exactly one author — the decode
    /// pass, which is also the pass that withholds the rows — and the audit's
    /// own `domain_data_payload_status` call cannot stand in for it: that call
    /// passes a zero sequence, hour and offset, so a payload whose validity
    /// depends on its envelope would be classified differently by the two.
    /// One rule, one owner; this field carries the owner's answer.
    ///
    /// Nonzero here is the whole point of the join. The record is well-framed
    /// and counted, its PERF is clean, the collector mints its `H4FD` notice
    /// FROM that clean PERF, and a payload that does not decode yields no
    /// status word to classify — so before this field existed, every signal the
    /// report carried said the domain was complete while the fold held nothing.
    excluded_data_records: u64,
    /// The ledger's OWN reason strings for those exclusions, carried through
    /// verbatim so the report says WHY the domain is incomplete and not merely
    /// that it is.
    ///
    /// A set, not a list: it is keyed by nothing and joined on nothing, so
    /// deduplicating identical reasons keeps it bounded by the number of
    /// DISTINCT decoder verdicts rather than by capture length, and the
    /// `BTreeSet` ordering keeps the rendered report deterministic. The reason
    /// text is deliberately never used as a join key — the join is on the
    /// ledger entry's own (hour, domain) identity.
    excluded_data_reasons: std::collections::BTreeSet<String>,
}

impl DomainCompleteness {
    /// Exactly one PERF or DOMAIN_GAP exhausts each scheduled domain, so this is
    /// the count that the FRAME_END seal and the expected roster are checked
    /// against.
    fn terminal_outcomes(&self) -> u64 {
        self.perf_records.saturating_add(self.gap_records)
    }

    /// What state this domain is in for this hour, named in the SHARED
    /// vocabulary rather than a private one.
    ///
    /// `compact_contract::frame_degraded` owns the three-state classifier —
    /// complete / expected-partial / degraded-unknown — and says so explicitly:
    /// "the collector decides what to publish with it, the fold decides what to
    /// apply with it, and the certifier decides what to accept with it, so the
    /// three cannot drift into disagreeing". This certifier is that third
    /// reader, so it CALLS that classifier instead of restating it. An earlier
    /// draft of this function answered with its own boolean and collapsed
    /// expected-partial into complete, which is precisely the mislabel that
    /// module warns is worse than the deadlock it was built to solve.
    ///
    /// Two states sit OUTSIDE that classifier because they are not status-word
    /// facts at all, and both mean the rows never reached the fold:
    ///
    ///  * `absent`   — a `DOMAIN_GAP`: the collector accounted the domain's
    ///    extraction as missing for this hour.
    ///  * `excluded` — a `DOMAIN_DATA` payload the decoder's authority rejected
    ///    and wrote to the exclusion ledger (the join this lane added).
    ///
    /// They are checked first because a record that never reached the fold is a
    /// stronger statement than any status word carried alongside it. The report
    /// publishes the raw counts next to this name, so a domain that is both
    /// excluded and degraded still shows both.
    fn state(&self, domain_id: u16) -> &'static str {
        if self.gap_records != 0 {
            return "absent";
        }
        if self.excluded_data_records != 0 {
            return "excluded";
        }
        // Degraded is decided on the two residual words the `FRAME_END`
        // reconciliation itself uses — the PERF's and the payload's, kept apart
        // for the reasons `non_tolerated_data_payload_status_flags` documents.
        // Either one being nonzero means rows were withheld.
        if self.non_tolerated_status_flags != 0 || self.non_tolerated_data_payload_status_flags != 0
        {
            return "degraded_unknown";
        }
        // Neither residual survived tolerance, so the only remaining question is
        // whether tolerance was ENGAGED — expected-partial versus complete —
        // and it must be asked of BOTH status sources. Asking only the PERF word
        // was a real defect: a domain whose PERF is clean while its own
        // DOMAIN_DATA payload carries a TOLERATED partial gets no `H4FD` notice
        // (the notice is minted from the PERF) and clears the payload coverage
        // loop at `FRAME_END` (its residual is zero), so nothing else would stop
        // it being reported complete while its rows are a sparse patch.
        //
        // `normalized_status_flags_or` is the union of both sources, and on this
        // branch the union answers for both EXACTLY rather than approximately.
        // A zero residual means a source carries no failure bits at all, or only
        // tolerated ones (the helper returns the whole failure word when no
        // tolerance applies, so a domain without an entry cannot reach here with
        // any failure bit set). Both residuals are zero here, so the union is
        // itself entirely tolerated and cannot classify as degraded; and since
        // `expected_partial_tolerance_applies` tests
        // `failure_bits & tolerated != 0`, the union engages the tolerance iff at
        // least one source did. Normalization is a bitmask, so it distributes
        // over the OR and the union is normalized exactly as each part was.
        // Classifying the union is therefore the more-conservative-of-the-two
        // answer — and it needs no new field, so the report keeps exactly the
        // per-domain evidence its identity hash already covers.
        //
        // The union is safe to pass as stored: the classifier's first act is to
        // strip lane-local bits through the same normalizer this field was built
        // with. (A union is deliberately NOT used for the `H4FD` reconciliation
        // at `FRAME_END`, where it could certify a word neither record carried;
        // that is a different question, and it is asked there.)
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

    /// Did everything this hour scheduled for the domain actually reach the
    /// fold? True for `complete` alone — an expected-partial is a reviewed
    /// SPARSE patch, not a complete one, and the shared vocabulary keeps the two
    /// apart by construction.
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

/// A reason missing from `gap_reason_names` is not a compile error -- the table
/// is a list, not a `match` -- so it would silently drop out of every report
/// while the mask still carried its bit.
///
/// Driven by `DomainGapReason::ALL` rather than by a second hand-written list:
/// a list checked against a list is satisfied by forgetting the variant twice.
/// `ALL` is itself pinned complete by an exhaustive match in `compact_contract`,
/// so a new variant reaches this test whether or not anyone remembered it.
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

/// One offending domain named by a `FRAME_REFUSED` (`H4FR`) record, retained
/// verbatim: the refusal was decided on these bits, so the report must show
/// them rather than a count.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RefusalOffender {
    domain_id: u16,
    status_flags: u32,
    non_tolerated_flags: u32,
    reason: String,
    /// The offender rendered with everything the record carries: predicate,
    /// authority, locator path, typed units, truncation accounting, off-frame
    /// outcome.
    ///
    /// The record moved the deciding facts OUT of the reason string and into
    /// typed fields, so a certifier that reports only `reason` reports strictly
    /// less than the producer said.
    ///
    /// Kept as an `Option` because the JSON boundary distinguishes "no typed
    /// diagnosis" from an empty one, and a future version that carries no
    /// rendering must be able to say so rather than emit a rendering of
    /// nothing.
    detail: Option<String>,
}

/// One reason a `FRAME_REFUSED` attributed to the frame rather than to any
/// domain.
///
/// The predicate NAME is kept apart from the rendering because a rendering
/// carries the finding's free-form reason and its locator ordinals and is
/// therefore distinct in every hour, while names repeat. The union over a
/// capture is taken on names for that reason.
///
/// Neither is bounded by the WIRE, though: the contract bounds a name at 64
/// bytes and a record at 256 findings, but nothing constrains a producer to a
/// fixed vocabulary, so a stream that invents a name per hour would grow the
/// union linearly with the refused-hour count. The union is therefore reported
/// the same way the per-hour detail is -- exact count, bounded list, and the
/// report says where it stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FrameFinding {
    predicate: String,
    detail: String,
}

/// One game-hour that was assembled and REFUSED. A refusal is a TERMINATED
/// FRAME: the hour existed, was attempted, and closed without publishing. It
/// therefore never becomes a folded snapshot, and it is never silently absent
/// either — it appears here so the report can say the hour was attempted and
/// why it did not publish.
#[derive(Clone, Debug, PartialEq, Eq)]
struct HourRefusal {
    /// The `H4FR` version the record DECLARED, kept because a report that does
    /// not say which shape it read cannot be checked against the bytes.
    wire_version: u16,
    offenders: Vec<RefusalOffender>,
    /// Reasons the refusal attributed to the FRAME rather than to any domain --
    /// its own clock, its identity, a framing fault found while assembling it.
    ///
    /// A refusal may legally name ZERO offenders and substantiate itself
    /// entirely from here, so a certifier that read only `offenders` would
    /// record that hour as refused and be unable to say why -- exactly the
    /// unaccounted-cause failure this record exists to prevent.
    frame_findings: Vec<FrameFinding>,
    /// Domains that had already produced a terminal outcome in the hour the
    /// refusal terminated. A refusal that discards accumulated outcomes is
    /// still a truthful close, but the report must not pretend the hour was
    /// empty.
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
    /// Domains this hour's `H4FD` notice declared UNKNOWN, and the exact
    /// non-tolerated bits it declared them with. Empty for a clean hour.
    ///
    /// This is the certifier's record of "the producer told me not to trust
    /// these": present here means the domain is UNKNOWN for the hour, and the
    /// bits say what the producer declared it unknown with. Absent but carrying
    /// tolerated failure bits means expected-partial.
    ///
    /// It is NOT the whole completeness answer, and reading it as if it were is
    /// the defect the exclusion-ledger join closed. This map is derived from the
    /// producer's `H4FD` notice, which the collector mints from the PERF status;
    /// a `DOMAIN_DATA` payload the decoder could not decode at all has a clean
    /// PERF, therefore no notice, therefore no entry here — while `WorldFold`
    /// applied none of its rows. `DomainCompleteness::is_complete` is the one
    /// place that answers "is this domain complete for this hour", and the
    /// report publishes its answer per domain so no consumer re-derives it.
    declared_degraded_domains: BTreeMap<u16, u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedDomain {
    wire_name: String,
    requires_data_record: bool,
}

/// A caller-selected authority that is independent of the stream being
/// certified. The stream may report its own scheduled/emitted counts, but it
/// cannot define what production was expected to contain.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedRoster {
    profile: String,
    domains: BTreeMap<u16, ExpectedDomain>,
}

impl ExpectedRoster {
    fn production() -> Self {
        // Production-profile status and passive-product admissibility are two
        // different questions (`PASSIVE_PRODUCTION_REFUSED_DOMAIN_IDS`): d71,
        // d82 and d87 hold production status but their producers need owners
        // the shipped passive product does not install, so the collector
        // refuses them at attach and `domain_roster` never schedules them.
        // The contract's own rule is that ANYTHING building a passive-product
        // roster must filter through `passive_production_admits` — including
        // this expectation. Without the filter, certification demanded a
        // 71-domain frame no passive capture can produce and refused every
        // live capture on the roster count alone (t26 and t27 both).
        //
        // If a non-passive product (authenticated early-render owners) ever
        // ships, its captures need their own expected profile rather than a
        // widening of this one.
        let domains = compact_contract::production_profile_domains()
            .into_iter()
            .filter(|descriptor| compact_contract::passive_production_admits(descriptor.id))
            .map(|descriptor| {
                (
                    descriptor.id,
                    ExpectedDomain {
                        wire_name: descriptor.wire_name.to_string(),
                        requires_data_record: descriptor.requires_data_record,
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
        // A DOMAIN_GAP is this domain's terminal outcome for the hour, so the
        // hour is not short a record — it is short the EXTRACTION. Say that,
        // with the reason the collector recorded, rather than reporting a
        // PERF-count shortfall that would send a reader looking for a lost
        // record. The hour still cannot certify: an accounted absence is an
        // absence, and the production profile requires an extracted outcome.
        if gap_records != 0 {
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
    hours: BTreeMap<i64, HourCompleteness>,
    /// Hours that closed WITHOUT publishing. Kept separate from `hours`
    /// precisely because a refused hour has no folded snapshot to compare — but
    /// dropping it would make an attempted-and-refused hour indistinguishable
    /// from an hour the collector never reached.
    refusals: BTreeMap<i64, HourRefusal>,
    current_domains: BTreeMap<u16, DomainCompleteness>,
    /// The open hour's `H4FD` declaration, if it has carried one yet. The notice
    /// rides immediately before `FRAME_END`, so it is always populated (or
    /// legitimately absent) by the time the hour closes and the two sides are
    /// compared.
    current_declared_degraded: BTreeMap<u16, u32>,
}

impl CompletenessAudit {
    fn observe(
        &mut self,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) -> Result<(), String> {
        validate_compact_record_semantics(domain_id, record_type, record_flags, payload)?;
        match record_type {
            COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                // A DOMAIN_DATA payload carries its own status, and that is the
                // status `WorldFold` actually decides on: 35 of its 90 fold
                // functions read it straight out of the payload and return
                // WITHOUT applying any row when it is a failure. Certification
                // used to read only the matching DOMAIN_PERF record, so a
                // payload declaring failure under a clean PERF certified as a
                // complete hour while the fold had silently withheld every row
                // of that domain.
                //
                // One classifier for all 73 domains, reached through the single
                // payload authority that already decodes each one — the same
                // call the decoder's `validate_domain_data_payload` makes, which
                // simply threw the status away. `None` means this domain's
                // authority decodes no status word; that is not "clean" and is
                // not treated as clean.
                //
                // Payload VALIDITY is deliberately NOT judged here. It has one
                // authority: the decoder pass counts every payload that fails
                // exact validation and writes it into the exclusion ledger, so a
                // payload this call cannot decode yields no status and is
                // accounted there. Re-deciding it here would be a second copy of
                // that rule.
                //
                // `unwrap_or(None)` therefore reduces a payload the authority
                // rejects to "no status" -- e.g. a valid header carrying
                // `source_unavailable` plus one trailing byte. That is correct
                // HERE (validity is not this call's question) but it used to be
                // the end of the story: `DecodeStats::certification_verdict`
                // does not refuse a stream on the malformed count, it only checks
                // that the count and the ledger AGREE
                // (`validate_excluded_domain_data_accounting`), so if such a
                // record's PERF was clean the hour certified with the domain
                // reported COMPLETE while the fold applied none of its rows.
                //
                // CLOSED by the owner's 2026-08-06 ruling, and closed the way the
                // ruling asked: NOT by refusing here. An excluded payload is
                // already accounted -- domain, hour, offset and decoder reason
                // are in the ledger the report prints -- and refusing would turn
                // every capture carrying one malformed record into an
                // uncertifiable capture, the opposite of the per-domain
                // degradation the zero-data-loss ruling asked for. Instead the
                // ledger is JOINED into per-hour completeness by (hour, domain),
                // so the hour still seals and simply stops claiming a
                // completeness it does not have. See `attribute_exclusion_ledger`
                // and `DomainCompleteness::is_complete`.
                //
                // A non-tolerated failure found here is DEFERRED to the frame
                // close, where the `H4FD` notice is checked to COVER it -- the
                // same place, and the same notice, the PERF arm below defers to.
                // It was refused on the spot, and that made this arm a SECOND,
                // stricter answer to a question the reconciliation already owns.
                // The two disagreed in production: t26 publishes d86 with
                // `source_unavailable` in the payload, the identical word in the
                // matching PERF, and an `H4FD` notice naming domain 86 with
                // exactly that bit and the reader's typed reason. That is an
                // ACCOUNTED degraded domain -- the fold withholds its rows and
                // retains its accumulator, which is precisely what the notice
                // declares -- and the hour certifies with the domain recorded
                // unknown. Refusing it here made the same status word certifiable
                // through one record type and fatal through the other, so d18,
                // d47, d53, d60, d85 and d90 certified as declared-degraded on
                // the very hours d86 refused the whole stream.
                //
                // Deferring is not a relaxation, and specifically it does not
                // restore the defect this check was added for. The residual is
                // carried to the frame close in its OWN field -- never merged
                // into the PERF's -- where a payload failure the notice does not
                // cover refuses with a verdict that names it. A payload declaring
                // failure under a clean PERF has no `H4FD` entry at all, because
                // the collector derives the notice from the PERF status, so it
                // refuses: the original finding, stated by its owner.
                let payload_status =
                    replay_decoder::domain_data_payload_status(domain_id, payload, record_flags)
                        .unwrap_or(None);
                let residual = payload_status.map_or(0, |status_flags| {
                    compact_status_failure_flags_after_expected_partial_tolerance(
                        domain_id,
                        status_flags,
                    )
                });
                // An unassigned id has no domain descriptor, so it can never be
                // declared degraded and there is nothing to defer to. Same
                // reasoning, same verdict, as the PERF arm's probe case.
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
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                let status_flags = read_u32_at(payload, 32, "DOMAIN_PERF status")?;
                let normalized = compact_status_for_global_interpretation(domain_id, status_flags);
                let residual = compact_status_failure_flags_after_expected_partial_tolerance(
                    domain_id,
                    status_flags,
                );
                // A non-tolerated failure is NOT accepted here and NOT accepted
                // silently. It is DEFERRED to the frame close, where the only
                // thing that can certify it is the frame's own `H4FD` notice
                // declaring this domain with exactly these bits.
                //
                // Deferring is not a relaxation. An hour that carries a
                // non-tolerated failure and no matching declaration fails with
                // the same verdict it always did -- see the FRAME_END arm. What
                // deferring buys is the ability to distinguish "the producer
                // published a defect and told me" from "the producer published a
                // defect and did not", which is precisely the distinction that
                // decides whether a degraded frame is honest or dangerous. At a
                // PERF record that distinction is not yet knowable, because the
                // notice has not been read.
                //
                // Unassigned PERF-only ids (currently diagnostic probes such as
                // 250) are instrumentation, not extraction completeness. Their
                // timing/unit payloads remain intentionally outside G1 identity.
                if compact_contract::assigned_descriptor_by_id(domain_id).is_some() {
                    let domain = self.current_domains.entry(domain_id).or_default();
                    domain.perf_records += 1;
                    domain.normalized_status_flags_or |= normalized;
                    domain.lane_local_status_flags_or |=
                        status_flags & lane_local_status_mask(domain_id);
                    domain.non_tolerated_status_flags |= residual;
                } else if residual != 0 {
                    // An unassigned probe id has no domain descriptor, so it can
                    // never be declared degraded and there is nothing to defer
                    // to. Keep the original immediate verdict for it.
                    return Err(format!(
                        "domain {domain_id} PERF status {status_flags:#010x} has non-tolerated failure flags {residual:#010x}"
                    ));
                }
            }
            COMPACT_RECORD_TYPE_FRAME_DEGRADED => {
                let record = compact_contract::frame_degraded::FrameDegradedRecordV1::decode(
                    payload,
                )
                .map_err(|reason| format!("FRAME_DEGRADED payload is unreadable: {reason}"))?;
                // Deliberately NOT re-checked here: that the payload's
                // `degraded_hour` equals the envelope hour. `stream_fold`'s
                // boundary tracker owns that rule for both frame-level records
                // (it owns the identical one for FRAME_REFUSED) and every path
                // into this audit runs through it. A second copy here would be a
                // third independent statement of a boundary rule, which is the
                // shape that already cost this project a real bug -- and a copy
                // that no mutation can kill, because the other copy catches
                // everything first. One rule, one owner.
                //
                // What this audit owns instead is the thing no boundary tracker
                // can see: whether the DECLARATION agrees with the per-domain
                // status words. That reconciliation happens at FRAME_END.
                for domain in &record.domains {
                    if self
                        .current_declared_degraded
                        .insert(domain.domain_id, domain.non_tolerated_flags)
                        .is_some()
                    {
                        return Err(format!(
                            "hour {game_hour_raw} declares domain {} degraded more than once",
                            domain.domain_id
                        ));
                    }
                }
            }
            COMPACT_RECORD_TYPE_FRAME_END => {
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

                // Reconcile OBSERVED degradation against DECLARED degradation.
                //
                // This is the whole certification rule for a degraded frame, and
                // it is deliberately an EQUALITY in both directions:
                //
                //  * a domain whose PERF carries non-tolerated failure bits that
                //    the notice does not declare is an undeclared defect -- the
                //    stream would be handing a consumer broken data that looks
                //    like every other domain in the frame. Same verdict as
                //    before this record existed.
                //  * a domain the notice declares but whose PERF is clean (or
                //    only tolerated) is a fabricated degradation. Harmless to
                //    data, but it means notice and status disagree, and a
                //    consumer that trusts either one alone is then reading a
                //    different frame from a consumer that trusts the other.
                //  * declaring the right domain with the WRONG bits is the same
                //    disagreement in a subtler form, so the bits must match too.
                //
                // The result is that "which domains of this hour are unusable"
                // has exactly one answer, reachable from either the per-record
                // status words or the frame-level notice, and the certifier
                // refuses any stream where those two answers differ.
                let observed_degraded: BTreeMap<u16, u32> = self
                    .current_domains
                    .iter()
                    .filter(|(_, domain)| domain.non_tolerated_status_flags != 0)
                    .map(|(domain_id, domain)| (*domain_id, domain.non_tolerated_status_flags))
                    .collect();
                let declared_degraded = std::mem::take(&mut self.current_declared_degraded);
                if observed_degraded != declared_degraded {
                    for (domain_id, residual) in &observed_degraded {
                        match declared_degraded.get(domain_id) {
                            Some(declared) if declared == residual => {}
                            Some(declared) => {
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

                // And the SECOND half of the same reconciliation: the status the
                // `DOMAIN_DATA` payload carries INSIDE ITSELF.
                //
                // It is a separate rule, not a second term in the one above, and
                // the relation is COVERAGE rather than equality. Two reasons, and
                // both were mistakes this audit actually made:
                //
                //  * ORing the two residuals into one observed word erases the
                //    disagreement between them. `PARTIAL_DATA` in the PERF and
                //    `SOURCE_UNAVAILABLE` in the payload present a union of
                //    `0x00000300`, and a notice declaring `0x00000300` would then
                //    certify an hour against a status NEITHER record carries.
                //  * equality is the wrong relation for the payload. The notice
                //    is minted from the PERF status, so a payload whose residual
                //    is a strict subset of a correctly declared PERF residual is
                //    ordinary and honest. What must never happen is a payload bit
                //    the notice does not name -- which is exactly the defect the
                //    payload-status check exists to catch, because `WorldFold`
                //    withholds the domain's rows on that bit while the PERF and
                //    the notice both say the hour is fine.
                for (domain_id, domain) in &self.current_domains {
                    let payload_residual = domain.non_tolerated_data_payload_status_flags;
                    if payload_residual == 0 {
                        continue;
                    }
                    match declared_degraded.get(domain_id) {
                        Some(declared) if (payload_residual & !declared) == 0 => {}
                        Some(declared) => {
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
                // An hour is published or it is refused; it is never both.
                // Checked in BOTH directions: the refusal arm rejects a
                // refusal of an already-published hour, and this rejects a
                // publication of an already-refused one. A one-sided check
                // would leave the audit holding two contradictory accounts of
                // the same hour and hand the published one to the caller.
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
            COMPACT_RECORD_TYPE_DOMAIN_GAP => {
                // An ACCOUNTED ABSENCE. `H4DG` consumes the ordinary H4CR
                // sequence under the affected domain id and IS that scheduled
                // domain's terminal outcome, so it belongs in the hour's
                // per-domain accounting next to PERF — not swallowed by a
                // catch-all arm, which would let a gap hour look like an hour
                // that simply never scheduled the domain.
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
            COMPACT_RECORD_TYPE_FRAME_REFUSED => {
                // A TERMINATED FRAME. The hour was assembled and closed WITHOUT
                // publishing, so it produces no folded snapshot and must not
                // enter `hours`. It closes the hour exactly as FRAME_END does,
                // which is why it clears `current_domains`: leaving them
                // populated would bleed the refused hour's outcomes into the
                // next one.
                //
                // The boundary rules a refusal must satisfy — that it may only
                // close the hour actually open, that closed hours are strictly
                // increasing, that the envelope hour equals the payload's
                // `refused_hour` — are NOT restated here. `BoundaryTracker` is
                // the single authority for them and the certification verdict
                // reports what it found. Restating them is how the last
                // duplicate of a boundary rule drifted from its original.
                // A version this build does not know still fails loudly here --
                // the decoder names it rather than lifting it to a default.
                let record = FrameRefusalRecordV2::decode(payload)
                    .map_err(|error| format!("FRAME_REFUSED payload: {error}"))?;
                // The decode above succeeded, so the payload bore the magic and
                // declared the one version this build knows; the fallback
                // cannot be reached from here.
                let wire_version = compact_contract::frame_refusal::
                    declared_frame_refusal_version(payload)
                    .unwrap_or(compact_contract::frame_refusal::FRAME_REFUSAL_VERSION);
                let hour_index = compact_contract::calendar::hour_index_from_raw(game_hour_raw)
                    .ok_or_else(|| format!("FRAME_REFUSED game hour {game_hour_raw} is invalid"))?;
                // The open hour's `H4FD` declaration dies with the hour, and it
                // is an ERROR for one to be here at all.
                //
                // `H4FD` annotates an hour that PUBLISHED; `H4FR` closes an hour
                // that published nothing. The collector cannot emit both -- its
                // completeness verdict is Degraded or Refused, never both, and
                // only the Degraded arm splices a notice -- so a frame carrying
                // the pair is a stream whose two frame-level records contradict
                // each other about the same hour.
                //
                // Silently clearing it would stop the leak and still be wrong.
                // Leaving `current_declared_degraded` populated made the NEXT
                // hour inherit this one's declaration, so a domain that failed
                // there with no notice of its own certified against a notice for
                // an hour that never published: certification of an unnamed
                // failure, reached through the record that closes hours.
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
            COMPACT_RECORD_TYPE_CAPTURE_TERMINATED => {
                let termination = compact_contract::capture_termination::
                    CaptureTerminationRecordV1::decode(payload)
                    .map_err(|error| format!("CAPTURE_TERMINATED payload: {error}"))?;
                return Err(format!(
                    "capture terminated abnormally after {} ms: reasons={:?}; detail={}",
                    termination.elapsed_wall_ms,
                    termination.reasons.names(),
                    termination.detail
                ));
            }
            COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL => {
                // AN ACCOUNTED ABSENCE OF HOURS, which is why it appears here
                // and does almost nothing.
                //
                // The hours it names were never observed: no request was built
                // for them, no domain produced an outcome, and no frame was
                // assembled. So there is no per-domain accounting to do and
                // nothing to close -- it must NOT clear `current_domains`, which
                // belong to a real hour that is still open or still being built,
                // and it must not enter `hours` or `refusals`, which are the two
                // dispositions of hours that WERE swept.
                //
                // What it must not do is fall into the arm below and refuse the
                // stream. The boundary rules (envelope hour equals the payload's
                // resumed hour; the range does not reach an already-closed hour)
                // belong to `BoundaryTracker`, which owns every boundary rule in
                // this workspace; restating them here is how the last duplicate
                // of a boundary rule drifted from its original.
                compact_contract::clock_unreadable_interval::
                    ClockUnreadableIntervalRecordV1::decode(payload)
                    .map_err(|error| format!("CLOCK_UNREADABLE_INTERVAL payload: {error}"))?;
            }
            COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY => {
                // AN UNACCOUNTED ABSENCE OF HOURS -- the difference from the
                // arm above is the whole reason this is a separate record.
                //
                // The interval vouches that its hours elapsed; this one refuses
                // to vouch for anything, because the span rests on a clock read
                // the producer rejected. So it enters no accounting at all: not
                // `hours`, not `refusals`, and not any accounted-absence set
                // either. Counting it as accounted would let one failed clock
                // read retire hours the producer explicitly declined to claim.
                //
                // Its boundary rules (envelope hour equals the payload's
                // resumed hour; the recovery does not reach back over a closed
                // hour; it does not contradict an interval) belong to
                // `BoundaryTracker`, which owns every boundary rule in this
                // workspace.
                compact_contract::untrusted_clock_recovery::
                    UntrustedClockRecoveryRecordV1::decode(payload)
                    .map_err(|error| format!("UNTRUSTED_CLOCK_RECOVERY payload: {error}"))?;
            }
            other => {
                // Not a skip. A record type this audit has not been taught is
                // exactly how a certifier drifts out of date while still
                // reporting success — which is what the previous `unreachable!`
                // turned into a panic on every real capture, because DOMAIN_GAP
                // and FRAME_REFUSED were always reachable here.
                return Err(format!(
                    "compact record type {other} is not accounted by the completeness audit"
                ));
            }
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

/// Audit normalized status and completeness directly from decoded H4CR bytes.
/// This is deliberately separate from the world fold: PERF and FRAME_END carry
/// certification evidence but are not folded state. The shared stateful stream
/// validator owns first-record identity placement and ordinary sequence
/// continuity before any record reaches completeness accounting.
/// What one completeness pass observed: the hours that PUBLISHED (each with a
/// folded snapshot to match) and the hours that were REFUSED (attempted, closed,
/// nothing published). Both are outcomes; only the first has a snapshot.
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
                    // Pre-2.22 diagnostic containers can contain the old
                    // domain-zero FRAME_END envelope. Domain zero remains a
                    // hard protocol failure (and is never accepted as an
                    // ordinary Legacy record), but retain the current record
                    // semantics error as context so compatibility diagnostics
                    // still identify the historical FRAME_END shape that
                    // prevented certification.
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
    // The stream-boundary rule ("no hour may be left open at EOF; the final
    // record must be a canonical closed-hour boundary") is NOT restated here.
    // It has exactly one authority — `recorder_host::stream_fold::BoundaryTracker::eof_check`
    // — reached through the decoder's `certification_verdict()`. A third
    // independent copy of that rule is what let the cursor path and the spooled
    // path disagree about honestly-refused streams; this pass owns completeness
    // accounting and nothing else.
    if identity_mode == StreamIdentityMode::IdentityV1 && stream_validator.identity().is_none() {
        return Err("identity-v1 stream ended before its mandatory CID1 header".to_string());
    }
    Ok(AuditResult {
        hours: audit.hours,
        refusals: audit.refusals,
    })
}

fn audit_source(
    source: &Source,
    identity_mode: StreamIdentityMode,
) -> Result<AuditResult, String> {
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
                // fold_check audits the RAW WIRE RECORDS — framing, per-domain
                // record counts, refusals — which the delta cache does not
                // carry at all: the cache proves reconstructed-world parity,
                // not record completeness. So this cannot degrade to a
                // cache-only audit; it degrades to a refusal that names why.
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

/// Join the decoder's exclusion ledger into ONE hour's per-domain completeness,
/// and return how many ledger entries this hour took.
///
/// The join key is the ledger entry's own identity — the `game_hour_raw` the
/// RECORD carried in its envelope, and its domain id. It is emphatically not
/// the hour the entry was processed in and not its position in the ledger:
/// attributing a loss to the wrong hour would leave one hour claiming
/// completeness it does not have while another reports a loss it never
/// suffered. The reason text is carried through but never matched on; reasons
/// are renamed deliberately and are not a join key.
///
/// Called once per published hour with the decoder's running ledger. Every
/// record of hour H precedes H's `FRAME_END` — `BoundaryTracker` refuses an
/// interleaved or reopened hour — so by the time the cursor yields H, every
/// exclusion belonging to H is already in the ledger. `verify_exclusion_ledger_attribution`
/// checks that conclusion rather than assuming it.
///
/// `next` is a resume position into the ledger, so the whole walk is LINEAR
/// rather than a full rescan per hour. That matters on exactly the streams this
/// join exists for: a rescan costs `hours x exclusions`, and a long capture that
/// is losing payloads steadily is the case where both factors are large at once.
/// It is sound for the same reason the timing is: the decoder appends in stream
/// order and closed hours strictly increase, so the ledger is non-decreasing in
/// hour. The walk therefore stops at the first entry belonging to a LATER hour
/// and leaves it for that hour, and steps over entries belonging to earlier
/// hours — those closed without publishing, so no completeness record exists for
/// them to reach. If that ordering ever failed, the entry would go unattributed
/// and `verify_exclusion_ledger_attribution` would refuse the report rather than
/// let the loss disappear.
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
                // The audit counts EVERY framed DOMAIN_DATA record, so the
                // domain this excluded record names must already be here. If it
                // is not, the two passes disagree about what the stream
                // contains, and a certification gate must say so rather than
                // synthesize the missing domain and certify on top of it.
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

/// Every excluded record must land SOMEWHERE, and this says where.
///
/// An exclusion in a published hour must have reached that hour's completeness
/// record; an exclusion in a refused hour has no completeness record to reach,
/// because the hour published nothing and the refusal already says so. An
/// exclusion in neither is a loss that no hour reports — the exact blindness
/// this join exists to remove, merely relocated — so it is named and refused.
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
    // Name the disagreeing domains and both counts. The two sides walk the same
    // bytes, so a disagreement means one of them saw records the other did not
    // attribute to this hour -- which is what a refused hour's records reaching
    // the next published hour looks like from here. A bare pair of map dumps
    // makes the reader diff them by eye.
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

/// One closed game-hour: the canonical hash of its folded snapshot plus the
/// normalized record-level status/completeness evidence that fed it.
struct HourReport {
    hour_index: i64,
    date: String,
    snapshot_hash: String,
    /// The same snapshot with its `provenance` member removed — see
    /// [`world_only_hash`]. Test-only, because nothing the tool PUBLISHES may
    /// be a hash that ignores an hour's account of itself.
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

/// Convert a [`replay_decoder`] pass's totals into the report `Totals`. The two
/// share the same accounting shape (the decoder mirrors what this tool used to
/// count inline); this is a plain field copy.
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

/// Fold an input file through the shared [`replay_decoder`] cursor (auto-detecting
/// a raw `collector_stream.bin` vs a `.battleplan` container), invoking `on_hour`
/// as each game-hour closes so the caller can hash-and-release. The decoder folds
/// under [`FOLD_CHECK_GAME_ID`] regardless of source. The container path inherits
/// the in-RAM entry reader's per-entry cap; use a raw stream for multi-GiB checks.
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
        // Before anything reads this hour's completeness: fold in the records
        // the decoder excluded from THIS hour, so a domain that lost its payload
        // stops being reported complete.
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

/// A fold-equivalence certification over ZERO published hours certifies
/// nothing. Without this, a stream whose every frame was refused reports
/// `hours: []` and the SHA-256 of the empty chain, exit 0 — and `compare` calls
/// two such streams identical. The refusals are the reason, so the verdict
/// names them.
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
            // A version 2 refusal may name no domain at all and carry its whole
            // cause as a frame-level finding. Reporting only the domain union
            // would then say "naming 0 offending domain(s) {}" for a stream
            // that in fact stated exactly why it published nothing. Exact
            // count, bounded names: this is a one-line verdict, and the wire
            // does not cap how many distinct names a stream can invent.
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

/// Hash one published hour with its `provenance` member removed.
///
/// `snapshot_hash` covers every published byte, and since lane GQ taught
/// `WorldFold::ingest_record` to dispatch `COMPACT_RECORD_TYPE_DOMAIN_GAP`, an
/// accounted absence writes a `domain_problems` entry and a note NAMING its
/// reason into `provenance`. So two streams differing only in a gap — or in a
/// gap's reason — legitimately publish different snapshot bytes. That is the
/// point of the per-hour seal: why a domain was absent is part of the hour.
///
/// It also means `snapshot_hash` can no longer answer "did the WORLD move",
/// which is the question the fold-equivalence gates are about. This hash
/// answers that one, by removing exactly the member the gap writes into:
/// `provenance.collector.domain_problems`, `provenance.collector
/// .unfolded_domains` and `provenance.notes` all live under it
/// (`world_fold.rs` builds `collector` into `provenance`), and nothing else
/// does.
///
/// Not a second canonicalization: `serde_json::Map` is `BTreeMap`-backed and
/// this crate does not enable `preserve_order`, so removing one top-level key
/// and re-serializing yields the byte-identical document minus that member.
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

/// Record one closed hour into the running `hours` list. The fold's output is
/// canonical by construction — every accumulator is a BTreeMap/BTreeSet and
/// serde_json (no `preserve_order`) writes object keys sorted, so identical
/// input yields byte-identical snapshot bytes; we hash those shipped bytes.
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

/// Fold an input file into an in-memory [`FoldRun`] (per-hour hashes + totals).
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

/// Fold an already-opened in-memory reader into a [`FoldRun`] through the shared
/// [`replay_decoder`] cursor. Used by the unit tests, which fold an in-memory
/// `Cursor` (under [`FOLD_CHECK_GAME_ID`]) without going through a file.
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

/// Chain hash over ordered per-hour folded hashes and normalized completeness:
/// a single anchor for the full G1 identity rather than folded state alone.
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
            // An accounted absence is part of the hour's identity: two streams
            // that differ only in which domain was gapped, or why, are not the
            // same hour.
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

// ---------------------------------------------------------------------------
// hash
// ---------------------------------------------------------------------------

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

/// How many refused hours are reported in FULL detail. Measured against the
/// captures on disk: one of them refuses 449 consecutive hours, each naming up
/// to two dozen offenders with a typed reason string, so an unbounded rendering
/// turns a report into megabytes of near-identical text. The count, the hour
/// span and the union of offending domains are always exact — only the
/// per-hour detail is bounded, and the report says where it stopped.
const REFUSAL_DETAIL_LIMIT: usize = 64;

/// How many DISTINCT frame-finding predicate names the report lists.
///
/// The count is always exact. The list is bounded because the wire does not
/// constrain predicate names to a vocabulary — [`FrameFinding`] says why — so a
/// stream naming a new predicate every refused hour would otherwise put an
/// unbounded set into a report whose per-hour detail is deliberately bounded.
const FRAME_FINDING_PREDICATE_LIMIT: usize = 32;

/// The exact count of distinct frame-finding predicates over every refused
/// hour, and at most [`FRAME_FINDING_PREDICATE_LIMIT`] of the names.
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
        names.into_iter().take(FRAME_FINDING_PREDICATE_LIMIT).collect(),
    )
}

/// Every hour that was attempted and refused, with the offenders the collector
/// named. This is what makes an attempted-and-refused hour distinguishable from
/// an hour the stream never reached.
fn refusals_json(refusals: &BTreeMap<i64, HourRefusal>) -> Value {
    let mut offending_domains: std::collections::BTreeSet<u16> =
        std::collections::BTreeSet::new();
    for refusal in refusals.values() {
        offending_domains.extend(refusal.offenders.iter().map(|offender| offender.domain_id));
    }
    // A frame-level finding is the only cause a refusal that names no domain
    // has, so the COUNT is exact however many hours were refused; the names
    // are bounded for the reason `FRAME_FINDING_PREDICATE_LIMIT` gives.
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
                // The hour's verdict for this domain, stated rather than left to
                // be re-derived from the fields below. `data_records` counts the
                // RECORD, which is why it cannot answer this: an excluded
                // payload is a counted record whose rows never reached the fold.
                //
                // `state` is the authoritative name, in the shared three-state
                // vocabulary plus the two record-level states it cannot see;
                // `complete` is the single-predicate shorthand for `state ==
                // "complete"` and never widens it — an expected-partial is a
                // sparse patch, not a complete one.
                "state": domain.state(*domain_id),
                "complete": domain.is_complete(*domain_id),
                "data_records": domain.data_records,
                "perf_records": domain.perf_records,
                "gap_records": domain.gap_records,
                "gap_reasons": gap_reason_names(domain.gap_reason_mask),
                "gap_parts": gap_part_names(domain.gap_part_mask),
                // The accounted loss, with the decoder's own words for it.
                "excluded_data_records": domain.excluded_data_records,
                "excluded_data_reasons": domain.excluded_data_reasons,
                "data_payload_status_flags_or": domain.data_payload_status_flags_or,
                "normalized_status_flags_or": domain.normalized_status_flags_or,
                "lane_local_status_flags_or": domain.lane_local_status_flags_or,
            }),
        );
    }
    // WHICH DOMAINS OF THIS HOUR ARE UNKNOWN, stated rather than left to be
    // re-derived. A degraded hour certifies precisely BECAUSE its failures are
    // accounted; a report that certifies it without naming them makes the
    // accounting invisible at the one place a consumer reads the verdict, and
    // "certified" would then look identical for a clean hour and for an hour
    // with six declared-unknown domains.
    //
    // Deliberately NOT added to the chain hash: it is an exact function of the
    // per-domain status words that hash already covers (the FRAME_END
    // reconciliation refuses any stream where the two disagree), so hashing it
    // would add no discriminating power while invalidating every identity
    // already recorded against a certified stream.
    let declared_degraded: Map<String, Value> = completeness
        .declared_degraded_domains
        .iter()
        .map(|(domain_id, non_tolerated_flags)| {
            (
                domain_id.to_string(),
                json!({
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

// ---------------------------------------------------------------------------
// compare
// ---------------------------------------------------------------------------

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
    let (run_a, kind_a) =
        run_input_with_identity_mode(path_a, &expected_roster, identity_mode)?;
    let (run_b, kind_b) =
        run_input_with_identity_mode(path_b, &expected_roster, identity_mode)?;

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

/// The whole "are these two runs the same run" decision, in one place so it can
/// be exercised without a command line. `compare` reports `identical: true`
/// exactly when this returns `None`.
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

    // First divergent hour across the union of hour indices, in order.
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

/// Two streams that publish the same hours but REFUSED different hours did not
/// do the same thing. Fold equivalence over the published hours alone would
/// call them identical and lose the refusals entirely — and a refused hour is
/// precisely the hour a consumer must not assume it has.
fn refusal_divergence(
    a: &BTreeMap<i64, HourRefusal>,
    b: &BTreeMap<i64, HourRefusal>,
) -> Option<Value> {
    if a == b {
        return None;
    }
    let only_a: Vec<i64> = a.keys().filter(|hour| !b.contains_key(hour)).copied().collect();
    let only_b: Vec<i64> = b.keys().filter(|hour| !a.contains_key(hour)).copied().collect();
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

/// Build the drill-down for a hash-mismatched hour: re-fold BOTH inputs, capture
/// only that hour's snapshot bytes, and diff the two folded Values.
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

/// Re-fold `path`, keeping only the snapshot bytes for `target` hour.
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

/// Per top-level snapshot section (the fold's output shape): the "row count" in
/// each side and whether they are equal. A concise map of where the divergence
/// lives before the fine-grained `section_diffs`.
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

/// "Row count" of a value: array/object length, 0 for null/absent, 1 for a scalar.
fn value_len(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => json!(0),
        Some(Value::Array(a)) => json!(a.len()),
        Some(Value::Object(o)) => json!(o.len()),
        Some(_) => json!(1),
    }
}

/// Recursive bounded structural diff. Collects at most `limit` differing leaves,
/// each with its path and a short preview of both sides.
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

/// A bounded preview of a value for the drill-down (scalars pass through; large
/// containers/strings are summarized so the report stays small).
fn preview(v: &Value) -> Value {
    match v {
        Value::Array(a) => json!(format!("<array len {}>", a.len())),
        Value::Object(o) => json!(format!("<object len {}>", o.len())),
        Value::String(s) if s.len() > 120 => json!(format!("{}…({} chars)", &s[..117], s.len())),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// arg parsing + output
// ---------------------------------------------------------------------------

/// Extract positional args and an optional `--out <file>`.
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
                i += 1; // value validated in parse_sample; skip here
            }
            "--expected-profile" => {
                i += 1; // value validated in parse_expected_profile; skip here
            }
            "--identity-mode" => {
                i += 1; // value validated in parse_stream_identity_mode; skip here
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

/// Write a report as deterministic pretty JSON to a file or stdout.
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
