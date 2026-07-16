//! Fail-closed provenance binding for a production scientists stream proof.
//!
//! H4CR bytes prove wire semantics, not that those bytes came from the loaded
//! collector DLL or the claimed game process.  Production certification must
//! additionally consume this independently emitted source report and bind its
//! exact bytes, collector image, game image, process/module identity, route,
//! and stream hashes.  This module never manufactures source evidence.

use crate::{
    CHARACTER_OFF_SCIENTIST_PTR, CHARACTER_OFF_TYPE, CHARACTER_TYPE, CHARACTER_VTABLE_RVA_870EBF,
    COUNTRY_OFF_SCIENTIST_ROSTER, SCIENTIST_OFF_SKILLS_PTR, SCIENTIST_ROSTER_VECTOR,
    SCIENTIST_ROSTER_VECTOR_COUNT, SCIENTIST_ROSTER_WRAPPER_CHARACTER, SCIENTIST_VTABLE_RVA_870EBF,
    SKILL_HEADER_BASE_870EBF,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const SOURCE_EVIDENCE_SCHEMA: &str = "scientists_live_dll_scan_source_v1";
pub const SOURCE_EVIDENCE_BUILD_ID: &str = "870ebf";

#[derive(Clone, Copy, Debug)]
pub struct ExpectedStreamIdentity<'a> {
    pub stored_stream: &'a [u8],
    pub decoded_stream: &'a [u8],
    pub requested_game_hour_advancements: u32,
    pub closed_frames: usize,
    pub first_game_hour_raw: i32,
    pub last_game_hour_raw: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct ExpectedStreamHashes<'a> {
    pub stored_sha256: &'a str,
    pub decoded_sha256: &'a str,
    pub requested_game_hour_advancements: u32,
    pub closed_frames: usize,
    pub first_game_hour_raw: i32,
    pub last_game_hour_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedDllScanSourceEvidence {
    pub source_report_sha256: String,
    pub collector_dll_sha256: String,
    pub collector_image_size: u32,
    pub game_exe_sha256: String,
    pub session_id: String,
    pub evidence_nonce_hex: String,
    pub process_id: u32,
    pub process_creation_filetime_utc: u64,
    pub module_base: u64,
    pub module_size: u64,
}

pub fn verify_required_dll_scan_source_evidence(
    report_bytes: Option<&[u8]>,
    collector_dll_bytes: Option<&[u8]>,
    game_exe_bytes: Option<&[u8]>,
    expected_stream: ExpectedStreamIdentity<'_>,
) -> Result<VerifiedDllScanSourceEvidence, String> {
    let stored_sha256 = sha256_hex(expected_stream.stored_stream);
    let decoded_sha256 = sha256_hex(expected_stream.decoded_stream);
    verify_required_dll_scan_source_evidence_hashes(
        report_bytes,
        collector_dll_bytes,
        game_exe_bytes,
        ExpectedStreamHashes {
            stored_sha256: &stored_sha256,
            decoded_sha256: &decoded_sha256,
            requested_game_hour_advancements: expected_stream.requested_game_hour_advancements,
            closed_frames: expected_stream.closed_frames,
            first_game_hour_raw: expected_stream.first_game_hour_raw,
            last_game_hour_raw: expected_stream.last_game_hour_raw,
        },
    )
}

pub fn verify_required_dll_scan_source_evidence_hashes(
    report_bytes: Option<&[u8]>,
    collector_dll_bytes: Option<&[u8]>,
    game_exe_bytes: Option<&[u8]>,
    expected_stream: ExpectedStreamHashes<'_>,
) -> Result<VerifiedDllScanSourceEvidence, String> {
    let report_bytes = report_bytes.ok_or("missing authoritative DLL-scan source report")?;
    let collector_dll_bytes =
        collector_dll_bytes.ok_or("missing exact collector DLL bytes for source evidence")?;
    let game_exe_bytes =
        game_exe_bytes.ok_or("missing exact game executable bytes for source evidence")?;
    let report: Value = serde_json::from_slice(report_bytes)
        .map_err(|error| format!("source report is not valid JSON: {error}"))?;
    let root = exact_object(
        &report,
        "source report",
        &[
            "schema",
            "verdict",
            "source_kind",
            "domain_id",
            "session_id",
            "evidence_nonce_hex",
            "stream",
            "collector",
            "game",
            "route",
            "capture",
        ],
    )?;
    require_string(root, "schema", SOURCE_EVIDENCE_SCHEMA)?;
    require_string(root, "verdict", "passed")?;
    require_string(root, "source_kind", "dll_scan")?;
    require_u64(root, "domain_id", 41)?;
    let session_id = string(root, "session_id")?;
    if !canonical_uuid(session_id) {
        return Err("source report session_id is not a canonical lowercase UUID".to_string());
    }
    let evidence_nonce_hex = string(root, "evidence_nonce_hex")?;
    validate_lower_hex(evidence_nonce_hex, 64, "source report evidence_nonce_hex")?;

    validate_lower_hex(
        expected_stream.stored_sha256,
        64,
        "expected stored stream SHA-256",
    )?;
    validate_lower_hex(
        expected_stream.decoded_sha256,
        64,
        "expected decoded stream SHA-256",
    )?;
    let collector_dll_sha256 = sha256_hex(collector_dll_bytes);
    let game_exe_sha256 = sha256_hex(game_exe_bytes);
    let collector_image = parse_pe64(collector_dll_bytes, "collector DLL")?;
    let game_image = parse_pe64(game_exe_bytes, "game executable")?;
    verify_supported_game_sentinels(game_exe_bytes, &game_image)?;

    let stream = exact_object(
        value(root, "stream")?,
        "source report stream",
        &[
            "stored_sha256",
            "decoded_sha256",
            "requested_game_hour_advancements",
            "closed_frames",
            "first_game_hour_raw",
            "last_game_hour_raw",
        ],
    )?;
    require_string(stream, "stored_sha256", expected_stream.stored_sha256)?;
    require_string(stream, "decoded_sha256", expected_stream.decoded_sha256)?;
    require_u64(
        stream,
        "requested_game_hour_advancements",
        expected_stream.requested_game_hour_advancements as u64,
    )?;
    require_u64(
        stream,
        "closed_frames",
        expected_stream.closed_frames as u64,
    )?;
    require_i64(
        stream,
        "first_game_hour_raw",
        expected_stream.first_game_hour_raw as i64,
    )?;
    require_i64(
        stream,
        "last_game_hour_raw",
        expected_stream.last_game_hour_raw as i64,
    )?;

    let collector = exact_object(
        value(root, "collector")?,
        "source report collector",
        &[
            "source_dll_sha256",
            "staged_dll_sha256",
            "loaded_dll_sha256",
            "pe_image_size",
        ],
    )?;
    for key in [
        "source_dll_sha256",
        "staged_dll_sha256",
        "loaded_dll_sha256",
    ] {
        require_string(collector, key, &collector_dll_sha256)?;
    }
    require_u64(
        collector,
        "pe_image_size",
        collector_image.image_size as u64,
    )?;

    let game = exact_object(
        value(root, "game")?,
        "source report game",
        &[
            "exe_sha256",
            "build_id",
            "process_id",
            "process_creation_filetime_utc",
            "module_base",
            "module_size",
            "module_backing_file_sha256",
        ],
    )?;
    require_string(game, "exe_sha256", &game_exe_sha256)?;
    require_string(game, "module_backing_file_sha256", &game_exe_sha256)?;
    require_string(game, "build_id", SOURCE_EVIDENCE_BUILD_ID)?;
    let process_id = unsigned(game, "process_id")?;
    if process_id == 0 || process_id > u32::MAX as u64 {
        return Err(format!("source report process_id {process_id} is invalid"));
    }
    let process_creation_filetime_utc = unsigned(game, "process_creation_filetime_utc")?;
    if process_creation_filetime_utc == 0 {
        return Err("source report process creation FILETIME is zero".to_string());
    }
    let module_base_text = string(game, "module_base")?;
    let module_base = parse_canonical_hex_u64(module_base_text, "source report module_base")?;
    if module_base == 0 || module_base % 0x1000 != 0 {
        return Err(format!(
            "source report module_base {module_base_text:?} is not nonzero/page-aligned"
        ));
    }
    let module_size = unsigned(game, "module_size")?;
    if module_size != game_image.image_size as u64 {
        return Err(format!(
            "source report module_size {module_size} is invalid"
        ));
    }

    let route = exact_object(
        value(root, "route")?,
        "source report route",
        &[
            "country_scientist_roster_offset",
            "roster_vector_offset",
            "roster_vector_count_offset",
            "wrapper_character_offset",
            "character_type_offset",
            "character_type",
            "character_scientist_offset",
            "character_vtable_rva",
            "scientist_vtable_rva",
            "scientist_skills_offset",
            "skill_header_base",
        ],
    )?;
    for (key, expected) in [
        (
            "country_scientist_roster_offset",
            COUNTRY_OFF_SCIENTIST_ROSTER as u64,
        ),
        ("roster_vector_offset", SCIENTIST_ROSTER_VECTOR as u64),
        (
            "roster_vector_count_offset",
            SCIENTIST_ROSTER_VECTOR_COUNT as u64,
        ),
        (
            "wrapper_character_offset",
            SCIENTIST_ROSTER_WRAPPER_CHARACTER as u64,
        ),
        ("character_type_offset", CHARACTER_OFF_TYPE as u64),
        ("character_type", CHARACTER_TYPE as u64),
        (
            "character_scientist_offset",
            CHARACTER_OFF_SCIENTIST_PTR as u64,
        ),
        ("character_vtable_rva", CHARACTER_VTABLE_RVA_870EBF as u64),
        ("scientist_vtable_rva", SCIENTIST_VTABLE_RVA_870EBF as u64),
        ("scientist_skills_offset", SCIENTIST_OFF_SKILLS_PTR as u64),
        ("skill_header_base", SKILL_HEADER_BASE_870EBF as u64),
    ] {
        require_u64(route, key, expected)?;
    }

    let capture = exact_object(
        value(root, "capture")?,
        "source report capture",
        &[
            "special_projects_scan",
            "projects_source_kind",
            "bridge_rows_supplied",
        ],
    )?;
    if capture
        .get("special_projects_scan")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("source report special_projects_scan is not true".to_string());
    }
    require_string(capture, "projects_source_kind", "dll_scan")?;
    require_u64(capture, "bridge_rows_supplied", 0)?;

    Ok(VerifiedDllScanSourceEvidence {
        source_report_sha256: sha256_hex(report_bytes),
        collector_dll_sha256,
        collector_image_size: collector_image.image_size,
        game_exe_sha256,
        session_id: session_id.to_string(),
        evidence_nonce_hex: evidence_nonce_hex.to_string(),
        process_id: process_id as u32,
        process_creation_filetime_utc,
        module_base,
        module_size,
    })
}

