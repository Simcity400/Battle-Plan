use std::{fmt, fs::File, io, path::Path};

pub const UNVERIFIED_SYSTEM_CACHE_REASON: &str = "Separate buffered opens clear only process-local caches; Windows standby/file cache eviction is not performed or verified";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheState {
    ProcessColdSystemUnverified,
    SystemColdVerified,
}

impl CacheState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProcessColdSystemUnverified => "process-cold (system file cache unverified)",
            Self::SystemColdVerified => "system-cold (verified)",
        }
    }

    pub const fn system_cache_verified(self) -> bool {
        match self {
            Self::ProcessColdSystemUnverified => false,
            Self::SystemColdVerified => true,
        }
    }
}

impl fmt::Display for CacheState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub state: CacheState,
    pub label: &'static str,
    pub system_cache_verified: bool,
    pub system_cache_evicted: bool,
    pub file_bytes: u64,
    pub reason: &'static str,
}

pub fn probe_file(path: &Path) -> io::Result<Probe> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    let state = CacheState::ProcessColdSystemUnverified;
    Ok(Probe {
        state,
        label: state.label(),
        system_cache_verified: state.system_cache_verified(),
        system_cache_evicted: false,
        file_bytes: metadata.len(),
        reason: UNVERIFIED_SYSTEM_CACHE_REASON,
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
    fn probe_file_reports_unverified_system_cache_and_surfaces_open_failures() {
        let dir = TempDir::new("cold-cache");
        let missing = dir.0.join("missing.battleplan");
        let err = probe_file(&missing).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        assert!(probe_file(&dir.0).is_err());

        let file = dir.0.join("sample.battleplan");
        std::fs::write(&file, b"recording bytes").unwrap();
        let probe = probe_file(&file).unwrap();
        assert_eq!(probe.state, CacheState::ProcessColdSystemUnverified);
        assert_eq!(probe.label, "process-cold (system file cache unverified)");
        assert_eq!(probe.state.to_string(), probe.label);
        assert!(!probe.system_cache_verified);
        assert!(!probe.system_cache_evicted);
        assert_eq!(probe.file_bytes, 15);
        assert_eq!(probe.reason, UNVERIFIED_SYSTEM_CACHE_REASON);

        let serialized = serde_json::to_value(&probe).unwrap();
        assert_eq!(serialized["state"], "process-cold-system-unverified");
        assert_eq!(serialized["label"], "process-cold (system file cache unverified)");
        assert_eq!(serialized["systemCacheVerified"], false);
        assert_eq!(serialized["systemCacheEvicted"], false);
        assert_eq!(serialized["fileBytes"], 15);
    }

    #[test]
    fn cache_state_variants_distinguish_verified_from_unverified_system_cache() {
        assert!(!CacheState::ProcessColdSystemUnverified.system_cache_verified());
        assert!(CacheState::SystemColdVerified.system_cache_verified());
        assert_ne!(
            CacheState::ProcessColdSystemUnverified.label(),
            CacheState::SystemColdVerified.label()
        );
        assert_eq!(CacheState::SystemColdVerified.to_string(), "system-cold (verified)");
    }
}
