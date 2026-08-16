//! Per-attempt payload staging (design §5 "Unique DLL path").
//!
//! Every attach attempt copies the packaged collector DLL into a fresh,
//! never-before-used directory. A live incident proved that reusing a DLL path
//! already hooked into a running game crashes the game; unique per-attempt
//! directories fixed it (memory: `live-capture-2nd-injection-crash`,
//! `PRODUCTION_ENDGAME_PLAN.md:211-239`). The staged copy's SHA-256 is verified
//! against the source; a mismatch fails closed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{fs::OpenOptions, io::Write};

use snapshot::sha256::{to_hex, Sha256};
use serde::Deserialize;

use super::identity::ProcessKey;
use super::sources::{PayloadStager, StagedPayload};

/// Process-global monotonic counter making generated ids unique even within the
/// same millisecond.
static ATTEMPT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a collision-resistant attempt id from (instance, pid, filetime,
/// wall-ms, counter). Not a security token — just a never-repeating directory /
/// pipe discriminator (design §7 "Generate random session/attempt ids").
pub fn generate_attempt_id(instance_id: &str, key: ProcessKey, now_ms: u64) -> String {
    let n = ATTEMPT_COUNTER.fetch_add(1, Ordering::Relaxed);
    // Hash the composite so the id is fixed-width and filesystem-safe.
    let mut h = Sha256::new();
    h.update(instance_id.as_bytes());
    h.update(&key.pid.to_le_bytes());
    h.update(&key.creation_filetime.0.to_le_bytes());
    h.update(&now_ms.to_le_bytes());
    h.update(&n.to_le_bytes());
    let hex = to_hex(&h.finalize());
    hex[..24].to_string()
}

/// Generate a random-ish pipe token (directory-unique discriminator) and return
/// (raw_token, sha256_hex). The raw token is used to name the pipe; only the
/// hash is ever journalled (design §8).
pub fn generate_secret_128() -> Result<[u8; 16], String> {
    #[cfg(windows)]
    {
        const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
        #[link(name = "bcrypt")]
        extern "system" {
            fn BCryptGenRandom(
                algorithm: *mut std::ffi::c_void,
                buffer: *mut u8,
                len: u32,
                flags: u32,
            ) -> i32;
        }
        let mut bytes = [0u8; 16];
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status < 0 || bytes == [0; 16] {
            return Err(format!(
                "BCryptGenRandom failed with NTSTATUS {status:#010x}"
            ));
        }
        Ok(bytes)
    }
    #[cfg(not(windows))]
    {
        Err("auto-attach secrets require the Windows CSPRNG".to_string())
    }
}

pub fn generate_pipe_token() -> Result<(String, String), String> {
    let token = to_hex(&generate_secret_128()?);
    let mut h2 = Sha256::new();
    h2.update(token.as_bytes());
    let token_hash = to_hex(&h2.finalize());
    Ok((token, token_hash))
}

fn hash_file(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(to_hex(&h.finalize()))
}

fn persist_identity_commitment(
    staging_root: &std::path::Path,
    key: ProcessKey,
    attempt_id: &str,
    dll_sha256: &str,
    pipe_token_hash: &str,
) -> Result<PathBuf, String> {
    let root = staging_root.join("identity_commitments");
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("create identity commitment root {}: {e}", root.display()))?;
    let path = root.join(format!(
        "pid_{}_filetime_{}.json",
        key.pid, key.creation_filetime.0
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            format!(
                "identity {}:{} already has an injection commitment or cannot be committed at {}: {e}",
                key.pid,
                key.creation_filetime.0,
                path.display()
            )
        })?;
    let mut bytes = serde_json::to_vec(&serde_json::json!({
        "schema": "auto_attach_identity_commitment.v1",
        "pid": key.pid,
        "process_creation_filetime": key.creation_filetime.0,
        "attempt_id": attempt_id,
        "dll_sha256": dll_sha256,
        "pipe_token_hash": pipe_token_hash,
    }))
    .map_err(|e| format!("serialize identity commitment: {e}"))?;
    bytes.push(b'\n');
    file.write_all(&bytes)
        .map_err(|e| format!("write identity commitment {}: {e}", path.display()))?;
    file.flush()
        .map_err(|e| format!("flush identity commitment {}: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| format!("sync identity commitment {}: {e}", path.display()))?;
    Ok(path)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityCommitment {
    schema: String,
    pid: u32,
    process_creation_filetime: u64,
    attempt_id: String,
    dll_sha256: String,
    pipe_token_hash: String,
}

fn validate_identity_commitment(
    path: &std::path::Path,
    key: ProcessKey,
    expected_attempt_id: &str,
    expected_dll_sha256: Option<&str>,
) -> Result<(), String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("read identity commitment {}: {error}", path.display()))?;
    let commitment: IdentityCommitment = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse identity commitment {}: {error}", path.display()))?;
    if commitment.schema != "auto_attach_identity_commitment.v1"
        || commitment.pid != key.pid
        || commitment.process_creation_filetime != key.creation_filetime.0
        || commitment.attempt_id != expected_attempt_id
        || commitment.dll_sha256.len() != 64
        || !commitment
            .dll_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || expected_dll_sha256
            .is_some_and(|expected| commitment.dll_sha256 != expected)
        || commitment.pipe_token_hash.len() != 64
        || !commitment
            .pipe_token_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!(
            "identity commitment {} does not bind the proven-unloaded generation",
            path.display()
        ));
    }
    Ok(())
}

