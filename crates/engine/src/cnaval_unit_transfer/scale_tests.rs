//! Live-scale transport worlds: many moving owners, every strategic region and
//! ordered enemy navies, read through the hourly country-movement packet.

use super::*;
use crate::{
    carmy::country_movement_inputs::{
        CountryMovementPacket, RegionTransportSpeed, TransportRegion, capture_transport,
    },
    ccountry::synthetic::World,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

const MODULE: usize = 0x4000_0000;

/// Two coalitions at war in canonical slots `first..`: every member of one
/// side lists every member of the other as an enemy, in native roster order.
struct Shape {
    countries: u32,
    regions: u32,
    side_a: u32,
    side_b: u32,
    first: u32,
}

impl Shape {
    fn members(&self) -> std::ops::Range<u32> {
        self.first..self.first + self.side_a + self.side_b
    }
}

struct ScaleWorld {
    world: World,
    globals: BTreeMap<usize, Vec<u8>>,
    regions: Vec<TransportRegion>,
    /// Navy pointer and regional operand array by canonical slot (0: none).
    navies: Vec<usize>,
    operands: Vec<usize>,
    max_id: u32,
}

impl Memory for ScaleWorld {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some((&start, bytes)) = self.globals.range(..=address).next_back()
            && let Some(bytes) = bytes.get(address - start..address - start + output.len())
        {
            output.copy_from_slice(bytes);
            return true;
        }
        self.world.memory().copy(address, output)
    }
}

fn operand(navy: u32, region: u32) -> i64 {
    i64::from((navy * 7_919 + region * 104_729) % 40_000) + 1_000
}

