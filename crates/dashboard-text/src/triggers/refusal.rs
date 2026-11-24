use super::Store;
use replay_data::rows::TableId;
use std::{cell::RefCell, collections::BTreeSet, fmt};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Identity {
    Input {
        table: TableId,
        field: String,
        scope: Option<Store>,
    },
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct Failure {
    message: String,
    identities: BTreeSet<Identity>,
}

thread_local! {
    static CAPTURES: RefCell<Vec<BTreeSet<Identity>>> = const { RefCell::new(Vec::new()) };
    static DEFERRED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static RESOLUTIONS: RefCell<Vec<BTreeSet<Identity>>> = const { RefCell::new(Vec::new()) };
}

fn record_resolutions(identities: &BTreeSet<Identity>) {
    record(identities);
    RESOLUTIONS.with(|captures| {
        for capture in captures.borrow_mut().iter_mut() {
            capture.extend(identities.iter().cloned());
        }
    });
}

pub(crate) fn resolutions<T>(run: impl FnOnce() -> T) -> (T, BTreeSet<Identity>) {
    RESOLUTIONS.with(|captures| captures.borrow_mut().push(BTreeSet::new()));
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            RESOLUTIONS.with(|captures| {
                captures.borrow_mut().pop();
            });
        }
    }
    let pop = Pop;
    let value = run();
    let failures =
        RESOLUTIONS.with(|captures| std::mem::take(captures.borrow_mut().last_mut().unwrap()));
    drop(pop);
    (value, failures)
}

#[derive(Clone, Debug)]
pub(crate) struct Resolved<T> {
    value: T,
    failures: BTreeSet<Identity>,
}
impl<T> Resolved<T> {
    pub(crate) fn new(run: impl FnOnce() -> T) -> Self {
        let (value, failures) = resolutions(run);
        Self { value, failures }
    }

    pub(crate) fn value(&self) -> &T {
        record_resolutions(&self.failures);
        &self.value
    }
}

pub(crate) fn reference<T>(
    value: Option<T>,
    positive: bool,
    missing: impl FnOnce() -> Failure,
) -> Option<T> {
    if value.is_none() && positive {
        missing().resolution_failed();
    }
    value
}

pub(crate) fn missing_reference(table: TableId, scope: Option<Store>) -> Failure {
    Failure::input(
        table,
        "",
        scope,
        format!("{}: the object it refers to is not recorded", table.words()),
    )
}

pub(crate) fn membership(found: bool, complete: bool, missing: impl FnOnce() -> Failure) -> bool {
    if !found && !complete {
        missing().resolution_failed();
    }
    found
}

fn record(identities: &BTreeSet<Identity>) {
    if DEFERRED.with(|depth| depth.get()) != 0 {
        return;
    }
    CAPTURES.with(|captures| {
        for capture in captures.borrow_mut().iter_mut() {
            capture.extend(identities.iter().cloned());
        }
    });
}

impl Failure {
    pub(crate) fn input(
        table: TableId,
        field: impl Into<String>,
        scope: Option<Store>,
        message: impl Into<String>,
    ) -> Self {
        let identities = BTreeSet::from([Identity::Input {
            table,
            field: field.into(),
            scope,
        }]);
        record_resolutions(&identities);
        Self {
            message: message.into(),
            identities,
        }
    }

    pub fn context(self, context: impl FnOnce(&str) -> String) -> Self {
        Self {
            message: context(&self.message),
            ..self
        }
    }

    pub fn merge(failures: impl IntoIterator<Item = Self>) -> Self {
        let mut identities = BTreeSet::new();
        let mut messages = Vec::new();
        for failure in failures {
            identities.extend(failure.identities);
            messages.push(failure.message);
        }
        Self {
            message: messages.join("; "),
            identities,
        }
    }

    pub(crate) fn reached(&self) {
        record(&self.identities);
    }

    pub(crate) fn resolution_failed(&self) {
        record_resolutions(&self.identities);
    }

    pub(crate) fn consumed(&self) {
        #[cfg(test)]
        assert_reached(self);
        self.reached();
    }

    fn canonical(identity: Identity) -> Self {
        let message = match &identity {
            Identity::Input {
                table,
                field,
                scope,
            } => format!("{}: not available or refused", subject(*table, field, *scope)),
            Identity::Unsupported(why) => why.clone(),
        };
        Self {
            message,
            identities: BTreeSet::from([identity]),
        }
    }
}

#[cfg(test)]
impl Failure {
    pub(crate) fn names(&self, name: &str) -> bool {
        self.identities.iter().any(|identity| {
            let Identity::Input {
                table,
                field,
                scope,
            } = identity
            else {
                return false;
            };
            let field = if field.is_empty() {
                String::new()
            } else {
                format!(".{field}")
            };
            let scope = scope
                .map(|scope| format!(" ({scope:?})"))
                .unwrap_or_default();
            format!("{table:?}{field}{scope}")
                .strip_prefix(name)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', ' ']))
        })
    }

