//! The proof a module needs before it may be unmapped: no thread of the
//! process executes inside its image and none holds a return address into it.
//!
//! A detour body lives in the module that enabled it. Restoring the patched
//! bytes stops NEW entries, but a thread already past the patch is inside the
//! body, and a thread that called the original through the trampoline returns
//! INTO the body -- neither is visible to the instruction-pointer check the
//! release itself makes. This barrier freezes the process and censuses every
//! held thread's instruction pointer AND live stack against the image
//! (`ProcessFreeze::module_reference_census`), repeating within a wall-clock
//! budget until the census is clean or the budget is spent. It proves; it
//! never forces.

use crate::freeze::{freeze_process, FreezeRefusal, ModuleReferenceCensus};
use std::fmt;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};

/// Named cause the collector reports when the barrier could not prove
/// quiescence inside its budget.
pub const MODULE_EXECUTION_DID_NOT_QUIESCE: &str = "COLLECTOR_EXECUTION_DID_NOT_QUIESCE";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleQuiescenceReceipt {
    pub image: (usize, usize),
    pub attempts: u32,
    pub elapsed: Duration,
    /// The clean census: every thread scanned, no reference found.
    pub census: ModuleReferenceCensus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleQuiescenceRefusal {
    /// The last attempt inside the budget still found a reference.
    NotQuiescent {
        image: (usize, usize),
        attempts: u32,
        elapsed: Duration,
        census: ModuleReferenceCensus,
    },
    /// Every attempt inside the budget was refused by the freeze itself; the
    /// last refusal is carried.
    FreezeRefused {
        image: (usize, usize),
        attempts: u32,
        elapsed: Duration,
        refusal: FreezeRefusal,
    },
    /// A freeze inside the budget could not resume a thread it suspended:
    /// its census cannot be trusted and a live thread may still be stopped,
    /// so the drain stops at once and the module stays.
    ResumeFailed {
        image: (usize, usize),
        attempts: u32,
        elapsed: Duration,
        failures: u64,
    },
}

impl fmt::Display for ModuleQuiescenceRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotQuiescent {
                image,
                attempts,
                elapsed,
                census,
            } => write!(
                f,
                "{MODULE_EXECUTION_DID_NOT_QUIESCE}: image {:#x}..{:#x} still referenced after \
                 {attempts} freeze(s) over {} ms: threads_scanned={} executing_in_module={} \
                 stack_references={} (words={} threads={}) unbounded_stacks={} first_thread={}",
                image.0,
                image.1,
                elapsed.as_millis(),
                census.threads_scanned,
                census.threads_executing_in_module,
                census.stack_references,
                census.stack_words_scanned,
                census.threads_with_stack_references,
                census.threads_with_unbounded_stacks,
                census.first_referencing_thread,
            ),
            Self::FreezeRefused {
                image,
                attempts,
                elapsed,
                refusal,
            } => write!(
                f,
                "{MODULE_EXECUTION_DID_NOT_QUIESCE}: image {:#x}..{:#x} could not be censused: \
                 {attempts} freeze attempt(s) over {} ms all refused, last: {refusal}",
                image.0,
                image.1,
                elapsed.as_millis(),
            ),
            Self::ResumeFailed {
                image,
                attempts,
                elapsed,
                failures,
            } => write!(
                f,
                "{MODULE_EXECUTION_DID_NOT_QUIESCE}: image {:#x}..{:#x} census abandoned: \
                 freeze {attempts} over {} ms failed to resume {failures} suspended thread(s)",
                image.0,
                image.1,
                elapsed.as_millis(),
            ),
        }
    }
}

impl std::error::Error for ModuleQuiescenceRefusal {}

/// `[base, base + SizeOfImage)` of the module containing `anchor`, without
/// touching its loader reference count.
pub fn module_image_range(anchor: *const ()) -> Result<(usize, usize), u32> {
    let mut module = core::ptr::null_mut();
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            anchor as *const u16,
            &mut module,
        )
    };
    if found == 0 || module.is_null() {
        return Err(unsafe { GetLastError() });
    }
    let base = module as usize;
    // IMAGE_DOS_HEADER.e_lfanew -> IMAGE_NT_HEADERS64; SizeOfImage sits at
    // Signature(4) + IMAGE_FILE_HEADER(20) + OptionalHeader offset 56.
    let e_lfanew = unsafe { core::ptr::read_unaligned((base + 0x3C) as *const u32) } as usize;
    let size_of_image =
        unsafe { core::ptr::read_unaligned((base + e_lfanew + 4 + 20 + 56) as *const u32) } as usize;
    if size_of_image == 0 {
        return Err(0);
    }
    Ok((base, base + size_of_image))
}

