use super::super::*;
use super::*;
use crate::{
    carmy::organisation_movement::{LOW_ORGANISATION_FOR_ATTACK_RVA, land_speed_demand},
    memory::{CountedMemory, ReadCounts},
};

fn below_threshold(current: i64, maximum: i64, threshold: i64) -> Result<bool, &'static str> {
    let share = if (current as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        MOVEMENT_ONE
    } else {
        cunit::eta_ratio(current, maximum)?.clamp(0, MOVEMENT_ONE)
    };
    Ok(share < threshold)
}

fn land_province(
    memory: &impl Memory,
    base: usize,
    province: usize,
) -> Result<(u32, u8), &'static str> {
    let definition = movement_definition(memory, base, province)?;
    Ok((
        presentation_field(memory, province, cprovince::ID)?,
        presentation_field::<u8>(
            memory,
            definition,
            crate::cprovince::definition::TERRAIN_FLAGS,
        )?,
    ))
}

fn capture_serial(
    packet: &mut StatusPacket,
    memory: &impl Memory,
    base: usize,
    state: usize,
    seeds: &[DivisionSeed],
) -> u64 {
    let mut refused = 0;
    let mut threshold = None;
    for index in 0..packet.units.len() {
        let value = (|| -> Result<Option<[i64; 2]>, &'static str> {
            let seed = seeds.get(index).ok_or("organisation census missing")?;
            let route = seed
                .ordinary_route
                .and_then(|h| Array::from_header(h, 4))
                .ok_or("organisation movement route unreadable")?;
            if route.count == 0 {
                return Ok(None);
            }
            if seed
                .naval_transfer_membership
                .is_some_and(|value| value != 0)
            {
                return Ok(None);
            }
            let location = seed.location.ok_or("organisation location unreadable")?;
            let (ordinary, route_refused) = land_speed_demand(
                land_province(memory, base, location),
                (0..route.count).map(|index| {
                    movement_route_province(memory, base, state, route, index)
                        .and_then(|province| land_province(memory, base, province))
                }),
            );
            refused += route_refused.saturating_sub(u64::from(ordinary.is_err()));
            if !ordinary? {
                return Ok(None);
            }
            seed.naval_transfer_membership
                .ok_or("organisation transfer unreadable")?;
            if seed
                .strategic_redeployment
                .ok_or("organisation redeployment unreadable")?
                != 0
            {
                let roster = || seed.roster_owner.ok_or("organisation roster unreadable");
                let expeditionary = seed
                    .expeditionary_owner
                    .ok_or("organisation owner unreadable")?;
                let raw = if expeditionary as i32 > 0 {
                    expeditionary
                } else {
                    roster()?
                };
                let owner = ccountry::Reader::new(memory, base, state)?.resolve(raw)?;
                let inputs = strategic_action::read_inputs(
                    memory,
                    base,
                    state,
                    raw,
                    owner.pointer,
                    roster,
                    || movement_pointer(memory, location, cprovince::STATE),
                )
                .map_err(|failure| failure.reason())?;
                if !inputs.blocked() {
                    return Ok(None);
                }
            }
            let current = seed
                .organisation
                .ok_or("movement organisation unreadable")?;
            let maximum = packet
                .unit_maxima(index)
                .ok_or("movement maximum organisation unreadable")?
                .effective()
                .1;
            let limit = *threshold.get_or_insert_with(|| {
                presentation_global(memory, base, LOW_ORGANISATION_FOR_ATTACK_RVA)
            });
            if !below_threshold(current, maximum, limit?)? {
                return Ok(None);
            }
            if !movement_attacking(memory, base, state, seed.pointer)? {
                return Ok(None);
            }
            Ok(Some([current, maximum]))
        })();
        refused += u64::from(value.is_err());
        packet.units[index].movement_organisation = value.ok();
    }
    refused
}

fn fixture(count: usize) -> (PresentationMemory, Vec<DivisionSeed>) {
    let mut source = normal_route_fixture();
    p64(
        &mut source.bytes,
        0x11000 + crate::csub_unit_definition::MAXIMUM_ORGANISATION,
        1_000_000,
    );
    p64(&mut source.bytes, A1 + ORGANISATION, 1_000_000);
    let mut seed = [DivisionSeed {
        pointer: A1,
        division: presentation_source()[0].division,
        ..Default::default()
    }];
    capture_division_seeds(&source, &mut seed);
    let seeds = (0..count)
        .map(|index| {
            let division = ((index as u64 + 1) << 32) | 9;
            DivisionSeed {
                division,
                handle: Some(division),
                ..seed[0]
            }
        })
        .collect();
    (source, seeds)
}

