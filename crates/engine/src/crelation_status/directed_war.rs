//! Optional direct-war observations in the sparse `links` pass. Direct-war
//! and selected-attack predicates use a nonnull war pointer and its nonretired
//! byte. Owned typed war endpoints are candidates, without a completeness
//! certificate. Missing cells remain unknown; dates, symmetry and enemy sets
//! do not substitute. Exact query identities add sparse demand only at the
//! admitted hourly boundary; query values are never carried.
//! Visit every candidate in original order, including repeats, before the
//! existing type and retirement branches. Optional faults do not alter LinkWalk.

use super::{DIPLOMACY_STATUS, Resolver, TARGET_COUNTRY, VTABLE_RVA, WAR_RELATION};
use crate::{
    Memory, ccountry, cdiplomacy_status,
    cpdx_array::Array,
    crelation, cwar_relation,
    directed_war_queries::{self, Demand},
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Buffer,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DirectedWarOwner {
    pub country: u32,
    pub canonical_slot: u32,
    pub matrix_count: u32,
    pub cells_start: usize,
    pub cells_len: usize,
    /// Discovery/header/capacity failure withholds this owner's whole packet.
    pub reason: Option<&'static str>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DirectedWarObservation {
    pub target_slot: u32,
    /// Exact null/retired predicate, or a refused cell. Never inferred.
    pub active: Option<bool>,
    pub reason: Option<&'static str>,
}

pub struct DirectedWarOutput<'a> {
    pub owners: &'a mut [DirectedWarOwner],
    pub cells: &'a mut [DirectedWarObservation],
}

pub struct DirectedWarBuffers<'a> {
    pub owners: Buffer<'a, DirectedWarOwner>,
    pub cells: Buffer<'a, DirectedWarObservation>,
}

impl<'a> From<DirectedWarOutput<'a>> for DirectedWarBuffers<'a> {
    fn from(output: DirectedWarOutput<'a>) -> Self {
        Self {
            owners: Buffer::Fixed(output.owners),
            cells: Buffer::Fixed(output.cells),
        }
    }
}

pub const DIRECTED_WAR_FAULT_NAMES: [&str; 9] = [
    "directed war pass discovery failed",
    "directed war owner identity refused",
    "directed war matrix header refused",
    "directed war candidate identity refused",
    "directed war endpoint discovery refused",
    "directed war status identity refused",
    "directed war cell unreadable",
    "directed war prepared arena capacity exceeded",
    "directed war query demand refused",
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DirectedWarWalk {
    pub owners_len: usize,
    pub cells_len: usize,
    pub required_owners: usize,
    /// Sum of live owned-vector counts and admitted query demand. Includes
    /// non-war relations, repeats and rows not fitting the arenas.
    pub required_cells: usize,
    pub required_demands: usize,
    pub refused_demands: u64,
    /// A refused demand suffix never withholds independently observed cells.
    pub demand_reason: Option<&'static str>,
    pub observed_cells: u64,
    pub refused_cells: u64,
    pub faults: [u64; DIRECTED_WAR_FAULT_NAMES.len()],
    pub pass_reason: Option<&'static str>,
}

pub(super) struct Observer<'a> {
    output: Option<DirectedWarBuffers<'a>>,
    pub walk: DirectedWarWalk,
    current: Option<usize>,
    demands: &'a [Demand],
    attempted_demands: usize,
}

impl<'a> Observer<'a> {
    pub fn new(output: Option<DirectedWarBuffers<'a>>, demands: &'a [Demand]) -> Self {
        let required = output.as_ref().map_or(0, |_| demands.len());
        let mut observer = Self {
            output,
            walk: DirectedWarWalk {
                required_cells: required,
                required_demands: required,
                ..Default::default()
            },
            current: None,
            demands,
            attempted_demands: 0,
        };
        if required != 0
            && !demands
                .windows(2)
                .all(|rows| rows[0].receiver_raw <= rows[1].receiver_raw)
        {
            observer.refuse_demands(
                required,
                "directed war query demand is not sorted by receiver",
            );
            observer.demands = &[];
            observer.attempted_demands = required;
        }
        observer
    }

    fn fault(&mut self, index: usize) {
        self.walk.faults[index] = self.walk.faults[index].saturating_add(1);
    }

