//! Bounded original strategic search graph; numerical projection is off-thread.
use crate::table_capture::{Arena, grow_vec};
use engine::{
    transport_ledger_observation::{PointerSlot, index_capacity},
    transport_naval_search as native,
};
use replay::rows::{TransportNavalEdge, TransportNavalRegion, TransportNavalSearch};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static REFUSALS: AtomicU64 = AtomicU64::new(0);
pub(crate) fn refusals() -> u64 {
    REFUSALS.load(Relaxed)
}

pub(crate) struct Captured {
    regions: Arena<native::Region>,
    edges: Arena<native::Edge>,
    occurrences: Arena<Option<u32>>,
    names: Arena<native::Name>,
    bytes: Arena<u8>,
    region_controls: Vec<native::RegionControl>,
    region_slots: Vec<PointerSlot>,
    name_controls: Vec<native::NameControl>,
    name_slots: Vec<PointerSlot>,
    original: Option<native::Capture>,
    normalization: engine::transport_air_normalization::Capture,
    normalization_required: usize,
    normalization_entries_required: usize,
    weather_active: bool,
    weather_records: Vec<Option<Result<usize, &'static str>>>,
}

struct SpottingLoan<'a, M: engine::Memory> {
    memory: &'a M,
    base: usize,
    weather: Result<&'a engine::cweather_manager::Reader<'a, M>, &'static str>,
    weather_records: &'a mut Vec<Option<Result<usize, &'static str>>>,
    context: &'a mut super::transport_spotting_context::Captured,
    normalization: &'a mut engine::transport_air_normalization::Capture,
    clocks: Result<engine::cpdx_array::Array, &'static str>,
}
impl<M: engine::Memory> native::SpottingObserver for SpottingLoan<'_, M> {
    fn begin(&mut self, count: Result<usize, &'static str>) {
        self.context.begin_regions(count);
        if let Ok(count) = count {
            grow_vec(self.weather_records, count);
        }
    }
    fn region(
        &mut self,
        slot: u32,
        original: Result<Option<engine::transport_spotting_context::RegionOriginal>, &'static str>,
    ) {
        let normalization = &mut self.normalization;
        let memory = self.memory;
        let base = self.base;
        let clocks = self.clocks;
        let record = match original {
            Ok(Some(region)) => match self.weather_records.get_mut(slot as usize) {
                Some(memo) => *memo.get_or_insert_with(|| {
                    self.weather
                        .and_then(|weather| weather.region_record(region.id))
                }),
                None => Err("transport original weather record memo arena exhausted"),
            },
            _ => Err("transport original weather selector unreached"),
        };
        self.context.region_with_observed_weather(
            memory,
            base,
            slot,
            original,
            record,
            &mut |sampled, region, weather_record| {
                let (environment, detection, _) = normalization.environment_detection_and_naval(
                    memory,
                    base,
                    sampled,
                    region,
                    weather_record,
                    clocks,
                );
                (
                    detection,
                    Some(replay::rows::TransportAirEnvironment::from(environment)),
                )
            },
        );
    }
}
impl Captured {
    pub(crate) fn prepare(&mut self, demand: native::Demand, weather_entries: usize) {
        self.regions.required = self.regions.required.max(demand.regions);
        self.edges.required = self.edges.required.max(demand.edges);
        self.occurrences.required = self.occurrences.required.max(demand.occurrences);
        self.names.required = self.names.required.max(demand.names);
        self.bytes.required = self.bytes.required.max(demand.bytes);
        self.normalization_required = self
            .normalization_required
            .max(demand.regions.saturating_mul(2));
        self.normalization_entries_required =
            self.normalization_entries_required.max(weather_entries);
        self.grow();
    }

