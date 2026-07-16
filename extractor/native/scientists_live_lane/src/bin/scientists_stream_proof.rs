use scientists_live_lane::source_evidence::{
    sha256_hex, verify_required_dll_scan_source_evidence, ExpectedStreamIdentity,
};
use scientists_live_lane::stream_certification::{
    certify_canonical_stream, StreamCertificationPolicy,
};
use serde_json::json;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("scientists_stream_proof: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mut stream_path = None;
    let mut proof_path = None;
    let mut profile = "production".to_string();
    let mut requested_game_hours = None;
    let mut source_report_path = None;
    let mut collector_dll_path = None;
    let mut game_exe_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stream" => stream_path = args.next(),
            "--proof" => proof_path = args.next(),
            "--profile" => profile = args.next().ok_or("--profile requires a value")?,
            "--requested-game-hours" => {
                let raw = args
                    .next()
                    .ok_or("--requested-game-hours requires a positive integer")?;
                requested_game_hours = Some(raw.parse::<u32>().map_err(|_| {
                    format!("--requested-game-hours must be a positive integer, got {raw:?}")
                })?);
            }
            "--source-report" => source_report_path = args.next(),
            "--collector-dll" => collector_dll_path = args.next(),
            "--game-exe" => game_exe_path = args.next(),
            "--help" | "-h" => {
                println!("usage: scientists_stream_proof --stream <collector_stream.bin[.zst]> --proof <proof.json> --requested-game-hours <N> --source-report <dll-scan-source.json> --collector-dll <collector.dll> --game-exe <hoi4.exe> [--profile production|scientists-fixture]");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let stream_path = stream_path.ok_or("missing --stream")?;
    let proof_path = proof_path.ok_or("missing --proof")?;
    let canonical_stream = fs::canonicalize(&stream_path)
        .map_err(|error| format!("failed to resolve stream {stream_path}: {error}"))?;
    let canonical_proof = canonical_output_path(&proof_path)?;
    if canonical_stream == canonical_proof {
        return Err("--proof must not name or resolve to the input stream".to_string());
    }
    let policy = match profile.as_str() {
        "production" => StreamCertificationPolicy::production(
            requested_game_hours
                .ok_or("production certification requires explicit --requested-game-hours <N>")?,
        )?,
        "scientists-fixture" => {
            if requested_game_hours.is_some()
                || source_report_path.is_some()
                || collector_dll_path.is_some()
                || game_exe_path.is_some()
            {
                return Err(
                    "production duration/source-evidence arguments are forbidden for scientists-fixture"
                        .to_string(),
                );
            }
            StreamCertificationPolicy::scientists_fixture()
        }
        other => return Err(format!("unknown profile {other}")),
    };

    let stored = fs::read(&stream_path)
        .map_err(|error| format!("failed to read stored stream {stream_path}: {error}"))?;
    let mut opened = compact_stream::open(&stream_path)
        .map_err(|error| format!("failed to open/decode stream {stream_path}: {error}"))?;
    let compression = opened.compression().as_str();
    let mut decoded = Vec::new();
    opened
        .read_to_end(&mut decoded)
        .map_err(|error| format!("failed to drain stream to physical EOF: {error}"))?;
    let stored_after = fs::read(&stream_path)
        .map_err(|error| format!("failed to re-read stored stream {stream_path}: {error}"))?;
    if stored_after != stored {
        return Err("stored stream changed while certification was reading it".to_string());
    }
    let mut report = certify_canonical_stream(&decoded, &policy);
    let source_evidence = if profile == "production" && report.ok {
        let evidence_result = (|| {
            let source_report_path = source_report_path
                .as_deref()
                .ok_or("production certification requires --source-report <path>")?;
            let collector_dll_path = collector_dll_path
                .as_deref()
                .ok_or("production certification requires --collector-dll <path>")?;
            let game_exe_path = game_exe_path
                .as_deref()
                .ok_or("production certification requires --game-exe <path>")?;
            let source_report =
                read_bound_input(source_report_path, "source report", &canonical_proof)?;
            let collector_dll =
                read_bound_input(collector_dll_path, "collector DLL", &canonical_proof)?;
            let game_exe = read_bound_input(game_exe_path, "game executable", &canonical_proof)?;
            let first_game_hour_raw = report
                .first_hour
                .ok_or("source evidence cannot bind a stream without a first closed hour")?;
            let last_game_hour_raw = report
                .last_hour
                .ok_or("source evidence cannot bind a stream without a last closed hour")?;
            let requested = policy
                .requested_game_hour_advancements
                .ok_or("source evidence cannot bind production without requested game hours")?;
            verify_required_dll_scan_source_evidence(
                Some(&source_report),
                Some(&collector_dll),
                Some(&game_exe),
                ExpectedStreamIdentity {
                    stored_stream: &stored,
                    decoded_stream: &decoded,
                    requested_game_hour_advancements: requested,
                    closed_frames: report.closed_frames,
                    first_game_hour_raw,
                    last_game_hour_raw,
                },
            )
        })();
        match evidence_result {
            Ok(verified) => Some(verified),
            Err(error) => {
                report
                    .problems
                    .push(format!("production source evidence rejected: {error}"));
                report.ok = false;
                None
            }
        }
    } else {
        None
    };
    let production_certifying = profile == "production" && report.ok && source_evidence.is_some();
    let source_evidence_json = source_evidence.as_ref().map(|verified| {
        json!({
            "verified": true,
            "source_report_path": source_report_path.as_deref().map(path_slash),
            "source_report_sha256": verified.source_report_sha256,
            "collector_dll_path": collector_dll_path.as_deref().map(path_slash),
            "collector_dll_sha256": verified.collector_dll_sha256,
            "collector_pe_image_size": verified.collector_image_size,
            "game_exe_path": game_exe_path.as_deref().map(path_slash),
            "game_exe_sha256": verified.game_exe_sha256,
            "build_id": scientists_live_lane::source_evidence::SOURCE_EVIDENCE_BUILD_ID,
            "session_id": verified.session_id,
            "evidence_nonce_hex": verified.evidence_nonce_hex,
            "process_id": verified.process_id,
            "process_creation_filetime_utc": verified.process_creation_filetime_utc,
            "module_base": format!("0x{:x}", verified.module_base),
            "module_size": verified.module_size,
        })
    });
    let proof = json!({
        "schema": "scientists_live_stream_proof_v2",
        "source_stream": stream_path.replace('\\', "/"),
        "requested_profile": profile,
        "profile_is_production": production_certifying,
        "production_certifying": production_certifying,
        "source_capture_verified": source_evidence.is_some(),
        "source_evidence": source_evidence_json,
        "stream_identity": {
            "compression": compression,
            "stored_bytes": stored.len(),
            "stored_sha256": sha256_hex(&stored),
            "decoded_bytes": decoded.len(),
            "decoded_sha256": sha256_hex(&decoded),
        },
        "authority": {
            "expected_perf_roster": policy.expected_perf_roster,
            "expected_data_roster": policy.expected_data_roster,
            "requires_exact_dll_scan_source": true,
            "requires_progress_v2": true,
            "requires_data_perf_frame_end_equality": true,
            "requires_clean_physical_eof": true,
            "requested_game_hour_advancements": policy.requested_game_hour_advancements,
            "required_closed_frames": policy.requested_game_hour_advancements.map(|hours| hours + 1),
        },
        "result": {
            "ok": report.ok,
            "problems": report.problems.clone(),
            "physical_records": report.physical_records,
            "closed_frames": report.closed_frames,
            "first_hour": report.first_hour,
            "last_hour": report.last_hour,
            "projects_data_records": report.projects_data_records,
            "scientist_upserts": report.scientist_upserts,
            "scientist_deletes": report.scientist_deletes,
            "final_scientist_rows": report.final_scientist_rows,
            "max_observed_scientists": report.max_observed_scientists,
            "requested_game_hour_advancements": report.requested_game_hour_advancements,
            "actual_game_hour_advancements": report.actual_game_hour_advancements,
        }
    });
    fs::write(
        &proof_path,
        serde_json::to_vec_pretty(&proof).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write proof {proof_path}: {error}"))?;
    if !report.ok {
        return Err(format!("certification failed; wrote {proof_path}"));
    }
    if !production_certifying {
        return Err(format!(
            "proof is not production-certifying; wrote {proof_path}"
        ));
    }
    Ok(())
}

fn read_bound_input(path: &str, label: &str, canonical_proof: &Path) -> Result<Vec<u8>, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {label} {path}: {error}"))?;
    if canonical == canonical_proof {
        return Err(format!("--proof must not name or resolve to the {label}"));
    }
    let first =
        fs::read(path).map_err(|error| format!("failed to read {label} {path}: {error}"))?;
    let second =
        fs::read(path).map_err(|error| format!("failed to re-read {label} {path}: {error}"))?;
    if first != second {
        return Err(format!(
            "{label} changed while certification was reading it"
        ));
    }
    Ok(first)
}

fn path_slash(path: &str) -> String {
    path.replace('\\', "/")
}

fn canonical_output_path(path: &str) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if path.exists() {
        return fs::canonicalize(path)
            .map_err(|error| format!("failed to resolve proof path {path:?}: {error}"));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("proof path {path:?} has no file name"))?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("failed to resolve proof parent {parent:?}: {error}"))?;
    Ok(canonical_parent.join(file_name))
}
