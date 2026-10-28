use super::*;

#[test]
fn ordered_vision_pressure_retains_the_ticket_until_release_and_cancels_without_a_gap() {
    for cancel in [false, true] {
        let owner = engine_allocation::credit::MemoryBudget::new(1 << 20);
        let mut f = RealDriver::over(
            Arc::new(Pool::new(1).unwrap()),
            Arc::new(crate::playback::tests::fixture()),
            SourceMailbox::new(|_| true),
            &owner,
        );
        f.driver.core.vision_pressure = Some(owner.clone());
        let held = owner.admit(owner.limit() - owner.used()).unwrap();
        f.plan_and_request();
        let ticket = f.driver.decode.as_ref().unwrap().ticket;
        let current = super::super::super::tests::sized_frame(100, 1);
        let Current::Frame(frame) = &current else {
            panic!("frame")
        };
        let weak = Arc::downgrade(frame);
        f.reply(current);
        assert!(f.driver.vision_retry.is_some());
        assert!(f.driver.targets.is_empty());
        assert_eq!(f.driver.core.vision_calls, 1);
        for _ in 0..3 {
            let _ = f.driver.turn();
        }
        assert_eq!(
            f.driver.core.vision_calls, 1,
            "own cleanup never spins the stage"
        );
        assert!(f.driver.ledger.next_admission().is_none());
        let error = exact_refusal(&mut f, 0, 100);
        assert!(error.to_string().contains("ordered fog vision"));
        if cancel {
            f.want(1, 200, false);
            assert!(f.driver.vision_retry.is_none());
            assert!(weak.upgrade().is_none());
            assert!(f.driver.ledger.get(ticket).is_none());
            drop(held);
        } else {
            release(&mut f, held);
            assert!(matches!(f.driver.turn(), Turn::Progress));
            assert_eq!(f.driver.core.vision_calls, 2);
            assert!(f.driver.vision_retry.is_none());
            assert!(matches!(
                f.driver.targets[&ticket],
                PreparedTarget::Parts(..)
            ));
            assert_eq!(f.driver.ledger.next_admission(), Some((ticket, 0)));
            assert!(f.driver.core.refusal.for_target(0, (1, 100)).is_ok());
        }
    }
}

fn exact_refusal(f: &mut RealDriver, epoch: u64, target: Hour) -> io::Error {
    let (_, placeholder) = mpsc::channel();
    let results = std::mem::replace(&mut f.results, placeholder);
    let completed = Completions::new(
        results,
        f.driver.core.refusal.clone(),
        f.driver.core.result_space.clone(),
        f.driver.core.events.clone(),
    );
    let rescue = f.driver.core.outbox.clone();
    let (send, done) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let result = completed.wait(epoch, (1, target)).map(|built| built.hour);
        let _ = send.send((result, completed.received.get(), completed.results));
    });
    let reply = done.recv_timeout(crate::map::test_support::TEST_HANG_GUARD);
    if reply.is_err() {
        let _ = rescue.try_send(Output::Hour(gap_built(target, "hang-guard rescue")));
    }
    waiter.join().unwrap();
    let (result, received, results) = reply.expect("credit retry must not strand an exact wait");
    f.results = results;
    assert_eq!(received, 0, "pressure is never delivered as an hour");
    result.err().expect("named pressure, not a completed gap")
}

fn listen(f: &mut RealDriver, owner: &engine_allocation::credit::MemoryBudget) {
    let events = f.driver.core.events.clone();
    f.driver.product_space = Some(
        owner
            .listen(move || {
                let _ = events.send(Event::ProductSpace);
            })
            .unwrap(),
    );
}

fn release(f: &mut RealDriver, held: engine_allocation::credit::Credit) {
    f.driver.product_space.as_ref().unwrap().acknowledge();
    drop(held);
    loop {
        let event = f
            .inbox
            .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
            .unwrap();
        let released = matches!(event, Event::ProductSpace);
        assert!(f.driver.event(event));
        if released {
            break;
        }
    }
}

