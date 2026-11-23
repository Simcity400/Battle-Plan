use super::*;
use engine_native_memory::{cpdx_array::Array, memory::{LiveMemory, SliceMemory}};
use replay_data::rows::RowCodec;
use std::{cell::Cell, hint::black_box, time::Instant};

struct ReadCensus<'a, M> {
    source: &'a M,
    calls: Cell<usize>,
    spans: Cell<usize>,
    bytes: Cell<usize>,
}

impl<'a, M: Memory> ReadCensus<'a, M> {
    fn new(source: &'a M) -> Self {
        Self {
            source,
            calls: Cell::new(0),
            spans: Cell::new(0),
            bytes: Cell::new(0),
        }
    }
    fn calls(&self) -> usize {
        self.calls.get()
    }
    fn spans(&self) -> usize {
        self.spans.get()
    }
    fn bytes(&self) -> usize {
        self.bytes.get()
    }
    fn observe(&self, spans: usize, bytes: usize) {
        self.calls.set(self.calls.get() + 1);
        self.spans.set(self.spans.get() + spans);
        self.bytes.set(self.bytes.get() + bytes);
    }
}

impl<M: Memory> Memory for ReadCensus<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let copied = self.source.copy(address, destination);
        self.observe(
            usize::from(copied && !destination.is_empty()),
            if copied { destination.len() } else { 0 },
        );
        copied
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let lengths = destinations.each_ref().map(|destination| destination.len());
        let copied = self.source.copy_scattered(addresses, destinations);
        let completed = &lengths[..copied.min(N)];
        self.observe(
            completed.iter().filter(|&&len| len != 0).count(),
            completed.iter().sum(),
        );
        copied
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let completed = self.source.copy_records(sources, layout, destination);
        self.observe(
            layout.completed_spans(completed),
            layout.completed_bytes(completed),
        );
        completed
    }
}

const TYPICAL_CHANGED: usize = 584;
const STATE: usize = 0x100;
const MANAGER: usize = 0x1000;
const REGION: usize = 0x1800;
const PROVINCE_DATA: usize = 0x2000;

#[derive(Debug, Clone, Copy)]
enum Hour {
    Quiet,
    Typical,
    AllChanged,
}

