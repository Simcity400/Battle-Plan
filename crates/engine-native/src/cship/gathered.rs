//! Fixed fields of a ship, its task force, the task force's location and the
//! game singleton, each object's gathered in one guarded call ahead of the
//! ShipModels readers ([`super::read_model_with_selection`],
//! [`super::read_maximum_organisation_memo`] and the suffix readers).
//!
//! Those readers read about thirty fixed fields of these objects per ship,
//! one guarded call each. [`ShipFields::visit`] gathers each object's fields
//! in one scattered call: the ship's every time, its task force's and the
//! task force's location's when they differ from the previous ship's (ships
//! of a task force are consecutive in the census), and the game singleton
//! once per capture. [`GatheredMemory`] then serves the readers' reads: a
//! read inside a gathered field is served from its bytes, a read of exactly
//! the field a gather failed on fails again without another read, and every
//! other read, including one of a field after that failure, goes to the
//! source, live spans of one scattered read still in one guarded call. Every
//! gathered object is one the readers dereference for that ship, so in the
//! stopped world each served byte is the byte a direct read returns, and the
//! rows and failure reasons are the direct readers' (tests). No
//! byte outlives the capture that gathered it.
//!
//! What a ship's fields point at follows in one more call ([`Heap`]): its
//! equipment pool entries when it holds at most [`POOL_ENTRIES`], as the
//! model reader copies them, and its heap fallback sprite with the
//! terminator. The variants the model reader reads from the entries (each
//! positive entry's and the selected one) are gathered too; ships of a class
//! share variants, so a variant is gathered once per capture while it stays
//! among the latest [`VARIANT_SLOTS`].
//!
//! A ship's own objects ([`HeldShip`]) can also be served from an earlier
//! hour's read ([`ShipFields::visit_held`]) while no hooked writer touched
//! them (`ship_model_mutations`); everything else is read in the
//! hour.

use super::{
    ANTI_AIR_BASE, BASE_ORGANISATION, DEFINITION_VALID, EQUIPMENT_POOL, FALLBACK_SPRITE,
    FORMATION_FLAGS, MODEL_NUMBER, SUBUNIT_DEFINITION, SUBUNIT_INDEX, TASK_FORCE,
};
use crate::{
    Memory, cequipment_variant, cequipment_variant_pool, cgame_state,
    cpdx_array::Array,
    ctask_force, cunit, equipment_entry,
    memory::{Raw, RecordLayout, RecordSources, scalar_from_bytes},
    msvc_string,
};
use std::cmp::Ordering;

/// The widest gathered field, a string header.
const WIDTH: usize = msvc_string::STRIDE;

/// Ship modifier calculation's weather factor, ship modifier accumulator,
/// experience modifier source and critical-effect array, as
/// [`super::read_maximum_organisation_memo`] reads them.
const WEATHER_FACTOR: usize = 0x270;
const MODIFIERS: usize = 0x750;
const EXPERIENCE: usize = 0x8f8;
const CRITICALS: usize = 0x918;
/// The province definition used to select terrain or the ocean fallback.
const PROVINCE_DEFINITION: usize = 0xc8;

/// Every fixed ship field the readers read, each the exact span (or a span
/// enclosing the exact spans) they request: identity (vtable and handle),
/// the equipment pool's vtable and array header, the embedded subunit
/// definition's identity (its class id at +0x88), the definition-valid byte,
/// the fallback sprite's header, the anti-air base, the weather factor, the
/// base organisation with the maximum strength, the subunit index (also the
/// leader-class selector), the formation flags, the task force with the
/// carrier air base, the modifier accumulator's header, the model number,
/// the experience source and the critical array's header.
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
/// The ship field holding the task force pointer, with the carrier air base
/// pointer in its second word.
const SHIP_TASK_FORCE: usize = 11;
/// The ship fields holding the fallback sprite's header and the medal store.
const SHIP_SPRITE: usize = 5;
const SHIP_MEDALS: usize = 14;
/// The ship fields holding the equipment pool's vtable and array header.
const SHIP_POOL: usize = 1;
const SHIP_POOL_ITEMS: usize = 2;

/// Pool entries gathered ahead of the model reader at most: as many as one
/// gathered field holds.
const POOL_ENTRIES: usize = WIDTH / equipment_entry::STRIDE;

