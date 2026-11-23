
use super::{
    ANTI_AIR_BASE, BASE_ORGANISATION, DEFINITION_VALID, EQUIPMENT_POOL, FALLBACK_SPRITE,
    FORMATION_FLAGS, MODEL_NUMBER, SUBUNIT_DEFINITION, SUBUNIT_INDEX, TASK_FORCE,
};
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{Raw, RecordLayout, RecordSources, scalar_from_bytes},
    msvc_string,
};
use crate::{
    cequipment_variant::{self, pool as cequipment_variant_pool},
    cgame_state,
    ctask_force,
    cunit,
};
use engine_native_parts::equipment_entry;
use std::cmp::Ordering;

const WIDTH: usize = msvc_string::STRIDE;

const WEATHER_FACTOR: usize = 0x270;
const MODIFIERS: usize = 0x750;
const EXPERIENCE: usize = 0x8f8;
const CRITICALS: usize = 0x918;
const PROVINCE_DEFINITION: usize = 0xc8;

const SHIP: [(usize, usize); 16] = [
    (0, 16),
    (EQUIPMENT_POOL, 8),
    (EQUIPMENT_POOL + cequipment_variant_pool::ITEMS, 16),
    (SUBUNIT_DEFINITION, 12),
    (DEFINITION_VALID, 1),
    (FALLBACK_SPRITE, msvc_string::STRIDE),
    (ANTI_AIR_BASE, 8),
    (WEATHER_FACTOR, 8),
    (BASE_ORGANISATION, 16),
    (SUBUNIT_INDEX, 4),
    (FORMATION_FLAGS, 8),
    (TASK_FORCE, 16),
    (MODIFIERS, 16),
    (MODEL_NUMBER, 4),
    (EXPERIENCE, 8),
    (CRITICALS, 16),
];
const SHIP_TASK_FORCE: usize = 11;
const SHIP_SPRITE: usize = 5;
const SHIP_MEDALS: usize = 14;
const SHIP_POOL: usize = 1;
const SHIP_POOL_ITEMS: usize = 2;
const SHIP_SUBUNIT: usize = 3;

const POOL_ENTRIES: usize = WIDTH / equipment_entry::STRIDE;

const VARIANT: [(usize, usize); 8] = [
    (0, 16),
    (cequipment_variant::EQUIPMENT_TOKEN, 4),
    (0xc4, 4),
    (cequipment_variant::DEFINITION, 8),
    (cequipment_variant::CLASSIFIER_FLAGS, 8),
    (0x414, 8),
    (cequipment_variant::VERSION, 1),
    (0x4b0, 4),
];
const NAVAL_POWER: [(usize, usize); 3] = [
    (super::naval_power::HEAVY_ATTACK, 8),
    (super::naval_power::SPEED_RANGE, 16),
    (super::naval_power::CARRIER_SIZE, 8),
];
const VARIANT_FLAGS: usize = 4;

fn naval_power_fields(variant: &Object<8>) -> Option<[(usize, usize); 3]> {
    if variant.completed <= VARIANT_FLAGS {
        return None;
    }
    let mut fields = NAVAL_POWER;
    fields[2].0 += usize::from(
        variant.word(VARIANT_FLAGS) as u64 & super::naval_power::CARRIER_SIZE_SELECTOR != 0,
    ) * 8;
    Some(fields)
}

fn same_naval_power(variant: &Object<8>, left: &Object<3>, right: &Object<3>) -> bool {
    if left.pointer == 0 && right.pointer == 0 {
        return true;
    }
    naval_power_fields(variant).is_some_and(|fields| left.same(right, &fields))
}
const VARIANT_SLOTS: usize = 16;

const SPRITE_BYTES: usize = super::MODEL_TEXT_CAPACITY + 1;

const MEDAL_MODIFIERS: usize = 0x1b0;

const TASK: [(usize, usize); 7] = [
    (0, 8),
    (cunit::HANDLE, 8),
    (cunit::ROSTER_OWNER, 4),
    (cunit::LOCATION, 8),
    (ctask_force::FLEET, 8),
    (MINE_EXPOSURE, 8),
    (ctask_force::ATTACHED_HEADQUARTERS, 16),
];
const MINE_EXPOSURE: usize = 0x650;
const TASK_MODIFIERS: usize = 0x668;
const TASK_LOCATION: usize = 3;

const LOCATION: [(usize, usize); 2] = [(0, 8), (PROVINCE_DEFINITION, 8)];

