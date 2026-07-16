//! Durable recorder-owned provenance report for production domain 41 DLL scans.

use serde_json::json;
use snapshot::sha256::Sha256;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScientistSourceReportRequest<'a> {
    pub output_path: &'a Path,
    pub session_id: &'a str,
    pub stored_stream_sha256: &'a str,
    pub decoded_stream_sha256: &'a str,
    pub requested_game_hour_advancements: u32,
    pub closed_frames: usize,
    pub first_game_hour_raw: i32,
    pub last_game_hour_raw: i32,
    pub collector_source_dll_sha256: &'a str,
    pub collector_staged_dll_sha256: &'a str,
    pub collector_loaded_dll_sha256: &'a str,
    pub collector_pe_image_size: u32,
    pub game_exe_sha256: &'a str,
    pub process_id: u32,
    pub process_creation_filetime_utc: u64,
    pub game_module_base: u64,
    pub game_module_size: u64,
    pub game_module_backing_file_sha256: &'a str,
    pub bridge_rows_supplied: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedScientistSourceReport {
    pub path: PathBuf,
    pub sha256: String,
    pub evidence_nonce_hex: String,
}

pub fn build_and_write(
    request: ScientistSourceReportRequest<'_>,
) -> Result<VerifiedScientistSourceReport, String> {
    if !canonical_uuid(request.session_id) {
        return Err("scientist source session_id is not a canonical lowercase UUID".to_string());
    }
    for (value, label) in [
        (request.stored_stream_sha256, "stored stream"),
        (request.decoded_stream_sha256, "decoded stream"),
        (request.collector_source_dll_sha256, "collector source DLL"),
        (request.collector_staged_dll_sha256, "collector staged DLL"),
        (request.collector_loaded_dll_sha256, "collector loaded DLL"),
        (request.game_exe_sha256, "game executable"),
        (
            request.game_module_backing_file_sha256,
            "game module backing file",
        ),
    ] {
        validate_sha(value, label)?;
    }
    if request.collector_source_dll_sha256 != request.collector_staged_dll_sha256
        || request.collector_staged_dll_sha256 != request.collector_loaded_dll_sha256
    {
        return Err("scientist source collector source/staged/loaded hashes differ".to_string());
    }
    if request.game_exe_sha256 != request.game_module_backing_file_sha256 {
        return Err("scientist source game executable/module hashes differ".to_string());
    }
    if request.process_id == 0
        || request.process_creation_filetime_utc == 0
        || request.game_module_base == 0
        || request.game_module_base % 0x1000 != 0
        || request.game_module_size == 0
        || request.collector_pe_image_size == 0
    {
        return Err("scientist source process or PE identity is invalid".to_string());
    }
    if request.closed_frames == 0 || request.first_game_hour_raw > request.last_game_hour_raw {
        return Err("scientist source stream boundary facts are invalid".to_string());
    }
    if request.bridge_rows_supplied != 0 {
        return Err("scientist production DLL-scan source cannot include bridge rows".to_string());
    }
    if request.output_path.exists() {
        return Err(format!(
            "scientist source report output already exists: {}",
            request.output_path.display()
        ));
    }

    let nonce_material = format!(
        "scientists-live-source-v1\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
        request.session_id,
        request.decoded_stream_sha256,
        request.collector_loaded_dll_sha256,
        request.game_exe_sha256,
        request.process_id,
        request.process_creation_filetime_utc,
        request.game_module_base,
    );
    let evidence_nonce_hex = Sha256::hex(nonce_material.as_bytes());
    let report = json!({
        "schema": scientists_live_lane::source_evidence::SOURCE_EVIDENCE_SCHEMA,
        "verdict": "passed",
        "source_kind": "dll_scan",
        "domain_id": scientists_live_lane::PROJECTS_LIVE_DOMAIN_ID,
        "session_id": request.session_id,
        "evidence_nonce_hex": evidence_nonce_hex,
        "stream": {
            "stored_sha256": request.stored_stream_sha256,
            "decoded_sha256": request.decoded_stream_sha256,
            "requested_game_hour_advancements": request.requested_game_hour_advancements,
            "closed_frames": request.closed_frames,
            "first_game_hour_raw": request.first_game_hour_raw,
            "last_game_hour_raw": request.last_game_hour_raw,
        },
        "collector": {
            "source_dll_sha256": request.collector_source_dll_sha256,
            "staged_dll_sha256": request.collector_staged_dll_sha256,
            "loaded_dll_sha256": request.collector_loaded_dll_sha256,
            "pe_image_size": request.collector_pe_image_size,
        },
        "game": {
            "exe_sha256": request.game_exe_sha256,
            "build_id": scientists_live_lane::source_evidence::SOURCE_EVIDENCE_BUILD_ID,
            "process_id": request.process_id,
            "process_creation_filetime_utc": request.process_creation_filetime_utc,
            "module_base": format!("0x{:x}", request.game_module_base),
            "module_size": request.game_module_size,
            "module_backing_file_sha256": request.game_module_backing_file_sha256,
        },
        "route": {
            "country_scientist_roster_offset": scientists_live_lane::COUNTRY_OFF_SCIENTIST_ROSTER,
            "roster_vector_offset": scientists_live_lane::SCIENTIST_ROSTER_VECTOR,
            "roster_vector_count_offset": scientists_live_lane::SCIENTIST_ROSTER_VECTOR_COUNT,
            "wrapper_character_offset": scientists_live_lane::SCIENTIST_ROSTER_WRAPPER_CHARACTER,
            "character_type_offset": scientists_live_lane::CHARACTER_OFF_TYPE,
            "character_type": scientists_live_lane::CHARACTER_TYPE,
            "character_scientist_offset": scientists_live_lane::CHARACTER_OFF_SCIENTIST_PTR,
            "character_vtable_rva": scientists_live_lane::CHARACTER_VTABLE_RVA_870EBF,
            "scientist_vtable_rva": scientists_live_lane::SCIENTIST_VTABLE_RVA_870EBF,
            "scientist_skills_offset": scientists_live_lane::SCIENTIST_OFF_SKILLS_PTR,
            "skill_header_base": scientists_live_lane::SKILL_HEADER_BASE_870EBF,
        },
        "capture": {
            "special_projects_scan": true,
            "projects_source_kind": "dll_scan",
            "bridge_rows_supplied": request.bridge_rows_supplied,
        },
    });
    let mut bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("serialize scientist source report failed: {error}"))?;
    bytes.push(b'\n');
    write_create_new_durable(request.output_path, &bytes)?;
    Ok(VerifiedScientistSourceReport {
        path: request.output_path.to_path_buf(),
        sha256: Sha256::hex(&bytes),
        evidence_nonce_hex,
    })
}

