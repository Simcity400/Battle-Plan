//! The controller and owner areas behind each province, which the selected army
//! route search reads for land-area paths and costs, held between hours and
//! read again only where a native writer may have changed them.
//!
//! - The province's controller-area pointer names the province's controller area (vtable
//!   [`ccontroller_area::VTABLE_RVA`]). Its pointer setter is reached at run
//!   time only by controller changes and their split and join operations; only
//!   province controller notifications reach those changes. Load construction
//!   and flood fill, reached from the game-state header loader and in-game
//!   initialization, and world clearing run only around a load.
//!
//! - An area's members are its province vector (`ccontroller_area::PROVINCE_DATA`). Insertion
//!   uses binary order by province id (the province's id), and removal operates
//!   in place, so the first member has the smallest id: that id labels the area
//!   here. Both operations keep the representative (`ccontroller_area::REPRESENTATIVE_PROVINCE`) used for path costs
//!   and ordering: the first land province (the definition's land bit) added,
//!   chosen again when it leaves.
//!
//! - Contacts (`CONTACTS`): first and last node, signed count and
//!   deferred-removal byte. Each 0x20-byte node holds the payload,
//!   previous, next and removal mark; each payload is
//!   `{area, signed count}`. Land-area search follows the list in
//!   order. Contact insertion appends a new target and counts an existing one;
//!   it and removal run only from controller changes, splitting and joining,
//!   between the area of a province whose area changes and a neighbour's area
//!   (the definition's adjacency list, 0x30-byte entries keyed by id).
//!
//! - The province's owner-area pointer names its owner area; only the global rebuild writes it
//!   at run time, and only equality and null are read.
//!
//! Every province's two area pointers arrive with the province walk's movement
//! gather. A carried row ([`cprovince::carry`]) brings its held pointers, which
//! the collector reads again for every capture that compares them
//! ([`Store::carries_quietly`]). An hour on which no province was marked
//! ([`Store::marks`]: the territory family's province writers, including
//! controller notification) publishes the held areas and checks its sampled
//! slice. A marked hour compares the pointers of every province a controller
//! change can re-point ([`Store::repointable`]; every province when that is not
//! bounded) with the held ones and reads again the area of every marked or
//! re-pointed province, the area it left and its neighbours' areas. This covers
//! every area whose members, representative or contacts a controller change
//! touches. Areas are labelled by their smallest member, owner areas likewise;
//! no pointer leaves the collector.

use crate::{
    Memory, allocation, ccontroller_area, cpdx_array::Array, cprovince, province_definition,
    province_neighbour, read, read_slice,
};

/// From the representative on: representative, member data, capacity and count.
const MEMBERS: usize = ccontroller_area::REPRESENTATIVE_PROVINCE;
/// Contacts: first and last contact, signed count, deferred byte.
const CONTACTS: usize = 0x90;
const CONTACT_HEADER: usize = 21;
const CONTACT_NODE: usize = 25;

/// What the province walk gathers for the areas, in row order: the province,
/// its definition and both area pointers (null is zero).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pointers {
    pub province: usize,
    pub definition: usize,
    pub controller: usize,
    pub owner: usize,
}

/// One controller area as recorded: it lives on the province whose id is its
/// label (its smallest member).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub label: u32,
    /// The representative as an authenticated member; None refused.
    pub first_land: Option<u32>,
    /// The contacts' labels in native order, a range of
    /// [`Output::neighbours`]; None refused.
    pub neighbours: Option<(usize, usize)>,
}

/// The hour's nodes, sorted by label, and their contacts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub nodes: Vec<Node>,
    pub neighbours: Vec<u32>,
}

impl Output {
    pub fn neighbours(&self, node: &Node) -> Option<&[u32]> {
        let (start, len) = node.neighbours?;
        self.neighbours.get(start..start.checked_add(len)?)
    }

    pub fn allocated_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<Node>() + self.neighbours.capacity() * size_of::<u32>()
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.neighbours.clear();
    }

    fn copy_from(&mut self, other: &Self) -> Result<(), &'static str> {
        self.clear();
        allocation::reserve(&mut self.nodes, other.nodes.len()).map_err(|_| ALLOCATION)?;
        allocation::reserve(&mut self.neighbours, other.neighbours.len())
            .map_err(|_| ALLOCATION)?;
        self.nodes.extend_from_slice(&other.nodes);
        self.neighbours.extend_from_slice(&other.neighbours);
        Ok(())
    }
}

