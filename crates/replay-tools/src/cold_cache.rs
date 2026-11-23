use std::{fmt, fs::File, io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResetScope {
    FreshRecordingPerSample,
    FreshSourcePerSample,
    FreshRecordingAndIndexPerBatch,
    FreshSourcePreIndexedPerExchange,
}

impl ResetScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::FreshRecordingPerSample => "fresh recording per sample",
            Self::FreshSourcePerSample => "fresh source per sample",
            Self::FreshRecordingAndIndexPerBatch => "fresh recording and index per batch",
            Self::FreshSourcePreIndexedPerExchange => {
                "fresh source with pre-built indexes per exchange"
            }
        }
    }

    pub const fn detail(self) -> &'static str {
        match self {
            Self::FreshRecordingPerSample => {
                "Reopens the recording for each sample; process-wide caches (such as decoded executable tables) remain populated and the Windows system file cache is neither evicted nor verified"
            }
            Self::FreshSourcePerSample => {
                "Opens a fresh Source for each sample; process-wide caches (such as decoded executable tables) remain populated and the Windows system file cache is neither evicted nor verified"
            }
            Self::FreshRecordingAndIndexPerBatch => {
                "Opens a fresh recording and query index once per tile batch and shares them across that batch; process-wide caches (such as decoded executable tables) remain populated and the Windows system file cache is neither evicted nor verified"
            }
            Self::FreshSourcePreIndexedPerExchange => {
                "Opens a fresh Source and pre-builds answer indexes before each timed exchange; process-wide caches (such as decoded executable tables) remain populated and the Windows system file cache is neither evicted nor verified"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SystemCacheState {
    Unverified,
    VerifiedCold,
}

impl SystemCacheState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unverified => "system file cache unverified",
            Self::VerifiedCold => "system file cache verified cold",
        }
    }

    pub const fn verified_cold(self) -> bool {
        match self {
            Self::Unverified => false,
            Self::VerifiedCold => true,
        }
    }
}

impl fmt::Display for SystemCacheState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub reset_scope: ResetScope,
    pub reset_label: &'static str,
    pub process_caches_cleared: bool,
    pub system_cache: SystemCacheState,
    pub system_cache_verified: bool,
    pub system_cache_evicted: bool,
    pub label: String,
    pub file_bytes: u64,
    pub reason: &'static str,
}

pub fn probe_file(path: &Path, reset_scope: ResetScope) -> io::Result<Probe> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    let system_cache = SystemCacheState::Unverified;
    let reset_label = reset_scope.label();
    Ok(Probe {
        reset_scope,
        reset_label,
        process_caches_cleared: false,
        system_cache,
        system_cache_verified: system_cache.verified_cold(),
        system_cache_evicted: false,
        label: format!(
            "{reset_label} (process-wide caches retained; {})",
            system_cache.label()
        ),
        file_bytes: metadata.len(),
        reason: reset_scope.detail(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "battleplan-{prefix}-{}-{id}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn probe_file_reports_actual_reset_scope_and_surfaces_open_failures() {
        let dir = TempDir::new("cold-cache");
        let missing = dir.0.join("missing.battleplan");
        let err = probe_file(&missing, ResetScope::FreshRecordingPerSample).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        assert!(probe_file(&dir.0, ResetScope::FreshRecordingPerSample).is_err());

        let file = dir.0.join("sample.battleplan");
        std::fs::write(&file, b"recording bytes").unwrap();
        for (scope, slug, expected_reset) in [
            (
                ResetScope::FreshRecordingPerSample,
                "fresh-recording-per-sample",
                "fresh recording per sample",
            ),
            (
                ResetScope::FreshSourcePerSample,
                "fresh-source-per-sample",
                "fresh source per sample",
            ),
            (
                ResetScope::FreshRecordingAndIndexPerBatch,
                "fresh-recording-and-index-per-batch",
                "fresh recording and index per batch",
            ),
            (
                ResetScope::FreshSourcePreIndexedPerExchange,
                "fresh-source-pre-indexed-per-exchange",
                "fresh source with pre-built indexes per exchange",
            ),
        ] {
            let probe = probe_file(&file, scope).unwrap();
            assert_eq!(probe.reset_scope, scope);
            assert_eq!(probe.reset_label, expected_reset);
            assert!(!probe.process_caches_cleared);
            assert_eq!(probe.system_cache, SystemCacheState::Unverified);
            assert!(!probe.system_cache_verified);
            assert!(!probe.system_cache_evicted);
            assert_eq!(
                probe.label,
                format!("{expected_reset} (process-wide caches retained; system file cache unverified)")
            );
            assert_eq!(probe.file_bytes, 15);
            assert!(probe.reason.contains("process-wide caches"));

            let serialized = serde_json::to_value(&probe).unwrap();
            assert_eq!(serialized["resetScope"], slug);
            assert_eq!(serialized["processCachesCleared"], false);
            assert_eq!(serialized["systemCache"], "unverified");
            assert_eq!(serialized["systemCacheVerified"], false);
            assert_eq!(serialized["systemCacheEvicted"], false);
            assert_eq!(serialized["fileBytes"], 15);
        }
    }

    #[test]
    fn system_cache_state_variants_distinguish_verified_from_unverified() {
        assert!(!SystemCacheState::Unverified.verified_cold());
        assert!(SystemCacheState::VerifiedCold.verified_cold());
        assert_ne!(
            SystemCacheState::Unverified.label(),
            SystemCacheState::VerifiedCold.label()
        );
        assert_eq!(
            SystemCacheState::VerifiedCold.to_string(),
            "system file cache verified cold"
        );
    }
}