#[derive(Clone, Debug)]
struct Pe64Image {
    image_size: u32,
    sections: Vec<PeSection>,
}

#[derive(Clone, Copy, Debug)]
struct PeSection {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

fn parse_pe64(bytes: &[u8], label: &str) -> Result<Pe64Image, String> {
    if bytes.get(0..2) != Some(&b"MZ"[..]) || bytes.len() < 0x40 {
        return Err(format!("{label} is not a DOS/PE image"));
    }
    let pe_offset = slice_u32(bytes, 0x3c, label)? as usize;
    if bytes.get(pe_offset..pe_offset.saturating_add(4)) != Some(&b"PE\0\0"[..]) {
        return Err(format!("{label} PE signature is invalid"));
    }
    let coff = pe_offset
        .checked_add(4)
        .ok_or_else(|| format!("{label} COFF offset overflow"))?;
    if slice_u16(bytes, coff, label)? != 0x8664 {
        return Err(format!("{label} is not an AMD64 image"));
    }
    let section_count = slice_u16(bytes, coff + 2, label)? as usize;
    if section_count == 0 || section_count > 96 {
        return Err(format!("{label} section count {section_count} is invalid"));
    }
    let optional_size = slice_u16(bytes, coff + 16, label)? as usize;
    let optional = coff
        .checked_add(20)
        .ok_or_else(|| format!("{label} optional-header offset overflow"))?;
    if optional_size < 64 || slice_u16(bytes, optional, label)? != 0x20b {
        return Err(format!("{label} is not PE32+"));
    }
    let image_size = slice_u32(bytes, optional + 56, label)?;
    if image_size == 0 || image_size % 0x1000 != 0 {
        return Err(format!("{label} SizeOfImage {image_size:#x} is invalid"));
    }
    let section_table = optional
        .checked_add(optional_size)
        .ok_or_else(|| format!("{label} section-table offset overflow"))?;
    let table_end = section_count
        .checked_mul(40)
        .and_then(|length| section_table.checked_add(length))
        .ok_or_else(|| format!("{label} section-table length overflow"))?;
    if table_end > bytes.len() {
        return Err(format!("{label} section table is truncated"));
    }
    let mut sections = Vec::with_capacity(section_count);
    for index in 0..section_count {
        let row = section_table + index * 40;
        let section = PeSection {
            virtual_size: slice_u32(bytes, row + 8, label)?,
            virtual_address: slice_u32(bytes, row + 12, label)?,
            raw_size: slice_u32(bytes, row + 16, label)?,
            raw_offset: slice_u32(bytes, row + 20, label)?,
        };
        let raw_end = (section.raw_offset as usize)
            .checked_add(section.raw_size as usize)
            .ok_or_else(|| format!("{label} section {index} raw extent overflow"))?;
        if raw_end > bytes.len() {
            return Err(format!("{label} section {index} raw extent is truncated"));
        }
        sections.push(section);
    }
    Ok(Pe64Image {
        image_size,
        sections,
    })
}

fn verify_supported_game_sentinels(bytes: &[u8], image: &Pe64Image) -> Result<(), String> {
    for (label, rva, expected) in [
        (
            "daily-update",
            compact_contract::build_profile::C_GAME_STATE_DAILY_UPDATE_OFFSET,
            compact_contract::build_profile::DAILY_UPDATE_SENTINEL.as_slice(),
        ),
        (
            "long-string",
            compact_contract::build_profile::GET_LONG_STRING_OFFSET,
            compact_contract::build_profile::GET_LONG_STRING_SENTINEL.as_slice(),
        ),
    ] {
        let offset = rva_to_file_offset(image, rva as u32)
            .ok_or_else(|| format!("game executable {label} sentinel RVA is not file-backed"))?;
        let actual = bytes
            .get(offset..offset.saturating_add(expected.len()))
            .ok_or_else(|| format!("game executable {label} sentinel is truncated"))?;
        if actual != expected {
            return Err(format!(
                "game executable {label} sentinel does not match supported build"
            ));
        }
    }
    Ok(())
}

fn rva_to_file_offset(image: &Pe64Image, rva: u32) -> Option<usize> {
    image.sections.iter().find_map(|section| {
        let relative = rva.checked_sub(section.virtual_address)?;
        if relative >= section.virtual_size || relative >= section.raw_size {
            return None;
        }
        section
            .raw_offset
            .checked_add(relative)
            .map(|offset| offset as usize)
    })
}

fn slice_u16(bytes: &[u8], offset: usize, label: &str) -> Result<u16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| format!("{label} u16 offset overflow"))?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| format!("{label} u16 at {offset:#x} is truncated"))?;
    Ok(u16::from_le_bytes(
        slice.try_into().expect("checked u16 slice"),
    ))
}

