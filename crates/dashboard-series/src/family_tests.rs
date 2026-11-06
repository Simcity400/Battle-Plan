use crate::{
    carry::{Carry, Read},
    codec::Binary,
    family::{Actors, Directory, Place, Scalar},
    memory::{Memory, publish},
    query::Snapshot,
    *,
};
use std::collections::BTreeMap;

const SCHEMA: u64 = 11;
const DOMAIN: u64 = 0;

fn identity() -> Identity {
    Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: 3,
    }
}

fn span(first: i32, last: i32) -> Span {
    Span::new(first, last).unwrap()
}

/// What one key's runs should be: (first, last, value, break), equal values joined.
#[derive(Default)]
struct Model {
    runs: Vec<(i32, i32, i128, bool)>,
    open: Option<(i32, i128, bool)>,
}
impl Model {
    fn value(&mut self, hour: i32, value: Option<i128>, broken: bool) {
        if let (Some((_, held, _)), Some(value)) = (self.open, value)
            && held == value
            && !broken
        {
            return;
        }
        self.end(hour - 1);
        self.open = value.map(|value| (hour, value, broken));
    }
    fn end(&mut self, last: i32) {
        if let Some((first, value, broken)) = self.open.take()
            && first <= last
        {
            self.runs.push((first, last, value, broken));
        }
    }
}

fn next(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed >> 33
}