/// Variant fields the model and hull-variant readers read: identity (vtable
/// and handle), equipment token, module count, definition, classifier flags,
/// upgrade level and module priority sums, version and organisation marker.
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
/// Variants kept for later ships of the capture.
const VARIANT_SLOTS: usize = 16;

/// The longest heap fallback sprite gathered, with its terminator: the
/// model reader's inline text capacity. A longer one is read by the reader.
const SPRITE_BYTES: usize = super::MODEL_TEXT_CAPACITY + 1;

/// The medal store's modifier accumulator, as ship modifier calculation reads
/// it.
const MEDAL_MODIFIERS: usize = 0x1b0;

/// Task-force fields: vtable, handle, roster owner, location, fleet and the
/// mine exposure (`+0x650`) its modifier accumulator `+0x668` is built from.
const TASK: [(usize, usize); 6] = [
    (0, 8),
    (cunit::HANDLE, 8),
    (cunit::ROSTER_OWNER, 4),
    (cunit::LOCATION, 8),
    (ctask_force::FLEET, 8),
    (MINE_EXPOSURE, 8),
];
/// Mine-exposure updating rebuilds `+0x668` every hour from this value and
/// content alone.
const MINE_EXPOSURE: usize = 0x650;
/// The task-force accumulator ship modifier calculation reads beside the
/// location's terms.
const TASK_MODIFIERS: usize = 0x668;
/// The task-force field holding the location pointer.
const TASK_LOCATION: usize = 3;

/// Location fields: vtable and province definition.
const LOCATION: [(usize, usize); 2] = [(0, 8), (PROVINCE_DEFINITION, 8)];

/// Module globals: the game state singleton.
const GLOBALS: [(usize, usize); 1] = [(cgame_state::SINGLETON_RVA, 8)];

/// Where a read's bytes come from.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum Lookup<'a> {
    Served(&'a [u8]),
    Failed,
    Source,
}

/// Gathered bytes a [`GatheredMemory`] serves ahead of its source: a read
/// inside a gathered field is served, a read of exactly the field a gather
/// failed on fails, and every other read goes to the source.
pub trait Served {
    /// Where a read of `len` bytes at `address` comes from.
    fn lookup(&self, address: usize, len: usize) -> Lookup<'_>;
}

/// One object's fixed fields, gathered in one guarded call ahead of the
/// readers of that object and served to them through [`Self::memory`] as
/// [`ShipFields`] serves a ship's. `fields` are `(offset, len)` spans that
/// ascend, are disjoint and are at most 32 bytes each, and must be fields
/// the readers dereference on that object.
pub struct GatheredObject<const N: usize> {
    object: Object<N>,
    fields: [(usize, usize); N],
}

impl<const N: usize> GatheredObject<N> {
    /// One guarded call; a null `pointer` gathers nothing.
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

    /// Whether the gather copied every field: when not, the readers read
    /// the rest from the source.
    pub fn complete(&self) -> bool {
        self.object.completed >= N
    }

    /// The readers' memory for the gathered object over `source`.
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

/// One object's gathered fields.
#[derive(Clone, Copy)]
struct Object<const N: usize> {
    /// Zero: nothing gathered, and every read goes to the source.
    pointer: usize,
    /// Fields copied, in order. The field at this index, when there is one,
    /// failed, and later fields were not read.
    completed: usize,
    bytes: [[u8; WIDTH]; N],
}

impl<const N: usize> Object<N> {
    const NONE: Self = Self {
        pointer: 0,
        completed: 0,
        bytes: [[0; WIDTH]; N],
    };

    /// One guarded call. An overflowing address is null for the gather, so
    /// that field fails, as a single read of it would.
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

    /// Whether `other` gathered the same object and bytes, field by copied
    /// field.
    fn same(&self, other: &Self, fields: &[(usize, usize); N]) -> bool {
        self.pointer == other.pointer
            && self.completed == other.completed
            && fields[..self.completed.min(N)]
                .iter()
                .enumerate()
                .all(|(index, &(_, len))| self.bytes[index][..len] == other.bytes[index][..len])
    }