const GLOBALS: [(usize, usize); 1] = [(0, 8)];

#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum Lookup<'a> {
    Served(&'a [u8]),
    Failed,
    Source,
}

pub trait Served {
    fn lookup(&self, address: usize, len: usize) -> Lookup<'_>;
}

pub struct GatheredObject<const N: usize> {
    object: Object<N>,
    fields: [(usize, usize); N],
}

impl<const N: usize> GatheredObject<N> {
    pub fn gather(memory: &impl Memory, pointer: usize, fields: [(usize, usize); N]) -> Self {
        debug_assert!(fields.iter().all(|&(_, len)| len <= WIDTH));
        debug_assert!(
            fields
                .windows(2)
                .all(|pair| pair[0].0 + pair[0].1 <= pair[1].0)
        );
        Self {
            object: Object::gather(memory, pointer, fields),
            fields,
        }
    }

    pub fn complete(&self) -> bool {
        self.object.completed >= N
    }

    pub fn memory<'a, M: Memory>(&'a self, source: &'a M) -> GatheredMemory<'a, M, Self> {
        GatheredMemory {
            source,
            fields: self,
        }
    }
}

impl<const N: usize> Served for GatheredObject<N> {
    fn lookup(&self, address: usize, len: usize) -> Lookup<'_> {
        self.object.lookup(&self.fields, address, len)
    }
}

#[derive(Clone, Copy)]
struct Object<const N: usize> {
    pointer: usize,
    completed: usize,
    bytes: [[u8; WIDTH]; N],
}

impl<const N: usize> Object<N> {
    const NONE: Self = Self {
        pointer: 0,
        completed: 0,
        bytes: [[0; WIDTH]; N],
    };

    fn gather(memory: &impl Memory, pointer: usize, fields: [(usize, usize); N]) -> Self {
        if pointer == 0 {
            return Self::NONE;
        }
        let mut object = Self {
            pointer,
            completed: 0,
            bytes: [[0; WIDTH]; N],
        };
        let addresses = fields.map(|(offset, _)| pointer.checked_add(offset).unwrap_or(0));
        let mut index = 0;
        let mut destinations = object.bytes.each_mut().map(|bytes| {
            let len = fields[index].1;
            index += 1;
            &mut bytes[..len]
        });
        object.completed = memory
            .copy_scattered_prefetched(addresses, &mut destinations)
            .min(N);
        object
    }

    fn same(&self, other: &Self, fields: &[(usize, usize); N]) -> bool {
        self.pointer == other.pointer
            && self.completed == other.completed
            && fields[..self.completed.min(N)]
                .iter()
                .enumerate()
                .all(|(index, &(_, len))| self.bytes[index][..len] == other.bytes[index][..len])
    }

    fn word(&self, index: usize) -> usize {
        if index < self.completed {
            usize::try_from(u64::from_le_bytes(
                self.bytes[index][..8].try_into().unwrap(),
            ))
            .unwrap_or(0)
        } else {
            0
        }
    }

    fn lookup(&self, fields: &[(usize, usize); N], address: usize, len: usize) -> Lookup<'_> {
        if self.pointer == 0 || len == 0 {
            return Lookup::Source;
        }
        let Some(offset) = address.checked_sub(self.pointer) else {
            return Lookup::Source;
        };
        let Some(end) = offset.checked_add(len) else {
            return Lookup::Source;
        };
        if fields
            .last()
            .is_none_or(|&(start, width)| offset >= start + width)
        {
            return Lookup::Source;
        }
        let Some(index) = fields
            .partition_point(|&(start, _)| start <= offset)
            .checked_sub(1)
        else {
            return Lookup::Source;
        };
        let (start, width) = fields[index];
        if end > start + width {
            return Lookup::Source;
        }
        match index.cmp(&self.completed) {
            Ordering::Less => Lookup::Served(&self.bytes[index][offset - start..end - start]),
            Ordering::Equal if offset == start && len == width => Lookup::Failed,
            _ => Lookup::Source,
        }
    }
}

#[derive(Clone, Copy)]
struct Heap {
    spans: [(usize, usize); 2],
    completed: usize,
    pool: [u8; WIDTH],
    sprite: [u8; SPRITE_BYTES],
}

impl Heap {
    const NONE: Self = Self {
        spans: [(0, 0); 2],
        completed: 0,
        pool: [0; WIDTH],
        sprite: [0; SPRITE_BYTES],
    };