fn has_retired_identity_commitment(
    root: &std::path::Path,
    key: ProcessKey,
) -> Result<bool, String> {
    if !root.is_dir() {
        return Ok(false);
    }
    let prefix = format!(
        "pid_{}_filetime_{}.retired_",
        key.pid, key.creation_filetime.0
    );
    let mut found = false;
    for entry in std::fs::read_dir(root)
        .map_err(|error| format!("enumerate identity commitments {}: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| {
            format!("enumerate identity commitment entry {}: {error}", root.display())
        })?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(attempt_id) = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".json"))
        else {
            continue;
        };
        if attempt_id.is_empty() {
            return Err("retired identity commitment has an empty attempt id".into());
        }
        let bytes = std::fs::read(entry.path()).map_err(|error| {
            format!(
                "read retired identity commitment {}: {error}",
                entry.path().display()
            )
        })?;
        let commitment: IdentityCommitment = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "parse retired identity commitment {}: {error}",
                entry.path().display()
            )
        })?;
        if commitment.schema != "auto_attach_identity_commitment.v1"
            || commitment.pid != key.pid
            || commitment.process_creation_filetime != key.creation_filetime.0
            || commitment.attempt_id != attempt_id
            || commitment.dll_sha256.len() != 64
        {
            return Err(format!(
                "retired identity commitment {} is malformed or misbound",
                entry.path().display()
            ));
        }
        found = true;
    }
    Ok(found)
}

/// Filesystem-backed stager. Copies `source_dll` into a fresh directory under
/// `staging_root` per attempt.
pub struct FsPayloadStager {
    source_dll: PathBuf,
    staging_root: PathBuf,
}

impl FsPayloadStager {
    pub fn new(source_dll: PathBuf, staging_root: PathBuf, _now_ms: u64) -> Self {
        Self {
            source_dll,
            staging_root,
        }
    }
}

impl PayloadStager for FsPayloadStager {
    fn rearm_after_proven_unload(
        &self,
        key: ProcessKey,
        prior_dll_path: &std::path::Path,
    ) -> Result<(), String> {
        let attempt_dir = prior_dll_path
            .parent()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .ok_or("prior staged DLL has no UTF-8 attempt directory")?;
        let attempt_id = attempt_dir
            .strip_prefix("attempt_")
            .filter(|value| !value.is_empty())
            .ok_or("prior staged DLL directory is not an attempt directory")?;
        // Capture cleanup may remove the completed attempt directory after the
        // collector has been proven absent from the retained process. The
        // durable identity commitment, not disposable staged payload bytes, is
        // the generation authority in that case. If the old DLL still exists,
        // retain the stronger byte-for-byte hash cross-check.
        let expected_dll_sha256 = prior_dll_path
            .is_file()
            .then(|| hash_file(prior_dll_path))
            .transpose()?;
        let root = self.staging_root.join("identity_commitments");
        let active = root.join(format!(
            "pid_{}_filetime_{}.json",
            key.pid, key.creation_filetime.0
        ));
        let retired = root.join(format!(
            "pid_{}_filetime_{}.retired_{}.json",
            key.pid, key.creation_filetime.0, attempt_id
        ));
        if !active.exists() {
            validate_identity_commitment(
                &retired,
                key,
                attempt_id,
                expected_dll_sha256.as_deref(),
            )?;
            return Ok(());
        }
        validate_identity_commitment(
            &active,
            key,
            attempt_id,
            expected_dll_sha256.as_deref(),
        )?;
        std::fs::rename(&active, &retired).map_err(|error| {
            format!(
                "retire proven-unloaded identity commitment {} -> {}: {error}",
                active.display(),
                retired.display()
            )
        })
    }

