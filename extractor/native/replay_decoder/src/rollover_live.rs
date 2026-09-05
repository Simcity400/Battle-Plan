use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use compact_stream::StreamCompression;
use recorder_host::duplex_session::rollover_fs::{open_path, sealed_path};
use snapshot::sha256::Sha256;

use super::{CertifiedReplayArtifacts, LiveReplayArtifactBuilder, StreamIdentityMode};

pub type RolloverFoldResults = BTreeMap<u64, Result<StoredArtifacts, String>>;

// Only these small custody receipts survive a member's fold; its hour maps
// live on disk until publication, so reloads cannot retain whole fold states.
#[derive(Debug)]
pub struct StoredArtifacts {
    pub spill_dir: PathBuf,
    receipt_path: PathBuf,
    receipt_len: u64,
    receipt_sha256: String,
}

impl StoredArtifacts {
    fn store(spill_dir: PathBuf, artifacts: &CertifiedReplayArtifacts) -> Result<Self, String> {
        let bytes = serde_json::to_vec(artifacts).map_err(|e| e.to_string())?;
        let receipt_path = spill_dir.join("live-artifacts.json");
        fs::write(&receipt_path, &bytes).map_err(|e| e.to_string())?;
        Ok(Self {
            spill_dir,
            receipt_path,
            receipt_len: bytes.len() as u64,
            receipt_sha256: Sha256::hex(&bytes),
        })
    }

    pub fn load(
        self,
        source_len: u64,
        source_sha256: &str,
    ) -> Result<CertifiedReplayArtifacts, String> {
        let file = File::open(&self.receipt_path).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() != self.receipt_len {
            return Err("rollover live artifact receipt length changed".into());
        }
        let mut bytes = Vec::new();
        file.take(self.receipt_len.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 != self.receipt_len || Sha256::hex(&bytes) != self.receipt_sha256 {
            return Err("rollover live artifact receipt digest changed".into());
        }
        let artifacts: CertifiedReplayArtifacts =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if artifacts.source_len != source_len || artifacts.source_sha256 != source_sha256 {
            return Err("rollover live artifacts disagree with sealed member identity".into());
        }
        Ok(artifacts)
    }
}

#[derive(Default)]
struct CaptureEnd {
    ended: Mutex<bool>,
    changed: Condvar,
}

impl CaptureEnd {
    fn finish(&self) {
        *self.ended.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.changed.notify_all();
    }

    fn ended(&self) -> bool {
        *self.ended.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wait(&self) {
        let ended = self.ended.lock().unwrap_or_else(|e| e.into_inner());
        if !*ended {
            let _ = self.changed.wait_timeout(ended, Duration::from_millis(20));
        }
    }
}

// The file handle survives the store's atomic open -> sealed rename. EOF on
// an open member means wait; only a seal or capture termination closes it.
struct MemberTail {
    file: File,
    sealed: PathBuf,
    end: Arc<CaptureEnd>,
}

impl Read for MemberTail {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            let n = self.file.read(bytes)?;
            if n != 0 {
                return Ok(n);
            }
            if self.sealed.try_exists()? || self.end.ended() {
                // The seal can race the first read; drain bytes written before
                // its rename before exposing the terminal EOF.
                return self.file.read(bytes);
            }
            self.end.wait();
        }
    }
}

pub struct RolloverLiveFold {
    end: Arc<CaptureEnd>,
    worker: Option<JoinHandle<RolloverFoldResults>>,
}