    fn differs(&self, other: &Self, fields: &[(usize, usize); N]) -> bool {
        self.pointer == other.pointer
            && fields[..self.completed.min(other.completed).min(N)]
                .iter()
                .enumerate()
                .any(|(index, &(_, len))| self.bytes[index][..len] != other.bytes[index][..len])
    }

    /// A pointer-sized field the gather copied, else zero.
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
        // The fields ascend and are disjoint, so at most one encloses the
        // read, and none past the last one's end.
        if fields
            .last()
            .is_none_or(|&(start, width)| offset >= start + width)
        {
            return Lookup::Source;
        }
        for (index, &(start, width)) in fields.iter().enumerate() {
            if offset < start || end > start + width {
                continue;
            }
            return match index.cmp(&self.completed) {
                Ordering::Less => Lookup::Served(&self.bytes[index][offset - start..end - start]),
                Ordering::Equal if offset == start && len == width => Lookup::Failed,
                _ => Lookup::Source,
            };
        }
        Lookup::Source
    }
}

/// The bytes a ship's fields point at, gathered in one call after them: its
/// pool entries and its heap fallback sprite with the terminator, each an
/// absolute `(address, len)` span; an empty span is not gathered. Served
/// like an object's fields.
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

    /// The spans `ship`'s copied fields select: the model reader's pool
    /// entries (after its vtable and header checks) and the heap sprite span
    /// its header names.
    fn gather(memory: &impl Memory, base: usize, ship: &Object<16>) -> Self {
        let mut heap = Self::NONE;
        let vtable = base
            .checked_add(cequipment_variant_pool::VTABLE_RVA)
            .unwrap_or(0);
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

    /// The copied pool entries, if any.
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

    fn differs(&self, other: &Self) -> bool {
        (0..self.completed.min(other.completed).min(2)).any(|index| {
            self.spans[index].1 != 0
                && other.spans[index].1 != 0
                && (self.spans[index] != other.spans[index]
                    || self.bytes(index) != other.bytes(index))
        })
    }
}

/// The task-force fields a carry of its terms compares hour to hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskForceFields {
    pub pointer: usize,
    pub handle: u64,
    pub roster_owner: u32,
    pub location: usize,
    pub fleet: usize,
    /// `+0x650`, raw.
    pub mine_exposure: u64,
    /// `+0x668`'s address.
    pub modifiers: usize,
}

/// A ship's own gathered objects, its fixed fields and [`Heap`], kept from
/// one hour's read to serve a later hour's ([`ShipFields::visit_held`]).
#[derive(Clone, Copy)]
pub struct HeldShip {
    ship: Object<16>,
    heap: Heap,
    /// The variants read from its pool entries, by entry; not the ship's
    /// own, so [`Self::same`] leaves them out.
    variants: [Object<8>; POOL_ENTRIES],
}

/// One variant's exact gathered observation from a held ship. Its pointer
/// selects the same model-selection and hull-variant fields; equality includes the copied
/// prefix and failure position, as a ship's variant comparison does.
#[derive(Clone, Copy)]
pub struct HeldVariant(Object<8>);

impl HeldVariant {
    pub fn pointer(&self) -> usize {
        self.0.pointer
    }

    pub fn same(&self, other: &Self) -> bool {
        self.0.same(&other.0, &VARIANT)
    }
}

impl HeldShip {
    /// Every pool-selected variant is represented by the held observations.
    /// Larger pools and incomplete entry or variant gathers use the model reader.
    pub fn variants_complete(&self) -> bool {
        pool_complete(&self.ship, &self.heap) && self.variants.iter().all(variant_complete)
    }

    /// A moved certificate can be revalidated only when every owned byte
    /// the reader could have served was gathered, including the heap sprite
    /// and its terminator. Larger payloads use the original reader.
    pub fn revalidation_complete(&self) -> bool {
        self.ship.completed == SHIP.len()
            && pool_complete(&self.ship, &self.heap)
            && msvc_string::heap_span(&self.ship.bytes[SHIP_SPRITE])
                .is_none_or(|span| self.heap.completed > 1 && self.heap.spans[1] == span)
    }

    /// The pool-selected observations, without copying the ship or heap.
    pub fn variants(&self) -> [HeldVariant; POOL_ENTRIES] {
        self.variants.map(HeldVariant)
    }