/// 1,500 actors with two measures each walked over 300 hours, cut at irregular hours (one of
/// them right after another), resumed from the publication and the saved carry at one cut, a
/// gap closing every run: each key's runs read back equal the model, equal values continuing
/// across every cut, and a cut appends only the keys open.
#[test]
fn carried_edges_join_runs_across_cuts_and_resume_and_read_back_by_place() {
    let mut pages = Memory::default();
    let mut writer = Writer::new(identity(), 0).unwrap();
    let mut actors = Actors::new(SCHEMA);
    let mut carry = Carry::default();
    let mut models: BTreeMap<u64, Model> = BTreeMap::new();
    // Each evidence key's reads as set, by hour; a gap ends them.
    let mut reads: BTreeMap<u64, Vec<(i32, Option<i32>)>> = BTreeMap::new();
    let cuts = [23, 47, 48, 95, 131, 199, 299];
    let gap = 160..=169;
    let mut root = 0;
    let mut seed = 0x9e37_79b9_7f4a_7c15;
    let mut generation = 0;
    let mut start = 0;
    let mut last_read = None;
    for &end in &cuts {
        for hour in start..=end {
            if gap.contains(&hour) {
                continue;
            }
            if last_read.is_some_and(|last: i32| last + 1 != hour) {
                carry.close(&mut writer, last_read.unwrap()).unwrap();
                for model in models.values_mut() {
                    model.end(last_read.unwrap());
                }
                for log in reads.values_mut() {
                    log.push((last_read.unwrap() + 1, None));
                }
            }
            last_read = Some(hour);
            // Actors join over the first hours: a few at each.
            let joined = (1500 * (hour + 1) / 20).min(1500) as u64;
            for actor in 0..joined {
                let name = format!("C{actor:04}");
                let id = actors.actor(&mut writer, name.as_bytes(), hour).unwrap();
                assert_eq!(id, actor);
                for measure in 0..2 {
                    let place = Place {
                        actor,
                        measure,
                        dimension: None,
                    };
                    let key = actors
                        .key(
                            &mut writer,
                            place,
                            (RunMode::Exact, 0),
                            (measure == 0).then_some(DOMAIN),
                        )
                        .unwrap();
                    let roll = next(&mut seed) % 40;
                    let fresh = models.get(&key).is_none_or(|model| model.open.is_none());
                    let (value, broken) = match roll {
                        0 => (None, false),
                        1 => (Some((next(&mut seed) % 3) as i128), true),
                        2..=4 => (Some((next(&mut seed) % 3) as i128), false),
                        _ if fresh => (Some(1), true),
                        _ => continue,
                    };
                    carry.value(&mut writer, key, hour, value, broken).unwrap();
                    models.entry(key).or_default().value(hour, value, broken);
                    if measure == 1 {
                        let read = value.map(|_| Read {
                            holder: Some(actor),
                            read: Some(hour - hour % 24),
                            identity_first: None,
                            presented: true,
                        });
                        carry.evidence(&mut writer, key, hour, read).unwrap();
                        reads
                            .entry(key)
                            .or_default()
                            .push((hour, read.and_then(|read| read.read)));
                    }
                }
            }
        }
        let open = carry.len();
        carry.cut(&mut writer, end).unwrap();
        assert!(carry.len() <= open);
        writer
            .domain(
                DOMAIN,
                Evidence {
                    span: span(start, end),
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )
            .unwrap();
        if end == 95 {
            let cell = crate::family::cell(&mut writer, &Scalar::Boolean(true)).unwrap();
            writer.catalogue(3, cell).unwrap();
        }
        generation += 1;
        root = publish(
            &mut writer,
            &mut pages,
            Publication {
                generation,
                segment: generation - 1,
                accepted: span(0, end),
                reached: end,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        if end == 131 {
            let mut saved = Vec::new();
            carry.save(&mut saved).unwrap();
            carry = Carry::load(&saved).unwrap();
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            actors = Actors::resume(SCHEMA, &directory).unwrap();
            assert_eq!(actors.len(), 1500);
        }
        start = end + 1;
    }
    for model in models.values_mut() {
        model.end(*cuts.last().unwrap());
    }
    let mut snapshot = Snapshot::open(pages, root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    assert_eq!(directory.actors.len(), 1500);
    assert_eq!(&*directory.actors[0].name, b"C0000");
    assert_eq!(directory.actors[0].first_hour, 0);
    assert_eq!(directory.actors[1499].first_hour, 19);
    assert_eq!(directory.scalar(3), Some(&Scalar::Boolean(true)));
    for (&key, model) in &models {
        let mut runs = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot
                .run_page(
                    query::Selection {
                        key,
                        membership: None,
                    },
                    span(0, 299),
                    cursor,
                    Run::SIZE * 7,
                    |run| {
                        runs.push((run.span.first, run.span.last, run.value, run.break_before));
                        Ok(())
                    },
                )
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(runs, model.runs, "key {key}");
    }
    let actor = directory.actor(b"C0007").unwrap();
    let key = directory.key(actor, 1, None).unwrap();
    for hour in [0, 30, 48, 131, 132, 170, 299] {
        let held = models[&key]
            .runs
            .iter()
            .find(|run| run.0 <= hour && hour <= run.1)
            .map(|run| run.2);
        match snapshot.value_at(key, hour).unwrap() {
            Observation::Value { run, evidence } => {
                assert_eq!(Some(run.value), held, "hour {hour}");
                assert_eq!(evidence.holder, Some(actor));
                let read = reads[&key]
                    .iter()
                    .rev()
                    .find(|(at, _)| *at <= hour)
                    .and_then(|(_, read)| *read);
                assert_eq!(evidence.supplying_read, read, "hour {hour}");
            }
            _ => assert_eq!(held, None, "hour {hour}"),
        }
    }
    let bound = directory.key(actor, 0, None).unwrap();
    let held = models[&bound]
        .runs
        .iter()
        .find(|run| run.0 <= 140 && 140 <= run.1)
        .map(|run| run.2);
    match snapshot.value_at(bound, 140).unwrap() {
        Observation::Value { run, .. } => assert_eq!(Some(run.value), held),
        Observation::KnownAbsent(_) => assert_eq!(held, None),
        other => panic!("domain-bound key unknown at a known hour: {other:?}"),
    }
}

/// A carry saved and loaded holds the same edges.
#[test]
fn a_saved_carry_loads_back_the_same() {
    let mut writer = Writer::new(identity(), 0).unwrap();
    let mut actors = Actors::new(SCHEMA);
    let mut carry = Carry::default();
    for actor in 0..5 {
        actors
            .actor(&mut writer, format!("T{actor}").as_bytes(), 0)
            .unwrap();
        let key = actors
            .key(
                &mut writer,
                Place {
                    actor,
                    measure: 0,
                    dimension: Some(actor * 3),
                },
                (RunMode::Exact, 5),
                None,
            )
            .unwrap();
        carry
            .value(
                &mut writer,
                key,
                0,
                Some(i128::from(actor) - 2),
                actor % 2 == 0,
            )
            .unwrap();
        if actor != 3 {
            carry
                .evidence(
                    &mut writer,
                    key,
                    0,
                    Some(Read {
                        holder: (actor != 1).then_some(actor << 32 | 9),
                        read: Some(-4),
                        identity_first: (actor == 2).then_some(0),
                        presented: actor == 4,
                    }),
                )
                .unwrap();
        }
    }
    carry.cut(&mut writer, 6).unwrap();
    let mut bytes = Vec::new();
    carry.save(&mut bytes).unwrap();
    assert_eq!(Carry::load(&bytes).unwrap(), carry);
    assert!(Carry::load(&bytes[..bytes.len() - 1]).is_err());
}

/// A supplied value starts a run of its own at its hour, even equal to the run before and
/// across a cut and a saved carry, while the equal values read between continue it.
#[test]
fn supplied_values_start_their_own_runs_across_a_cut_and_a_saved_carry() {
    let mut pages = Memory::default();
    let mut writer = Writer::new(identity(), 0).unwrap();
    let mut actors = Actors::new(SCHEMA);
    let mut carry = Carry::default();
    let actor = actors.actor(&mut writer, b"T", 0).unwrap();
    let place = Place {
        actor,
        measure: 0,
        dimension: None,
    };
    let key = actors
        .key(&mut writer, place, (RunMode::Exact, 0), None)
        .unwrap();
    let read = |hour: i32| {
        Some(Read {
            holder: Some(7),
            read: Some(hour),
            identity_first: Some(0),
            presented: false,
        })
    };
    let mut root = 0;
    for (generation, (first, last)) in [(0, 30), (31, 70)].into_iter().enumerate() {
        for hour in first..=last {
            match hour {
                0 | 24 | 48 => {
                    carry
                        .supplied(&mut writer, key, hour, 5, hour == 0)
                        .unwrap();
                    carry.evidence(&mut writer, key, hour, read(hour)).unwrap();
                }
                60 => {
                    carry.value(&mut writer, key, hour, None, false).unwrap();
                    carry.evidence(&mut writer, key, hour, None).unwrap();
                }
                _ if hour < 60 => {
                    assert!(!carry.value(&mut writer, key, hour, Some(5), false).unwrap());
                }
                _ => {}
            }
        }
        carry.cut(&mut writer, last).unwrap();
        let generation = generation as u64 + 1;
        root = publish(
            &mut writer,
            &mut pages,
            Publication {
                generation,
                segment: generation - 1,
                accepted: span(0, last),
                reached: last,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        let mut saved = Vec::new();
        carry.save(&mut saved).unwrap();
        carry = Carry::load(&saved).unwrap();
    }
    let mut snapshot = Snapshot::open(pages, root, identity()).unwrap();
    for (hour, expected) in [
        (10, Some((0, 23, true))),
        (30, Some((24, 47, false))),
        (31, Some((24, 47, false))),
        (59, Some((48, 59, false))),
        (60, None),
    ] {
        let found = match snapshot.value_at(key, hour).unwrap() {
            Observation::Value { run, evidence } => {
                assert_eq!(evidence.supplying_read, Some(run.span.first), "hour {hour}");
                Some((run.span.first, run.span.last, run.break_before))
            }
            _ => None,
        };
        assert_eq!(found, expected, "hour {hour}");
    }
}