struct World {
    bytes: Vec<u8>,
    origin: usize,
}
impl World {
    fn new(provinces: usize) -> Self {
        let snow = PROVINCE_DATA + provinces * sweather_per_province::STRIDE;
        let mud = snow + provinces;
        let clocks = mud + provinces;
        let mut bytes = vec![0; clocks + provinces * sweather_per_province::CLOCK_STRIDE];
        let origin = bytes.as_ptr() as usize;
        let mut put = |offset: usize, value: &[u8]| {
            bytes[offset..offset + value.len()].copy_from_slice(value)
        };
        put(
            STATE + engine_native::cgame_state::WEATHER_MANAGER,
            &((origin + MANAGER) as u64).to_le_bytes(),
        );
        put(
            MANAGER,
            &(cweather_manager::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        for (offset, data, count) in [
            (cweather_manager::REGIONS, REGION, 1),
            (cweather_manager::PROVINCES, PROVINCE_DATA, provinces),
            (cweather_manager::DISPLAY_SNOW, snow, provinces),
            (cweather_manager::DISPLAY_MUD, mud, provinces),
            (cweather_manager::PROVINCE_CLOCKS, clocks, provinces),
        ] {
            put(MANAGER + offset, &((origin + data) as u64).to_le_bytes());
            put(MANAGER + offset + 8, &(count as u32).to_le_bytes());
            put(MANAGER + offset + 12, &(count as u32).to_le_bytes());
        }
        put(
            REGION,
            &(sweather_per_region::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        for slot in 0..provinces {
            let row = PROVINCE_DATA + slot * sweather_per_province::STRIDE;
            put(
                row,
                &(sweather_per_province::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                row + sweather_per_province::ID,
                &(slot as u32).to_le_bytes(),
            );
            for word in [
                sweather_per_province::TEMPERATURE,
                sweather_per_province::STAGED_TEMPERATURE,
            ] {
                put(
                    row + word,
                    &((slot as i64 % 100 - 50) * 100_000).to_le_bytes(),
                );
            }
            put(
                row + sweather_per_province::RAW_MUD,
                &[(slot % 2 * 7) as u8],
            );
            let clock = clocks + slot * sweather_per_province::CLOCK_STRIDE;
            put(
                clock + sweather_per_province::CLOCK_LOCAL_HOUR,
                &(slot as i32 % 25).to_le_bytes(),
            );
            put(
                clock + sweather_per_province::CLOCK_NIGHT_FACTOR,
                &((slot as i64 % 2) * 100_000).to_le_bytes(),
            );
        }
        Self { bytes, origin }
    }
    fn state(&self) -> usize {
        self.origin + STATE
    }
    fn array(&self) -> Array {
        Array::read(
            &SliceMemory {
                base: self.origin,
                bytes: &self.bytes,
            },
            self.origin + MANAGER + cweather_manager::PROVINCES,
            sweather_per_province::STRIDE,
        )
        .unwrap()
    }
    fn advance(&mut self, hour: Hour, tick: usize) -> usize {
        let provinces = self.array().count as usize;
        let count = match hour {
            Hour::Quiet => 0,
            Hour::Typical => TYPICAL_CHANGED.min(provinces),
            Hour::AllChanged => provinces,
        };
        for index in 0..count {
            let slot = (tick * TYPICAL_CHANGED + index) % provinces;
            let offset = PROVINCE_DATA
                + slot * sweather_per_province::STRIDE
                + sweather_per_province::TEMPERATURE;
            let raw = i64::from_le_bytes(self.bytes[offset..offset + 8].try_into().unwrap());
            let step =
                ((slot * 131 % 200_000 + 1) as i64) * if slot.is_multiple_of(2) { 1 } else { -1 };
            let committed = raw.wrapping_add(step).to_le_bytes();
            self.bytes[offset..offset + 8].copy_from_slice(&committed);
            let staged = offset - sweather_per_province::TEMPERATURE
                + sweather_per_province::STAGED_TEMPERATURE;
            self.bytes[staged..staged + 8].copy_from_slice(&committed);
        }
        count
    }
}

fn temperature_delta(
    before: &[rows::WeatherProvinceRow],
    after: &[rows::WeatherProvinceRow],
) -> (usize, usize) {
    let mut bytes = Vec::new();
    let mut changed = 0;
    let mut previous_key = 0;
    for (old, new) in before.iter().zip(after) {
        if new
            .encode_change(Some(old), previous_key, &mut bytes)
            .unwrap()
        {
            let mask = new.delta(Some(old)).unwrap().unwrap().mask & !(1 << 13);
            if mask != 0 {
                assert_eq!(mask, 1 << 4);
                changed += 1;
            }
            previous_key = new.key();
        }
    }
    (changed, bytes.len())
}

#[test]
#[ignore = "synthetic guarded weather capture timing and encoded delta sizes"]
fn province_hover_weather_bench() {
    for hour in [Hour::Quiet, Hour::Typical, Hour::AllChanged] {
        let mut world = World::new(13_414);
        let provinces = world.array().count as usize;
        let mut captured = Captured::with_capacities(&Capacities {
            rows: [1, provinces],
            ..Default::default()
        });
        let mut job = WeatherJob::new();
        let mut old_payload = DensePayload::new(provinces * 28);
        let mut new_payload = DensePayload::new(provinces * sweather_per_province::RECORD_BYTES);
        let old_layout = RecordLayout::new(&sweather_per_province::FIELDS[..3], 28).unwrap();
        let new_layout = RecordLayout::new(
            &sweather_per_province::FIELDS,
            sweather_per_province::RECORD_BYTES,
        )
        .unwrap();
        let mut block = Vec::new();
        let mut decode = Vec::new();
        let mut old_gather = Vec::new();
        let mut new_gather = Vec::new();
        let mut previous: Option<Vec<rows::WeatherProvinceRow>> = None;
        let mut delta = (0, 0);
        for pass in 0..67 {
            world.advance(hour, pass);
            let live = unsafe { LiveMemory::new() };
            let start = Instant::now();
            let report = copy_weather_refresh(
                &live,
                WeatherCopyContext {
                    base: 0,
                    game_state: world.state(),
                },
                &mut job,
            );
            let elapsed = start.elapsed().as_secs_f64() * 1e6;
            assert!(report.raw_ok);
            captured.raw_mode = true;
            let start = Instant::now();
            captured.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
            let decoded = start.elapsed().as_secs_f64() * 1e6;
            assert_eq!(captured.provinces.outcome, Outcome::Rows { len: provinces });
            let [_, TableView::Complete(rows)] = captured.tables() else {
                panic!("weather provinces complete");
            };
            let after = rows.rows::<tables::WeatherProvinces>().unwrap().to_vec();
            if let Some(before) = &previous {
                delta = temperature_delta(before, &after);
            }
            previous = Some(after);
            let gathers = if pass % 2 == 0 {
                [
                    (&mut old_payload, &old_layout, &mut old_gather),
                    (&mut new_payload, &new_layout, &mut new_gather),
                ]
            } else {
                [
                    (&mut new_payload, &new_layout, &mut new_gather),
                    (&mut old_payload, &old_layout, &mut old_gather),
                ]
            };
            for (payload, layout, times) in gathers {
                payload.clear();
                payload.prepare(Some(world.array()), layout.stride());
                let start = Instant::now();
                payload.capture(&live, sweather_per_province::STRIDE, layout);
                if pass >= 3 {
                    times.push(start.elapsed().as_secs_f64() * 1e6);
                }
                black_box(&payload.bytes);
            }
            if pass >= 3 {
                block.push(elapsed);
                decode.push(decoded);
            }
        }
        let live = unsafe { LiveMemory::new() };
        let memory = ReadCensus::new(&live);
        copy_weather_refresh(
            &memory,
            WeatherCopyContext {
                base: 0,
                game_state: world.state(),
            },
            &mut job,
        );
        let requests = (memory.calls(), memory.spans(), memory.bytes());
        let percentiles = |mut samples: Vec<f64>| {
            samples.sort_by(f64::total_cmp);
            (
                samples[samples.len() / 2],
                samples[samples.len() * 95 / 100],
            )
        };
        println!(
            "WEATHER_BENCH hour={hour:?} provinces={provinces} raw_copy_us={:?} decode_us={:?} old_gather_us={:?} new_gather_us={:?} requests={requests:?} temperature_delta_rows={} temperature_delta_bytes={} extra_native_bytes={}",
            percentiles(block),
            percentiles(decode),
            percentiles(old_gather),
            percentiles(new_gather),
            delta.0,
            delta.1,
            provinces * 9
        );
    }
}

#[test]
#[ignore = "synthetic guarded weather component timing"]
fn weather_component_bench() {
    let world = World::new(13_414);
    let provinces = world.array().count as usize;
    let mut evict = vec![0_u8; 256 << 20];
    let province_layout = RecordLayout::new(
        &sweather_per_province::FIELDS,
        sweather_per_province::RECORD_BYTES,
    )
    .unwrap();
    let clock_layout = RecordLayout::new(
        &sweather_per_province::CLOCK_FIELDS,
        sweather_per_province::CLOCK_RECORD_BYTES,
    )
    .unwrap();
    let live = unsafe { LiveMemory::new() };
    let clocks = Array::read(
        &SliceMemory {
            base: world.origin,
            bytes: &world.bytes,
        },
        world.origin + MANAGER + cweather_manager::PROVINCE_CLOCKS,
        sweather_per_province::CLOCK_STRIDE,
    )
    .unwrap();
    let window = Array {
        data: world.array().data,
        count: 584,
    };
    let mut payload = DensePayload::new(provinces * sweather_per_province::RECORD_BYTES);
    let mut clock_payload =
        DensePayload::new(provinces * sweather_per_province::CLOCK_RECORD_BYTES);
    let mut samples: [Vec<f64>; 3] = Default::default();
    for round in 0..15 {
        for (part, sample) in samples.iter_mut().enumerate() {
            evict.fill(round as u8);
            black_box(&evict);
            let started = Instant::now();
            match part {
                0 => {
                    payload.clear();
                    payload.prepare(Some(world.array()), province_layout.stride());
                    payload.capture(&live, sweather_per_province::STRIDE, &province_layout);
                }
                1 => {
                    clock_payload.clear();
                    clock_payload.prepare(Some(clocks), clock_layout.stride());
                    clock_payload.capture(
                        &live,
                        sweather_per_province::CLOCK_STRIDE,
                        &clock_layout,
                    );
                }
                _ => {
                    payload.clear();
                    payload.prepare(Some(window), province_layout.stride());
                    payload.capture(&live, sweather_per_province::STRIDE, &province_layout);
                }
            }
            sample.push(started.elapsed().as_secs_f64() * 1e3);
        }
    }
    for (name, mut sample) in [
        "province records",
        "province clocks",
        "one pass window (584)",
    ]
    .into_iter()
    .zip(samples)
    {
        sample.sort_by(f64::total_cmp);
        println!(
            "WEATHER_COMPONENT {name}: {:.3} ms median ({:.3}..{:.3})",
            sample[7], sample[0], sample[14]
        );
    }
}