fn packet(source: &impl Memory, seeds: &[DivisionSeed]) -> StatusPacket {
    let mut packet = StatusPacket::new(seeds.len()).unwrap();
    assert_eq!(
        packet.capture(
            source,
            0,
            GS,
            seeds,
            Outcome::Rows { len: seeds.len() },
        ),
        Outcome::Rows { len: seeds.len() }
    );
    packet
}

fn projected(packet: &StatusPacket) -> Vec<StatusRow> {
    let mut rows = vec![StatusRow::default(); packet.units.len()];
    assert_eq!(
        project_status(packet, &mut rows),
        Outcome::Rows { len: rows.len() }
    );
    rows
}

fn compare(
    source: &PresentationMemory,
    seeds: &[DivisionSeed],
    fault: Option<usize>,
) -> (ReadCounts, ReadCounts) {
    let mut serial = packet(source, seeds);
    let mut batched = packet(source, seeds);
    let old = CountedMemory::new(ObservedMovementMemory {
        source,
        reads: Default::default(),
        fault,
    });
    let new = CountedMemory::new(ObservedMovementMemory {
        source,
        reads: Default::default(),
        fault,
    });
    ReadCounts::take_current();
    let expected = capture_serial(&mut serial, &old, 0, GS, seeds);
    let old_counts = ReadCounts::take_current();
    let actual = batched.capture_movement_organisation(&new, 0, GS, seeds);
    let new_counts = ReadCounts::take_current();
    assert_eq!(actual, expected, "fault {fault:?}");
    assert_eq!(projected(&batched), projected(&serial), "fault {fault:?}");
    if fault.is_none() {
        let previous = old.0.reads.borrow();
        assert!(new.0.reads.borrow().iter().all(|at| previous.contains(at)));
        assert!(new_counts.bytes <= old_counts.bytes);
    }
    (old_counts, new_counts)
}

#[test]
fn division_status_organisation_batches_keep_every_field_fault_and_later_edge() {
    let (mut source, mut seeds) = fixture(3);
    use crate::{cdiplomacy_status as diplomacy, crelation::status as relation};
    const DIPLOMACY: usize = 0x2d000;
    const RELATIONS: usize = 0x2d200;
    const RELATION: usize = 0x2d300;
    const WAR: usize = 0x2d500;
    let b = &mut source.bytes;
    p32(b, 0x18400 + cprovince::CONTROLLER, 2);
    p64(b, C2 + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64);
    p64(b, DIPLOMACY, diplomacy::VTABLE_RVA.0 as u64);
    p64(b, DIPLOMACY + diplomacy::COUNTRY, C2 as u64);
    array(b, DIPLOMACY + diplomacy::RELATIONS, RELATIONS, 4);
    p64(b, RELATIONS + 8, RELATION as u64);
    p64(b, RELATION, relation::VTABLE_RVA.0 as u64);
    p64(b, RELATION + relation::DIPLOMACY_STATUS, DIPLOMACY as u64);
    p32(b, RELATION + relation::TARGET_COUNTRY, 3);
    p64(b, RELATION + relation::WAR_RELATION, WAR as u64);
    p64(b, WAR, crate::cwar_relation::VTABLE_RVA.0 as u64);
    p32(
        b,
        WAR + crate::crelation::TYPE_ID,
        crate::cwar_relation::TYPE,
    );
    array(b, A1 + cunit::ORDINARY_ROUTE_DATA, 0x1f000, 3);
    p32(b, 0x1f004, 2);
    p32(b, 0x1f008, 3);
    let header = b[A1 + cunit::ORDINARY_ROUTE_DATA..A1 + cunit::ORDINARY_ROUTE_DATA + 16]
        .try_into()
        .unwrap();
    for seed in &mut seeds {
        seed.organisation = Some(0);
        seed.ordinary_route = Some(header);
    }
    let mut serial = packet(&source, &seeds);
    let observed = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: None,
    };
    assert_eq!(capture_serial(&mut serial, &observed, 0, GS, &seeds), 0);
    assert!(
        serial
            .units
            .iter()
            .all(|unit| { unit.movement_organisation == Some(Some([0, 1_000_000])) })
    );
    let addresses: std::collections::BTreeSet<_> =
        observed.reads.into_inner().into_iter().collect();
    for fault in addresses {
        compare(&source, &seeds, Some(fault));
    }
    for flags in [
        0,
        crate::cprovince::definition::LAND_FLAG,
        crate::cprovince::definition::LAKE_FLAG,
    ] {
        source.bytes[0x12000 + crate::cprovince::definition::TERRAIN_FLAGS] = flags;
        for first in [2, 99] {
            p32(&mut source.bytes, 0x1f000, first);
            compare(&source, &seeds, None);
            if first == 99 && flags != 0 {
                let mut reference = packet(&source, &seeds);
                assert_eq!(
                    capture_serial(&mut reference, &source, 0, GS, &seeds),
                    seeds.len() as u64
                );
                assert!(
                    reference
                        .units
                        .iter()
                        .all(|unit| { unit.movement_organisation == Some(Some([0, 1_000_000])) })
                );
            }
        }
    }
}