    fn stage(
        &self,
        key: ProcessKey,
        attempt_id: &str,
        pipe_token: &str,
        pipe_token_hash: &str,
    ) -> Result<StagedPayload, String> {
        std::fs::create_dir_all(&self.staging_root)
            .map_err(|e| format!("create staging root {}: {e}", self.staging_root.display()))?;
        let staging_dir = self.staging_root.join(format!("attempt_{attempt_id}"));
        // A fresh directory per attempt: creation MUST fail if it already exists
        // (design §5 "Atomically create a never-before-used attempt/staging
        // directory"; gate 8 "existing destination fail closed").
        std::fs::create_dir(&staging_dir).map_err(|e| {
            format!(
                "staging dir {} already exists or cannot be created: {e}",
                staging_dir.display()
            )
        })?;
        let source_sha256 = hash_file(&self.source_dll)?;
        let dll_path = staging_dir.join("collector.dll");
        std::fs::copy(&self.source_dll, &dll_path)
            .map_err(|e| format!("copy collector dll: {e}"))?;
        let dll_sha256 = hash_file(&dll_path)?;
        if dll_sha256 != source_sha256 {
            // Corrupt copy — remove and fail closed (gate 8).
            let _ = std::fs::remove_dir_all(&staging_dir);
            return Err(format!(
                "staged dll hash {dll_sha256} != source {source_sha256}"
            ));
        }
        let identity_commitment_root = self.staging_root.join("identity_commitments");
        let rearmed_after_proven_unload =
            has_retired_identity_commitment(&identity_commitment_root, key)?;
        // This durable create-new marker is independent of both journal files.
        // Once staged bytes could be authorized for a process identity, no
        // rewritten/rolled-back journal can authorize another attempt for that
        // same retained PID+FILETIME. Any uncertainty is a permanent refusal for
        // that process, never a retry.
        if let Err(error) = persist_identity_commitment(
            &self.staging_root,
            key,
            attempt_id,
            &dll_sha256,
            pipe_token_hash,
        ) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            return Err(error);
        }
        let control_path = staging_dir.join("collector_control.json");
        Ok(StagedPayload {
            staging_dir,
            dll_path,
            control_path,
            dll_sha256,
            source_sha256,
            pipe_token: pipe_token.to_string(),
            pipe_token_hash: pipe_token_hash.to_string(),
            rearmed_after_proven_unload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn attempt_ids_do_not_collide_over_1000() {
        // Design gate 8: 1,000 generated attempt paths do not collide.
        let key = ProcessKey::new(1234, 133_000_000_000_000_000);
        let mut seen = HashSet::new();
        for i in 0..1000 {
            let id = generate_attempt_id("coord-inst", key, 10_000 + i);
            assert!(seen.insert(id), "attempt id collision at {i}");
        }
    }

    #[test]
    fn attempt_ids_unique_even_same_inputs() {
        // Same (instance, key, now) still differ thanks to the counter.
        let key = ProcessKey::new(1, 2);
        let a = generate_attempt_id("c", key, 5);
        let b = generate_attempt_id("c", key, 5);
        assert_ne!(a, b);
    }

    #[test]
    fn pipe_token_hash_is_not_the_raw_token() {
        let (raw, hash) = generate_pipe_token().unwrap();
        assert_ne!(raw, hash);
        assert_eq!(hash.len(), 64);
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "aa_staging_{tag}_{}_{}",
            std::process::id(),
            fastish()
        ));
        p
    }
    fn fastish() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    }

    #[test]
    fn stage_copies_and_verifies_hash() {
        let root = tmp_dir("ok");
        let src = root.join("src_collector.dll");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&src, b"fake collector payload bytes").unwrap();
        let stager = FsPayloadStager::new(src.clone(), root.join("stage"), 1);
        let key = ProcessKey::new(9, 9);
        let payload = stager
            .stage(key, "attempt-abc", "token", "token-hash")
            .unwrap();
        assert!(payload.dll_path.exists());
        assert_eq!(payload.dll_sha256, payload.source_sha256);
        assert!(payload.staging_dir.ends_with("attempt_attempt-abc"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn proven_unload_retires_exact_commitment_and_allows_one_new_generation() {
        let root = tmp_dir("rearm");
        let src = root.join("src_collector.dll");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&src, b"fake collector payload bytes").unwrap();
        let stager = FsPayloadStager::new(src, root.join("stage"), 1);
        let key = ProcessKey::new(10, 11);
        let first = stager
            .stage(key, "first", "token-a", &"ab".repeat(32))
            .unwrap();
        stager
            .rearm_after_proven_unload(key, &first.dll_path)
            .unwrap();
        // Crash-idempotent: the active marker is already retired, but the exact
        // retired generation still validates.
        stager
            .rearm_after_proven_unload(key, &first.dll_path)
            .unwrap();
        let second = stager
            .stage(key, "second", "token-b", &"cd".repeat(32))
            .unwrap();
        assert_ne!(first.staging_dir, second.staging_dir);
        assert!(root
            .join("stage")
            .join("identity_commitments")
            .join("pid_10_filetime_11.retired_first.json")
            .is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn proven_unload_rearms_after_disposable_attempt_directory_was_removed() {
        let root = tmp_dir("rearm_removed_attempt");
        let src = root.join("src_collector.dll");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&src, b"fake collector payload bytes").unwrap();
        let stager = FsPayloadStager::new(src, root.join("stage"), 1);
        let key = ProcessKey::new(12, 13);
        let first = stager
            .stage(key, "first", "token-a", &"ab".repeat(32))
            .unwrap();

        // Completed capture data and its staged DLL are disposable. The
        // process-identity commitment remains durable so a recorder-only
        // restart can retire the proven-unloaded generation without depending
        // on deleted payload bytes.
        portable_test_fs_retry::remove_dir_all(&first.staging_dir).unwrap();
        assert!(!first.dll_path.exists());
        stager
            .rearm_after_proven_unload(key, &first.dll_path)
            .unwrap();
        let second = stager
            .stage(key, "second", "token-b", &"cd".repeat(32))
            .unwrap();

        assert_ne!(first.staging_dir, second.staging_dir);
        assert!(root
            .join("stage")
            .join("identity_commitments")
            .join("pid_12_filetime_13.retired_first.json")
            .is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stage_fails_on_existing_destination() {
        let root = tmp_dir("dup");
        let src = root.join("src.dll");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&src, b"bytes").unwrap();
        let stage_root = root.join("stage");
        let stager = FsPayloadStager::new(src.clone(), stage_root.clone(), 1);
        let key = ProcessKey::new(1, 1);
        stager.stage(key, "same", "token", "token-hash").unwrap();
        // Second stage with the SAME attempt id must fail closed (dir exists).
        let err = stager
            .stage(key, "same", "token", "token-hash")
            .unwrap_err();
        assert!(err.contains("already exists") || err.contains("cannot be created"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stage_refuses_different_attempt_for_committed_process_identity() {
        let root = tmp_dir("identity_poison");
        let src = root.join("src.dll");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&src, b"bytes").unwrap();
        let stager = FsPayloadStager::new(src, root.join("stage"), 1);
        let key = ProcessKey::new(55, 5500);
        stager
            .stage(key, "first", "token-a", "token-hash-a")
            .unwrap();
        let error = stager
            .stage(key, "second", "token-b", "token-hash-b")
            .expect_err("same process identity must retain the first injection poison");
        assert!(
            error.contains("already has an injection commitment"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