#[test]
fn exact_wait_sees_repeated_colour_credit_pressure_for_prepared_and_native_sources() {
    for prepared in [false, true] {
        for shared in [false, true] {
            let mut f = PressureFixture::span(prepared, shared, 100, 103, 102);
            let ticket = f.pump.driver.decode.as_ref().unwrap().ticket;
            let initial = f
                .pump
                .driver
                .decode
                .as_ref()
                .unwrap()
                .colour
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .clone();
            assert!(matches!(f.pump.driver.turn(), Turn::Progress));
            f.source_reply();
            let current = super::super::super::tests::source_frame(&f.source.client(), 1, 101);
            assert!(matches!(&current, Current::Frame(frame) if frame.hour == 101));
            let owner = engine_allocation::credit::MemoryBudget::new(8192);
            listen(&mut f.pump, &owner);
            let held = owner.admit(owner.limit() - owner.used()).unwrap();
            let style_owner = owner.clone();
            let calls = Arc::new(AtomicU64::new(0));
            let attempts = calls.clone();
            f.pump.driver.history.as_mut().unwrap().work = Some(dispatch_plan::colour_job(
                ticket,
                (1, 101),
                current,
                f.pump.driver.core.events.clone(),
                None,
                Some(Ok(initial)),
                move |frame, previous| {
                    attempts.fetch_add(1, Ordering::Relaxed);
                    let _style = style_owner.admit(1).map_err(|_| {
                        io::Error::new(io::ErrorKind::WouldBlock, "style bytes remain pinned")
                    })?;
                    kept_border_fill(frame, previous)
                },
            ));
            assert!(matches!(f.pump.driver.turn(), Turn::Progress));
            while f.pump.driver.history.as_ref().unwrap().pending.is_none() {
                f.pump.next_event();
            }
            f.pump.want(0, 103, true);
            assert!(!f.pump.driver.ledger.can_plan());
            for expected_calls in [1, 2] {
                assert_eq!(calls.load(Ordering::Relaxed), expected_calls);
                assert_eq!(f.pump.driver.decode.as_ref().unwrap().next_source, 101);
                assert!(f.pump.driver.core.refusal.for_target(0, (1, 101)).is_ok());
                assert!(f.pump.driver.core.refusal.for_target(0, (1, 103)).is_ok());
                assert_eq!(
                    f.pump
                        .driver
                        .core
                        .refusal
                        .for_target(0, (1, 102))
                        .unwrap_err()
                        .to_string(),
                    "display colour fold at branch 1 hour 101 awaits credit: style bytes remain pinned"
                );
                f.pump.driver.pool_blocked = true;
                f.pump.driver.turn();
                let error = exact_refusal(&mut f.pump, 0, 103);
                assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                assert_eq!(
                    error.to_string(),
                    "branch 1 hour 102 blocks exact branch 1 hour 103: display colour fold at branch 1 hour 101 awaits credit: style bytes remain pinned"
                );
                assert!(f.pump.driver.core.sent.is_empty());
                f.pump.driver.pool_blocked = false;
                if expected_calls == 1 {
                    f.pump.driver.history.as_mut().unwrap().pending =
                        Some((owner.generation().wrapping_sub(1), Instant::now()));
                    assert!(matches!(f.pump.driver.turn(), Turn::Progress));
                    assert!(
                        f.pump.driver.core.refusal.for_target(0, (1, 102)).is_err(),
                        "submission is not recovery"
                    );
                    while f.pump.driver.history.as_ref().unwrap().pending.is_none() {
                        f.pump.next_event();
                    }
                }
            }
            f.pump.driver.history.as_mut().unwrap().pending = Some((
                owner.generation(),
                Instant::now() + Duration::from_secs(86400),
            ));
            release(&mut f.pump, held);
            assert!(matches!(f.pump.driver.turn(), Turn::Progress));
            while f.pump.driver.history.is_some() {
                f.pump.next_event();
            }
            assert_eq!(calls.load(Ordering::Relaxed), 3);
            assert!(f.pump.driver.core.refusal.is_empty());
            let plan = f.pump.driver.decode.as_ref().unwrap();
            assert_eq!(
                (plan.ticket, plan.hour, plan.next_source),
                (ticket, 102, 102)
            );
            let colour = plan
                .colour
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap();
            assert_eq!(colour.mode, "border:100:101");
            assert_eq!(
                colour.layers[0].borders.values().next().unwrap().provinces,
                [1, 2]
            );
            assert!(f.pump.driver.core.sent.is_empty());
            f.close();
        }
    }
}

fn retry_part(
    job: HeldJob,
    ticket: u64,
    owner: engine_allocation::credit::MemoryBudget,
    events: Sender<Event>,
) -> HeldJob {
    let name = job.name;
    HeldJob {
        name,
        work: Box::new(move || match owner.admit(1) {
            Err(_) => {
                events
                    .send(Event::RetryPart {
                        ticket,
                        generation: owner.generation(),
                        reason: format!("{name} bytes remain pinned"),
                        job: retry_part(job, ticket, owner, events.clone()),
                    })
                    .unwrap();
            }
            Ok(credit) => {
                drop(credit);
                (job.work)();
            }
        }),
    }
}