    pub(crate) fn identities(&self) -> &BTreeSet<Identity> {
        &self.identities
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        let identities = BTreeSet::from([Identity::Unsupported(message.clone())]);
        record(&identities);
        Self {
            message,
            identities,
        }
    }
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl std::ops::Deref for Failure {
    type Target = str;
    fn deref(&self) -> &str {
        &self.message
    }
}
impl std::borrow::Borrow<str> for Failure {
    fn borrow(&self) -> &str {
        &self.message
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Failure {}
impl PartialEq<str> for Failure {
    fn eq(&self, other: &str) -> bool {
        self.message == other
    }
}
impl PartialEq<&str> for Failure {
    fn eq(&self, other: &&str) -> bool {
        self.message == *other
    }
}
impl PartialEq<String> for Failure {
    fn eq(&self, other: &String) -> bool {
        self.message == *other
    }
}

pub fn capture<T>(run: impl FnOnce() -> T) -> (T, BTreeSet<Failure>) {
    CAPTURES.with(|captures| captures.borrow_mut().push(BTreeSet::new()));
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            CAPTURES.with(|captures| {
                captures.borrow_mut().pop();
            });
        }
    }
    let pop = Pop;
    let value = run();
    let failures =
        CAPTURES.with(|captures| std::mem::take(captures.borrow_mut().last_mut().unwrap()));
    drop(pop);
    (
        value,
        failures.into_iter().map(Failure::canonical).collect(),
    )
}

#[cfg(test)]
pub(crate) fn assert_reached(failure: &Failure) {
    if DEFERRED.with(|depth| depth.get()) != 0 {
        return;
    }
    CAPTURES.with(|captures| {
        if let Some(capture) = captures.borrow().last() {
            assert!(
                failure.identities.is_subset(capture),
                "deferred failure consumed without reached(): {:?}",
                failure.identities
            );
        }
    });
}

pub(crate) trait FrameInputs {
    fn trigger_table<T: replay_data::rows::TableSchema>(&self) -> Result<&[T::Row], Failure>;
}

pub(crate) fn input_error(table: TableId, why: std::io::Error) -> Failure {
    let failure = Failure::input(table, "", None, why.to_string());
    failure.resolution_failed();
    failure
}
pub(crate) fn complete_table<F, T>(
    frame: &F,
    mask: u64,
) -> Result<replay_frame::frame::Rowset<'_, T::Row>, Failure>
where
    F: replay_frame::frame::Tables + ?Sized,
    T: replay_data::rows::TableSchema,
{
    frame.complete_rows::<T>(mask).map_err(|why| {
        let failure = Failure::input(T::ID, "", None, why.to_string());
        failure.resolution_failed();
        failure
    })
}

impl FrameInputs for replay_frame::frame::DecodedFrame {
    fn trigger_table<T: replay_data::rows::TableSchema>(&self) -> Result<&[T::Row], Failure> {
        self.complete::<T>().map_err(|why| {
            let failure = Failure::input(T::ID, "", None, why.to_string());
            failure.resolution_failed();
            failure
        })
    }
}

impl FrameInputs for dyn replay_frame::frame::DecodedSource + '_ {
    fn trigger_table<T: replay_data::rows::TableSchema>(&self) -> Result<&[T::Row], Failure> {
        self.complete::<T>().map_err(|why| {
            let failure = Failure::input(T::ID, "", None, why.to_string());
            failure.resolution_failed();
            failure
        })
    }
}

pub(crate) fn optional<'a, T>(
    value: &'a replay_data::rows::Recorded<Option<T>>,
    table: TableId,
    field: &str,
    scope: Option<Store>,
) -> Result<&'a T, Failure> {
    let name = subject(table, field, scope);
    value
        .require(&name)
        .map_err(|why| Failure::input(table, field, scope, why))?
        .as_ref()
        .ok_or_else(|| Failure::input(table, field, scope, format!("{name}: not captured")))
}

fn subject(table: TableId, field: &str, scope: Option<Store>) -> String {
    let mut out = table.words();
    if !field.is_empty() {
        out.push_str(", ");
        out.push_str(&field.replace('_', " "));
    }
    let whose = match scope {
        None | Some(Store::Global) => None,
        Some(Store::Country(id)) => Some(format!("country {id}")),
        Some(Store::State(id)) => Some(format!("state {id}")),
        Some(Store::Character(id)) => Some(format!("character {id}")),
        Some(Store::Project { id, .. }) => Some(format!("project {id}")),
        Some(Store::IndustrialOrganisation(id)) => Some(format!("industrial organisation {id}")),
    };
    if let Some(whose) = whose {
        out.push_str(" of ");
        out.push_str(&whose);
    }
    out
}

pub(crate) fn deferred<T>(run: impl FnOnce() -> T) -> T {
    DEFERRED.with(|depth| depth.set(depth.get() + 1));
    struct Resume;
    impl Drop for Resume {
        fn drop(&mut self) {
            DEFERRED.with(|depth| depth.set(depth.get() - 1));
        }
    }
    let _resume = Resume;
    run()
}

