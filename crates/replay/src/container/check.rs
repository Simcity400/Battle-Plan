use std::io;

pub(super) struct Check<'a> {
    callback: Option<&'a mut dyn FnMut() -> io::Result<()>>,
    failed: bool,
}

impl<'a> Check<'a> {
    pub(super) fn new(callback: &'a mut dyn FnMut() -> io::Result<()>) -> Self {
        #[cfg(test)]
        ENABLED_CREATED.set(ENABLED_CREATED.get() + 1);
        Self {
            callback: Some(callback),
            failed: false,
        }
    }

    pub(super) fn disabled() -> Self {
        Self {
            callback: None,
            failed: false,
        }
    }

    pub(super) fn run(&mut self) -> io::Result<()> {
        let Some(callback) = &mut self.callback else {
            return Ok(());
        };
        #[cfg(test)]
        ENABLED_INVOKED.set(ENABLED_INVOKED.get() + 1);
        let result = callback();
        self.failed |= result.is_err();
        result
    }

    // A caller stop must not become a recording refusal or trigger a decoder retry.
    pub(super) fn failed(&self) -> bool {
        self.failed
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Table(crate::rows::TableId),
    Raw(crate::rows::TableId, usize),
    Group(crate::rows::TableId, usize),
    Alias(crate::rows::TableId, usize),
    CatchUp(crate::rows::TableId, usize),
    Keys(usize),
    Verify(usize),
    Source(usize),
}

#[cfg(test)]
thread_local! {
    static STEP: std::cell::Cell<Option<Step>> = const { std::cell::Cell::new(None) };
    static STARTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ENABLED_CREATED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ENABLED_INVOKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn step() -> Option<Step> {
    STEP.get()
}

#[cfg(test)]
pub(super) fn started() -> usize {
    STARTED.get()
}

#[cfg(test)]
pub(super) fn enabled_callbacks() -> (usize, usize) {
    (ENABLED_CREATED.get(), ENABLED_INVOKED.get())
}

#[cfg(test)]
pub(super) struct Scope(Option<Step>);

#[cfg(test)]
impl Scope {
    pub(super) fn enter(step: Step) -> Self {
        STARTED.set(STARTED.get() + 1);
        Self(STEP.replace(Some(step)))
    }
}

#[cfg(test)]
impl Drop for Scope {
    fn drop(&mut self) {
        STEP.set(self.0);
    }
}
