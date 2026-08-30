//! Names the process holding the handle when a packaging open is refused with
//! ERROR_SHARING_VIOLATION.
//!
//! The refusal carries no holder: os-32 is raised alike by a writer, a deleter
//! and an opener whose own share mode denies the read
//! (`extractor/native/runs/ve_flake_family/sharing_violation_mechanism.txt`).
//! The Restart Manager is asked who holds the refused path, and only on the
//! refusal: every entry point here returns the caller's message untouched
//! unless the OS code is 32. All callers are post-capture packaging paths, so
//! the query cannot make the engine wait, and the query is bounded so a stalled
//! Restart Manager cannot swallow the refusal either.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub const ERROR_SHARING_VIOLATION: i32 = 32;

/// Most holders named in one refusal message.
const MAX_NAMED_HOLDERS: usize = 8;

/// Longest one refusal waits for the holder query. Past it the refusal fires
/// naming the timeout.
const HOLDER_QUERY_BUDGET: Duration = Duration::from_millis(2000);

/// Most holder queries that may be outstanding at once. A query abandoned at
/// the budget keeps its thread, and any Restart Manager session it started,
/// until the call it is blocked in returns; past this bound a refusal names
/// the backlog rather than adding to it.
const MAX_OUTSTANDING_QUERIES: usize = 4;

static OUTSTANDING_QUERIES: AtomicUsize = AtomicUsize::new(0);

/// The work one query thread does.
type Query = Box<dyn FnOnce() + Send + 'static>;

/// One query's place in the outstanding bound, given back when the query ends
/// unless it left a Restart Manager session behind.
struct OutstandingSlot {
    counter: &'static AtomicUsize,
    release: bool,
}