    /// The task force the ship named.
    pub fn task_force(&self) -> usize {
        self.ship.word(SHIP_TASK_FORCE)
    }

    /// The carrier air base the ship named (`+0x730`), else zero.
    pub fn carrier_air_base(&self) -> usize {
        if self.ship.completed > SHIP_TASK_FORCE {
            u64::from_le_bytes(self.ship.bytes[SHIP_TASK_FORCE][8..16].try_into().unwrap()) as usize
        } else {
            0
        }
    }

    /// The medal store the ship named (`+0x8F8`), else zero.
    pub fn medal_store(&self) -> usize {
        self.ship.word(SHIP_MEDALS)
    }

    /// The two modifier accumulators the organisation reader reads at fixed
    /// addresses of the ship: its experience modifiers and its medal store's
    /// (zero without a store).
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

    /// Whether both reads gathered every owned input and its bytes match,
    /// so a moved certificate can be revalidated.
    pub fn same(&self, other: &Self) -> bool {
        self.revalidation_complete()
            && other.revalidation_complete()
            && self.ship.same(&other.ship, &SHIP)
            && self.heap.same(&other.heap)
    }

    /// Whether any owned field or payload observed on both sides differs.
    /// Unavailable observations do not hide independent byte differences.
    pub fn differs(&self, other: &Self) -> bool {
        self.ship.differs(&other.ship, &SHIP) || self.heap.differs(&other.heap)
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

/// One capture's gathered objects: the ship being read, the task force and
/// location of the latest ship that named them, and the game singleton.
/// Create one per capture; it holds nothing a later capture may use.
pub struct ShipFields {
    base: usize,
    ship: Object<16>,
    task: Object<6>,
    location: Object<2>,
    globals: Object<1>,
    heap: Heap,
    variants: [Object<8>; VARIANT_SLOTS],
    /// The next slot a new variant replaces.
    next_variant: usize,
    /// The slots of the visited ship's gathered variants, by pool entry.
    current: [Option<usize>; POOL_ENTRIES],
}

impl ShipFields {
    /// Nothing gathered yet; `base` is the module base.
    pub fn new(base: usize) -> Self {
        Self {
            base,
            ship: Object::NONE,
            task: Object::NONE,
            location: Object::NONE,
            globals: Object::NONE,
            heap: Heap::NONE,
            variants: [Object::NONE; VARIANT_SLOTS],
            next_variant: 0,
            current: [None; POOL_ENTRIES],
        }
    }

    /// Before reading the ship at `ship`: gather its fields, its task
    /// force's and that task force's location's when they differ from the
    /// previous ship's, and the game singleton on the capture's first ship.
    pub fn visit(&mut self, memory: &impl Memory, ship: usize) {
        self.ship = Object::gather(memory, ship, SHIP);
        self.visit_shared(memory);
        self.heap = Heap::gather(memory, self.base, &self.ship);
        self.visit_variants(memory);
    }

    /// [`Self::visit`] serving the ship's own objects from `held` without
    /// reading them; its task force, location, the game singleton and its
    /// variants are gathered as for a read. Incomplete owned observations
    /// are refreshed, so a prior gather failure never supplies a later hour.
    pub fn visit_held(&mut self, memory: &impl Memory, held: &HeldShip) {
        if !held.revalidation_complete() {
            self.visit(memory, held.ship.pointer);
            return;
        }
        self.ship = held.ship;
        // The held variants are not served: they are gathered for this hour.
        self.visit_shared(memory);
        self.heap = held.heap;
        self.visit_variants(memory);
    }

    /// The visited ship's task force: its pointer, handle, roster owner, mine
    /// exposure and modifier accumulator address, when all were gathered.
    pub fn task_force(&self) -> Option<TaskForceFields> {
        let word =
            |index: usize| u64::from_le_bytes(self.task.bytes[index][..8].try_into().unwrap());
        (self.task.pointer != 0 && self.task.completed == TASK.len()).then(|| TaskForceFields {
            pointer: self.task.pointer,
            handle: word(1),
            roster_owner: u32::from_le_bytes(self.task.bytes[2][..4].try_into().unwrap()),
            location: word(3) as usize,
            fleet: word(4) as usize,
            mine_exposure: word(5),
            modifiers: self.task.pointer.saturating_add(TASK_MODIFIERS),
        })
    }

    /// The visited ship's own objects, to serve a later hour's read, with
    /// the variants this hour gathered for it.
    pub fn held(&self) -> HeldShip {
        HeldShip {
            ship: self.ship,
            heap: self.heap,
            variants: self.current_variants(),
        }
    }

    /// Whether `held`'s variants, gathered for this hour as
    /// [`Self::visit_held`] gathers them, have the bytes `held` holds,
    /// without serving its objects or its task force: a ship whose finished
    /// row is carried needs nothing else. A later read visits its ship.
    pub fn held_variants_unchanged(&mut self, memory: &impl Memory, held: &HeldShip) -> bool {
        let heap = held.heap;
        self.visit_variants_of(memory, &heap);
        held.variants_complete() && self.same_variant_observations(held)
    }

    /// Compare one held observation with this capture's gathered bytes.
    /// The certified pool's selection is already represented by the held
    /// observations; no heap copy or selection walk is needed to repeat it.
    pub fn held_variant_unchanged(&mut self, memory: &impl Memory, held: &HeldVariant) -> bool {
        if !variant_complete(&held.0) {
            return false;
        }
        if held.pointer() == 0 {
            return true;
        }
        let slot = self.variant(memory, held.pointer());
        variant_complete(&self.variants[slot]) && self.variants[slot].same(&held.0, &VARIANT)
    }

    /// Whether the visited ship's variants were gathered with the same bytes
    /// as `held`'s.
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
    }

    fn current_variants(&self) -> [Object<8>; POOL_ENTRIES] {
        self.current
            .map(|slot| slot.map_or(Object::NONE, |slot| self.variants[slot]))
    }

    /// The task force `held`'s ship named, with its location and the game
    /// singleton gathered as [`Self::visit_held`] gathers them, without
    /// serving the ship: for reading its terms ahead of its ships.
    pub fn task_force_of(
        &mut self,
        memory: &impl Memory,
        held: &HeldShip,
    ) -> Option<TaskForceFields> {
        self.visit_task(memory, held.task_force());
        self.task_force()
    }

    /// Gather the task force named by a certified ship's compact record.
    pub fn task_force_at(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
    ) -> Option<TaskForceFields> {
        self.visit_task(memory, pointer);
        self.task_force()
    }

    fn visit_shared(&mut self, memory: &impl Memory) {
        self.visit_task(memory, self.ship.word(SHIP_TASK_FORCE));
    }

    fn visit_task(&mut self, memory: &impl Memory, task: usize) {
        if self.globals.pointer == 0 {
            self.globals = Object::gather(memory, self.base, GLOBALS);
        }
        if task != self.task.pointer {
            self.task = Object::gather(memory, task, TASK);
        }
        let location = self.task.word(TASK_LOCATION);
        if location != self.location.pointer {
            self.location = Object::gather(memory, location, LOCATION);
        }
    }

    /// The model reader's pool walk (`model_selection_entries`) reads each
    /// copied positive entry's variant identity and flags and the selected
    /// (first largest) entry's identity, token and definition.
    fn visit_variants(&mut self, memory: &impl Memory) {
        let heap = self.heap;
        self.visit_variants_of(memory, &heap);
    }

    /// [`Self::visit_variants`] for the pool entries `heap` holds.
    fn visit_variants_of(&mut self, memory: &impl Memory, heap: &Heap) {
        self.current = [None; POOL_ENTRIES];
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
    }

    /// The slot holding `variant`, gathering it into the next slot no
    /// variant of the visited ship holds when it is not kept.
    fn variant(&mut self, memory: &impl Memory, variant: usize) -> usize {
        if let Some(slot) = self
            .variants
            .iter()
            .position(|kept| kept.pointer == variant)
        {
            return slot;
        }
        while self.current.contains(&Some(self.next_variant)) {
            self.next_variant = (self.next_variant + 1) % VARIANT_SLOTS;
        }
        let slot = self.next_variant;
        self.next_variant = (slot + 1) % VARIANT_SLOTS;
        self.variants[slot] = Object::gather(memory, variant, VARIANT);
        slot
    }

    /// The readers' memory for the visited ship over `source`.
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
        }
        Lookup::Source
    }
}