    fn gather(memory: &impl Memory, base: usize, ship: &Object<16>) -> Self {
        let mut heap = Self::NONE;
        let vtable = engine_sites::at(base, cequipment_variant_pool::VTABLE_RVA).unwrap_or(0);
        if ship.completed > SHIP_POOL_ITEMS
            && ship.word(SHIP_POOL) == vtable
            && let Some(array) = Array::from_header(
                ship.bytes[SHIP_POOL_ITEMS][..16].try_into().unwrap(),
                equipment_entry::STRIDE,
            )
            .filter(|array| (1..=POOL_ENTRIES).contains(&(array.count as usize)))
        {
            heap.spans[0] = (array.data, array.count as usize * equipment_entry::STRIDE);
        }
        if ship.completed > SHIP_SPRITE
            && let Some(span) = msvc_string::heap_span(&ship.bytes[SHIP_SPRITE])
                .filter(|&(_, len)| len <= SPRITE_BYTES)
        {
            heap.spans[1] = span;
        }
        if heap.spans.iter().all(|&(_, len)| len == 0) {
            return heap;
        }
        let [(_, pool), (_, sprite)] = heap.spans;
        heap.completed = memory
            .copy_scattered_prefetched(
                heap.spans.map(|(address, _)| address),
                &mut [&mut heap.pool[..pool], &mut heap.sprite[..sprite]],
            )
            .min(2);
        heap
    }

    fn bytes(&self, index: usize) -> &[u8] {
        let len = self.spans[index].1;
        if index == 0 {
            &self.pool[..len]
        } else {
            &self.sprite[..len]
        }
    }

    fn entries(&self) -> Option<&[u8]> {
        (self.spans[0].1 != 0 && self.completed > 0).then(|| self.bytes(0))
    }

