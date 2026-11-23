use super::carry::Carry;
use super::census_tests::{
    DEFINITION, DYNAMIC, HISTORICAL, MODULE, STRIDES, World, capacities, context,
};
use super::leader_carry::{UNITS, UnitCarry};
use super::quiet::QuietRoster;
use super::*;
use collector_observers::character_changes::Watches;
use collector_observers::roster_changes::Isolated;
use std::cell::Cell;
use std::hint::black_box;
use std::time::{Duration, Instant};

struct Flat<'a> {
    world: &'a World,
    regions: Vec<(usize, Vec<u8>)>,
}

impl<'a> Flat<'a> {
    fn new(world: &'a World) -> Self {
        let mut regions = Vec::new();
        for (region, count) in [
            (HISTORICAL, world.historical),
            (DYNAMIC, world.characters - world.historical),
        ] {
            let mut bytes = vec![0; count * 8];
            assert!(world.copy(region, &mut bytes));
            regions.push((region, bytes));
        }
        for (region, stride) in STRIDES {
            let count = if region == DEFINITION {
                world.definitions
            } else {
                world.characters
            };
            let mut bytes = vec![0; count * stride];
            for (index, object) in bytes.chunks_exact_mut(stride).enumerate() {
                let _ = world.copy(region + index * stride, object);
            }
            regions.push((region, bytes));
        }
        Self { world, regions }
    }
}

impl Memory for Flat<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let region = address & !0xffff_ffff;
        match self.regions.iter().find(|(start, _)| *start == region) {
            Some((_, bytes)) => {
                let offset = address - region;
                match bytes.get(offset..offset + output.len()) {
                    Some(source) => {
                        output.copy_from_slice(source);
                        true
                    }
                    None => false,
                }
            }
            None => self.world.copy(address, output),
        }
    }
}

struct Timed<'a> {
    inner: &'a Flat<'a>,
    ticks: Cell<u64>,
    reads: Cell<u64>,
}

#[allow(unused_unsafe)]
fn ticks() -> u64 {
    unsafe { std::arch::x86_64::_rdtsc() }
}

impl Memory for Timed<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let start = ticks();
        let copied = self.inner.copy(address, output);
        self.ticks.set(self.ticks.get() + ticks() - start);
        self.reads.set(self.reads.get() + 1);
        copied
    }
}

#[derive(Clone, Copy, Default)]
struct Times {
    copy: Duration,
    reads: f64,
    calls: u64,
    decode: Duration,
    rebuild: Duration,
}