impl ScaleWorld {
    fn new(shape: &Shape) -> Self {
        let mut world = World::new(MODULE, shape.countries, 0x1000);
        let state = world.game_state();
        let slots = shape.countries + 1;
        let members = shape.members();
        let side_b = shape.first + shape.side_a;
        // Strategic regions occupy runtime slots 1..=regions; ids skip every
        // 50th value, so the id domain has gaps like a modded map.
        let region_array = world.alloc((shape.regions as usize + 1) * 8);
        world.array(
            state + cgame_state::STRATEGIC_REGIONS,
            region_array,
            shape.regions + 1,
        );
        let mut regions = Vec::new();
        let mut max_id = 0;
        for slot in 1..=shape.regions {
            let id = slot + slot / 50;
            max_id = id;
            let pointer = world.alloc(0x80);
            world.u64(pointer, (MODULE + cstrategic_region::VTABLE_RVA) as u64);
            world.u32(pointer + cstrategic_region::ID, id);
            world.u32(pointer + cstrategic_region::RUNTIME_SLOT, slot);
            world.u64(region_array + slot as usize * 8, pointer as u64);
            regions.push(TransportRegion { id, pointer });
        }
        // Per-region weather rows indexed by region id; most carry no speed key.
        let weather = world.alloc(0x80);
        world.u64(state + cgame_state::WEATHER_MANAGER, weather as u64);
        world.u64(weather, (MODULE + cweather_manager::VTABLE_RVA) as u64);
        let rows = world.alloc((max_id as usize + 1) * sweather_per_region::STRIDE);
        world.array(weather + cweather_manager::REGIONS, rows, max_id + 1);
        for id in 0..=max_id {
            let row = rows + id as usize * sweather_per_region::STRIDE;
            world.u64(row, (MODULE + sweather_per_region::VTABLE_RVA) as u64);
            world.u32(row + sweather_per_region::ID, id);
            if id % 7 == 0 {
                let entry = world.alloc(16);
                world.array(row + sweather_per_region::MODIFIERS, entry, 1);
                world.u32(entry, SPEED_TOKEN);
                world.u64(entry + 8, (-(i64::from(id % 5) * 5_000)) as u64);
            }
        }
        // Every country has a navy in its canonical manager slot.
        let manager = world.alloc(0x40);
        world.u64(state + cgame_state::STRATEGIC_NAVY_MANAGER, manager as u64);
        world.u64(
            manager,
            (MODULE + cstrategic_navy_manager::VTABLE_RVA) as u64,
        );
        let navy_slots = world.alloc(slots as usize * 8);
        world.array(manager + cstrategic_navy_manager::NAVIES, navy_slots, slots);
        let mut navies = vec![0; slots as usize];
        let mut operands = vec![0; slots as usize];
        for slot in 1..slots {
            let navy = world.alloc(0x100);
            world.u64(navy_slots + slot as usize * 8, navy as u64);
            world.u64(navy, (MODULE + cstrategic_navy::VTABLE_RVA) as u64);
            world.u64(
                navy + cstrategic_navy::SECONDARY,
                (MODULE + cstrategic_navy::SECONDARY_VTABLE_RVA) as u64,
            );
            world.u32(navy + cstrategic_navy::OWNER, slot);
            navies[slot as usize] = navy;
            if members.contains(&slot) {
                let values = world.alloc((max_id as usize + 1) * 8);
                world.array(
                    navy + cstrategic_navy::REGIONAL_OPERANDS,
                    values,
                    max_id + 1,
                );
                for id in 0..=max_id {
                    world.u64(values + id as usize * 8, operand(slot, id) as u64);
                }
                operands[slot as usize] = values;
            }
        }
        // Coalition members: diplomacy, enemy rosters, intel and fog.
        for slot in members.clone() {
            let country = world.object(slot);
            let diplomacy = world.alloc(cdiplomacy_status::CAPITULATED + 0x20);
            world.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
            world.u64(diplomacy, (MODULE + cdiplomacy_status::VTABLE_RVA) as u64);
            world.u64(diplomacy + cdiplomacy_status::COUNTRY, country as u64);
            let enemies: Vec<u32> = if slot < side_b {
                (side_b..members.end).collect()
            } else {
                (members.start..side_b).collect()
            };
            let roster = world.alloc(enemies.len() * 4 + 4);
            world.array(
                diplomacy + cdiplomacy_status::ENEMY_COUNTRIES,
                roster,
                enemies.len() as u32,
            );
            for (index, enemy) in enemies.iter().enumerate() {
                world.u32(roster + index * 4, *enemy);
            }
            // Native gates: some enemies have no fog network or capitulated.
            world.u32(
                country + ccountry::FOG_NETWORK_TABLE_COUNT,
                u32::from(slot % 11 != 0),
            );
            if slot % 13 == 0 {
                world.put(diplomacy + cdiplomacy_status::CAPITULATED, &[1]);
            }
            let intel = world.alloc(0x40);
            world.u64(country + ccountry::INTEL_TABLE, intel as u64);
            world.u64(intel, (MODULE + ccountry_intel::VTABLE_RVA) as u64);
            world.u32(intel + ccountry_intel::OWNER, slot);
            let data = world.alloc(members.end as usize * country_intel_row::STRIDE);
            world.array(intel + ccountry_intel::DATA, data, members.end);
            for target in members.clone() {
                world.u64(
                    data + target as usize * country_intel_row::STRIDE + country_intel_row::NAVY,
                    u64::from((slot * 31 + target * 17) % 90) * 1_000,
                );
            }
            if slot % 3 == 0 {
                let entry = world.alloc(16);
                world.array(country + ccountry::MODIFIER_ACCUMULATOR, entry, 1);
                world.u32(entry, REDUCTION_TOKEN);
                world.u64(entry + 8, 10_000);
            }
        }
        let set = world.alloc(TRANSPORT_MODIFIER + 8);
        let definition = world.alloc(0x40);
        world.u64(set + TRANSPORT_MODIFIER, definition as u64);
        world.u64(definition, (MODULE + cstatic_modifier::VTABLE_RVA) as u64);
        let entry = world.alloc(16);
        world.array(definition + cstatic_modifier::ACCUMULATOR, entry, 1);
        world.u32(entry, SPEED_TOKEN);
        world.u64(entry + 8, (-50_000_i64) as u64);
        let metadata = world.alloc((REDUCTION_TOKEN as usize + 1) * modifier_definition::STRIDE);
        world.u32(
            metadata
                + SPEED_TOKEN as usize * modifier_definition::STRIDE
                + modifier_definition::CATEGORY_MASK,
            1,
        );
        let mut globals = BTreeMap::new();
        for (rva, value) in [
            (MODIFIER_SET_RVA, set as u64),
            (modifier_definition::TABLE_RVA, metadata as u64),
            (modifier_definition::TRADE_FEATURE_FLAGS_RVA, 1 << 4),
            (BASE_SPEED_RVA, 200_000),
            (MAXIMUM_TOTAL_RVA, 400_000),
            (MINIMUM_INTEL_RVA, 0),
            (MAXIMUM_INTEL_RVA, 100_000),
            (INTEL_REDUCTION_RVA, 50_000),
        ] {
            globals.insert(MODULE + rva, value.to_le_bytes().to_vec());
        }
        Self {
            world,
            globals,
            regions,
            navies,
            operands,
            max_id,
        }
    }

    fn state(&self) -> usize {
        self.world.game_state()
    }

    fn country(&self, raw: u32) -> ccountry::Country {
        ccountry::Reader::new(self, MODULE, self.state())
            .unwrap()
            .resolve(raw)
            .unwrap()
    }

    /// The navy's regional operand array for this exact region id.
    fn operand_at(&self, navy: u32, id: u32) -> usize {
        self.operands[navy as usize] + id as usize * 8
    }