/// The readers' memory: gathered fields first ([`ShipFields`] for the
/// ShipModels readers, or a [`GatheredObject`]), then the source.
pub struct GatheredMemory<'a, M, F = ShipFields> {
    source: &'a M,
    fields: &'a F,
}

impl<'a, M: Memory, F: Served> GatheredMemory<'a, M, F> {
    /// `fields` served ahead of `source`.
    pub(crate) fn new(source: &'a M, fields: &'a F) -> Self {
        Self { source, fields }
    }

    /// The ordered scattered copy: served spans from their fields, the live
    /// spans before the first failed field in one call to the source, and
    /// nothing at or after the first failure.
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
            // Nothing gathered: the source's own single call.
            return if prefetched {
                self.source
                    .copy_scattered_prefetched(addresses, destinations)
            } else {
                self.source.copy_scattered(addresses, destinations)
            };
        }
        let copied = if (0..failed).any(|index| !destinations[index].is_empty() && live(index)) {
            // Served and later spans are empty for the source: empty spans
            // complete without a read, so the count stays in span order.
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

    /// The source's gather unless a field lies in a gathered object; then
    /// the ordered field-by-field copy, served or failed field by field.
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let address = |row: usize, field: &crate::memory::RecordField| {
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
    ) -> Option<crate::ccountry::AuthenticatedRoster<'_>> {
        self.source.authenticated_countries(base, game_state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<crate::ccountry::Country, &'static str>> {
        self.source.authenticated_reserved_country(base, game_state)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<crate::ccountry::ReservedCountryObservation> {
        self.source
            .authenticated_reserved_country_observation(base, game_state)
    }

    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<crate::modifier_accumulator::Value, &'static str>> {
        self.source.modifier_accumulator_value(address, key)
    }

    fn modifier_definitions(&self) -> Option<&crate::modifier_definition::PassDefinitions> {
        self.source.modifier_definitions()
    }
}

