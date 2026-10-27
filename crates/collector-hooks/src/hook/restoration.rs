//! Prepared loader-thread storage for restoration and final IP quiescence.
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

pub struct Scratch {
    references: Vec<*mut Hook>,
    previous: Vec<(bool, bool)>,
    ranges: Vec<Range<usize>>,
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
        // The iterator lends disjoint hooks only for this call; no reference survives it.
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
                        for hook in hooks.iter_mut() {
                            hook.needs_flush = false;
                        }
                    }
                    return Err(code);
                }
                if hooks
                    .iter()
                    .zip(&self.previous)
                    .any(|(hook, (_, owned))| *owned && !hook.bytes_match(false))
                {
                    return Err(0);
                }
                for hook in hooks.iter_mut() {
                    hook.needs_flush = false;
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
mod tests;
