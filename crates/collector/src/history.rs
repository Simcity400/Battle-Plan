//! Prepare exact prior branch history before installing the hourly hook.

use crate::spooling::campaign_name;
use replay::{
    branches::BranchTracker,
    codec::invalid,
    container::Replay,
    identity::{Identity, SessionStart},
};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

struct Prepared {
    campaign_id: Vec<u8>,
    identity: Identity,
    previous_session: Option<SessionStart>,
    tracker: BranchTracker,
    lease: File,
}

#[derive(Default)]
pub struct PreparedCampaigns {
    directory: PathBuf,
    entries: BTreeMap<String, Result<Prepared, String>>,
}

impl PreparedCampaigns {
    pub fn load(
        directory: &Path,
        session: &SessionStart,
        mut progress: impl FnMut(&str) -> io::Result<()>,
    ) -> io::Result<Self> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file()
                || !path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("battleplan") || ext.eq_ignore_ascii_case("hoi4replay"))
            {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if let Ok(id) = campaign_name(stem.as_bytes()) {
                engine::allocation::reserve(&mut paths, 1)?;
                paths.push((id, path));
            }
        }
        paths.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let mut entries = BTreeMap::new();
        for (id, path) in paths {
            let deadline = Instant::now() + Duration::from_secs(2);
            let prepared = loop {
                progress(&id)?;
                match Self::read(&path, &id, session).map_err(engine::allocation::allocation_error)
                {
                    Err(error)
                        if matches!(error.raw_os_error(), Some(5 | 32 | 33))
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(error) if error.kind() == io::ErrorKind::OutOfMemory => return Err(error),
                    result => break result.map_err(|error| error.to_string()),
                }
            };
            if entries.insert(id.to_ascii_lowercase(), prepared).is_some() {
                return Err(invalid("duplicate campaign replay filename"));
            }
        }
        Ok(Self {
            directory: directory.into(),
            entries,
        })
    }

    fn read(path: &Path, name: &str, session: &SessionStart) -> io::Result<Prepared> {
        // Readers remain allowed. Keep this handle until detach/process exit so
        // the history used for parentage cannot change beneath the collector.
        let mut lease = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)?;
        let mut bytes = Vec::new();
        let length =
            usize::try_from(lease.metadata()?.len()).map_err(|_| engine::allocation::oom())?;
        engine::allocation::reserve_exact(&mut bytes, length)?;
        let mut chunk = [0_u8; 8192];
        loop {
            let count = match lease.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            engine::allocation::reserve(&mut bytes, count)?;
            bytes.extend_from_slice(&chunk[..count]);
        }
        let history = Replay::open(bytes)?.into_history()?;
        if !history.campaign_id.eq_ignore_ascii_case(name.as_bytes()) {
            return Err(invalid(
                "existing replay filename differs from its campaign id",
            ));
        }
        let previous_session = history
            .sessions
            .iter()
            .find(|old| old.id == session.id)
            .cloned();
        // The callback has not verified the in-memory content roster yet. Seed
        // a resumed process using its old metadata, then compare the actual new
        // observation before consuming this prepared tracker.
        let tracker = history.tracker_for(previous_session.as_ref().unwrap_or(session))?;
        Ok(Prepared {
            campaign_id: history.campaign_id,
            identity: history.identity,
            previous_session,
            tracker,
            lease,
        })
    }

    pub fn take(
        &mut self,
        id: &str,
        session: &SessionStart,
    ) -> io::Result<(BranchTracker, Option<File>)> {
        let key = id.to_ascii_lowercase();
        match self.entries.get(&key) {
            Some(Err(reason)) => return Err(invalid(reason.clone())),
            Some(Ok(prepared)) => {
                if prepared.campaign_id != id.as_bytes() {
                    return Err(invalid("existing replay has a different campaign id"));
                }
                if !prepared.identity.same_content(&session.identity) {
                    return Err(invalid("existing campaign replay has different content"));
                }
                if prepared
                    .previous_session
                    .as_ref()
                    .is_some_and(|old| old != session)
                {
                    return Err(invalid(
                        "same process session has conflicting collector or content metadata",
                    ));
                }
            }
            None => {
                if self
                    .directory
                    .join(format!("{id}.battleplan"))
                    .try_exists()?
                {
                    return Err(invalid(
                        "campaign replay appeared after history preparation; restart the recorder",
                    ));
                }
                let mut tracker = BranchTracker::new();
                tracker.begin_session(session.id)?;
                return Ok((tracker, None));
            }
        }
        let prepared = self.entries.remove(&key).unwrap().map_err(invalid)?;
        Ok((prepared.tracker, Some(prepared.lease)))
    }
}