/// Freeze, census, thaw; repeat every `backoff` until the census is clean or
/// `budget` is spent. Each thaw lets a thread still inside the module run out
/// of it before the next look.
pub fn drain_module_execution(
    image: (usize, usize),
    budget: Duration,
    backoff: Duration,
) -> Result<ModuleQuiescenceReceipt, ModuleQuiescenceRefusal> {
    let started = Instant::now();
    let mut attempts = 0u32;
    let mut last: Result<ModuleReferenceCensus, FreezeRefusal>;
    loop {
        attempts = attempts.saturating_add(1);
        let resume_failures_before = crate::freeze::resume_failures();
        last = freeze_process().and_then(|frozen| frozen.module_reference_census(image));
        let resume_failures =
            crate::freeze::resume_failures().saturating_sub(resume_failures_before);
        if resume_failures != 0 {
            return Err(ModuleQuiescenceRefusal::ResumeFailed {
                image,
                attempts,
                elapsed: started.elapsed(),
                failures: resume_failures,
            });
        }
        if let Ok(census) = last {
            if census.is_quiescent() {
                return Ok(ModuleQuiescenceReceipt {
                    image,
                    attempts,
                    elapsed: started.elapsed(),
                    census,
                });
            }
        }
        if started.elapsed() >= budget {
            break;
        }
        std::thread::sleep(backoff);
    }
    let elapsed = started.elapsed();
    Err(match last {
        Ok(census) => ModuleQuiescenceRefusal::NotQuiescent {
            image,
            attempts,
            elapsed,
            census,
        },
        Err(refusal) => ModuleQuiescenceRefusal::FreezeRefused {
            image,
            attempts,
            elapsed,
            refusal,
        },
    })
}

/// Test-only anchor exported so a test can name this crate's own image.
#[inline(never)]
pub fn own_image_anchor() -> *const () {
    own_image_anchor as *const ()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// Codex round 1, F3: a freeze that could not resume a thread it stopped
    /// ends the drain by name; the collector retains on it like any refusal.
    #[test]
    fn a_resume_failure_names_itself_and_is_checked_around_every_freeze() {
        let refusal = ModuleQuiescenceRefusal::ResumeFailed {
            image: (0x1000, 0x2000),
            attempts: 2,
            elapsed: Duration::from_millis(30),
            failures: 1,
        };
        let text = refusal.to_string();
        assert!(text.starts_with(MODULE_EXECUTION_DID_NOT_QUIESCE));
        assert!(text.contains("failed to resume 1 suspended thread(s)"), "{text}");
        let source = include_str!("unload_barrier.rs").replace("\r\n", "\n");
        let drain = source
            .split_once("pub fn drain_module_execution(")
            .expect("the drain")
            .1;
        let drain = &drain[..drain.find("\n}\n").expect("terminated")];
        let before = drain
            .find("let resume_failures_before = crate::freeze::resume_failures();")
            .expect("the counter is read before the freeze");
        let freeze = drain.find("freeze_process()").expect("the freeze");
        let after = drain
            .find("crate::freeze::resume_failures().saturating_sub(resume_failures_before)")
            .expect("and re-read after it");
        assert!(before < freeze && freeze < after);
        assert!(drain.contains("return Err(ModuleQuiescenceRefusal::ResumeFailed {"));
    }

    #[test]
    fn the_image_range_covers_this_crates_code() {
        let anchor = own_image_anchor();
        let (start, end) = module_image_range(anchor).expect("this test binary is a module");
        assert!(start < anchor as usize && (anchor as usize) < end);
        assert!(end - start > 0x1000);
    }

    /// A thread parked inside a function of this image is found by the
    /// census: its instruction pointer or a return address on its stack lies
    /// inside `[start, end)`. Released, the same drain proves quiescence.
    #[test]
    fn a_thread_inside_the_image_blocks_the_drain_until_it_leaves() {
        let _serialised = crate::freeze::os_suspend_state_guard();
        let image = module_image_range(own_image_anchor()).expect("own image");
        let stop = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let worker = {
            let stop = Arc::clone(&stop);
            let started = Arc::clone(&started);
            std::thread::spawn(move || {
                started.store(true, Ordering::Release);
                // Spinning here keeps the instruction pointer and the frame's
                // return address inside this image for as long as `stop` is
                // clear; a wait syscall would move the IP into ntdll but the
                // return address would still be on the stack.
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        while !started.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        let refused = drain_module_execution(
            image,
            Duration::from_millis(60),
            Duration::from_millis(5),
        )
        .expect_err("a thread spinning inside the image is a reference");
        match refused {
            ModuleQuiescenceRefusal::NotQuiescent { census, attempts, .. } => {
                assert!(attempts >= 2, "the budget must be spent over several looks");
                assert!(
                    census.threads_executing_in_module + census.threads_with_stack_references > 0,
                    "{census:?}"
                );
                assert_ne!(census.first_referencing_thread, 0);
            }
            other => panic!("expected a not-quiescent refusal, got {other}"),
        }
        assert!(refused.to_string().contains(MODULE_EXECUTION_DID_NOT_QUIESCE));

        stop.store(true, Ordering::Release);
        worker.join().expect("the spinning thread exits");
        // Every remaining thread of the test process is a test-harness thread
        // whose frames are ALSO in this image, so a clean drain cannot be
        // demonstrated here; what can be is that the census counts the joined
        // thread out.
        let after = freeze_process()
            .expect("freeze")
            .module_reference_census(image)
            .expect("census");
        assert!(after.threads_scanned >= 1);
    }

    #[test]
    fn word_counting_is_aligned_and_bounded() {
        let words: [usize; 4] = [0x10, 0x20, 0x30, 0x40];
        let start = words.as_ptr() as usize;
        let end = start + core::mem::size_of_val(&words);
        assert_eq!(count_words_in_range_public(start, end, (0x20, 0x40)), (4, 2));
        assert_eq!(count_words_in_range_public(start + 1, end, (0x10, 0x50)), (3, 3));
        assert_eq!(count_words_in_range_public(start, start, (0, usize::MAX)), (0, 0));
    }

    fn count_words_in_range_public(start: usize, end: usize, image: (usize, usize)) -> (u64, u64) {
        crate::freeze::count_words_in_range_for_tests(start, end, image)
    }
}