    pub(crate) fn new(regions: usize, edges: usize) -> Self {
        Self {
            regions: Arena::new(regions),
            edges: Arena::new(edges),
            occurrences: Arena::new(edges),
            names: Arena::new(edges),
            bytes: Arena::new(0),
            region_controls: engine::allocation::owned_vec(engine::allocation::default_vec(
                regions,
            )),
            region_slots: engine::allocation::owned_vec(engine::allocation::default_vec(
                index_capacity(regions).unwrap_or(0),
            )),
            name_controls: engine::allocation::owned_vec(engine::allocation::default_vec(edges)),
            name_slots: engine::allocation::owned_vec(engine::allocation::default_vec(
                index_capacity(edges).unwrap_or(0),
            )),
            original: None,
            normalization: engine::transport_air_normalization::Capture::try_new(
                regions.saturating_mul(2),
                0,
            )
            .map_err(crate::allocation::io)
            .unwrap_or_else(|_| engine::transport_air_normalization::Capture::vacant()),
            normalization_required: 0,
            normalization_entries_required: 0,
            weather_active: false,
            weather_records: engine::allocation::owned_vec(engine::allocation::filled(
                regions, None,
            )),
        }
    }
    pub(crate) fn reset(&mut self) {
        self.original = None;
        self.normalization_required = self
            .normalization_required
            .max(self.normalization.required().0);
        self.normalization_entries_required = self
            .normalization_entries_required
            .max(self.normalization.required().1);
        self.normalization.clear();
        self.weather_active = false;
        self.weather_records.fill(None);
        let demand = [
            self.regions.required,
            self.edges.required,
            self.occurrences.required,
            self.names.required,
            self.bytes.required,
        ];
        self.regions.reset();
        self.edges.reset();
        self.occurrences.reset();
        self.names.reset();
        self.bytes.reset();
        self.regions.required = demand[0];
        self.edges.required = demand[1];
        self.occurrences.required = demand[2];
        self.names.required = demand[3];
        self.bytes.required = demand[4];
        for control in &mut self.region_controls {
            *control = Default::default();
        }
        for control in &mut self.name_controls {
            *control = Default::default();
        }
        self.region_slots.fill(Default::default());
        self.name_slots.fill(Default::default());
    }
    #[cfg(test)]
    pub(crate) fn capture(&mut self, memory: &impl engine::Memory, base: usize, state: usize) {
        self.capture_inner(memory, base, state, None, None, false);
    }
    /// Both borrowers live in the same original mission callback. The source
    /// owner must retire this numerical/native weather union after strict read.
    pub(crate) fn capture_with_spotting_weather<M: engine::Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        context: &mut super::transport_spotting_context::Captured,
        weather: Result<&engine::cweather_manager::Reader<'_, M>, &'static str>,
    ) {
        self.capture_inner(memory, base, state, Some(context), Some(weather), true);
    }
    fn capture_inner<M: engine::Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        mut context: Option<&mut super::transport_spotting_context::Captured>,
        weather: Option<Result<&engine::cweather_manager::Reader<'_, M>, &'static str>>,
        defer_weather: bool,
    ) {
        if let Some(original) = &mut self.original {
            REFUSALS.fetch_add(1, Relaxed);
            original.lengths = Err("transport original naval graph attempted twice");
            if let Some(context) = context {
                context.fail();
            }
            return;
        }
        self.weather_active = defer_weather && context.is_some();
        if defer_weather {
            self.normalization.enable_worker_growth();
        }
        let output = if defer_weather {
            native::GrowingOutput {
                regions: &mut self.regions.first,
                edges: &mut self.edges.first,
                occurrences: &mut self.occurrences.first,
                names: &mut self.names.first,
                bytes: &mut self.bytes.first,
                region_controls: &mut self.region_controls,
                region_slots: &mut self.region_slots,
                name_controls: &mut self.name_controls,
                name_slots: &mut self.name_slots,
            }
            .into()
        } else {
            native::Output {
                regions: &mut self.regions.first,
                edges: &mut self.edges.first,
                occurrences: &mut self.occurrences.first,
                names: &mut self.names.first,
                bytes: &mut self.bytes.first,
                region_controls: &mut self.region_controls,
                region_slots: &mut self.region_slots,
                name_controls: &mut self.name_controls,
                name_slots: &mut self.name_slots,
            }
            .into()
        };
        let original = match context.as_deref_mut() {
            Some(context) => {
                let weather =
                    weather.unwrap_or(Err("transport original weather reader not loaned"));
                let clocks = weather.and_then(|weather| weather.province_clocks());
                native::capture_buffered(
                    memory,
                    base,
                    state,
                    output,
                    Some(&mut SpottingLoan {
                        memory,
                        base,
                        weather,
                        context,
                        normalization: &mut self.normalization,
                        weather_records: &mut self.weather_records,
                        clocks,
                    }),
                )
            }
            None => native::capture_buffered(memory, base, state, output, None),
        };
        if let Some(context) = context {
            context.finish();
        }
        self.normalization_required = self
            .normalization_required
            .max(original.required.regions.saturating_mul(2))
            .max(self.normalization.required().0);
        if !defer_weather {
            self.retire_weather();
        }
        self.regions.required = self.regions.required.max(original.required.regions);
        self.edges.required = self.edges.required.max(original.required.edges);
        self.occurrences.required = self.occurrences.required.max(original.required.occurrences);
        self.names.required = self.names.required.max(original.required.names);
        self.bytes.required = self.bytes.required.max(original.required.bytes);
        if original.lengths.is_err() || original.width.is_err() {
            REFUSALS.fetch_add(1, Relaxed);
        }
        self.original = Some(original);
    }
    pub(crate) fn strict_weather<M: engine::Memory>(
        &mut self,
        memory: &M,
        base: usize,
        region: u32,
        weather: Result<&engine::cweather_manager::Reader<'_, M>, &'static str>,
    ) -> Result<[engine::modifier_accumulator::Value; 4], &'static str> {
        if !self.weather_active {
            return Err("transport original weather union is retired");
        }
        let record = match self.weather_records.get_mut(region as usize) {
            Some(memo) => *memo
                .get_or_insert_with(|| weather.and_then(|weather| weather.region_record(region))),
            None => weather.and_then(|weather| weather.region_record(region)),
        };
        self.normalization.strict_weather(memory, base, record)
    }
    pub(crate) fn retire_weather(&mut self) {
        self.weather_active = false;
        self.normalization_required = self
            .normalization_required
            .max(self.normalization.required().0);
        self.normalization_entries_required = self
            .normalization_entries_required
            .max(self.normalization.required().1);
        self.normalization.retire();
        self.weather_records.fill(None);
    }

    pub(crate) fn grow(&mut self) {
        self.normalization_required = self
            .normalization_required
            .max(self.normalization.required().0);
        self.normalization_entries_required = self
            .normalization_entries_required
            .max(self.normalization.required().1);
        let (mut nodes, mut entries) = self.normalization.capacity();
        crate::table_capture::grow_capacity(&mut nodes, self.normalization_required);
        crate::table_capture::grow_capacity(&mut entries, self.normalization_entries_required);
        let _ = self
            .normalization
            .try_grow(nodes, entries)
            .map_err(crate::allocation::io);
        let mut count = self.regions.first.len();
        self.regions.grow(&mut count);
        grow_vec(&mut self.region_controls, count);
        grow_vec(&mut self.weather_records, count);
        if let Some(slots) = index_capacity(count) {
            grow_vec(&mut self.region_slots, slots);
        }
        let mut count = self.edges.first.len();
        self.edges.grow(&mut count);
        let mut count = self.occurrences.first.len();
        self.occurrences.grow(&mut count);
        let mut count = self.names.first.len();
        self.names.grow(&mut count);
        grow_vec(&mut self.name_controls, count);
        if let Some(slots) = index_capacity(count) {
            grow_vec(&mut self.name_slots, slots);
        }
        let mut count = self.bytes.first.len();
        self.bytes.grow(&mut count);
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.normalization.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.edges.allocated_bytes()
            + self.occurrences.allocated_bytes()
            + self.names.allocated_bytes()
            + self.bytes.allocated_bytes()
            + self.weather_records.capacity()
                * std::mem::size_of::<Option<Result<usize, &'static str>>>()
            + self.region_controls.capacity() * std::mem::size_of::<native::RegionControl>()
            + self.name_controls.capacity() * std::mem::size_of::<native::NameControl>()
            + (self.region_slots.capacity() + self.name_slots.capacity())
                * std::mem::size_of::<PointerSlot>()
    }
    pub(crate) fn projection(&self) -> std::io::Result<Option<TransportNavalSearch>> {
        let Some(original) = self.original else {
            return Ok(None);
        };
        let Ok(lengths) = original.lengths else {
            return Ok(Some(TransportNavalSearch {
                width: original.width.ok(),
                ..Default::default()
            }));
        };
        let mut rules = engine::allocation::vec(lengths.names)?;
        for name in &self.names.first[..lengths.names] {
            rules.push(engine::allocation::copy_vec(
                &self.bytes.first[name.first..name.first + name.count],
            )?);
        }
        let mut regions = engine::allocation::vec(lengths.regions)?;
        for region in &self.regions.first[..lengths.regions] {
            if !region.present {
                regions.push(None);
                continue;
            }
            let mut edges = engine::allocation::vec(region.count)?;
            for edge in &self.edges.first[region.first..region.first + region.count] {
                edges.push(TransportNavalEdge {
                    to: edge.to,
                    flags: edge.flags,
                    unrestricted: edge.unrestricted,
                    rules: engine::allocation::copy_vec(
                        &self.occurrences.first[edge.first..edge.first + edge.count],
                    )?,
                });
            }
            regions.push(Some(TransportNavalRegion {
                position: region.position,
                edges,
            }));
        }
        Ok(Some(TransportNavalSearch {
            width: original.width.ok(),
            regions: Some(regions),
            rules,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Image {
        bytes: Vec<u8>,
        reads: Cell<usize>,
        fault: Option<usize>,
    }
    impl engine::Memory for Image {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if self.fault == Some(at) {
                return false;
            }
            if at == engine::cmap::SINGLETON_RVA && output.len() == 8 {
                output.copy_from_slice(&0x2000_u64.to_le_bytes());
                return true;
            }
            let Some(bytes) = at
                .checked_add(output.len())
                .and_then(|end| self.bytes.get(at..end))
            else {
                return false;
            };
            output.copy_from_slice(bytes);
            true
        }
    }
    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn header(bytes: &mut [u8], at: usize, pointer: u64, count: u32) {
        put(bytes, at, &pointer.to_le_bytes());
        put(bytes, at + 8, &count.to_le_bytes());
        put(bytes, at + 12, &count.to_le_bytes());
    }
    fn image() -> Image {
        let mut bytes = vec![0; 0x7000];
        header(
            &mut bytes,
            0x1000 + engine::cgame_state::STRATEGIC_REGIONS,
            0x2800,
            3,
        );
        put(
            &mut bytes,
            0x2000 + engine::cmap::WIDTH,
            &1024_i32.to_le_bytes(),
        );
        put(&mut bytes, 0x2808, &0x3000_u64.to_le_bytes());
        put(&mut bytes, 0x2810, &0x4000_u64.to_le_bytes());
        for (object, id) in [(0x3000, 1_u32), (0x4000, 2)] {
            put(
                &mut bytes,
                object,
                &(engine::cstrategic_region::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                object + engine::cstrategic_region::ID,
                &id.to_le_bytes(),
            );
        }
        header(&mut bytes, 0x30b0, 0x5000, 1);
        header(&mut bytes, 0x30c8, 0x5100, 1);
        put(&mut bytes, 0x5000, &0x4000_u64.to_le_bytes());
        bytes[0x5100] = 2;
        bytes[0x5158] = 1;
        Image {
            bytes,
            reads: Cell::new(0),
            fault: None,
        }
    }

    struct FirstFault<'a> {
        image: &'a Image,
        fault: Option<usize>,
        failed: Cell<bool>,
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
    }

    #[test]
    fn live_headers_prepare_a_complete_graph_on_the_first_capture() {
        let mut memory = image();
        memory.bytes[0x5158] = 0;
        header(&mut memory.bytes, 0x5110, 0x5200, 2);
        put(&mut memory.bytes, 0x5200, &0x6000_u64.to_le_bytes());
        put(&mut memory.bytes, 0x5208, &0x6000_u64.to_le_bytes());
        put(&mut memory.bytes, 0x6000, &0x6100_u64.to_le_bytes());
        put(&mut memory.bytes, 0x6100, &0x029b_f7f8_u64.to_le_bytes());
        put(&mut memory.bytes, 0x6138, b"RULE");
        put(&mut memory.bytes, 0x6148, &4_u64.to_le_bytes());
        put(&mut memory.bytes, 0x6150, &15_u64.to_le_bytes());
        let demand = native::demand(&memory, 0, 0x1000).unwrap();
        let mut captured = Captured::new(0, 0);
        captured.prepare(demand, 0);
        captured.capture(&memory, 0, 0x1000);
        let packet = captured.projection().unwrap().unwrap();
        assert_eq!(packet.regions.as_ref().unwrap().len(), 3);
        assert_eq!(packet.rules, vec![b"RULE".to_vec()]);
        assert_eq!(packet.regions.unwrap()[1].as_ref().unwrap().edges.len(), 1);
    }
    impl engine::Memory for FirstFault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            if self.fault == Some(address) && !self.failed.replace(true) {
                return false;
            }
            engine::Memory::copy(self.image, address, output)
        }
    }
    fn weather_image() -> Image {
        let mut image = image();
        let bytes = &mut image.bytes;
        put(
            bytes,
            0x1000 + engine::cgame_state::WEATHER_MANAGER,
            &0x5800_u64.to_le_bytes(),
        );
        put(
            bytes,
            0x5800,
            &(engine::cweather_manager::VTABLE_RVA as u64).to_le_bytes(),
        );
        header(bytes, 0x5800 + engine::cweather_manager::REGIONS, 0x6000, 3);
        for id in [1_u32, 2] {
            let record = 0x6000 + id as usize * engine::sweather_per_region::STRIDE;
            put(
                bytes,
                record,
                &(engine::sweather_per_region::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(
                bytes,
                record + engine::sweather_per_region::ID,
                &id.to_le_bytes(),
            );
        }
        header(
            bytes,
            0x6160 + engine::sweather_per_region::MODIFIERS,
            0x6900,
            1,
        );
        put(
            bytes,
            0x6900,
            &engine::naval_mission_inputs::TASK_FORCE_KEYS[0].to_le_bytes(),
        );
        put(bytes, 0x6908, &17_i64.to_le_bytes());
        image
    }

    #[test]
    fn original_weather_bridge_preserves_strict_raw_values_at_zero_extension_entry_capacity() {
        let image = weather_image();
        let memory = FirstFault {
            image: &image,
            fault: None,
            failed: Cell::new(false),
            calls: Default::default(),
        };
        let weather = engine::cweather_manager::Reader::new(&memory, 0, 0x1000).unwrap();
        let mut graph = Captured::new(3, 1);
        let mut context = super::super::transport_spotting_context::Captured::new(
            super::super::transport_spotting_context::Capacities {
                regions: 3,
                ..Default::default()
            },
        );
        context.begin(43_800_000, Ok(0));
        graph.capture_with_spotting_weather(&memory, 0, 0x1000, &mut context, Ok(&weather));
        let calls = memory.calls.borrow().len();
        let values = graph.strict_weather(&memory, 0, 1, Ok(&weather)).unwrap();
        assert_eq!(
            values[0],
            engine::modifier_accumulator::Value {
                raw: 17,
                present: true
            }
        );
        assert_eq!(
            memory.calls.borrow().len(),
            calls,
            "strict borrows original selector/header/full body result"
        );
        graph.retire_weather();
        assert!(graph.weather_records.iter().all(Option::is_none));
        assert!(
            graph.strict_weather(&memory, 0, 1, Ok(&weather)).is_err(),
            "retired source cannot start a new selector"
        );
        assert!(graph.normalization_entries_required >= 1);
        graph.grow();
        assert!(
            graph.normalization.capacity().1 >= 1,
            "one growth admits the known original full body"
        );
    }

    #[test]
    fn live_weather_headers_prepare_detection_on_the_first_capture() {
        let image = weather_image();
        let weather = engine::cweather_manager::Reader::new(&image, 0, 0x1000).unwrap();
        let mut graph = Captured::new(0, 0);
        graph.prepare(
            native::demand(&image, 0, 0x1000).unwrap(),
            engine::transport_air_normalization::weather_entry_demand(&image, 0, 0x1000).unwrap(),
        );
        let mut context = super::super::transport_spotting_context::Captured::new(
            super::super::transport_spotting_context::Capacities {
                regions: 3,
                ..Default::default()
            },
        );
        context.begin(43_800_000, Ok(0));
        graph.capture_with_spotting_weather(&image, 0, 0x1000, &mut context, Ok(&weather));
        assert_eq!(
            context.projection().unwrap().unwrap().regions[0].weather_detection,
            Some(Some(0))
        );
    }

    #[test]
    fn original_weather_bridge_never_heals_first_selector_identity_header_or_body_fault() {
        for fault in [
            0x5800 + engine::cweather_manager::REGIONS_COUNT,
            0x5800 + engine::cweather_manager::REGIONS,
            0x6160,
            0x6160 + engine::sweather_per_region::MODIFIERS,
            0x6900,
        ] {
            let image = weather_image();
            let memory = FirstFault {
                image: &image,
                fault: Some(fault),
                failed: Cell::new(false),
                calls: Default::default(),
            };
            let weather = engine::cweather_manager::Reader::new(&memory, 0, 0x1000).unwrap();
            let mut graph = Captured::new(3, 1);
            let mut context = super::super::transport_spotting_context::Captured::new(
                super::super::transport_spotting_context::Capacities {
                    regions: 3,
                    ..Default::default()
                },
            );
            context.begin(43_800_000, Ok(0));
            graph.capture_with_spotting_weather(&memory, 0, 0x1000, &mut context, Ok(&weather));
            let calls = memory.calls.borrow().len();
            assert!(
                memory.failed.get(),
                "fixture must reach the original fault {fault:x}"
            );
            assert!(graph.strict_weather(&memory, 0, 1, Ok(&weather)).is_err());
            assert_eq!(
                memory.calls.borrow().len(),
                calls,
                "second native read would succeed but is forbidden for {fault:x}"
            );
            graph.retire_weather();
            assert!(graph.weather_records.iter().all(Option::is_none));
        }
    }

    #[test]
    fn unattempted_strict_weather_survives_unrelated_original_graph_roster_failure() {
        let image = weather_image();
        let fault = 0x1000 + engine::cgame_state::STRATEGIC_REGIONS;
        let memory = FirstFault {
            image: &image,
            fault: Some(fault),
            failed: Cell::new(false),
            calls: Default::default(),
        };
        let weather = engine::cweather_manager::Reader::new(&memory, 0, 0x1000).unwrap();
        let mut graph = Captured::new(0, 0);
        let mut context =
            super::super::transport_spotting_context::Captured::new(Default::default());
        context.begin(43_800_000, Ok(0));
        graph.capture_with_spotting_weather(&memory, 0, 0x1000, &mut context, Ok(&weather));
        assert!(graph.original.unwrap().lengths.is_err());
        let values = graph.strict_weather(&memory, 0, 1, Ok(&weather)).unwrap();
        assert_eq!(values[0].raw, 17);
        graph.retire_weather();
    }

    #[test]
    fn original_naval_graph_second_attempt_refuses_without_rereading() {
        let memory = image();
        let mut captured = Captured::new(3, 1);
        captured.capture(&memory, 0, 0x1000);
        assert!(captured.projection().unwrap().unwrap().regions.is_some());
        let reads = memory.reads.get();
        captured.capture(&memory, 0, 0x1000);
        assert_eq!(memory.reads.get(), reads);
        assert!(captured.projection().unwrap().unwrap().regions.is_none());
    }

    #[test]
    fn reset_drops_old_graph_and_new_world_at_same_addresses_is_fresh_or_refused() {
        let mut memory = image();
        let mut captured = Captured::new(3, 1);
        captured.capture(&memory, 0, 0x1000);
        captured.reset();
        assert_eq!(captured.projection().unwrap(), None);
        put(&mut memory.bytes, 0x30f0, &(-17_i32).to_le_bytes());
        memory.bytes[0x5158] = 23;
        captured.capture(&memory, 0, 0x1000);
        let graph = captured.projection().unwrap().unwrap();
        let region = graph.regions.unwrap()[1].clone().unwrap();
        assert_eq!(region.position, [-17, 0]);
        assert_eq!(region.edges[0].unrestricted, 23);
        captured.reset();
        memory.fault = Some(0x5100);
        captured.capture(&memory, 0, 0x1000);
        assert!(captured.projection().unwrap().unwrap().regions.is_none());
    }

    #[test]
    fn original_graph_capacity_demand_survives_reset_and_recovers_after_growth() {
        let memory = image();
        let mut captured = Captured::new(1, 0);
        captured.capture(&memory, 0, 0x1000);
        assert!(captured.projection().unwrap().unwrap().regions.is_none());
        assert_eq!((captured.regions.required, captured.edges.required), (3, 1));
        captured.reset();
        captured.grow();
        captured.capture(&memory, 0, 0x1000);
        let graph = captured.projection().unwrap().unwrap();
        assert_eq!(graph.regions.unwrap().len(), 3);
    }
}
