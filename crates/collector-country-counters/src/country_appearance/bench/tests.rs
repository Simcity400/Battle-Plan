use super::*;
use engine_native::{
    ccountry_database,
    cgame_state,
    memory::{CountedMemory, LiveMemory, SliceMemory},
    msvc_string,
};
use engine_native_politics::cdiplomacy_status;
use replay_data::rows::RowCodec;
use std::{hint::black_box, time::Instant};

const STATE: usize = 0x100;
const DATABASE: usize = 0x800;
pub(super) const ARRAYS: usize = 0x1000;
const COUNTRY_BYTES: usize = 0x1600;
const DIPLOMACY_BYTES: usize = 0x300;
const SUBJECTS_PER_COUNTRY: usize = 3;

pub(super) struct World {
    pub(super) bytes: Vec<u8>,
    pub(super) origin: usize,
    pub(super) module: usize,
    pub(super) countries: usize,
    subjects: usize,
}

impl World {
    pub(super) fn new(countries: usize, populated: bool) -> Self {
        let slots = countries + 1;
        let links = ARRAYS + slots * 8;
        let tags = links + slots * 4;
        let objects = (tags + slots * msvc_string::STRIDE + 7) & !7;
        let diplomacy = objects + countries * COUNTRY_BYTES;
        let subjects = diplomacy + countries * DIPLOMACY_BYTES;
        let mut bytes = vec![0; subjects + countries * SUBJECTS_PER_COUNTRY * 4];
        let origin = bytes.as_ptr() as usize;
        let module = origin
            .checked_sub(ccountry_database::SINGLETON_RVA.0)
            .unwrap();
        let put = |bytes: &mut [u8], offset: usize, value: &[u8]| {
            bytes[offset..offset + value.len()].copy_from_slice(value);
        };
        let array = |bytes: &mut [u8], offset: usize, data: usize, count: usize| {
            put(bytes, offset, &((origin + data) as u64).to_le_bytes());
            put(bytes, offset + 8, &(count as u32).to_le_bytes());
            put(bytes, offset + 12, &(count as u32).to_le_bytes());
        };
        put(&mut bytes, 0, &((origin + DATABASE) as u64).to_le_bytes());
        put(
            &mut bytes,
            DATABASE + ccountry_database::DYNAMIC_TAG_BOUNDARY,
            &(slots as u32).to_le_bytes(),
        );
        array(&mut bytes, STATE + cgame_state::COUNTRIES, ARRAYS, slots);
        array(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, links, slots);
        array(&mut bytes, STATE + cgame_state::COUNTRY_TAGS, tags, slots);
        for slot in 0..slots {
            put(
                &mut bytes,
                tags + slot * msvc_string::STRIDE + msvc_string::CAPACITY,
                &msvc_string::INLINE_CAPACITY.to_le_bytes(),
            );
        }
        for index in 0..countries {
            let slot = index + 1;
            let country = objects + index * COUNTRY_BYTES;
            let diplomacy = diplomacy + index * DIPLOMACY_BYTES;
            put(
                &mut bytes,
                ARRAYS + slot * 8,
                &((origin + country) as u64).to_le_bytes(),
            );
            put(&mut bytes, links + slot * 4, &(slot as u32).to_le_bytes());
            put(
                &mut bytes,
                country,
                &((module + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                country + ccountry::ID,
                &(slot as u32).to_le_bytes(),
            );
            put(
                &mut bytes,
                country + ccountry::DIPLOMACY_STATUS,
                &((origin + diplomacy) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                diplomacy,
                &((module + cdiplomacy_status::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            for offset in [
                ccountry::COSMETIC_TAG,
                ccountry::GRAPHICAL_CULTURE,
                ccountry::GRAPHICAL_CULTURE_2D,
                ccountry::OLD_NAME,
                ccountry::OLD_ADJECTIVE,
                ccountry::OLD_DEFINITE_NAME,
            ] {
                put(
                    &mut bytes,
                    country + offset + msvc_string::CAPACITY,
                    &msvc_string::INLINE_CAPACITY.to_le_bytes(),
                );
            }
            let payload = subjects + index * SUBJECTS_PER_COUNTRY * 4;
            array(
                &mut bytes,
                diplomacy + cdiplomacy_status::SUBJECTS,
                payload,
                if populated { SUBJECTS_PER_COUNTRY } else { 0 },
            );
            for (entry, value) in [slot as i32, -1, slot as i32].into_iter().enumerate() {
                put(&mut bytes, payload + entry * 4, &value.to_le_bytes());
            }
        }
        Self {
            bytes,
            origin,
            module,
            countries,
            subjects,
        }
    }

    pub(super) fn state(&self) -> usize {
        self.origin + STATE
    }

    fn advance(&mut self, hour: Hour, tick: usize) -> usize {
        let changed = match hour {
            Hour::Empty | Hour::Quiet => 0,
            Hour::Typical => 1,
            Hour::AllChanged => self.countries,
        };
        for index in 0..changed {
            let slot = (tick + index) % self.countries;
            let offset = self.subjects + slot * SUBJECTS_PER_COUNTRY * 4;
            let value = i32::from_le_bytes(self.bytes[offset..offset + 4].try_into().unwrap());
            self.bytes[offset..offset + 4].copy_from_slice(&value.wrapping_add(1).to_le_bytes());
        }
        changed
    }

    pub(super) fn capacities(&self) -> Capacities {
        Capacities {
            rows: self.countries,
            subjects: self.countries * SUBJECTS_PER_COUNTRY,
            naming_bytes: 0,
            ..Default::default()
        }
    }

    pub(super) fn read(&self, captured: &mut Captured) -> Vec<CountryAppearanceRow> {
        captured.reset();
        captured.capture(
            &SliceMemory {
                base: self.origin,
                bytes: &self.bytes,
            },
            self.module,
            self.state(),
        );
        rows(captured)
    }
}

pub(super) fn rows(captured: &Captured) -> Vec<CountryAppearanceRow> {
    let [
        TableView::Complete(replay_data::rows::TableRows::CountryAppearance(rows)),
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete appearance");
    };
    rows
}

fn subject_delta(
    before: &[CountryAppearanceRow],
    after: &[CountryAppearanceRow],
) -> (usize, usize) {
    let mut bytes = Vec::new();
    let mut changed = 0;
    let mut previous_key = 0;
    for (old, new) in before.iter().zip(after) {
        if new
            .encode_change(Some(old), previous_key, &mut bytes)
            .unwrap()
        {
            assert_eq!(new.delta(Some(old)).unwrap().unwrap().mask, 1 << 18);
            previous_key = new.key();
            changed += 1;
        }
    }
    (changed, bytes.len())
}

#[derive(Debug, Clone, Copy)]
enum Hour {
    Empty,
    Quiet,
    Typical,
    AllChanged,
}

#[test]
#[ignore = "synthetic ordered-subject capture and hourly encoded delta costs"]
fn border_effect_subject_scale_bench() {
    for countries in [193, 1_500, 5_000] {
        let mut empty_requests = None;
        for hour in [Hour::Empty, Hour::Quiet, Hour::Typical, Hour::AllChanged] {
            let mut world = World::new(countries, !matches!(hour, Hour::Empty));
            let mut captured = Captured::with_capacities(&world.capacities());
            let mut capture_times = Vec::new();
            let mut conversion_times = Vec::new();
            let mut delta_times = Vec::new();
            let mut previous = world.read(&mut captured);
            let mut delta = (0, 0);
            for pass in 0..67 {
                world.advance(hour, pass);
                captured.reset();
                let live = unsafe { LiveMemory::new() };
                let start = Instant::now();
                captured.capture(&live, world.module, world.state());
                let capture_us = start.elapsed().as_secs_f64() * 1e6;
                let start = Instant::now();
                let after = rows(&captured);
                let conversion_us = start.elapsed().as_secs_f64() * 1e6;
                let start = Instant::now();
                delta = subject_delta(&previous, &after);
                let delta_us = start.elapsed().as_secs_f64() * 1e6;
                previous = after;
                if pass >= 3 {
                    capture_times.push(capture_us);
                    conversion_times.push(conversion_us);
                    delta_times.push(delta_us);
                }
                black_box(&captured);
            }
            let memory = CountedMemory::new(unsafe { LiveMemory::new() });
            let accounting = crate::read_accounting::Capture::new();
            captured.capture(&memory, world.module, world.state());
            let requests = accounting.finish()[TableId::CountryAppearance.index()];
            if matches!(hour, Hour::Empty) {
                empty_requests = Some(requests);
            }
            let empty = empty_requests.unwrap();
            let payload_bytes = captured.subjects.len * 4;
            assert_eq!(requests.bytes - empty.bytes, payload_bytes as u64);
            let added_bytes = countries * 24 + payload_bytes;
            let percentiles = |mut samples: Vec<f64>| {
                samples.sort_by(f64::total_cmp);
                (
                    samples[samples.len() / 2],
                    samples[samples.len() * 95 / 100],
                )
            };
            println!(
                "SUBJECT_BENCH hour={hour:?} countries={countries} capture_us={:?} conversion_us={:?} delta_us={:?} requests={requests:?} prior_requested_bytes={} added_header_bytes={} positive_payload_bytes={payload_bytes} delta_rows={} delta_bytes={} fixture_bytes={} retained_bytes={}",
                percentiles(capture_times),
                percentiles(conversion_times),
                percentiles(delta_times),
                requests.bytes - added_bytes as u64,
                countries * 24,
                delta.0,
                delta.1,
                world.bytes.len(),
                captured.allocated_bytes(),
            );
        }
    }
}
