use super::*;
use std::cell::RefCell;

thread_local! {
    static SCRATCH: RefCell<Option<Scratch>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub reason: &'static str,
    pub code: u32,
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} (Windows error {})", self.reason, self.code)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Serial,
    Collect,
    Report,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Restored,
    Busy,
    Refused(Refusal),
}

struct Group {
    start: usize,
    len: usize,
    ranges: Range<usize>,
    protected: usize,
    outcome: Option<Outcome>,
}

const BUSY: Refusal = Refusal {
    reason: "restore site or native flight remains busy",
    code: ERROR_RETRY,
};

pub struct Scratch {
    references: Vec<*mut Hook>,
    previous: Vec<(bool, bool)>,
    ranges: Vec<Range<usize>>,
    groups: Vec<Group>,
    mode: Mode,
    threads: Threads,
    refusal: Option<Refusal>,
    resume_failed: bool,
    family_attempt: bool,
}

impl Scratch {
    pub fn prepare(hooks: usize) -> Result<Self, &'static str> {
        let mut threads = Threads {
            threads: Vec::new(),
            excluded: Vec::new(),
            census: Vec::new(),
            census_required: 0,
            census_short: false,
            census_refusal: None,
            context: AlignedContext(unsafe { std::mem::zeroed() }),
            snapshot_valid: false,
            diagnostics: publication::Attempt::new(),
        };
        threads
            .prepare_census()
            .map_err(|_| "restore census preparation failed")?;
        let length = query_census(&mut threads.census, &mut threads.census_required)
            .map_err(|_| "restore census preparation failed")?;
        let bytes =
            unsafe { std::slice::from_raw_parts(threads.census.as_ptr().cast::<u8>(), length) };
        let mut count = 0;
        visit_census(
            bytes,
            unsafe { GetCurrentProcessId() },
            unsafe { GetCurrentThreadId() },
            &[],
            |_| {
                count += 1;
                Ok(())
            },
        )
        .map_err(|_| "restore census preparation failed")?;
        threads
            .threads
            .try_reserve_exact(count + count / 2 + 16)
            .map_err(|_| "restore thread preparation failed")?;
        Ok(Self {
            references: engine_allocation::vec(hooks)
                .map_err(|_| "restore hook preparation failed")?,
            previous: engine_allocation::vec(hooks)
                .map_err(|_| "restore state preparation failed")?,
            ranges: engine_allocation::vec(hooks)
                .map_err(|_| "restore range preparation failed")?,
            groups: engine_allocation::vec(hooks)
                .map_err(|_| "restore batch preparation failed")?,
            mode: Mode::Serial,
            threads,
            refusal: None,
            resume_failed: false,
            family_attempt: false,
        })
    }

    pub fn enter(self) -> Guard {
        SCRATCH.with_borrow_mut(|scratch| {
            assert!(scratch.is_none());
            *scratch = Some(self);
        });
        Guard(std::marker::PhantomData)
    }

    fn refuse(&mut self, reason: &'static str, code: u32) -> bool {
        self.refusal.get_or_insert(Refusal { reason, code });
        false
    }

    fn restore<'a>(&mut self, hooks: impl Iterator<Item = &'a mut Hook>) -> bool {
        if self.resume_failed {
            return false;
        }
        if !self.family_attempt {
            self.refusal = None;
        }
        match self.mode {
            Mode::Serial => {}
            Mode::Collect => return self.record(hooks),
            Mode::Report => return self.answer(hooks),
        }
        self.references.clear();
        for hook in hooks {
            if self.references.len() == self.references.capacity()
                || self.references.len() == self.previous.capacity()
                || self.references.len() == self.ranges.capacity()
            {
                return self.refuse(
                    "prepared restore hook storage insufficient",
                    ERROR_INSUFFICIENT_BUFFER,
                );
            }
            self.references.push(hook as *mut Hook);
        }
        let hooks = unsafe {
            std::slice::from_raw_parts_mut(
                self.references.as_mut_ptr().cast::<&mut Hook>(),
                self.references.len(),
            )
        };
        let ready = self.restore_prepared(hooks);
        self.references.clear();
        ready
    }

    fn record<'a>(&mut self, hooks: impl Iterator<Item = &'a mut Hook>) -> bool {
        let start = self.references.len();
        for hook in hooks {
            if self.references.len() == self.references.capacity()
                || self.references.len() == self.previous.capacity()
                || self.references.len() == self.ranges.capacity()
            {
                self.references.truncate(start);
                return self.refuse(
                    "prepared restore hook storage insufficient",
                    ERROR_INSUFFICIENT_BUFFER,
                );
            }
            self.references.push(hook as *mut Hook);
        }
        // SAFETY: the pointers were exclusive references lent by this call.
        if self.references[start..]
            .iter()
            .all(|hook| !unsafe { &**hook }.is_enabled())
        {
            self.references.truncate(start);
            return true;
        }
        if self.groups.len() == self.groups.capacity() {
            self.references.truncate(start);
            return self.refuse(
                "prepared restore hook storage insufficient",
                ERROR_INSUFFICIENT_BUFFER,
            );
        }
        self.groups.push(Group {
            start,
            len: self.references.len() - start,
            ranges: 0..0,
            protected: 0,
            outcome: None,
        });
        false
    }

    fn answer<'a>(&mut self, hooks: impl Iterator<Item = &'a mut Hook>) -> bool {
        let (mut first, mut len, mut enabled) = (ptr::null_mut(), 0, false);
        for hook in hooks {
            if len == 0 {
                first = hook as *mut Hook;
            }
            len += 1;
            enabled |= hook.is_enabled();
        }
        let references = &self.references;
        let outcome = self
            .groups
            .iter_mut()
            .find(|group| {
                group.len == len && references[group.start] == first && group.outcome.is_some()
            })
            .and_then(|group| group.outcome.take());
        match outcome {
            Some(Outcome::Restored) => true,
            Some(Outcome::Refused(refusal)) => {
                self.refusal.get_or_insert(refusal);
                false
            }
            None if !enabled => true,
            Some(Outcome::Busy) | None => self.refuse(BUSY.reason, BUSY.code),
        }
    }

    fn restore_prepared(&mut self, hooks: &mut [&mut Hook]) -> bool {
        if hooks.iter().all(|hook| !hook.is_enabled()) {
            return true;
        }
        let _publication = PUBLICATION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.previous.clear();
        self.previous
            .extend(hooks.iter().map(|hook| (hook.enabled, hook.is_enabled())));
        self.ranges.clear();
        self.ranges.extend(
            hooks
                .iter()
                .zip(&self.previous)
                .filter(|(_, (_, owned))| *owned)
                .map(|(hook, _)| hook.site..hook.site + hook.sentinel_len),
        );
        let mut protected = 0;
        let mut failure = None;
        for (hook, (_, owned)) in hooks.iter_mut().zip(&self.previous) {
            if !owned {
                continue;
            }
            let mut previous = 0;
            if unsafe {
                VirtualProtect(
                    hook.site as *const c_void,
                    hook.sentinel_len,
                    PAGE_EXECUTE_READWRITE,
                    &mut previous,
                )
            } == 0
            {
                failure = Some(Refusal {
                    reason: "restore page preparation failed",
                    code: unsafe { GetLastError() },
                });
                break;
            }
            protected += 1;
        }
        let mut snapshot_refusal = false;
        let outcome = if failure.is_none() {
            (|| {
                self.threads.suspend_prepared()?;
                if !Hook::family_ready::<false>(
                    hooks,
                    false,
                    |index| self.previous[index].1,
                    &self.ranges,
                    &mut self.threads,
                    &mut snapshot_refusal,
                )? {
                    return Ok(false);
                }
                for (hook, (_, owned)) in hooks.iter_mut().zip(&self.previous) {
                    if *owned && hook.enabled {
                        unsafe {
                            ptr::copy_nonoverlapping(
                                hook.original.as_ptr(),
                                hook.site as *mut u8,
                                hook.patch_len,
                            )
                        };
                        hook.enabled = false;
                        hook.needs_flush = true;
                    }
                }
                if let Err(code) = shared::real_flush() {
                    for (hook, (enabled, owned)) in hooks.iter_mut().zip(&self.previous) {
                        if !owned {
                            continue;
                        }
                        let bytes = if *enabled {
                            &hook.patch
                        } else {
                            &hook.original
                        };
                        unsafe {
                            ptr::copy_nonoverlapping(
                                bytes.as_ptr(),
                                hook.site as *mut u8,
                                hook.patch_len,
                            )
                        };
                        hook.enabled = *enabled;
                        hook.needs_flush = true;
                    }
                    if shared::real_flush().is_ok()
                        && hooks
                            .iter()
                            .filter(|hook| hook.needs_flush)
                            .all(|hook| hook.bytes_match(hook.enabled))
                    {
                        for hook in hooks.iter_mut().filter(|hook| hook.needs_flush) {
                            hook.needs_flush = false;
                            hook.verification_failed = false;
                        }
                    }
                    return Err(code);
                }
                for (hook, (_, owned)) in hooks.iter_mut().zip(&self.previous) {
                    if *owned && !hook.bytes_match(false) {
                        hook.verification_failed = true;
                        return Err(0);
                    }
                }
                for hook in hooks.iter_mut() {
                    hook.needs_flush = false;
                    hook.verification_failed = false;
                }
                Ok(true)
            })()
        } else {
            Ok(false)
        };
        for (hook, _) in hooks
            .iter_mut()
            .zip(&self.previous)
            .filter(|(_, (_, owned))| *owned)
            .take(protected)
        {
            let mut previous = 0;
            if unsafe {
                VirtualProtect(
                    hook.site as *const c_void,
                    hook.sentinel_len,
                    hook.page_protection,
                    &mut previous,
                )
            } == 0
            {
                failure = Some(Refusal {
                    reason: "restore page protection failed",
                    code: unsafe { GetLastError() },
                });
                hook.needs_flush = true;
            }
        }
        if let Err(code) = self.threads.resume() {
            self.resume_failed = true;
            return self.refuse("restore thread resume failed", code);
        }
        if let Some(failure) = failure {
            self.refusal.get_or_insert(failure);
            return false;
        }
        match outcome {
            Ok(true) => true,
            Ok(false) => self.refuse("restore site or native flight remains busy", ERROR_RETRY),
            Err(code) => self.refuse(
                if snapshot_refusal || hooks.iter().any(|hook| !hook.bytes_match(hook.enabled)) {
                    "restore hook bytes differ"
                } else {
                    self.threads
                        .census_refusal()
                        .unwrap_or("restore suspension or publication failed")
                },
                code,
            ),
        }
    }

    fn publish_batch(&mut self) -> usize {
        self.mode = Mode::Report;
        if self.groups.is_empty() {
            return 0;
        }
        if self.resume_failed {
            for group in &mut self.groups {
                group.outcome = Some(Outcome::Refused(Refusal {
                    reason: "restore thread resume failed",
                    code: 0,
                }));
            }
            return 0;
        }
        let _publication = PUBLICATION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Self {
            references,
            previous,
            ranges,
            groups,
            threads,
            ..
        } = self;
        // SAFETY: each pointer was an exclusive reference to a distinct hook
        // lent to the collect pass; nothing else reaches those hooks until the
        // report pass.
        let hooks = unsafe {
            std::slice::from_raw_parts_mut(references.as_mut_ptr().cast::<&mut Hook>(), references.len())
        };
        previous.clear();
        previous.extend(hooks.iter().map(|hook| (hook.enabled, hook.is_enabled())));
        ranges.clear();
        for group in groups.iter_mut() {
            let begin = ranges.len();
            let owned = group.start..group.start + group.len;
            ranges.extend(
                hooks[owned.clone()]
                    .iter()
                    .zip(&previous[owned])
                    .filter(|(_, (_, owned))| *owned)
                    .map(|(hook, _)| hook.site..hook.site + hook.sentinel_len),
            );
            group.ranges = begin..ranges.len();
        }
        for group in groups.iter_mut() {
            for index in group.start..group.start + group.len {
                if !previous[index].1 {
                    continue;
                }
                let hook = &hooks[index];
                let mut prior = 0;
                if unsafe {
                    VirtualProtect(
                        hook.site as *const c_void,
                        hook.sentinel_len,
                        PAGE_EXECUTE_READWRITE,
                        &mut prior,
                    )
                } == 0
                {
                    group.outcome = Some(Outcome::Refused(Refusal {
                        reason: "restore page preparation failed",
                        code: unsafe { GetLastError() },
                    }));
                    break;
                }
                group.protected += 1;
            }
        }
        let mut written = false;
        match threads.suspend_prepared() {
            Err(code) => {
                let reason = threads
                    .census_refusal()
                    .unwrap_or("restore suspension or publication failed");
                for group in groups.iter_mut().filter(|group| group.outcome.is_none()) {
                    group.outcome = Some(Outcome::Refused(Refusal { reason, code }));
                }
            }
            Ok(()) => {
                for group in groups.iter_mut().filter(|group| group.outcome.is_none()) {
                    let owned = group.start..group.start + group.len;
                    let members = &mut hooks[owned.clone()];
                    let states = &previous[owned];
                    let mut snapshot_refusal = false;
                    match Hook::family_ready::<false>(
                        members,
                        false,
                        |index| states[index].1,
                        &ranges[group.ranges.clone()],
                        threads,
                        &mut snapshot_refusal,
                    ) {
                        Ok(false) => group.outcome = Some(Outcome::Busy),
                        Err(code) => {
                            group.outcome = Some(Outcome::Refused(Refusal {
                                reason: if snapshot_refusal {
                                    "restore hook bytes differ"
                                } else {
                                    threads
                                        .census_refusal()
                                        .unwrap_or("restore suspension or publication failed")
                                },
                                code,
                            }));
                        }
                        Ok(true) => {
                            for (hook, (_, owned)) in members.iter_mut().zip(states) {
                                if *owned && hook.enabled {
                                    unsafe {
                                        ptr::copy_nonoverlapping(
                                            hook.original.as_ptr(),
                                            hook.site as *mut u8,
                                            hook.patch_len,
                                        )
                                    };
                                    hook.enabled = false;
                                    hook.needs_flush = true;
                                }
                            }
                            group.outcome = Some(Outcome::Restored);
                            written = true;
                        }
                    }
                }
            }
        }
        if written {
            let restored = |group: &Group| group.outcome == Some(Outcome::Restored);
            if let Err(code) = shared::real_flush() {
                for group in groups.iter_mut().filter(|group| restored(group)) {
                    for index in group.start..group.start + group.len {
                        let (enabled, owned) = previous[index];
                        if !owned {
                            continue;
                        }
                        let hook = &mut hooks[index];
                        let bytes = if enabled { &hook.patch } else { &hook.original };
                        unsafe {
                            ptr::copy_nonoverlapping(
                                bytes.as_ptr(),
                                hook.site as *mut u8,
                                hook.patch_len,
                            )
                        };
                        hook.enabled = enabled;
                        hook.needs_flush = true;
                    }
                    group.outcome = Some(Outcome::Refused(Refusal {
                        reason: "restore suspension or publication failed",
                        code,
                    }));
                }
                if shared::real_flush().is_ok()
                    && hooks
                        .iter()
                        .filter(|hook| hook.needs_flush)
                        .all(|hook| hook.bytes_match(hook.enabled))
                {
                    for hook in hooks.iter_mut().filter(|hook| hook.needs_flush) {
                        hook.needs_flush = false;
                        hook.verification_failed = false;
                    }
                }
            } else {
                for group in groups.iter_mut().filter(|group| restored(group)) {
                    let owned = group.start..group.start + group.len;
                    let members = &mut hooks[owned.clone()];
                    let mut verified = true;
                    for (hook, (_, owned)) in members.iter_mut().zip(&previous[owned]) {
                        if *owned && !hook.bytes_match(false) {
                            hook.verification_failed = true;
                            verified = false;
                        }
                    }
                    if !verified {
                        group.outcome = Some(Outcome::Refused(Refusal {
                            reason: "restore hook bytes differ",
                            code: 0,
                        }));
                    } else {
                        for hook in members.iter_mut() {
                            hook.needs_flush = false;
                            hook.verification_failed = false;
                        }
                    }
                }
            }
        }
        for group in groups.iter_mut() {
            let mut protected = group.protected;
            for index in group.start..group.start + group.len {
                if protected == 0 {
                    break;
                }
                if !previous[index].1 {
                    continue;
                }
                protected -= 1;
                let hook = &mut hooks[index];
                let mut prior = 0;
                if unsafe {
                    VirtualProtect(
                        hook.site as *const c_void,
                        hook.sentinel_len,
                        hook.page_protection,
                        &mut prior,
                    )
                } == 0
                {
                    group.outcome = Some(Outcome::Refused(Refusal {
                        reason: "restore page protection failed",
                        code: unsafe { GetLastError() },
                    }));
                    hook.needs_flush = true;
                }
            }
        }
        if let Err(code) = threads.resume() {
            self.resume_failed = true;
            for group in &mut self.groups {
                group.outcome = Some(Outcome::Refused(Refusal {
                    reason: "restore thread resume failed",
                    code,
                }));
            }
            return 0;
        }
        self.groups
            .iter()
            .filter(|group| group.outcome == Some(Outcome::Restored))
            .count()
    }

    fn quiet(
        &mut self,
        ranges: &[Range<usize>],
        excluded: &[u32],
        active: impl FnOnce() -> bool,
    ) -> Result<bool, &'static str> {
        if self.resume_failed {
            return Err("collector quiet thread resume refused; collector retained");
        }
        let result = self.threads.suspend_prepared().and_then(|()| {
            if active() {
                Ok(false)
            } else {
                self.threads
                    .find_collision(
                        excluded,
                        |ip| ranges.iter().any(|range| range.contains(&ip)).then_some(()),
                        thread_exited,
                    )
                    .map(|collision| collision.is_none())
            }
        });
        if let Err(code) = self.threads.resume() {
            self.resume_failed = true;
            self.refuse("quiet thread resume failed", code);
            return Err("collector quiet thread resume refused; collector retained");
        }
        match result {
            Ok(ready) => Ok(ready),
            Err(code) => Ok(self.refuse(
                self.threads
                    .census_refusal()
                    .unwrap_or("quiet thread census or context failed"),
                code,
            )),
        }
    }
}

