//! `reader_gate_tests`, extracted verbatim from `attach_log.rs`.

        use super::*;

        #[test]
        fn reader_gate_zero_is_unbounded() {
            // 0 permits configured -> no gate installed -> spawn_gated is a plain spawn.
            assert!(resolve_reader_gate(0).is_none());
            assert!(resolve_reader_gate(1).is_some());
            assert!(resolve_reader_gate(8).is_some());
        }

        #[test]
        fn reader_gate_never_exceeds_permit_count() {
            use std::sync::atomic::{AtomicUsize, Ordering};
            // With N permits and many contending workers, the number of workers
            // holding a permit simultaneously must never exceed N.
            for permits in [1usize, 2, 3, 6] {
                let gate = ReaderGate::new(permits);
                let live = AtomicUsize::new(0);
                let peak = AtomicUsize::new(0);
                thread::scope(|scope| {
                    for _ in 0..48 {
                        scope.spawn(|| {
                            let _p = gate.acquire();
                            let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                            peak.fetch_max(now, Ordering::SeqCst);
                            // brief contended window
                            for _ in 0..2000 {
                                std::hint::spin_loop();
                            }
                            live.fetch_sub(1, Ordering::SeqCst);
                        });
                    }
                });
                assert_eq!(live.load(Ordering::SeqCst), 0, "all permits released");
                assert!(
                    peak.load(Ordering::SeqCst) <= permits,
                    "peak {} exceeded permits {}",
                    peak.load(Ordering::SeqCst),
                    permits
                );
                assert_eq!(gate.available(), permits, "gate restored to full");
            }
        }

        #[test]
        fn reader_gate_permit_released_on_panic() {
            // A panicking worker must not leak its permit (RAII Drop fires while
            // unwinding), else a later frame would deadlock at reduced capacity.
            let gate = ReaderGate::new(1);
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _p = gate.acquire();
                assert_eq!(gate.available(), 0);
                panic!("boom");
            }));
            assert!(panicked.is_err());
            assert_eq!(gate.available(), 1, "permit returned after panic unwind");
            // The gate is still usable.
            {
                let _p = gate.acquire();
                assert_eq!(gate.available(), 0);
            }
            assert_eq!(gate.available(), 1);
        }

        #[test]
        fn spawn_gated_without_gate_runs_all_workers() {
            // None gate: every worker runs and returns its value; each ungated
            // worker gets its own lane in the span report.
            let lanes = FrameLaneTracker::new(Instant::now());
            let out: Vec<usize> = thread::scope(|scope| {
                let handles: Vec<_> = (0..8)
                    .map(|i| spawn_gated(scope, None, &lanes, 300 + i as u16, move || i * 2))
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            assert_eq!(out, vec![0, 2, 4, 6, 8, 10, 12, 14]);
            let spans = lanes.take_spans();
            assert_eq!(spans.len(), 8);
            let mut seen_lanes: Vec<u16> = spans.iter().map(|s| s.lane).collect();
            seen_lanes.sort_unstable();
            seen_lanes.dedup();
            assert_eq!(seen_lanes.len(), 8, "unbounded workers get distinct lanes");
        }

        #[test]
        fn spawn_gated_with_gate_returns_same_results() {
            // Gated: same results as ungated; the gate only affects scheduling.
            let gate = ReaderGate::new(2);
            let lanes = FrameLaneTracker::new(Instant::now());
            let out: Vec<usize> = thread::scope(|scope| {
                let gref = Some(&gate);
                let handles: Vec<_> = (0..8)
                    .map(|i| spawn_gated(scope, gref, &lanes, 300 + i as u16, move || i * i))
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            assert_eq!(out, vec![0, 1, 4, 9, 16, 25, 36, 49]);
            assert_eq!(gate.available(), 2);
            let spans = lanes.take_spans();
            assert_eq!(spans.len(), 8);
            assert!(
                spans.iter().all(|s| s.lane < 2),
                "lane ids are permit slots"
            );
        }