fn slice_u32(bytes: &[u8], offset: usize, label: &str) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| format!("{label} u32 offset overflow"))?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| format!("{label} u32 at {offset:#x} is truncated"))?;
    Ok(u32::from_le_bytes(
        slice.try_into().expect("checked u32 slice"),
    ))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn exact_object<'a>(
    value: &'a Value,
    label: &str,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label} is not an object"))?;
    let actual: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = keys.iter().copied().collect();
    if actual != expected {
        return Err(format!(
            "{label} keys are not exact (actual {actual:?}, expected {expected:?})"
        ));
    }
    Ok(object)
}

fn value<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a Value, String> {
    object
        .get(key)
        .ok_or_else(|| format!("source report missing {key}"))
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    value(object, key)?
        .as_str()
        .ok_or_else(|| format!("source report {key} is not a string"))
}

fn unsigned(object: &Map<String, Value>, key: &str) -> Result<u64, String> {
    value(object, key)?
        .as_u64()
        .ok_or_else(|| format!("source report {key} is not an unsigned integer"))
}

fn require_string(object: &Map<String, Value>, key: &str, expected: &str) -> Result<(), String> {
    let actual = string(object, key)?;
    if actual != expected {
        return Err(format!(
            "source report {key} {actual:?} != expected {expected:?}"
        ));
    }
    Ok(())
}