pub struct Guard(std::marker::PhantomData<*const ()>);
impl Drop for Guard {
    fn drop(&mut self) {
        SCRATCH.with_borrow_mut(|scratch| drop(scratch.take()));
    }
}

pub fn active() -> bool {
    SCRATCH.with_borrow(|scratch| scratch.is_some())
}

pub fn collecting() -> bool {
    SCRATCH.with_borrow(|scratch| {
        scratch
            .as_ref()
            .is_some_and(|scratch| scratch.mode == Mode::Collect)
    })
}

pub struct Batch(std::marker::PhantomData<*const ()>);

impl Batch {
    pub fn collect() -> Option<Self> {
        SCRATCH.with_borrow_mut(|scratch| {
            let scratch = scratch.as_mut()?;
            if scratch.mode != Mode::Serial {
                return None;
            }
            scratch.references.clear();
            scratch.groups.clear();
            scratch.mode = Mode::Collect;
            Some(Self(std::marker::PhantomData))
        })
    }

    pub fn publish(&mut self) -> usize {
        SCRATCH.with_borrow_mut(|scratch| {
            scratch
                .as_mut()
                .expect("prepared restoration")
                .publish_batch()
        })
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        SCRATCH.with_borrow_mut(|scratch| {
            if let Some(scratch) = scratch {
                scratch.mode = Mode::Serial;
                scratch.groups.clear();
                scratch.references.clear();
            }
        });
    }
}