fn hour(job: &mut CharactersJob, memory: &Flat<'_>, captured: &mut Captured, timed: bool) -> Times {
    let counted = Timed {
        inner: memory,
        ticks: Cell::new(0),
        reads: Cell::new(0),
    };
    let ticks_before = ticks();
    let started = Instant::now();
    let report = if timed {
        copy_characters(&counted, context(PART_MASKS[0]), job, Roles::Gather)
    } else {
        copy_characters(memory, context(PART_MASKS[0]), job, Roles::Gather)
    };
    let copy = started.elapsed();
    let total = (ticks() - ticks_before).max(1);
    assert!(report.raw_ok && !report.plan_failed);
    let started = Instant::now();
    captured.decode_characters_payload(MODULE, job, &mut [0; N]);
    let decode = started.elapsed();
    let started = Instant::now();
    captured.remember_templates(job);
    let rebuild = started.elapsed();
    if job.grow_for_next() {
        job.grow();
    }
    job.prepare_read_plans();
    job.clear();
    Times {
        copy,
        reads: counted.ticks.get() as f64 / total as f64,
        calls: counted.reads.get(),
        decode,
        rebuild,
    }
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

#[test]
#[ignore = "bench: run in release with --ignored --nocapture"]
fn quiet_roster_hour_costs_at_late_war_scale() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: collector_observers::unit_leader_changes::Watches =
        collector_observers::unit_leader_changes::Watches::new();
    static UNOBSERVED: collector_observers::unit_leader_changes::Watches =
        collector_observers::unit_leader_changes::Watches::new();
    static CENSUS: collector_certificates::watch_registry::WatchRegistry =
        collector_certificates::watch_registry::WatchRegistry::new();
    static GATHERED: collector_certificates::watch_registry::WatchRegistry =
        collector_certificates::watch_registry::WatchRegistry::new();
    const HOURS: usize = 15;
    let world = World::new(13_200);
    let memory = Flat::new(&world);
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = CharactersJob::new_part(0);
    job.unit_carry = UnitCarry::with(&LEADERS, UNITS);
    CENSUS.set_enabled(true);
    job.census_carry = super::census::CensusCarry::with(&CENSUS);
    job.carry = Carry::with(&WATCHES);
    job.roster = QuietRoster::with(&DOMAIN);
    let mut captured = Captured::with_capacities(&capacities());
    for _ in 0..5 {
        hour(&mut job, &memory, &mut captured, false);
    }
    let mut evict = vec![0_u8; 256 << 20];
    for (label, cold) in [("warm", false), ("cold", true)] {
        let mut runs = Vec::new();
        for _ in 0..HOURS {
            if cold {
                for line in evict.chunks_mut(64) {
                    line[0] = line[0].wrapping_add(1);
                }
                black_box(&evict);
            }
            runs.push(hour(&mut job, &memory, &mut captured, false));
            assert!(job.roster.counts().quiet, "a settled hour is quiet");
        }
        let timed: Vec<Times> = (0..HOURS)
            .map(|_| {
                if cold {
                    for line in evict.chunks_mut(64) {
                        line[0] = line[0].wrapping_add(1);
                    }
                    black_box(&evict);
                }
                hour(&mut job, &memory, &mut captured, true)
            })
            .collect();
        let share = timed.iter().map(|times| times.reads).sum::<f64>() / HOURS as f64;
        eprintln!(
            "{label}: copy {:?} (reads {:.0}% over {} calls), decode {:?}, rebuilds {:?}",
            median(runs.iter().map(|times| times.copy).collect()),
            share * 100.0,
            timed[0].calls,
            median(runs.iter().map(|times| times.decode).collect()),
            median(runs.iter().map(|times| times.rebuild).collect()),
        );
    }
    let rebuilt: Vec<Times> = (0..HOURS)
        .map(|_| {
            job.published.forget();
            hour(&mut job, &memory, &mut captured, false)
        })
        .collect();
    eprintln!(
        "without republishing: decode {:?}, rebuilds {:?}",
        median(rebuilt.iter().map(|times| times.decode).collect()),
        median(rebuilt.iter().map(|times| times.rebuild).collect()),
    );
    job.census_carry = super::census::CensusCarry::with(&GATHERED);
    let gathered: Vec<Times> = (0..HOURS)
        .map(|_| hour(&mut job, &memory, &mut captured, false))
        .collect();
    eprintln!(
        "without a kept census: copy {:?}, decode {:?}",
        median(gathered.iter().map(|times| times.copy).collect()),
        median(gathered.iter().map(|times| times.decode).collect()),
    );
    job.unit_carry = UnitCarry::with(&UNOBSERVED, UNITS);
    let read: Vec<Times> = (0..HOURS)
        .map(|_| hour(&mut job, &memory, &mut captured, false))
        .collect();
    eprintln!(
        "without carried leaders: copy {:?}, decode {:?}",
        median(read.iter().map(|times| times.copy).collect()),
        median(read.iter().map(|times| times.decode).collect()),
    );
}

fn trait_hour(
    job: &mut CharactersJob,
    memory: &Flat<'_>,
    loan: &census::RoleCensus,
    captured: &mut Captured,
) -> (Duration, Duration) {
    let started = Instant::now();
    let report = copy_characters(memory, context(PART_MASKS[3]), job, Roles::Borrow(loan));
    let copy = started.elapsed();
    assert!(report.raw_ok && !report.plan_failed);
    let started = Instant::now();
    captured.decode_characters_payload(MODULE, job, &mut [0; N]);
    captured.remember_templates(job);
    let decode = started.elapsed();
    if job.grow_for_next() {
        job.grow();
    }
    job.prepare_read_plans();
    job.clear();
    (copy, decode)
}

#[test]
#[ignore = "bench: run in release with --ignored --nocapture"]
fn trait_hour_costs_at_late_war_scale() {
    let _globals = crate::extended::characters::test_globals();
    use super::leader_carry::{TRAITS as FAMILY, TraitCarry};
    static TRAITS: collector_observers::unit_leader_changes::Watches =
        collector_observers::unit_leader_changes::Watches::new();
    static UNOBSERVED: collector_observers::unit_leader_changes::Watches =
        collector_observers::unit_leader_changes::Watches::new();
    const HOURS: usize = 15;
    let world = World::new(13_200);
    let memory = Flat::new(&world);
    TRAITS.enable(world.characters);
    let owner = super::census_tests::roster(&memory);
    let loan = owner.role_census().unwrap();
    let mut captured = Captured::with_capacities(&capacities());
    for (label, changes) in [("carrying", &TRAITS), ("reading every leader", &UNOBSERVED)] {
        let mut job = CharactersJob::new_part(3);
        job.trait_carry = TraitCarry::with(changes, FAMILY);
        for _ in 0..5 {
            trait_hour(&mut job, &memory, &loan, &mut captured);
        }
        let runs: Vec<(Duration, Duration)> = (0..HOURS)
            .map(|_| trait_hour(&mut job, &memory, &loan, &mut captured))
            .collect();
        eprintln!(
            "traits {label}: copy {:?}, decode {:?}",
            median(runs.iter().map(|run| run.0).collect()),
            median(runs.iter().map(|run| run.1).collect()),
        );
    }
}

