use super::*;
use std::cell::Cell;

const SLOTS: usize = 0x1000;
const OBJECTS: usize = 0x20000;
struct Source<'a> {
    bytes: &'a [u8],
    fault: Option<usize>,
    once: bool,
    spent: Cell<bool>,
}
impl Memory for Source<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        if self
            .fault
            .is_some_and(|fault| at <= fault && fault - at < out.len())
            && (!self.once || !self.spent.replace(true))
        {
            out.fill(0xff);
            return false;
        }
        engine_native::memory::SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(at, out)
    }
}
struct Reads<'a> {
    source: Source<'a>,
    requests: Cell<usize>,
}
impl Memory for Reads<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        self.requests.set(self.requests.get() + 1);
        self.source.copy(at, out)
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        out: &mut [u8],
    ) -> usize {
        self.requests.set(self.requests.get() + 1);
        self.source.copy_records(sources, layout, out)
    }
}
fn world(countries: usize) -> (Vec<u8>, Vec<carmy::DivisionSeed>) {
    let mut bytes = vec![0; OBJECTS + countries * 0x100];
    let seeds = (0..countries)
        .map(|index| {
            let pointer = OBJECTS + index * 0x100;
            bytes[pointer..pointer + 8]
                .copy_from_slice(&(cprovince::VTABLE_RVA.0 as u64).to_le_bytes());
            let at = pointer + cprovince::ID;
            bytes[at..at + 4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
            let at = SLOTS + (index + 1) * 8;
            bytes[at..at + 8].copy_from_slice(&(pointer as u64).to_le_bytes());
            carmy::DivisionSeed {
                division: ((index as u64 + 1) << 32) | 9,
                roster_owner: Some(index as u32 + 1),
                location: Some(pointer),
                ..Default::default()
            }
        })
        .collect();
    (bytes, seeds)
}
#[derive(Debug, PartialEq)]
struct Snapshot {
    ids: Vec<u32>,
    failure: Option<&'static str>,
    memo: Vec<(usize, u64)>,
}
fn capture(
    bytes: &[u8],
    seeds: &[carmy::DivisionSeed],
    batched: bool,
    fault: Option<usize>,
    once: bool,
) -> (Snapshot, usize) {
    let memory = Reads {
        source: Source {
            bytes,
            fault,
            once,
            spent: Cell::new(false),
        },
        requests: Cell::new(0),
    };
    let mut identities = CoreIdentities::new(
        0,
        Array {
            data: SLOTS,
            count: seeds
                .iter()
                .map(|seed| (seed.division >> 32) as u32)
                .max()
                .unwrap_or(0)
                + 1,
        },
    );
    let mut ids = Vec::new();
    let mut failure = None;
    'walk: for chunk in seeds.chunks(64) {
        if batched {
            identities.stage_provinces(&memory, chunk);
        }
        for seed in chunk {
            match identities.province(&memory, seed.location.unwrap_or(0)) {
                Ok(id) => ids.push(id),
                Err(reason) => {
                    failure = Some(reason);
                    break 'walk;
                }
            }
        }
    }
    (
        Snapshot {
            ids,
            failure,
            memo: identities
                .locations
                .iter()
                .map(|entry| (entry.pointer, entry.value))
                .collect(),
        },
        memory.requests.get(),
    )
}
#[test]
fn batched_core_province_persistent_faults_keep_serial_reason_and_prefix() {
    let (bytes, seeds) = world(65);
    let pointer = seeds[20].location.unwrap();
    for at in [pointer, pointer + cprovince::ID, SLOTS + 21 * 8] {
        assert_eq!(
            capture(&bytes, &seeds, true, Some(at), false).0,
            capture(&bytes, &seeds, false, Some(at), false).0
        );
    }
}
#[test]
fn batched_core_province_one_shot_refuses_no_more_and_counts_fallback() {
    let (bytes, seeds) = world(65);
    for at in [seeds[20].location.unwrap() + cprovince::ID, SLOTS + 21 * 8] {
        let before = engine_native::batch_fallback::count(
            engine_native::batch_fallback::Batch::DivisionProvinces,
        );
        let actual = capture(&bytes, &seeds, true, Some(at), true).0;
        assert_eq!(
            engine_native::batch_fallback::count(
                engine_native::batch_fallback::Batch::DivisionProvinces
            ) - before,
            1
        );
        let serial = capture(&bytes, &seeds, false, Some(at), true).0;
        assert!(actual.ids.len() >= serial.ids.len());
        assert_eq!(actual, capture(&bytes, &seeds, false, None, false).0);
    }
}

fn serial_core(census: &mut Census, memory: &impl Memory, limit: usize) {
    let roots = census.core_roots.take().unwrap();
    let result = (|| {
        let provinces = Array::read(memory, roots.state + cgame_state::PROVINCES, 8)
            .ok_or("province array unreadable or invalid")?;
        for index in 0..census.entries.len().min(limit) {
            let mut theatres = ctheatre::UnitTheatres::default();
            let row = project(
                memory,
                &roots.countries,
                census.entries[index],
                &census.unit_inputs.as_ref().unwrap().seeds[index],
                &mut CoreIdentities::new(roots.base, provinces),
                &mut TheatreLookup::new(&mut theatres),
            )?;
            Census::count_division(
                &mut census.counts,
                census.required_counts,
                &mut census.counts_error,
                &row,
            );
            census.sources.push(DivisionSource {
                pointer: census.entries[index].pointer,
                division: row.id,
                template: row.template,
                country: row.roster_owner,
            });
            census.rows.push(row);
        }
        Ok::<_, &'static str>(())
    })();
    census.row_error = result.err();
    census.core_captured = true;
}

