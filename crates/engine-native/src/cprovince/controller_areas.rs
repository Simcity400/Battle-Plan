
use crate::{
    Memory,
    allocation,
    ccontroller_area,
    cpdx_array::Array,
    cprovince::{self, definition as province_definition},
    province_neighbour,
    read,
    read_slice,
};

const MEMBERS: usize = ccontroller_area::REPRESENTATIVE_PROVINCE;
const CONTACTS: usize = 0x90;
const CONTACT_HEADER: usize = 21;
const CONTACT_NODE: usize = 25;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pointers {
    pub province: usize,
    pub definition: usize,
    pub controller: usize,
    pub owner: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub label: u32,
    pub first_land: Option<u32>,
    pub neighbours: Option<(usize, usize)>,
}

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub full: bool,
    pub quiet: bool,
    pub changed: u64,
    pub read: u64,
    pub carried: u64,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Area {
    pointer: usize,
    label: u32,
    members: u32,
    failure: Option<&'static str>,
    first_land: Option<u32>,
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
}

#[derive(Debug, Default)]
pub struct Store {
    epochs: Option<[u64; 2]>,
    provinces: Vec<Held>,
    present: usize,
    areas: Vec<Area>,
    published: Output,
    published_current: bool,
    marks: Vec<u32>,
    changed: Vec<(u32, usize)>,
    affected: Vec<usize>,
    relabeled: Vec<usize>,
    pairs: Vec<(usize, u32)>,
    neighbour_ids: Vec<u32>,
    neighbour_bytes: Vec<u8>,
    bytes: usize,
    pub force_full: bool,
    pub scope: Vec<u32>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn marks(&mut self) -> &mut Vec<u32> {
        &mut self.marks
    }

    pub fn forget(&mut self) {
        self.epochs = None;
        self.marks.clear();
        self.published_current = false;
    }

    pub fn carries_quietly(&self, take: Option<[u64; 2]>, rows: usize) -> bool {
        !self.force_full
            && take.is_some()
            && take == self.epochs
            && rows == self.present
            && self.marks.is_empty()
    }

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
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &mut [cprovince::Row],
        pointers: &[Pointers],
        output: &mut Output,
        take: Option<[u64; 2]>,
    ) -> Result<Counts, &'static str> {
        let result = self.capture_inner(memory, base, rows, pointers, take);
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

    fn capture_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &[cprovince::Row],
        pointers: &[Pointers],
        take: Option<[u64; 2]>,
    ) -> Result<Counts, &'static str> {
        if pointers.len() < rows.len() {
            return Err("province area pointers shorter than the walk");
        }
        let pointers = &pointers[..rows.len()];
        let mut counts = Counts::default();
        let eligible = !self.force_full && take.is_some() && take == self.epochs;
        self.force_full = false;
        let carried = eligible && self.carry(memory, base, rows, pointers, &mut counts)?;
        if !carried {
            self.full(memory, base, rows, pointers, &mut counts)?;
        }
        self.epochs = take;
        Ok(counts)
    }

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

    fn carry(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &[cprovince::Row],
        pointers: &[Pointers],
        counts: &mut Counts,
    ) -> Result<bool, &'static str> {
        if rows.len() != self.present {
            return Ok(false);
        }
        if self.marks.is_empty() {
            counts.quiet = true;
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
        Ok(true)
    }

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
        let expected = engine_sites::at(base, ccontroller_area::VTABLE_RVA).map(|v| v as u64);
        if copied < 1 || Some(u64::from_le_bytes(vtable)) != expected {
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