#[test]
#[ignore = "bench: run in release with --ignored --nocapture"]
fn advisor_hour_costs_at_late_war_scale() {
    let _globals = crate::extended::characters::test_globals();
    use super::census_tests::Advisors;
    static DOMAIN: collector_observers::character_changes::Counting =
        collector_observers::character_changes::Counting::new();
    const HOURS: usize = 15;
    let world = World::new(13_200);
    let memory = Flat::new(&world);
    DOMAIN.watches.enable(world.characters);
    let owner = super::census_tests::roster(&memory);
    let loan = owner.role_census().unwrap();
    let ctx = context(PART_MASKS[2]);
    let observer = Advisors {
        revision: Cell::new(0),
    };
    let demand =
        cadvisor::modifier_demand(&memory, MODULE, super::census_tests::GAME_STATE).unwrap();
    let mut cache = advisors::Cache::with_modifier_demand(demand);
    cache.observe_roles(&DOMAIN);
    let mut held = Captured::with_capacities(&capacities());
    let hour = |cache: &mut advisors::Cache, frame: &mut Captured| {
        let started = Instant::now();
        cache.capture(
            &memory,
            ctx,
            &observer,
            loan.admitted(MODULE, super::census_tests::GAME_STATE),
        );
        let copy = started.elapsed();
        if cache.needs_growth() {
            cache.grow();
        }
        cache.prepare();
        let started = Instant::now();
        frame.reset();
        cache.decode(frame);
        (copy, started.elapsed())
    };
    for _ in 0..5 {
        hour(&mut cache, &mut held);
    }
    let kept: Vec<(Duration, Duration)> = (0..HOURS).map(|_| hour(&mut cache, &mut held)).collect();
    let fresh: Vec<(Duration, Duration)> = (0..HOURS)
        .map(|_| hour(&mut cache, &mut Captured::with_capacities(&capacities())))
        .collect();
    eprintln!(
        "advisors quiet: copy {:?}, decode {:?} into the held frame, {:?} into a fresh frame",
        median(kept.iter().map(|run| run.0).collect()),
        median(kept.iter().map(|run| run.1).collect()),
        median(fresh.iter().map(|run| run.1).collect()),
    );
}

#[test]
#[ignore = "bench: run in release with --ignored --nocapture"]
fn portrait_hour_costs_at_late_war_scale() {
    let _globals = crate::extended::characters::test_globals();
    use crate::extended::character_portraits::{self as portraits, CaptureContext, PARTS};
    struct Quiet;
    impl engine_native_politics::portrait_cache::Changes for Quiet {
        fn revision(&self, _: usize) -> Option<(u64, u64)> {
            Some((1, 0))
        }
        fn quiet_revision(&self) -> Option<(u64, u64)> {
            Some((1, 0))
        }
    }
    const HOURS: usize = 15;
    let world = World::new(13_200);
    let memory = Flat::new(&world);
    let owner = super::census_tests::roster(&memory);
    let state = super::census_tests::GAME_STATE;
    let rows = owner
        .portrait_census()
        .unwrap()
        .rows(MODULE, state)
        .unwrap()
        .to_vec();
    let acquire = |census: &mut engine_native::ccharacter_manager::PortraitCensus| {
        census.begin(MODULE, state);
        for row in &rows {
            census.push(*row);
        }
        census.finish(rows.len());
    };
    let mut census =
        engine_native::ccharacter_manager::PortraitCensus::try_new(rows.len()).unwrap();
    acquire(&mut census);
    let context = CaptureContext {
        base: MODULE,
        game_state: state,
    };
    let mut capacities = portraits::Capacities::default();
    capacities.presize(&memory, MODULE, state).unwrap();
    let mut capture = portraits::Captured::with_capacities(&capacities);
    let mut hour = |census: &engine_native::ccharacter_manager::PortraitCensus| {
        let started = Instant::now();
        for part in 0..PARTS {
            let mut share = capture.take_part(part);
            share.capture_observed(&memory, context, part, Some(census), &Quiet);
            capture.restore_part(part, share);
        }
        started.elapsed()
    };
    for _ in 0..5 {
        hour(&census);
    }
    let kept: Vec<Duration> = (0..HOURS).map(|_| hour(&census)).collect();
    let fresh: Vec<Duration> = (0..HOURS)
        .map(|_| {
            acquire(&mut census);
            hour(&census)
        })
        .collect();
    eprintln!(
        "portraits quiet: {:?} with the matched stamp, {:?} comparing a fresh census",
        median(kept),
        median(fresh),
    );
}
