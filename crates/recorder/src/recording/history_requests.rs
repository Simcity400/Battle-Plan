use replay_settings::publication::ReplaceError;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

const WAIT: Duration = Duration::from_secs(180);
const RETURN_WAIT: Duration = WAIT;
static NEXT: AtomicU64 = AtomicU64::new(0);
type Loans = BTreeMap<(PathBuf, String), HistoryRequest>;
static LOANS: OnceLock<Mutex<Loans>> = OnceLock::new();
fn loans() -> &'static Mutex<Loans> {
    LOANS.get_or_init(Mutex::default)
}

pub(super) fn open_shared(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
}

fn read_shared(path: &Path) -> io::Result<String> {
    let mut text = String::new();
    open_shared(path)?.read_to_string(&mut text)?;
    Ok(text)
}

fn publish(path: &Path, bytes: &[u8]) -> io::Result<()> {
    publish_with(path, bytes, replay_settings::publication::replace_file).map_err(io::Error::from)
}

fn publish_with(
    path: &Path,
    bytes: &[u8],
    replace: impl FnOnce(&Path, &Path) -> Result<(), ReplaceError>,
) -> Result<(), ReplaceError> {
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension().unwrap_or_default().to_string_lossy()
    ));
    File::create(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .map_err(ReplaceError::Unrenamed)?;
    replace(&temporary, path)
}

#[derive(Clone)]
pub(super) struct HistoryRequest {
    directory: PathBuf,
    token: String,
    action: String,
    campaign: String,
}

impl HistoryRequest {
    pub(super) fn publish(session: &Path, action: &str, subject: &str) -> Result<Self, String> {
        Self::publish_using(session, action, subject, replay_settings::publication::replace_file)
    }

    fn publish_using(
        session: &Path,
        action: &str,
        subject: &str,
        replace: impl FnOnce(&Path, &Path) -> Result<(), ReplaceError>,
    ) -> Result<Self, String> {
        let directory = session.join("history-requests");
        fs::create_dir_all(&directory).map_err(|e| super::failure_text(&directory, e))?;
        let request = Self {
            directory,
            token: format!(
                "{:020}-{}",
                NEXT.fetch_add(1, Ordering::Relaxed),
                std::process::id()
            ),
            action: action.to_owned(),
            campaign: subject.to_owned(),
        };
        let mut pending = super::locked(loans());
        let key = (request.directory.clone(), request.campaign.clone());
        let loan = matches!(action, "lend" | "release");
        if loan && pending.contains_key(&key) {
            return Err(format!(
                "campaign {subject} still has an unresolved history return"
            ));
        }
        let path = request.path("request");
        let published = publish_with(
            &path,
            format!("{} {action} {subject}", request.token).as_bytes(),
            replace,
        );
        if loan && !matches!(published, Err(ReplaceError::Unrenamed(_))) {
            pending.insert(key, request.clone());
        }
        published.map_err(|e| super::failure_text(&path, e))?;
        Ok(request)
    }

    pub(super) fn lent(session: &Path, campaign: &str) -> Option<Self> {
        super::locked(loans())
            .get(&(session.join("history-requests"), campaign.to_owned()))
            .cloned()
    }

    pub(super) fn retry_returns(
        session: &Path,
        campaign: &str,
        stopped: &AtomicBool,
    ) -> Result<(), String> {
        if let Some(previous) = Self::lent(session, campaign) {
            previous.retake(stopped)?;
        }
        Ok(())
    }

    pub(super) fn retire_session(session: &Path, collector_absent: bool, process_exited: bool) {
        if collector_absent || process_exited {
            let directory = session.join("history-requests");
            super::locked(loans()).retain(|(owner, _), _| *owner != directory);
        }
    }

    fn path(&self, extension: &str) -> PathBuf {
        self.directory.join(&self.token).with_extension(extension)
    }