fn canonical_uuid(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| match index {
        8 | 13 | 18 | 23 => byte == b'-',
        _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
    })
}

fn validate_sha(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("{label} SHA-256 is not canonical lowercase hex"))
    }
}

fn write_create_new_durable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create scientist report directory failed: {error}"))?;
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("create scientist source report failed: {error}"))?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|error| format!("write scientist source report failed: {error}"))?;
        file.flush()
            .map_err(|error| format!("flush scientist source report failed: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync scientist source report failed: {error}"))
    })();
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_noncanonical_session_before_io() {
        let request = ScientistSourceReportRequest {
            output_path: Path::new("unused.json"),
            session_id: "not-a-uuid",
            stored_stream_sha256: &"a".repeat(64),
            decoded_stream_sha256: &"b".repeat(64),
            requested_game_hour_advancements: 1,
            closed_frames: 1,
            first_game_hour_raw: 1,
            last_game_hour_raw: 2,
            collector_source_dll_sha256: &"c".repeat(64),
            collector_staged_dll_sha256: &"c".repeat(64),
            collector_loaded_dll_sha256: &"c".repeat(64),
            collector_pe_image_size: 4096,
            game_exe_sha256: &"d".repeat(64),
            process_id: 1,
            process_creation_filetime_utc: 1,
            game_module_base: 4096,
            game_module_size: 4096,
            game_module_backing_file_sha256: &"d".repeat(64),
            bridge_rows_supplied: 0,
        };
        assert!(build_and_write(request)
            .unwrap_err()
            .contains("canonical lowercase UUID"));
    }
}