#[test]
fn exact_wait_sees_each_part_retry_and_retired_pressure_for_prepared_and_native_sources() {
    for prepared in [false, true] {
        for shared in [false, true] {
            for retire in [false, true] {
                let mut f = PressureFixture::span(prepared, shared, 100, 103, 102);
                f.fold(101);
                let ticket = f.pump.driver.decode.as_ref().unwrap().ticket;
                assert!(matches!(f.pump.driver.turn(), Turn::Progress));
                f.source_reply();
                let owner = engine_allocation::credit::MemoryBudget::new(8192);
                listen(&mut f.pump, &owner);
                let held = owner.admit(owner.limit() - owner.used()).unwrap();
                let events = f.pump.driver.core.events.clone();
                let Some(PreparedTarget::Parts(_, jobs)) = f.pump.driver.targets.get_mut(&ticket)
                else {
                    panic!("endpoint parts");
                };
                for rank in [1, 6] {
                    let job = jobs.remove(rank).unwrap();
                    let work = retry_part(job, ticket, owner.clone(), events.clone());
                    jobs.insert(rank, work);
                }
                f.pump.admit_parts();
                while f.pump.driver.retries.len() != 2
                    || match f.pump.driver.targets.get(&ticket).unwrap() {
                        PreparedTarget::Parts(pending, _) => pending.received != 6,
                        _ => panic!("credit is not completion"),
                    }
                {
                    f.pump.next_event();
                }
                assert_eq!(f.pump.driver.core.refusal.current.lock().unwrap().len(), 2);
                let epoch = u64::from(retire);
                f.pump.want(epoch, 103, true);
                f.pump.driver.pool_blocked = true;
                f.pump.driver.turn();
                let error = exact_refusal(&mut f.pump, epoch, 103);
                assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                assert_eq!(
                    error.to_string(),
                    if retire {
                        "branch 1 hour 102 epoch 0 blocks exact branch 1 hour 103 epoch 1: display names awaits credit: names bytes remain pinned"
                    } else {
                        "branch 1 hour 102 blocks exact branch 1 hour 103: display names awaits credit: names bytes remain pinned"
                    }
                );
                assert!(
                    f.pump
                        .driver
                        .core
                        .refusal
                        .for_target(epoch, (1, 103))
                        .is_ok()
                );
                assert!(f.pump.driver.core.sent.is_empty());
                for retry in f.pump.driver.retries.values_mut() {
                    retry.0 = owner.generation();
                    retry.1 = Instant::now() + Duration::from_secs(86400);
                }
                release(&mut f.pump, held);
                f.pump.driver.pool_blocked = false;
                assert!(matches!(f.pump.driver.turn(), Turn::Progress));
                loop {
                    let event = f
                        .pump
                        .inbox
                        .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
                        .unwrap();
                    let names = matches!(&event, Event::Part { name: "names", .. });
                    assert!(f.pump.driver.event(event));
                    if names {
                        break;
                    }
                }
                assert_eq!(
                    f.pump
                        .driver
                        .core
                        .refusal
                        .for_target(0, (1, 102))
                        .unwrap_err()
                        .to_string(),
                    "display fog awaits credit: fog bytes remain pinned"
                );
                assert!(matches!(f.pump.driver.turn(), Turn::Progress));
                while !f.pump.driver.core.refusal.is_empty() {
                    f.pump.next_event();
                }
                assert!(f.pump.driver.retries.is_empty());
                f.pump.driver.turn();
                if retire {
                    assert!(f.pump.driver.ledger.get(ticket).is_none());
                    assert!(f.pump.results.try_recv().is_err());
                } else {
                    let built = f.pump.take_output();
                    assert_eq!(built.hour, 102);
                    assert!(matches!(built.current, Current::Frame(frame) if frame.hour == 102));
                }
                f.close();
            }
        }
    }
}

#[test]
fn cancelling_a_parked_colour_fold_clears_only_its_ticket() {
    let mut f = RealDriver::new(Arc::new(Pool::new(1).unwrap()));
    f.want(0, 102, true);
    assert!(matches!(f.driver.turn(), Turn::Progress));
    f.driver.decode.as_mut().unwrap().next_source = 101;
    assert!(matches!(f.driver.turn(), Turn::Progress));
    f.reply(super::super::super::tests::sized_frame(101, 1));
    let history = f.driver.history.as_mut().unwrap();
    let ticket = history.ticket;
    let work = history.work.take().unwrap();
    assert!(f.driver.event(Event::RetryColour {
        ticket,
        key: (1, 101),
        generation: 0,
        reason: "cancelled style credit".into(),
        work
    }));
    f.driver.core.refusal.set_stage(
        ticket + 1,
        PressureStage::Part(1),
        1,
        (1, 103),
        "independent names credit".into(),
    );
    f.want(1, 103, false);
    f.driver.turn();
    assert!(f.driver.history.is_none() && f.driver.ledger.get(ticket).is_none());
    assert!(f.driver.core.refusal.for_target(0, (1, 102)).is_ok());
    assert_eq!(
        f.driver
            .core
            .refusal
            .for_target(1, (1, 103))
            .unwrap_err()
            .to_string(),
        "independent names credit"
    );
}
