//! Logical-frame transport scripts: source/pool/output releases are explicit,
//! never sleeps or speed assertions. Delivered products have fixed expectations.
use super::*;

type Key = (BranchId, Hour);
const NAMES: [&str; 8] = [
    "political",
    "names",
    "overlay",
    "rails",
    "buildings",
    "movers",
    "fog",
    "snow",
];

#[derive(Clone, Debug, Eq, PartialEq)]
struct Product {
    epoch: u64,
    key: Key,
    bytes: Vec<u8>,
}
fn job(key: Key, rank: u8) -> Vec<u8> {
    // A content-dependent synthetic job with both changing and unchanged rows.
    // Comparing bytes, not a final hash, retains every delivered endpoint.
    let mut bytes = vec![rank];
    bytes.extend_from_slice(&key.0.to_le_bytes());
    bytes.extend_from_slice(&(key.1 / (i32::from(rank) + 1)).to_le_bytes());
    bytes
}
fn expected(epoch: u64, key: Key) -> Product {
    Product {
        epoch,
        key,
        bytes: [
            key.1,
            key.1 / 2,
            key.1 / 3,
            key.1 / 4,
            key.1 / 5,
            key.1 / 6,
            key.1 / 7,
            key.1 / 8,
        ]
        .into_iter()
        .enumerate()
        .flat_map(|(rank, value)| {
            let mut row = vec![rank as u8];
            row.extend(key.0.to_le_bytes());
            row.extend(value.to_le_bytes());
            row
        })
        .collect(),
    }
}
struct Target {
    epoch: u64,
    key: Key,
    parts: BTreeMap<u8, Vec<u8>>,
}
impl Target {
    fn finish(&mut self, rank: u8) {
        assert!(self.parts.insert(rank, job(self.key, rank)).is_none());
    }
    fn product(&self) -> Product {
        assert_eq!(self.parts.len(), 8, "never a partial hour");
        Product {
            epoch: self.epoch,
            key: self.key,
            bytes: self.parts.values().flatten().copied().collect(),
        }
    }
}
struct Pump {
    ledger: Ledger<Key>,
    targets: BTreeMap<u64, Target>,
    queue: VecDeque<(u64, u8)>,
    output: VecDeque<Product>,
    delivered: Vec<Product>,
    width: usize,
}
impl Pump {
    fn new(width: usize) -> Self {
        Self {
            ledger: Ledger::new(width),
            targets: BTreeMap::new(),
            queue: VecDeque::new(),
            output: VecDeque::new(),
            delivered: Vec::new(),
            width,
        }
    }
    fn plan(&mut self, id: u64, epoch: u64, key: Key, predecessors: &[u64]) {
        assert!(
            self.ledger
                .reserve(id, epoch, key, predecessors.iter().copied().collect())
        );
        self.targets.insert(
            id,
            Target {
                epoch,
                key,
                parts: BTreeMap::new(),
            },
        );
        assert!(self.ledger.source_started(id));
    }
    fn decoded(&mut self, id: u64) {
        assert!(self.ledger.source_finished(id));
        assert!(self.ledger.decoded(id, 0));
    }
    fn admit(&mut self, vacancies: usize) {
        for _ in 0..vacancies {
            let Some((id, rank)) = self.ledger.next_admission() else {
                break;
            };
            assert!(self.ledger.accepted(id, rank));
            self.queue.push_back((id, rank));
        }
        assert!(self.ledger.len() <= self.width);
    }
    fn complete(&mut self, id: u64, rank: u8) {
        if self.ledger.completed(id, rank) {
            self.targets.get_mut(&id).unwrap().finish(rank);
        }
    }
    fn poll(&mut self) {
        for id in self.ledger.terminal_retired() {
            self.targets.remove(&id);
            assert!(self.ledger.release(id));
        }
        for id in self.ledger.ready() {
            if self.output.len() == self.width {
                break;
            }
            let product = self.targets.remove(&id).unwrap().product();
            assert_eq!(product, expected(product.epoch, product.key));
            self.output.push_back(product);
            assert!(self.ledger.release(id));
        }
        assert!(self.ledger.len() <= self.width);
    }
    fn consume(&mut self) {
        if let Some(product) = self.output.pop_front() {
            self.delivered.push(product);
        }
    }
    fn retarget(&mut self, epoch: u64, keep: &[Key]) {
        for id in self
            .ledger
            .retarget(|e, key| e == epoch && keep.contains(key))
        {
            self.targets.remove(&id);
        }
    }
}