impl RolloverLiveFold {
    pub fn start(
        raw_root: PathBuf,
        spill_root: PathBuf,
        game_id: String,
        first_recording_id: u64,
        compression: StreamCompression,
        identity_mode: StreamIdentityMode,
        checkpoint_interval_hours: u32,
    ) -> Result<Self, String> {
        let end = Arc::new(CaptureEnd::default());
        let worker_end = end.clone();
        let worker = thread::Builder::new()
            .name("replay-rollover-fold".into())
            .spawn(move || {
                super::lower_current_thread_priority();
                let mut results = BTreeMap::new();
                let mut id = first_recording_id;
                loop {
                    let sealed = sealed_path(&raw_root, id);
                    let open = open_path(&raw_root, id);
                    let file = loop {
                        match File::open(&sealed).or_else(|e| {
                            if e.kind() == io::ErrorKind::NotFound {
                                File::open(&open)
                            } else {
                                Err(e)
                            }
                        }) {
                            Ok(file) => break Some(Ok(file)),
                            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                                if worker_end.ended() {
                                    // Recheck after observing completion: the rename
                                    // may have occurred between the two opens.
                                    break match File::open(&sealed) {
                                        Ok(file) => Some(Ok(file)),
                                        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                                        Err(e) => Some(Err(e)),
                                    };
                                }
                                worker_end.wait();
                            }
                            Err(e) => break Some(Err(e)),
                        }
                    };
                    let file = match file {
                        Some(Ok(file)) => file,
                        Some(Err(error)) => {
                            // No member was opened: do not invent later outcomes.
                            // Final packaging still folds every sealed descriptor member.
                            results.insert(id, Err(format!(
                                "rollover live member {id} open failed: {error}"
                            )));
                            break;
                        }
                        None => break,
                    };
                    let member_game_id = if id == first_recording_id {
                        game_id.clone()
                    } else {
                        format!("{game_id}-logical-stream-{id}")
                    };
                    let result = fold_member(
                        file,
                        &sealed,
                        &spill_root,
                        id,
                        &member_game_id,
                        compression,
                        identity_mode,
                        checkpoint_interval_hours,
                        worker_end.clone(),
                    );
                    results.insert(id, result);
                    let Some(next) = id.checked_add(1) else { break };
                    id = next;
                }
                results
            })
            .map_err(|e| format!("start rollover live fold: {e}"))?;
        Ok(Self {
            end,
            worker: Some(worker),
        })
    }

    pub fn close_capture(&self) {
        self.end.finish();
    }

    pub fn finish(mut self) -> Result<RolloverFoldResults, String> {
        self.end.finish();
        self.worker
            .take()
            .expect("one rollover fold worker")
            .join()
            .map_err(|_| "rollover live fold worker panicked".into())
    }
}

