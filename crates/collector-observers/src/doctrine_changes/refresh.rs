use super::{Domain, WatchRegistry};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const REFRESH_DOMAINS: [Domain; 9] = [
    Domain::Status,
    Domain::StatusFolders,
    Domain::Folder,
    Domain::Track,
    Domain::Bank,
    Domain::Progress,
    Domain::Gains,
    Domain::Equipment,
    Domain::Costs,
];
static MARKS: [WatchRegistry; REFRESH_DOMAINS.len()] =
    [const { WatchRegistry::marks_only() }; REFRESH_DOMAINS.len()];
static LOST: AtomicBool = AtomicBool::new(false);
static UNKNOWN: AtomicU64 = AtomicU64::new(0);

fn registry(domain: Domain) -> Option<&'static WatchRegistry> {
    let index = match domain {
        Domain::Status => 0,
        Domain::StatusFolders => 1,
        Domain::Folder => 2,
        Domain::Track => 3,
        Domain::Bank => 4,
        Domain::Progress => 5,
        Domain::Gains => 6,
        Domain::Equipment => 7,
        Domain::Costs => 8,
        _ => return None,
    };
    Some(&MARKS[index])
}

pub fn mark(domain: Domain, pointer: usize) -> bool {
    let Some(marks) = registry(domain) else {
        return false;
    };
    let notification = marks.notify(pointer);
    if !notification.has_mark() {
        UNKNOWN.fetch_add(1, Ordering::Relaxed);
        LOST.store(true, Ordering::Release);
    }
    true
}

pub fn prepare(
    domain: Domain,
    count: usize,
    pointers: impl Iterator<Item = usize>,
) -> Result<(), &'static str> {
    let marks = registry(domain).expect("coalesced doctrine domain");
    marks.collect();
    marks.offer(count);
    marks.boundary(true);
    for pointer in pointers {
        if marks.register(pointer).is_none() {
            LOST.store(true, Ordering::Release);
            return Err("doctrine refresh receiver registration unavailable");
        }
    }
    marks
        .take(|_| {})
        .ok_or("doctrine refresh preparation overlapped a writer")?;
    Ok(())
}

pub fn take(domain: Domain, visit: impl FnMut(usize)) -> Option<u64> {
    registry(domain)?.take(visit)
}

pub fn take_lost() -> bool {
    LOST.swap(false, Ordering::AcqRel)
}

pub(super) fn enabled(enabled: bool) {
    for marks in &MARKS {
        marks.set_enabled(enabled);
    }
}

pub(super) fn invalidate() {
    for marks in &MARKS {
        marks.boundary(true);
    }
}

/// # Safety
/// The enclosing doctrine observer and acquisition worker are quiescent.
pub(super) unsafe fn release() {
    for marks in &MARKS {
        unsafe { marks.release() };
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn prepared_marks_coalesce_without_hook_allocation_and_keep_writer_tokens() {
        let _guard = LOCK.lock().unwrap();
        super::super::enabled(true);
        let _ = take_lost();
        let token = super::super::token(Domain::Track);
        prepare(Domain::Track, 2, [7, 9].into_iter()).unwrap();
        assert_eq!(super::super::token(Domain::Track), token);
        collector_test_support::with_allocations_denied(|| {
            for _ in 0..16 {
                assert!(mark(Domain::Track, 7));
                assert!(mark(Domain::Track, 9));
            }
        });
        assert_eq!(collector_test_support::ATTEMPTS.load(Ordering::Relaxed), 0);
        let mut pointers = Vec::new();
        take(Domain::Track, |pointer| pointers.push(pointer)).unwrap();
        pointers.sort_unstable();
        assert_eq!(pointers, [7, 9]);
        assert!(!take_lost());
        take(Domain::Track, |_| {
            panic!("quiet doctrine receiver marked twice")
        })
        .unwrap();
        super::super::enabled(false);
    }

    #[test]
    fn unknown_or_unprepared_receivers_lose_observation_without_registration() {
        let _guard = LOCK.lock().unwrap();
        super::super::enabled(true);
        let _ = take_lost();
        prepare(Domain::Bank, 1, [5].into_iter()).unwrap();
        collector_test_support::with_allocations_denied(|| {
            assert!(mark(Domain::Bank, 6));
            assert!(mark(Domain::Bank, 0));
        });
        assert!(take_lost());
        assert_eq!(registry(Domain::Bank).unwrap().registered(), 1);
        take(Domain::Bank, |_| {
            panic!("unknown doctrine receiver became registered")
        })
        .unwrap();
        super::super::enabled(false);
    }

    #[test]
    fn exhausted_prepared_slots_lose_observation_instead_of_dropping_a_receiver() {
        let _guard = LOCK.lock().unwrap();
        super::super::enabled(true);
        let _ = take_lost();
        assert_eq!(
            prepare(Domain::Progress, 0, 1usize..),
            Err("doctrine refresh receiver registration unavailable")
        );
        assert!(take_lost());
        super::super::enabled(false);
    }
}
