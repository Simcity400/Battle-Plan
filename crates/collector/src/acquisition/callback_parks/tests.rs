use super::*;

#[test]
fn callback_contention_is_counted_without_worker_fast_path_or_poison_noise() {
    let mutex = Mutex::new(0);
    let held = mutex.lock().unwrap();
    std::thread::scope(|threads| {
        threads.spawn(|| {
            let _callback = Scope::enter();
            *lock_callback(&mutex, Site::Inputs) = 7;
        });
        let until = Instant::now() + crate::TEST_HANG_GUARD;
        while snapshot()[Site::Inputs as usize].calls == 0 {
            assert!(Instant::now() < until, "callback did not attempt its lock");
            std::thread::yield_now();
        }
        drop(held);
    });
    assert_eq!(*mutex.lock().unwrap(), 7);
    let blocked = &snapshot()[Site::Inputs as usize];
    assert_eq!(blocked.calls, 1);
    assert_eq!(blocked.ns, blocked.max_ns);

    // Other threads and uncontended callback locks add no blocking attempt.
    drop(Park::new(Site::Inputs));
    drop(lock_callback(&mutex, Site::Inputs));
    {
        let _callback = Scope::enter();
        drop(lock_callback(&mutex, Site::Inputs));
        let _nested = Scope::enter();
        assert!(ACTIVE.get());
    }
    assert!(!ACTIVE.get());
    assert_eq!(snapshot()[Site::Inputs as usize].calls, 1);

    let _ = std::panic::catch_unwind(|| {
        let _guard = mutex.lock().unwrap();
        panic!("fixture poison");
    });
    let result = std::panic::catch_unwind(|| {
        let _callback = Scope::enter();
        assert_eq!(*lock_callback(&mutex, Site::Inputs), 7);
        let _park = Park::new(Site::FinalInputs);
        panic!("fixture callback unwind");
    });
    assert!(result.is_err());
    assert!(!ACTIVE.get());
    assert_eq!(snapshot()[Site::Inputs as usize].calls, 1);
    assert_eq!(snapshot()[Site::FinalInputs as usize].calls, 1);
    drop(Park::new(Site::FinalInputs));
    assert_eq!(snapshot()[Site::FinalInputs as usize].calls, 1);
}
