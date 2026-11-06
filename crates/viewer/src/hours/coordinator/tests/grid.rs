use super::*;

fn on_grid(base: &Want, hour: Hour, sequence: u64, stride: u32, anchor: Option<Hour>) -> Want {
    Want {
        hour,
        grid: Some(Grid {
            cadence: Cadence {
                sequence,
                stride,
                count: 2,
                anchor,
                lead: 0,
            },
            installed: None,
            usable: Vec::new(),
        }),
        ..base.clone()
    }
}

fn with_feedback(mut want: Want, installed: Option<Hour>, usable: &[Hour]) -> Want {
    let grid = want.grid.as_mut().unwrap();
    (grid.installed, grid.usable) = (installed, usable.to_vec());
    want
}

fn post(f: &mut RealDriver, epoch: u64, want: Want) {
    *f.driver.core.requested.lock().unwrap() = Some((epoch, want, None));
    assert!(f.driver.event(Event::Want));
}

fn driver() -> (RealDriver, Want) {
    let f = RealDriver::new(Arc::new(Pool::new(1).unwrap()));
    let base = f.driver.core.want.clone().unwrap();
    (f, base)
}

fn reading(f: &RealDriver) -> Option<Key> {
    f.driver.reading.as_ref().map(|reading| reading.key)
}

fn occupy_output(f: &mut RealDriver) {
    f.driver
        .core
        .outbox
        .try_send(gap_built(99, "occupy output"))
        .ok()
        .unwrap();
}

fn free_output(f: &mut RealDriver) {
    assert_eq!(f.take_output().hour, 99);
    f.driver.core.result_space.store(true, Ordering::Release);
    assert!(f.driver.event(Event::ResultSpace));
}

#[test]
fn the_grid_driver_builds_an_anchor_once_and_keeps_it_sent_until_the_window_receives_it() {
    let (mut f, base) = driver();
    post(&mut f, 0, on_grid(&base, 107, 1, 20, Some(101)));
    f.plan_and_request();
    assert_eq!(reading(&f), Some((1, 101)));
    f.reply(fixed_gap(101, "unrecorded"));
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert_eq!(f.driver.core.sent.get(&(1, 101)), Some(&0));
    assert_eq!(f.driver.decode.as_ref().map(|plan| plan.hour), Some(120));
    post(&mut f, 0, on_grid(&base, 113, 1, 20, Some(101)));
    assert_eq!(f.driver.core.sent.get(&(1, 101)), Some(&0));
    let wanted = f
        .driver
        .core
        .wanted_hours(f.driver.core.want.as_ref().unwrap(), (100, 820));
    assert_eq!(wanted, [101, 120, 140]);
    assert_eq!(f.driver.next_hour(), Some(140));
    let built = f.take_output();
    assert_eq!((built.epoch, built.hour), (0, 101));
    let received = Want {
        received: 1,
        ..on_grid(&base, 115, 1, 20, Some(101))
    };
    post(&mut f, 0, received.clone());
    assert!(!f.driver.core.sent.contains_key(&(1, 101)));
    assert_eq!(f.driver.next_hour(), Some(140));
    post(
        &mut f,
        0,
        Want {
            hour: 119,
            ..received
        },
    );
    assert_eq!(f.driver.next_hour(), Some(140));
}

#[test]
fn an_old_epochs_anchor_flight_is_never_sent_and_never_acknowledges_a_new_seeks_anchor() {
    let (mut f, base) = driver();
    occupy_output(&mut f);
    post(&mut f, 0, on_grid(&base, 107, 1, 20, Some(101)));
    f.plan_and_request();
    f.reply(fixed_gap(101, "unrecorded"));
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert!(f.driver.output_blocked);
    assert!(!f.driver.core.sent.contains_key(&(1, 101)));
    post(&mut f, 1, on_grid(&base, 101, 2, 20, Some(101)));
    assert!(
        !f.driver.ledger.contains(0, &(1, 101)),
        "the old flight is gone"
    );
    assert_eq!(f.driver.decode.as_ref().map(|plan| plan.hour), None);
    free_output(&mut f);
    f.plan_and_request();
    assert_eq!(reading(&f), Some((1, 101)));
    f.reply(fixed_gap(101, "unrecorded"));
    assert!(matches!(f.driver.turn(), Turn::Progress));
    let sent: Vec<_> = f
        .results
        .try_iter()
        .map(output_hour)
        .map(|built| (built.epoch, built.hour))
        .collect();
    assert_eq!(sent, [(1, 101)], "only the new epoch's anchor is sent");
    let anchor = f.driver.core.anchor.unwrap();
    assert_eq!(
        (anchor.sequence, anchor.epoch, anchor.key, anchor.sent),
        (2, 1, (1, 101), Some(0))
    );
    assert!(!anchor.done);
    assert_ne!(f.driver.next_hour(), Some(101));
}