    fn answer(&self, returning: bool) -> Option<Result<(), String>> {
        if self.directory.join("detached").exists() {
            return Some(if returning {
                Ok(())
            } else {
                Err(format!(
                    "collector detached before {} was acknowledged",
                    self.action
                ))
            });
        }
        let path = self.path(if returning { "returned" } else { "ack" });
        let text = read_shared(&path)
            .or_else(|_| {
                read_shared(
                    &self
                        .directory
                        .parent()
                        .unwrap()
                        .join("history-completed")
                        .join(path.file_name().unwrap()),
                )
            })
            .ok()?;
        match text.split_once(' ') {
            Some((token, "ok")) if token == self.token => Some(Ok(())),
            Some((token, answer)) if token == self.token && answer.starts_with("refused ") => {
                Some(Err(answer[8..].to_owned()))
            }
            _ => None,
        }
    }

    pub(super) fn wait(&self, stopped: &AtomicBool) -> Result<(), String> {
        self.wait_with(
            false,
            WAIT,
            Instant::now,
            || stopped.load(Ordering::Acquire),
            || {
                std::thread::sleep(Duration::from_millis(50));
            },
        )
    }

    fn wait_with(
        &self,
        returning: bool,
        budget: Duration,
        mut now: impl FnMut() -> Instant,
        mut stopped: impl FnMut() -> bool,
        mut pause: impl FnMut(),
    ) -> Result<(), String> {
        let began = now();
        let deadline = began + budget;
        loop {
            if !returning && stopped() {
                return Err(format!(
                    "{} cancelled: the capture ended first",
                    self.action
                ));
            }
            if let Some(answer) = self.answer(returning) {
                return answer;
            }
            if now() >= deadline || (returning && stopped()) {
                let stage = if returning { "retake" } else { &self.action };
                let detail = read_shared(&self.path("waiting")).unwrap_or_default();
                return Err(format!(
                    "{stage} acknowledgement pending at {} after {:.0} s; durable request retained{}",
                    self.path(if returning { "return" } else { "request" })
                        .display(),
                    now().saturating_duration_since(began).as_secs_f64(),
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    },
                ));
            }
            pause();
        }
    }

    pub(super) fn return_history(&self) -> Result<(), String> {
        let path = self.path("return");
        if !path.exists() {
            publish(&path, self.token.as_bytes()).map_err(|e| super::failure_text(&path, e))?;
        }
        Ok(())
    }

    pub(super) fn retake(&self, stopped: &AtomicBool) -> Result<(), String> {
        if matches!(self.answer(true), Some(Ok(()))) {
            self.forget();
            return Ok(());
        }
        self.return_history()?;
        let result = self.wait_with(
            true,
            RETURN_WAIT,
            Instant::now,
            || stopped.load(Ordering::Acquire),
            || {
                std::thread::sleep(Duration::from_millis(50));
            },
        );
        if result.is_ok() {
            self.forget();
        }
        result
    }

    fn forget(&self) {
        let mut pending = super::locked(loans());
        let key = (self.directory.clone(), self.campaign.clone());
        if pending
            .get(&key)
            .is_some_and(|request| request.token == self.token)
        {
            pending.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn delayed_acknowledgement_and_cancellation_keep_the_exact_request() {
        let root = crate::test_support::allocation_test_directory("history-journal");
        let request = HistoryRequest::publish(&root, "lend", "campaign").unwrap();
        let start = Instant::now();
        let elapsed = Cell::new(0);
        request
            .wait_with(
                false,
                WAIT,
                || start + Duration::from_secs(elapsed.get()),
                || false,
                || {
                    elapsed.set(45);
                    publish(
                        &request.path("ack"),
                        format!("{} ok", request.token).as_bytes(),
                    )
                    .unwrap();
                },
            )
            .unwrap();
        assert_eq!(elapsed.get(), 45);
        let other = HistoryRequest::publish(&root, "close", "other").unwrap();
        assert_ne!(request.path("request"), other.path("request"));
        assert!(request.path("request").exists());
        assert!(
            other
                .wait_with(false, WAIT, || start, || true, || panic!("cancelled"))
                .is_err()
        );
        other.return_history().unwrap();
        assert!(other.path("return").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retake_deadline_is_bounded_and_survives_late_acknowledgement_and_detach() {
        let root = crate::test_support::allocation_test_directory("history-return");
        let request = HistoryRequest::publish(&root, "lend", "campaign").unwrap();
        request.return_history().unwrap();
        let start = Instant::now();
        let elapsed = Cell::new(Duration::ZERO);
        let error = request
            .wait_with(
                true,
                RETURN_WAIT,
                || start + elapsed.get(),
                || false,
                || {
                    elapsed.set(RETURN_WAIT);
                },
            )
            .unwrap_err();
        assert!(error.contains("retake acknowledgement pending"));
        assert!(request.path("request").exists());
        assert!(request.path("return").exists());
        publish(
            &request.path("returned"),
            format!("{} ok", request.token).as_bytes(),
        )
        .unwrap();
        request.retake(&AtomicBool::new(true)).unwrap();
        publish(&request.directory.join("detached"), b"released").unwrap();
        assert!(
            request
                .wait(&AtomicBool::new(false))
                .unwrap_err()
                .contains("detached")
        );
        request.retake(&AtomicBool::new(true)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_sharing_reader_allows_atomic_replacement() {
        let root = crate::test_support::allocation_test_directory("history-shared-reader");
        let path = root.join("progress.json");
        publish(&path, b"old").unwrap();
        let reader = open_shared(&path).unwrap();
        publish(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        drop(reader);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn final_spool_done_keeps_tokens_until_confirmed_unload_or_game_exit() {
        use super::super::{CollectorPresence, refresh_unload_completion};
        let unloaded = crate::test_support::allocation_test_directory("done-loans-unloaded");
        let exited = crate::test_support::allocation_test_directory("done-loans-exited");
        for root in [&unloaded, &exited] {
            HistoryRequest::publish(root, "lend", "campaign").unwrap();
            super::super::write_json(
                &root.join("completed.json"),
                &serde_json::json!({"drain_complete":true,"unload_pending":true,"warnings":[]}),
            )
            .unwrap();
        }
        let contains = |root: &Path| {
            super::super::locked(loans())
                .contains_key(&(root.join("history-requests"), "campaign".to_owned()))
        };
        for collector_loaded in [Some(true), None] {
            refresh_unload_completion(
                &unloaded,
                CollectorPresence {
                    process_exited: false,
                    collector_loaded,
                },
            )
            .unwrap();
            assert!(contains(&unloaded));
            assert!(contains(&exited));
        }
        refresh_unload_completion(
            &unloaded,
            CollectorPresence {
                process_exited: false,
                collector_loaded: Some(false),
            },
        )
        .unwrap();
        assert!(!contains(&unloaded));
        assert!(contains(&exited));
        refresh_unload_completion(&exited, CollectorPresence::exited()).unwrap();
        assert!(!contains(&exited));
        for root in [unloaded, exited] {
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn completed_sessions_retire_only_their_tokens_after_unload_or_game_exit() {
        let unloaded = crate::test_support::allocation_test_directory("loans-unloaded");
        let exited = crate::test_support::allocation_test_directory("loans-exited");
        let live = crate::test_support::allocation_test_directory("loans-live");
        let requests: Vec<_> = [&unloaded, &exited, &live]
            .into_iter()
            .map(|root| HistoryRequest::publish(root, "lend", "campaign").unwrap())
            .collect();
        for request in &requests {
            fs::create_dir(request.path("return.tmp")).unwrap();
            assert!(request.retake(&AtomicBool::new(true)).is_err());
        }
        let contains = |root: &Path| {
            super::super::locked(loans())
                .contains_key(&(root.join("history-requests"), "campaign".to_owned()))
        };
        HistoryRequest::retire_session(&unloaded, false, false);
        assert!(contains(&unloaded));
        HistoryRequest::retire_session(&unloaded, true, false);
        assert!(!contains(&unloaded));
        assert!(contains(&exited));
        assert!(contains(&live));
        HistoryRequest::retire_session(&exited, false, true);
        assert!(!contains(&exited));
        assert!(contains(&live));
        HistoryRequest::retire_session(&live, false, false);
        assert!(contains(&live));
        publish(
            &requests[2].path("returned"),
            format!("{} ok", requests[2].token).as_bytes(),
        )
        .unwrap();
        requests[2].retake(&AtomicBool::new(true)).unwrap();
        assert!(!contains(&live));
        for root in [unloaded, exited, live] {
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn failed_explicit_and_guard_returns_survive_until_the_next_loan() {
        let root = crate::test_support::allocation_test_directory("history-unpublished-return");
        let request = HistoryRequest::publish(&root, "lend", "campaign").unwrap();
        let token = request.token.clone();
        let obstacle = request.path("return.tmp");
        fs::create_dir(&obstacle).unwrap();
        let stopped = AtomicBool::new(true);
        let mut returns = 0;
        assert!(
            super::super::with_lent_history(
                |action| {
                    if action == "retake" {
                        returns += 1;
                        request.retake(&stopped)
                    } else {
                        Ok(())
                    }
                },
                || Ok(())
            )
            .is_err()
        );
        assert_eq!(returns, 2);
        drop(request);
        let key = (root.join("history-requests"), "campaign".to_owned());
        assert_eq!(
            super::super::locked(loans()).get(&key).unwrap().token,
            token
        );
        assert!(HistoryRequest::publish(&root, "release", "campaign").is_err());
        fs::remove_dir(obstacle).unwrap();
        assert!(HistoryRequest::retry_returns(&root, "campaign", &stopped).is_err());
        let request = super::super::locked(loans()).get(&key).unwrap().clone();
        assert!(request.path("return").exists());
        assert_eq!(request.token, token);
        let archive = root.join("history-completed");
        fs::create_dir(&archive).unwrap();
        publish(
            &archive.join(request.path("returned").file_name().unwrap()),
            format!("{token} ok").as_bytes(),
        )
        .unwrap();
        HistoryRequest::retry_returns(&root, "campaign", &stopped).unwrap();
        assert!(!super::super::locked(loans()).contains_key(&key));
        let next = HistoryRequest::publish(&root, "lend", "campaign").unwrap();
        assert_ne!(next.token, token);
        publish(
            &next.path("returned"),
            format!("{} ok", next.token).as_bytes(),
        )
        .unwrap();
        next.retake(&stopped).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unrenamed_request_is_never_lent_even_when_its_staged_file_is_gone() {
        let root = crate::test_support::allocation_test_directory("history-unrenamed-loan");
        let refused = || -> Result<(), ReplaceError> {
            Err(ReplaceError::Unrenamed(io::Error::other("rename refused")))
        };
        assert!(
            HistoryRequest::publish_using(&root, "release", "campaign", |_, _| refused()).is_err()
        );
        assert!(
            HistoryRequest::publish_using(&root, "release", "campaign", |from, _| {
                fs::remove_file(from).unwrap();
                refused()
            })
            .is_err()
        );
        assert!(HistoryRequest::lent(&root, "campaign").is_none());
        assert!(
            fs::read_dir(root.join("history-requests"))
                .unwrap()
                .all(|entry| {
                    entry.unwrap().path().extension() != Some(std::ffi::OsStr::new("request"))
                })
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_loan_renamed_before_its_flush_failed_is_still_returned() {
        let root = crate::test_support::allocation_test_directory("history-unflushed-loan");
        let stopped = AtomicBool::new(true);
        assert!(
            HistoryRequest::publish_using(&root, "release", "campaign", |from, to| {
                replay_settings::publication::replace_file(from, to)?;
                Err(ReplaceError::Unflushed(io::Error::other("flush failed")))
            })
            .is_err()
        );
        let request = HistoryRequest::lent(&root, "campaign").unwrap();
        assert!(request.path("request").exists());
        assert!(HistoryRequest::publish(&root, "lend", "campaign").is_err());
        assert!(HistoryRequest::retry_returns(&root, "campaign", &stopped).is_err());
        assert!(request.path("return").exists());
        publish(
            &request.path("returned"),
            format!("{} ok", request.token).as_bytes(),
        )
        .unwrap();
        HistoryRequest::retry_returns(&root, "campaign", &stopped).unwrap();
        assert!(HistoryRequest::lent(&root, "campaign").is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