    fn operand_reads(&self, reads: &[(usize, usize)]) -> usize {
        let span = (self.max_id as usize + 1) * 8;
        reads
            .iter()
            .filter(|&&(at, _)| {
                self.operands
                    .iter()
                    .any(|&values| values != 0 && values <= at && at < values + span)
            })
            .count()
    }
}

/// Records every native copy and can fail chosen byte ranges.
struct Counted<'a> {
    source: &'a ScaleWorld,
    faults: Vec<(usize, usize)>,
    reads: RefCell<Vec<(usize, usize)>>,
    copies: Cell<usize>,
}

impl<'a> Counted<'a> {
    fn new(source: &'a ScaleWorld, faults: Vec<(usize, usize)>) -> Self {
        Self {
            source,
            faults,
            reads: RefCell::new(Vec::new()),
            copies: Cell::new(0),
        }
    }

    fn attempts(&self, address: usize, len: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|&&(at, width)| at <= address && address + len <= at + width)
            .count()
    }
}

impl Memory for Counted<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.copies.set(self.copies.get() + 1);
        self.reads.borrow_mut().push((address, output.len()));
        if self
            .faults
            .iter()
            .any(|&(start, len)| address < start + len && start < address + output.len())
        {
            return false;
        }
        self.source.copy(address, output)
    }
}

/// Every owner's complete region domain through one hourly packet.
fn capture_all(
    memory: &impl Memory,
    world: &ScaleWorld,
    owners: &[ccountry::Country],
    regions: &[TransportRegion],
    packet: &mut CountryMovementPacket,
) -> Vec<RegionTransportSpeed> {
    let mut rows = vec![RegionTransportSpeed::default(); owners.len() * regions.len()];
    for (index, owner) in owners.iter().enumerate() {
        let capture = capture_transport(
            memory,
            MODULE,
            world.state(),
            *owner,
            regions,
            &mut rows[index * regions.len()..(index + 1) * regions.len()],
            packet,
        );
        assert_eq!(capture.len, Ok(regions.len()));
    }
    rows
}

/// The unshared reader: one complete native query per owner and region.
fn direct_all(
    memory: &impl Memory,
    world: &ScaleWorld,
    owners: &[ccountry::Country],
    regions: &[TransportRegion],
) -> Vec<RegionTransportSpeed> {
    let mut rows = Vec::new();
    for owner in owners {
        for region in regions {
            let result = cstrategic_region::id(memory, MODULE, region.pointer)
                .and_then(|id| {
                    (id == Some(region.id))
                        .then_some(())
                        .ok_or("country movement source-region key mismatch")
                })
                .and_then(|()| {
                    movement_speed(memory, MODULE, world.state(), owner.id, region.pointer)
                });
            rows.push(RegionTransportSpeed {
                region: region.id,
                speed: result.ok(),
                failure: result.err(),
            });
        }
    }
    rows
}

fn late_war() -> Shape {
    Shape {
        countries: 440,
        regions: 304,
        side_a: 60,
        side_b: 25,
        first: 1,
    }
}

fn movers(world: &ScaleWorld, shape: &Shape) -> Vec<ccountry::Country> {
    shape.members().map(|raw| world.country(raw)).collect()
}

fn packet_for(world: &ScaleWorld, owners: usize, slots: usize) -> CountryMovementPacket {
    let mut packet = CountryMovementPacket::try_new(slots).unwrap();
    packet
        .reserve_transport(owners * world.regions.len(), world.regions.len())
        .unwrap();
    packet
}

#[test]
fn coalition_transport_rows_equal_the_unshared_native_reader() {
    let shape = Shape {
        countries: 200,
        regions: 160,
        side_a: 24,
        side_b: 10,
        first: 1,
    };
    let world = ScaleWorld::new(&shape);
    let owners = movers(&world, &shape);
    let mut packet = packet_for(&world, owners.len(), shape.countries as usize + 1);
    let expected = direct_all(&world, &world, &owners, &world.regions);
    assert!(expected.iter().all(|row| row.failure.is_none()));
    // Enemy gates, reductions and intel give distinct totals per owner.
    assert!(
        expected
            .iter()
            .map(|row| row.speed)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 100
    );
    for _ in 0..2 {
        let memory = Counted::new(&world, Vec::new());
        assert_eq!(
            capture_all(&memory, &world, &owners, &world.regions, &mut packet),
            expected
        );
        // Each enemy navy copies its consecutive region ids in batches, not
        // one read per enemy and region (25,840 element reads here).
        let runs = world.regions.len() / OPERAND_BATCH + world.regions.len() / 50 + 2;
        let navies = shape.members().len();
        let reads = world.operand_reads(&memory.reads.borrow());
        assert!(reads <= navies * runs, "{reads} operand copies");
        assert!(reads >= navies);
        packet.clear();
    }
}