#[test]
fn an_accepted_target_newer_than_the_hour_installed_drains_to_the_window() {
    for installed in [None, Some(125)] {
        let (mut f, base) = driver();
        post(&mut f, 0, on_grid(&base, 115, 1, 20, None));
        f.plan_and_request();
        assert_eq!(reading(&f), Some((1, 120)));
        f.reply(super::super::super::tests::sized_frame(120, 1));
        f.admit_parts();
        let later = with_feedback(on_grid(&base, 135, 1, 20, None), installed, &[130]);
        post(&mut f, 0, later);
        f.finish_parts();
        assert!(matches!(f.driver.turn(), Turn::Progress));
        let sent: Vec<_> = f
            .results
            .try_iter()
            .map(output_hour)
            .map(|built| built.hour)
            .collect();
        let expected = if installed.is_none() {
            vec![120]
        } else {
            vec![]
        };
        assert_eq!(sent, expected, "installed {installed:?}");
        assert!(!f.driver.ledger.contains(0, &(1, 120)));
    }
}

#[test]
fn a_ready_result_nothing_accepted_waits_out_a_full_output_and_is_delivered() {
    for installed in [None, Some(125)] {
        let (mut f, base) = driver();
        occupy_output(&mut f);
        post(&mut f, 0, on_grid(&base, 115, 1, 20, None));
        f.plan_and_request();
        f.reply(fixed_gap(120, "unrecorded"));
        assert!(matches!(f.driver.turn(), Turn::Progress));
        assert!(f.driver.output_blocked);
        let later = with_feedback(on_grid(&base, 135, 1, 20, None), installed, &[130]);
        post(&mut f, 0, later);
        free_output(&mut f);
        f.driver.turn();
        let sent: Vec<_> = f
            .results
            .try_iter()
            .map(output_hour)
            .map(|built| built.hour)
            .collect();
        let expected = if installed.is_none() {
            vec![120]
        } else {
            vec![]
        };
        assert_eq!(sent, expected, "installed {installed:?}");
        assert!(!f.driver.ledger.contains(0, &(1, 120)));
    }
}

#[test]
fn an_immediate_off_grid_capture_is_requested_and_delivered_after_the_anchor_completed() {
    let warmup_wait = crate::app::exact_capture_wait(Some(1), true);
    assert!(!warmup_wait);
    let stride =
        crate::app::grid_activation(true, true, false, false, true, warmup_wait, true, 720.0)
            .unwrap();
    assert_eq!(stride, 20);
    let (mut f, base) = driver();
    post(&mut f, 0, on_grid(&base, 107, 1, stride, Some(101)));
    f.plan_and_request();
    assert_eq!(reading(&f), Some((1, 101)));
    f.reply(fixed_gap(101, "unrecorded"));
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert_eq!(f.take_output().hour, 101);

    let active = Want {
        received: 1,
        have: vec![101],
        ..with_feedback(on_grid(&base, 137, 1, stride, Some(101)), Some(101), &[])
    };
    post(&mut f, 0, active);
    assert!(f.driver.core.anchor.unwrap().done);
    assert_eq!(
        f.driver
            .core
            .wanted_hours(f.driver.core.want.as_ref().unwrap(), (100, 820)),
        [140, 160],
    );

    let wait = crate::app::exact_capture_wait(Some(0), true);
    assert!(wait);
    let activation = crate::app::grid_activation(true, true, false, false, true, wait, true, 720.0);
    assert_eq!(activation, Err("capture"));
    let exact = Want {
        hour: 137,
        ahead: 1,
        received: 1,
        have: vec![101],
        grid: activation.ok().map(|stride| Grid {
            cadence: Cadence {
                sequence: 1,
                stride,
                count: 2,
                anchor: Some(101),
                lead: 0,
            },
            installed: Some(101),
            usable: Vec::new(),
        }),
        ..base
    };
    post(&mut f, 0, exact);
    assert_eq!(
        f.driver
            .core
            .wanted_hours(f.driver.core.want.as_ref().unwrap(), (100, 820)),
        [137, 138],
    );

    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert_eq!(reading(&f), Some((1, 120)));
    f.reply(fixed_gap(120, "unrecorded"));
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert_eq!(f.take_output().hour, 120);
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert_eq!(reading(&f), Some((1, 137)));
    let expected = super::super::super::tests::sized_frame(137, 1);
    f.reply(expected.clone());
    f.admit_parts();
    f.finish_parts();
    assert!(matches!(f.driver.turn(), Turn::Progress));
    let captured = f
        .results
        .try_recv()
        .map(output_hour)
        .expect("the exact capture cannot wait on another Want");
    assert_eq!(
        (captured.epoch, captured.branch, captured.hour),
        (0, 1, 137)
    );
    assert_eq!(captured.current, expected);
    assert!(!f.driver.ledger.contains(0, &(1, 137)));
}
