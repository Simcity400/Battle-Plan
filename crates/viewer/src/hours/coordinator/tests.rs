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
    vision_pressure: Option<engine_allocation::credit::MemoryBudget>,
    vision_calls: usize,
    fill: fn(
        &replay_frame::frame::DecodedFrame,
        Option<&viewer_world::map::map_mode::ColourState>,
    ) -> io::Result<dispatch_plan::Colour>,
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
        viewing: None,
        laps: Vec::new(),
        child_laps: Vec::new(),
        view: super::super::tests::test_view(),
        wall: 0.0,
        trace: Default::default(),
    }
}
impl BuildTarget for TestBuilder {
    fn colour_work(&self, plan: &mut DispatchPlan, current: Current) -> Box<dyn FnOnce() + Send> {
        plan.colour_work(current, self.events.clone(), self.fill)
    }
    fn prepare(&mut self, plan: DispatchPlan, current: Current) -> PreparedTarget {
        if let Some(owner) = &self.vision_pressure {
            self.vision_calls += 1;
            if owner.admit_item(1).is_err() {
                return PreparedTarget::Pressure(Box::new(plan), current);
            }
        }
        if let Current::Gap { reason, .. } = &current {
            let mut built = gap_built(plan.hour, reason);
            built.epoch = plan.epoch;
            built.trace = plan.trace;
            return PreparedTarget::Ready(Box::new(built));
        }
        if let Some(colour) = plan.colour {
            let colour =
                colour.and_then(|colour| carry_colour(colour, [current.clone()], self.fill));
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
fn output_hour(output: Output) -> Built {
    match output {
        Output::Hour(built) => built,
        Output::Pressure => panic!("unexpected pressure notice"),
    }
}
struct RealDriver {
    driver: Driver<TestBuilder>,
    inbox: Receiver<Event>,
    results: Receiver<Output>,
}
impl RealDriver {
    fn new(pool: Arc<Pool>) -> Self {
        Self::over(
            pool,
            Arc::new(crate::playback::tests::fixture()),
            SourceMailbox::new(|_| true),
            map_display_data::credit::display_products(),
        )
    }
    fn over(
        pool: Arc<Pool>,
        replay: Arc<Replay>,
        source: Arc<SourceMailbox>,
        owner: &engine_allocation::credit::MemoryBudget,
    ) -> Self {
        let (events, inbox) = mpsc::channel();
        let (outbox, results) = mpsc::sync_channel(1);
        let core = TestBuilder {
            vision_pressure: None,
            vision_calls: 0,
            fill: colour_fill,
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
                usable_sent: BTreeSet::new(),
                anchor: None,
                slots: BTreeMap::new(),
                memos: Arc::new(Memos::new()),
                reusable: BTreeMap::new(),
                pool,
                parallel: 2,
                events,
                outbox,
                result_space: Default::default(),
                refusal: Default::default(),
            },
        };
        let mut fixture = Self {
            driver: Driver::new(core, source, owner).unwrap(),
            inbox,
            results,
        };
        fixture.want(0, 100, false);
        fixture
    }
    fn take_output(&self) -> Built {
        output_hour(self.results.try_recv().unwrap())
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
            grid: None,
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
        .try_send(Output::Hour(gap_built(99, "occupy output")))
        .ok()
        .unwrap();
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert!(f.driver.output_blocked);
    assert_eq!(f.driver.ledger.len(), 1);
    assert!(matches!(
        f.driver.targets[&ticket],
        PreparedTarget::Ready(_)
    ));
    assert_eq!(f.take_output().hour, 99);
    f.driver.core.result_space.store(true, Ordering::Release);
    assert!(f.driver.event(Event::ResultSpace));
    assert!(!f.driver.core.result_space.load(Ordering::Acquire));
    f.driver.turn();
    let built = f.take_output();
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
    assert_eq!(f.take_output().hour, 100);
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
        let slots = Slots::new();
        Fill(slots.political.clone()).set(None);
        let plan = f.driver.decode.as_mut().unwrap();
        plan.previous = Some((99, slots));
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
    let built = f.take_output();
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

mod credit_pressure;
mod grid;

#[test]
fn original_ticket_is_completed_once_after_a_byte_credit_retry() {
    let pool = Arc::new(Pool::new(1).unwrap());
    let mut f = RealDriver::new(pool);
    f.plan_and_request();
    f.reply(super::super::tests::sized_frame(100, 1));
    let ticket = *f.driver.targets.keys().next().unwrap();
    let Some(PreparedTarget::Parts(_, jobs)) = f.driver.targets.get_mut(&ticket) else {
        panic!("parts")
    };
    let original = jobs.pop_front().unwrap();
    let events = f.driver.core.events.clone();
    jobs.push_front(HeldJob {
        name: original.name,
        work: Box::new(move || {
            let generation = map_display_data::credit::display_products().generation();
            events
                .send(Event::RetryPart {
                    ticket,
                    generation,
                    reason: "synthetic part bytes pinned".into(),
                    job: original,
                })
                .unwrap();
        }),
    });
    f.admit_parts();
    loop {
        let complete = match f.driver.targets.get(&ticket).unwrap() {
            PreparedTarget::Parts(pending, _) => pending.received == 7,
            _ => false,
        };
        if complete && !f.driver.retries.is_empty() {
            break;
        }
        f.next_event();
    }
    assert!(f.driver.ledger.ready().is_empty());
    f.driver.pool_blocked = true;
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert!(matches!(f.results.try_recv().unwrap(), Output::Pressure));
    assert_eq!(
        f.driver
            .core
            .refusal
            .for_target(0, (1, 100))
            .unwrap_err()
            .to_string(),
        "display political awaits credit: synthetic part bytes pinned"
    );
    assert!(f.driver.event(Event::ResultSpace));
    f.driver.pool_blocked = false;
    let retry = f.driver.retries.values_mut().next().unwrap();
    retry.0 = map_display_data::credit::display_products()
        .generation()
        .wrapping_sub(1);
    assert!(matches!(f.driver.turn(), Turn::Progress));
    while f.driver.ledger.ready().is_empty() {
        f.next_event();
    }
    assert!(f.driver.retries.is_empty());
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert_eq!(f.take_output().hour, 100);
    assert!(f.results.try_recv().is_err());
}

#[test]
fn product_releases_without_waiters_stay_coalesced() {
    let pool = Arc::new(Pool::new(1).unwrap());
    let mut f = RealDriver::new(pool);
    let owner = map_display_data::credit::MemoryBudget::new(8192);
    let wakes = Arc::new(AtomicU64::new(0));
    let callback = Arc::clone(&wakes);
    f.driver.product_space = Some(
        owner
            .listen(move || {
                callback.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap(),
    );
    drop(owner.admit_item(1).unwrap());
    assert_eq!(wakes.load(Ordering::Relaxed), 1);
    assert!(f.driver.event(Event::ProductSpace));
    f.driver.turn();
    for _ in 0..1500 {
        drop(owner.admit_item(1).unwrap());
    }
    assert_eq!(wakes.load(Ordering::Relaxed), 1, "no per-node idle wake");
    let generation = f.driver.product_space.as_ref().unwrap().acknowledge();
    drop(owner.admit_item(1).unwrap());
    assert_eq!(
        wakes.load(Ordering::Relaxed),
        2,
        "a waiter can rearm releases"
    );
    assert_ne!(generation, owner.generation());
}

fn kept_border_fill(
    frame: &replay_frame::frame::DecodedFrame,
    previous: Option<&viewer_world::map::map_mode::ColourState>,
) -> io::Result<dispatch_plan::Colour> {
    use map_display_data::landscape::{
        self, ColourOp, ColourState, LayerProgram, ProvinceIds, Style, TargetId, TargetOp,
        TargetPrograms,
    };
    let provinces = ProvinceIds([1, 2].into_iter().try_fold(
        Default::default(),
        |ids: map_display_data::value_index::ValueIndex<u32, ()>, id| ids.inserted(id, ()),
    )?);
    let id = TargetId::Country(1);
    let target = map_display_data::retained::Retained::build(0, || TargetOp {
        id,
        provinces,
        operation: if frame.hour == 10 || frame.hour == 100 {
            ColourOp::Set {
                style: Style {
                    color: [7, 19, 31, 255],
                    pulse: false,
                    fill: 0.5,
                },
            }
        } else {
            ColourOp::Keep {}
        },
    })?;
    let program = LayerProgram {
        targets: TargetPrograms(Default::default())
            .0
            .inserted(id, target)
            .map(TargetPrograms)?,
        ..Default::default()
    };
    Ok(Some(Arc::new(ColourState {
        mode: format!(
            "{}:{}",
            previous.map_or("border", |colour| colour.mode.as_str()),
            frame.hour
        ),
        layers: [
            landscape::apply_layer(&program, previous.map(|colour| &colour.layers[0])),
            Default::default(),
        ],
    })))
}

// The real source actors and Driver pump run here; only authored colour/body work is synthetic.
enum PressureSource {
    Lane(DecodeLane),
    Shared(SharedDecoder),
}
impl PressureSource {
    fn client(&self) -> Arc<SourceMailbox> {
        match self {
            Self::Lane(lane) => lane.client(),
            Self::Shared(actor) => actor.client(),
        }
    }
}
struct PressureFixture {
    pump: RealDriver,
    source: PressureSource,
    owner: engine_allocation::credit::MemoryBudget,
    prepared: Option<(std::path::PathBuf, uuid::Uuid)>,
}
impl PressureFixture {
    fn new(prepared: bool, shared: bool) -> Self {
        Self::span(prepared, shared, 10, 15, 15)
    }
    fn span(prepared: bool, shared: bool, first: Hour, last: Hour, endpoint: Hour) -> Self {
        let replay = Arc::new(
            Replay::from_records(
                b"campaign".to_vec(),
                &[crate::playback::tests::records(
                    1,
                    &(first..=last).collect::<Vec<_>>(),
                )],
            )
            .unwrap(),
        );
        let owner = engine_allocation::credit::MemoryBudget::new(1_000_000);
        let mut playback = Playback::unloaded(replay.clone())
            .unwrap()
            .with_change_budget(owner.clone());
        let prepared = if prepared {
            let (map, path, id) = super::super::tests::paged_map(&replay);
            playback = playback.with_checkpoints(map);
            Some((path, id))
        } else {
            None
        };
        playback.seek(1, first).unwrap();
        let Current::Frame(initial) = playback.current() else {
            panic!("recorded first frame")
        };
        let colour = kept_border_fill(initial, None).unwrap();
        let source = if shared {
            let actor = SharedDecoder::new(replay.clone()).unwrap();
            actor.seed(playback).unwrap();
            PressureSource::Shared(actor)
        } else {
            PressureSource::Lane(DecodeLane::start(playback).unwrap())
        };
        let mut pump = RealDriver::over(
            Arc::new(Pool::new(1).unwrap()),
            replay,
            source.client(),
            &owner,
        );
        pump.driver.core.fill = kept_border_fill;
        pump.want(0, endpoint, true);
        assert!(matches!(pump.driver.turn(), Turn::Progress));
        let plan = pump.driver.decode.as_mut().unwrap();
        plan.next_source = first + 1;
        plan.colour = Some(Ok(colour));
        Self {
            pump,
            source,
            owner,
            prepared,
        }
    }
    fn source_reply(&mut self) {
        loop {
            let event = self
                .pump
                .inbox
                .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
                .unwrap();
            let frame = matches!(event, Event::Frame { .. });
            assert!(self.pump.driver.event(event));
            if frame {
                return;
            }
        }
    }
    fn fold(&mut self, hour: Hour) {
        assert_eq!(self.pump.driver.decode.as_ref().unwrap().next_source, hour);
        assert!(matches!(self.pump.driver.turn(), Turn::Progress));
        self.source_reply();
        assert!(self.pump.driver.history.is_some());
        assert!(matches!(self.pump.driver.turn(), Turn::Progress));
        while self.pump.driver.history.is_some() {
            self.pump.next_event();
        }
        assert_eq!(
            self.pump.driver.decode.as_ref().unwrap().next_source,
            hour + 1
        );
    }
    fn pressure(&mut self, hour: Hour) -> engine_allocation::credit::Credit {
        let held = self
            .owner
            .admit(self.owner.limit() - self.owner.used())
            .unwrap();
        assert!(matches!(self.pump.driver.turn(), Turn::Progress));
        self.source_reply();
        let driver = &mut self.pump.driver;
        let plan = driver.decode.as_ref().unwrap();
        assert_eq!((plan.hour, plan.next_source), (15, hour));
        assert!(driver.source_retry.is_some());
        assert!(driver.history.is_none());
        assert!(driver.targets.is_empty());
        assert!(driver.core.sent.is_empty());
        assert_eq!(driver.core.sends, 0);
        assert!(driver.core.slots.is_empty());
        assert_eq!(
            driver.ledger.get(plan.ticket).unwrap().phase,
            flow::Phase::Decode
        );
        assert_eq!(
            driver
                .core
                .refusal
                .current
                .lock()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .reason,
            "display change-fact working set remains pinned; common paging is required"
        );
        if let PressureSource::Lane(lane) = &self.source {
            let state = DecodeLane::lock(&lane.shared);
            assert!(!state.frames.contains_key(&(1, hour)));
            assert_eq!(state.pressure, Some((1, hour)));
        }
        held
    }
    fn park_and_release(&mut self, held: engine_allocation::credit::Credit) {
        // A named notice travels outside Built; taking it never acknowledges an hour.
        assert!(matches!(self.pump.driver.turn(), Turn::Progress));
        assert!(matches!(
            self.pump.results.try_recv().unwrap(),
            Output::Pressure
        ));
        assert!(self.pump.driver.event(Event::ResultSpace));
        assert!(matches!(self.pump.driver.turn(), Turn::Park));
        assert!(self.pump.driver.reading.is_none());
        while let Ok(event) = self.pump.inbox.try_recv() {
            assert!(self.pump.driver.event(event));
        }
        self.pump
            .driver
            .product_space
            .as_ref()
            .unwrap()
            .acknowledge();
        drop(held);
        assert!(matches!(
            self.pump
                .inbox
                .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
                .unwrap(),
            Event::ProductSpace
        ));
        // No Want, timer or polling: the owner release is the only wake.
        assert!(self.pump.driver.event(Event::ProductSpace));
        assert!(matches!(self.pump.driver.turn(), Turn::Progress));
        self.source_reply();
        assert!(self.pump.driver.source_retry.is_none());
        assert!(
            self.pump
                .driver
                .core
                .refusal
                .current
                .lock()
                .unwrap()
                .is_empty()
        );
    }
    fn finish(mut self) {
        let actual = self
            .pump
            .driver
            .core
            .colours
            .lock()
            .unwrap()
            .pop()
            .unwrap()
            .unwrap()
            .unwrap();
        use map_display_data::landscape::{Border, Style, TargetId};
        assert_eq!(actual.mode, "border:10:11:12:13:14:15");
        assert_eq!(
            actual.layers[0].borders,
            BTreeMap::from([(
                TargetId::Country(1),
                Border {
                    provinces: vec![1, 2],
                    style: Some(Style {
                        color: [7, 19, 31, 255],
                        pulse: false,
                        fill: 0.5
                    })
                }
            )])
        );
        assert!(!actual.layers[0].refused);
        self.pump.admit_parts();
        self.pump.finish_parts();
        self.pump.driver.turn();
        let built = self.pump.take_output();
        assert_eq!((built.epoch, built.branch, built.hour), (0, 1, 15));
        assert!(matches!(built.current, Current::Frame(ref frame) if frame.hour == 15));
        assert_eq!(self.pump.driver.core.sends, 1);
        assert_eq!(
            self.pump
                .driver
                .core
                .sent
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [(1, 15)]
        );
        assert!(self.pump.results.try_recv().is_err());
        self.close();
    }
    fn close(mut self) {
        drop(self.pump);
        drop(self.source);
        if let Some((path, id)) = self.prepared.take() {
            let _ = store::Store::local(id).unwrap().clear();
            let _ = std::fs::remove_file(path);
        }
    }
}

#[test]
fn intermediate_pressure_through_lane_and_coordinator_preserves_keep_borders_for_both_readers() {
    for prepared in [false, true] {
        for shared in [false, true] {
            let mut f = PressureFixture::new(prepared, shared);
            f.fold(11);
            let pin = super::super::tests::source_frame(&f.source.client(), 1, 11);
            let before = f
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
            let held = f.pressure(12);
            let after = f
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
            assert!(Arc::ptr_eq(
                before.as_ref().unwrap(),
                after.as_ref().unwrap()
            ));
            assert_eq!(after.unwrap().mode, "border:10:11");
            f.park_and_release(held);
            assert!(f.pump.driver.history.is_some());
            assert!(matches!(f.pump.driver.turn(), Turn::Progress));
            while f.pump.driver.history.is_some() {
                f.pump.next_event();
            }
            assert_eq!(f.pump.driver.decode.as_ref().unwrap().next_source, 13);
            f.fold(13);
            f.fold(14);
            assert!(matches!(f.pump.driver.turn(), Turn::Progress));
            f.source_reply();
            drop(pin);
            f.finish();
        }
    }
}

#[test]
fn endpoint_pressure_through_lane_and_coordinator_is_retried_and_never_cached_for_both_readers() {
    for prepared in [false, true] {
        for shared in [false, true] {
            let mut f = PressureFixture::new(prepared, shared);
            for hour in 11..=14 {
                f.fold(hour);
            }
            let pin = super::super::tests::source_frame(&f.source.client(), 1, 14);
            let held = f.pressure(15);
            // A second client receives pressure, not a cached completed gap.
            let (send, done) = mpsc::channel();
            let second = f.source.client();
            let _handle = second.post(next_id(), (1, 15), move |_, reply| {
                send.send(reply).unwrap();
            });
            assert!(matches!(
                done.recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
                    .unwrap(),
                source_mailbox::Reply::Pressure(_)
            ));
            f.park_and_release(held);
            assert!(f.pump.driver.decode.is_none());
            let retry = super::super::tests::source_frame(&f.source.client(), 1, 15);
            assert!(matches!(retry, Current::Frame(ref frame) if frame.hour == 15));
            drop((pin, retry));
            f.finish();
        }
    }
}

#[test]
fn source_pressure_raced_release_retries_without_a_second_wake_and_supersession_clears_the_notice()
{
    let mut f = RealDriver::new(Arc::new(Pool::new(1).unwrap()));
    let owner = engine_allocation::credit::MemoryBudget::new(8192);
    let events = f.driver.core.events.clone();
    f.driver.product_space = Some(
        owner
            .listen(move || {
                let _ = events.send(Event::ProductSpace);
            })
            .unwrap(),
    );
    f.plan_and_request();
    let ticket = f.driver.decode.as_ref().unwrap().ticket;
    let held = owner.admit(owner.limit() - owner.used()).unwrap();
    f.driver.source.take().unwrap().pressure(DecodePressure {
        budget: owner.clone(),
        bytes: 128,
        cursor: Some(99),
        release: owner.release_token(),
        reason: "synthetic pinned working set".into(),
    });
    let reply = f
        .inbox
        .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
        .unwrap();
    drop(held); // release before the coordinator receives the pressure continuation
    assert!(f.driver.event(reply));
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert!(f.driver.reading.is_some());
    assert_eq!(f.driver.decode.as_ref().unwrap().ticket, ticket);
    assert!(f.driver.core.sent.is_empty());
    // A repeated synchronous refusal at the same cursor cannot spin on its own staging drops.
    f.driver.source.take().unwrap().pressure(DecodePressure {
        budget: owner.clone(),
        bytes: 128,
        cursor: Some(99),
        release: owner.release_token(),
        reason: "synthetic pinned working set".into(),
    });
    while f.driver.reading.is_some() {
        f.next_event();
    }
    f.driver.turn();
    assert!(f.driver.source_retry.as_ref().unwrap().needs_release);
    assert!(matches!(f.driver.turn(), Turn::Park));
    assert!(f.driver.reading.is_none());
    assert_eq!(
        f.driver
            .core
            .refusal
            .current
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .reason,
        "synthetic pinned working set"
    );
    drop(owner.admit(1).unwrap()); // even a net-zero allocation/release must wake this continuation
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert!(f.driver.reading.is_some());
    f.want(1, 105, false);
    while f.driver.reading.is_some() {
        f.next_event();
    }
    assert!(f.driver.source_retry.is_none());
    assert!(f.driver.core.refusal.current.lock().unwrap().is_empty());
    assert!(f.driver.ledger.get(ticket).is_none());
}

#[test]
fn repeated_admission_keeps_release_between_failure_and_error_conversion() {
    use replay_data::rows::TableId;
    use replay_frame::frame::FrameDecoder;
    let mut f = RealDriver::new(Arc::new(Pool::new(1).unwrap()));
    let owner = engine_allocation::credit::MemoryBudget::new(8192);
    let events = f.driver.core.events.clone();
    f.driver.product_space = Some(
        owner
            .listen(move || {
                let _ = events.send(Event::ProductSpace);
            })
            .unwrap(),
    );
    let replay = Replay::from_records(
        b"campaign".to_vec(),
        &[crate::playback::tests::records(1, &[10, 11])],
    )
    .unwrap();
    let replay::container::SeekResult::Frame(initial) = replay.seek(1, 10).unwrap() else {
        panic!("recorded first hour")
    };
    let frames = replay.read_block(&replay.index[0]).unwrap();
    assert_eq!(frames[1].hour, 11);
    let mut decoder = FrameDecoder::resume_for_tables_with_change_facts(
        &[TableId::Clock],
        *initial,
        owner.clone(),
    )
    .unwrap();
    f.plan_and_request();
    let held = owner.admit(owner.limit() - owner.used()).unwrap();
    let scope = owner.track_releases();
    let attempt = owner.release_token();
    let error = decoder.advance(&frames[1]).unwrap_err();
    assert!(FrameDecoder::change_fact_admission_refused(&error));
    let pressure = DecodePressure::from_error(&error, Some(10), &owner, attempt).unwrap();
    drop(scope);
    f.driver.source.take().unwrap().pressure(pressure);
    while f.driver.reading.is_some() {
        f.next_event();
    }
    f.driver.turn();
    assert!(matches!(f.results.try_recv().unwrap(), Output::Pressure));
    f.driver.event(Event::ResultSpace);
    drop(held);
    assert!(matches!(f.driver.turn(), Turn::Progress));
    assert!(f.driver.reading.is_some());

    let mut held = owner.admit(owner.limit() - owner.used()).unwrap();
    let cleanup = held.split_off(1);
    let scope = owner.track_releases();
    let attempt = owner.release_token();
    let error = decoder.advance(&frames[1]).unwrap_err();
    // Exact r5 interval: independent release after failure, before error conversion.
    std::thread::spawn(move || drop(held)).join().unwrap();
    drop(cleanup);
    let pressure = DecodePressure::from_error(&error, Some(10), &owner, attempt).unwrap();
    assert!(owner.released_since(pressure.release));
    drop(scope);
    f.driver.source.take().unwrap().pressure(pressure);
    while f.driver.reading.is_some() {
        f.next_event();
    }
    assert!(f.driver.source_retry.as_ref().unwrap().needs_release);
    f.driver.turn();
    assert!(
        f.driver.reading.is_some(),
        "queued ProductSpace must retry without a third release"
    );
    assert!(f.driver.source_retry.is_none());
    assert_eq!(f.driver.decode.as_ref().unwrap().next_source, 100);
    assert!(f.driver.core.sent.is_empty());
    assert_eq!(decoder.current().unwrap().hour, 10);
}

#[test]
fn a_paused_pressure_notice_repaints_on_release_and_completed_delivery() {
    let mut f = RealDriver::new(Arc::new(Pool::new(1).unwrap()));
    f.driver.core.want.as_mut().unwrap().playing = false;
    let owner = engine_allocation::credit::MemoryBudget::new(8192);
    let events = f.driver.core.events.clone();
    f.driver.product_space = Some(
        owner
            .listen(move || {
                let _ = events.send(Event::ProductSpace);
            })
            .unwrap(),
    );
    let context = eframe::egui::Context::default();
    let wakes = Arc::new(AtomicU64::new(0));
    let seen = wakes.clone();
    context.set_request_repaint_callback(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
    });
    f.driver.core.refusal.bind(&context);
    f.plan_and_request();
    let held = owner.admit(owner.limit() - owner.used()).unwrap();
    f.driver.source.take().unwrap().pressure(DecodePressure {
        budget: owner.clone(),
        bytes: 128,
        cursor: Some(99),
        release: owner.release_token(),
        reason: "paused display credit pinned".into(),
    });
    while f.driver.reading.is_some() {
        f.next_event();
    }
    assert_eq!(
        f.driver
            .core
            .refusal
            .for_target(0, (1, 100))
            .unwrap_err()
            .to_string(),
        "paused display credit pinned"
    );
    f.driver.turn();
    let _ = f.results.try_recv().unwrap();
    f.driver.event(Event::ResultSpace);
    f.driver.turn();
    while let Ok(event) = f.inbox.try_recv() {
        f.driver.event(event);
    }
    // Settle egui's initial repaint requests; no timer or playing clock remains.
    for _ in 0..3 {
        let mut output = context.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
    }
    wakes.store(0, Ordering::Relaxed);
    f.driver.product_space.as_ref().unwrap().acknowledge();
    drop(held);
    assert!(matches!(
        f.inbox
            .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
            .unwrap(),
        Event::ProductSpace
    ));
    f.driver.event(Event::ProductSpace);
    assert!(
        wakes.load(Ordering::Relaxed) > 0,
        "release wakes a paused presentation"
    );
    f.driver.turn();
    f.reply(super::super::tests::sized_frame(100, 1));
    assert!(f.driver.core.refusal.for_target(0, (1, 100)).is_ok());
    f.admit_parts();
    f.finish_parts();
    for _ in 0..3 {
        let mut output = context.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
    }
    wakes.store(0, Ordering::Relaxed);
    f.driver.turn();
    assert!(
        wakes.load(Ordering::Relaxed) > 0,
        "delivery also wakes after the source recovered"
    );
    assert_eq!(f.take_output().hour, 100);
}

#[test]
fn an_exact_wait_reports_the_retained_decode_that_blocks_its_new_target() {
    let mut f = RealDriver::new(Arc::new(Pool::new(1).unwrap()));
    let (outbox, results) = mpsc::sync_channel(2);
    f.driver.core.outbox = outbox;
    f.results = results;
    f.want(0, 102, false);
    f.plan_and_request();
    let ticket = f.driver.decode.as_ref().unwrap().ticket;
    let owner = engine_allocation::credit::MemoryBudget::new(8192);
    let held = owner.admit(8192).unwrap();
    f.driver.source.take().unwrap().pressure(DecodePressure {
        budget: owner.clone(),
        bytes: 128,
        cursor: Some(101),
        release: owner.release_token(),
        reason: "display credit remains pinned".into(),
    });
    while f.driver.reading.is_some() {
        f.next_event();
    }
    f.driver
        .core
        .outbox
        .try_send(Output::Hour(gap_built(101, "queued completed hour")))
        .ok()
        .unwrap();
    f.driver.turn(); // the completed hour is followed by the pressure notice
    f.want(0, 103, false);
    assert_eq!(f.driver.decode.as_ref().unwrap().ticket, ticket);
    assert_eq!(f.driver.decode.as_ref().unwrap().hour, 102);
    assert_eq!(
        f.driver.ledger.get(ticket).unwrap().phase,
        flow::Phase::Decode
    );
    assert!(!f.driver.ledger.can_plan());
    assert_eq!(f.driver.next_hour(), Some(103));
    assert!(f.driver.core.refusal.for_target(0, (1, 103)).is_ok());
    let completed = Completions::new(
        f.results,
        f.driver.core.refusal.clone(),
        f.driver.core.result_space.clone(),
        f.driver.core.events.clone(),
    );
    let rescue = f.driver.core.outbox.clone();
    let (send, done) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let queued = completed.wait(0, (1, 103)).map(|built| built.hour);
        let refusal = completed.wait(0, (1, 103));
        let _ = send.send((queued, refusal, completed.received.get()));
    });
    let reply = done.recv_timeout(crate::map::test_support::TEST_HANG_GUARD);
    if reply.is_err() {
        // Unpark the old recv path before failing, so the regression cannot strand a thread.
        let _ = rescue.try_send(Output::Hour(gap_built(103, "hang-guard rescue")));
    }
    waiter.join().unwrap();
    let (queued, refusal, received) =
        reply.expect("retained pressure must return without another release/result");
    assert_eq!(queued.unwrap(), 101);
    assert_eq!(
        received, 1,
        "a pressure notice is never counted as delivered data"
    );
    let error = refusal.err().expect("retained decode refusal");
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(
        error.to_string(),
        "branch 1 hour 102 blocks exact branch 1 hour 103: display credit remains pinned"
    );
    assert_eq!(f.driver.decode.as_ref().unwrap().hour, 102);
    assert!(f.driver.core.sent.is_empty());
    drop(held);
}
