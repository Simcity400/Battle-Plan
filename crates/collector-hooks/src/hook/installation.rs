use std::{borrow::Cow, cell::RefCell};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sentinel,
    Construction,
    Publication,
    Rollback,
    Registration,
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sentinel => "sentinel",
            Self::Construction => "construction",
            Self::Publication => "publication",
            Self::Rollback => "rollback",
            Self::Registration => "registration",
        }
    }
    pub fn fatal(self) -> bool {
        matches!(self, Self::Rollback | Self::Registration)
    }
}
#[derive(Clone, Debug)]
pub struct Failure {
    pub kind: Kind,
    pub site: Option<usize>,
    pub cause: Cow<'static, str>,
}
#[derive(Default, Debug)]
pub struct Report {
    pub sites: Vec<usize>,
    pub failure: Option<Failure>,
}
thread_local! { static CURRENT: RefCell<Option<Report>> = const { RefCell::new(None) }; }
pub fn capture<R>(work: impl FnOnce() -> R) -> (R, Report) {
    struct Restore(Option<Report>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|state| *state.borrow_mut() = self.0.take());
        }
    }
    let previous = CURRENT.with(|state| state.replace(Some(Report::default())));
    let restore = Restore(previous);
    let result = work();
    let report = CURRENT.with(|state| state.borrow_mut().take()).unwrap();
    drop(restore);
    (result, report)
}
pub fn site(site: usize) {
    CURRENT.with(|state| {
        let mut state = state.borrow_mut();
        if let Some(report) = state.as_mut()
            && !report.sites.contains(&site)
        {
            if engine_allocation::reserve(&mut report.sites, 1).is_ok() {
                report.sites.push(site);
            } else {
                report.failure = Some(Failure {
                    kind: Kind::Registration,
                    site: Some(site),
                    cause: "hook installation audit allocation failed".into(),
                });
            }
        }
    });
}
pub fn refused(kind: Kind, site: Option<usize>, cause: &str) {
    CURRENT.with(|state| {
        let mut state = state.borrow_mut();
        let Some(report) = state.as_mut() else {
            return;
        };
        if report
            .failure
            .as_ref()
            .is_some_and(|old| old.kind.fatal() || !kind.fatal())
        {
            return;
        }
        let cause = match (&report.failure, kind) {
            (Some(old), Kind::Rollback) => {
                engine_allocation::string(&format!("{}; rollback: {cause}", old.cause))
            }
            _ => engine_allocation::string(cause),
        }
        .map(Cow::Owned)
        .unwrap_or(Cow::Borrowed(
            "hook installation diagnostic allocation failed",
        ));
        report.failure = Some(Failure { kind, site, cause });
    });
}
pub fn sentinel<const N: usize>(
    memory: &impl engine_native_memory::Memory,
    at: usize,
    expected: [u8; N],
) -> bool {
    let matches = engine_native_memory::read::<[u8; N]>(memory, at) == Some(expected);
    if !matches {
        refused(
            Kind::Sentinel,
            Some(at),
            "hook sentinel differs or unreadable",
        );
    }
    matches
}
pub fn construct<T>(at: usize, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    site(at);
    let result = work();
    if let Err(cause) = &result {
        refused(Kind::Construction, Some(at), cause);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_native_constructor_rejection_is_observed_even_when_caller_ignores_it() {
        static FLIGHTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let (_, report) = capture(|| {
            let _ = unsafe {
                super::super::Hook::new_with_safety(
                    17,
                    25,
                    [0; 16],
                    super::super::TapSafety {
                        in_flight: &FLIGHTS,
                        stub: 30..31,
                    },
                )
            };
        });
        assert_eq!(report.sites, [17]);
        let failure = report.failure.unwrap();
        assert_eq!(failure.kind, Kind::Construction);
        assert_eq!(failure.site, Some(17));
        assert!(failure.cause.contains("stub"));
    }

    #[test]
    fn swallowed_errors_and_initial_rollback_failure_keep_their_classification() {
        let (_, report) = capture(|| {
            let _ = construct(17, || Err::<(), _>("relocation refused".into()));
            refused(Kind::Rollback, Some(17), "restore failed");
            let _ = construct(17, || Ok(()));
        });
        let failure = report.failure.unwrap();
        assert_eq!(failure.kind, Kind::Rollback);
        assert!(failure.cause.contains("relocation refused"));
        assert!(failure.cause.contains("restore failed"));
        assert_eq!(report.sites, [17]);
    }
    #[test]
    fn scopes_restore_and_inactive_callbacks_have_no_report_storage() {
        let (_, outer) = capture(|| {
            site(1);
            let (_, inner) = capture(|| refused(Kind::Sentinel, Some(2), "different"));
            assert_eq!(inner.failure.unwrap().kind, Kind::Sentinel);
            site(3);
        });
        assert_eq!(outer.sites, [1, 3]);
        assert!(outer.failure.is_none());
    }
}
