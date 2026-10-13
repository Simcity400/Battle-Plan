use super::*;
use std::os::windows::io::AsRawHandle;

unsafe extern "system" fn original() -> usize {
    7
}
unsafe extern "system" fn callback() -> usize {
    19
}
unsafe fn invoke(address: usize) -> usize {
    let function: unsafe extern "system" fn() -> usize = unsafe { std::mem::transmute(address) };
    unsafe { function() }
}
fn hook_at(code: &TestCode, offset: usize, target: usize) -> (usize, Hook) {
    let (site, bytes) = code.stub(offset, target);
    let hook = unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap();
    (site, hook)
}
fn suspensions() -> usize {
    SUSPENSIONS.with(std::cell::Cell::get)
}
fn lock() -> std::sync::MutexGuard<'static, ()> {
    FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A worker held inside `site`, a stub that jumps to itself, until its target
/// is rewritten to `original`.
struct Held {
    site: usize,
    worker: Option<std::thread::JoinHandle<usize>>,
}
impl Held {
    fn new(site: usize) -> Self {
        let worker = std::thread::spawn(move || unsafe { invoke(site) });
        let handle = worker.as_raw_handle() as HANDLE;
        let started = std::time::Instant::now();
        loop {
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            assert_ne!(unsafe { SuspendThread(handle) }, u32::MAX);
            let mut context = AlignedContext(unsafe { std::mem::zeroed() });
            context.0.ContextFlags = CONTEXT_CONTROL_AMD64;
            assert_ne!(unsafe { GetThreadContext(handle, &mut context.0) }, 0);
            assert_ne!(unsafe { ResumeThread(handle) }, u32::MAX);
            if (site..site + 16).contains(&(context.0.Rip as usize)) {
                break;
            }
            std::thread::yield_now();
        }
        Self {
            site,
            worker: Some(worker),
        }
    }
    /// Let the worker out and put the site's bytes back as they were.
    fn release(mut self) {
        let target = self.site + 11;
        let flush = || {
            assert_ne!(
                unsafe { FlushInstructionCache(GetCurrentProcess(), target as *const c_void, 8) },
                0
            );
        };
        unsafe { ptr::write_unaligned(target as *mut u64, original as *const () as u64) };
        flush();
        assert_eq!(self.worker.take().unwrap().join().unwrap(), 7);
        unsafe { ptr::write_unaligned(target as *mut u64, self.site as u64) };
        flush();
    }
}