    fn lookup(&self, address: usize, len: usize) -> Lookup<'_> {
        for (index, &(start, width)) in self.spans.iter().enumerate() {
            if width == 0 || len == 0 {
                continue;
            }
            let Some(offset) = address.checked_sub(start) else {
                continue;
            };
            if offset.checked_add(len).is_none_or(|end| end > width) {
                continue;
            }
            return match index.cmp(&self.completed) {
                Ordering::Less => Lookup::Served(&self.bytes(index)[offset..offset + len]),
                Ordering::Equal if offset == 0 && len == width => Lookup::Failed,
                _ => Lookup::Source,
            };
        }
        Lookup::Source
    }

    fn same(&self, other: &Self) -> bool {
        self.spans == other.spans
            && self.completed == other.completed
            && (0..self.completed.min(2)).all(|index| self.bytes(index) == other.bytes(index))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskForceFields {
    pub pointer: usize,
    pub vtable: u64,
    pub handle: u64,
    pub roster_owner: u32,
    pub location: usize,
    pub fleet: usize,
    pub mine_exposure: u64,
    pub modifiers: usize,
    pub influences_count: Option<u32>,
}

#[derive(Clone, Copy)]
pub struct HeldShip {
    ship: Object<16>,
    heap: Heap,
    variants: [Object<8>; POOL_ENTRIES],
    naval_variants: [Object<3>; POOL_ENTRIES],
}

#[derive(Clone, Copy)]
pub struct HeldVariant(Object<8>, Object<3>);

impl HeldVariant {
    pub fn pointer(&self) -> usize {
        self.0.pointer
    }

    pub fn same(&self, other: &Self) -> bool {
        self.0.same(&other.0, &VARIANT) && same_naval_power(&self.0, &self.1, &other.1)
    }

    pub fn complete(&self) -> bool {
        variant_complete(&self.0) && (self.1.pointer == 0 || self.1.completed == NAVAL_POWER.len())
    }
}

impl HeldShip {
    pub fn variants_complete(&self) -> bool {
        pool_complete(&self.ship, &self.heap)
            && self.variants.iter().all(variant_complete)
            && self
                .naval_variants
                .iter()
                .all(|v| v.pointer == 0 || v.completed == NAVAL_POWER.len())
    }

    pub fn revalidation_complete(&self) -> bool {
        self.ship.completed == SHIP.len()
            && pool_complete(&self.ship, &self.heap)
            && msvc_string::heap_span(&self.ship.bytes[SHIP_SPRITE])
                .is_none_or(|span| self.heap.completed > 1 && self.heap.spans[1] == span)
    }

    pub fn variants(&self) -> [HeldVariant; POOL_ENTRIES] {
        std::array::from_fn(|index| HeldVariant(self.variants[index], self.naval_variants[index]))
    }

    pub fn task_force(&self) -> usize {
        self.ship.word(SHIP_TASK_FORCE)
    }

    pub fn carrier_air_base(&self) -> usize {
        if self.ship.completed > SHIP_TASK_FORCE {
            u64::from_le_bytes(self.ship.bytes[SHIP_TASK_FORCE][8..16].try_into().unwrap()) as usize
        } else {
            0
        }
    }

    pub fn medal_store(&self) -> usize {
        self.ship.word(SHIP_MEDALS)
    }

    pub fn accumulators(&self) -> [usize; 2] {
        let store = self.medal_store();
        [
            self.ship.pointer.saturating_add(MODIFIERS),
            if store == 0 {
                0
            } else {
                store.saturating_add(MEDAL_MODIFIERS)
            },
        ]
    }

    pub fn same(&self, other: &Self) -> bool {
        self.revalidation_complete()
            && other.revalidation_complete()
            && self.ship.same(&other.ship, &SHIP)
            && self.heap.same(&other.heap)
    }

    pub fn same_model(&self, other: &Self, class_id: i32) -> bool {
        let class = super::CLASS_DATABASE_ID - SUBUNIT_DEFINITION;
        self.revalidation_complete()
            && other.revalidation_complete()
            && self.ship.pointer == other.ship.pointer
            && self.ship.bytes[SHIP_SUBUNIT][class..class + size_of::<i32>()]
                == class_id.to_le_bytes()
            && SHIP.iter().enumerate().all(|(index, &(offset, len))| {
                if matches!(offset, WEATHER_FACTOR | MODIFIERS | EXPERIENCE | CRITICALS) {
                    return true;
                }
                let start = if offset == BASE_ORGANISATION {
                    super::MAXIMUM_STRENGTH - BASE_ORGANISATION
                } else {
                    0
                };
                self.ship.bytes[index][start..len] == other.ship.bytes[index][start..len]
            })
            && self.heap.same(&other.heap)
    }
}

fn variant_complete(variant: &Object<8>) -> bool {
    variant.pointer == 0 || variant.completed == VARIANT.len()
}

fn pool_complete(ship: &Object<16>, heap: &Heap) -> bool {
    ship.completed > SHIP_POOL_ITEMS
        && Array::from_header(
            ship.bytes[SHIP_POOL_ITEMS][..16].try_into().unwrap(),
            equipment_entry::STRIDE,
        )
        .is_some_and(|array| {
            array.count == 0
                || heap.completed > 0
                    && heap.spans[0] == (array.data, array.count as usize * equipment_entry::STRIDE)
        })
}

pub struct ShipFields {
    base: usize,
    ship: Object<16>,
    task: Object<7>,
    location: Object<2>,
    globals: Object<1>,
    heap: Heap,
    variants: [Object<8>; VARIANT_SLOTS],
    naval_variants: [Object<3>; VARIANT_SLOTS],
    naval_power: bool,
    next_variant: usize,
    current: [Option<usize>; POOL_ENTRIES],
    current_power: Option<usize>,
}

impl ShipFields {
    pub fn new(base: usize) -> Self {
        Self {
            base,
            ship: Object::NONE,
            task: Object::NONE,
            location: Object::NONE,
            globals: Object::NONE,
            heap: Heap::NONE,
            variants: [Object::NONE; VARIANT_SLOTS],
            naval_variants: [Object::NONE; VARIANT_SLOTS],
            naval_power: false,
            next_variant: 0,
            current: [None; POOL_ENTRIES],
            current_power: None,
        }
    }

    pub fn enable_naval_power(&mut self) {
        self.naval_power = true;
    }

    pub fn visit(&mut self, memory: &impl Memory, ship: usize) {
        self.ship = Object::gather(memory, ship, SHIP);
        self.visit_shared(memory);
        self.heap = Heap::gather(memory, self.base, &self.ship);
        self.visit_variants(memory);
    }

    pub fn visit_held(&mut self, memory: &impl Memory, held: &HeldShip) {
        if !held.revalidation_complete() {
            self.visit(memory, held.ship.pointer);
            return;
        }
        self.ship = held.ship;
        self.visit_shared(memory);
        self.heap = held.heap;
        self.visit_variants(memory);
    }

    pub fn visit_held_certified(&mut self, memory: &impl Memory, held: &HeldShip) {
        if !held.revalidation_complete() {
            self.visit(memory, held.ship.pointer);
            return;
        }
        self.ship = held.ship;
        self.visit_shared(memory);
        self.heap = held.heap;
        self.current = [None; POOL_ENTRIES];
        self.current_power = None;
        for (index, (variant, power)) in held.variants.iter().zip(&held.naval_variants).enumerate()
        {
            if variant.pointer == 0 {
                continue;
            }
            let slot = self.slot(variant.pointer).unwrap_or_else(|slot| slot);
            self.variants[slot] = *variant;
            self.naval_variants[slot] = *power;
            self.current[index] = Some(slot);
            if power.pointer != 0 {
                self.current_power = Some(index);
            }
        }
    }

    pub fn task_force(&self) -> Option<TaskForceFields> {
        let word =
            |index: usize| u64::from_le_bytes(self.task.bytes[index][..8].try_into().unwrap());
        (self.task.pointer != 0 && self.task.completed == TASK.len()).then(|| TaskForceFields {
            pointer: self.task.pointer,
            vtable: word(0),
            handle: word(1),
            roster_owner: u32::from_le_bytes(self.task.bytes[2][..4].try_into().unwrap()),
            location: word(3) as usize,
            fleet: word(4) as usize,
            mine_exposure: word(5),
            modifiers: self.task.pointer.saturating_add(TASK_MODIFIERS),
            influences_count: Array::from_header(
                self.task.bytes[6][..16].try_into().unwrap(),
                8,
            )
            .map(|array| array.count),
        })
    }

    pub fn game_state(&self) -> Option<usize> {
        (self.globals.pointer != 0 && self.globals.completed == GLOBALS.len())
            .then(|| self.globals.word(0))
    }

    pub fn held(&self) -> HeldShip {
        HeldShip {
            ship: self.ship,
            heap: self.heap,
            variants: self.current_variants(),
            naval_variants: self.current_naval_variants(),
        }
    }

    pub fn held_variants_unchanged(&mut self, memory: &impl Memory, held: &HeldShip) -> bool {
        let heap = held.heap;
        self.visit_variants_of(memory, &heap);
        held.variants_complete() && self.same_variant_observations(held)
    }

    pub fn held_variant_unchanged(&mut self, memory: &impl Memory, held: &HeldVariant) -> bool {
        if !variant_complete(&held.0) {
            return false;
        }
        if held.pointer() == 0 {
            return true;
        }
        let slot = self.variant(memory, held.pointer());
        if held.1.pointer != 0 {
            self.naval_variant(memory, slot);
        }
        variant_complete(&self.variants[slot])
            && self.variants[slot].same(&held.0, &VARIANT)
            && (held.1.pointer == 0
                || same_naval_power(&self.variants[slot], &self.naval_variants[slot], &held.1)
                    && self.naval_variants[slot].completed == NAVAL_POWER.len())
    }

    pub fn same_variants(&self, held: &HeldShip) -> bool {
        held.variants_complete()
            && pool_complete(&self.ship, &self.heap)
            && self.same_variant_observations(held)
    }

    fn same_variant_observations(&self, held: &HeldShip) -> bool {
        self.current_variants()
            .iter()
            .zip(&held.variants)
            .all(|(current, held)| variant_complete(current) && current.same(held, &VARIANT))
            && self
                .current_naval_variants()
                .iter()
                .zip(&held.naval_variants)
                .zip(self.current_variants())
                .all(|((current, held), variant)| {
                    (current.pointer == 0 || current.completed == NAVAL_POWER.len())
                        && same_naval_power(&variant, current, held)
                })
    }

    fn current_naval_variants(&self) -> [Object<3>; POOL_ENTRIES] {
        std::array::from_fn(|index| {
            if self.current_power == Some(index) {
                self.current[index].map_or(Object::NONE, |slot| self.naval_variants[slot])
            } else {
                Object::NONE
            }
        })
    }

    fn current_variants(&self) -> [Object<8>; POOL_ENTRIES] {
        self.current
            .map(|slot| slot.map_or(Object::NONE, |slot| self.variants[slot]))
    }

    pub fn task_force_of(
        &mut self,
        memory: &impl Memory,
        held: &HeldShip,
    ) -> Option<TaskForceFields> {
        self.visit_task_only(memory, held.task_force());
        self.task_force()
    }

    pub fn task_force_at(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
    ) -> Option<TaskForceFields> {
        self.visit_task_only(memory, pointer);
        self.task_force()
    }

    pub fn visit_location(&mut self, memory: &impl Memory) {
        let location = self.task.word(TASK_LOCATION);
        if location != self.location.pointer {
            self.location = Object::gather(memory, location, LOCATION);
        }
    }

    fn visit_shared(&mut self, memory: &impl Memory) {
        self.visit_task(memory, self.ship.word(SHIP_TASK_FORCE));
    }

    fn visit_task(&mut self, memory: &impl Memory, task: usize) {
        self.visit_task_only(memory, task);
        self.visit_location(memory);
    }

    fn visit_task_only(&mut self, memory: &impl Memory, task: usize) {
        if self.globals.pointer == 0 {
            let singleton = engine_sites::at(self.base, cgame_state::SINGLETON_RVA).unwrap_or(0);
            self.globals = Object::gather(memory, singleton, GLOBALS);
        }
        if task != self.task.pointer {
            self.task = Object::gather(memory, task, TASK);
        }
    }

    fn visit_variants(&mut self, memory: &impl Memory) {
        let heap = self.heap;
        self.visit_variants_of(memory, &heap);
    }

    fn visit_variants_of(&mut self, memory: &impl Memory, heap: &Heap) {
        self.current = [None; POOL_ENTRIES];
        self.current_power = None;
        let Some(copied) = heap.entries() else {
            return;
        };
        let count = copied.len() / equipment_entry::STRIDE;
        let mut entries = [0; WIDTH];
        entries[..copied.len()].copy_from_slice(copied);
        let entry = |index: usize| {
            let bytes = &entries[index * equipment_entry::STRIDE..];
            (
                u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize,
                i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            )
        };
        let mut selected = 0;
        for index in 1..count {
            if entry(index).1 > entry(selected).1 {
                selected = index;
            }
        }
        for index in 0..count {
            let (variant, amount) = entry(index);
            if variant != 0 && (amount > 0 || index == selected) {
                self.current[index] = Some(self.variant(memory, variant));
            }
        }
        if self.naval_power
            && let Some(slot) = self.current[selected]
        {
            self.current_power = Some(selected);
            self.naval_variant(memory, slot);
        }
    }

    fn variant(&mut self, memory: &impl Memory, variant: usize) -> usize {
        match self.slot(variant) {
            Ok(slot) => slot,
            Err(slot) => {
                self.variants[slot] = Object::gather(memory, variant, VARIANT);
                self.naval_variants[slot] = Object::NONE;
                slot
            }
        }
    }

    fn slot(&mut self, variant: usize) -> Result<usize, usize> {
        if let Some(slot) = self
            .variants
            .iter()
            .position(|kept| kept.pointer == variant)
        {
            return Ok(slot);
        }
        while self.current.contains(&Some(self.next_variant)) {
            self.next_variant = (self.next_variant + 1) % VARIANT_SLOTS;
        }
        let slot = self.next_variant;
        self.next_variant = (slot + 1) % VARIANT_SLOTS;
        Err(slot)
    }

    fn naval_variant(&mut self, memory: &impl Memory, slot: usize) {
        if !self.naval_power || self.naval_variants[slot].pointer != 0 {
            return;
        }
        let original = &self.variants[slot];
        let Some(fields) = naval_power_fields(original) else {
            return;
        };
        let Some(expected) = engine_sites::at(self.base, engine_native_parts::equipment_variant_identity::VTABLE_RVA) else {
            return;
        };
        let header: [u8; 16] = original.bytes[0][..16]
            .try_into()
            .expect("variant identity span");
        if engine_native_parts::equipment_variant_identity::identity_from_header(expected, header).is_err() {
            return;
        }
        self.naval_variants[slot] = Object::gather(memory, original.pointer, fields);
    }

    pub fn memory<'a, M: Memory>(&'a self, source: &'a M) -> GatheredMemory<'a, M> {
        GatheredMemory {
            source,
            fields: self,
        }
    }
}

impl Served for ShipFields {
    fn lookup(&self, address: usize, len: usize) -> Lookup<'_> {
        match self.ship.lookup(&SHIP, address, len) {
            Lookup::Source => {}
            found => return found,
        }
        match self.task.lookup(&TASK, address, len) {
            Lookup::Source => {}
            found => return found,
        }
        match self.location.lookup(&LOCATION, address, len) {
            Lookup::Source => {}
            found => return found,
        }
        match self.globals.lookup(&GLOBALS, address, len) {
            Lookup::Source => {}
            found => return found,
        }
        match self.heap.lookup(address, len) {
            Lookup::Source => {}
            found => return found,
        }
        for &slot in self.current.iter().flatten() {
            match self.variants[slot].lookup(&VARIANT, address, len) {
                Lookup::Source => {}
                found => return found,
            }
            let Some(fields) = naval_power_fields(&self.variants[slot]) else {
                continue;
            };
            match self.naval_variants[slot].lookup(&fields, address, len) {
                Lookup::Source => {}
                found => return found,
            }
        }
        Lookup::Source
    }
}