/// A source serving one selected object's gathered fields at a time
/// ([`GatheredObject`]): readers built once over it (a walk's collectors)
/// read each object the walk [`Self::select`]s from that object's single
/// gather, and every other read from the source.
pub struct SwitchedObject<'a, M, const N: usize> {
    source: &'a M,
    fields: [(usize, usize); N],
    current: std::cell::RefCell<GatheredObject<N>>,
}

impl<'a, M: Memory, const N: usize> SwitchedObject<'a, M, N> {
    /// Nothing selected yet; `fields` as for [`GatheredObject::gather`].
    pub fn new(source: &'a M, fields: [(usize, usize); N]) -> Self {
        Self {
            source,
            fields,
            current: std::cell::RefCell::new(GatheredObject::gather(source, 0, fields)),
        }
    }

    /// Gather the object at `pointer` (one guarded call) and serve it until
    /// the next selection.
    pub fn select(&self, pointer: usize) {
        *self.current.borrow_mut() = GatheredObject::gather(self.source, pointer, self.fields);
    }

    /// Whether the selected object's gather copied every field: when not,
    /// its readers read the rest from the source.
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
    ) -> Option<crate::ccountry::AuthenticatedRoster<'_>> {
        self.source.authenticated_countries(base, game_state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<crate::ccountry::Country, &'static str>> {
        self.source.authenticated_reserved_country(base, game_state)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<crate::ccountry::ReservedCountryObservation> {
        self.source
            .authenticated_reserved_country_observation(base, game_state)
    }

    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<crate::modifier_accumulator::Value, &'static str>> {
        self.source.modifier_accumulator_value(address, key)
    }

    fn modifier_definitions(&self) -> Option<&crate::modifier_definition::PassDefinitions> {
        self.source.modifier_definitions()
    }
}

/// Tests: a source whose reads fail when they touch `fault`, recording every
/// read and every failed read. Scattered and record reads copy span by span.
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

    /// Whether one failed read was made again.
    pub(super) fn retried(&self) -> bool {
        let failed = self.failed.borrow();
        failed
            .iter()
            .enumerate()
            .any(|(index, read)| failed[..index].contains(read))
    }

    /// The first and last byte of every read: the faults worth sweeping.
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
