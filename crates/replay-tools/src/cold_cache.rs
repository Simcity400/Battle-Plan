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