pub struct GatheredMemory<'a, M, F = ShipFields> {
    source: &'a M,
    fields: &'a F,
}

impl<'a, M: Memory, F: Served> GatheredMemory<'a, M, F> {
    pub(crate) fn new(source: &'a M, fields: &'a F) -> Self {
        Self { source, fields }
    }

    fn copy_mixed<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
        prefetched: bool,
    ) -> usize {
        let lookups: [Lookup<'_>; N] = std::array::from_fn(|index| {
            if destinations[index].is_empty() {
                Lookup::Source
            } else {
                self.fields
                    .lookup(addresses[index], destinations[index].len())
            }
        });
        let failed = lookups
            .iter()
            .position(|lookup| matches!(lookup, Lookup::Failed))
            .unwrap_or(N);
        let live = |index: usize| index < failed && matches!(lookups[index], Lookup::Source);
        if failed == N
            && destinations
                .iter()
                .enumerate()
                .all(|(index, destination)| destination.is_empty() || live(index))
        {
            return if prefetched {
                self.source
                    .copy_scattered_prefetched(addresses, destinations)
            } else {
                self.source.copy_scattered(addresses, destinations)
            };
        }
        let copied = if (0..failed).any(|index| !destinations[index].is_empty() && live(index)) {
            let mut spans = destinations.iter_mut().enumerate();
            let mut live_spans: [&mut [u8]; N] = std::array::from_fn(|_| {
                let (index, destination) = spans.next().unwrap();
                if live(index) {
                    &mut **destination
                } else {
                    &mut []
                }
            });
            if prefetched {
                self.source
                    .copy_scattered_prefetched(addresses, &mut live_spans)
            } else {
                self.source.copy_scattered(addresses, &mut live_spans)
            }
        } else {
            N
        };
        let stop = copied.min(failed);
        for (destination, lookup) in destinations.iter_mut().zip(lookups).take(stop) {
            if let Lookup::Served(bytes) = lookup {
                destination.copy_from_slice(bytes);
            }
        }
        stop
    }
}