fn require_u64(object: &Map<String, Value>, key: &str, expected: u64) -> Result<(), String> {
    let actual = unsigned(object, key)?;
    if actual != expected {
        return Err(format!(
            "source report {key} {actual} != expected {expected}"
        ));
    }
    Ok(())
}

fn require_i64(object: &Map<String, Value>, key: &str, expected: i64) -> Result<(), String> {
    let actual = value(object, key)?
        .as_i64()
        .ok_or_else(|| format!("source report {key} is not an integer"))?;
    if actual != expected {
        return Err(format!(
            "source report {key} {actual} != expected {expected}"
        ));
    }
    Ok(())
}

fn validate_lower_hex(value: &str, length: usize, label: &str) -> Result<(), String> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not {length} lowercase hex characters"));
    }
    Ok(())
}

fn parse_canonical_hex_u64(value: &str, label: &str) -> Result<u64, String> {
    let digits = value
        .strip_prefix("0x")
        .ok_or_else(|| format!("{label} lacks 0x prefix"))?;
    if digits.is_empty()
        || digits.len() > 16
        || digits.starts_with('0')
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase hex"));
    }
    u64::from_str_radix(digits, 16).map_err(|_| format!("{label} does not fit u64"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const STORED: &[u8] = b"stored stream";
    const DECODED: &[u8] = b"decoded stream";

    fn expected() -> ExpectedStreamIdentity<'static> {
        ExpectedStreamIdentity {
            stored_stream: STORED,
            decoded_stream: DECODED,
            requested_game_hour_advancements: 2,
            closed_frames: 3,
            first_game_hour_raw: 100,
            last_game_hour_raw: 102,
        }
    }

    fn pe64_fixture(raw_size: u32, game_sentinels: bool) -> Vec<u8> {
        let pe_offset = 0x80usize;
        let coff = pe_offset + 4;
        let optional = coff + 20;
        let section = optional + 0xf0;
        let raw_offset = 0x200usize;
        let image_size = (0x1000u32 + raw_size + 0xfff) & !0xfff;
        let mut bytes = vec![0u8; raw_offset + raw_size as usize];
        bytes[0..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&(pe_offset as u32).to_le_bytes());
        bytes[pe_offset..pe_offset + 4].copy_from_slice(b"PE\0\0");
        bytes[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
        bytes[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        bytes[coff + 16..coff + 18].copy_from_slice(&0xf0u16.to_le_bytes());
        bytes[optional..optional + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        bytes[optional + 56..optional + 60].copy_from_slice(&image_size.to_le_bytes());
        bytes[section..section + 5].copy_from_slice(b".all\0");
        bytes[section + 8..section + 12].copy_from_slice(&raw_size.to_le_bytes());
        bytes[section + 12..section + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        bytes[section + 16..section + 20].copy_from_slice(&raw_size.to_le_bytes());
        bytes[section + 20..section + 24].copy_from_slice(&(raw_offset as u32).to_le_bytes());
        if game_sentinels {
            for (rva, sentinel) in [
                (
                    compact_contract::build_profile::C_GAME_STATE_DAILY_UPDATE_OFFSET,
                    compact_contract::build_profile::DAILY_UPDATE_SENTINEL.as_slice(),
                ),
                (
                    compact_contract::build_profile::GET_LONG_STRING_OFFSET,
                    compact_contract::build_profile::GET_LONG_STRING_SENTINEL.as_slice(),
                ),
            ] {
                let offset = raw_offset + rva - 0x1000;
                bytes[offset..offset + sentinel.len()].copy_from_slice(sentinel);
            }
        }
        bytes
    }

    fn dll() -> Vec<u8> {
        pe64_fixture(0x1000, false)
    }

    fn exe() -> Vec<u8> {
        pe64_fixture(0x530000, true)
    }

    fn report(dll: &[u8], exe: &[u8]) -> Value {
        let dll_image_size = parse_pe64(dll, "fixture DLL").unwrap().image_size;
        let exe_image_size = parse_pe64(exe, "fixture EXE").unwrap().image_size;
        json!({
            "schema": SOURCE_EVIDENCE_SCHEMA,
            "verdict": "passed",
            "source_kind": "dll_scan",
            "domain_id": 41,
            "session_id": "a4b17fed-ed12-4361-9064-fb6576638110",
            "evidence_nonce_hex": "12".repeat(32),
            "stream": {
                "stored_sha256": sha256_hex(STORED),
                "decoded_sha256": sha256_hex(DECODED),
                "requested_game_hour_advancements": 2,
                "closed_frames": 3,
                "first_game_hour_raw": 100,
                "last_game_hour_raw": 102
            },
            "collector": {
                "source_dll_sha256": sha256_hex(dll),
                "staged_dll_sha256": sha256_hex(dll),
                "loaded_dll_sha256": sha256_hex(dll),
                "pe_image_size": dll_image_size
            },
            "game": {
                "exe_sha256": sha256_hex(exe),
                "build_id": SOURCE_EVIDENCE_BUILD_ID,
                "process_id": 22764,
                "process_creation_filetime_utc": 133971854920000000u64,
                "module_base": "0x140000000",
                "module_size": exe_image_size,
                "module_backing_file_sha256": sha256_hex(exe)
            },
            "route": {
                "country_scientist_roster_offset": COUNTRY_OFF_SCIENTIST_ROSTER,
                "roster_vector_offset": SCIENTIST_ROSTER_VECTOR,
                "roster_vector_count_offset": SCIENTIST_ROSTER_VECTOR_COUNT,
                "wrapper_character_offset": SCIENTIST_ROSTER_WRAPPER_CHARACTER,
                "character_type_offset": CHARACTER_OFF_TYPE,
                "character_type": CHARACTER_TYPE,
                "character_scientist_offset": CHARACTER_OFF_SCIENTIST_PTR,
                "character_vtable_rva": CHARACTER_VTABLE_RVA_870EBF,
                "scientist_vtable_rva": SCIENTIST_VTABLE_RVA_870EBF,
                "scientist_skills_offset": SCIENTIST_OFF_SKILLS_PTR,
                "skill_header_base": SKILL_HEADER_BASE_870EBF
            },
            "capture": {
                "special_projects_scan": true,
                "projects_source_kind": "dll_scan",
                "bridge_rows_supplied": 0
            }
        })
    }

    fn verify(
        value: &Value,
        dll: &[u8],
        exe: &[u8],
    ) -> Result<VerifiedDllScanSourceEvidence, String> {
        verify_required_dll_scan_source_evidence(
            Some(&serde_json::to_vec(value).unwrap()),
            Some(dll),
            Some(exe),
            expected(),
        )
    }

    #[test]
    fn exact_source_report_binds_stream_dll_game_process_module_and_route() {
        let dll = dll();
        let exe = exe();
        let verified = verify(&report(&dll, &exe), &dll, &exe).unwrap();
        let report_bytes = serde_json::to_vec(&report(&dll, &exe)).unwrap();
        assert_eq!(verified.source_report_sha256, sha256_hex(&report_bytes));
        assert_eq!(verified.collector_dll_sha256, sha256_hex(&dll));
        assert_eq!(verified.game_exe_sha256, sha256_hex(&exe));
        assert_eq!(verified.process_id, 22764);
        assert_eq!(verified.module_base, 0x140000000);
    }

    #[test]
    fn missing_source_artifacts_never_certify() {
        let dll = dll();
        let exe = exe();
        let error =
            verify_required_dll_scan_source_evidence(None, Some(&dll), Some(&exe), expected())
                .unwrap_err();
        assert!(error.contains("missing authoritative"));
        let bytes = serde_json::to_vec(&report(&dll, &exe)).unwrap();
        assert!(verify_required_dll_scan_source_evidence(
            Some(&bytes),
            None,
            Some(&exe),
            expected()
        )
        .unwrap_err()
        .contains("missing exact collector"));
        assert!(verify_required_dll_scan_source_evidence(
            Some(&bytes),
            Some(&dll),
            None,
            expected()
        )
        .unwrap_err()
        .contains("missing exact game"));
    }

    #[test]
    fn forged_source_kind_verdict_route_or_extra_keys_are_rejected() {
        let dll = dll();
        let exe = exe();
        for mutate in [
            ("verdict", json!("failed")),
            ("source_kind", json!("bridge")),
        ] {
            let mut value = report(&dll, &exe);
            value[mutate.0] = mutate.1;
            assert!(verify(&value, &dll, &exe).is_err());
        }
        let mut route = report(&dll, &exe);
        route["route"]["country_scientist_roster_offset"] = json!(0);
        assert!(verify(&route, &dll, &exe)
            .unwrap_err()
            .contains("roster_offset"));
        let mut extra = report(&dll, &exe);
        extra["untrusted_claim"] = json!(true);
        assert!(verify(&extra, &dll, &exe)
            .unwrap_err()
            .contains("keys are not exact"));
    }

    #[test]
    fn mismatched_stream_dll_game_or_module_identity_is_rejected() {
        let dll = dll();
        let exe = exe();
        let mut stream = report(&dll, &exe);
        stream["stream"]["decoded_sha256"] = json!("00".repeat(32));
        assert!(verify(&stream, &dll, &exe)
            .unwrap_err()
            .contains("decoded_sha256"));
        let bytes = serde_json::to_vec(&report(&dll, &exe)).unwrap();
        assert!(verify_required_dll_scan_source_evidence(
            Some(&bytes),
            Some(&pe64_fixture(0x2000, false)),
            Some(&exe),
            expected()
        )
        .unwrap_err()
        .contains("source_dll_sha256"));
        let mut loaded_dll = report(&dll, &exe);
        loaded_dll["collector"]["loaded_dll_sha256"] = json!("aa".repeat(32));
        assert!(verify(&loaded_dll, &dll, &exe)
            .unwrap_err()
            .contains("loaded_dll_sha256"));
        let mut mismatched_exe = exe.clone();
        let last = mismatched_exe.len() - 1;
        mismatched_exe[last] ^= 0x5a;
        assert!(verify_required_dll_scan_source_evidence(
            Some(&bytes),
            Some(&dll),
            Some(&mismatched_exe),
            expected()
        )
        .unwrap_err()
        .contains("exe_sha256"));
        let mut module = report(&dll, &exe);
        module["game"]["module_backing_file_sha256"] = json!("ff".repeat(32));
        assert!(verify(&module, &dll, &exe)
            .unwrap_err()
            .contains("module_backing_file_sha256"));
        let mut module_size = report(&dll, &exe);
        module_size["game"]["module_size"] = json!(0x1000);
        assert!(verify(&module_size, &dll, &exe)
            .unwrap_err()
            .contains("module_size"));
    }

    #[test]
    fn malformed_pe_or_wrong_supported_build_sentinel_is_rejected() {
        let dll = dll();
        let exe = exe();
        let bytes = serde_json::to_vec(&report(&dll, &exe)).unwrap();
        assert!(verify_required_dll_scan_source_evidence(
            Some(&bytes),
            Some(b"not a PE"),
            Some(&exe),
            expected(),
        )
        .unwrap_err()
        .contains("DOS/PE"));

        let mut wrong_build = exe.clone();
        let sentinel_offset =
            0x200 + compact_contract::build_profile::C_GAME_STATE_DAILY_UPDATE_OFFSET - 0x1000;
        wrong_build[sentinel_offset] ^= 0xff;
        let wrong_report = serde_json::to_vec(&report(&dll, &wrong_build)).unwrap();
        assert!(verify_required_dll_scan_source_evidence(
            Some(&wrong_report),
            Some(&dll),
            Some(&wrong_build),
            expected(),
        )
        .unwrap_err()
        .contains("sentinel"));
    }
}