/// One capture's work, for status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Every area was read: nothing held certified the hour.
    pub full: bool,
    /// No province was marked: the held areas were published.
    pub quiet: bool,
    /// Provinces whose controller area changed.
    pub changed: u64,
    /// Areas read.
    pub read: u64,
    /// Areas published from the held capture.
    pub carried: u64,
    /// Held provinces and areas compared with a fresh read.
    pub verified: u64,
    /// Of those, the ones that differed.
    pub mismatches: u64,
}

const ALLOCATION: &str = "controller area storage allocation failed";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Held {
    present: bool,
    province: usize,
    definition: usize,
    controller: usize,
    owner: usize,
    controller_label: u32,
    owner_label: u32,
}

impl Held {
    fn same(&self, pointers: &Pointers) -> bool {
        self.present
            && self.province == pointers.province
            && self.definition == pointers.definition
            && self.controller == pointers.controller
            && self.owner == pointers.owner
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Area {
    pointer: usize,
    label: u32,
    /// Provinces whose controller-area pointer names this area.
    members: u32,
    /// Why the node was refused: it is published without either operand.
    failure: Option<&'static str>,
    first_land: Option<u32>,
    /// Contact areas in native order, valid when `contacts_failure` is None.
    contacts: Vec<usize>,
    contacts_failure: Option<&'static str>,
}

impl Area {
    fn new(pointer: usize, label: u32, members: u32) -> Self {
        Self {
            pointer,
            label,
            members,
            failure: None,
            first_land: None,
            contacts: Vec::new(),
            contacts_failure: Some("controller area not read"),
        }
    }

    /// What a read publishes, for comparisons.
    fn same_read(&self, other: &Self) -> bool {
        self.label == other.label
            && self.failure == other.failure
            && self.first_land == other.first_land
            && self.contacts_failure.is_none() == other.contacts_failure.is_none()
            && (self.contacts_failure.is_some() || self.contacts == other.contacts)
    }
}

/// The held capture, lent to the province walk's job each hour.
#[derive(Debug, Default)]
pub struct Store {
    /// The dirty-set epochs the held capture was taken under; None when
    /// nothing is held, so the next capture reads every area.
    epochs: Option<[u64; 2]>,
    /// By province id; index zero is no province.
    provinces: Vec<Held>,
    present: usize,
    /// Sorted by pointer.
    areas: Vec<Area>,
    /// The held capture's published nodes, current while `published_current`.
    published: Output,
    published_current: bool,
    marks: Vec<u32>,
    changed: Vec<(u32, usize)>,
    affected: Vec<usize>,
    relabeled: Vec<usize>,
    pairs: Vec<(usize, u32)>,
    neighbour_ids: Vec<u32>,
    neighbour_bytes: Vec<u8>,
    /// The carried labels a verifying capture compares with its fresh read.
    carried_labels: Vec<(u32, u32, u32)>,
    carried_areas: Vec<Area>,
    poisoned: bool,
    /// [`Self::allocated_bytes`] as of the latest capture.
    bytes: usize,
    /// A requested full refresh: the next capture reads every area.
    pub force_full: bool,
    /// Storage for the hour's [`Self::repointable`] province ids, lent to the
    /// caller while it reads their pointers.
    pub scope: Vec<u32>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// The province ids the hour's take marked; [`Self::capture`] consumes
    /// them.
    pub fn marks(&mut self) -> &mut Vec<u32> {
        &mut self.marks
    }

    /// A failed job or an incomplete walk: nothing is held.
    pub fn forget(&mut self) {
        self.epochs = None;
        self.marks.clear();
        self.published_current = false;
    }

    /// Whether [`Self::capture`] of `rows` rows under `take` would publish
    /// the held areas comparing only its slice's pointers: nothing marked
    /// ([`Self::marks`]), nothing refused and no full read requested. Every
    /// other capture compares or reads every province's pointers.
    pub fn carries_quietly(&self, take: Option<[u64; 2]>, rows: usize, verify: bool) -> bool {
        !self.poisoned
            && !self.force_full
            && !verify
            && take.is_some()
            && take == self.epochs
            && rows == self.present
            && self.marks.is_empty()
    }

    /// The province ids whose area pointers a marked capture must compare, into `scope` (sorted):
    /// the marked provinces, their neighbours and every held member of the held controller area of
    /// a marked province or of one of its neighbours. A controller change re-points no other
    /// province: a controller change stores only the changed province's controller-area pointer
    /// (through the pointer setter), splitting clears and re-fills only the members of the area the
    /// province left, and joining moves the members of one area into another, both areas of the
    /// changed province or of its neighbours; by induction over the hour's changes every area a
    /// later change touches is a union of members of those held areas. The split's flood fill also
    /// takes a null-area province of the new controller, so a marked province whose controller
    /// `rows` give as zero or a held null area of a nonzero controller answers false: compare every
    /// province. False too when nothing is held or a province or its neighbours do not read.
    pub fn repointable(
        &mut self,
        memory: &impl Memory,
        rows: &[cprovince::Row],
        scope: &mut Vec<u32>,
    ) -> Result<bool, &'static str> {
        scope.clear();
        if self.epochs.is_none() || self.marks.is_empty() {
            return Ok(false);
        }
        let mut areas = std::mem::take(&mut self.affected);
        areas.clear();
        let result = (|| {
            for index in 0..self.marks.len() {
                let id = self.marks[index];
                let Some(held) = self.provinces.get(id as usize).copied() else {
                    return Ok(false);
                };
                let Ok(row) = rows.binary_search_by_key(&id, |row| row.id) else {
                    return Ok(false);
                };
                if !held.present || rows[row].controller == 0 {
                    return Ok(false);
                }
                if !self.neighbours(memory, held.definition)? {
                    return Ok(false);
                }
                allocation::reserve(&mut areas, self.neighbour_ids.len() + 1)
                    .map_err(|_| ALLOCATION)?;
                allocation::reserve(scope, self.neighbour_ids.len() + 1).map_err(|_| ALLOCATION)?;
                areas.push(held.controller);
                scope.push(id);
                for &neighbour in &self.neighbour_ids {
                    scope.push(neighbour);
                    areas.push(
                        self.provinces
                            .get(neighbour as usize)
                            .map_or(0, |held| held.controller),
                    );
                }
            }
            areas.retain(|&pointer| pointer != 0);
            areas.sort_unstable();
            areas.dedup();
            for (id, held) in self.provinces.iter().enumerate() {
                if !held.present {
                    continue;
                }
                let member = held.controller != 0 && areas.binary_search(&held.controller).is_ok();
                let null_controlled = held.controller == 0
                    && rows
                        .binary_search_by_key(&(id as u32), |row| row.id)
                        .is_ok_and(|row| rows[row].controller != 0);
                if null_controlled {
                    return Ok(false);
                }
                if member {
                    allocation::reserve(scope, 1).map_err(|_| ALLOCATION)?;
                    scope.push(id as u32);
                }
            }
            scope.sort_unstable();
            scope.dedup();
            Ok(true)
        })();
        self.affected = areas;
        if !matches!(result, Ok(true)) {
            scope.clear();
        }
        result
    }

    /// A fresh read differed from what the held capture published: every
    /// later capture reads every area.
    pub fn poisoned(&self) -> bool {
        self.poisoned
    }

    /// The storage the store held after its latest capture; constant time,
    /// for the callback that takes it back.
    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    fn measure(&self) -> usize {
        self.provinces.capacity() * size_of::<Held>()
            + self.areas.capacity() * size_of::<Area>()
            + self
                .areas
                .iter()
                .map(|area| area.contacts.capacity() * size_of::<usize>())
                .sum::<usize>()
            + self.published.allocated_bytes()
            + self.marks.capacity() * size_of::<u32>()
            + self.changed.capacity() * size_of::<(u32, usize)>()
            + (self.affected.capacity() + self.relabeled.capacity()) * size_of::<usize>()
            + self.pairs.capacity() * size_of::<(usize, u32)>()
            + self.neighbour_ids.capacity() * size_of::<u32>()
            + self.neighbour_bytes.capacity()
            + self.carried_labels.capacity() * size_of::<(u32, u32, u32)>()
            + self.carried_areas.capacity() * size_of::<Area>()
    }

    /// Labels `rows` and writes the hour's nodes. `pointers` holds each
    /// row's areas in row order from the same complete walk; `take` the
    /// territory family's province and province-state epochs, None when it
    /// refused; `covers` the sampled slice by province id and area label;
    /// `verify` reads every area afresh and compares it with what the carry
    /// would publish. On an error no row is labelled and nothing is held.
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &mut [cprovince::Row],
        pointers: &[Pointers],
        output: &mut Output,
        take: Option<[u64; 2]>,
        covers: &dyn Fn(u64) -> bool,
        verify: bool,
    ) -> Result<Counts, &'static str> {
        let result = self.capture_inner(memory, base, rows, pointers, take, covers, verify);
        self.marks.clear();
        let result = result.and_then(|counts| {
            self.publish(rows, output)?;
            Ok(counts)
        });
        self.bytes = self.measure();
        if result.is_err() {
            self.epochs = None;
            for row in rows.iter_mut() {
                row.controller_area = None;
                row.owner_area = None;
                row.area_node = None;
            }
            output.clear();
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &[cprovince::Row],
        pointers: &[Pointers],
        take: Option<[u64; 2]>,
        covers: &dyn Fn(u64) -> bool,
        verify: bool,
    ) -> Result<Counts, &'static str> {
        if pointers.len() < rows.len() {
            return Err("province area pointers shorter than the walk");
        }
        let pointers = &pointers[..rows.len()];
        let mut counts = Counts::default();
        let eligible = !self.poisoned && !self.force_full && take.is_some() && take == self.epochs;
        self.force_full = false;
        let carried =
            eligible && self.carry(memory, base, rows, pointers, covers, verify, &mut counts)?;
        if !carried {
            self.full(memory, base, rows, pointers, &mut counts)?;
        } else if verify {
            // Keep what the carry would publish, read everything afresh and
            // compare; the fresh read is published.
            self.keep_carried()?;
            self.full(memory, base, rows, pointers, &mut counts)?;
            self.compare_carried(&mut counts);
        }
        if counts.mismatches != 0 {
            self.poisoned = true;
        }
        self.epochs = take;
        Ok(counts)
    }

    /// Reads every area afresh.
    fn full(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &[cprovince::Row],
        pointers: &[Pointers],
        counts: &mut Counts,
    ) -> Result<(), &'static str> {
        counts.full = true;
        counts.carried = 0;
        self.published_current = false;
        self.index(rows, pointers)?;
        self.group_controllers()?;
        self.group_owners()?;
        for at in 0..self.areas.len() {
            let label = self.areas[at].label;
            let first = self.read_area(memory, base, at)?;
            if self.areas[at].failure.is_none() && first != Some(label) {
                self.areas[at].failure =
                    Some("controller area first member is not its smallest linked province");
            }
            counts.read += 1;
        }
        Ok(())
    }

    fn index(
        &mut self,
        rows: &[cprovince::Row],
        pointers: &[Pointers],
    ) -> Result<(), &'static str> {
        let last = rows.iter().map(|row| row.id as usize).max().unwrap_or(0);
        self.provinces.clear();
        allocation::resize_default(&mut self.provinces, last + 1).map_err(|_| ALLOCATION)?;
        for (row, pointers) in rows.iter().zip(pointers) {
            let held = &mut self.provinces[row.id as usize];
            if held.present || row.id == 0 {
                return Err("province walk repeats or nulls a province id");
            }
            *held = Held {
                present: true,
                province: pointers.province,
                definition: pointers.definition,
                controller: pointers.controller,
                owner: pointers.owner,
                ..Held::default()
            };
        }
        self.present = rows.len();
        Ok(())
    }

    /// Groups the provinces by controller-area pointer: each group is one area, labelled by its
    /// smallest province id.
    fn group_controllers(&mut self) -> Result<(), &'static str> {
        self.fill_pairs(false)?;
        let Self {
            pairs,
            areas,
            provinces,
            ..
        } = self;
        areas.clear();
        let mut start = 0;
        while start < pairs.len() {
            let (pointer, label) = pairs[start];
            let end = start + pairs[start..].partition_point(|pair| pair.0 == pointer);
            allocation::reserve(areas, 1).map_err(|_| ALLOCATION)?;
            areas.push(Area::new(pointer, label, (end - start) as u32));
            for &(_, id) in &pairs[start..end] {
                provinces[id as usize].controller_label = label;
            }
            start = end;
        }
        Ok(())
    }

    /// Labels the provinces by owner-area group, each by its smallest province id.
    fn group_owners(&mut self) -> Result<(), &'static str> {
        self.fill_pairs(true)?;
        let Self {
            pairs, provinces, ..
        } = self;
        for held in provinces.iter_mut() {
            held.owner_label = 0;
        }
        let mut start = 0;
        while start < pairs.len() {
            let (pointer, label) = pairs[start];
            let end = start + pairs[start..].partition_point(|pair| pair.0 == pointer);
            for &(_, id) in &pairs[start..end] {
                provinces[id as usize].owner_label = label;
            }
            start = end;
        }
        Ok(())
    }

    fn fill_pairs(&mut self, owner: bool) -> Result<(), &'static str> {
        self.pairs.clear();
        allocation::reserve(&mut self.pairs, self.present).map_err(|_| ALLOCATION)?;
        for (id, held) in self.provinces.iter().enumerate() {
            let pointer = if owner { held.owner } else { held.controller };
            if held.present && pointer != 0 {
                self.pairs.push((pointer, id as u32));
            }
        }
        self.pairs.sort_unstable();
        Ok(())
    }

    fn find(&self, pointer: usize) -> Result<usize, usize> {
        self.areas
            .binary_search_by_key(&pointer, |area| area.pointer)
    }

    fn label_of(&self, pointer: usize) -> u32 {
        if pointer == 0 {
            return 0;
        }
        self.find(pointer).map_or(0, |at| self.areas[at].label)
    }

    /// Publishes the held capture when nothing marked a province, otherwise
    /// follows the marks. False when the hour must read every area afresh.
    #[allow(clippy::too_many_arguments)]
    fn carry(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &[cprovince::Row],
        pointers: &[Pointers],
        covers: &dyn Fn(u64) -> bool,
        verify: bool,
        counts: &mut Counts,
    ) -> Result<bool, &'static str> {
        if rows.len() != self.present {
            return Ok(false);
        }
        if self.marks.is_empty() {
            counts.quiet = true;
            if !verify {
                for (row, current) in rows.iter().zip(pointers) {
                    if !covers(u64::from(row.id)) {
                        continue;
                    }
                    counts.verified += 1;
                    if !self
                        .provinces
                        .get(row.id as usize)
                        .is_some_and(|held| held.same(current))
                    {
                        counts.mismatches += 1;
                    }
                }
                if counts.mismatches == 0 {
                    self.affected.clear();
                    self.verify_areas(memory, base, covers, counts)?;
                }
                if counts.mismatches != 0 {
                    return Ok(false);
                }
            }
            counts.carried = self.areas.len() as u64;
            return Ok(true);
        }
        self.changed.clear();
        self.published_current = false;
        let mut owners = false;
        for (row, current) in rows.iter().zip(pointers) {
            let Some(held) = self.provinces.get_mut(row.id as usize) else {
                return Ok(false);
            };
            if !held.present
                || held.province != current.province
                || held.definition != current.definition
            {
                return Ok(false);
            }
            if held.controller != current.controller {
                allocation::reserve(&mut self.changed, 1).map_err(|_| ALLOCATION)?;
                self.changed.push((row.id, held.controller));
                held.controller = current.controller;
            }
            if held.owner != current.owner {
                owners = true;
                held.owner = current.owner;
            }
        }
        counts.changed = self.changed.len() as u64;
        if owners {
            self.group_owners()?;
        }
        // Membership follows the re-pointed provinces.
        for index in 0..self.changed.len() {
            let (id, old) = self.changed[index];
            let new = self.provinces[id as usize].controller;
            if old != 0 {
                let Ok(at) = self.find(old) else {
                    return Ok(false);
                };
                let Some(members) = self.areas[at].members.checked_sub(1) else {
                    return Ok(false);
                };
                self.areas[at].members = members;
            }
            if new != 0 {
                match self.find(new) {
                    Ok(at) => self.areas[at].members += 1,
                    Err(at) => {
                        allocation::reserve(&mut self.areas, 1).map_err(|_| ALLOCATION)?;
                        self.areas.insert(at, Area::new(new, 0, 1));
                    }
                }
            }
        }
        // Every area a controller change can touch.
        self.affected.clear();
        for index in 0..self.changed.len() + self.marks.len() {
            let (id, old) = match self.changed.get(index) {
                Some(&changed) => changed,
                None => (self.marks[index - self.changed.len()], 0),
            };
            let Some(held) = self.provinces.get(id as usize).copied() else {
                return Ok(false);
            };
            if !held.present {
                return Ok(false);
            }
            allocation::reserve(&mut self.affected, 2).map_err(|_| ALLOCATION)?;
            self.affected.extend([old, held.controller]);
            if !self.neighbours(memory, held.definition)? {
                return Ok(false);
            }
            for at in 0..self.neighbour_ids.len() {
                let neighbour = self.neighbour_ids[at];
                let controller = self
                    .provinces
                    .get(neighbour as usize)
                    .map_or(0, |held| held.controller);
                allocation::reserve(&mut self.affected, 1).map_err(|_| ALLOCATION)?;
                self.affected.push(controller);
            }
        }
        self.affected.retain(|&pointer| pointer != 0);
        self.affected.sort_unstable();
        self.affected.dedup();
        self.areas.retain(|area| area.members != 0);
        self.relabeled.clear();
        for index in 0..self.affected.len() {
            let pointer = self.affected[index];
            let Ok(at) = self.find(pointer) else {
                continue;
            };
            let before = self.areas[at].label;
            // An area's label is its first member: member insertion keeps id order.
            let Some(first) = self.read_area(memory, base, at)? else {
                return Ok(false);
            };
            counts.read += 1;
            self.areas[at].label = first;
            if first != before {
                allocation::reserve(&mut self.relabeled, 1).map_err(|_| ALLOCATION)?;
                self.relabeled.push(pointer);
            }
        }
        counts.carried = (self.areas.len() as u64).saturating_sub(counts.read);
        for index in 0..self.changed.len() {
            let id = self.changed[index].0 as usize;
            let label = self.label_of(self.provinces[id].controller);
            self.provinces[id].controller_label = label;
        }
        if !self.relabeled.is_empty() {
            let Self {
                provinces,
                relabeled,
                areas,
                ..
            } = self;
            for held in provinces.iter_mut() {
                if held.present
                    && held.controller != 0
                    && relabeled.binary_search(&held.controller).is_ok()
                {
                    held.controller_label = areas
                        .binary_search_by_key(&held.controller, |area| area.pointer)
                        .map_or(0, |at| areas[at].label);
                }
            }
        }
        if !verify {
            self.verify_areas(memory, base, covers, counts)?;
            if counts.mismatches != 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Reads afresh the held areas of the hour's slice that the hour did not
    /// read already, and counts those that differ.
    fn verify_areas(
        &mut self,
        memory: &impl Memory,
        base: usize,
        covers: &dyn Fn(u64) -> bool,
        counts: &mut Counts,
    ) -> Result<(), &'static str> {
        for at in 0..self.areas.len() {
            let area = &self.areas[at];
            if !covers(u64::from(area.label)) || self.affected.binary_search(&area.pointer).is_ok()
            {
                continue;
            }
            let held = area.clone();
            let first = self.read_area(memory, base, at)?;
            counts.verified += 1;
            let fresh = &mut self.areas[at];
            if first.is_some() {
                fresh.label = first.unwrap_or(0);
            }
            if first.is_none() || !fresh.same_read(&held) {
                counts.mismatches += 1;
            }
        }
        Ok(())
    }

    /// The definition's neighbour ids in native order, into `neighbour_ids`.
    /// False when the definition does not read.
    fn neighbours(
        &mut self,
        memory: &impl Memory,
        definition: usize,
    ) -> Result<bool, &'static str> {
        self.neighbour_ids.clear();
        let Some(array) = definition
            .checked_add(province_definition::NEIGHBOURS)
            .and_then(|address| Array::read(memory, address, province_neighbour::STRIDE))
        else {
            return Ok(false);
        };
        let Some(bytes) = (array.count as usize).checked_mul(province_neighbour::STRIDE) else {
            return Ok(false);
        };
        self.neighbour_bytes.clear();
        allocation::resize_default(&mut self.neighbour_bytes, bytes).map_err(|_| ALLOCATION)?;
        if !read_slice(memory, array.data, &mut self.neighbour_bytes) {
            return Ok(false);
        }
        allocation::reserve(&mut self.neighbour_ids, array.count as usize)
            .map_err(|_| ALLOCATION)?;
        for entry in self
            .neighbour_bytes
            .chunks_exact(province_neighbour::STRIDE)
        {
            let at = province_neighbour::PROVINCE_ID;
            self.neighbour_ids
                .push(u32::from_le_bytes(entry[at..at + 4].try_into().unwrap()));
        }
        Ok(true)
    }

    /// Reads area `at`'s node; a fault the node alone carries stays on it.
    /// Returns the id of its first member when that member authenticates as
    /// a province linking to it.
    fn read_area(
        &mut self,
        memory: &impl Memory,
        base: usize,
        at: usize,
    ) -> Result<Option<u32>, &'static str> {
        let Self {
            provinces, areas, ..
        } = self;
        let area = &mut areas[at];
        let pointer = area.pointer;
        area.failure = None;
        area.first_land = None;
        area.contacts.clear();
        area.contacts_failure = None;
        let (Some(members), Some(contacts)) =
            (pointer.checked_add(MEMBERS), pointer.checked_add(CONTACTS))
        else {
            area.failure = Some("controller area address overflow");
            return Ok(None);
        };
        let mut vtable = [0_u8; 8];
        let mut fields = [0_u8; 24];
        let mut header = [0_u8; CONTACT_HEADER];
        let copied = memory.copy_scattered(
            [pointer, members, contacts],
            &mut [&mut vtable, &mut fields, &mut header],
        );
        let expected = base.wrapping_add(ccontroller_area::VTABLE_RVA) as u64;
        if copied < 1 || u64::from_le_bytes(vtable) != expected {
            area.failure = Some("controller area type unreadable or unsupported");
            return Ok(None);
        }
        if copied < 3 {
            area.failure = Some("controller area header unreadable");
            return Ok(None);
        }
        let word = |bytes: &[u8], at: usize| {
            u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
        };
        let representative = word(&fields, 0);
        let data = word(&fields, 8);
        let count = i32::from_le_bytes(fields[20..24].try_into().unwrap());
        if count < 1 || count as u32 != area.members {
            area.failure = Some("controller area member count differs from its provinces' links");
            return Ok(None);
        }
        // A member authenticates through its id slot of the same walk.
        let member = |province: usize| -> Option<u32> {
            if province == 0 {
                return None;
            }
            let id = read::<u32>(memory, province.checked_add(cprovince::ID)?)?;
            let held = provinces.get(id as usize)?;
            (held.present && held.province == province && held.controller == pointer).then_some(id)
        };
        let first = read::<u64>(memory, data).and_then(|first| member(first as usize));
        area.first_land = member(representative);
        let first_node = word(&header, 0);
        let last = word(&header, 8);
        let contact_count = i32::from_le_bytes(header[16..20].try_into().unwrap());
        let result = (|| {
            if contact_count < 0 {
                return Err("controller area contact count negative");
            }
            if header[20] != 0 {
                return Err("controller area deferred contact removal unsupported");
            }
            allocation::reserve(&mut area.contacts, contact_count as usize)
                .map_err(|_| "controller area contact storage allocation failed")?;
            let mut cursor = first_node;
            let mut previous = 0;
            for _ in 0..contact_count {
                if cursor == 0 {
                    return Err("controller area contacts end before their count");
                }
                let node = read::<[u8; CONTACT_NODE]>(memory, cursor)
                    .ok_or("controller area contact unreadable")?;
                let payload = word(&node, 0);
                let back = word(&node, 8);
                let next = word(&node, 16);
                // Contact removal marks instead of unlinking under the deferred
                // byte; a marked node's payload may already be freed.
                if node[24] != 0 {
                    return Err("controller area contact marked for removal");
                }
                if back != previous || payload == 0 {
                    return Err("controller area contact links invalid");
                }
                let target = read::<u64>(memory, payload)
                    .filter(|&target| target != 0)
                    .ok_or("controller area contact payload unreadable or null")?;
                area.contacts.push(target as usize);
                previous = cursor;
                cursor = next;
            }
            if cursor != 0 || previous != last || (contact_count == 0 && first_node != 0) {
                return Err("controller area contact links differ from their count");
            }
            Ok(())
        })();
        if let Err(reason) = result {
            area.contacts.clear();
            area.contacts_failure = Some(reason);
        }
        Ok(first)
    }

    fn keep_carried(&mut self) -> Result<(), &'static str> {
        self.carried_labels.clear();
        allocation::reserve(&mut self.carried_labels, self.present).map_err(|_| ALLOCATION)?;
        for (id, held) in self.provinces.iter().enumerate() {
            if held.present {
                self.carried_labels
                    .push((id as u32, held.controller_label, held.owner_label));
            }
        }
        self.carried_areas.clear();
        allocation::reserve(&mut self.carried_areas, self.areas.len()).map_err(|_| ALLOCATION)?;
        for area in &self.areas {
            let mut copy = Area::new(area.pointer, area.label, area.members);
            copy.failure = area.failure;
            copy.first_land = area.first_land;
            copy.contacts_failure = area.contacts_failure;
            allocation::reserve(&mut copy.contacts, area.contacts.len()).map_err(|_| ALLOCATION)?;
            copy.contacts.extend_from_slice(&area.contacts);
            self.carried_areas.push(copy);
        }
        Ok(())
    }

    /// Counts every province label and area the carry would have published
    /// differently from the fresh read.
    fn compare_carried(&mut self, counts: &mut Counts) {
        for &(id, controller, owner) in &self.carried_labels {
            counts.verified += 1;
            let held = self.provinces.get(id as usize);
            if !held.is_some_and(|held| {
                held.present && held.controller_label == controller && held.owner_label == owner
            }) {
                counts.mismatches += 1;
            }
        }
        if self.carried_labels.len() != self.present {
            counts.mismatches += 1;
        }
        counts.verified += self.carried_areas.len() as u64;
        if self.carried_areas.len() != self.areas.len() {
            counts.mismatches += 1;
        }
        for (carried, fresh) in self.carried_areas.iter().zip(&self.areas) {
            if carried.pointer != fresh.pointer
                || carried.members != fresh.members
                || !carried.same_read(fresh)
            {
                counts.mismatches += 1;
            }
        }
    }

    /// Writes the held labels into `rows` and the nodes into `output`.
    fn publish(
        &mut self,
        rows: &mut [cprovince::Row],
        output: &mut Output,
    ) -> Result<(), &'static str> {
        if !self.published_current {
            self.build_published()?;
            self.published_current = true;
        }
        output.copy_from(&self.published)?;
        for row in rows.iter_mut() {
            let held = self
                .provinces
                .get(row.id as usize)
                .filter(|held| held.present)
                .ok_or("province walk row outside the held areas")?;
            row.controller_area = Some(held.controller_label);
            row.owner_area = Some(held.owner_label);
            row.area_node = if held.controller != 0 && held.controller_label == row.id {
                output
                    .nodes
                    .binary_search_by_key(&row.id, |node| node.label)
                    .ok()
                    .map(|index| index as u32)
            } else {
                None
            };
        }
        Ok(())
    }

    fn build_published(&mut self) -> Result<(), &'static str> {
        let Self {
            areas, published, ..
        } = self;
        published.clear();
        // A failed build leaves nothing current; the next publish rebuilds.
        allocation::reserve(&mut published.nodes, areas.len()).map_err(|_| ALLOCATION)?;
        for area in areas.iter() {
            let mut node = Node {
                label: area.label,
                first_land: None,
                neighbours: None,
            };
            if area.failure.is_none() {
                node.first_land = area.first_land;
                if area.contacts_failure.is_none() {
                    let start = published.neighbours.len();
                    allocation::reserve(&mut published.neighbours, area.contacts.len())
                        .map_err(|_| ALLOCATION)?;
                    let mut complete = true;
                    for &contact in &area.contacts {
                        // A contact outside the provinces' areas has no label.
                        match areas.binary_search_by_key(&contact, |area| area.pointer) {
                            Ok(at) => published.neighbours.push(areas[at].label),
                            Err(_) => {
                                complete = false;
                                break;
                            }
                        }
                    }
                    if complete {
                        node.neighbours = Some((start, published.neighbours.len() - start));
                    } else {
                        published.neighbours.truncate(start);
                    }
                }
            }
            published.nodes.push(node);
        }
        published.nodes.sort_unstable_by_key(|node| node.label);
        if published
            .nodes
            .windows(2)
            .any(|pair| pair[0].label == pair[1].label)
        {
            return Err("controller areas share a label");
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "controller_areas/tests.rs"]
mod tests;