#[test]
fn operand_faults_refuse_only_their_own_regions_like_the_unshared_reader() {
    let shape = Shape {
        countries: 60,
        regions: 120,
        side_a: 9,
        side_b: 6,
        first: 1,
    };
    let mut world = ScaleWorld::new(&shape);
    // One navy's array ends early: later region ids are outside it.
    let short = shape.first + 1;
    let navy = world.navies[short as usize];
    world.world.array(
        navy + cstrategic_navy::REGIONAL_OPERANDS,
        world.operands[short as usize],
        70,
    );
    let owners = movers(&world, &shape);
    // A single unreadable element inside a copied run of another navy, and
    // an unreadable owner diplomacy object.
    let enemy = shape.first + shape.side_a + 2;
    let faulted = world.regions[30].id;
    let faults = vec![
        (world.operand_at(enemy, faulted), 8),
        (
            world.world.object(shape.first + 2) + ccountry::DIPLOMACY_STATUS,
            8,
        ),
    ];
    let direct = Counted::new(&world, faults.clone());
    let expected = direct_all(&direct, &world, &owners, &world.regions);
    let failures = |rows: &[RegionTransportSpeed], reason| {
        rows.iter()
            .filter(|row| row.failure == Some(reason))
            .count()
    };
    assert!(failures(&expected, OPERAND_UNREADABLE) > 0);
    assert!(failures(&expected, "navy regional operand outside array") > 0);
    assert!(failures(&expected, "transport input unreadable") >= world.regions.len());
    let mut packet = packet_for(&world, owners.len(), shape.countries as usize + 1);
    let shared = Counted::new(&world, faults);
    assert_eq!(
        capture_all(&shared, &world, &owners, &world.regions, &mut packet),
        expected
    );
    // The failed run falls back to this region's own element, once per pass.
    let element = world.operand_at(enemy, faulted);
    assert_eq!(shared.attempts(element, 8), 2);
    assert!(direct.attempts(element, 8) > 2);
    // A neighbour in the same failed run reads its own element once.
    assert_eq!(shared.attempts(element + 8, 8), 2);
}

#[test]
fn five_thousand_countries_and_a_small_packet_keep_every_transport_row() {
    let shape = Shape {
        countries: 5_000,
        regions: 40,
        side_a: 9,
        side_b: 7,
        first: 4_980,
    };
    let world = ScaleWorld::new(&shape);
    let owners = movers(&world, &shape);
    let expected = direct_all(&world, &world, &owners, &world.regions);
    assert!(expected.iter().all(|row| row.speed.is_some()));
    // Live sizing gives every canonical slot its own navy and owner entry;
    // an undersized packet shares masked slots and keeps the per-operand path.
    for slots in [5_001, 8] {
        let mut packet = packet_for(&world, owners.len(), slots);
        assert_eq!(
            capture_all(&world, &world, &owners, &world.regions, &mut packet),
            expected,
            "{slots} packet slots"
        );
    }
}

#[test]
fn rebinding_a_different_domain_keeps_exact_rows_for_each_domain() {
    let shape = Shape {
        countries: 40,
        regions: 90,
        side_a: 5,
        side_b: 4,
        first: 1,
    };
    let world = ScaleWorld::new(&shape);
    let owners = movers(&world, &shape);
    let subset: Vec<_> = world.regions.iter().copied().step_by(3).collect();
    let mut packet = packet_for(&world, owners.len(), shape.countries as usize + 1);
    for regions in [&world.regions[..], &subset, &world.regions[..]] {
        for owner in &owners {
            let owner = std::slice::from_ref(owner);
            assert_eq!(
                capture_all(&world, &world, owner, regions, &mut packet),
                direct_all(&world, &world, owner, regions)
            );
        }
    }
}

#[test]
#[ignore = "synthetic transport memo cost; run release with --nocapture"]
fn late_war_transport_capture_cost() {
    let shape = late_war();
    let world = ScaleWorld::new(&shape);
    let owners = movers(&world, &shape);
    let mut packet = packet_for(&world, owners.len(), shape.countries as usize + 1);
    let mut samples = Vec::new();
    for _ in 0..21 {
        let started = std::time::Instant::now();
        let rows = capture_all(&world, &world, &owners, &world.regions, &mut packet);
        samples.push(started.elapsed().as_secs_f64() * 1e3);
        assert_eq!(rows.len(), owners.len() * world.regions.len());
        packet.clear();
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "transport owners={} regions={} enemies_per_owner={}/{} p50_ms={:.3} max_ms={:.3} packet_bytes={}",
        owners.len(),
        world.regions.len(),
        shape.side_b,
        shape.side_a,
        samples[samples.len() / 2],
        samples[samples.len() - 1],
        packet.allocated_bytes(),
    );
}
