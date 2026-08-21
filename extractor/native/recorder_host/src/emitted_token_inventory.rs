use crate::domain_data_authority::{
    collect_static_join_tokens, DomainDataContext, StaticJoinToken,
};
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode,
};
use serde_json::json;
use snapshot::sha256::{to_hex, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const EMITTED_TOKEN_INVENTORY_SCHEMA: &str = "extractor.emitted_token_inventory.v1";
const MAX_RECORD_LEN: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedEmittedTokenInventory {
    pub path: PathBuf,
    pub sha256: String,
    pub token_count: usize,
    pub decoded_stream_bytes: u64,
}

pub fn build_and_write(
    stream_path: &Path,
    output_path: &Path,
    capture_id: &str,
    session_id: &str,
    expected_stream_sha256: &str,
    identity_mode: StreamIdentityMode,
) -> Result<VerifiedEmittedTokenInventory, String> {
    require_nonempty(capture_id, "capture_id")?;
    require_nonempty(session_id, "session_id")?;
    validate_sha256(expected_stream_sha256, "expected stream SHA-256")?;
    if output_path.exists() {
        return Err(format!(
            "emitted token inventory output already exists: {}",
            output_path.display()
        ));
    }

    let (tokens, decoded_stream_sha256, decoded_stream_bytes) =
        scan_stream(stream_path, identity_mode)?;
    if decoded_stream_sha256 != expected_stream_sha256 {
        return Err(format!(
            "decoded stream SHA-256 {} does not match capture {}",
            decoded_stream_sha256, expected_stream_sha256
        ));
    }
    if tokens.is_empty() {
        return Err(
            "completed capture emitted no static-join string tokens; refusing stale/empty inventory"
                .to_string(),
        );
    }

    let rows = tokens
        .iter()
        .map(|row| json!({"kind": row.kind, "token": row.token}))
        .collect::<Vec<_>>();
    let mut bytes = serde_json::to_vec_pretty(&json!({
        "schema": EMITTED_TOKEN_INVENTORY_SCHEMA,
        "provenance": {
            "capture_id": capture_id,
            "session_id": session_id,
            "stream_sha256": decoded_stream_sha256,
        },
        "tokens": rows,
    }))
    .map_err(|error| format!("serialize emitted token inventory failed: {error}"))?;
    bytes.push(b'\n');
    write_create_new_durable(output_path, &bytes)?;
    Ok(VerifiedEmittedTokenInventory {
        path: output_path.to_path_buf(),
        sha256: snapshot::sha256::Sha256::hex(&bytes),
        token_count: tokens.len(),
        decoded_stream_bytes,
    })
}

fn scan_stream(
    stream_path: &Path,
    identity_mode: StreamIdentityMode,
) -> Result<(BTreeSet<StaticJoinToken>, String, u64), String> {
    let mut reader = compact_stream::open(stream_path)
        .map_err(|error| format!("open emitted-token stream failed: {error}"))?;
    let mut validator = match identity_mode {
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
    };
    let mut hasher = Sha256::new();
    let mut decoded_bytes = 0u64;
    let mut stream_offset = 0usize;
    let mut tokens = BTreeSet::new();
    loop {
        let mut header_bytes = [0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
        let count = read_up_to(&mut reader, &mut header_bytes)
            .map_err(|error| format!("read emitted-token H4CR header failed: {error}"))?;
        if count == 0 {
            break;
        }
        if count != header_bytes.len() {
            return Err(format!(
                "emitted-token stream has truncated H4CR header at offset {stream_offset}"
            ));
        }
        let header = compact_contract::parse_compact_record_header(&header_bytes)
            .map_err(|error| format!("H4CR header at offset {stream_offset} failed: {error}"))?;
        let record_len = header.record_len();
        if record_len > MAX_RECORD_LEN {
            return Err(format!(
                "H4CR record at offset {stream_offset} exceeds {MAX_RECORD_LEN} byte bound"
            ));
        }
        let mut payload = vec![0u8; header.payload_len];
        let payload_count = read_up_to(&mut reader, &mut payload).map_err(|error| {
            format!("read H4CR payload at offset {stream_offset} failed: {error}")
        })?;
        if payload_count != payload.len() {
            return Err(format!(
                "emitted-token stream has truncated H4CR payload at offset {stream_offset}"
            ));
        }
        hasher.update(&header_bytes);
        hasher.update(&payload);
        decoded_bytes = decoded_bytes.saturating_add(record_len as u64);

        match validator
            .validate_record(&header, &payload)
            .map_err(|error| format!("H4CR validation at offset {stream_offset} failed: {error}"))?
        {
            AcceptedRecordKind::CampaignIdentity => {}
            AcceptedRecordKind::Ordinary
                if header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA =>
            {
                tokens.extend(
                    collect_static_join_tokens(
                        header.domain_id,
                        &payload,
                        DomainDataContext {
                            stream_offset,
                            record_flags: header.record_flags,
                            sequence: header.sequence,
                            header_hour_raw: header.game_hour_raw,
                            session_header_identity: None,
                            // Token collection only; no changed-row count is read.
                            xmap07_snow_mud_previous_grid_sha256: None,
                        },
                    )
                    .map_err(|error| {
                        format!("static-join DATA decode failed at offset {stream_offset}: {error}")
                    })?,
                );
            }
            AcceptedRecordKind::Ordinary => {}
        }
        stream_offset = stream_offset
            .checked_add(record_len)
            .ok_or_else(|| "decoded stream offset overflow".to_string())?;
    }
    Ok((tokens, to_hex(&hasher.finalize()), decoded_bytes))
}

fn read_up_to(reader: &mut impl Read, output: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0usize;
    while filled < output.len() {
        match reader.read(&mut output[filled..])? {
            0 => break,
            count => filled += count,
        }
    }
    Ok(filled)
}

fn write_create_new_durable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create inventory directory failed: {error}"))?;
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("create emitted token inventory failed: {error}"))?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|error| format!("write emitted token inventory failed: {error}"))?;
        file.flush()
            .map_err(|error| format!("flush emitted token inventory failed: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync emitted token inventory failed: {error}"))
    })();
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    result
}

fn require_nonempty(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(())
    }
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("{label} is not canonical lowercase SHA-256"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_provenance_rejects_empty_or_noncanonical_inputs_before_io() {
        let path = Path::new("missing-stream.bin");
        let out = Path::new("unused-inventory.json");
        assert!(build_and_write(
            path,
            out,
            "",
            "session",
            &"a".repeat(64),
            StreamIdentityMode::Legacy
        )
        .unwrap_err()
        .contains("capture_id"));
        assert!(build_and_write(
            path,
            out,
            "capture",
            "session",
            "AA",
            StreamIdentityMode::Legacy
        )
        .unwrap_err()
        .contains("canonical lowercase"));
    }
}
