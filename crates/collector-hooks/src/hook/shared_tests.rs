use super::*;
use std::os::windows::io::AsRawHandle;

unsafe extern "system-unwind" fn tap_observer(_: &Registers, _: usize) {}

fn prefix(groups: &mut [Group<'_>], flush: impl FnMut() -> Result<(), u32>) {
    publish_groups_with_order(groups, flush, true);
}

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

#[test]
fn attach_outcome_retains_typed_rollback_before_reference_rebuilding() {
    let _guard = lock();
    let code = TestCode::new();
    let (site, hook) = hook_at(&code, 0, original as *const () as usize);
    let mut hooks = [hook];
    let mut round = Round::collect_attach().unwrap();
    publication::select_family(Some("experience_gains"));
    assert_eq!(
        Hook::set_enabled_batch(&mut [&mut hooks[0]], true),
        Ok(true)
    );
    fail_flushes(2);
    round.publish();
    let (outcome, report) = installation::capture(attach_outcome);
    assert!(outcome.unwrap().is_err());
    let failure = report.failure.unwrap();
    assert_eq!(failure.kind, installation::Kind::Rollback);
    assert_eq!(failure.site, Some(site));
    drop(round);
    assert_eq!(Hook::set_family_enabled(&mut hooks, false), Ok(true));
    assert!(!hooks[0].is_enabled());
    assert!(failure.cause.contains("restored bytes remain unflushed"));
}

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
            assert_ne!(unsafe { GetThreadContext(handle, &raw mut context.0) }, 0);
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

#[test]
fn a_round_publishes_recorded_families_once_and_reports_each() {
    let _guard = lock();
    let code = TestCode::new();
    let (first, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (second, second_hook) = hook_at(&code, 64, original as *const () as usize);
    let (late, late_hook) = hook_at(&code, 128, original as *const () as usize);
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

fn protection_at(site: usize) -> u32 {
    let mut region: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    assert_ne!(
        unsafe {
            VirtualQuery(
                site as *const c_void,
                &raw mut region,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        },
        0
    );
    region.Protect
}

#[test]
fn a_detach_round_restores_a_shared_page_when_one_familys_restore_fails() {
    let _guard = lock();
    let code = TestCode::new();
    let (first, first_bytes) = code.stub(0, original as *const () as usize);
    let (later, later_bytes) = code.stub(64, original as *const () as usize);
    let mut prior = 0;
    assert_ne!(
        unsafe { VirtualProtect(code.0, 4096, PAGE_EXECUTE_READ, &raw mut prior) },
        0
    );
    let hook = |site, bytes| unsafe { Hook::new(site, callback as *const () as usize, bytes) };
    let mut first_family = vec![hook(first, first_bytes).unwrap()];
    let mut later_family = vec![hook(later, later_bytes).unwrap()];
    assert_eq!(first_family[0].page_protection, PAGE_EXECUTE_READ);
    let protection =
        || -> Result<bool, String> { Err("failed to restore hook-page protection".into()) };
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
    unsafe { ptr::write(first as *mut u8, 0xe9) };
    for family in [&mut first_family, &mut later_family] {
        assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
    }
    assert_eq!(unsafe { invoke(first) }, 7);
    assert_eq!(unsafe { invoke(later) }, 7);
}

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

#[test]
fn attach_many_families_and_taps_share_one_measured_suspension() {
    let _guard = lock();
    const NAMES: [&str; 4] = ["a", "b", "c", "d"];
    let code = TestCode::new();
    let mut families: Vec<_> = (0..NAMES.len())
        .map(|index| vec![hook_at(&code, index * 64, original as *const () as usize).1])
        .collect();
    let (tap_site, sentinel) = code.stub(2048, original as *const () as usize);
    let mut tap = unsafe { Tap::new(tap_site, tap_observer, 0, sentinel) }.unwrap();
    let (ordinary_site, mut ordinary) = hook_at(&code, 2112, original as *const () as usize);
    let measurement = || {
        publication::snapshot()
            .0
            .into_iter()
            .find(|(name, _)| *name == "attach_shared")
            .map(|(_, summary)| summary)
            .unwrap_or_default()
    };
    let prior = measurement();
    let before = suspensions();
    {
        let mut round = Round::collect_attach().unwrap();
        for (name, family) in NAMES.iter().zip(&mut families) {
            publication::select_family(Some(name));
            assert_eq!(Hook::set_family_enabled_shared(family, true), Ok(true));
        }
        publication::select_family(Some("mixed_taps"));
        assert_eq!(
            Hook::set_enabled_batch(&mut [tap.publication_hook(), &mut ordinary], true),
            Ok(true)
        );
        assert_eq!(round.publish().families, NAMES.len() + 1);
        for (name, family) in NAMES.iter().zip(&mut families) {
            publication::select_family(Some(name));
            assert_eq!(Hook::set_family_enabled_shared(family, true), Ok(true));
        }
        publication::select_family(Some("mixed_taps"));
        crate::test_support::with_allocations_denied(|| {
            assert_eq!(attach_outcome(), Some(Ok(true)));
        });
    }
    publication::select_family(None);
    assert_eq!(suspensions() - before, 1);
    let measured = measurement();
    assert_eq!(measured.attempts - prior.attempts, 1);
    assert!(measured.total_hold_ns > prior.total_hold_ns);
    assert!(measured.longest_hold_ns > 0);
    assert!(families.iter().flatten().all(Hook::is_enabled));
    assert!(tap.publication_hook().is_enabled() && ordinary.is_enabled());
    assert_eq!(unsafe { invoke(ordinary_site) }, 19);
    for family in &mut families {
        assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
    }
    assert_eq!(
        Hook::set_enabled_batch(&mut [tap.publication_hook(), &mut ordinary], false),
        Ok(true)
    );
}

#[test]
fn attach_takes_one_suspension_for_a_family_of_several_publications() {
    let _guard = lock();
    let code = TestCode::new();
    let (main_site, main_hook) = hook_at(&code, 0, original as *const () as usize);
    let (radar_site, radar_hook) = hook_at(&code, 64, original as *const () as usize);
    let (crews_site, crews_hook) = hook_at(&code, 128, original as *const () as usize);
    let (later_site, later_hook) = hook_at(&code, 192, original as *const () as usize);
    let mut main = vec![main_hook];
    let mut radar = radar_hook;
    let mut crews = vec![crews_hook];
    let mut later = vec![later_hook];
    let set_enabled = |main: &mut Vec<Hook>, radar: &mut Hook, crews: &mut Vec<Hook>| {
        publication::select_family(Some("missions"));
        let mut steps = 0;
        if !Hook::set_family_enabled_shared(main, true)? {
            return Ok((false, steps));
        }
        steps += 1;
        if !radar.set_enabled(true)? {
            return Ok((false, steps));
        }
        Hook::set_family_enabled(crews, true).map(|ready| (ready, steps))
    };
    let before = suspensions();
    {
        let mut round = Round::collect_attach().unwrap();
        assert_eq!(set_enabled(&mut main, &mut radar, &mut crews), Ok((true, 1)));
        publication::select_family(Some("later"));
        assert_eq!(Hook::set_family_enabled_shared(&mut later, true), Ok(true));
        assert_eq!(round.publish().families, 4);
        assert_eq!(suspensions() - before, 1);
        assert_eq!(set_enabled(&mut main, &mut radar, &mut crews), Ok((true, 1)));
        publication::select_family(Some("later"));
        assert_eq!(Hook::set_family_enabled_shared(&mut later, true), Ok(true));
    }
    publication::select_family(None);
    assert_eq!(suspensions() - before, 1, "every publication in one suspension");
    for site in [main_site, radar_site, crews_site, later_site] {
        assert_eq!(unsafe { invoke(site) }, 19);
    }
    assert_eq!(Hook::set_family_enabled(&mut main, false), Ok(true));
    assert_eq!(radar.set_enabled(false), Ok(true));
    assert_eq!(Hook::set_family_enabled(&mut crews, false), Ok(true));
    assert_eq!(Hook::set_family_enabled(&mut later, false), Ok(true));
}

#[test]
fn attach_busy_middle_publishes_only_the_prefix_and_retries_from_it() {
    assert_busy_prefix_publication(true);
}

#[test]
fn untracked_shared_family_keeps_markers_untouched_and_never_rescans() {
    assert_busy_prefix_publication(false);
}

fn assert_busy_prefix_publication(track_busy: bool) {
    let _guard = lock();
    let code = TestCode::new();
    let (first_site, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let held_site = code.0 as usize + 64;
    let (_, held_hook) = hook_at(&code, 64, held_site);
    let (later_site, later_hook) = hook_at(&code, 128, original as *const () as usize);
    let mut first = vec![first_hook];
    let mut middle = vec![held_hook];
    let mut later = vec![later_hook];
    first[0]
        .publication_busy
        .store(!track_busy, Ordering::Relaxed);
    let later_bytes = later[0].original;
    let middle_bytes = middle[0].original;
    let held = Held::new(held_site);
    let before = suspensions();
    {
        let mut groups = [
            Group::new(&mut first, true),
            Group::new(&mut middle, true),
            Group::new(&mut later, true),
        ];
        groups[1].track_busy = track_busy;
        IP_SCANS.with(|count| count.set(0));
        prefix(&mut groups, real_flush);
        assert_eq!(groups[0].outcome(), Some(&Ok(true)));
        assert_eq!(groups[1].outcome(), Some(&Ok(false)));
        assert_eq!(groups[2].outcome(), Some(&Ok(false)));
    }
    assert_eq!(
        IP_SCANS.with(std::cell::Cell::get),
        2,
        "one scan per reached family"
    );
    assert_eq!(first[0].publication_busy(), !track_busy);
    assert_eq!(middle[0].publication_busy(), track_busy);
    assert!(!later[0].publication_busy());
    assert_eq!(suspensions() - before, 1);
    assert_eq!(unsafe { invoke(first_site) }, 19);
    assert_eq!(
        unsafe { std::slice::from_raw_parts(held_site as *const u8, 16) },
        middle_bytes
    );
    assert_eq!(
        unsafe { std::slice::from_raw_parts(later_site as *const u8, 16) },
        later_bytes
    );
    held.release();
    let before = suspensions();
    {
        let mut groups = [
            Group::new(&mut first, true),
            Group::new(&mut middle, true),
            Group::new(&mut later, true),
        ];
        groups[1].track_busy = track_busy;
        prefix(&mut groups, real_flush);
        assert!(
            groups
                .iter()
                .all(|group| group.outcome() == Some(&Ok(true)))
        );
    }
    assert_eq!(suspensions() - before, 1);
    assert!(
        !middle[0].publication_busy(),
        "a recovered publication clears its prior busy site"
    );
    assert_eq!(
        first[0].publication_busy(),
        !track_busy,
        "untracked markers never reset"
    );
    assert_eq!(unsafe { invoke(later_site) }, 19);
    for family in [&mut first, &mut middle, &mut later] {
        assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
    }
}

#[test]
fn attach_snapshot_and_preparation_refusals_leave_later_families_untouched() {
    let _guard = lock();
    for preparation in [false, true] {
        let code = TestCode::new();
        let (_, first_hook) = hook_at(&code, 0, original as *const () as usize);
        let (middle_site, middle_hook) = hook_at(&code, 64, original as *const () as usize);
        let (later_site, later_hook) = hook_at(&code, 128, original as *const () as usize);
        let mut first = vec![first_hook];
        let mut middle = vec![middle_hook];
        let mut later = vec![later_hook];
        if preparation {
            middle[0].site = 0;
        } else {
            unsafe { ptr::write((middle_site + 15) as *mut u8, 0xcc) };
        }
        let before = suspensions();
        {
            let mut groups = [
                Group::new(&mut first, true),
                Group::new(&mut middle, true),
                Group::new(&mut later, true),
            ];
            prefix(&mut groups, real_flush);
            assert_eq!(groups[0].outcome(), Some(&Ok(true)));
            assert!(groups[1].outcome().unwrap().is_err());
            if !preparation {
                assert_eq!(
                    groups[1].outcome().unwrap().as_ref().unwrap_err(),
                    BYTE_VERIFICATION_FAILURE
                );
            }
            assert_eq!(groups[2].outcome(), Some(&Ok(false)));
        }
        assert_eq!(suspensions() - before, 1);
        assert!(first[0].is_enabled());
        assert!(!middle[0].is_enabled() && !later[0].is_enabled());
        assert_eq!(
            unsafe { std::slice::from_raw_parts(later_site as *const u8, 16) },
            later[0].original
        );
        middle[0].site = middle_site;
        unsafe { ptr::write((middle_site + 15) as *mut u8, middle[0].original[15]) };
        let before = suspensions();
        {
            let mut groups = [
                Group::new(&mut first, true),
                Group::new(&mut middle, true),
                Group::new(&mut later, true),
            ];
            prefix(&mut groups, real_flush);
            assert!(
                groups
                    .iter()
                    .all(|group| group.outcome() == Some(&Ok(true)))
            );
        }
        assert_eq!(
            suspensions() - before,
            1,
            "retry batches all three families"
        );
        for family in [&mut first, &mut middle, &mut later] {
            assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
        }
    }
}

#[test]
fn attach_verifies_the_middle_family_before_writing_a_later_one() {
    let _guard = lock();
    let code = TestCode::new();
    let (_, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (middle_site, middle_hook) = hook_at(&code, 64, original as *const () as usize);
    let (later_site, later_hook) = hook_at(&code, 128, original as *const () as usize);
    let mut first = vec![first_hook];
    let mut middle = vec![middle_hook];
    let mut later = vec![later_hook];
    let mut flushes = 0;
    let before = suspensions();
    {
        let mut groups = [
            Group::new(&mut first, true),
            Group::new(&mut middle, true),
            Group::new(&mut later, true),
        ];
        prefix(&mut groups, || {
            flushes += 1;
            let result = real_flush();
            if flushes == 2 {
                unsafe { ptr::write(middle_site as *mut u8, 0x90) };
            }
            result
        });
        assert_eq!(groups[0].outcome(), Some(&Ok(true)));
        assert!(
            groups[1]
                .outcome()
                .unwrap()
                .as_ref()
                .unwrap_err()
                .starts_with("patch refused or verification failed")
        );
        assert_eq!(groups[2].outcome(), Some(&Ok(false)));
    }
    assert_eq!(flushes, 2);
    assert_eq!(suspensions() - before, 1);
    assert!(middle[0].needs_flush);
    assert_eq!(
        unsafe { std::slice::from_raw_parts(later_site as *const u8, 16) },
        later[0].original
    );
    unsafe { ptr::write(middle_site as *mut u8, 0xe9) };
    {
        let mut groups = [
            Group::new(&mut first, true),
            Group::new(&mut middle, true),
            Group::new(&mut later, true),
        ];
        prefix(&mut groups, real_flush);
        assert!(
            groups
                .iter()
                .all(|group| group.outcome() == Some(&Ok(true)))
        );
    }
    for family in [&mut first, &mut middle, &mut later] {
        assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
    }
}

#[test]
fn attach_later_flush_failure_rolls_back_every_written_family() {
    let _guard = lock();
    let code = TestCode::new();
    let (first_site, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (_, middle_hook) = hook_at(&code, 64, original as *const () as usize);
    let (later_site, later_hook) = hook_at(&code, 128, original as *const () as usize);
    let mut first = vec![first_hook];
    let mut middle = vec![middle_hook];
    let mut later = vec![later_hook];
    let before = suspensions();
    let mut flushes = 0;
    {
        let mut groups = [
            Group::new(&mut first, true),
            Group::new(&mut middle, true),
            Group::new(&mut later, true),
        ];
        prefix(&mut groups, || {
            flushes += 1;
            if flushes == 2 { Err(31) } else { real_flush() }
        });
        for group in &groups[..2] {
            assert!(
                group
                    .outcome()
                    .unwrap()
                    .as_ref()
                    .unwrap_err()
                    .starts_with("instruction-cache publication failed")
            );
        }
        assert_eq!(groups[2].outcome(), Some(&Ok(false)));
    }
    assert_eq!(suspensions() - before, 1);
    assert_eq!(flushes, 3);
    assert!(
        first
            .iter()
            .chain(&middle)
            .chain(&later)
            .all(|hook| !hook.is_enabled())
    );
    assert_eq!(unsafe { invoke(first_site) }, 7);
    assert_eq!(
        unsafe { std::slice::from_raw_parts(later_site as *const u8, 16) },
        later[0].original
    );
}

#[test]
fn attach_protection_refusal_stops_before_a_later_write_on_a_shared_page() {
    let _guard = lock();
    let code = TestCode::new();
    let (first_site, first_bytes) = code.stub(0, original as *const () as usize);
    let (middle_site, middle_bytes) = code.stub(64, original as *const () as usize);
    let (later_site, later_bytes) = code.stub(128, original as *const () as usize);
    let mut prior = 0;
    assert_ne!(
        unsafe { VirtualProtect(code.0, 4096, PAGE_EXECUTE_READ, &raw mut prior) },
        0
    );
    let make = |site, bytes| {
        vec![unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap()]
    };
    let mut first = make(first_site, first_bytes);
    let mut middle = make(middle_site, middle_bytes);
    let mut later = make(later_site, later_bytes);
    fail_restores(1, 1);
    let before = suspensions();
    {
        let mut groups = [
            Group::new(&mut first, true),
            Group::new(&mut middle, true),
            Group::new(&mut later, true),
        ];
        prefix(&mut groups, real_flush);
        assert_eq!(groups[0].outcome(), Some(&Ok(true)));
        assert_eq!(
            groups[1].outcome(),
            Some(&Err("failed to restore hook-page protection".into()))
        );
        assert_eq!(groups[2].outcome(), Some(&Ok(false)));
    }
    assert_eq!(protection_at(first_site), PAGE_EXECUTE_READ);
    assert_eq!(suspensions() - before, 1);
    assert_eq!(
        unsafe { std::slice::from_raw_parts(later_site as *const u8, 16) },
        later_bytes
    );
    for family in [&mut first, &mut middle, &mut later] {
        assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
    }
}

#[test]
fn attach_recording_allocation_failure_abandons_the_whole_batch() {
    let _guard = lock();
    let code = TestCode::new();
    let (_, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (_, middle_hook) = hook_at(&code, 64, original as *const () as usize);
    let (later_site, later_hook) = hook_at(&code, 128, original as *const () as usize);
    let mut first = vec![first_hook];
    let mut middle = middle_hook;
    let mut later = vec![later_hook];
    let before = suspensions();
    {
        let mut round = Round::collect_attach().unwrap();
        publication::select_family(Some("allocation_first"));
        assert_eq!(Hook::set_family_enabled_shared(&mut first, true), Ok(true));
        publication::select_family(Some("allocation_middle"));
        crate::test_support::with_allocations_denied(|| {
            assert_eq!(
                Hook::set_enabled_batch(&mut [&mut middle], true),
                Ok(false)
            );
        });
        publication::select_family(Some("allocation_later"));
        assert_eq!(Hook::set_family_enabled_shared(&mut later, true), Ok(false));
        assert_eq!(round.publish().families, 0);
        assert_eq!(suspensions() - before, 0);
        assert_eq!(
            unsafe { std::slice::from_raw_parts(later_site as *const u8, 16) },
            later[0].original
        );
        publication::select_family(Some("allocation_first"));
        assert_eq!(Hook::set_family_enabled_shared(&mut first, true), Ok(true));
        publication::select_family(Some("allocation_middle"));
        assert_eq!(
            Hook::set_enabled_batch(&mut [&mut middle], true),
            Ok(true)
        );
        publication::select_family(Some("allocation_later"));
        assert_eq!(Hook::set_family_enabled_shared(&mut later, true), Ok(true));
    }
    publication::select_family(None);
    assert_eq!(suspensions() - before, 3);
    assert_eq!(Hook::set_family_enabled(&mut first, false), Ok(true));
    assert_eq!(middle.set_enabled(false), Ok(true));
    assert_eq!(Hook::set_family_enabled(&mut later, false), Ok(true));
}

#[test]
fn detach_recording_allocation_failure_leaves_only_that_family_to_itself() {
    let _guard = lock();
    let code = TestCode::new();
    let (first_site, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (middle_site, middle_hook) = hook_at(&code, 64, original as *const () as usize);
    let (later_site, later_hook) = hook_at(&code, 128, original as *const () as usize);
    let mut families = [vec![first_hook], vec![middle_hook], vec![later_hook]];
    for family in families.iter_mut() {
        assert_eq!(Hook::set_family_enabled(family, true), Ok(true));
    }
    let before = suspensions();
    {
        let mut round = Round::collect().unwrap();
        let [first, middle, later] = &mut families;
        assert_eq!(Hook::set_family_enabled_shared(first, false), Ok(false));
        super::RECORDED.with_borrow_mut(Vec::shrink_to_fit);
        crate::test_support::with_allocations_denied(|| {
            assert_eq!(Hook::set_family_enabled_shared(middle, false), Ok(false));
        });
        assert_eq!(Hook::set_family_enabled_shared(later, false), Ok(false));
        assert_eq!(round.publish().families, 2);
        for family in families.iter_mut() {
            assert_eq!(Hook::set_family_enabled_shared(family, false), Ok(true));
        }
    }
    assert_eq!(suspensions() - before, 2, "one shared, one of its own");
    for site in [first_site, middle_site, later_site] {
        assert_eq!(unsafe { invoke(site) }, 7);
    }
}

#[test]
fn attach_rollback_reopen_failure_keeps_executable_owners_for_detach() {
    let _guard = lock();
    let code = TestCode::new();
    let (first_site, first_bytes) = code.stub(0, original as *const () as usize);
    let (second_site, second_bytes) = code.stub(64, original as *const () as usize);
    let mut prior = 0;
    assert_ne!(
        unsafe { VirtualProtect(code.0, 4096, PAGE_EXECUTE_READ, &raw mut prior) },
        0
    );
    let make = |site, bytes| {
        vec![unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap()]
    };
    let mut first = make(first_site, first_bytes);
    let mut second = make(second_site, second_bytes);
    FAIL_REOPENS.set((2, 1));
    let mut flushes = 0;
    {
        let mut groups = [Group::new(&mut first, true), Group::new(&mut second, true)];
        prefix(&mut groups, || {
            flushes += 1;
            if flushes == 2 { Err(31) } else { real_flush() }
        });
        let message = groups[0].outcome().unwrap().as_ref().unwrap_err();
        assert!(
            message.contains("instruction-cache publication failed")
                && message.contains("rollback page preparation failed")
                && message.contains("executable storage retained")
        );
        assert!(
            groups[1]
                .outcome()
                .unwrap()
                .as_ref()
                .unwrap_err()
                .starts_with("instruction-cache publication failed")
        );
    }
    assert!(first[0].enabled && first[0].needs_flush);
    assert!(!second[0].is_enabled());
    assert_eq!(protection_at(first_site), PAGE_EXECUTE_READ);
    assert_eq!(unsafe { invoke(first_site) }, 19);
    for family in [&mut first, &mut second] {
        assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
    }
    assert_eq!(unsafe { invoke(first_site) }, 7);
}

#[test]
fn attach_first_preparation_refusal_never_suspends_or_writes_the_suffix() {
    let _guard = lock();
    let code = TestCode::new();
    let (first_site, first_hook) = hook_at(&code, 0, original as *const () as usize);
    let (later_site, later_hook) = hook_at(&code, 64, original as *const () as usize);
    let mut first = vec![first_hook];
    let mut later = vec![later_hook];
    first[0].site = 0;
    let before = suspensions();
    {
        let mut groups = [Group::new(&mut first, true), Group::new(&mut later, true)];
        prefix(&mut groups, real_flush);
        assert!(groups[0].outcome().unwrap().is_err());
        assert_eq!(groups[1].outcome(), Some(&Ok(false)));
    }
    first[0].site = first_site;
    assert_eq!(suspensions() - before, 0);
    assert_eq!(
        unsafe { std::slice::from_raw_parts(later_site as *const u8, 16) },
        later[0].original
    );
    assert!(!first[0].is_enabled() && !later[0].is_enabled());
    let before = suspensions();
    {
        let mut groups = [Group::new(&mut first, true), Group::new(&mut later, true)];
        prefix(&mut groups, real_flush);
        assert!(
            groups
                .iter()
                .all(|group| group.outcome() == Some(&Ok(true)))
        );
    }
    assert_eq!(suspensions() - before, 1, "retry batches both families");
    for family in [&mut first, &mut later] {
        assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
    }
}

#[test]
fn named_publication_reports_failing_site_and_cause_after_shared_resume() {
    let _guard = lock();
    for shared in [false, true] {
        let code = TestCode::new();
        let (_, first) = hook_at(&code, 0, original as *const () as usize);
        let (failed_site, second) = hook_at(&code, 64, original as *const () as usize);
        let mut hooks = vec![first, second];
        unsafe { ptr::write((failed_site + 15) as *mut u8, 0xcc) };
        let mut round = shared.then(Round::collect_attach).flatten();
        if let Some(round) = &mut round {
            assert_eq!(
                Hook::set_family_enabled_shared_named(
                    &mut hooks,
                    true,
                    "occupation_changes",
                    &["healthy", "failed_writer"]
                ),
                Ok(true)
            );
            round.publish();
        }
        let error = Hook::set_family_enabled_shared_named(
            &mut hooks,
            true,
            "occupation_changes",
            &["healthy", "failed_writer"],
        )
        .unwrap_err();
        assert_eq!(
            error,
            format!(
                "occupation_changes hook sites [failed_writer] state change failed: {BYTE_VERIFICATION_FAILURE}"
            )
        );
        assert!(hooks.iter().all(|hook| !hook.is_enabled()));
        drop(round);
        unsafe { ptr::write((failed_site + 15) as *mut u8, hooks[1].original[15]) };
        assert_eq!(
            Hook::set_family_enabled_shared_named(
                &mut hooks,
                true,
                "occupation_changes",
                &["healthy", "failed_writer"]
            ),
            Ok(true)
        );
        assert!(hooks.iter().all(|hook| !hook.diagnostic_failure));
        assert_eq!(
            Hook::set_family_enabled_shared_named(
                &mut hooks,
                false,
                "occupation_changes",
                &["healthy", "failed_writer"]
            ),
            Ok(true)
        );
    }
}

#[test]
fn named_publication_reports_preparation_site_without_reading_refused_memory() {
    let _guard = lock();
    let code = TestCode::new();
    let (_, first) = hook_at(&code, 0, original as *const () as usize);
    let (site, mut second) = hook_at(&code, 64, original as *const () as usize);
    second.site = 0;
    let mut hooks = vec![first, second];
    let mut round = Round::collect_attach().unwrap();
    assert_eq!(
        Hook::set_family_enabled_shared_named(
            &mut hooks,
            true,
            "naval_tree_changes",
            &["healthy", "unwritable"]
        ),
        Ok(true)
    );
    round.publish();
    let error = Hook::set_family_enabled_shared_named(
        &mut hooks,
        true,
        "naval_tree_changes",
        &["healthy", "unwritable"],
    )
    .unwrap_err();
    assert!(error.starts_with("naval_tree_changes hook sites [unwritable] state change failed: "));
    assert!(!error.ends_with("failed: "));
    hooks[1].site = site;
}