#[test]
fn batched_core_projection_alias_duplicates_and_faults_equal_serial() {
    use crate::census_test_fixture as kit;
    let mut bytes = kit::fixture();
    for offset in [8, 12] {
        let at = kit::C0 + ccountry::DIVISIONS + offset;
        bytes[at..at + 4].copy_from_slice(&65_u32.to_le_bytes());
    }
    let roster = 0x25000;
    let at = kit::C0 + ccountry::DIVISIONS;
    bytes[at..at + 8].copy_from_slice(&(roster as u64).to_le_bytes());
    for index in 0..65 {
        let pointer = [kit::A1, kit::A2][index % 2];
        bytes[roster + index * 8..roster + index * 8 + 8]
            .copy_from_slice(&(pointer as u64).to_le_bytes());
    }
    for pointer in [kit::A1, kit::A2] {
        let at = pointer + engine_native::cunit::ROSTER_OWNER;
        bytes[at..at + 4].copy_from_slice(&3_u32.to_le_bytes());
    }
    for invalid in 0..4 {
        let mut bytes = bytes.clone();
        if invalid == 1 {
            bytes[kit::P1..kit::P1 + 8].copy_from_slice(&0_u64.to_le_bytes());
        }
        for once in [false, true] {
            for fault in [
                None,
                Some(kit::LTAB + 12),
                Some(kit::P1),
                Some(kit::P1 + cprovince::ID),
                Some(kit::PTAB + 8),
            ] {
                let make_census = || {
                    let mut census = Census::new();
                    census.capture_inputs(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
                    assert_eq!(census.entries.len(), 66);
                    let inputs = census.unit_inputs.as_mut().unwrap();
                    let seeds =
                        &mut engine_allocation::SharedBox::get_mut(inputs).unwrap().seeds;
                    if invalid == 2 {
                        seeds[0].primary_vtable = Some(0);
                    } else if invalid == 3 {
                        seeds[0].roster_owner = Some(0);
                    }
                    census
                };
                let memory = || Source {
                    bytes: &bytes,
                    fault,
                    once,
                    spent: Cell::new(false),
                };
                let mut actual = make_census();
                let mut serial = make_census();
                let allocated = actual.allocated_bytes();
                actual.capture_core(&memory());
                serial_core(&mut serial, &memory(), usize::MAX);
                if once && fault.is_some() {
                    assert!(actual.rows.len() >= serial.rows.len());
                    assert_eq!(&actual.rows[..serial.rows.len()], serial.rows);
                    assert_eq!(&actual.sources[..serial.sources.len()], serial.sources);
                    if actual.rows.len() == serial.rows.len() {
                        assert_eq!(actual.outcome(), serial.outcome());
                    }
                    let mut healthy_prefix = make_census();
                    serial_core(&mut healthy_prefix, &kit::memory(&bytes), actual.rows.len());
                    assert_eq!(actual.rows, healthy_prefix.rows);
                    assert_eq!(actual.sources, healthy_prefix.sources);
                    assert_eq!(actual.counts, healthy_prefix.counts);
                    assert_eq!(actual.counts_error, healthy_prefix.counts_error);
                } else {
                    assert_eq!(actual.rows, serial.rows);
                    assert_eq!(actual.sources, serial.sources);
                    assert_eq!(actual.counts, serial.counts);
                    assert_eq!(actual.counts_error, serial.counts_error);
                    assert_eq!(actual.outcome(), serial.outcome());
                }
                assert_eq!(
                    actual.unit_inputs().unwrap().seeds(),
                    serial.unit_inputs().unwrap().seeds()
                );
                assert_eq!(actual.source_requirement(), serial.source_requirement());
                assert_eq!(actual.required_counts, serial.required_counts);
                assert_eq!(actual.allocated_bytes(), allocated);
            }
        }
    }
}
#[test]
fn batched_core_province_request_count_is_two_per_sixty_four_misses() {
    let (bytes, seeds) = world(129);
    let actual = capture(&bytes, &seeds, true, None, false);
    assert_eq!(actual.0.ids.len(), 129);
    assert_eq!(actual.1, 6, "old code used 258 guarded requests");
}

#[test]
fn batched_core_province_collision_revisit_duplicates_and_null_equal_serial() {
    let (bytes, all) = world(512);
    let first = all[0];
    let collision = *all
        .iter()
        .skip(1)
        .find(|seed| {
            CoreIdentities::slot(seed.location.unwrap())
                == CoreIdentities::slot(first.location.unwrap())
        })
        .unwrap();
    let seeds = [
        first,
        collision,
        first,
        first,
        carmy::DivisionSeed::default(),
    ];
    let actual = capture(&bytes, &seeds, true, None, false);
    let serial = capture(&bytes, &seeds, false, None, false);
    assert_eq!(actual.0, serial.0);
    assert_eq!(actual.0.ids.last(), Some(&0));
    assert_eq!(
        (actual.1, serial.1),
        (2, 6),
        "a colliding revisit still uses its staged identity"
    );
    for fault in [
        collision.location.unwrap(),
        collision.location.unwrap() + cprovince::ID,
        SLOTS + (collision.division >> 32) as usize * 8,
    ] {
        assert_eq!(
            capture(&bytes, &seeds, true, Some(fault), false).0,
            capture(&bytes, &seeds, false, Some(fault), false).0
        );
        let actual = capture(&bytes, &seeds, true, Some(fault), true).0;
        let serial = capture(&bytes, &seeds, false, Some(fault), true).0;
        assert!(actual.ids.len() >= serial.ids.len());
        assert_eq!(actual, capture(&bytes, &seeds, false, None, false).0);
    }
}