pub fn refusal() -> Option<Refusal> {
    SCRATCH.with_borrow(|scratch| scratch.as_ref().and_then(|scratch| scratch.refusal))
}

pub fn attempt<R>(work: impl FnOnce() -> R) -> (R, Option<Refusal>) {
    struct Attempt;
    impl Drop for Attempt {
        fn drop(&mut self) {
            SCRATCH.with_borrow_mut(|scratch| {
                if let Some(scratch) = scratch {
                    scratch.family_attempt = false;
                }
            });
        }
    }
    SCRATCH.with_borrow_mut(|scratch| {
        if let Some(scratch) = scratch {
            assert!(!scratch.family_attempt);
            scratch.family_attempt = true;
            if !scratch.resume_failed {
                scratch.refusal = None;
            }
        }
    });
    let _attempt = Attempt;
    (work(), refusal())
}

pub(super) fn restore<'a>(hooks: impl Iterator<Item = &'a mut Hook>) -> bool {
    SCRATCH.with_borrow_mut(|scratch| {
        scratch
            .as_mut()
            .expect("prepared restoration")
            .restore(hooks)
    })
}

pub fn quiet(
    ranges: &[Range<usize>],
    excluded: &[u32],
    active: impl FnOnce() -> bool,
) -> Option<Result<bool, &'static str>> {
    SCRATCH.with_borrow_mut(|scratch| {
        scratch
            .as_mut()
            .map(|scratch| scratch.quiet(ranges, excluded, active))
    })
}

#[cfg(test)]
#[path = "restoration/tests.rs"]
mod tests;
