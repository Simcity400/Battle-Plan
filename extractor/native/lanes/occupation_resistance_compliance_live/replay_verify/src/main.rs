#[path = "../../domain52_compact.rs"]
#[allow(dead_code)]
mod domain52_compact;

use compact_contract as compact_record_header;
use compact_contract as compact_record_semantics;
use compact_contract as compact_status_flags;
use compact_record_header::{parse_compact_record_header, COMPACT_RECORD_HEADER_LEN};
#[cfg(test)]
use compact_record_semantics::COMPACT_DOMAIN_FRAME;
use compact_record_semantics::{
    validate_compact_record_semantics, COMPACT_FRAME_END_PAYLOAD_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END,
};
use compact_status_flags::{
    compact_status_unknown_flags_v1, COMPACT_STATUS_BRIDGE_UNAVAILABLE,
    COMPACT_STATUS_CONTROL_INVALID, COMPACT_STATUS_NO_CLOCK, COMPACT_STATUS_NO_SCALAR_SOURCE,
    COMPACT_STATUS_PARTIAL_DATA, COMPACT_STATUS_PROFILE_MISMATCH,
    COMPACT_STATUS_SOURCE_UNAVAILABLE, COMPACT_STATUS_STATE_UNAVAILABLE,
    COMPACT_STATUS_WORKER_PANIC,
};
use serde_json::{json, Value};
use snapshot::sha256::Sha256;
use snapshot::{pack, parse_json_strict, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const GAME_ID_DEFAULT: &str = "orc52";
const SCHEMA_VERSION: &str = "native_collector_session_v1";
const CERTIFICATE_SCHEMA: &str =
    "occupation_resistance_compliance_replay_verification_certificate_v2";
const STREAM_FILE: &str = "collector_stream.bin";
const SOURCE_SUMMARY_FILE: &str = "summary.json";
const CERTIFICATE_FILE: &str = "replay_verification_manifest.json";
const REPLAY_FILE: &str = "native_session.battleplan";
const DOMAIN_ID: u16 = domain52_compact::DOMAIN_ID;
const EXPECTED_BUILD_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const EXPECTED_MODULE_NAME: &str = "hoi4.exe";
const EXPECTED_MODULE_SIZE: u64 = 58_531_840;
const SOURCE_IDENTITY_SCHEMA: &str = "occupation_resistance_compliance_live_probe_source_v1";
const SOURCE_IDENTITY_PATH: &str =
    "extractor/native/lanes/occupation_resistance_compliance_live/live_probe.py";

const STATUS_FAILURE_MASK: u32 = COMPACT_STATUS_STATE_UNAVAILABLE
    | COMPACT_STATUS_PROFILE_MISMATCH
    | COMPACT_STATUS_NO_CLOCK
    | COMPACT_STATUS_NO_SCALAR_SOURCE
    | COMPACT_STATUS_WORKER_PANIC
    | COMPACT_STATUS_BRIDGE_UNAVAILABLE
    | COMPACT_STATUS_SOURCE_UNAVAILABLE
    | COMPACT_STATUS_PARTIAL_DATA
    | COMPACT_STATUS_CONTROL_INVALID;

const D44_GATE_SOURCE: &[u8] = include_bytes!("../../domain44_gate.rs");
const D52_GATE_SOURCE: &[u8] = include_bytes!("../../domain52_gate.rs");
const D44_COMPACT_SOURCE: &[u8] = include_bytes!("../../domain44_compact.rs");
const D52_COMPACT_SOURCE: &[u8] = include_bytes!("../../domain52_compact.rs");
const LIVE_PROBE_SOURCE: &[u8] = include_bytes!("../../live_probe.py");
const REPLAY_VERIFY_SOURCE: &[u8] = include_bytes!("main.rs");

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse(env::args().skip(1))?;
    let report = pack_and_verify(&args.run_dir, &args.game_id)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

struct Args {
    run_dir: PathBuf,
    game_id: String,
}

impl Args {
    fn parse<I>(mut values: I) -> Result<Self, String>
    where
        I: Iterator<Item = String>,
    {
        let mut run_dir = None;
        let mut game_id = GAME_ID_DEFAULT.to_string();
        while let Some(arg) = values.next() {
            match arg.as_str() {
                "--run-dir" => {
                    let value = values
                        .next()
                        .ok_or_else(|| "--run-dir requires a value".to_string())?;
                    run_dir = Some(PathBuf::from(value));
                }
                "--game-id" => {
                    game_id = values
                        .next()
                        .ok_or_else(|| "--game-id requires a value".to_string())?;
                    validate_game_id(&game_id)?;
                }
                "--help" | "-h" => return Err(usage()),
                _ => return Err(format!("unknown argument {arg:?}\n{}", usage())),
            }
        }
        Ok(Self {
            run_dir: run_dir.ok_or_else(usage)?,
            game_id,
        })
    }
}

fn usage() -> String {
    "usage: replay_verify --run-dir <lane-proof-run> [--game-id orc52]".to_string()
}

fn validate_game_id(game_id: &str) -> Result<(), String> {
    if game_id.is_empty()
        || !game_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("game id must be non-empty ASCII alnum, '_' or '-'".to_string());
    }
    Ok(())
}

#[derive(Debug)]
struct CertificationError(Vec<String>);

impl fmt::Display for CertificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "replay verification certification failed")?;
        for error in &self.0 {
            write!(f, "\n- {error}")?;
        }
        Ok(())
    }
}