pub(crate) fn refused(reason: impl Into<Failure>) -> Failure {
    reason
        .into()
        .context(|reason| format!("{}{}", super::REFUSED, reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_filtered_joins_replay_on_cold_warm_and_nested_reads() {
        let cache = Resolved::new(|| {
            deferred(|| {
                Failure::input(
                    TableId::CountryLinks,
                    "canonical_slot",
                    None,
                    "missing owner",
                )
                .resolution_failed();
                42
            })
        });
        let outer = Resolved::new(|| deferred(|| *cache.value()));
        for _ in 0..2 {
            let (value, failures) = capture(|| *outer.value());
            assert_eq!(value, 42);
            assert_eq!(failures.len(), 1);
            assert!(failures.first().unwrap().names("CountryLinks.canonical_slot"));
        }
    }

    #[test]
    fn runtime_dispatch_preserves_its_source_error_without_a_string_round_trip() {
        let (_, failures) = capture(|| {
            let dispatched = content_reader_script::trigger_keys::dispatch_with_error::<(), (), Failure>(
                content_reader_script::trigger_keys::Key {
                    keyword: None,
                    text: b"synthetic",
                    quoted: false,
                    opens: true,
                },
                |_, _| {
                    content_reader_script::trigger_keys::ScopeStep::Unknown(Failure::input(
                        TableId::Countries,
                        "",
                        None,
                        "incomplete countries",
                    ))
                },
                |_| Ok(None),
                |_| None,
            )
            .unwrap();
            assert!(
                dispatched
                    .unless_scope
                    .unwrap()
                    .identities
                    .iter()
                    .all(|id| matches!(
                        id,
                        Identity::Input {
                            table: TableId::Countries,
                            ..
                        }
                    ))
            );
        });
        assert_eq!(failures.len(), 1);
        let _: content_reader_script::trigger_keys::Dispatched<(), ()> =
            content_reader_script::trigger_keys::dispatch(
                content_reader_script::trigger_keys::Key {
                    keyword: None,
                    text: b"synthetic",
                    quoted: false,
                    opens: true,
                },
                |_, _| content_reader_script::trigger_keys::ScopeStep::NotScope,
                |_| Ok(None),
                |_| None,
            )
            .unwrap();
    }

    #[test]
    fn contexts_and_merged_errors_keep_source_identity_without_observing_messages() {
        let (_, failures) = capture(|| {
            let failure =
                Failure::input(TableId::LexicalTokens, "", None, "dictionary unavailable");
            let nested = failure
                .clone()
                .context(|why| format!("tag alias ALI: {why}"));
            let merged = Failure::merge([nested, failure]);
            let wrapped = merged.context(|why| format!("trigger tag = ALI: {why}"));
            assert_eq!(
                wrapped.identities,
                BTreeSet::from([Identity::Input {
                    table: TableId::LexicalTokens,
                    field: String::new(),
                    scope: None,
                }])
            );
        });
        assert_eq!(failures.len(), 1);
        assert!(failures.iter().all(|failure| {
            failure
                .identities
                .iter()
                .all(|id| matches!(id, Identity::Input { .. }))
        }));
    }

    #[test]
    fn lazy_cached_errors_publish_the_original_identity_only_when_reached() {
        let failure = deferred(|| Failure::input(TableId::LexicalTokens, "", None, "unavailable"));
        let (_, untouched) = capture(|| {
            let _ = &failure;
        });
        assert!(untouched.is_empty());
        for _ in 0..2 {
            let (_, reached) = capture(|| {
                failure.reached();
                failure.reached();
            });
            assert_eq!(reached.len(), 1);
            assert_eq!(reached.first().unwrap().identities, failure.identities);
        }
    }

    #[test]
    fn optional_capture_forms_and_contexts_share_the_same_source_identity() {
        use replay_data::rows::Recorded;
        let (_, failures) = capture(|| {
            for value in [Recorded::<Option<i32>>::NotRecorded, Recorded::Value(None)] {
                let failure = optional(
                    &value,
                    TableId::TrainInventory,
                    "required_trains",
                    Some(Store::Country(1)),
                )
                .unwrap_err();
                failure.context(|why| format!("supply temporary: {why}"));
            }
        });
        assert_eq!(failures.len(), 1);
        assert!(failures.first().unwrap().identities.iter().all(|identity| matches!(identity, Identity::Input { table: TableId::TrainInventory, field, scope: Some(Store::Country(1)) } if field == "required_trains")));
    }

    #[test]
    fn an_independent_unsupported_stays_separate_from_a_missing_input() {
        let (_, failures) = capture(|| {
            Failure::input(TableId::LexicalTokens, "", None, "unavailable");
            let failure = Failure::from("unimplemented callback");
            failure.context(|why| format!("trigger context: {why}"));
        });
        assert_eq!(failures.len(), 2);
        assert_eq!(
            failures
                .iter()
                .filter(|failure| failure
                    .identities
                    .iter()
                    .any(|id| matches!(id, Identity::Unsupported(_))))
                .count(),
            1
        );
    }
}