    fn owner_refusal(&mut self, index: usize, reason: &'static str) {
        self.fault(index);
        if let (Some(output), Some(current)) = (self.output.as_mut(), self.current) {
            output.owners[current].reason.get_or_insert(reason);
        }
    }

    fn refuse_demands(&mut self, count: usize, reason: &'static str) {
        self.walk.refused_demands = self.walk.refused_demands.saturating_add(count as u64);
        self.walk.faults[8] = self.walk.faults[8].saturating_add(count as u64);
        self.walk.demand_reason.get_or_insert(reason);
    }

    /// Return the already-observed header, including its failure, so the old
    /// links consumer shares it and never retries an optional failed read.
    /// None means no sink: links retains its original lazy header behavior.
    pub fn begin(
        &mut self,
        memory: &impl Memory,
        country: ccountry::Country,
        diplomacy: usize,
        owned_count: u32,
    ) -> Option<Result<Array, &'static str>> {
        self.current = None;
        self.output.as_ref()?;
        self.walk.required_owners = match self.walk.required_owners.checked_add(1) {
            Some(value) => value,
            None => {
                self.walk.pass_reason = Some("directed war owner demand overflow");
                return None;
            }
        };
        self.walk.required_cells = match self.walk.required_cells.checked_add(owned_count as usize)
        {
            Some(value) => value,
            None => {
                self.walk.pass_reason = Some("directed war cell demand overflow");
                return None;
            }
        };
        let output = self.output.as_mut().unwrap();
        // Direct-war demand visits owned candidates and admitted query identities.
        if let Err(reason) = output
            .owners
            .ensure(self.walk.required_owners)
            .and_then(|_| output.cells.ensure(self.walk.required_cells))
        {
            self.walk.pass_reason = Some(reason);
            self.fault(7);
            return None;
        }
        if let Some(row) = output.owners.get_mut(self.walk.owners_len) {
            *row = DirectedWarOwner {
                country: country.id,
                canonical_slot: country.canonical_slot,
                cells_start: self.walk.cells_len,
                ..Default::default()
            };
            self.current = Some(self.walk.owners_len);
            self.walk.owners_len += 1;
        } else {
            self.fault(7);
        }
        if country.id as i32 <= 0 || country.canonical_slot == 0 {
            self.owner_refusal(
                1,
                "directed war receiver is not an authenticated positive country",
            );
        }
        let matrix = diplomacy
            .checked_add(cdiplomacy_status::RELATIONS)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("relation matrix unreadable or invalid");
        // Direct-war lookup uses full signed-32-bit slots; retain the signed array
        // count without a 16-bit restriction.
        match matrix {
            Ok(matrix) if matrix.count != 0 && country.canonical_slot < matrix.count => {
                if let (Some(output), Some(current)) = (self.output.as_mut(), self.current) {
                    output.owners[current].matrix_count = matrix.count;
                }
            }
            _ => self.owner_refusal(2, "directed war native matrix extent refused"),
        }
        Some(matrix)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn candidate<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        resolver: &mut Resolver,
        country: ccountry::Country,
        diplomacy: usize,
        matrix: Option<Result<Array, &'static str>>,
        pointer: usize,
        first_raw: u32,
        second_raw: u32,
    ) {
        let Some(current) = self.current else {
            return;
        };
        if self.output.as_ref().unwrap().owners[current]
            .reason
            .is_some()
        {
            return;
        }
        if self.walk.cells_len == self.output.as_ref().unwrap().cells.len() {
            self.owner_refusal(7, "directed war cell arena capacity exceeded");
            return;
        }
        let expected = base
            .checked_add(cwar_relation::VTABLE_RVA)
            .map(|at| at as u64);
        if expected.is_none() || read::<u64>(memory, pointer) != expected {
            self.owner_refusal(
                3,
                "directed war owned candidate vtable mismatch or unreadable",
            );
            return;
        }
        // The shared batch already acquired the candidate's exact type and
        // endpoints. Authenticate both objects; raw zero uses only the root
        // observer, never a new owner-zero matrix walk.
        let resolve = |resolver: &mut Resolver, raw| {
            if raw == 0 {
                countries.reserved()
            } else {
                resolver.resolve(countries, raw)
            }
        };
        let first = match resolve(resolver, first_raw) {
            Ok(first) => first,
            Err(reason) => {
                self.owner_refusal(4, reason);
                return;
            }
        };
        let second = match resolve(resolver, second_raw) {
            Ok(second) => second,
            Err(reason) => {
                self.owner_refusal(4, reason);
                return;
            }
        };
        let target = if first.canonical_slot == country.canonical_slot
            && second.canonical_slot != country.canonical_slot
        {
            second
        } else if second.canonical_slot == country.canonical_slot
            && first.canonical_slot != country.canonical_slot
        {
            first
        } else {
            self.owner_refusal(
                4,
                "directed war candidate does not name a distinct owner endpoint",
            );
            return;
        };
        let result = match matrix {
            Some(Ok(matrix)) => cell(memory, base, countries, resolver, diplomacy, matrix, target),
            _ => Err((5, "directed war matrix header unavailable")),
        };
        let (active, reason) = match result {
            Ok(active) => {
                self.walk.observed_cells = self.walk.observed_cells.saturating_add(1);
                (Some(active), None)
            }
            Err((index, reason)) => {
                self.fault(index);
                self.walk.refused_cells = self.walk.refused_cells.saturating_add(1);
                (None, Some(reason))
            }
        };
        let output = self.output.as_mut().unwrap();
        output.cells[self.walk.cells_len] = DirectedWarObservation {
            target_slot: target.canonical_slot,
            active,
            reason,
        };
        output.owners[current].cells_len += 1;
        self.walk.cells_len += 1;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn demands<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        resolver: &mut Resolver,
        country: ccountry::Country,
        diplomacy: usize,
        matrix: Option<Result<Array, &'static str>>,
    ) {
        if self.output.is_none() {
            return;
        }
        let start = self
            .demands
            .partition_point(|d| d.receiver_raw < country.id);
        let len = self.demands[start..].partition_point(|d| d.receiver_raw == country.id);
        if len == 0 {
            return;
        }
        self.attempted_demands += len;
        let Some(current) = self
            .current
            .filter(|&at| self.output.as_ref().unwrap().owners[at].reason.is_none())
        else {
            self.refuse_demands(len, "directed war query owner observation unavailable");
            return;
        };
        let instance = country
            .pointer
            .checked_add(ccountry::ID_INSTANCE)
            .and_then(|at| read::<u32>(memory, at));
        let demands = self.demands;
        for demand in &demands[start..start + len] {
            let result = (|| {
                if demand.receiver != country.pointer
                    || demand.receiver_raw != country.id
                    || demand.diplomacy != diplomacy
                    || instance != Some(demand.receiver_instance)
                {
                    return Err("directed war query receiver occurrence differs from demand");
                }
                let mut resolve = |raw| {
                    if raw == 0 || countries.link(raw)?.is_none() {
                        countries.reserved()
                    } else {
                        resolver.resolve(countries, raw)
                    }
                };
                let target = resolve(demand.target_raw)?;
                if target.canonical_slot != demand.target_slot {
                    return Err("directed war query target canonical slot changed");
                }
                let matrix = matrix.ok_or("directed war query matrix header unavailable")??;
                directed_war_queries::authenticate_status(
                    memory,
                    base,
                    diplomacy,
                    matrix,
                    demand.status,
                    target,
                    resolve,
                )?;
                Ok(target)
            })();
            // An attempted sparse cell retains its value or first failure. Authentication
            // never retries war pointer or retirement reads. Consult the original sparse
            // span using full signed slots.
            let observed = demand.target_slot
                < self.output.as_ref().unwrap().owners[current].matrix_count
                && {
                    let output = self.output.as_ref().unwrap();
                    let owner = output.owners[current];
                    output.cells[owner.cells_start..][..owner.cells_len]
                        .iter()
                        .any(|cell| cell.target_slot == demand.target_slot)
                };
            if let Err(reason) = result {
                self.refuse_demands(1, reason);
                if observed {
                    continue;
                }
            } else if observed {
                continue;
            }
            if self.walk.cells_len == self.output.as_ref().unwrap().cells.len() {
                self.fault(7);
                if result.is_ok() {
                    self.refuse_demands(1, "directed war demanded cell arena capacity exceeded");
                }
                continue;
            }
            // A malformed/out-of-matrix slot is counted without manufacturing
            // a replay target outside the authenticated owner's native extent.
            if demand.target_slot >= self.output.as_ref().unwrap().owners[current].matrix_count {
                continue;
            }
            let (active, reason) = match result {
                Ok(_) => match active(memory, demand.status) {
                    Ok(value) => {
                        self.walk.observed_cells = self.walk.observed_cells.saturating_add(1);
                        (Some(value), None)
                    }
                    Err(reason) => {
                        self.fault(6);
                        self.refuse_demands(1, reason);
                        (None, Some(reason))
                    }
                },
                Err(reason) => (None, Some(reason)),
            };
            if active.is_none() {
                self.walk.refused_cells = self.walk.refused_cells.saturating_add(1);
            }
            let output = self.output.as_mut().unwrap();
            output.cells[self.walk.cells_len] = DirectedWarObservation {
                target_slot: demand.target_slot,
                active,
                reason,
            };
            output.owners[current].cells_len += 1;
            self.walk.cells_len += 1;
        }
    }

    pub fn finish(&mut self, reason: Option<&'static str>) {
        if self.output.is_some() && self.attempted_demands < self.walk.required_demands {
            self.refuse_demands(
                self.walk.required_demands - self.attempted_demands,
                "directed war query receiver absent from authenticated pass",
            );
        }
        if self.output.is_some()
            && let Some(reason) = reason
        {
            self.walk.pass_reason.get_or_insert(reason);
            self.fault(0);
        }
        self.current = None;
    }
}