impl Error for CertificationError {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PerfAuthority {
    elapsed_wall_ms: u64,
    read_time_us: u64,
    row_count: u32,
    changed_rows: u32,
    unit_count: u32,
    changed_unit_count: u32,
    status_flags: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DataAuthority {
    elapsed_wall_ms: u64,
    row_count: u64,
    changed_rows: u64,
    observed_state_rows: u64,
    /// The frame's coverage verdict, header `+52`. Certification requires zero: a
    /// capture whose wire says its coverage is not to be trusted is a capture
    /// whose occupation census the replay map correctly refuses, so certifying
    /// it here would put the two authorities in disagreement. Only the zero/
    /// nonzero distinction is read — the magnitude is the producer's accounting
    /// and not a count this verifier could defend.
    invalid_state_rows: u64,
    status_flags: u32,
    units: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OpenFrame {
    game_hour_raw: Option<i32>,
    perf_records: u32,
    perf_status_flags_or: u32,
    d52_perf: Option<PerfAuthority>,
    d52_data: Option<DataAuthority>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CertifiedFrame {
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
    read_time_us: u64,
    rows_observed: u64,
    rows_emitted: u64,
    rows_changed: u64,
    rows_invalid: u64,
    domain_elapsed_wall_ms: u64,
    status_flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StreamCertification {
    records_seen: u64,
    frame_count: u64,
    domain_perf_records: u64,
    domain_data_records: u64,
    rows_observed_total: u64,
    rows_emitted_total: u64,
    rows_changed_total: u64,
    status_flags_or: u32,
    first_game_hour_raw: i32,
    last_game_hour_raw: i32,
    game_hours_advanced: u64,
    first_elapsed_wall_ms: u64,
    last_elapsed_wall_ms: u64,
    certified_duration_ms: u64,
    domain_elapsed_wall_ms_total: u64,
    units: BTreeMap<String, u64>,
    frames: Vec<CertifiedFrame>,
}

impl StreamCertification {
    fn to_json(&self) -> Value {
        json!({
            "records_seen": self.records_seen,
            "frame_count": self.frame_count,
            "domain_perf_records": self.domain_perf_records,
            "domain_data_records": self.domain_data_records,
            "rows_observed_total": self.rows_observed_total,
            "rows_emitted_total": self.rows_emitted_total,
            "rows_changed_total": self.rows_changed_total,
            "status_flags_or": self.status_flags_or,
            "first_game_hour_raw": self.first_game_hour_raw,
            "last_game_hour_raw": self.last_game_hour_raw,
            "game_hours_advanced": self.game_hours_advanced,
            "first_elapsed_wall_ms": self.first_elapsed_wall_ms,
            "last_elapsed_wall_ms": self.last_elapsed_wall_ms,
            "certified_duration_ms": self.certified_duration_ms,
            "domain_elapsed_wall_ms_total": self.domain_elapsed_wall_ms_total,
            "units": self.units,
            "frames": self.frames.iter().map(|frame| json!({
                "game_hour_raw": frame.game_hour_raw,
                "elapsed_wall_ms": frame.elapsed_wall_ms,
                "read_time_us": frame.read_time_us,
                "rows_observed": frame.rows_observed,
                "rows_emitted": frame.rows_emitted,
                "rows_changed": frame.rows_changed,
                "domain_elapsed_wall_ms": frame.domain_elapsed_wall_ms,
                "status_flags": frame.status_flags,
            })).collect::<Vec<_>>(),
        })
    }
}

fn pack_and_verify(run_dir: &Path, game_id: &str) -> Result<Value, Box<dyn Error>> {
    validate_game_id(game_id)?;
    let summary_path = run_dir.join(SOURCE_SUMMARY_FILE);
    let stream_path = run_dir.join(STREAM_FILE);
    let certificate_path = run_dir.join(CERTIFICATE_FILE);
    let replay_path = run_dir.join(REPLAY_FILE);

    let summary_bytes = fs::read(&summary_path)?;
    let summary = parse_json_strict(&summary_bytes)?;
    let stream_bytes = fs::read(&stream_path)?;
    let source = validate_source_summary(&summary, &stream_bytes)?;
    let stream = certify_stream(&stream_bytes).map_err(CertificationError)?;
    validate_summary_against_authority(&summary, &stream).map_err(CertificationError)?;

    let summary_sha256 = Sha256::hex(&summary_bytes);
    let stream_sha256 = Sha256::hex(&stream_bytes);
    let validator_sources = validator_source_identity();
    let d44_source_bound = validator_hash_matches(
        &validator_sources,
        "domain44_gate_rs_sha256",
        D44_GATE_SOURCE,
    ) && validator_hash_matches(
        &validator_sources,
        "domain44_compact_rs_sha256",
        D44_COMPACT_SOURCE,
    );
    let d52_source_bound = validator_hash_matches(
        &validator_sources,
        "domain52_gate_rs_sha256",
        D52_GATE_SOURCE,
    ) && validator_hash_matches(
        &validator_sources,
        "domain52_compact_rs_sha256",
        D52_COMPACT_SOURCE,
    );
    let certifying = source.evidence.certifying() && d44_source_bound && d52_source_bound;
    if !certifying {
        return Err(Box::new(CertificationError(vec![
            "source evidence verdict is non-certifying".to_string(),
        ])));
    }
    let certificate = json!({
        "schema": CERTIFICATE_SCHEMA,
        "lane": domain52_compact::DOMAIN_NAME,
        "domain_id": DOMAIN_ID,
        "game_id": game_id,
        "source_identity": {
            "summary_file": SOURCE_SUMMARY_FILE,
            "summary_sha256": summary_sha256,
            "stream_file": STREAM_FILE,
            "stream_sha256": stream_sha256,
            "stream_bytes": stream_bytes.len(),
            "build_exe_sha256": source.build_sha256,
            "live_probe_emitter_sha256": source.emitter_sha256,
            "process": source.process_json(),
            "offsets_sha256": source.offsets_sha256,
        },
        "validator_source_identity": validator_sources,
        "stream_authority": stream.to_json(),
        "certification": {
            "certifying": certifying,
            "canonical_d44_compatibility_source_bound": d44_source_bound,
            "canonical_d52_source_bound": d52_source_bound,
            "source_evidence": source.evidence.to_json(),
            "fatal_status_mask": format!("0x{STATUS_FAILURE_MASK:08X}"),
        },
    });
    let certificate_bytes = serde_json::to_vec_pretty(&certificate)?;
    atomic_write(&certificate_path, &certificate_bytes)?;

    let index_bytes = serde_json::to_vec_pretty(&json!({
        "game_id": game_id,
        "schema_version": SCHEMA_VERSION,
        "snapshots": [],
    }))?;
    let timeline_bytes = serde_json::to_vec_pretty(&json!({
        "format": "hoi4_compact_stream_timeline_v1",
        "stream_format": "compact",
        "stream": format!("collector_raw/{STREAM_FILE}"),
        "source_summary": format!("source/{SOURCE_SUMMARY_FILE}"),
        "verification_manifest": format!("verification/{CERTIFICATE_FILE}"),
        "raw_stream_role": "collector_compact_stream",
        "domain_id": DOMAIN_ID,
        "lane": domain52_compact::DOMAIN_NAME,
        "stream_sha256": stream_sha256,
        "source_summary_sha256": summary_sha256,
        "stream_authority": stream.to_json(),
    }))?;
    let entries = vec![
        PackEntry {
            path: format!("payload/{game_id}/index.json"),
            bytes: index_bytes.clone(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/timeline.json"),
            bytes: timeline_bytes.clone(),
            role: "timeline",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/collector_raw/{STREAM_FILE}"),
            bytes: stream_bytes.clone(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/source/{SOURCE_SUMMARY_FILE}"),
            bytes: summary_bytes.clone(),
            role: "source_summary",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/verification/{CERTIFICATE_FILE}"),
            bytes: certificate_bytes.clone(),
            role: "verification_manifest",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some(SCHEMA_VERSION.to_string()),
        build_exe_sha256: Some(source.build_sha256.clone()),
        build_pinned: true,
        created_utc: Some(created_label()),
        ..Default::default()
    };
    let replay_bytes = pack(game_id, &entries, &identity, true)?;
    atomic_write(&replay_path, &replay_bytes)?;

    let expected = ExpectedIdentity {
        build_exe_sha256: Some(source.build_sha256.clone()),
        schema_version: Some(SCHEMA_VERSION.to_string()),
        ..Default::default()
    };
    let container = Container::open_strict(&replay_path, &expected)?;
    container.verify_all()?;
    let expected_entries = [
        ("index.json".to_string(), "index", index_bytes.as_slice()),
        (
            "timeline.json".to_string(),
            "timeline",
            timeline_bytes.as_slice(),
        ),
        (
            format!("collector_raw/{STREAM_FILE}"),
            "collector_compact_stream",
            stream_bytes.as_slice(),
        ),
        (
            format!("source/{SOURCE_SUMMARY_FILE}"),
            "source_summary",
            summary_bytes.as_slice(),
        ),
        (
            format!("verification/{CERTIFICATE_FILE}"),
            "verification_manifest",
            certificate_bytes.as_slice(),
        ),
    ];
    verify_container_identity(&container, game_id, &source.build_sha256, &expected_entries)?;
    let exact_container_readback = true;

    Ok(json!({
        "verified": true,
        "certificate_schema": CERTIFICATE_SCHEMA,
        "replay": replay_path,
        "replay_bytes": fs::metadata(&replay_path)?.len(),
        "game_id": container.game_id(),
        "schema_version": SCHEMA_VERSION,
        "build_sha256": source.build_sha256,
        "source_evidence": source.evidence.to_json(),
        "source_summary_sha256": summary_sha256,
        "collector_compact_stream_sha256": stream_sha256,
        "collector_compact_stream_bytes": stream_bytes.len(),
        "stream_authority": stream.to_json(),
        "validator_source_identity": validator_sources,
        "exact_container_readback": exact_container_readback,
        "container_entry_count": container.entries().len(),
        "index_snapshot_count": container.index().snapshots.len(),
    }))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceIdentity {
    build_sha256: String,
    emitter_sha256: String,
    pid: u32,
    module_base: u64,
    module_size: u64,
    module_path_sha256: String,
    offsets_sha256: String,
    evidence: SourceEvidence,
}

impl SourceIdentity {
    fn process_json(&self) -> Value {
        json!({
            "pid": self.pid,
            "module": EXPECTED_MODULE_NAME,
            "module_base": format!("0x{:X}", self.module_base),
            "module_size": self.module_size,
            "module_path_sha256": self.module_path_sha256,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceEvidence {
    strict_source_json: bool,
    exact_live_probe_source_bound: bool,
    exact_executable_build_bound: bool,
    process_module_bound: bool,
    module_geometry_bound: bool,
    exact_offset_table_bound: bool,
    live_root_evidence_bound: bool,
}

impl SourceEvidence {
    fn certifying(&self) -> bool {
        self.strict_source_json
            && self.exact_live_probe_source_bound
            && self.exact_executable_build_bound
            && self.process_module_bound
            && self.module_geometry_bound
            && self.exact_offset_table_bound
            && self.live_root_evidence_bound
    }

    fn to_json(&self) -> Value {
        json!({
            "strict_source_json": self.strict_source_json,
            "exact_live_probe_source_bound": self.exact_live_probe_source_bound,
            "exact_executable_build_bound": self.exact_executable_build_bound,
            "process_module_bound": self.process_module_bound,
            "module_geometry_bound": self.module_geometry_bound,
            "exact_offset_table_bound": self.exact_offset_table_bound,
            "live_root_evidence_bound": self.live_root_evidence_bound,
        })
    }
}

fn validate_source_summary(
    summary: &Value,
    stream: &[u8],
) -> Result<SourceIdentity, Box<dyn Error>> {
    let mut errors = Vec::new();
    let expected_emitter_sha256 = Sha256::hex(LIVE_PROBE_SOURCE);
    expect_str(
        summary,
        "/source_identity/schema",
        SOURCE_IDENTITY_SCHEMA,
        &mut errors,
    );
    expect_str(
        summary,
        "/source_identity/emitter_path",
        SOURCE_IDENTITY_PATH,
        &mut errors,
    );
    expect_str(
        summary,
        "/source_identity/emitter_sha256",
        &expected_emitter_sha256,
        &mut errors,
    );
    expect_str(
        summary,
        "/source_identity/expected_exe_sha256",
        EXPECTED_BUILD_SHA256,
        &mut errors,
    );
    expect_u64(summary, "/domain/id", u64::from(DOMAIN_ID), &mut errors);
    expect_str(
        summary,
        "/domain/name",
        domain52_compact::DOMAIN_NAME,
        &mut errors,
    );
    expect_str(
        summary,
        "/compact/collector_stream_file",
        STREAM_FILE,
        &mut errors,
    );
    expect_u64(
        summary,
        "/compact/collector_stream_bytes",
        stream.len() as u64,
        &mut errors,
    );
    expect_str(summary, "/compact/record_magic", "H4CR", &mut errors);
    expect_u64(
        summary,
        "/compact/domain_id",
        u64::from(DOMAIN_ID),
        &mut errors,
    );
    let expected_types = json!(["domain_perf", "domain_data", "frame_end"]);
    if summary.pointer("/compact/record_types") != Some(&expected_types) {
        errors.push("summary /compact/record_types is not the exact canonical record order".into());
    }
    let build_sha256 = summary
        .pointer("/process/sha256")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let exact_executable_build_bound =
        is_canonical_sha256(build_sha256) && build_sha256 == EXPECTED_BUILD_SHA256;
    if !exact_executable_build_bound {
        errors.push(format!(
            "summary /process/sha256 must equal pinned build {EXPECTED_BUILD_SHA256}"
        ));
    }

    let pid = summary.pointer("/process/pid").and_then(Value::as_u64);
    let module = summary.pointer("/process/module").and_then(Value::as_str);
    let module_path = summary.pointer("/process/path").and_then(Value::as_str);
    let python_bits = summary
        .pointer("/process/python_bits")
        .and_then(Value::as_u64);
    let process_module_bound = pid.is_some_and(|value| value != 0 && value <= u64::from(u32::MAX))
        && module == Some(EXPECTED_MODULE_NAME)
        && module_path.is_some_and(is_canonical_hoi4_module_path)
        && python_bits == Some(64);
    if !process_module_bound {
        errors.push(
            "summary process identity must bind a nonzero u32 PID, hoi4.exe module/path, and 64-bit reader"
                .into(),
        );
    }

    let module_base_text = summary.pointer("/process/base").and_then(Value::as_str);
    let module_base = module_base_text.and_then(parse_canonical_hex_u64);
    let module_size = summary.pointer("/process/size").and_then(Value::as_u64);
    let module_geometry_bound = module_base.is_some_and(|base| {
        base >= 0x1_0000
            && base <= 0x0000_7FFF_FFFF_FFFF
            && base % 0x1_0000 == 0
            && base.checked_add(EXPECTED_MODULE_SIZE).is_some()
    }) && module_size == Some(EXPECTED_MODULE_SIZE);
    if !module_geometry_bound {
        errors.push(format!(
            "summary process geometry must bind canonical aligned module base and exact size {EXPECTED_MODULE_SIZE}"
        ));
    }

    let expected_offsets = expected_offset_table();
    let actual_offsets = summary.pointer("/offsets");
    let exact_offset_table_bound = actual_offsets == Some(&expected_offsets);
    if !exact_offset_table_bound {
        errors.push("summary /offsets does not exactly equal the pinned live-probe table".into());
    }
    let offsets_sha256 = actual_offsets
        .and_then(|offsets| serde_json::to_vec(offsets).ok())
        .map(|bytes| Sha256::hex(&bytes))
        .unwrap_or_default();

    let live_root_evidence_bound = validate_live_root_evidence(summary, &mut errors);
    let exact_live_probe_source_bound = summary
        .pointer("/source_identity/emitter_sha256")
        .and_then(Value::as_str)
        == Some(expected_emitter_sha256.as_str());
    let evidence = SourceEvidence {
        strict_source_json: true,
        exact_live_probe_source_bound,
        exact_executable_build_bound,
        process_module_bound,
        module_geometry_bound,
        exact_offset_table_bound,
        live_root_evidence_bound,
    };
    if !evidence.certifying() && errors.is_empty() {
        errors.push("source evidence verdict is non-certifying".into());
    }
    if errors.is_empty() {
        Ok(SourceIdentity {
            build_sha256: build_sha256.to_string(),
            emitter_sha256: expected_emitter_sha256,
            pid: pid.expect("validated PID") as u32,
            module_base: module_base.expect("validated module base"),
            module_size: module_size.expect("validated module size"),
            module_path_sha256: Sha256::hex(module_path.expect("validated module path").as_bytes()),
            offsets_sha256,
            evidence,
        })
    } else {
        Err(Box::new(CertificationError(errors)))
    }
}

fn expected_offset_table() -> Value {
    json!({
        "CG_STATE_DATE_OFFSET": "0x3316E90",
        "GS_CURRENT_DATE_OFFSET": "0x468",
        "CGAMESTATE_CPROVINCE_ARRAY": "0x2B0",
        "CGAMESTATE_CPROVINCE_COUNT": "0x2BC",
        "CPROVINCE_VTABLE_RVA": "0x295AF68",
        "CPROVINCE_OFF_ID": "0xA4",
        "CPROVINCE_OFF_STATE": "0xC0",
        "STATE_VTABLE_RVA": "0x29206C0",
        "STATE_OFF_SID": "0x58",
        "STATE_OFF_OWNER": "0xC8",
        "STATE_OFF_CTRL": "0xCC",
        "STATE_OFF_RESISTANCE": "0x278",
        "STATE_OFF_RESISTANCE_SPEED": "0x280",
        "STATE_OFF_COMPLIANCE": "0x2A8",
        "STATE_OFF_COMPLIANCE_SPEED": "0x2B0",
        "CGAMESTATE_COUNTRY_TABLE": "0x310",
        "CGAMESTATE_COUNTRY_COUNT": "0x31C",
        "COUNTRY_VTABLE_RVA": "0x27D0E90",
        "COUNTRY_OFF_TAG_DB_ID": "0x8",
        "COUNTRY_OFF_OCCUPATION_STATUS": "0xFD0",
        "COUNTRY_OCCUPATION_STATUS_VTABLE_RVA": "0x296DD50",
        "COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA": "0x296DD88",
        "COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_OFF": "0x18",
        "COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF": "0x118",
        "COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF": "0x78",
        "OCCUPATION_LAW_VTABLE_RVA": "0x2928218",
        "OCCUPATION_LAW_NAME_OFF": "0x10",
        "DIVISION_TEMPLATE_VTABLE_RVA": "0x2938F70",
        "DIVISION_TEMPLATE_OFF_TYPE": "0x8",
        "DIVISION_TEMPLATE_OFF_ID": "0xC",
        "COUNTRY_OFF_COLLABORATION_STATUS": "0xFD8",
        "COUNTRY_COLLABORATION_STATUS_VTABLE_RVA": "0x29A5D58",
        "FIXED_SCALE_RAW": 100000,
    })
}

fn validate_live_root_evidence(summary: &Value, errors: &mut Vec<String>) -> bool {
    let game_state = summary
        .pointer("/roots/game_state")
        .and_then(Value::as_str)
        .and_then(parse_canonical_hex_u64);
    let province_array = summary
        .pointer("/roots/province_array")
        .and_then(Value::as_str)
        .and_then(parse_canonical_hex_u64);
    let province_count = summary
        .pointer("/roots/province_count")
        .and_then(Value::as_u64);
    let province_rows = summary
        .pointer("/roots/province_rows")
        .and_then(Value::as_u64);
    let invalid_province_rows = summary
        .pointer("/roots/invalid_province_rows")
        .and_then(Value::as_u64);
    let invalid_state_links = summary
        .pointer("/roots/invalid_state_links")
        .and_then(Value::as_u64);
    let state_roots = summary
        .pointer("/roots/state_roots")
        .and_then(Value::as_u64);
    let sample_roots_valid = summary
        .pointer("/roots/sample_state_roots")
        .and_then(Value::as_object)
        .is_some_and(|samples| {
            !samples.is_empty()
                && samples.iter().all(|(state_id, address)| {
                    state_id
                        .parse::<u32>()
                        .ok()
                        .is_some_and(|state_id| state_id != 0 && state_id <= 2_147_483_646)
                        && address
                            .as_str()
                            .and_then(parse_canonical_hex_u64)
                            .is_some_and(is_heap_ptr)
                })
        });
    let valid = game_state.is_some_and(is_heap_ptr)
        && province_array.is_some_and(is_heap_ptr)
        // The 10_000-province floor was lane GK's residual R3 shape: an unproven
        // claim that a real game has at least a vanilla-sized map, which refuses
        // a legal smaller total conversion. What is structural is that slot 0 is
        // the vacant province, so a loaded table declares at least two, and that
        // the walk must authenticate at least one row rather than publish a
        // successful empty.
        && province_count.is_some_and(|count| (2..=65_536).contains(&count))
        && province_rows.is_some_and(|rows| {
            rows >= 1 && province_count.is_some_and(|count| rows <= count)
        })
        && invalid_province_rows == Some(0)
        && invalid_state_links == Some(0)
        // Same for the 1_000-state floor: a conversion may define far fewer.
        // "at least one authenticated state root" is the structural rule.
        && state_roots.is_some_and(|rows| {
            rows >= 1 && province_rows.is_some_and(|provinces| rows <= provinces)
        })
        && sample_roots_valid;
    if !valid {
        errors.push(
            "summary live-root evidence is missing, malformed, below coverage minima, or reports invalid rows"
                .into(),
        );
    }
    valid
}

fn parse_canonical_hex_u64(value: &str) -> Option<u64> {
    let digits = value.strip_prefix("0x")?;
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
    {
        return None;
    }
    let parsed = u64::from_str_radix(digits, 16).ok()?;
    (format!("0x{parsed:X}") == value).then_some(parsed)
}

fn is_heap_ptr(value: u64) -> bool {
    (0x100_0000_0000..0x7FF0_0000_0000).contains(&value) && value % 8 == 0
}

fn is_canonical_hoi4_module_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    let normalized = value.replace('/', "\\");
    bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && matches!(bytes.get(2), Some(b'\\' | b'/'))
        && !bytes.contains(&0)
        && normalized
            .split('\\')
            .skip(1)
            .all(|component| !component.is_empty() && component != "." && component != "..")
        && normalized.to_ascii_lowercase().ends_with("\\hoi4.exe")
}

fn expect_u64(summary: &Value, pointer: &str, expected: u64, errors: &mut Vec<String>) {
    match summary.pointer(pointer).and_then(Value::as_u64) {
        Some(actual) if actual == expected => {}
        actual => errors.push(format!(
            "summary {pointer} {actual:?} does not equal authoritative value {expected}"
        )),
    }
}

fn expect_str(summary: &Value, pointer: &str, expected: &str, errors: &mut Vec<String>) {
    match summary.pointer(pointer).and_then(Value::as_str) {
        Some(actual) if actual == expected => {}
        actual => errors.push(format!(
            "summary {pointer} {actual:?} does not equal canonical value {expected:?}"
        )),
    }
}

fn is_canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_summary_against_authority(
    summary: &Value,
    stream: &StreamCertification,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let summary_status = summary
        .pointer("/rows/status_flags")
        .and_then(Value::as_u64);
    if summary_status != Some(u64::from(stream.status_flags_or)) {
        errors.push(format!(
            "summary /rows/status_flags {summary_status:?} != stream authority {}",
            stream.status_flags_or
        ));
    }
    if stream.frame_count == 1 {
        let frame = &stream.frames[0];
        compare_summary_u64(
            summary,
            "/rows/emitted_state_rows",
            frame.rows_emitted,
            &mut errors,
        );
        compare_summary_u64(
            summary,
            "/roots/state_roots",
            frame.rows_observed,
            &mut errors,
        );
    }
    if let Some(invalid) = summary
        .pointer("/rows/invalid_state_rows")
        .and_then(Value::as_u64)
    {
        if invalid != 0 {
            errors.push(format!(
                "summary reports {invalid} invalid state row(s); certification requires zero"
            ));
        }
    }
    // The WIRE's own coverage verdict, a different claim from the summary's.
    // The summary is written beside the capture and says what the producer
    // believed; `invalid_state_rows` at header `+52` is what the producer
    // actually encoded, hour by hour. Certifying a stream whose wire reports
    // untrusted coverage while its summary claims a clean extraction would put this
    // verifier in direct disagreement with the replay map, which refuses the
    // occupation census of exactly those hours
    // (`replay_interface/native/map_viewer/src/map_authority.rs::occupation_resistance_compliance_status`).
    for frame in &stream.frames {
        if frame.rows_invalid != 0 {
            errors.push(format!(
                "hour {} wire reports untrusted occupation coverage (verdict {}); certification requires zero",
                frame.game_hour_raw, frame.rows_invalid
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn compare_summary_u64(summary: &Value, pointer: &str, expected: u64, errors: &mut Vec<String>) {
    match summary.pointer(pointer).and_then(Value::as_u64) {
        Some(actual) if actual == expected => {}
        actual => errors.push(format!(
            "summary {pointer} {actual:?} != one-frame stream authority {expected}"
        )),
    }
}

fn certify_stream(bytes: &[u8]) -> Result<StreamCertification, Vec<String>> {
    let mut errors = Vec::new();
    if let Err(error) = compact_contract::stream_validator::validate_freeze_census_stream_placement(bytes) {
        errors.push(format!("stateful stream placement: {error}"));
    }
    let mut cursor = 0usize;
    let mut records_seen = 0u64;
    let mut previous_sequence: Option<u64> = None;
    let mut open = OpenFrame::default();
    let mut frames = Vec::new();
    let mut final_record_is_frame_end = false;
    let mut release_epilogue_seen = false;
    let mut status_flags_or = 0u32;
    let mut units = BTreeMap::<String, u64>::new();

    while cursor < bytes.len() {
        let record_offset = cursor;
        let header = match parse_compact_record_header(&bytes[cursor..]) {
            Ok(header) => header,
            Err(error) => {
                errors.push(format!("offset {record_offset}: {error}"));
                break;
            }
        };
        match previous_sequence {
            None if header.sequence != 1 => errors.push(format!(
                "offset {record_offset}: sequence {} does not start at 1",
                header.sequence
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if header.sequence != expected => errors.push(format!(
                    "offset {record_offset}: sequence discontinuity {previous} -> {} (expected {expected})",
                    header.sequence
                )),
                None => errors.push(format!(
                    "offset {record_offset}: sequence space exhausted after {previous}"
                )),
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(header.sequence);
        let previous_record_closed_world = final_record_is_frame_end;
        final_record_is_frame_end = false;
        let payload_start = cursor.saturating_add(COMPACT_RECORD_HEADER_LEN);
        let Some(payload_end) = payload_start.checked_add(header.payload_len) else {
            errors.push(format!("offset {record_offset}: payload length overflows"));
            break;
        };
        if payload_end > bytes.len() {
            errors.push(format!(
                "offset {record_offset}: compact record payload is truncated"
            ));
            break;
        }
        let payload = &bytes[payload_start..payload_end];
        records_seen = records_seen.saturating_add(1);
        let semantics_ok = match validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        ) {
            Ok(()) => true,
            Err(error) => {
                errors.push(format!("offset {record_offset}: {error}"));
                false
            }
        };
        if semantics_ok {
            let record_type = match compact_contract::KnownRecordType::try_from(header.record_type)
            {
                Ok(record_type) => record_type,
                Err(unknown) => {
                    errors.push(format!(
                        "offset {record_offset}: unknown record type {}",
                        unknown.0
                    ));
                    cursor = payload_end;
                    continue;
                }
            };
            if !matches!(
                record_type,
                compact_contract::KnownRecordType::FreezeCensus
                    | compact_contract::KnownRecordType::StreamHeader
            ) {
            observe_frame_hour(&mut errors, &mut open, header.game_hour_raw, record_offset);
            }
            match record_type {
                compact_contract::KnownRecordType::DomainPerf => {
                    open.perf_records = open.perf_records.saturating_add(1);
                    let perf = decode_perf(payload);
                    open.perf_status_flags_or |= perf.status_flags;
                    if header.domain_id == DOMAIN_ID {
                        validate_status(
                            &mut errors,
                            "DOMAIN_PERF",
                            record_offset,
                            perf.status_flags,
                        );
                        status_flags_or |= perf.status_flags;
                        if open.d52_perf.replace(perf).is_some() {
                            errors.push(format!(
                                "offset {record_offset}: duplicate domain {DOMAIN_ID} PERF in frame"
                            ));
                        }
                    }
                }
                compact_contract::KnownRecordType::DomainData => {
                    if header.domain_id == DOMAIN_ID {
                        let decoded = domain52_compact::decode_payload(payload_start, payload);
                        errors.extend(decoded.parse_errors);
                        validate_status(
                            &mut errors,
                            "DOMAIN_DATA",
                            record_offset,
                            decoded.status_flags_or,
                        );
                        status_flags_or |= decoded.status_flags_or;
                        let data_units = decoded
                            .data_units_total
                            .into_iter()
                            .map(|(key, value)| (key.to_string(), value))
                            .collect::<BTreeMap<_, _>>();
                        // The header words below are read by fixed offset, and
                        // nothing in the generic record framing bounds a
                        // DOMAIN_DATA payload's length -- so a short payload has
                        // to be refused HERE, not indexed. `decode_payload` has
                        // already pushed its own named short-header error, which
                        // is the refusal this frame should carry; building an
                        // authority out of bytes that are not there would replace
                        // it with a panic. The window is the whole header,
                        // including the trailing `invalid_state_rows` at `+52`
                        // that widened it from 24 bytes to 56.
                        if payload.len() < domain52_compact::HEADER_LEN {
                            errors.push(format!(
                                "offset {record_offset}: domain {DOMAIN_ID} DATA payload is {}                                  byte(s), too short for the {}-byte header",
                                payload.len(),
                                domain52_compact::HEADER_LEN
                            ));
                        } else {
                            let data = DataAuthority {
                                elapsed_wall_ms: read_u64_le(payload, 0),
                                row_count: decoded.data_rows_total,
                                changed_rows: decoded.data_changed_rows_total,
                                observed_state_rows: u64::from(read_u32_le(payload, 20)),
                                invalid_state_rows: u64::from(read_u32_le(payload, 52)),
                                status_flags: decoded.status_flags_or,
                                units: data_units,
                            };
                            if open.d52_data.replace(data).is_some() {
                                errors.push(format!(
                                    "offset {record_offset}: duplicate domain {DOMAIN_ID} DATA in frame"
                                ));
                            }
                        }
                    }
                }
                compact_contract::KnownRecordType::FrameEnd => {
                    let frame_status = read_u32_le(payload, 24);
                    validate_status(&mut errors, "FRAME_END", record_offset, frame_status);
                    status_flags_or |= frame_status;
                    close_frame(
                        &mut errors,
                        &mut frames,
                        &mut units,
                        &open,
                        payload,
                        record_offset,
                    );
                    open = OpenFrame::default();
                    final_record_is_frame_end = true;
                }
                compact_contract::KnownRecordType::FreezeCensus => {
                    if let Ok(census) = compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload) {
                        final_record_is_frame_end = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(previous_record_closed_world, census.phase(), release_epilogue_seen);
                        release_epilogue_seen |= census.phase() == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                    }
                }
                compact_contract::KnownRecordType::StreamHeader => {}
                compact_contract::KnownRecordType::DomainGap
                | compact_contract::KnownRecordType::FrameRefused
                | compact_contract::KnownRecordType::FrameDegraded
                | compact_contract::KnownRecordType::CaptureTerminated
                | compact_contract::KnownRecordType::ClockUnreadableInterval
                | compact_contract::KnownRecordType::UntrustedClockRecovery => errors.push(
                    format!(
                        "offset {record_offset}: record type {} is outside the occupation replay grammar",
                        record_type.canonical_name()
                    ),
                ),
            }
        }
        cursor = payload_end;
    }

    if open.game_hour_raw.is_some() {
        errors.push("stream ended with an open frame".into());
    }
    if !final_record_is_frame_end {
        errors.push("stream is not closed by a final canonical FRAME_END record".into());
    }
    if frames.is_empty() {
        errors.push(format!(
            "stream contains no certified domain {DOMAIN_ID} frames"
        ));
    }
    let fatal = status_flags_or & STATUS_FAILURE_MASK;
    if fatal != 0 {
        errors.push(format!(
            "domain {DOMAIN_ID} normalized status contains fatal bits 0x{fatal:08X}"
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let first = frames.first().expect("nonempty certified frames");
    let last = frames.last().expect("nonempty certified frames");
    let game_hours_advanced = i64::from(last.game_hour_raw)
        .checked_sub(i64::from(first.game_hour_raw))
        .and_then(|value| u64::try_from(value).ok())
        .expect("validated monotonic frame hours");
    let certified_duration_ms = last
        .elapsed_wall_ms
        .checked_sub(first.elapsed_wall_ms)
        .expect("validated monotonic frame elapsed time");
    Ok(StreamCertification {
        records_seen,
        frame_count: frames.len() as u64,
        domain_perf_records: frames.len() as u64,
        domain_data_records: frames.len() as u64,
        rows_observed_total: frames.iter().map(|frame| frame.rows_observed).sum(),
        rows_emitted_total: frames.iter().map(|frame| frame.rows_emitted).sum(),
        rows_changed_total: frames.iter().map(|frame| frame.rows_changed).sum(),
        status_flags_or,
        first_game_hour_raw: first.game_hour_raw,
        last_game_hour_raw: last.game_hour_raw,
        game_hours_advanced,
        first_elapsed_wall_ms: first.elapsed_wall_ms,
        last_elapsed_wall_ms: last.elapsed_wall_ms,
        certified_duration_ms,
        domain_elapsed_wall_ms_total: frames
            .iter()
            .map(|frame| frame.domain_elapsed_wall_ms)
            .sum(),
        units,
        frames,
    })
}

fn observe_frame_hour(
    errors: &mut Vec<String>,
    open: &mut OpenFrame,
    game_hour_raw: i32,
    offset: usize,
) {
    match open.game_hour_raw {
        None => open.game_hour_raw = Some(game_hour_raw),
        Some(open_hour) if open_hour == game_hour_raw => {}
        Some(open_hour) => errors.push(format!(
            "offset {offset}: record hour {game_hour_raw} does not match open frame hour {open_hour}"
        )),
    }
}

fn decode_perf(payload: &[u8]) -> PerfAuthority {
    PerfAuthority {
        elapsed_wall_ms: read_u64_le(payload, 0),
        read_time_us: read_u64_le(payload, 8),
        row_count: read_u32_le(payload, 16),
        changed_rows: read_u32_le(payload, 20),
        unit_count: read_u32_le(payload, 24),
        changed_unit_count: read_u32_le(payload, 28),
        status_flags: read_u32_le(payload, 32),
    }
}

fn close_frame(
    errors: &mut Vec<String>,
    frames: &mut Vec<CertifiedFrame>,
    aggregate_units: &mut BTreeMap<String, u64>,
    open: &OpenFrame,
    payload: &[u8],
    offset: usize,
) {
    debug_assert_eq!(payload.len(), COMPACT_FRAME_END_PAYLOAD_LEN);
    let frame_hour = open
        .game_hour_raw
        .expect("FRAME_END established frame hour");
    let frame_elapsed_wall_ms = read_u64_le(payload, 0);
    let emitted_domains = read_u32_le(payload, 16);
    let scheduled_domains = read_u32_le(payload, 20);
    let frame_status = read_u32_le(payload, 24);
    let reserved = read_u32_le(payload, 28);
    if emitted_domains != open.perf_records {
        errors.push(format!(
            "offset {offset}: FRAME_END emitted_domains {emitted_domains} != observed PERF count {}",
            open.perf_records
        ));
    }
    if scheduled_domains != emitted_domains {
        errors.push(format!(
            "offset {offset}: FRAME_END scheduled_domains {scheduled_domains} != emitted_domains {emitted_domains}"
        ));
    }
    if reserved != 0 {
        errors.push(format!(
            "offset {offset}: FRAME_END reserved word is nonzero (0x{reserved:08X})"
        ));
    }
    let Some(perf) = &open.d52_perf else {
        errors.push(format!(
            "offset {offset}: frame hour {frame_hour} has no domain {DOMAIN_ID} PERF"
        ));
        return;
    };
    let Some(data) = &open.d52_data else {
        errors.push(format!(
            "offset {offset}: frame hour {frame_hour} has no domain {DOMAIN_ID} DATA"
        ));
        return;
    };
    if perf.status_flags != data.status_flags {
        errors.push(format!(
            "offset {offset}: domain {DOMAIN_ID} PERF status 0x{:08X} != DATA status 0x{:08X}",
            perf.status_flags, data.status_flags
        ));
    }
    if frame_status != open.perf_status_flags_or {
        errors.push(format!(
            "offset {offset}: FRAME_END status 0x{frame_status:08X} != observed PERF status OR 0x{:08X}",
            open.perf_status_flags_or
        ));
    }
    if u64::from(perf.row_count) != data.observed_state_rows {
        errors.push(format!(
            "offset {offset}: domain {DOMAIN_ID} PERF row_count {} != DATA observed_state_rows {}",
            perf.row_count, data.observed_state_rows
        ));
    }
    if u64::from(perf.changed_rows) != data.row_count || data.changed_rows != data.row_count {
        errors.push(format!(
            "offset {offset}: domain {DOMAIN_ID} emitted-row authority differs: PERF changed_rows {}, DATA row_count {}, DATA changed_rows {}",
            perf.changed_rows, data.row_count, data.changed_rows
        ));
    }
    if perf.unit_count != perf.changed_rows || perf.changed_unit_count != perf.changed_rows {
        errors.push(format!(
            "offset {offset}: domain {DOMAIN_ID} PERF unit cardinality ({}, {}) != changed_rows {}",
            perf.unit_count, perf.changed_unit_count, perf.changed_rows
        ));
    }
    if perf.elapsed_wall_ms != frame_elapsed_wall_ms {
        errors.push(format!(
            "offset {offset}: domain {DOMAIN_ID} PERF elapsed_wall_ms {} != FRAME_END elapsed_wall_ms {frame_elapsed_wall_ms}",
            perf.elapsed_wall_ms
        ));
    }
    if let Some(previous) = frames.last() {
        match previous.game_hour_raw.checked_add(1) {
            Some(expected) if frame_hour == expected => {}
            Some(expected) => errors.push(format!(
                "offset {offset}: frame hour {frame_hour} follows {}; expected {expected}",
                previous.game_hour_raw
            )),
            None => errors.push(format!(
                "offset {offset}: previous frame hour exhausted i32 range"
            )),
        }
        if frame_elapsed_wall_ms < previous.elapsed_wall_ms {
            errors.push(format!(
                "offset {offset}: FRAME_END elapsed_wall_ms regressed {} -> {frame_elapsed_wall_ms}",
                previous.elapsed_wall_ms
            ));
        }
    }
    for (key, value) in &data.units {
        let total = aggregate_units.entry(key.clone()).or_default();
        *total = total.saturating_add(*value);
    }
    frames.push(CertifiedFrame {
        game_hour_raw: frame_hour,
        elapsed_wall_ms: frame_elapsed_wall_ms,
        read_time_us: perf.read_time_us,
        rows_observed: data.observed_state_rows,
        rows_emitted: data.row_count,
        rows_changed: data.changed_rows,
        rows_invalid: data.invalid_state_rows,
        domain_elapsed_wall_ms: data.elapsed_wall_ms,
        status_flags: data.status_flags,
    });
}

fn validate_status(errors: &mut Vec<String>, label: &str, offset: usize, status: u32) {
    let unknown = compact_status_unknown_flags_v1(status);
    if unknown != 0 {
        errors.push(format!(
            "offset {offset}: {label} status 0x{status:08X} contains unknown bits 0x{unknown:08X}"
        ));
    }
    let fatal = status & STATUS_FAILURE_MASK;
    if fatal != 0 {
        errors.push(format!(
            "offset {offset}: {label} status 0x{status:08X} contains fatal bits 0x{fatal:08X}"
        ));
    }
}

fn validator_source_identity() -> Value {
    json!({
        "live_probe_py_sha256": Sha256::hex(LIVE_PROBE_SOURCE),
        "domain44_gate_rs_sha256": Sha256::hex(D44_GATE_SOURCE),
        "domain52_gate_rs_sha256": Sha256::hex(D52_GATE_SOURCE),
        "domain44_compact_rs_sha256": Sha256::hex(D44_COMPACT_SOURCE),
        "domain52_compact_rs_sha256": Sha256::hex(D52_COMPACT_SOURCE),
        "replay_verify_main_rs_sha256": Sha256::hex(REPLAY_VERIFY_SOURCE),
    })
}

fn validator_hash_matches(identity: &Value, field: &str, source: &[u8]) -> bool {
    let expected = Sha256::hex(source);
    identity.get(field).and_then(Value::as_str) == Some(expected.as_str())
}

fn verify_container_identity(
    container: &Container,
    game_id: &str,
    build_sha256: &str,
    expected_entries: &[(String, &str, &[u8])],
) -> Result<(), Box<dyn Error>> {
    let mut errors = Vec::new();
    if container.game_id() != game_id {
        errors.push(format!(
            "container game id {:?} != expected {game_id:?}",
            container.game_id()
        ));
    }
    if container.index().game_id != game_id {
        errors.push("container index game id differs from requested game id".into());
    }
    if !container.index().snapshots.is_empty() {
        errors.push("stream-only certification index unexpectedly contains snapshots".into());
    }
    if container
        .manifest()
        .pointer("/build/pinned")
        .and_then(Value::as_bool)
        != Some(true)
    {
        errors.push("container build identity is not pinned".into());
    }
    if container
        .manifest()
        .pointer("/build/exe_sha256")
        .and_then(Value::as_str)
        != Some(build_sha256)
    {
        errors.push("container executable SHA differs from source identity".into());
    }
    let entries = container.entries();
    if entries.len() != expected_entries.len() {
        errors.push(format!(
            "container has {} entries; expected {}",
            entries.len(),
            expected_entries.len()
        ));
    }
    let mut seen = BTreeSet::new();
    for (suffix, role, expected_bytes) in expected_entries {
        let path = format!("payload/{game_id}/{suffix}");
        if !seen.insert(path.clone()) {
            errors.push(format!("duplicate expected entry path {path}"));
            continue;
        }
        let matching = entries
            .iter()
            .filter(|entry| entry.path == path && entry.role.as_deref() == Some(*role))
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            errors.push(format!(
                "container entry {path} role {role:?} appears {} time(s)",
                matching.len()
            ));
            continue;
        }
        let entry = matching[0];
        if entry.bytes != expected_bytes.len() as u64 || entry.sha256 != Sha256::hex(expected_bytes)
        {
            errors.push(format!("container manifest metadata mismatch for {path}"));
        }
        match container.read_entry_bytes(&path) {
            Ok(actual) if actual == *expected_bytes => {}
            Ok(_) => errors.push(format!("container exact readback mismatch for {path}")),
            Err(error) => errors.push(format!("container readback failed for {path}: {error}")),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Box::new(CertificationError(errors)))
    }
}

static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    atomic_write_with(path, bytes, replace_atomic)?;
    Ok(())
}

fn atomic_write_with<F>(path: &Path, bytes: &[u8], replace: F) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "output path has no UTF-8 file name",
            )
        })?;
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "output path has no parent directory",
        )
    })?;
    let (tmp_path, mut tmp_file) = create_unique_temp(parent, file_name)?;
    if let Err(error) = tmp_file.write_all(bytes).and_then(|()| tmp_file.sync_all()) {
        drop(tmp_file);
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }
    drop(tmp_file);
    if let Err(error) = replace(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }
    Ok(())
}

fn create_unique_temp(parent: &Path, file_name: &str) -> io::Result<(PathBuf, fs::File)> {
    let time_nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..64 {
        let counter = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let tmp_path = parent.join(format!(
            ".{file_name}.{}.{}.{}.tmp",
            std::process::id(),
            time_nonce,
            counter
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
        {
            Ok(file) => return Ok((tmp_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique atomic-write temporary file",
    ))
}

#[cfg(windows)]
fn replace_atomic(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_atomic(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)?;
    if let Some(parent) = destination.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed u32 slice"),
    )
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed u64 slice"),
    )
}

fn created_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix_seconds:{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain52_compact::{OccupationHeader, OccupationStateRow};

    #[test]
    fn game_id_validation_accepts_safe_ids_and_rejects_paths() {
        validate_game_id("orc52").unwrap();
        validate_game_id("orc-52_test").unwrap();
        assert!(validate_game_id("../orc52").is_err());
        assert!(validate_game_id("").is_err());
    }

    /// kills: reading the d52 header by fixed offset without bounding the
    /// payload first. Nothing in the generic record framing bounds a
    /// `DOMAIN_DATA` payload's length, so a truncated one -- including a capture
    /// written against the old 52-byte header -- indexed past its end and
    /// PANICKED instead of returning the decoder's named refusal. Discarding old
    /// captures is the standing rule; crashing on them is not fail-closed.
    #[test]
    fn a_short_d52_data_payload_is_refused_by_name_not_by_panic() {
        for len in [0usize, 24, 52, 55] {
            let mut stream = Vec::new();
            append_record(
                &mut stream,
                1,
                100,
                DOMAIN_ID,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &vec![0u8; len],
            );
            let errors =
                certify_stream(&stream).expect_err("a truncated d52 DATA payload cannot certify");
            assert!(
                errors.iter().any(|error| error.contains("too short")
                    || error.contains("expected at least")),
                "len {len}: {errors:?}"
            );
        }
    }

    /// kills: certifying a capture whose WIRE reports untrusted coverage.
    /// The summary is written beside the capture and says what the producer
    /// believed; `invalid_state_rows` at header `+52` is what it encoded. A
    /// stream where those disagree must not certify here, because the replay map
    /// refuses the occupation census of exactly those hours -- two authorities
    /// reaching opposite verdicts about one capture is the defect.
    #[test]
    fn a_stream_whose_wire_reports_dropped_state_roots_does_not_certify() {
        let mut stream = valid_stream(100, 20, 0);
        let summary = valid_summary(&stream);
        // Find the DATA payload's header and say one of its two roots was
        // unreadable, keeping the full-frame identity intact so the failure is
        // the census verdict and not a framing error.
        let data_at = stream
            .windows(4)
            .position(|w| w == 100_000u32.to_le_bytes())
            .expect("the DATA header carries the fixed scale word");
        let header = data_at - 32;
        stream[header + 20..header + 24].copy_from_slice(&2u32.to_le_bytes());
        stream[header + 52..header + 56].copy_from_slice(&1u32.to_le_bytes());
        // PERF's `row_count` is the same root census from the other record, and
        // `certify_frame` requires the two to agree. Moving only one would fail
        // on that disagreement instead of on the census verdict under test.
        let perf_at = stream
            .windows(8)
            .position(|w| w == 3_000u64.to_le_bytes())
            .expect("the PERF payload carries its read-time word");
        stream[perf_at + 8..perf_at + 12].copy_from_slice(&2u32.to_le_bytes());
        let cert = certify_stream(&stream).expect("the stream still frames and decodes");
        assert_eq!(cert.frames[0].rows_invalid, 1);
        let errors = validate_summary_against_authority(&summary, &cert)
            .expect_err("a wire-reported drop must refuse certification");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("untrusted occupation coverage")),
            "{errors:?}"
        );
    }

    #[test]
    fn valid_stream_certifies_frame_cardinality_status_and_duration() {
        let stream = two_frame_stream();
        let cert = certify_stream(&stream).unwrap();
        assert_eq!(cert.frame_count, 2);
        assert_eq!(cert.records_seen, 6);
        assert_eq!(cert.first_game_hour_raw, 100);
        assert_eq!(cert.last_game_hour_raw, 101);
        assert_eq!(cert.game_hours_advanced, 1);
        assert_eq!(cert.first_elapsed_wall_ms, 20);
        assert_eq!(cert.last_elapsed_wall_ms, 45);
        assert_eq!(cert.certified_duration_ms, 25);
        // Two frames, one observed root each: down from 2_000 with the DATA and
        // PERF fixtures, which declared a live-sized root count beside a single
        // emitted row.
        assert_eq!(cert.rows_observed_total, 2);
        assert_eq!(cert.rows_emitted_total, 2);
        assert_eq!(cert.rows_changed_total, 2);
    }

    #[test]
    fn rejects_sequence_truncation_and_nonterminal_frame_end() {
        let mut wrong_sequence = valid_stream(100, 20, 0);
        wrong_sequence[8..16].copy_from_slice(&2u64.to_le_bytes());
        assert_error_contains(certify_stream(&wrong_sequence), "does not start at 1");

        let mut truncated = valid_stream(100, 20, 0);
        truncated.pop();
        assert_error_contains(certify_stream(&truncated), "payload is truncated");

        let mut trailing = valid_stream(100, 20, 0);
        append_record(
            &mut trailing,
            4,
            101,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(30, 0),
        );
        assert_error_contains(certify_stream(&trailing), "open frame");
    }

    #[test]
    fn rejects_status_cardinality_and_duration_forgery() {
        let fatal = compact_status_flags::COMPACT_STATUS_SOURCE_UNAVAILABLE;
        assert_error_contains(certify_stream(&valid_stream(100, 20, fatal)), "fatal bits");

        let mut cardinality = valid_stream(100, 20, 0);
        let perf_payload_start = COMPACT_RECORD_HEADER_LEN;
        cardinality[perf_payload_start + 20..perf_payload_start + 24]
            .copy_from_slice(&2u32.to_le_bytes());
        assert_error_contains(
            certify_stream(&cardinality),
            "emitted-row authority differs",
        );

        let mut duration = valid_stream(100, 20, 0);
        let frame_offset = record_offset(&duration, 2);
        let frame_payload = frame_offset + COMPACT_RECORD_HEADER_LEN;
        duration[frame_payload..frame_payload + 8].copy_from_slice(&21u64.to_le_bytes());
        assert_error_contains(certify_stream(&duration), "PERF elapsed_wall_ms");
    }

    #[test]
    fn frame_end_status_must_equal_all_observed_perf_statuses() {
        let d52_status = compact_status_flags::COMPACT_STATUS_DELTA_DATA;
        let other_status = compact_status_flags::COMPACT_STATUS_EXTRACTION_PERF_GATE;
        let exact_status = d52_status | other_status;
        let certified = certify_stream(&stream_with_other_perf(exact_status)).unwrap();
        assert_eq!(certified.status_flags_or, exact_status);

        assert_error_contains(
            certify_stream(&stream_with_other_perf(d52_status)),
            "observed PERF status OR",
        );
        assert_error_contains(
            certify_stream(&stream_with_other_perf(
                exact_status | compact_status_flags::COMPACT_STATUS_WAR_BRIDGE_APPLIED,
            )),
            "observed PERF status OR",
        );
    }

    #[test]
    fn failed_atomic_replace_preserves_last_good_output_and_cleans_temp() {
        let dir = test_dir("atomic_failure");
        let output = dir.join("certificate.json");
        fs::write(&output, b"last-good").unwrap();

        let error = atomic_write_with(&output, b"replacement", |temporary, destination| {
            assert_eq!(portable_test_fs_retry::read(temporary).unwrap(), b"replacement");
            assert_eq!(portable_test_fs_retry::read(destination).unwrap(), b"last-good");
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected replace failure",
            ))
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(portable_test_fs_retry::read(&output).unwrap(), b"last-good");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn atomic_write_replaces_existing_output_without_temp_residue() {
        let dir = test_dir("atomic_success");
        let output = dir.join("certificate.json");
        fs::write(&output, b"last-good").unwrap();

        atomic_write(&output, b"replacement").unwrap();

        assert_eq!(portable_test_fs_retry::read(&output).unwrap(), b"replacement");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn strict_summary_requires_exact_source_process_build_offsets_and_roots() {
        let stream = valid_stream(100, 20, 0);
        let summary = valid_summary(&stream);
        let source = validate_source_summary(&summary, &stream).unwrap();
        assert!(source.evidence.certifying());

        let arbitrary_build = "1111111111111111111111111111111111111111111111111111111111111111";
        let mut self_consistent_wrong_build = summary.clone();
        self_consistent_wrong_build["process"]["sha256"] = json!(arbitrary_build);
        self_consistent_wrong_build["source_identity"]["expected_exe_sha256"] =
            json!(arbitrary_build);
        let build_error = validate_source_summary(&self_consistent_wrong_build, &stream)
            .expect_err("self-consistent arbitrary build unexpectedly certified")
            .to_string();
        assert!(build_error.contains("pinned build"), "{build_error}");
        assert!(build_error.contains("expected_exe_sha256"), "{build_error}");
        assert_source_mutation_rejected(
            &summary,
            &stream,
            "/source_identity/emitter_sha256",
            json!("1111111111111111111111111111111111111111111111111111111111111111"),
            "emitter_sha256",
        );
        assert_source_mutation_rejected(
            &summary,
            &stream,
            "/process/pid",
            json!(0),
            "nonzero u32 PID",
        );
        assert_source_mutation_rejected(
            &summary,
            &stream,
            "/process/module",
            json!("fake.exe"),
            "process identity",
        );
        assert_source_mutation_rejected(
            &summary,
            &stream,
            "/process/path",
            json!("X:\\SteamLibrary\\..\\hoi4.exe"),
            "process identity",
        );
        assert_source_mutation_rejected(
            &summary,
            &stream,
            "/process/base",
            json!("0x7FF700000001"),
            "process geometry",
        );
        assert_source_mutation_rejected(&summary, &stream, "/process/size", json!(1), "exact size");
        assert_source_mutation_rejected(
            &summary,
            &stream,
            "/offsets/STATE_OFF_SID",
            json!("0x60"),
            "pinned live-probe table",
        );
        assert_source_mutation_rejected(
            &summary,
            &stream,
            "/roots/invalid_state_links",
            json!(1),
            "live-root evidence",
        );

        let duplicate = br#"{"domain":{"id":52,"id":44}}"#;
        assert!(parse_json_strict(duplicate).is_err());
    }

    #[test]
    fn pack_certification_binds_every_source_and_container_entry_exactly() {
        let dir = test_dir("pack_exact");
        let stream = valid_stream(100, 20, 0);
        fs::write(dir.join(STREAM_FILE), &stream).unwrap();
        fs::write(
            dir.join(SOURCE_SUMMARY_FILE),
            serde_json::to_vec_pretty(&valid_summary(&stream)).unwrap(),
        )
        .unwrap();
        let report = pack_and_verify(&dir, "orc52").unwrap();
        assert_eq!(report["verified"], true);
        assert_eq!(report["container_entry_count"], 5);
        assert_eq!(report["stream_authority"]["frame_count"], 1);
        assert_eq!(
            report["source_evidence"]["exact_live_probe_source_bound"],
            true
        );
        assert_eq!(
            report["source_evidence"]["exact_executable_build_bound"],
            true
        );
        assert_eq!(report["source_evidence"]["process_module_bound"], true);
        assert_eq!(report["source_evidence"]["module_geometry_bound"], true);
        assert_eq!(report["source_evidence"]["exact_offset_table_bound"], true);
        assert_eq!(report["source_evidence"]["live_root_evidence_bound"], true);
        assert_eq!(report["exact_container_readback"], true);
        assert!(dir.join(CERTIFICATE_FILE).is_file());
        assert!(dir.join(REPLAY_FILE).is_file());
        assert!(!dir.join("native_session_manifest.json").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pack_rejects_summary_stream_disagreement_before_writing_outputs() {
        let dir = test_dir("summary_disagreement");
        let stream = valid_stream(100, 20, 0);
        let mut summary = valid_summary(&stream);
        summary["compact"]["collector_stream_bytes"] = json!(stream.len() + 1);
        fs::write(dir.join(STREAM_FILE), &stream).unwrap();
        fs::write(
            dir.join(SOURCE_SUMMARY_FILE),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .unwrap();
        let error = pack_and_verify(&dir, "orc52").unwrap_err().to_string();
        assert!(error.contains("collector_stream_bytes"), "{error}");
        assert!(!dir.join(CERTIFICATE_FILE).exists());
        assert!(!dir.join(REPLAY_FILE).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    fn valid_summary(stream: &[u8]) -> Value {
        json!({
            "source_identity": {
                "schema": SOURCE_IDENTITY_SCHEMA,
                "emitter_path": SOURCE_IDENTITY_PATH,
                "emitter_sha256": Sha256::hex(LIVE_PROBE_SOURCE),
                "expected_exe_sha256": EXPECTED_BUILD_SHA256,
            },
            "domain": {"id": DOMAIN_ID, "name": domain52_compact::DOMAIN_NAME},
            "process": {
                "pid": 1234,
                "module": EXPECTED_MODULE_NAME,
                "base": "0x7FF700000000",
                "size": EXPECTED_MODULE_SIZE,
                "path": "X:\\SteamLibrary\\steamapps\\common\\Hearts of Iron IV\\hoi4.exe",
                "python_bits": 64,
                "sha256": EXPECTED_BUILD_SHA256,
            },
            "roots": {
                "game_state": "0x10000000000",
                "province_array": "0x10000000008",
                "province_count": 15000,
                "province_rows": 14000,
                "invalid_province_rows": 0,
                "invalid_state_links": 0,
                // One authenticated state root, matching what the DATA and PERF
                // fixtures now declare. The rule this satisfies is "at least
                // one", not a thousand.
                "state_roots": 1,
                "sample_state_roots": {"1": "0x10000000010"},
            },
            "offsets": expected_offset_table(),
            "rows": {"emitted_state_rows": 1, "invalid_state_rows": 0, "status_flags": 0},
            "compact": {
                "collector_stream_file": STREAM_FILE,
                "collector_stream_bytes": stream.len(),
                "record_magic": "H4CR",
                "domain_id": DOMAIN_ID,
                "record_types": ["domain_perf", "domain_data", "frame_end"],
            },
        })
    }

    fn two_frame_stream() -> Vec<u8> {
        let mut stream = valid_stream(100, 20, 0);
        let mut second = valid_stream(101, 45, 0);
        for record_index in 0..3 {
            let offset = record_offset(&second, record_index);
            second[offset + 8..offset + 16]
                .copy_from_slice(&((record_index + 4) as u64).to_le_bytes());
        }
        stream.extend_from_slice(&second);
        stream
    }

    fn stream_with_other_perf(frame_status: u32) -> Vec<u8> {
        let d52_status = compact_status_flags::COMPACT_STATUS_DELTA_DATA;
        let other_status = compact_status_flags::COMPACT_STATUS_EXTRACTION_PERF_GATE;
        let payload = data_payload(d52_status);
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            100,
            10,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(20, other_status),
        );
        append_record(
            &mut stream,
            2,
            100,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(20, d52_status),
        );
        append_record(
            &mut stream,
            3,
            100,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_record(
            &mut stream,
            4,
            100,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload_with_counts(20, frame_status, 2),
        );
        stream
    }

    fn valid_stream(game_hour: i32, elapsed_wall_ms: u64, status: u32) -> Vec<u8> {
        let payload = data_payload(status);
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            game_hour,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(elapsed_wall_ms, status),
        );
        append_record(
            &mut stream,
            2,
            game_hour,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_record(
            &mut stream,
            3,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(elapsed_wall_ms, status),
        );
        stream
    }

    fn data_payload(status: u32) -> Vec<u8> {
        domain52_compact::encode_payload(
            &OccupationHeader {
                elapsed_wall_ms: 3,
                status_flags: status,
                changed_rows: 1,
                // ONE root walked, one row emitted, no drops. The fixture used
                // to declare a live-sized 1_000 against its single row, which
                // was never a shape the producer could build and which the
                // full-frame identity `row_count + invalid == observed` now
                // rejects. Restating it as 999 drops would have made it decode
                // while inverting what it asserts -- this fixture exists to
                // certify a CLEAN extraction, and the surrounding summary claims
                // zero invalid rows. So the observed count comes down to match
                // the rows instead.
                observed_state_rows: 1,
                occupied_state_rows: 0,
                wall_badge_rows: 1,
                law_token_rows: 0,
                garrison_template_rows: 0,
                garrison_scalar_rows: 0,
                garrison_anomaly_rows: 0,
                invalid_state_rows: 0,
            },
            &[OccupationStateRow {
                state_id: 1,
                owner_tag_db_id: 1,
                controller_tag_db_id: 1,
                occupied_country_tag_db_id: 0,
                resistance_raw: 0,
                compliance_raw: 0,
                resistance_speed_raw: 0,
                compliance_speed_raw: 0,
                garrison_manpower_need: 0,
                garrison_strength_ratio_raw: 0,
                garrison_required_raw: 0,
                occupation_law_token_id: 0,
                garrison_template_id: 0,
                flags: domain52_compact::ROW_FLAG_OWNER
                    | domain52_compact::ROW_FLAG_CONTROLLER
                    | domain52_compact::ROW_FLAG_RESISTANCE
                    | domain52_compact::ROW_FLAG_COMPLIANCE
                    | domain52_compact::ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
                    | domain52_compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
                    | domain52_compact::ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE,
            }],
        )
    }

    fn perf_payload(elapsed_wall_ms: u64, status: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
        payload.extend_from_slice(&3_000u64.to_le_bytes());
        // PERF row_count, which `certify_frame` requires to equal DATA's
        // `observed_state_rows`. It came down from 1_000 with the DATA fixture:
        // one root walked, one row emitted, no drops. A live-sized root count
        // beside a single emitted row was never a shape the producer could
        // build, and the full-frame identity now says so.
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&status.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn frame_end_payload(elapsed_wall_ms: u64, status: u32) -> Vec<u8> {
        frame_end_payload_with_counts(elapsed_wall_ms, status, 1)
    }

    fn frame_end_payload_with_counts(
        elapsed_wall_ms: u64,
        status: u32,
        domain_count: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
        payload.extend_from_slice(&3_000u64.to_le_bytes());
        payload.extend_from_slice(&domain_count.to_le_bytes());
        payload.extend_from_slice(&domain_count.to_le_bytes());
        payload.extend_from_slice(&status.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn append_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) {
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&record_flags.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn record_offset(stream: &[u8], record_index: usize) -> usize {
        let mut offset = 0;
        for _ in 0..record_index {
            let header = parse_compact_record_header(&stream[offset..]).unwrap();
            offset += header.record_len();
        }
        offset
    }

    fn assert_error_contains(result: Result<StreamCertification, Vec<String>>, needle: &str) {
        let errors = result.expect_err("forged stream unexpectedly certified");
        assert!(
            errors.iter().any(|error| error.contains(needle)),
            "missing {needle:?} in {errors:#?}"
        );
    }

    fn assert_source_mutation_rejected(
        source: &Value,
        stream: &[u8],
        pointer: &str,
        replacement: Value,
        needle: &str,
    ) {
        let mut forged = source.clone();
        *forged
            .pointer_mut(pointer)
            .unwrap_or_else(|| panic!("test pointer {pointer} is missing")) = replacement;
        let error = validate_source_summary(&forged, stream)
            .expect_err("forged source evidence unexpectedly certified")
            .to_string();
        assert!(error.contains(needle), "missing {needle:?} in {error}");
    }

    fn test_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "occupation_replay_verify_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