#[test]
fn division_status_organisation_batches_keep_identity_and_suffix_failures_independent() {
    let (mut source, seeds) = fixture(1);
    for (at, value) in [
        (P1, 0),
        (P1 + cprovince::DEFINITION, 0),
        (P1 + cprovince::ID, 9),
        (0x12000 + crate::cprovince::definition::ID, 9),
    ] {
        let old = source.bytes[at..at + 8].to_vec();
        p64(&mut source.bytes, at, value);
        compare(
            &source,
            &seeds,
            Some(0x12000 + crate::cprovince::definition::TERRAIN_FLAGS),
        );
        source.bytes[at..at + 8].copy_from_slice(&old);
    }
    p64(&mut source.bytes, PROVINCES + 2 * 8, P1 as u64);
    compare(
        &source,
        &seeds,
        Some(0x12000 + crate::cprovince::definition::TERRAIN_FLAGS),
    );
}

struct Native<'a> {
    source: &'a PresentationMemory,
    live: crate::memory::LiveMemory,
}
impl Native<'_> {
    fn address(&self, address: usize, len: usize) -> usize {
        if let Some((&start, bytes)) = self.source.globals.range(..=address).next_back()
            && let Some(bytes) = address
                .checked_sub(start)
                .and_then(|at| bytes.get(at..at.checked_add(len)?))
        {
            return bytes.as_ptr() as usize;
        }
        self.source
            .bytes
            .get(address..address.saturating_add(len))
            .map_or(0, |bytes| bytes.as_ptr() as usize)
    }
}
impl Memory for Native<'_> {
    fn read_scalar<T: crate::memory::Raw>(&self, address: usize) -> Option<T> {
        self.live
            .read_scalar(self.address(address, std::mem::size_of::<T>()))
    }
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        self.live.copy(self.address(address, out.len()), out)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        out: &mut [&mut [u8]; N],
    ) -> usize {
        let addresses =
            std::array::from_fn(|index| self.address(addresses[index], out[index].len()));
        self.live.copy_scattered_prefetched(addresses, out)
    }
}