fn active(memory: &impl Memory, status: usize) -> Result<bool, &'static str> {
    // War and friendliness predicates require pointer presence followed by
    // the pointed byte. No type, date, opposite cell or enemy-set gate.
    let war = status
        .checked_add(WAR_RELATION)
        .and_then(|at| read::<u64>(memory, at))
        .ok_or("directed war relation status pointer unreadable")?;
    if war == 0 {
        return Ok(false);
    }
    usize::try_from(war)
        .ok()
        .and_then(|p| p.checked_add(crelation::RETIRED))
        .and_then(|at| read::<u8>(memory, at))
        .map(|retired| retired == 0)
        .ok_or("directed war pointed retired byte unreadable")
}

fn cell<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    resolver: &mut Resolver,
    diplomacy: usize,
    matrix: Array,
    target: ccountry::Country,
) -> Result<bool, (usize, &'static str)> {
    let status = matrix
        .element(target.canonical_slot, 8)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|raw| usize::try_from(raw).ok())
        .filter(|&raw| raw != 0)
        .ok_or((5, "directed war matrix target missing or unreadable"))?;
    const FIELDS: [RecordField; 4] = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: DIPLOMACY_STATUS,
            destination_offset: 8,
            len: 8,
        },
        RecordField {
            source_offset: TARGET_COUNTRY,
            destination_offset: 16,
            len: 4,
        },
        RecordField {
            source_offset: WAR_RELATION,
            destination_offset: 20,
            len: 8,
        },
    ];
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&FIELDS, 28).expect("valid direct-war layout");
    let mut bytes = [0; 28];
    let copied = memory.copy_records(RecordSources::Pointers(&[status]), &LAYOUT, &mut bytes);
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or((5, "directed war module address overflow"))?;
    if copied == 0 || u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected as u64 {
        return Err((5, "directed war relation status vtable refused"));
    }
    if copied < 2 || u64::from_le_bytes(bytes[8..16].try_into().unwrap()) != diplomacy as u64 {
        return Err((5, "directed war relation status owner refused"));
    }
    if copied < 3 {
        return Err((5, "directed war relation status target unreadable"));
    }
    let raw = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let actual = if raw == 0 {
        countries.reserved()
    } else {
        resolver.resolve(countries, raw)
    }
    .map_err(|_| (5, "directed war relation status target unbound"))?;
    if actual.pointer != target.pointer || actual.canonical_slot != target.canonical_slot {
        return Err((
            5,
            "directed war relation status target differs from candidate",
        ));
    }
    if copied < 4 {
        return Err((6, "directed war relation status pointer unreadable"));
    }
    let war = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
    if war == 0 {
        return Ok(false);
    }
    // Predicate-only: do not substitute candidate retired state or demand
    // pointer equality with the discovery relation. Native tests this byte.
    usize::try_from(war)
        .ok()
        .and_then(|p| p.checked_add(crelation::RETIRED))
        .and_then(|at| read::<u8>(memory, at))
        .map(|retired| retired == 0)
        .ok_or((6, "directed war pointed retired byte unreadable"))
}