impl<M: Memory, F: Served> Memory for GatheredMemory<'_, M, F> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        match self.fields.lookup(address, destination.len()) {
            Lookup::Served(bytes) => {
                destination.copy_from_slice(bytes);
                true
            }
            Lookup::Failed => false,
            Lookup::Source => self.source.copy(address, destination),
        }
    }

    fn read_scalar<T: Raw>(&self, address: usize) -> Option<T> {
        match self.fields.lookup(address, size_of::<T>()) {
            Lookup::Served(bytes) => scalar_from_bytes(bytes),
            Lookup::Failed => None,
            Lookup::Source => self.source.read_scalar(address),
        }
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.source.prefetch(address, len);
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.copy_mixed(addresses, destinations, false)
    }

    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.copy_mixed(addresses, destinations, true)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let address = |row: usize, field: &engine_native_memory::memory::RecordField| {
            sources
                .pointer(row)
                .filter(|&pointer| pointer != 0)
                .and_then(|pointer| pointer.checked_add(field.source_offset))
                .filter(|&address| address.checked_add(field.len).is_some())
        };
        let gathered = (0..sources.len()).any(|row| {
            layout.fields().iter().any(|field| {
                field.len != 0
                    && address(row, field).is_some_and(|address| {
                        !matches!(self.fields.lookup(address, field.len), Lookup::Source)
                    })
            })
        });
        if !gathered {
            return self.source.copy_records(sources, layout, destination);
        }
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        if count == 0 || layout.fields().iter().all(|field| field.len == 0) {
            return count;
        }
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields() {
                if field.len != 0 {
                    let Some(address) = address(row, field) else {
                        return completed;
                    };
                    let start = row * layout.stride() + field.destination_offset;
                    if !self.copy(address, &mut destination[start..start + field.len]) {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        completed
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine_native_memory::country_identity::AuthenticatedRoster<'_>> {
        self.source.authenticated_countries(base, game_state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<engine_native_memory::country_identity::Country, &'static str>> {
        self.source.authenticated_reserved_country(base, game_state)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine_native_memory::country_identity::ReservedCountryObservation> {
        self.source
            .authenticated_reserved_country_observation(base, game_state)
    }

    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<engine_native_memory::modifier_cache::Value, &'static str>> {
        self.source.modifier_accumulator_value(address, key)
    }

    fn modifier_definitions(&self) -> Option<&engine_native_memory::modifier_cache::PassDefinitions> {
        self.source.modifier_definitions()
    }
}

pub struct SwitchedObject<'a, M, const N: usize> {
    source: &'a M,
    fields: [(usize, usize); N],
    current: std::cell::RefCell<GatheredObject<N>>,
}

impl<'a, M: Memory, const N: usize> SwitchedObject<'a, M, N> {
    pub fn new(source: &'a M, fields: [(usize, usize); N]) -> Self {
        Self {
            source,
            fields,
            current: std::cell::RefCell::new(GatheredObject::gather(source, 0, fields)),
        }
    }

    pub fn select(&self, pointer: usize) {
        *self.current.borrow_mut() = GatheredObject::gather(self.source, pointer, self.fields);
    }

    pub fn complete(&self) -> bool {
        self.current.borrow().object.completed >= N
    }
}

impl<M: Memory, const N: usize> Memory for SwitchedObject<'_, M, N> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.current
            .borrow()
            .memory(self.source)
            .copy(address, destination)
    }

    fn read_scalar<T: Raw>(&self, address: usize) -> Option<T> {
        self.current
            .borrow()
            .memory(self.source)
            .read_scalar(address)
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.source.prefetch(address, len);
    }

    fn copy_scattered<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        self.current
            .borrow()
            .memory(self.source)
            .copy_scattered(addresses, destinations)
    }

    fn copy_scattered_prefetched<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        self.current
            .borrow()
            .memory(self.source)
            .copy_scattered_prefetched(addresses, destinations)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.current
            .borrow()
            .memory(self.source)
            .copy_records(sources, layout, destination)
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine_native_memory::country_identity::AuthenticatedRoster<'_>> {
        self.source.authenticated_countries(base, game_state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<engine_native_memory::country_identity::Country, &'static str>> {
        self.source.authenticated_reserved_country(base, game_state)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine_native_memory::country_identity::ReservedCountryObservation> {
        self.source
            .authenticated_reserved_country_observation(base, game_state)
    }

    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<engine_native_memory::modifier_cache::Value, &'static str>> {
        self.source.modifier_accumulator_value(address, key)
    }

    fn modifier_definitions(&self) -> Option<&engine_native_memory::modifier_cache::PassDefinitions> {
        self.source.modifier_definitions()
    }
}