#[test]
fn scripted_decode_and_pool_waits_deliver_finished_hours_with_fixed_expected_bytes() {
    for block_in_decode in [true, false] {
        let mut new = Pump::new(2);
        for (id, hour) in [(1, 10), (2, 12)] {
            new.plan(id, 0, (1, hour), if id == 2 { &[1] } else { &[] });
            if id == 1 {
                new.decoded(id);
                new.admit(8);
            }
        }
        if !block_in_decode {
            new.decoded(2);
            new.admit(0);
        }
        for rank in (0..8).rev() {
            new.complete(1, rank);
        }
        new.poll();
        assert_eq!(new.output.front(), Some(&expected(0, (1, 10))));
        assert!(new.ledger.get(2).is_some());
        if block_in_decode {
            new.decoded(2);
        }
        new.admit(8);
        for rank in [4, 7, 2, 0, 6, 1, 5, 3] {
            new.complete(2, rank);
        }
        new.poll();
        new.consume();
        new.consume();
        assert_eq!(new.delivered, [expected(0, (1, 10)), expected(0, (1, 12))]);
        for (product, values) in new
            .delivered
            .iter()
            .zip([[10i32, 5, 3, 2, 2, 1, 1, 1], [12i32, 6, 4, 3, 2, 2, 1, 1]])
        {
            assert_eq!(product.bytes.len(), 104);
            for (rank, (row, value)) in product.bytes.chunks_exact(13).zip(values).enumerate() {
                assert_eq!(row[0], rank as u8);
                assert_eq!(&row[1..9], &1u64.to_le_bytes());
                assert_eq!(&row[9..13], &value.to_le_bytes());
            }
        }
        assert_eq!(
            new.queue.iter().copied().collect::<Vec<_>>(),
            (1..=2)
                .flat_map(|id| (0..8).map(move |rank| (id, rank)))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn scripted_output_full_keeps_credits_while_wants_and_accepted_work_still_drain() {
    let mut pump = Pump::new(2);
    pump.output
        .extend([expected(0, (1, 8)), expected(0, (1, 9))]);
    for (id, hour) in [(1, 10), (2, 11)] {
        pump.plan(id, 0, (1, hour), &[]);
        pump.decoded(id);
        pump.admit(8);
        for rank in (0..8).rev() {
            pump.complete(id, rank);
        }
    }
    pump.poll();
    assert_eq!(pump.ledger.len(), 2);
    assert!(!pump.ledger.can_plan());
    pump.retarget(1, &[(2, 100)]); // seek is serviced despite the full output
    pump.poll();
    assert_eq!(
        pump.ledger.len(),
        0,
        "old accepted results retire only at terminal"
    );
    pump.plan(3, 1, (2, 100), &[]);
    pump.decoded(3);
    pump.admit(8);
    for rank in 0..8 {
        pump.complete(3, rank);
    }
    pump.poll();
    assert_eq!(
        pump.ledger.len(),
        1,
        "unsent complete result retains its credit"
    );
    pump.consume();
    pump.poll();
    pump.consume();
    pump.consume();
    assert_eq!(
        pump.delivered,
        [
            expected(0, (1, 8)),
            expected(0, (1, 9)),
            expected(1, (2, 100))
        ]
    );
    assert_eq!(pump.ledger.len(), 0);
}

#[test]
fn scripted_first_acceptance_is_the_cancellation_boundary_and_late_tickets_are_ignored() {
    let mut pump = Pump::new(2);
    pump.plan(1, 0, (1, 10), &[]);
    pump.decoded(1);
    pump.retarget(0, &[]);
    assert!(pump.targets.is_empty());
    pump.plan(2, 0, (1, 10), &[]);
    pump.decoded(2);
    pump.admit(1);
    pump.retarget(1, &[(1, 10)]);
    assert_eq!(pump.ledger.next_admission(), Some((2, 1)));
    pump.admit(7); // accepted queued target retains ALL its held jobs
    pump.plan(3, 1, (1, 10), &[]);
    pump.decoded(3);
    pump.admit(8);
    pump.complete(1, 0); // cancelled target, same recorded endpoint
    for rank in (0..8).rev() {
        pump.complete(2, rank);
        pump.complete(3, rank);
    }
    pump.complete(3, 0); // duplicate cannot mutate/complete replacement
    pump.poll();
    pump.consume();
    assert_eq!(pump.delivered, [expected(1, (1, 10))]);
    assert!(pump.targets.is_empty());
}

#[test]
fn scripted_same_poll_readiness_is_stable_without_head_of_line_blocking() {
    let mut reference = None;
    for order in [[1, 2, 3], [3, 1, 2], [2, 3, 1], [3, 2, 1]] {
        let mut pump = Pump::new(4);
        for id in 1..=4 {
            pump.plan(id, 0, (1, 9 + id as i32), &[]);
            pump.decoded(id);
            pump.admit(8);
        }
        for id in order {
            for rank in (0..8).rev() {
                pump.complete(id, rank);
            }
        }
        pump.poll();
        let products: Vec<_> = pump.output.into_iter().collect();
        assert_eq!(
            products.iter().map(|p| p.key.1).collect::<Vec<_>>(),
            [10, 11, 12]
        );
        if let Some(reference) = &reference {
            assert_eq!(&products, reference);
        } else {
            reference = Some(products);
        }
        assert_eq!(pump.ledger.get(4).unwrap().phase, flow::Phase::Parts);
    }
}

#[test]
fn scripted_refusals_and_child_prefixes_keep_fixed_part_priority() {
    for rotation in 0..8 {
        let mut pending = super::super::tests::pending(77);
        for at in 0..8 {
            let rank = (at + rotation) % 8;
            let name = NAMES[rank];
            let children = vec![BuilderSpan {
                name: "completed.prefix",
                ms: 0.0,
                cpu_ms: Some(0.0),
            }];
            assert_eq!(
                pending.take(77, name, 0.0, children, Part::failed(name)),
                at == 7
            );
        }
        assert_eq!(pending.error_rank, Some(0));
        assert_eq!(
            pending.error.as_deref(),
            Some("political: political: the hour builder panicked")
        );
        let children = ordered_child_laps(std::mem::take(&mut pending.child_laps));
        assert_eq!(children.len(), 8);
        assert!(
            children
                .iter()
                .all(|child| child.name == "completed.prefix" && child.cpu_ms == Some(0.0))
        );
    }
}

// These fixtures replace content computation only. They run the production
// Driver::event/turn, its real bounded Pool, mailboxes, ledger and output.
struct TestBuilder {
    state: CoordinatorState,
    colours: Arc<Mutex<Vec<Result<dispatch_plan::Colour, String>>>>,
}
impl std::ops::Deref for TestBuilder {
    type Target = CoordinatorState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
impl std::ops::DerefMut for TestBuilder {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}
fn colour_fill(
    frame: &replay_frame::frame::DecodedFrame,
    previous: Option<&viewer_world::map::map_mode::ColourState>,
) -> io::Result<dispatch_plan::Colour> {
    let prior = previous.map_or(7, |colour| colour.mode.parse::<u64>().unwrap());
    Ok(Some(Arc::new(viewer_world::map::map_mode::ColourState {
        mode: prior
            .wrapping_mul(37)
            .wrapping_add(frame.hour as u64)
            .to_string(),
        layers: Default::default(),
    })))
}
fn fixed_gap(hour: Hour, reason: &str) -> Current {
    Current::Gap {
        branch: 1,
        hour,
        reason: reason.into(),
    }
}
fn gap_built(hour: Hour, reason: &str) -> Built {
    Built {
        epoch: 0,
        branch: 1,
        hour,
        current: fixed_gap(hour, reason),
        layers: Err(reason.into()),
        laps: Vec::new(),
        child_laps: Vec::new(),
        view: super::super::tests::test_view(),
        wall: 0.0,
        trace: Default::default(),
    }
}
impl BuildTarget for TestBuilder {
    fn colour_work(&self, plan: &mut DispatchPlan, current: Current) -> Box<dyn FnOnce() + Send> {
        plan.colour_work(current, self.events.clone(), colour_fill)
    }
    fn prepare(&mut self, plan: DispatchPlan, current: Current) -> PreparedTarget {
        if let Current::Gap { reason, .. } = &current {
            let mut built = gap_built(plan.hour, reason);
            built.epoch = plan.epoch;
            built.trace = plan.trace;
            return PreparedTarget::Ready(Box::new(built));
        }
        if let Some(colour) = plan.colour {
            let colour =
                colour.and_then(|colour| carry_colour(colour, [current.clone()], colour_fill));
            self.colours
                .lock()
                .unwrap()
                .push(colour.map_err(|e| e.to_string()));
        }
        let mut pending = super::super::tests::pending(plan.ticket);
        pending.current = current;
        pending.trace = plan.trace;
        pending.view = plan.want.view;
        let mut jobs = VecDeque::new();
        for name in NAMES {
            let events = self.events.clone();
            let (ticket, epoch, branch, hour) =
                (plan.ticket, plan.epoch, plan.want.branch, plan.hour);
            jobs.push_back(HeldJob {
                name,
                work: Box::new(move || {
                    events
                        .send(Event::Part {
                            ticket,
                            epoch,
                            branch,
                            hour,
                            name,
                            timing: 0.0.into(),
                            children: vec![BuilderSpan {
                                name,
                                ms: 0.0,
                                cpu_ms: Some(0.0),
                            }],
                            part: Box::new(Part::failed(name)),
                        })
                        .unwrap();
                }),
            });
        }
        PreparedTarget::Parts(Box::new(pending), jobs)
    }
}
struct RealDriver {
    driver: Driver<TestBuilder>,
    inbox: Receiver<Event>,
    results: Receiver<Built>,
}
impl RealDriver {
    fn new(pool: Arc<Pool>) -> Self {
        let replay = Arc::new(crate::playback::tests::fixture());
        let (events, inbox) = mpsc::channel();
        let (outbox, results) = mpsc::sync_channel(1);
        let core = TestBuilder {
            colours: Default::default(),
            state: CoordinatorState {
                device: Default::default(),
                playback: Playback::unloaded(replay).unwrap(),
                shared_decoder: None,
                lane: None,
                requested: Default::default(),
                profile: Default::default(),
                profile_want: None,
                profile_received: None,
                engaged: Default::default(),
                want: None,
                epoch: 0,
                sent: BTreeMap::new(),
                sends: 0,
                slots: BTreeMap::new(),
                memos: Arc::new(Memos::new()),
                reusable: BTreeMap::new(),
                pool,
                parallel: 2,
                events,
                outbox,
                result_space: Default::default(),
            },
        };
        let source = SourceMailbox::new(|_| true);
        let mut fixture = Self {
            driver: Driver::new(core, source),
            inbox,
            results,
        };
        fixture.want(0, 100, false);
        fixture
    }
    fn want(&mut self, epoch: u64, hour: Hour, scripted: bool) {
        let want = Want {
            branch: 1,
            hour,
            ahead: 0,
            behind: 0,
            playing: true,
            perspective: Default::default(),
            map_mode: if scripted {
                viewer_world::map::map_mode::MapMode::Scripted("synthetic".into())
            } else {
                Default::default()
            },
            show_allied_plans: false,
            counters: Default::default(),
            view: super::super::tests::test_view(),
            have: Vec::new(),
            received: 0,
        };
        *self.driver.core.requested.lock().unwrap() = Some((epoch, want, None));
        assert!(self.driver.event(Event::Want));
    }
    fn next_event(&mut self) -> bool {
        let event = self
            .inbox
            .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
            .unwrap();
        let part = matches!(event, Event::Part { .. });
        assert!(self.driver.event(event));
        part
    }
    fn reply(&mut self, current: Current) {
        let read = self
            .driver
            .source
            .take()
            .expect("Driver posted source read");
        read.ready(current, None);
        loop {
            let event = self
                .inbox
                .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
                .unwrap();
            let frame = matches!(event, Event::Frame { .. });
            assert!(self.driver.event(event));
            if frame {
                break;
            }
        }
    }
    fn plan_and_request(&mut self) {
        assert!(matches!(self.driver.turn(), Turn::Progress));
        assert!(matches!(self.driver.turn(), Turn::Progress));
        assert!(self.driver.reading.is_some());
    }
    fn admit_parts(&mut self) {
        for _ in 0..8 {
            assert!(matches!(self.driver.turn(), Turn::Progress));
        }
    }
    fn finish_parts(&mut self) {
        let mut count = 0;
        while count < 8 {
            count += usize::from(self.next_event());
        }
    }
}

#[test]
fn real_driver_pool_space_retries_owned_jobs_and_result_space_releases_complete_credit() {
    let pool = Arc::new(Pool::new(1).unwrap());
    let (entered, running) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    pool.submit(Box::new(move || {
        entered.send(()).unwrap();
        wait.recv().unwrap();
    }));
    running
        .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
        .unwrap();
    for _ in 0..15 {
        pool.submit(Box::new(|| {}));
    }
    let (drained, empty) = mpsc::channel();
    let (start_parts, parts_wait) = mpsc::channel();
    pool.submit(Box::new(move || {
        drained.send(()).unwrap();
        parts_wait.recv().unwrap();
    }));
    let mut f = RealDriver::new(pool);
    f.plan_and_request();
    f.reply(super::super::tests::sized_frame(100, 1));
    assert!(matches!(f.driver.turn(), Turn::Park));
    assert!(f.driver.pool_blocked);
    let ticket = f.driver.ledger.next_admission().unwrap().0;
    assert_eq!(f.driver.ledger.next_admission(), Some((ticket, 0)));
    assert!(
        matches!(&f.driver.targets[&ticket], PreparedTarget::Parts(_, jobs) if jobs.len() == 8)
    );
    release.send(()).unwrap();
    f.next_event(); // the real dequeue posts PoolSpace, no unrelated Want
    assert!(!f.driver.pool_blocked);
    empty
        .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
        .unwrap();
    f.admit_parts();
    start_parts.send(()).unwrap();
    f.finish_parts();
    f.driver
        .core
        .outbox
        .try_send(gap_built(99, "occupy output"))
        .ok()
        .unwrap();
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert!(f.driver.output_blocked);
    assert_eq!(f.driver.ledger.len(), 1);
    assert!(matches!(
        f.driver.targets[&ticket],
        PreparedTarget::Ready(_)
    ));
    assert_eq!(f.results.try_recv().unwrap().hour, 99);
    f.driver.core.result_space.store(true, Ordering::Release);
    assert!(f.driver.event(Event::ResultSpace));
    assert!(!f.driver.core.result_space.load(Ordering::Acquire));
    f.driver.turn();
    let built = f.results.try_recv().unwrap();
    assert_eq!((built.branch, built.hour), (1, 100));
    assert_eq!(
        built.layers.err().unwrap(),
        "political: political: the hour builder panicked"
    );
    assert_eq!(
        built.child_laps.iter().map(|s| s.name).collect::<Vec<_>>(),
        NAMES
    );
    assert_eq!(f.driver.ledger.len(), 0);
}

#[test]
fn real_driver_assembles_completed_parts_while_the_next_source_reply_is_withheld() {
    let mut f = RealDriver::new(Arc::new(Pool::new(1).unwrap()));
    f.plan_and_request();
    f.reply(super::super::tests::sized_frame(100, 1));
    f.admit_parts();
    f.want(0, 101, false);
    f.plan_and_request(); // original work accepted; leave source101 pending
    let reading = f.driver.reading.as_ref().unwrap().ticket;
    f.finish_parts();
    f.driver.turn();
    assert_eq!(f.results.try_recv().unwrap().hour, 100);
    assert_eq!(f.driver.reading.as_ref().unwrap().ticket, reading);
    f.want(1, 105, false); // actual in-flight cancellation acknowledgement
    while f.driver.reading.is_some() {
        f.next_event();
    }
    f.driver.turn();
    assert!(f.driver.ledger.len() <= 1);
    assert!(!f.driver.event(Event::Shutdown));
}

#[test]
fn real_driver_native_colour_folds_release_each_frame_and_match_collected_reference_bytes() {
    for refuse in [false, true] {
        let mut f = RealDriver::new(Arc::new(Pool::new(1).unwrap()));
        f.want(0, 820, true);
        assert!(matches!(f.driver.turn(), Turn::Progress));
        // An already accepted predecessor is captured by plan(), not newly
        // requested from the source. Its empty slot terminates as a refused
        // predecessor; the colour starts cold, exactly as before.
        let slot = Slot::new();
        Fill(slot.clone()).set(None);
        let plan = f.driver.decode.as_mut().unwrap();
        plan.previous = Some((99, slot));
        plan.next_source = 100;
        if refuse {
            plan.colour = Some(Err(io::Error::other("first colour refusal")));
        }
        for hour in 100..820 {
            assert!(matches!(f.driver.turn(), Turn::Progress));
            let current = if hour == 317 {
                fixed_gap(hour, "recorded gap")
            } else {
                super::super::tests::sized_frame(hour, 2)
            };
            let weak = match &current {
                Current::Frame(frame) => Some(Arc::downgrade(frame)),
                _ => None,
            };
            f.reply(current);
            if !refuse {
                assert!(f.driver.history.as_ref().unwrap().work.is_some());
                assert_eq!(f.driver.ledger.len(), 1);
                assert!(matches!(f.driver.turn(), Turn::Progress));
                while f.driver.history.is_some() {
                    f.next_event();
                }
            }
            assert!(
                weak.is_none_or(|weak| weak.upgrade().is_none()),
                "a consumed source frame survived its fold"
            );
            assert_eq!(f.driver.ledger.len(), 1);
        }
        assert!(matches!(f.driver.turn(), Turn::Progress));
        let endpoint = super::super::tests::sized_frame(820, 2);
        f.reply(endpoint);
        let reference: Vec<_> = (100..=820)
            .map(|hour| {
                if hour == 317 {
                    fixed_gap(hour, "recorded gap")
                } else {
                    super::super::tests::sized_frame(hour, 2)
                }
            })
            .collect();
        let expected = if refuse {
            Err(io::Error::other("first colour refusal"))
        } else {
            carry_colour(None, reference, colour_fill)
        };
        let actual = f.driver.core.colours.lock().unwrap().pop().unwrap();
        match (actual, expected) {
            (Ok(actual), Ok(expected)) => {
                assert_eq!(actual, expected);
                assert_eq!(
                    actual.unwrap().mode.as_bytes(),
                    expected.unwrap().mode.as_bytes()
                );
            }
            (Err(actual), Err(expected)) => assert_eq!(actual, expected.to_string()),
            _ => panic!("sequential colour refusal differs"),
        }
    }
}

#[test]
fn real_driver_retired_accepted_colour_fold_keeps_credit_until_its_reply() {
    let pool = Arc::new(Pool::new(1).unwrap());
    let (entered, running) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    pool.submit(Box::new(move || {
        entered.send(()).unwrap();
        wait.recv().unwrap();
    }));
    running
        .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
        .unwrap();
    let mut f = RealDriver::new(pool);
    f.want(0, 120, true);
    assert!(matches!(f.driver.turn(), Turn::Progress));
    f.driver.decode.as_mut().unwrap().next_source = 100;
    assert!(matches!(f.driver.turn(), Turn::Progress));
    f.reply(super::super::tests::sized_frame(100, 1));
    let old = f.driver.decode.as_ref().unwrap().ticket;
    assert!(matches!(f.driver.turn(), Turn::Progress)); // fold accepted, worker held
    f.want(1, 105, false);
    assert_eq!(f.driver.ledger.len(), 1);
    assert!(f.driver.ledger.get(old).unwrap().retired);
    assert!(f.driver.history.as_ref().unwrap().work.is_none());
    assert!(f.driver.event(Event::Colour {
        ticket: old + 1,
        key: (1, 100),
        folded: Box::new(dispatch_plan::ColourFolded {
            colour: Ok(None),
            timing: None
        })
    }));
    assert!(
        f.driver.history.is_some(),
        "stale fold cannot release another credit"
    );
    release.send(()).unwrap();
    while f.driver.history.is_some() {
        f.next_event();
    }
    assert_eq!(f.driver.ledger.len(), 0);
    assert!(
        f.results.try_recv().is_err(),
        "retired colour work never publishes an hour"
    );
    f.plan_and_request();
    f.reply(fixed_gap(105, "new epoch"));
    f.driver.turn();
    let built = f.results.try_recv().unwrap();
    assert_eq!(
        (built.epoch, built.hour, built.layers.err().unwrap()),
        (1, 105, "new epoch".into())
    );
}

#[test]
fn real_driver_unaccepted_colour_fold_releases_its_frame_on_supersession() {
    let pool = Arc::new(Pool::new(1).unwrap());
    let (entered, running) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    pool.submit(Box::new(move || {
        entered.send(()).unwrap();
        wait.recv().unwrap();
    }));
    running
        .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
        .unwrap();
    for _ in 0..16 {
        pool.submit(Box::new(|| {}));
    }
    let mut f = RealDriver::new(pool);
    f.want(0, 120, true);
    assert!(matches!(f.driver.turn(), Turn::Progress));
    f.driver.decode.as_mut().unwrap().next_source = 100;
    assert!(matches!(f.driver.turn(), Turn::Progress));
    let current = super::super::tests::sized_frame(100, 1);
    let weak = match &current {
        Current::Frame(frame) => Arc::downgrade(frame),
        _ => unreachable!(),
    };
    f.reply(current);
    assert!(matches!(f.driver.turn(), Turn::Park));
    assert!(f.driver.pool_blocked);
    f.want(1, 105, false);
    assert!(f.driver.history.is_none());
    assert!(f.driver.reading.is_none());
    assert_eq!(f.driver.ledger.len(), 0);
    assert!(weak.upgrade().is_none());
    release.send(()).unwrap();
}
