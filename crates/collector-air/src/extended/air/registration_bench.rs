use super::*;
use crate::extended::fog;
use collector_observers::air_base_carry_changes::Watches;
use engine_native_air::{
    cair_base,
    cair_wing,
    ccountry_air_container,
    cstrategic_air_manager,
};
use engine_native::{cair_wing_pool, ccountry, cgame_state, cprovince, cstate};
use std::hint::black_box;
use std::time::Instant;

#[cfg(test)]
const GAME_STATE: usize = 0x10000;
#[cfg(test)]
const MANAGER: usize = 0x20000;

#[cfg(test)]
struct Shape {
    name: &'static str,
    countries: u32,
    bases: usize,
    containers: usize,
    pools: usize,
    wings: usize,
}

#[cfg(test)]
struct World {
    bytes: Vec<u8>,
    next: usize,
}

#[cfg(test)]
impl World {
    #[cfg(test)]
    fn alloc(&mut self, len: usize) -> usize {
        let at = self.next;
        self.next += len.next_multiple_of(16);
        if self.bytes.len() < self.next {
            self.bytes.resize(self.next, 0);
        }
        at
    }

    #[cfg(test)]
    fn put32(&mut self, at: usize, value: u32) {
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[cfg(test)]
    fn put64(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    #[cfg(test)]
    fn header(&mut self, at: usize, data: usize, count: u32) {
        self.put64(at, data as u64);
        self.put32(at + 8, count);
        self.put32(at + 12, count);
    }
}

#[cfg(test)]
fn build(shape: &Shape) -> Vec<u8> {
    let mut world = World {
        bytes: vec![0; 0x30000],
        next: 0x30000,
    };
    let slots = shape.countries + 1;
    let table = world.alloc(slots as usize * 8);
    let links = world.alloc(slots as usize * 4);
    world.header(GAME_STATE + cgame_state::COUNTRIES, table, slots);
    world.header(GAME_STATE + cgame_state::COUNTRY_LINKS, links, slots);
    for slot in 1..slots {
        let country = world.alloc(ccountry::ACES + 16);
        world.put64(country, ccountry::VTABLE_RVA.0 as u64);
        world.put32(country + ccountry::ID, slot);
        world.put64(table + slot as usize * 8, country as u64);
        world.put32(links + slot as usize * 4, slot);
    }
    world.put64(
        GAME_STATE + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    world.put64(MANAGER, cstrategic_air_manager::VTABLE_RVA.0 as u64);
    let roster = world.alloc((shape.bases + 1) * 8);
    world.header(
        MANAGER + cstrategic_air_manager::STATE_AIR_BASES,
        roster,
        shape.bases as u32 + 1,
    );
    let mut wing_id = 1_u64;
    for index in 0..shape.bases {
        let id = index as u32 + 1;
        let base = world.alloc(cair_base::SIZE);
        let handle = (u64::from(id) << 32) | u64::from(cair_base::TYPE);
        let state = world.alloc(cstate::ID + 8);
        let province = world.alloc(cprovince::STATE + 8);
        world.put64(roster + id as usize * 8, base as u64);
        world.put64(base, cair_base::VTABLE_RVA.0 as u64);
        world.put64(base + cair_base::HANDLE, handle);
        world.put64(base + cair_base::STATE, state as u64);
        world.put64(base + cair_base::PROVINCE, province as u64);
        world.put32(base + cair_base::CAPACITY, 100);
        world.put64(state, cstate::VTABLE_RVA.0 as u64);
        world.put32(state + cstate::ID, id);
        world.put64(province, cprovince::VTABLE_RVA.0 as u64);
        world.put32(province + cprovince::ID, id);
        world.put64(province + cprovince::STATE, state as u64);
        let vector = world.alloc(slots as usize * 8);
        let dense = world.alloc(shape.containers * 8);
        world.header(base + cair_base::COUNTRY_CONTAINERS, vector, slots);
        world.header(
            base + cair_base::REGISTERED_CONTAINERS,
            dense,
            shape.containers as u32,
        );
        for entry in 0..shape.containers {
            let country = ((index * 7 + entry * 13) as u32 % shape.countries) + 1;
            let container = world.alloc(0xf8);
            world.put64(vector + country as usize * 8, container as u64);
            world.put64(dense + entry * 8, container as u64);
            world.put64(container, ccountry_air_container::VTABLE_RVA.0 as u64);
            world.put32(container + ccountry_air_container::COUNTRY, country);
            world.put64(container + ccountry_air_container::CAPACITY_RAW, 4);
            let pools = world.alloc(shape.pools * 8);
            world.header(
                container + ccountry_air_container::POOLS,
                pools,
                shape.pools as u32,
            );
            for pool_index in 0..shape.pools {
                let pool = world.alloc(0x40);
                world.put64(pools + pool_index * 8, pool as u64);
                world.put64(pool, cair_wing_pool::VTABLE_RVA.0 as u64);
                world.put64(pool + cair_wing_pool::AIR_BASE_REFERENCE, handle);
                let wings = world.alloc(shape.wings * 8);
                world.header(pool + cair_wing_pool::WINGS, wings, shape.wings as u32);
                for wing_index in 0..shape.wings {
                    let wing = world.alloc(0x40);
                    world.put64(wings + wing_index * 8, wing as u64);
                    world.put64(wing, cair_wing::VTABLE_RVA.0 as u64);
                    world.put64(
                        wing + cair_wing::SECONDARY,
                        cair_wing::SECONDARY_VTABLE_RVA.0 as u64,
                    );
                    world.put64(
                        wing + cair_wing::REFERENCE_TYPE,
                        (wing_id << 32) | u64::from(cair_wing::TYPE),
                    );
                    world.put64(wing + cair_wing::POOL, pool as u64);
                    wing_id += 1;
                }
            }
        }
    }
    world.bytes
}

#[cfg(test)]
fn copy(bytes: &[u8], job: &mut AirBasesJob) -> f64 {
    let memory = super::air_bases_test_kit::memory(bytes);
    let started = Instant::now();
    let report = copy_air_bases_fixture(
        &memory,
        AirBasesCopyContext {
            hour: 0,
            force_full: false,
            base: 0,
            game_state: GAME_STATE,
        },
        job,
    );
    let elapsed = started.elapsed().as_secs_f64() * 1e6;
    assert!(report.raw_ok);
    elapsed
}

#[cfg(test)]
fn decode(
    job: &AirBasesJob,
    captured: &mut fog::Captured,
    reuse: Option<&mut DecodedRegistrations>,
) -> (f64, fog::RegistrationsSnapshot) {
    let started = Instant::now();
    match reuse {
        Some(reuse) => {
            let indexed = reuse.begin(&job.segments);
            captured.decode_registrations_with_carrier_payload(
                0,
                job,
                &[],
                &mut super::carrier::Bases::new(0),
                &mut [0; AIR_BASES_N],
                indexed.then_some(reuse as &mut dyn cair_base::RegistrationReuse),
            );
            reuse.finish();
        }
        None => captured.decode_registrations_payload(0, job, &mut [0; AIR_BASES_N]),
    }
    let elapsed = started.elapsed().as_secs_f64() * 1e6;
    (elapsed, black_box(captured.registrations_snapshot()))
}

#[cfg(test)]
fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

#[test]
#[ignore = "bench: run alone in release"]
fn registration_bench() {
    const HOURS: usize = 41;
    for shape in [
        Shape {
            name: "late war",
            countries: 150,
            bases: 410,
            containers: 2,
            pools: 1,
            wings: 1,
        },
        Shape {
            name: "mod scale",
            countries: 1_500,
            bases: 2_000,
            containers: 3,
            pools: 2,
            wings: 2,
        },
    ] {
        let bytes = build(&shape);
        let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(shape.bases)));
        let mut job = AirBasesJob::fixture();
        job.registration_store = Some(Box::new(RegistrationStore::new()));
        job.carry = CarryRun {
            changes: watches,
            ..CarryRun::default()
        };
        job.slot_run = crate::air_base_changes::SlotRun { dense: true };
        let mut captured = fog::Captured::with_capacities(&fog::Capacities {
            rows: [16; 7],
            radar_provinces: 8,
            area_provinces: 8,
            wings: 16,
        });
        captured.registrations_raw_mode = true;
        let mut reuse = DecodedRegistrations::default();
        let store = job.registration_store.take();
        copy(&bytes, &mut job);
        job.grow();
        job.registration_store = store;
        copy(&bytes, &mut job);
        let (_, fresh_rows) = decode(&job, &mut captured, Some(&mut reuse));
        let rows = fresh_rows.rows.len();
        let mut fresh_copy = Vec::new();
        let mut carried_copy = Vec::new();
        let mut walked = Vec::new();
        let mut reused = Vec::new();
        for _ in 0..HOURS {
            let store = job.registration_store.take();
            fresh_copy.push(copy(&bytes, &mut job));
            job.registration_store = store;
            carried_copy.push(copy(&bytes, &mut job));
            assert_eq!(job.segments.len(), shape.bases);
            let (time, plain) = decode(&job, &mut captured, None);
            walked.push(time);
            let (time, kept) = decode(&job, &mut captured, Some(&mut reuse));
            reused.push(time);
            assert_eq!(
                (plain.outcome, &plain.rows, &plain.wings),
                (kept.outcome, &kept.rows, &kept.wings)
            );
            assert_eq!(plain.rows, fresh_rows.rows);
        }
        let counts = reuse.counts();
        println!(
            "{} ({} bases, {} countries, {rows} rows, {} wings): AirBases job fresh {:.1} us, carried {:.1} us; registrations decode walked {:.1} us, kept rows {:.1} us; decoded reused {} walked {}",
            shape.name,
            shape.bases,
            shape.countries,
            shape.bases * shape.containers * shape.pools * shape.wings,
            median(fresh_copy),
            median(carried_copy),
            median(walked),
            median(reused),
            counts.reused,
            counts.walked,
        );
    }
}