#[cfg(test)]
pub(super) struct FaultAt<'a, M> {
    pub(super) source: &'a M,
    pub(super) fault: Option<usize>,
    pub(super) reads: std::cell::RefCell<Vec<(usize, usize)>>,
    pub(super) failed: std::cell::RefCell<Vec<(usize, usize)>>,
}

#[cfg(test)]
impl<'a, M> FaultAt<'a, M> {
    pub(super) fn new(source: &'a M, fault: Option<usize>) -> Self {
        Self {
            source,
            fault,
            reads: std::cell::RefCell::new(Vec::new()),
            failed: std::cell::RefCell::new(Vec::new()),
        }
    }

    pub(super) fn retried(&self) -> bool {
        let failed = self.failed.borrow();
        failed
            .iter()
            .enumerate()
            .any(|(index, read)| failed[..index].contains(read))
    }

    pub(super) fn edges(&self) -> Vec<usize> {
        let mut edges: Vec<usize> = self
            .reads
            .borrow()
            .iter()
            .flat_map(|&(address, len)| [address, address + len.max(1) - 1])
            .collect();
        edges.sort_unstable();
        edges.dedup();
        edges
    }
}

#[cfg(test)]
impl<M: Memory> Memory for FaultAt<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        let touched = self.fault.is_some_and(|fault| {
            address <= fault && fault < address.saturating_add(destination.len())
        });
        if touched || !self.source.copy(address, destination) {
            self.failed.borrow_mut().push((address, destination.len()));
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests;