#[test]
#[ignore = "synthetic native-read timing; run explicitly with --nocapture"]
fn bench_division_status_organisation_guarded_reads() {
    use std::{hint::black_box, time::Instant};
    const DIVISIONS: usize = 5_000;
    let (source, seeds) = fixture(DIVISIONS);
    let memory = CountedMemory::new(Native {
        source: &source,
        live: unsafe { crate::memory::LiveMemory::new() },
    });
    let mut capture = packet(&source, &seeds);
    let mut rows = Vec::new();
    for (serial, capacity) in [(true, 0), (false, 0), (false, DIVISIONS)] {
        capture.organisation_memo = organisation_movement::OrganisationMemo::new(capacity).unwrap();
        let mut samples = Vec::new();
        let mut counts = ReadCounts::ZERO;
        for run in 0..28 {
            ReadCounts::take_current();
            let begin = Instant::now();
            let refused = if serial {
                capture_serial(&mut capture, &memory, 0, GS, &seeds)
            } else {
                capture.capture_movement_organisation(&memory, 0, GS, &seeds)
            };
            let elapsed = begin.elapsed().as_nanos() as f64 / DIVISIONS as f64;
            counts = ReadCounts::take_current();
            assert_eq!(refused, 0);
            black_box(&capture);
            if run >= 3 {
                samples.push(elapsed);
            }
        }
        rows.push(projected(&capture));
        samples.sort_by(f64::total_cmp);
        println!(
            "DIVISION_STATUS_ORGANISATION serial={serial} memo_capacity={capacity} divisions={DIVISIONS} p50_ns_per_division={:.1} copy_calls={} scattered_calls={} bytes={}",
            samples[samples.len() / 2],
            counts.copy_calls,
            counts.scattered_calls,
            counts.bytes
        );
    }
    assert_eq!(rows[0], rows[1]);
    assert_eq!(rows[0], rows[2]);

    let words = vec![7_u64; DIVISIONS];
    let live = CountedMemory::new(unsafe { crate::memory::LiveMemory::new() });
    let mut samples = Vec::new();
    for run in 0..28 {
        let begin = Instant::now();
        for index in 0..words.len() {
            let _ = black_box(read::<u64>(&live, words.as_ptr() as usize + index * 8));
        }
        if run >= 3 {
            samples.push(begin.elapsed().as_nanos() as f64 / DIVISIONS as f64);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "DIVISION_STATUS_GUARDED_WORD warm_p50_ns_per_read={:.1}",
        samples[samples.len() / 2]
    );
}

#[test]
fn division_status_organisation_memo_pressure_and_new_pass_preserve_rows() {
    let (mut source, seeds) = fixture(5);
    for capacity in [0, 1, 5] {
        let mut expected = packet(&source, &seeds);
        let mut actual = packet(&source, &seeds);
        actual.organisation_memo = organisation_movement::OrganisationMemo::new(capacity).unwrap();
        let allocated = actual.allocated_bytes();
        for flags in [
            crate::cprovince::definition::LAND_FLAG,
            0,
            crate::cprovince::definition::LAKE_FLAG,
        ] {
            source.bytes[0x12000 + crate::cprovince::definition::TERRAIN_FLAGS] = flags;
            let old = capture_serial(&mut expected, &source, 0, GS, &seeds);
            let new = actual.capture_movement_organisation(&source, 0, GS, &seeds);
            assert_eq!(new, old);
            assert_eq!(projected(&actual), projected(&expected));
            assert_eq!(actual.allocated_bytes(), allocated);
        }
    }
}

#[test]
fn division_status_organisation_transient_faults_do_not_poison_later_divisions() {
    use std::cell::Cell;
    struct Once<'a> {
        source: &'a PresentationMemory,
        fault: usize,
        spent: Cell<bool>,
    }
    impl Memory for Once<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            if address == self.fault && !self.spent.replace(true) {
                return false;
            }
            self.source.copy(address, out)
        }
    }
    let (source, seeds) = fixture(3);
    for fault in [
        P1,
        P1 + cprovince::DEFINITION,
        P1 + cprovince::ID,
        0x12000 + crate::cprovince::definition::ID,
        0x12000 + crate::cprovince::definition::TERRAIN_FLAGS,
        GS + cgame_state::PROVINCES,
    ] {
        let mut expected = packet(&source, &seeds);
        let mut actual = packet(&source, &seeds);
        let old = capture_serial(
            &mut expected,
            &Once {
                source: &source,
                fault,
                spent: Cell::new(false),
            },
            0,
            GS,
            &seeds,
        );
        let new = actual.capture_movement_organisation(
            &Once {
                source: &source,
                fault,
                spent: Cell::new(false),
            },
            0,
            GS,
            &seeds,
        );
        assert_eq!(new, old, "{fault:#x}");
        assert_eq!(projected(&actual), projected(&expected), "{fault:#x}");
        assert!(
            actual.units[1..]
                .iter()
                .all(|unit| unit.movement_organisation.is_some())
        );
    }
}

#[test]
fn division_status_organisation_optional_storage_failure_is_nonterminal() {
    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
    static FAILED: AtomicBool = AtomicBool::new(false);
    fn failed() {
        FAILED.store(true, Relaxed);
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            crate::allocation::set_hooks(None, None);
        }
    }
    let _reset = Reset;
    crate::allocation::set_hooks(None, Some(failed));
    assert!(organisation_movement::OrganisationMemo::new(usize::MAX).is_err());
    assert!(!FAILED.load(Relaxed));
    drop(crate::allocation::oom());
    assert!(FAILED.load(Relaxed));
}