impl Drop for RolloverLiveFold {
    fn drop(&mut self) {
        self.end.finish();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn fold_member(
    file: File,
    sealed: &Path,
    spill_root: &Path,
    id: u64,
    game_id: &str,
    compression: StreamCompression,
    identity_mode: StreamIdentityMode,
    interval: u32,
    end: Arc<CaptureEnd>,
) -> Result<StoredArtifacts, String> {
    let spill = spill_root.join(format!("logical-stream-{id}"));
    fs::create_dir(&spill).map_err(|e| e.to_string())?;
    let refold = spill_root.join(format!("logical-stream-{id}-refold"));
    let mut refold_created = false;
    let attempt = (|| {
        let tail = MemberTail {
            file,
            sealed: sealed.to_path_buf(),
            end: end.clone(),
        };
        let builder = compact_stream::decode_reader(tail, compression)
            .map_err(|e| e.to_string())
            .and_then(|reader| {
                LiveReplayArtifactBuilder::start(
                    Box::new(reader),
                    compression == StreamCompression::Zstd,
                    0,
                    sealed,
                    &spill,
                    game_id,
                    identity_mode,
                    interval,
                )
                .map_err(|e| e.to_string())
            });
        let result = (|| {
            while !sealed.try_exists().map_err(|e| e.to_string())? && !end.ended() {
                end.wait();
            }
            let mut source = compact_stream::open(sealed).map_err(|e| e.to_string())?;
            let mut hash = Sha256::new();
            let mut source_len = 0u64;
            let mut buffer = vec![0; 1024 * 1024];
            loop {
                let n = source.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                source_len = source_len
                    .checked_add(n as u64)
                    .ok_or("rollover source length overflow")?;
                hash.update(&buffer[..n]);
            }
            let digest = snapshot::sha256::to_hex(&hash.finalize());
            Ok((source_len, digest))
        })();
        let (source_len, digest) = match result {
            Ok(receipt) => receipt,
            Err(error) => {
                // Source termination also releases and joins both fold threads.
                if let Ok(builder) = builder {
                    let _ = builder.finish(0, "");
                }
                return Err(error);
            }
        };
        let artifacts = match builder {
            Ok(builder) => builder
                .finish_or_refold(
                    sealed,
                    || {
                        fs::create_dir(&refold)?;
                        refold_created = true;
                        Ok(refold.clone())
                    },
                    source_len,
                    &digest,
                )
                .map(|(artifacts, _)| artifacts)
                .map_err(|e| e.to_string()),
            Err(refusal) => {
                fs::create_dir(&refold).map_err(|e| e.to_string())?;
                refold_created = true;
                super::build_certified_replay_artifacts(
                    sealed,
                    &refold,
                    game_id,
                    identity_mode,
                    source_len,
                    &digest,
                    interval,
                )
                .map(|mut artifacts| {
                    artifacts.live_seal_refusal = Some(refusal);
                    artifacts
                })
                .map_err(|e| e.to_string())
            }
        }?;
        let used_spill = if artifacts.live_seal_refusal.is_some() {
            let _ = fs::remove_dir_all(&spill);
            refold.clone()
        } else {
            spill.clone()
        };
        StoredArtifacts::store(used_spill, &artifacts)
    })();
    if attempt.is_err() {
        let _ = fs::remove_dir_all(&spill);
        if refold_created {
            let _ = fs::remove_dir_all(&refold);
        }
    }
    attempt
}

pub(crate) mod coverage_serde {
    use recorder_host::hour_coverage::HourCoverageMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        map: &HourCoverageMap,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        map.to_json().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<HourCoverageMap, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        HourCoverageMap::from_json(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // A broken finish/Drop join must fail the regression without hanging the suite.
    fn bounded_child(function: &str) -> bool {
        const CHILD: &str = "FIN_ROLLOVER_JOIN_REGRESSION";
        let (_, module) = module_path!().split_once("::").expect("test module below crate");
        let test_name = format!("{module}::{function}");
        if std::env::var(CHILD).as_deref() == Ok(test_name.as_str()) {
            return false;
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test_name, "--format=pretty", "--color=never", "--test-threads=1"])
            .env(CHILD, &test_name)
            .env_remove("RUST_TEST_NOCAPTURE")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                let mut output = String::new();
                child.stdout.take().unwrap().take(64 * 1024)
                    .read_to_string(&mut output).unwrap();
                assert!(status.success(), "{test_name}: {status}: {output}");
                assert!(child_ran_exact_test(&output, &test_name),
                    "{test_name}: child did not execute exactly the selected test: {output}");
                return true;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{test_name}: rollover worker failed to exit/join within 30 seconds");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn child_ran_exact_test(output: &str, test_name: &str) -> bool {
        let passed = format!("test {test_name} ... ok");
        output.lines().any(|line| line == passed)
            && output.lines().any(|line| line.starts_with("test result: ok. 1 passed; 0 failed;"))
    }

    #[test]
    fn bounded_child_rejects_zero_test_success_and_wrong_selection() {
        let name = "replay_decoder::rollover_live::tests::selected";
        assert!(!child_ran_exact_test(
            "running 0 tests\r\ntest result: ok. 0 passed; 0 failed; 0 ignored;\r\n", name));
        assert!(!child_ran_exact_test(
            "test other ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n", name));
        let passed = format!("test {name} ... ok\r\ntest result: ok. 1 passed; 0 failed; 0 ignored;\r\n");
        assert!(child_ran_exact_test(&passed, name));
        assert!(!child_ran_exact_test(&passed.replace("1 passed", "2 passed"), name));
    }

    #[test]
    fn rollover_open_failure_exits_and_joins_without_inventing_members() {
        if bounded_child("rollover_open_failure_exits_and_joins_without_inventing_members") {
            return;
        }
        struct FailedOpenFixture(PathBuf);
        impl Drop for FailedOpenFixture {
            fn drop(&mut self) {
                for id in [7, 8] {
                    #[cfg(windows)]
                    let _ = fs::remove_dir(sealed_path(&self.0, id));
                    #[cfg(unix)]
                    let _ = fs::remove_file(sealed_path(&self.0, id));
                }
                let _ = fs::remove_dir(&self.0);
            }
        }
        let root = std::env::temp_dir().join(format!(
            "fin-rollover-open-failure-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let fixture = FailedOpenFixture(root.clone());
        for id in [7, 8] {
            let path = sealed_path(&root, id);
            #[cfg(windows)]
            fs::create_dir(&path).unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(path.file_name().unwrap(), &path).unwrap();
            for _ in 0..3 {
                let error = File::open(&path).unwrap_err();
                #[cfg(windows)]
                {
                    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
                    assert_eq!(error.raw_os_error(), Some(5));
                }
                assert_ne!(error.kind(), io::ErrorKind::NotFound);
            }
        }
        let start = || RolloverLiveFold::start(
            root.clone(), root.clone(), "open-failure".into(), 7,
            StreamCompression::None, StreamIdentityMode::IdentityV1, 1,
        ).unwrap();
        let worker = start();
        // It must stop on the open error even while capture remains active.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !worker.worker.as_ref().unwrap().is_finished() {
            assert!(std::time::Instant::now() < deadline, "open error did not terminate worker");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!worker.end.ended());
        let results = worker.finish().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results.keys().copied().collect::<Vec<_>>(), [7]);
        assert!(results[&7].as_ref().unwrap_err().contains("member 7 open failed"));
        // Completion and Drop can also race the failing open; both must join.
        let results = start().finish().unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[&7].is_err());
        drop(start());
        drop(fixture);
        assert!(!root.try_exists().unwrap());
    }

    #[test]
    fn rollover_decode_failure_still_attempts_later_sealed_members() {
        if bounded_child("rollover_decode_failure_still_attempts_later_sealed_members") {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "fin-rollover-decode-failure-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let spill = root.join("spill");
        fs::create_dir(&spill).unwrap();
        fs::write(sealed_path(&root, 7), b"invalid compact stream").unwrap();
        let stream = super::super::tests::three_hour_stream();
        fs::write(sealed_path(&root, 8), &stream).unwrap();
        let worker = RolloverLiveFold::start(
            root.clone(), spill, "decode-failure".into(), 7,
            StreamCompression::None, StreamIdentityMode::Legacy, 2,
        ).unwrap();
        let mut results = worker.finish().unwrap();
        assert_eq!(results.keys().copied().collect::<Vec<_>>(), [7, 8]);
        let error = results.remove(&7).unwrap().unwrap_err();
        assert!(!error.contains("open failed"), "{error}");
        let later = results.remove(&8).unwrap().unwrap()
            .load(stream.len() as u64, &Sha256::hex(&stream)).unwrap();
        assert_eq!(later.hour_count, 3);
        assert_eq!(fs::read(sealed_path(&root, 7)).unwrap(), b"invalid compact stream");
        assert_eq!(fs::read(sealed_path(&root, 8)).unwrap(), stream);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn member_tail_reads_during_capture_and_drains_the_rename_boundary() {
        let root = std::env::temp_dir().join(format!(
            "fin-member-tail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let open = open_path(&root, 0);
        let sealed = sealed_path(&root, 0);
        let mut writer = File::create(&open).unwrap();
        writer.write_all(b"prefix").unwrap();
        writer.flush().unwrap();
        let end = Arc::new(CaptureEnd::default());
        let mut tail = MemberTail {
            file: File::open(&open).unwrap(),
            sealed: sealed.clone(),
            end: end.clone(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = thread::spawn(move || {
            let mut prefix = [0; 6];
            let result = tail.read_exact(&mut prefix);
            tx.send(result.map(|_| prefix)).unwrap();
            let mut suffix = Vec::new();
            tail.read_to_end(&mut suffix).unwrap();
            suffix
        });
        let prefix = rx.recv_timeout(Duration::from_secs(5));
        if prefix.is_err() {
            end.finish();
        }
        assert_eq!(prefix.unwrap().unwrap(), *b"prefix");
        writer.write_all(b"suffix").unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        fs::rename(&open, &sealed).unwrap();
        assert_eq!(reader.join().unwrap(), b"suffix");
        fs::remove_dir_all(root).unwrap();
    }
}