/// Two families under one census and suspension: a thread inside one site
/// of the first holds that whole family back (nothing of it written, not
/// ready), while the second publishes; each gets the outcome its own
/// publication would have given. Two families' own publications took two
/// suspensions.
#[test]
fn shared_publication_holds_only_the_family_a_thread_is_in() {
    let _guard = lock();
    let code = TestCode::new();
    let held_site = code.0 as usize + 64;
    let (first, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (_, held_hook) = hook_at(&code, 64, held_site);
    let (other, other_hook) = hook_at(&code, 128, original as *const () as usize);
    let mut held_family = vec![first_hook, held_hook];
    let mut free_family = vec![other_hook];
    let held = Held::new(held_site);
    let before = suspensions();
    {
        let mut groups = [
            Group::new(&mut held_family, true),
            Group::new(&mut free_family, true),
        ];
        publish_groups(&mut groups, real_flush);
        assert_eq!(groups[0].outcome(), Some(&Ok(false)));
        assert_eq!(groups[1].outcome(), Some(&Ok(true)));
    }
    assert_eq!(
        suspensions() - before,
        1,
        "one suspension for both families"
    );
    assert!(
        held_family.iter().all(|hook| !hook.is_enabled()),
        "nothing of the held family"
    );
    assert_eq!(unsafe { invoke(first) }, 7);
    assert_eq!(unsafe { invoke(other) }, 19);
    held.release();
    // Once the thread left, the held family publishes; the other, already
    // enabled, is verified again in the same suspension.
    let before = suspensions();
    {
        let mut groups = [
            Group::new(&mut held_family, true),
            Group::new(&mut free_family, true),
        ];
        publish_groups(&mut groups, real_flush);
        assert_eq!(groups[0].outcome(), Some(&Ok(true)));
        assert_eq!(groups[1].outcome(), Some(&Ok(true)));
    }
    assert_eq!(suspensions() - before, 1);
    assert_eq!(unsafe { invoke(first) }, 19);
    // Restoring both together; then nothing is left to write and no
    // suspension is taken.
    let before = suspensions();
    {
        let mut groups = [
            Group::new(&mut held_family, false),
            Group::new(&mut free_family, false),
        ];
        publish_groups(&mut groups, real_flush);
        assert!(
            groups
                .iter()
                .all(|group| group.outcome() == Some(&Ok(true)))
        );
        publish_groups(&mut groups[..0], real_flush);
    }
    assert_eq!(suspensions() - before, 1);
    let before = suspensions();
    {
        let mut groups = [
            Group::new(&mut held_family, false),
            Group::new(&mut free_family, false),
        ];
        publish_groups(&mut groups, real_flush);
        assert!(
            groups
                .iter()
                .all(|group| group.outcome() == Some(&Ok(true)))
        );
    }
    assert_eq!(suspensions() - before, 0, "already restored");
    assert_eq!(unsafe { invoke(first) }, 7);
    assert_eq!(unsafe { invoke(other) }, 7);
}

/// A failed instruction-cache flush restores every family written in the
/// shared publication, each reporting the failure as its own would.
#[test]
fn a_failed_shared_flush_restores_every_written_family() {
    let _guard = lock();
    let code = TestCode::new();
    let (first, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (second, second_hook) = hook_at(&code, 64, original as *const () as usize);
    let mut first_family = vec![first_hook];
    let mut second_family = vec![second_hook];
    fail_flushes(1);
    {
        let mut groups = [
            Group::new(&mut first_family, true),
            Group::new(&mut second_family, true),
        ];
        publish_groups(&mut groups, real_flush);
        for group in &groups {
            let message = group.outcome().unwrap().as_ref().unwrap_err();
            assert!(
                message.starts_with("instruction-cache publication failed"),
                "{message}"
            );
        }
    }
    assert!(
        first_family
            .iter()
            .chain(&second_family)
            .all(|hook| !hook.is_enabled())
    );
    assert_eq!(unsafe { invoke(first) }, 7);
    assert_eq!(unsafe { invoke(second) }, 7);
}

/// A round: the collect pass records each taking-part family's publication
/// (not ready), one suspension publishes them, and the report pass answers
/// each family's own call with its outcome; a publication the round did not
/// record publishes on its own.
#[test]
fn a_round_publishes_recorded_families_once_and_reports_each() {
    let _guard = lock();
    let code = TestCode::new();
    let (first, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (second, second_hook) = hook_at(&code, 64, original as *const () as usize);
    let (late, late_hook) = hook_at(&code, 128, original as *const () as usize);
    // Both passes reach the same families, one after the other.
    let families = std::cell::RefCell::new([vec![first_hook], vec![second_hook]]);
    let late_family = std::cell::RefCell::new(vec![late_hook]);
    let before = suspensions();
    let mut shared = Shared::default();
    let reported = round(
        || {
            for family in families.borrow_mut().iter_mut() {
                assert_eq!(Hook::set_family_enabled_shared(family, true), Ok(false));
            }
        },
        |published| shared = published,
        || {
            let reports: Vec<_> = families
                .borrow_mut()
                .iter_mut()
                .map(|family| {
                    let outcome = Hook::set_family_enabled_shared(family, true);
                    (outcome, take_reported_ticks().is_some())
                })
                .collect();
            // Not recorded: published on its own, in its own suspension.
            let late = Hook::set_family_enabled_shared(&mut late_family.borrow_mut(), true);
            (reports, late, take_reported_ticks())
        },
    );
    assert_eq!(shared.families, 2);
    assert_eq!(reported.0, vec![(Ok(true), true), (Ok(true), true)]);
    assert_eq!(reported.1, Ok(true));
    assert_eq!(reported.2, None);
    assert_eq!(suspensions() - before, 2, "one shared, one of its own");
    for site in [first, second, late] {
        assert_eq!(unsafe { invoke(site) }, 19);
    }
    // Outside a round every publication is its own.
    assert_eq!(
        Hook::set_family_enabled_shared(&mut families.borrow_mut()[0], false),
        Ok(true)
    );
    let before = suspensions();
    let each = |enabled: bool, expected: Option<Result<bool, String>>| {
        let mut families = families.borrow_mut();
        let mut late = late_family.borrow_mut();
        for family in families.iter_mut().chain(std::iter::once(&mut *late)) {
            let outcome = Hook::set_family_enabled_shared(family, enabled);
            if let Some(expected) = &expected {
                assert_eq!(&outcome, expected);
            }
        }
    };
    round(
        || each(false, None),
        |published| assert_eq!(published.families, 2, "the first is already restored"),
        || each(false, Some(Ok(true))),
    );
    assert_eq!(suspensions() - before, 1);
    for site in [first, second, late] {
        assert_eq!(unsafe { invoke(site) }, 7);
    }
}

/// The protection of the page holding `site`, as `VirtualQuery` reports it.
fn protection_at(site: usize) -> u32 {
    let mut region: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    assert_ne!(
        unsafe {
            VirtualQuery(
                site as *const c_void,
                &mut region,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        },
        0
    );
    region.Protect
}

/// Two families with sites on one execute-read page restored in one detach
/// round, one of whose page-protection restores fails. That family reports
/// the failure, as its own publication would; the other still restores and
/// reports success, and the shared page ends execute-read whichever family
/// refused: nothing reopens a page after the restores, so the restore that
/// succeeded leaves it with its saved protection, and no family that
/// reports success leaves its page writable.
#[test]
fn a_detach_round_restores_a_shared_page_when_one_familys_restore_fails() {
    let _guard = lock();
    let code = TestCode::new();
    let (first, first_bytes) = code.stub(0, original as *const () as usize);
    let (later, later_bytes) = code.stub(64, original as *const () as usize);
    let mut prior = 0;
    assert_ne!(
        unsafe { VirtualProtect(code.0, 4096, PAGE_EXECUTE_READ, &mut prior) },
        0
    );
    let hook = |site, bytes| unsafe { Hook::new(site, callback as *const () as usize, bytes) };
    let mut first_family = vec![hook(first, first_bytes).unwrap()];
    let mut later_family = vec![hook(later, later_bytes).unwrap()];
    assert_eq!(first_family[0].page_protection, PAGE_EXECUTE_READ);
    let protection =
        || -> Result<bool, String> { Err("failed to restore hook-page protection".into()) };
    // The first family's restore refuses, then the later family's.
    for (skip, refused) in [(0, 0), (1, 1)] {
        for family in [&mut first_family, &mut later_family] {
            assert_eq!(Hook::set_family_enabled(family, true), Ok(true));
        }
        assert_eq!(protection_at(first), PAGE_EXECUTE_READ);
        assert_eq!(unsafe { invoke(first) }, 19);
        assert_eq!(unsafe { invoke(later) }, 19);
        fail_restores(skip, 1);
        {
            let mut groups = [
                Group::new(&mut first_family, false),
                Group::new(&mut later_family, false),
            ];
            publish_groups(&mut groups, real_flush);
            for (index, group) in groups.iter().enumerate() {
                let expected = if index == refused {
                    protection()
                } else {
                    Ok(true)
                };
                assert_eq!(group.outcome(), Some(&expected), "family {index}");
            }
        }
        assert_eq!(
            protection_at(first),
            PAGE_EXECUTE_READ,
            "the shared page is not left writable"
        );
        for family in [&first_family, &later_family] {
            assert!(!family[0].is_enabled() && !family[0].needs_flush);
        }
        assert_eq!(unsafe { invoke(first) }, 7);
        assert_eq!(unsafe { invoke(later) }, 7);
    }
}

/// A family whose verification fails after the shared flush reports that
/// failure, as its own publication would, and keeps its executable storage;
/// every other family of the round is still written, verified and reported
/// on its own merits, never skipped.
#[test]
fn a_shared_verification_failure_refuses_only_its_own_family() {
    let _guard = lock();
    let code = TestCode::new();
    let (first, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (later, later_hook) = hook_at(&code, 64, original as *const () as usize);
    let mut first_family = vec![first_hook];
    let mut later_family = vec![later_hook];
    {
        let mut groups = [
            Group::new(&mut first_family, true),
            Group::new(&mut later_family, true),
        ];
        // After a successful flush, the first site loses its jump.
        publish_groups(&mut groups, || {
            let flushed = real_flush();
            unsafe { ptr::write(first as *mut u8, 0x90) };
            flushed
        });
        let message = groups[0].outcome().unwrap().as_ref().unwrap_err();
        assert!(
            message.starts_with("patch refused or verification failed")
                && message.contains("executable storage retained"),
            "{message}"
        );
        assert_eq!(groups[1].outcome(), Some(&Ok(true)));
    }
    assert!(first_family[0].needs_flush, "storage retained for detach");
    assert_eq!(unsafe { invoke(later) }, 19);
    // Put the jump back so the first family restores as written.
    unsafe { ptr::write(first as *mut u8, 0xe9) };
    for family in [&mut first_family, &mut later_family] {
        assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
    }
    assert_eq!(unsafe { invoke(first) }, 7);
    assert_eq!(unsafe { invoke(later) }, 7);
}

/// [`a_shared_verification_failure_refuses_only_its_own_family`] in a detach
/// round: a family whose restored bytes fail verification after the shared
/// flush reports that failure and stays unrestored (its executable storage
/// kept), while the other family is restored and reports success, never
/// skipped. The next round retries the refused family on its own merits.
#[test]
fn a_detach_verification_failure_refuses_only_its_own_family() {
    let _guard = lock();
    let code = TestCode::new();
    let (first, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (later, later_hook) = hook_at(&code, 64, original as *const () as usize);
    let mut first_family = vec![first_hook];
    let mut later_family = vec![later_hook];
    for family in [&mut first_family, &mut later_family] {
        assert_eq!(Hook::set_family_enabled(family, true), Ok(true));
    }
    {
        let mut groups = [
            Group::new(&mut first_family, false),
            Group::new(&mut later_family, false),
        ];
        // After a successful flush, the first site's jump opcode is back.
        publish_groups(&mut groups, || {
            let flushed = real_flush();
            unsafe { ptr::write(first as *mut u8, 0xe9) };
            flushed
        });
        let message = groups[0].outcome().unwrap().as_ref().unwrap_err();
        assert!(
            message.starts_with("patch refused or verification failed")
                && message.contains("executable storage retained"),
            "{message}"
        );
        assert_eq!(groups[1].outcome(), Some(&Ok(true)));
    }
    assert!(
        first_family[0].is_enabled() && first_family[0].needs_flush,
        "unrestored, storage retained for detach"
    );
    assert!(!later_family[0].is_enabled());
    assert_eq!(unsafe { invoke(later) }, 7);
    // Once the site holds its original bytes again, the next round restores
    // the refused family; the other has nothing left to write.
    unsafe { ptr::write(first as *mut u8, 0x90) };
    {
        let mut groups = [
            Group::new(&mut first_family, false),
            Group::new(&mut later_family, false),
        ];
        publish_groups(&mut groups, real_flush);
        assert!(
            groups
                .iter()
                .all(|group| group.outcome() == Some(&Ok(true)))
        );
    }
    assert!(!first_family[0].is_enabled());
    assert_eq!(unsafe { invoke(first) }, 7);
    assert_eq!(unsafe { invoke(later) }, 7);
}