impl Drop for OutstandingSlot {
    fn drop(&mut self) {
        if self.release {
            self.counter.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HolderProcess {
    pub pid: u32,
    pub app: String,
}

/// One holder query's outcome. `session_close_failure` is separate because a
/// query that answered but could not release its Restart Manager session has
/// both an answer and a leak to report.
#[derive(Debug)]
pub struct HolderQuery {
    pub holders: Result<Vec<HolderProcess>, String>,
    pub session_close_failure: Option<String>,
}

impl HolderQuery {
    fn failed(reason: impl Into<String>) -> Self {
        Self {
            holders: Err(reason.into()),
            session_close_failure: None,
        }
    }
}

/// The ancestor component a retained container open was refused on, recovered
/// from the wrapped error so the holder query names the path that actually
/// refused rather than the container the caller asked for.
fn retained_ancestor_refusal(
    error: &std::io::Error,
) -> Option<&snapshot::container::RetainedAncestorRefusal> {
    error
        .get_ref()?
        .downcast_ref::<snapshot::container::RetainedAncestorRefusal>()
}

/// 32 is ERROR_SHARING_VIOLATION on Windows and an unrelated errno elsewhere,
/// so off Windows no error is one.
fn is_sharing_violation(code: Option<i32>) -> bool {
    cfg!(windows) && code == Some(ERROR_SHARING_VIOLATION)
}

/// The paths a sharing violation could have been refused on, empty when this
/// error is not one. A retained-open refusal names the exact component itself,
/// so it answers alone; otherwise every path the caller was operating on is a
/// candidate.
fn refused_paths(error: &std::io::Error, attempted: &[&Path]) -> Vec<PathBuf> {
    if let Some(refusal) = retained_ancestor_refusal(error) {
        if is_sharing_violation(refusal.raw_os_error()) {
            return vec![refusal.path().to_path_buf()];
        }
        return Vec::new();
    }
    if is_sharing_violation(error.raw_os_error()) {
        return attempted.iter().map(|path| path.to_path_buf()).collect();
    }
    Vec::new()
}

/// `message` with the holder of the refused path appended, when `error` is a
/// sharing violation. Any other error returns `message` unchanged and asks the
/// Restart Manager nothing.
pub fn with_holder(message: String, error: &std::io::Error, attempted: &Path) -> String {
    with_holders(message, error, &[attempted])
}

/// The same for an operation with more than one path the OS could have been
/// refused on - a rename is refused on its source as readily as on its
/// destination - so every candidate is named.
pub fn with_holders(message: String, error: &std::io::Error, attempted: &[&Path]) -> String {
    with_holders_using(
        message,
        error,
        attempted,
        HOLDER_QUERY_BUDGET,
        holder_clause,
    )
}

/// The one decision both entry points make, with the clause builder supplied
/// so a test can prove the builder is never reached for a non-sharing error.
fn with_holders_using(
    message: String,
    error: &std::io::Error,
    attempted: &[&Path],
    budget: Duration,
    mut clause: impl FnMut(&Path, Duration) -> String,
) -> String {
    let refused = refused_paths(error, attempted);
    if refused.is_empty() {
        return message;
    }
    // ONE deadline for the whole refusal, not one per candidate: naming two
    // paths may not cost twice the time a refusal is allowed to spend.
    let deadline = std::time::Instant::now() + budget;
    let clauses = refused
        .iter()
        .map(|path| {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            clause(path, remaining)
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("{message}; {clauses}")
}

/// The same for a container refusal, whose io error is one variant of it.
pub fn with_container_holder(
    message: String,
    error: &snapshot::container::ContainerError,
    attempted: &Path,
) -> String {
    match error {
        snapshot::container::ContainerError::Io(io) => with_holder(message, io, attempted),
        _ => message,
    }
}

fn holder_clause(refused: &Path, budget: Duration) -> String {
    holder_clause_within(
        refused,
        budget,
        &OUTSTANDING_QUERIES,
        enumerate_holders,
        spawn_query,
    )
}

/// Starting the query thread is fallible on purpose: a refusal must survive a
/// process that cannot spawn.
fn spawn_query(query: Query) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("holder-query".to_string())
        .spawn(query)
        .map(|_| ())
}

/// Runs the query off this thread so a Restart Manager call that never returns
/// costs the refusal `budget`, not the refusal itself. A query thread that
/// outlives the budget is abandoned - the packaging attempt is already over -
/// and `outstanding` bounds how many such threads, and the sessions they hold,
/// can pile up.
fn holder_clause_within<F, S>(
    refused: &Path,
    budget: Duration,
    outstanding: &'static AtomicUsize,
    enumerate: F,
    spawn: S,
) -> String
where
    F: FnOnce(&Path) -> HolderQuery + Send + 'static,
    S: FnOnce(Query) -> std::io::Result<()>,
{
    if budget.is_zero() {
        // A candidate that finds the refusal's shared budget already spent
        // says so rather than starting a query it cannot wait for.
        return format!(
            "the holder of {} could not be enumerated: no time was left in this \
             refusal's holder-query budget",
            refused.display()
        );
    }
    let already = outstanding.fetch_add(1, Ordering::SeqCst);
    if already >= MAX_OUTSTANDING_QUERIES {
        outstanding.fetch_sub(1, Ordering::SeqCst);
        return format!(
            "the holder of {} could not be enumerated: {already} earlier Restart Manager \
             quer(ies) have not returned or did not release their session, which is \
             the bound this refusal will not add to",
            refused.display()
        );
    }
    let queried = refused.to_path_buf();
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = spawn(Box::new(move || {
        // Held across the query so a panicking one gives its slot back, and
        // kept for good by one that leaked a session, so leaked sessions are
        // bounded by the same number as live query threads.
        let mut slot = OutstandingSlot {
            counter: outstanding,
            release: true,
        };
        let query = enumerate(&queried);
        slot.release = query.session_close_failure.is_none();
        let _ = sender.send(query);
    }));
    if let Err(error) = spawned {
        outstanding.fetch_sub(1, Ordering::SeqCst);
        return format!(
            "the holder of {} could not be enumerated: the query thread could not be \
             started: {error}",
            refused.display()
        );
    }
    match receiver.recv_timeout(budget) {
        Ok(query) => describe_holders(refused, &query),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => format!(
            "the holder of {} could not be enumerated: the Restart Manager query did not \
             return within {} ms",
            refused.display(),
            budget.as_millis()
        ),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => format!(
            "the holder of {} could not be enumerated: the query ended without an answer",
            refused.display()
        ),
    }
}

/// Names the holder, or names why it could not be named.
fn describe_holders(refused: &Path, query: &HolderQuery) -> String {
    let clause = match &query.holders {
        Ok(holders) if holders.is_empty() => format!(
            "no user-mode process holds {}: the holder is a kernel filter, a handle the \
             Restart Manager cannot enumerate, or a user-mode holder that closed between the \
             refusal and this query",
            refused.display()
        ),
        Ok(holders) => {
            let named = holders
                .iter()
                .take(MAX_NAMED_HOLDERS)
                .map(|holder| {
                    let app = if holder.app.is_empty() {
                        "<no application or service name reported>"
                    } else {
                        holder.app.as_str()
                    };
                    format!("pid={} app={app}", holder.pid)
                })
                .collect::<Vec<_>>()
                .join(", ");
            let overflow = holders.len().saturating_sub(MAX_NAMED_HOLDERS);
            if overflow == 0 {
                format!("{} is held open by {named}", refused.display())
            } else {
                format!(
                    "{} is held open by {named}, and {overflow} further process(es) not listed",
                    refused.display()
                )
            }
        }
        Err(reason) => format!(
            "the holder of {} could not be enumerated: {reason}",
            refused.display()
        ),
    };
    match &query.session_close_failure {
        Some(failure) => format!("{clause} (the Restart Manager session did not close: {failure})"),
        None => clause,
    }
}

/// The Restart Manager takes full paths only, so a relative refused path is
/// resolved before it is registered.
fn absolute_for_query(path: &Path) -> Result<PathBuf, String> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| {
                format!(
                    "the working directory could not be read to resolve the relative refused \
                     path: {error}"
                )
            })?
            .join(path)
    };
    Ok(lexically_normalised(&joined))
}

/// Drops `.` and resolves `..` textually. The path names a file some other
/// process holds open, so it is not resolved through the filesystem: doing so
/// would be one more open of the thing that just refused one.
fn lexically_normalised(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The name a holder is reported under: the Restart Manager's application
/// name, else the short service name, else empty for the caller to explain.
fn holder_name(app_name: &[u16], service_short_name: &[u16]) -> String {
    for field in [app_name, service_short_name] {
        let text = String::from_utf16_lossy(field);
        // A fixed-width NUL-terminated field: the name ends at the first NUL.
        let name = text.split('\0').next().unwrap_or_default().trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    String::new()
}

#[cfg(windows)]
mod restart_manager {
    use super::{holder_name, HolderProcess, HolderQuery};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    type Dword = u32;

    const CCH_RM_MAX_APP_NAME: usize = 255;
    const CCH_RM_MAX_SVC_NAME: usize = 63;
    const ERROR_MORE_DATA: Dword = 234;
    /// Bounds one refusal's query: the buffer the Restart Manager asks for is
    /// its own, and a session that wants more than this is refused rather than
    /// allocated for.
    const MAX_REPORTED_PROCESSES: u32 = 4096;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FileTime {
        low: Dword,
        high: Dword,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RmUniqueProcess {
        pid: Dword,
        start_time: FileTime,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RmProcessInfo {
        process: RmUniqueProcess,
        app_name: [u16; CCH_RM_MAX_APP_NAME + 1],
        service_short_name: [u16; CCH_RM_MAX_SVC_NAME + 1],
        application_type: u32,
        app_status: u32,
        tssession_id: Dword,
        restartable: i32,
    }

    #[link(name = "rstrtmgr")]
    extern "system" {
        fn RmStartSession(session_handle: *mut Dword, flags: Dword, key: *mut u16) -> Dword;
        fn RmEndSession(session_handle: Dword) -> Dword;
        fn RmRegisterResources(
            session_handle: Dword,
            num_files: u32,
            files: *const *const u16,
            num_processes: u32,
            processes: *const RmUniqueProcess,
            num_services: u32,
            service_names: *const *const u16,
        ) -> Dword;
        fn RmGetList(
            session_handle: Dword,
            proc_info_needed: *mut u32,
            proc_info: *mut u32,
            process_info: *mut RmProcessInfo,
            reboot_reasons: *mut Dword,
        ) -> Dword;
    }

    pub(super) fn holders(path: &Path) -> HolderQuery {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if wide[..wide.len() - 1].contains(&0) {
            return HolderQuery {
                holders: Err("the refused path contains an interior NUL".to_string()),
                session_close_failure: None,
            };
        }
        let mut handle: Dword = 0;
        // RmStartSession writes a session key of CCH_RM_SESSION_KEY * 2 + 1
        // UTF-16 units.
        let mut key = [0u16; 33];
        let started = unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) };
        if started != 0 {
            return HolderQuery {
                holders: Err(format!("RmStartSession={started}")),
                session_close_failure: None,
            };
        }
        // Every path out of here ends the session it started.
        let holders = list_holders(handle, &wide);
        let ended = unsafe { RmEndSession(handle) };
        HolderQuery {
            holders,
            session_close_failure: (ended != 0).then(|| format!("RmEndSession={ended}")),
        }
    }

    fn list_holders(session: Dword, wide: &[u16]) -> Result<Vec<HolderProcess>, String> {
        let files = [wide.as_ptr()];
        let registered = unsafe {
            RmRegisterResources(
                session,
                1,
                files.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        if registered != 0 {
            return Err(format!("RmRegisterResources={registered}"));
        }
        let mut needed: u32 = 0;
        let mut have: u32 = 0;
        let mut reasons: Dword = 0;
        let sized = unsafe {
            RmGetList(
                session,
                &mut needed,
                &mut have,
                std::ptr::null_mut(),
                &mut reasons,
            )
        };
        if sized == 0 {
            return Ok(Vec::new());
        }
        if sized != ERROR_MORE_DATA {
            return Err(format!("RmGetList={sized}"));
        }
        if needed == 0 {
            return Ok(Vec::new());
        }
        if needed > MAX_REPORTED_PROCESSES {
            return Err(format!(
                "the Restart Manager reported {needed} holders, above the \
                 {MAX_REPORTED_PROCESSES} this refusal will allocate for"
            ));
        }
        let mut buffer: Vec<RmProcessInfo> = vec![unsafe { std::mem::zeroed() }; needed as usize];
        have = needed;
        let filled = unsafe {
            RmGetList(
                session,
                &mut needed,
                &mut have,
                buffer.as_mut_ptr(),
                &mut reasons,
            )
        };
        if filled != 0 {
            return Err(format!("RmGetList(filled)={filled}"));
        }
        let reported = (have as usize).min(buffer.len());
        Ok(buffer[..reported]
            .iter()
            .map(|info| HolderProcess {
                pid: info.process.pid,
                app: holder_name(&info.app_name, &info.service_short_name),
            })
            .collect())
    }
}

#[cfg(windows)]
fn enumerate_holders(path: &Path) -> HolderQuery {
    match absolute_for_query(path) {
        Ok(absolute) => restart_manager::holders(&absolute),
        Err(reason) => HolderQuery::failed(reason),
    }
}

#[cfg(not(windows))]
fn enumerate_holders(_path: &Path) -> HolderQuery {
    HolderQuery::failed("holder enumeration is implemented for Windows only")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn sharing_violation() -> std::io::Error {
        std::io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION)
    }

    fn answered(holders: Vec<HolderProcess>) -> HolderQuery {
        HolderQuery {
            holders: Ok(holders),
            session_close_failure: None,
        }
    }

    /// The clause builder is the only thing that queries anything, so counting
    /// its calls through the real entry point proves both halves: nothing is
    /// asked for an error that is not a sharing violation, and the builder does
    /// run for one that is.
    #[test]
    fn only_a_sharing_violation_reaches_the_holder_query() {
        let asked = Cell::new(0usize);
        let counting = |refused: &Path, _budget: Duration| {
            asked.set(asked.get() + 1);
            format!("counted {}", refused.display())
        };
        let denied = std::io::Error::from_raw_os_error(5);
        let message = with_holders_using(
            "open failed: x".to_string(),
            &denied,
            &[Path::new("C:\\a\\b")],
            HOLDER_QUERY_BUDGET,
            &counting,
        );
        assert_eq!(message, "open failed: x");
        assert_eq!(asked.get(), 0, "a non-sharing error asks nothing");

        let message = with_holders_using(
            "open failed: y".to_string(),
            &sharing_violation(),
            &[Path::new("C:\\a\\b")],
            HOLDER_QUERY_BUDGET,
            &counting,
        );
        if cfg!(windows) {
            assert_eq!(message, "open failed: y; counted C:\\a\\b");
            assert_eq!(asked.get(), 1, "a sharing violation asks exactly once");
        } else {
            assert_eq!(message, "open failed: y");
            assert_eq!(
                asked.get(),
                0,
                "off Windows os-32 is not a sharing violation"
            );
        }
    }

    /// A rename is refused on its source as readily as on its destination, so
    /// an operation that names both must have both queried.
    #[cfg(windows)]
    #[test]
    fn an_operation_on_two_paths_names_the_holder_of_each() {
        let asked = std::cell::RefCell::new(Vec::new());
        let counting = |refused: &Path, _budget: Duration| {
            asked.borrow_mut().push(refused.to_path_buf());
            format!("counted {}", refused.display())
        };
        let message = with_holders_using(
            "rename failed".to_string(),
            &sharing_violation(),
            &[Path::new("C:\\a\\temp"), Path::new("C:\\a\\dest")],
            HOLDER_QUERY_BUDGET,
            &counting,
        );
        assert_eq!(
            message,
            "rename failed; counted C:\\a\\temp; counted C:\\a\\dest"
        );
        assert_eq!(
            *asked.borrow(),
            vec![
                PathBuf::from("C:\\a\\temp"),
                PathBuf::from("C:\\a\\dest")
            ]
        );
    }

    /// Naming two paths may not cost twice the budget: the whole refusal shares
    /// one deadline, and a candidate that finds none of it left says so instead
    /// of starting another query.
    #[cfg(windows)]
    #[test]
    fn every_candidate_of_one_refusal_shares_one_budget() {
        static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
        let shared = Duration::from_millis(300);
        let started = std::time::Instant::now();
        let message = with_holders_using(
            "rename failed".to_string(),
            &sharing_violation(),
            &[Path::new("C:\\a\\temp"), Path::new("C:\\a\\dest")],
            shared,
            |path, budget| {
                holder_clause_within(
                    path,
                    budget,
                    &OUTSTANDING,
                    |_| {
                        std::thread::sleep(Duration::from_millis(900));
                        answered(Vec::new())
                    },
                    spawn_query,
                )
            },
        );
        let elapsed = started.elapsed();
        assert!(
            message.contains("C:\\a\\temp") && message.contains("C:\\a\\dest"),
            "both candidates are accounted for: {message}"
        );
        assert!(
            message.contains("no time was left in this refusal's holder-query budget"),
            "the second candidate must say the shared budget was spent: {message}"
        );
        assert!(
            elapsed < shared + Duration::from_millis(300),
            "the refusal took {elapsed:?} for a {shared:?} budget"
        );
        assert_eq!(
            OUTSTANDING.load(Ordering::SeqCst),
            1,
            "only the candidate that was queried holds a slot"
        );
    }

    /// A retained-open refusal already names the exact component that refused,
    /// so it must not fan out over the caller's other candidates.
    #[cfg(windows)]
    #[test]
    fn an_ancestor_refusal_answers_alone() {
        let wrapped = std::io::Error::new(
            std::io::ErrorKind::Other,
            snapshot::container::RetainedAncestorRefusal::new(
                PathBuf::from("C:\\a"),
                sharing_violation(),
            ),
        );
        assert_eq!(
            refused_paths(&wrapped, &[Path::new("C:\\a\\b"), Path::new("C:\\a\\c")]),
            vec![PathBuf::from("C:\\a")]
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_sharing_violation_names_the_attempted_path() {
        assert_eq!(
            refused_paths(&sharing_violation(), &[Path::new("C:\\a\\b.battleplan")]),
            vec![PathBuf::from("C:\\a\\b.battleplan")]
        );
    }

    #[cfg(windows)]
    #[test]
    fn an_ancestor_refusal_names_the_ancestor_not_the_container() {
        let wrapped = std::io::Error::new(
            std::io::ErrorKind::Other,
            snapshot::container::RetainedAncestorRefusal::new(
                PathBuf::from("C:\\a"),
                sharing_violation(),
            ),
        );
        assert_eq!(
            refused_paths(&wrapped, &[Path::new("C:\\a\\b\\c.battleplan")]),
            vec![PathBuf::from("C:\\a")]
        );
    }

    /// 32 is ERROR_SHARING_VIOLATION only on Windows: elsewhere it is an
    /// unrelated errno and must not start a holder query.
    #[cfg(not(windows))]
    #[test]
    fn os_error_32_is_not_a_sharing_violation_off_windows() {
        assert!(refused_paths(&sharing_violation(), &[Path::new("/a/b")]).is_empty());
        let wrapped = std::io::Error::new(
            std::io::ErrorKind::Other,
            snapshot::container::RetainedAncestorRefusal::new(
                PathBuf::from("/a"),
                sharing_violation(),
            ),
        );
        assert!(refused_paths(&wrapped, &[Path::new("/a/b")]).is_empty());
    }

    #[test]
    fn an_ancestor_refusal_for_another_reason_is_not_a_sharing_violation() {
        let wrapped = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            snapshot::container::RetainedAncestorRefusal::new(
                PathBuf::from("C:\\a"),
                std::io::Error::from_raw_os_error(5),
            ),
        );
        assert!(refused_paths(&wrapped, &[Path::new("C:\\a\\b")]).is_empty());
    }

    #[test]
    fn the_clause_names_every_holder_it_is_given() {
        let clause = describe_holders(
            Path::new("C:\\a\\b"),
            &answered(vec![
                HolderProcess {
                    pid: 68128,
                    app: "probe32.exe".to_string(),
                },
                HolderProcess {
                    pid: 7,
                    app: String::new(),
                },
            ]),
        );
        assert_eq!(
            clause,
            "C:\\a\\b is held open by pid=68128 app=probe32.exe, pid=7 \
             app=<no application or service name reported>"
        );
    }

    #[test]
    fn the_clause_caps_the_list_and_counts_the_rest() {
        let holders: Vec<HolderProcess> = (0..MAX_NAMED_HOLDERS as u32 + 3)
            .map(|pid| HolderProcess {
                pid,
                app: "h.exe".to_string(),
            })
            .collect();
        let clause = describe_holders(Path::new("C:\\a\\b"), &answered(holders));
        assert_eq!(
            clause.matches("app=h.exe").count(),
            MAX_NAMED_HOLDERS,
            "{clause}"
        );
        assert!(
            clause.ends_with("and 3 further process(es) not listed"),
            "{clause}"
        );
    }

    #[test]
    fn an_empty_enumeration_says_no_user_mode_process_holds_it() {
        let clause = describe_holders(Path::new("C:\\a\\b"), &answered(Vec::new()));
        assert!(
            clause.starts_with("no user-mode process holds C:\\a\\b:"),
            "{clause}"
        );
        for cause in ["kernel filter", "cannot enumerate", "closed between"] {
            assert!(clause.contains(cause), "{cause} missing from {clause}");
        }
    }

    #[test]
    fn a_failed_enumeration_names_the_failure() {
        let clause = describe_holders(
            Path::new("C:\\a\\b"),
            &HolderQuery {
                holders: Err("RmGetList=5".to_string()),
                session_close_failure: None,
            },
        );
        assert_eq!(
            clause,
            "the holder of C:\\a\\b could not be enumerated: RmGetList=5"
        );
    }

    #[test]
    fn a_session_that_will_not_close_is_reported_beside_the_answer() {
        let clause = describe_holders(
            Path::new("C:\\a\\b"),
            &HolderQuery {
                holders: Ok(vec![HolderProcess {
                    pid: 4,
                    app: "System".to_string(),
                }]),
                session_close_failure: Some("RmEndSession=5".to_string()),
            },
        );
        assert_eq!(
            clause,
            "C:\\a\\b is held open by pid=4 app=System (the Restart Manager session did not \
             close: RmEndSession=5)"
        );
    }

    /// A query that has not returned must cost the refusal the budget and no
    /// more.
    #[test]
    fn a_stalled_query_still_lets_the_refusal_fire() {
        static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
        let started = std::time::Instant::now();
        let clause = holder_clause_within(
            Path::new("C:\\a\\b"),
            Duration::from_millis(120),
            &OUTSTANDING,
            |_| {
                std::thread::sleep(Duration::from_millis(700));
                answered(Vec::new())
            },
            spawn_query,
        );
        let elapsed = started.elapsed();
        assert!(clause.contains("did not return within 120 ms"), "{clause}");
        assert!(
            elapsed < Duration::from_millis(600),
            "the refusal waited {elapsed:?} on a query that had not returned"
        );
    }

    /// Abandoned queries are counted, and a refusal past the bound names the
    /// backlog instead of starting another thread and another session.
    #[test]
    fn outstanding_queries_are_bounded() {
        static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
        let stall = |_: &Path| {
            std::thread::sleep(Duration::from_millis(800));
            answered(Vec::new())
        };
        for attempt in 0..MAX_OUTSTANDING_QUERIES {
            let clause = holder_clause_within(
                Path::new("C:\\a\\b"),
                Duration::from_millis(20),
                &OUTSTANDING,
                stall,
                spawn_query,
            );
            assert!(
                clause.contains("did not return within 20 ms"),
                "attempt {attempt}: {clause}"
            );
        }
        assert_eq!(OUTSTANDING.load(Ordering::SeqCst), MAX_OUTSTANDING_QUERIES);
        let spawned = Cell::new(false);
        let clause = holder_clause_within(
            Path::new("C:\\a\\b"),
            Duration::from_millis(20),
            &OUTSTANDING,
            stall,
            |_| {
                spawned.set(true);
                Ok(())
            },
        );
        assert!(
            clause.contains("earlier Restart Manager quer(ies) have not returned"),
            "{clause}"
        );
        assert!(!spawned.get(), "the bounded refusal started no query");
        assert_eq!(
            OUTSTANDING.load(Ordering::SeqCst),
            MAX_OUTSTANDING_QUERIES,
            "a refused query must not raise the count it refused on"
        );
    }

    /// A process that cannot start the query thread must still refuse, not
    /// panic.
    #[test]
    fn a_query_thread_that_cannot_start_still_lets_the_refusal_fire() {
        static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
        let clause = holder_clause_within(
            Path::new("C:\\a\\b"),
            HOLDER_QUERY_BUDGET,
            &OUTSTANDING,
            |_| answered(Vec::new()),
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::OutOfMemory,
                    "no thread for you",
                ))
            },
        );
        assert_eq!(
            clause,
            "the holder of C:\\a\\b could not be enumerated: the query thread could not be \
             started: no thread for you"
        );
        assert_eq!(
            OUTSTANDING.load(Ordering::SeqCst),
            0,
            "a query that never started is not outstanding"
        );
    }

    #[test]
    fn a_query_that_dies_without_answering_still_names_the_failure() {
        static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
        let clause = holder_clause_within(
            Path::new("C:\\a\\b"),
            HOLDER_QUERY_BUDGET,
            &OUTSTANDING,
            |_| panic!("the query thread died"),
            spawn_query,
        );
        assert_eq!(
            clause,
            "the holder of C:\\a\\b could not be enumerated: the query ended without an answer"
        );
        assert_eq!(
            settled(&OUTSTANDING),
            0,
            "a query that panicked gave its slot back"
        );
    }

    /// A query that left a Restart Manager session behind keeps its slot, so
    /// leaked sessions are bounded by the same number as live query threads.
    #[test]
    fn a_leaked_session_keeps_its_slot_until_the_bound_refuses() {
        static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
        let leaking = |_: &Path| HolderQuery {
            holders: Ok(Vec::new()),
            session_close_failure: Some("RmEndSession=5".to_string()),
        };
        for attempt in 0..MAX_OUTSTANDING_QUERIES {
            let clause = holder_clause_within(
                Path::new("C:\\a\\b"),
                HOLDER_QUERY_BUDGET,
                &OUTSTANDING,
                leaking,
                spawn_query,
            );
            assert!(
                clause.contains("the Restart Manager session did not close"),
                "attempt {attempt}: {clause}"
            );
            assert_eq!(
                settled(&OUTSTANDING),
                attempt + 1,
                "attempt {attempt}: a leaked session must not give its slot back"
            );
        }
        let clause = holder_clause_within(
            Path::new("C:\\a\\b"),
            HOLDER_QUERY_BUDGET,
            &OUTSTANDING,
            leaking,
            spawn_query,
        );
        assert!(
            clause.contains("did not release their session"),
            "the bound must refuse once every slot is held: {clause}"
        );
    }

    /// The counter is decremented by the query thread, so a test that asserts
    /// on it waits for that thread rather than racing it.
    fn settled(counter: &AtomicUsize) -> usize {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen = counter.load(Ordering::SeqCst);
        loop {
            std::thread::sleep(Duration::from_millis(5));
            let now = counter.load(Ordering::SeqCst);
            if now == seen || std::time::Instant::now() > deadline {
                return now;
            }
            seen = now;
        }
    }

    #[test]
    fn the_refusal_message_keeps_the_original_text_and_appends_the_holder() {
        let refused = Path::new("C:\\a\\b");
        let message = format!(
            "retain stitched output directory {} failed: {}; {}",
            refused.display(),
            sharing_violation(),
            describe_holders(
                refused,
                &answered(vec![HolderProcess {
                    pid: 4,
                    app: "System".to_string()
                }])
            )
        );
        assert!(message.starts_with("retain stitched output directory C:\\a\\b failed:"));
        assert!(
            message.ends_with("C:\\a\\b is held open by pid=4 app=System"),
            "{message}"
        );
    }

    #[test]
    fn a_holder_falls_back_to_its_service_name_and_says_so_when_it_has_neither() {
        let wide = |text: &str, width: usize| {
            let mut field: Vec<u16> = text.encode_utf16().collect();
            field.resize(width, 0);
            field
        };
        assert_eq!(
            holder_name(&wide("probe32.exe", 256), &wide("", 64)),
            "probe32.exe"
        );
        assert_eq!(
            holder_name(&wide("", 256), &wide("WSearch", 64)),
            "WSearch",
            "an application-less service holder is still named"
        );
        assert_eq!(holder_name(&wide("", 256), &wide("", 64)), "");
        assert_eq!(
            holder_name(&wide("a.exe", 256), &wide("svc", 64)),
            "a.exe",
            "the application name wins when both are reported"
        );
    }

    #[test]
    fn a_relative_refused_path_is_resolved_before_it_is_queried() {
        let cwd = std::env::current_dir().expect("a working directory");
        assert_eq!(
            absolute_for_query(Path::new("out/c.battleplan")).expect("resolved"),
            cwd.join("out").join("c.battleplan")
        );
        assert_eq!(
            absolute_for_query(Path::new("./out/./c.battleplan")).expect("resolved"),
            cwd.join("out").join("c.battleplan")
        );
        let parent = cwd.parent().expect("a parent directory");
        assert_eq!(
            absolute_for_query(Path::new("../c.battleplan")).expect("resolved"),
            parent.join("c.battleplan")
        );
        let absolute = if cfg!(windows) {
            PathBuf::from("C:\\a\\b\\c.battleplan")
        } else {
            PathBuf::from("/a/b/c.battleplan")
        };
        assert_eq!(
            absolute_for_query(&absolute).expect("resolved"),
            absolute,
            "an absolute path is passed through"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_restart_manager_names_this_process_holding_its_own_file() {
        use std::io::Write;

        let dir = std::env::temp_dir().join(format!(
            "wg_holder_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create probe dir");
        let file_path = dir.join("held.battleplan");
        let mut held = std::fs::File::create(&file_path).expect("create held file");
        held.write_all(b"held").expect("write held file");
        held.flush().expect("flush held file");

        // The same file by an absolute path and by a path relative to this
        // process's working directory: the Restart Manager takes full paths
        // only, so both must reach it as the same full path.
        let relative = relative_from(
            &std::env::current_dir().expect("a working directory"),
            &file_path,
        );
        let by_absolute = enumerate_holders(&file_path);
        let by_relative = relative
            .as_ref()
            .map(|path| enumerate_holders(path.as_path()));

        drop(held);
        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir(&dir);

        let ours = |query: &HolderQuery| {
            query
                .holders
                .as_ref()
                .map(|holders| {
                    holders
                        .iter()
                        .any(|holder| holder.pid == std::process::id() && !holder.app.is_empty())
                })
                .unwrap_or(false)
        };
        assert!(
            ours(&by_absolute),
            "the Restart Manager did not name this process ({}) holding {}: {by_absolute:?}",
            std::process::id(),
            file_path.display()
        );
        let by_relative = by_relative.expect("a relative path to the held file exists");
        assert!(
            ours(&by_relative),
            "the same file named relatively ({relative:?}) was not resolved: {by_relative:?}"
        );
    }

    /// `target` expressed relative to `from`, or `None` when no relative form
    /// exists (a different volume).
    #[cfg(windows)]
    fn relative_from(from: &Path, target: &Path) -> Option<PathBuf> {
        let from: Vec<_> = from.components().collect();
        let target: Vec<_> = target.components().collect();
        if from.first() != target.first() {
            return None;
        }
        let shared = from
            .iter()
            .zip(target.iter())
            .take_while(|(a, b)| a == b)
            .count();
        let mut relative = PathBuf::new();
        for _ in shared..from.len() {
            relative.push("..");
        }
        for component in &target[shared..] {
            relative.push(component.as_os_str());
        }
        Some(relative)
    }

    // ---- the completeness gate -------------------------------------------

    /// Calls that ask the filesystem for an access another handle's share mode
    /// can deny. Attribute-only queries (`metadata`, `symlink_metadata`,
    /// `exists`) are absent on purpose: Windows does not refuse a
    /// FILE_READ_ATTRIBUTES-only open for sharing.
    const OPEN_TOKENS: [&str; 11] = [
        "File::open(",
        ".open(",
        "fs::read(",
        "fs::read_dir(",
        "fs::write(",
        "fs::rename(",
        "fs::create_dir_all(",
        "fs::canonicalize(",
        "Container::open(",
        "open_strict_with_file_receipt(",
        "rename_new_durable(",
    ];

    const HOLDER_CALL: &str = "sharing_violation_holders::with_";

    /// Calls a governed packaging region may not make. The general
    /// `hash_file_sha256_bytes` opens a file and deliberately does NOT name a
    /// holder, because it is also called before and during capture; the
    /// packaging path uses `hash_packaged_file_sha256_bytes` instead.
    const FORBIDDEN_CALLS: [&str; 1] = ["hash_file_sha256_bytes("];

    /// How far after an open its holder call may sit. The next open is a
    /// harder bound than this one: a holder call may not be shared backwards
    /// across another open.
    const HOLDER_CALL_WINDOW: usize = 1200;

    /// Every packaging region whose filesystem opens must name the holder, and
    /// how many holder calls in it belong to a refusal that is NOT one of the
    /// scanned opens. `None` governs the whole file.
    ///
    /// A region is scanned with its `#[cfg(test)]` and `#[cfg(not(windows))]`
    /// items and blocks removed: test code is not a product refusal, and off
    /// Windows there is no sharing violation to name.
    const GOVERNED_REGIONS: [(&str, Option<&str>, usize); 7] = [
        // The publish rename is `atomic_replace_retained`, which renames by
        // handle rather than through a scanned path open.
        ("stitching/publish.rs", None, 1),
        ("stitching/callsite.rs", None, 0),
        (
            "main.rs",
            Some("fn write_native_replay_container_streaming_with_pre_verify_hook"),
            0,
        ),
        ("main.rs", Some("fn create_unique_replay_temp"), 0),
        // Same: the publish rename goes through SetFileInformationByHandle.
        ("main.rs", Some("fn finalize_verified_replay"), 1),
        ("main.rs", Some("fn publish_rollover_embedded_manifest"), 0),
        // The packaging-only hash helper. The general `hash_file_sha256_bytes`
        // is deliberately NOT governed and NOT wired: it is also called before
        // and during capture, where a holder query has no business running.
        ("main.rs", Some("fn hash_packaged_file_sha256_bytes"), 0),
    ];

    /// The lines of `source` with every `#[cfg(test)]` and
    /// `#[cfg(not(windows))]` item or block removed. rustfmt closes an item at
    /// the attribute's own indentation, which is what this matches.
    fn without_test_and_non_windows(source: &str) -> String {
        const REMOVED: [&str; 2] = ["#[cfg(test)]", "#[cfg(not(windows))]"];
        let lines: Vec<&str> = source.lines().collect();
        let mut kept = Vec::new();
        let mut index = 0usize;
        while index < lines.len() {
            let line = lines[index];
            if !REMOVED.contains(&line.trim()) {
                kept.push(line);
                index += 1;
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            let closing = format!("{}}}", " ".repeat(indent));
            let mut cursor = index + 1;
            while cursor < lines.len() && !lines[cursor].trim_end().ends_with('{') {
                cursor += 1;
            }
            while cursor < lines.len() && lines[cursor].trim_end() != closing {
                cursor += 1;
            }
            assert!(cursor < lines.len(), "unterminated item after {line}");
            index = cursor + 1;
        }
        kept.join("\n")
    }

    /// A top-level `fn` body: from its signature to the next line that is a
    /// bare `}` in column zero.
    fn top_level_function<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(&format!("\n{signature}"))
            .unwrap_or_else(|| panic!("{signature} is not a top-level item"))
            + 1;
        let end = source[start..]
            .find("\n}\n")
            .unwrap_or_else(|| panic!("{signature} has no top-level end"));
        &source[start..start + end]
    }

    fn line_at(region: &str, offset: usize) -> &str {
        let start = region[..offset].rfind('\n').map(|at| at + 1).unwrap_or(0);
        let end = region[offset..]
            .find('\n')
            .map(|at| offset + at)
            .unwrap_or(region.len());
        &region[start..end]
    }

    /// Every scanned open in `region`, by byte offset. Counted per occurrence,
    /// not per line: two opens on one line are two opens. A `fn` signature is
    /// a definition, not a call, so it is not one.
    fn open_positions(region: &str) -> Vec<usize> {
        let mut found = Vec::new();
        for token in OPEN_TOKENS {
            let mut from = 0usize;
            while let Some(offset) = region[from..].find(token) {
                let at = from + offset;
                let line = line_at(region, at).trim_start();
                if !line.starts_with("fn ")
                    && !line.starts_with("pub fn ")
                    && !line.starts_with("pub(crate) fn ")
                    && !line.starts_with("pub(super) fn ")
                {
                    found.push(at);
                }
                from = at + token.len();
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }

    fn holder_positions(region: &str) -> Vec<usize> {
        region
            .match_indices(HOLDER_CALL)
            .map(|(offset, _)| offset)
            .collect()
    }

    /// The opens whose own holder call is missing: none between the open and
    /// whichever comes first, the next open or the end of the window.
    fn unwired_opens(region: &str) -> Vec<String> {
        let opens = open_positions(region);
        let holders = holder_positions(region);
        let mut unwired = Vec::new();
        for (index, open) in opens.iter().enumerate() {
            let next_open = opens.get(index + 1).copied().unwrap_or(region.len());
            let boundary = next_open.min(open + HOLDER_CALL_WINDOW).min(region.len());
            if !holders
                .iter()
                .any(|holder| *holder > *open && *holder < boundary)
            {
                unwired.push(line_at(region, *open).trim().to_string());
            }
        }
        unwired
    }

    #[test]
    fn every_packaging_open_names_the_holder() {
        for (file, function, extra_holder_calls) in GOVERNED_REGIONS {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(file);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            let source = source.replace("\r\n", "\n");
            let scanned = match function {
                Some(signature) => {
                    without_test_and_non_windows(top_level_function(&source, signature))
                }
                None => without_test_and_non_windows(&source),
            };
            let label = function.unwrap_or(file);
            for forbidden in FORBIDDEN_CALLS {
                assert!(
                    !scanned.contains(forbidden),
                    "{label} calls {forbidden}, which opens a file without naming the holder                      of a sharing violation"
                );
            }
            let unwired = unwired_opens(&scanned);
            assert!(
                unwired.is_empty(),
                "{label}: {} filesystem open(s) do not name the holder of a sharing \
                 violation: {unwired:#?}",
                unwired.len()
            );
            assert_eq!(
                holder_positions(&scanned).len(),
                open_positions(&scanned).len() + extra_holder_calls,
                "{label}: the scanned opens plus {extra_holder_calls} handle-based refusal(s) \
                 must be exactly the holder calls present",
            );
        }
    }

    /// The gate must fail for an open that does not name the holder, however
    /// the unwired open is arranged: alone, sharing a line with a wired one, or
    /// sitting just before one whose holder call it might borrow.
    #[test]
    fn the_completeness_gate_catches_an_unwired_open() {
        let wired = "fn a(path: &Path) -> Result<(), String> {\n    \
             let file = File::open(path).map_err(|error| {\n        \
             crate::sharing_violation_holders::with_holder(\n            \
             format!(\"open failed: {error}\"),\n            &error,\n            path,\n        \
             )\n    })?;\n    Ok(())\n}\n";
        assert_eq!(open_positions(wired).len(), 1);
        assert!(unwired_opens(wired).is_empty());
        assert_eq!(holder_positions(wired).len(), 1);

        let alone = format!(
            "{wired}fn b(path: &Path) -> Result<(), String> {{\n    \
             let bytes = fs::read(path).map_err(|error| format!(\"read failed: {{error}}\"))?;\n    \
             Ok(())\n}}\n"
        );
        assert_eq!(open_positions(&alone).len(), 2);
        assert_eq!(unwired_opens(&alone).len(), 1, "{:#?}", unwired_opens(&alone));
        assert_ne!(
            holder_positions(&alone).len(),
            open_positions(&alone).len(),
            "an added open must break the count as well as the window"
        );

        // Two opens on one line, one holder call: the first cannot borrow it.
        let shared_line = "fn c(a: &Path, b: &Path) -> Result<(), String> {\n    \
             let (x, y) = (fs::read(a)?, fs::read(b).map_err(|error| \
             crate::sharing_violation_holders::with_holder(String::new(), &error, b))?);\n    \
             Ok(())\n}\n";
        assert_eq!(open_positions(shared_line).len(), 2, "both opens are counted");
        assert_eq!(unwired_opens(shared_line).len(), 1);

        // An unwired open immediately before a wired one may not borrow the
        // later holder call.
        let borrowed = "fn d(a: &Path, b: &Path) -> Result<(), String> {\n    \
             let x = fs::read(a)?;\n    \
             let y = fs::read(b).map_err(|error| \
             crate::sharing_violation_holders::with_holder(String::new(), &error, b))?;\n    \
             Ok(())\n}\n";
        assert_eq!(unwired_opens(borrowed).len(), 1, "{:#?}", unwired_opens(borrowed));

        let test_only = format!(
            "{wired}#[cfg(test)]\nmod tests {{\n    fn c() {{\n        \
             fs::write(&path, b\"x\").unwrap();\n    }}\n}}\n"
        );
        let scanned = without_test_and_non_windows(&test_only);
        assert_eq!(open_positions(&scanned).len(), 1, "test code is not scanned");
        assert!(unwired_opens(&scanned).is_empty());
    }
}
