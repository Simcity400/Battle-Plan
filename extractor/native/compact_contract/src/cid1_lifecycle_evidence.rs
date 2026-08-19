use std::path::PathBuf;

use crate::campaign_identity::Sha256;

pub const CID1_LIFECYCLE_EVIDENCE_ENABLE_ENV: &str = "BATTLE_PLAN_CID1_LIFECYCLE_EVIDENCE";
pub const CID1_LIFECYCLE_EVIDENCE_PERMIT_PATH_ENV: &str =
    "BATTLE_PLAN_CID1_LIFECYCLE_EVIDENCE_PERMIT_PATH";
pub const CID1_LIFECYCLE_EVIDENCE_PERMIT_SHA256_ENV: &str =
    "BATTLE_PLAN_CID1_LIFECYCLE_EVIDENCE_PERMIT_SHA256";

const PERMIT_HEADER: &str = "CID1-LIFECYCLE-EVIDENCE-PERMIT-V1";
const SUPPORTED_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const MAX_PERMIT_BYTES: usize = 1024;

fn is_lower_nonzero_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key)?.strip_prefix('=')
}

pub fn cid1_lifecycle_evidence_permit_enabled() -> bool {
    if !matches!(
        std::env::var(CID1_LIFECYCLE_EVIDENCE_ENABLE_ENV).as_deref(),
        Ok("1")
    ) {
        return false;
    }
    let Ok(path_text) = std::env::var(CID1_LIFECYCLE_EVIDENCE_PERMIT_PATH_ENV) else {
        return false;
    };
    let path = PathBuf::from(path_text);
    if !path.is_absolute() {
        return false;
    }
    let Ok(expected_permit_sha256) = std::env::var(CID1_LIFECYCLE_EVIDENCE_PERMIT_SHA256_ENV)
    else {
        return false;
    };
    if !is_lower_nonzero_sha256(&expected_permit_sha256) {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    if bytes.is_empty()
        || bytes.len() > MAX_PERMIT_BYTES
        || lower_hex(&Sha256::digest(&bytes)) != expected_permit_sha256
    {
        return false;
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    if text.contains('\r') || !text.ends_with('\n') {
        return false;
    }
    let lines: Vec<&str> = text[..text.len() - 1].split('\n').collect();
    if lines.len() != 8 || lines[0] != PERMIT_HEADER {
        return false;
    }
    let Some(arm) = value(lines[1], "arm") else {
        return false;
    };
    if !matches!(
        arm,
        "S1" | "S2" | "S3" | "S4" | "D1" | "D2" | "M1" | "M2" | "M3"
    ) {
        return false;
    }
    let Some(matrix_nonce) = value(lines[2], "matrix_nonce_hex") else {
        return false;
    };
    let Some(arm_nonce) = value(lines[3], "arm_nonce_hex") else {
        return false;
    };
    let Some(request_sha256) = value(lines[4], "request_sha256") else {
        return false;
    };
    let Some(owner_profile_sha256) = value(lines[5], "owner_profile_sha256") else {
        return false;
    };
    let Some(contract_sha256) = value(lines[6], "contract_sha256") else {
        return false;
    };
    let Some(executable_sha256) = value(lines[7], "pinned_executable_sha256") else {
        return false;
    };
    is_lower_nonzero_sha256(matrix_nonce)
        && is_lower_nonzero_sha256(arm_nonce)
        && matrix_nonce != arm_nonce
        && is_lower_nonzero_sha256(request_sha256)
        && is_lower_nonzero_sha256(owner_profile_sha256)
        && is_lower_nonzero_sha256(contract_sha256)
        && executable_sha256 == SUPPORTED_EXECUTABLE_SHA256
}
