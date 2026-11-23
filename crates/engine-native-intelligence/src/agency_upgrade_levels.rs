
use engine_native_memory::{
    Memory,
    cpdx_array::{self, Array},
    cpdx_lexer,
    read,
    walk::Outcome,
};
use engine_native::ccountry;
use crate::cintelligence_agency;

pub const ACQUIRED: usize = 0x60;
pub const ENTRY_STRIDE: usize = 16;
pub use crate::{
    agency_upgrade_branch::{UPGRADES as BRANCH_UPGRADES, VTABLE_RVA as BRANCH_VTABLE_RVA},
    agency_upgrade_branch_database::{
        DEFINITIONS as DATABASE_UPGRADES, ITEMS as DATABASE_BRANCHES,
        SINGLETON_RVA as DATABASE_RVA, VTABLE_RVA as DATABASE_VTABLE_RVA,
    },
    cagency_upgrade::{
        BRANCH as DEFINITION_BRANCH, TOKEN, VALID, VTABLE_RVA as DEFINITION_VTABLE_RVA,
    },
};

pub const STATUS_COUNT: usize = 15;
pub const AUTHENTICATED: u8 = 0;
pub const NULL_DEFINITION: u8 = 1;
pub const DATABASE_UNAVAILABLE: u8 = 2;
pub const DATABASE_UNSUPPORTED: u8 = 3;
pub const CATALOG_CAPACITY: u8 = 4;
pub const DEFINITION_NOT_REGISTERED: u8 = 5;
pub const DEFINITION_VTABLE_MISMATCH: u8 = 6;
pub const DEFINITION_INVALID: u8 = 7;
pub const DEFINITION_UNREADABLE: u8 = 8;
pub const BRANCH_UNAVAILABLE: u8 = 9;
pub const BRANCH_OWNER_MISMATCH: u8 = 10;
pub const UPGRADE_TOKEN_UNAVAILABLE: u8 = 11;
pub const BRANCH_TOKEN_UNAVAILABLE: u8 = 12;
pub const LEXER_CHANGED: u8 = 13;
pub const DUPLICATE_MEMBERSHIP: u8 = 14;

pub fn status_name(status: u8) -> &'static str {
    match status {
        AUTHENTICATED => "authenticated agency upgrade branch membership",
        NULL_DEFINITION => "agency upgrade definition pointer is null",
        DATABASE_UNAVAILABLE => "agency upgrade database unreadable or invalid",
        DATABASE_UNSUPPORTED => "agency upgrade database form unsupported",
        CATALOG_CAPACITY => "agency upgrade identity catalog capacity exceeded",
        DEFINITION_NOT_REGISTERED => {
            "agency upgrade definition not registered in a database branch"
        }
        DEFINITION_VTABLE_MISMATCH => "agency upgrade definition vtable unsupported",
        DEFINITION_INVALID => "agency upgrade definition valid byte is zero",
        DEFINITION_UNREADABLE => "agency upgrade definition fields unreadable",
        BRANCH_UNAVAILABLE => "agency upgrade database branch identity unavailable",
        BRANCH_OWNER_MISMATCH => "agency upgrade stored owner differs from database member branch",
        UPGRADE_TOKEN_UNAVAILABLE => "agency upgrade lexical token unavailable",
        BRANCH_TOKEN_UNAVAILABLE => "agency upgrade branch lexical token unavailable",
        LEXER_CHANGED => "agency upgrade lexical dictionary changed during read",
        DUPLICATE_MEMBERSHIP => {
            "agency upgrade definition pointer repeated in database branch membership"
        }
        _ => "unknown agency upgrade identity status",
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub ordinal: u32,
    pub definition_pointer: u64,
    pub branch_pointer: u64,
    pub upgrade_token: Option<u32>,
    pub branch_token: Option<u32>,
    pub level: i32,
    pub identity_status: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    pub country: u32,
    pub ordinal: u32,
    pub definition_pointer: u64,
    pub branch_pointer: u64,
    pub upgrade_token: Option<u32>,
    pub branch_token: Option<u32>,
    pub identity_status: u8,
}

pub struct Walk {
    pub outcome: Outcome,
    pub required_rows: usize,
    pub refused_rows: u32,
    pub omitted_rows: u32,
    pub first_refused: Option<Refusal>,
    pub refusal_counts: [u32; STATUS_COUNT],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub catalog: usize,
}

pub fn native_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<Demand, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut demand = Demand::default();
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let Some(agency) = cintelligence_agency::root(memory, base, country)? else {
            continue;
        };
        let acquired = array(memory, agency, ACQUIRED, ENTRY_STRIDE)?;
        demand.rows = demand
            .rows
            .checked_add(acquired.count as usize)
            .ok_or("agency upgrade row demand overflow")?;
    }
    if demand.rows != 0 {
        let (_, branches) = database(memory, base)?;
        branches_each(memory, base, branches, |_, members| {
            demand.catalog = demand
                .catalog
                .checked_add(members.count as usize)
                .ok_or("agency upgrade catalog demand overflow")?;
            Ok(())
        })?;
    }
    Ok(demand)
}

#[derive(Debug, Default, Clone, Copy)]
struct Identity {
    pointer: usize,
    branch_pointer: u64,
    upgrade_token: Option<u32>,
    branch_token: Option<u32>,
    status: u8,
}

pub struct Catalog {
    definitions: Vec<Identity>,
    growing: bool,
    len: usize,
    required: usize,
    prepared: bool,
    unavailable: Option<u8>,
    witness: Option<cpdx_lexer::PreparedWitness>,
}

impl Catalog {
    pub fn new(definition_capacity: usize) -> Self {
        Self {
            definitions: engine_allocation::owned_vec(engine_allocation::default_vec(
                definition_capacity,
            )),
            growing: false,
            len: 0,
            required: 0,
            prepared: false,
            unavailable: None,
            witness: None,
        }
    }
    pub fn capacity(&self) -> usize {
        self.definitions.len()
    }
    pub fn required_capacity(&self) -> usize {
        self.required
    }
    pub fn allocated_bytes(&self) -> usize {
        self.definitions.capacity() * std::mem::size_of::<Identity>()
    }
    pub fn resize(&mut self, definition_capacity: usize) {
        if engine_allocation::resize_default(&mut self.definitions, definition_capacity).is_err() {
            return;
        }
        self.reset();
    }
    fn reset(&mut self) {
        self.len = 0;
        self.required = 0;
        self.prepared = false;
        self.unavailable = None;
        self.witness = None;
    }
    fn prepare(&mut self, memory: &impl Memory, base: usize) {
        if self.prepared {
            return;
        }
        self.prepared = true;
        let result = (|| {
            let (globals, branches) = database(memory, base).map_err(|reason| match reason {
                "agency upgrade database vtable mismatch"
                | "agency upgrade database has no native null object slot" => DATABASE_UNSUPPORTED,
                _ => DATABASE_UNAVAILABLE,
            })?;
            let (tokens, witness) = cpdx_lexer::TokenNames::witnessed(memory, base)
                .map_err(|_| UPGRADE_TOKEN_UNAVAILABLE)?;
            self.witness = Some(witness);
            branches_each(memory, base, branches, |branch, members| {
                self.required = self
                    .required
                    .checked_add(members.count as usize)
                    .ok_or("agency upgrade catalog demand overflow")?;
                if self.growing {
                    engine_native_memory::walk::Buffer::Growing(&mut self.definitions).ensure(self.required)?;
                }
                let branch_token = token(memory, tokens, branch).ok();
                cpdx_array::pointers(
                    memory,
                    members,
                    "agency upgrade branch member unreadable",
                    |_, pointer| {
                        if self.len < self.definitions.len() {
                            self.definitions[self.len] =
                                identity(memory, base, tokens, pointer, branch, branch_token);
                            self.len += 1;
                        }
                        Ok(())
                    },
                )
            })
            .map_err(|_| BRANCH_UNAVAILABLE)?;
            if self.required > self.definitions.len() {
                return Err(CATALOG_CAPACITY);
            }
            self.definitions[..self.len].sort_unstable_by_key(|definition| definition.pointer);
            for index in 1..self.len {
                if self.definitions[index - 1].pointer == self.definitions[index].pointer {
                    self.definitions[index - 1].status = DUPLICATE_MEMBERSHIP;
                    self.definitions[index].status = DUPLICATE_MEMBERSHIP;
                }
            }
            let globals = globals
                .slice(1, globals.count - 1, 8)
                .map_err(|_| DATABASE_UNSUPPORTED)?;
            cpdx_array::pointers(
                memory,
                globals,
                "agency upgrade global pointer unreadable",
                |_, pointer| {
                    if self.definitions[..self.len]
                        .binary_search_by_key(&pointer, |d| d.pointer)
                        .is_err()
                    {
                        return Err("agency upgrade global definition has no branch membership");
                    }
                    Ok(())
                },
            )
            .map_err(|_| DATABASE_UNSUPPORTED)?;
            Ok(())
        })();
        self.unavailable = result.err();
    }
    fn resolve(&self, pointer: usize) -> Identity {
        let fallback = Identity {
            pointer,
            status: self.unavailable.unwrap_or(DEFINITION_NOT_REGISTERED),
            ..Default::default()
        };
        if pointer == 0 {
            return Identity {
                status: NULL_DEFINITION,
                ..fallback
            };
        }
        if self.unavailable.is_some() {
            return fallback;
        }
        self.definitions[..self.len]
            .binary_search_by_key(&pointer, |d| d.pointer)
            .map(|index| self.definitions[index])
            .unwrap_or(fallback)
    }
}

pub(crate) struct Capture<'a> {
    output: engine_native_memory::walk::Buffer<'a, Row>,
    catalog: &'a mut Catalog,
    len: usize,
    required_rows: usize,
    shortages: u32,
    refused_rows: u32,
    observed_rows: u32,
    refusal_counts: [u32; STATUS_COUNT],
    first_refused: Option<Refusal>,
    first_row: Option<Row>,
    failure: Option<&'static str>,
}

impl<'a> Capture<'a> {
    pub(crate) fn new_buffer(
        output: engine_native_memory::walk::Buffer<'a, Row>,
        catalog: &'a mut Catalog,
    ) -> Self {
        catalog.reset();
        catalog.growing = output.is_growing();
        Self {
            output,
            catalog,
            len: 0,
            required_rows: 0,
            shortages: 0,
            refused_rows: 0,
            observed_rows: 0,
            refusal_counts: [0; STATUS_COUNT],
            first_refused: None,
            first_row: None,
            failure: None,
        }
    }
    pub(crate) fn failed(&mut self, reason: &'static str) {
        self.failure.get_or_insert(reason);
    }
    pub(crate) fn agency(
        &mut self,
        memory: &impl Memory,
        base: usize,
        country: u32,
        agency: usize,
    ) {
        if self.failure.is_some() {
            return;
        }
        let result = (|| {
            let acquired = array(memory, agency, ACQUIRED, ENTRY_STRIDE)?;
            self.required_rows = self
                .required_rows
                .checked_add(acquired.count as usize)
                .ok_or("agency upgrade row demand overflow")?;
            self.output.ensure(self.required_rows)?;
            if acquired.count != 0 {
                self.catalog.prepare(memory, base);
            }
            for ordinal in 0..acquired.count {
                let entry = read::<[u8; ENTRY_STRIDE]>(
                    memory,
                    acquired
                        .element(ordinal, ENTRY_STRIDE)
                        .ok_or("agency upgrade entry address overflow")?,
                )
                .ok_or("agency upgrade acquired entry unreadable")?;
                let pointer = u64::from_le_bytes(entry[..8].try_into().unwrap());
                let identity = usize::try_from(pointer)
                    .ok()
                    .map(|p| self.catalog.resolve(p))
                    .unwrap_or(Identity {
                        status: DEFINITION_NOT_REGISTERED,
                        ..Default::default()
                    });
                let row = Row {
                    country,
                    ordinal,
                    definition_pointer: pointer,
                    branch_pointer: identity.branch_pointer,
                    upgrade_token: identity.upgrade_token,
                    branch_token: identity.branch_token,
                    level: i32::from_le_bytes(entry[8..12].try_into().unwrap()),
                    identity_status: identity.status,
                };
                self.observed_rows = self
                    .observed_rows
                    .checked_add(1)
                    .ok_or("agency upgrade observation count overflow")?;
                self.first_row.get_or_insert(row);
                self.refusal(row)?;
                if self.len == self.output.len() {
                    self.shortages = self
                        .shortages
                        .checked_add(1)
                        .ok_or("agency upgrade shortage count overflow")?;
                } else {
                    self.output[self.len] = row;
                    self.len += 1;
                }
            }
            Ok(())
        })();
        if let Err(reason) = result {
            self.failed(reason);
        }
    }
    fn refusal(&mut self, row: Row) -> Result<(), &'static str> {
        if row.identity_status == AUTHENTICATED {
            return Ok(());
        }
        self.refused_rows = self
            .refused_rows
            .checked_add(1)
            .ok_or("agency upgrade refusal count overflow")?;
        let count = &mut self.refusal_counts[usize::from(row.identity_status)];
        *count = count
            .checked_add(1)
            .ok_or("agency upgrade refusal reason count overflow")?;
        self.first_refused.get_or_insert(Refusal {
            country: row.country,
            ordinal: row.ordinal,
            definition_pointer: row.definition_pointer,
            branch_pointer: row.branch_pointer,
            upgrade_token: row.upgrade_token,
            branch_token: row.branch_token,
            identity_status: row.identity_status,
        });
        Ok(())
    }
    pub(crate) fn finish(mut self, memory: &impl Memory, base: usize) -> Walk {
        if let Some(witness) = self.catalog.witness
            && cpdx_lexer::unchanged(memory, base, witness).is_err()
        {
            self.first_refused = self.first_row.map(|row| Refusal {
                country: row.country,
                ordinal: row.ordinal,
                definition_pointer: row.definition_pointer,
                branch_pointer: row.branch_pointer,
                upgrade_token: None,
                branch_token: None,
                identity_status: LEXER_CHANGED,
            });
            self.refused_rows = self.observed_rows;
            self.refusal_counts.fill(0);
            self.refusal_counts[LEXER_CHANGED as usize] = self.observed_rows;
            for index in 0..self.len {
                self.output[index].identity_status = LEXER_CHANGED;
                self.output[index].upgrade_token = None;
                self.output[index].branch_token = None;
            }
        }
        let outcome = match self.failure {
            Some(reason) => Outcome::Unavailable { reason },
            None if self.shortages == 0 => Outcome::Rows { len: self.len },
            None => Outcome::Truncated {
                len: self.len,
                unread_rows: self.shortages,
            },
        };
        Walk {
            outcome,
            required_rows: self.required_rows,
            refused_rows: self.refused_rows,
            omitted_rows: self.shortages,
            first_refused: self.first_refused,
            refusal_counts: self.refusal_counts,
        }
    }
}

fn database(memory: &impl Memory, base: usize) -> Result<(Array, Array), &'static str> {
    let pointer = read::<u64>(
        memory,
        engine_sites::at(base, DATABASE_RVA).ok_or("agency upgrade address overflow")?,
    )
    .and_then(|p| usize::try_from(p).ok())
    .filter(|&p| p != 0)
    .ok_or("agency upgrade database pointer unreadable or null")?;
    if read::<u64>(memory, pointer)
        != Some(
            engine_sites::at(base, DATABASE_VTABLE_RVA).ok_or("agency upgrade address overflow")?
                as u64,
        )
    {
        return Err("agency upgrade database vtable mismatch");
    }
    let globals = array(memory, pointer, DATABASE_UPGRADES, 8)?;
    let branches = array(memory, pointer, DATABASE_BRANCHES, 8)?;
    if globals.count == 0 || branches.count == 0 {
        return Err("agency upgrade database has no native null object slot");
    }
    Ok((globals, branches))
}

fn branches_each(
    memory: &impl Memory,
    base: usize,
    branches: Array,
    mut each: impl FnMut(usize, Array) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    cpdx_array::pointers(
        memory,
        branches.slice(1, branches.count - 1, 8)?,
        "agency upgrade database branch pointer unreadable",
        |_, pointer| {
            if pointer == 0
                || read::<u64>(memory, pointer)
                    != Some(
                        engine_sites::at(base, BRANCH_VTABLE_RVA)
                            .ok_or("agency upgrade address overflow")?
                            as u64,
                    )
            {
                return Err("agency upgrade database branch vtable mismatch");
            }
            if read::<u8>(memory, at(pointer, VALID)?).is_none_or(|valid| valid == 0) {
                return Err("agency upgrade database branch valid byte mismatch");
            }
            each(pointer, array(memory, pointer, BRANCH_UPGRADES, 8)?)
        },
    )
}

fn identity(
    memory: &impl Memory,
    base: usize,
    tokens: cpdx_lexer::TokenNames,
    pointer: usize,
    branch: usize,
    branch_token: Option<u32>,
) -> Identity {
    let mut identity = Identity {
        pointer,
        status: NULL_DEFINITION,
        ..Default::default()
    };
    if pointer == 0 {
        return identity;
    }
    let Some(vtable) = read::<u64>(memory, pointer) else {
        identity.status = DEFINITION_UNREADABLE;
        return identity;
    };
    if engine_sites::at(base, DEFINITION_VTABLE_RVA).map(|p| p as u64) != Some(vtable) {
        identity.status = DEFINITION_VTABLE_MISMATCH;
        return identity;
    }
    let result = (|| {
        let valid = read::<u8>(
            memory,
            at(pointer, VALID).map_err(|_| DEFINITION_UNREADABLE)?,
        )
        .ok_or(DEFINITION_UNREADABLE)?;
        if valid == 0 {
            return Err(DEFINITION_INVALID);
        }
        identity.branch_pointer = read::<u64>(
            memory,
            at(pointer, DEFINITION_BRANCH).map_err(|_| DEFINITION_UNREADABLE)?,
        )
        .ok_or(DEFINITION_UNREADABLE)?;
        if identity.branch_pointer != branch as u64 {
            return Err(BRANCH_OWNER_MISMATCH);
        }
        identity.upgrade_token =
            Some(token(memory, tokens, pointer).map_err(|_| UPGRADE_TOKEN_UNAVAILABLE)?);
        identity.branch_token = Some(branch_token.ok_or(BRANCH_TOKEN_UNAVAILABLE)?);
        Ok(())
    })();
    identity.status = result.err().unwrap_or(AUTHENTICATED);
    identity
}

fn token(
    memory: &impl Memory,
    tokens: cpdx_lexer::TokenNames,
    pointer: usize,
) -> Result<u32, &'static str> {
    let token = read::<u32>(memory, at(pointer, TOKEN)?)
        .ok_or("agency upgrade lexical token unreadable")?;
    if tokens.text(memory, token)?.is_empty() {
        return Err("agency upgrade lexical token is empty");
    }
    Ok(token)
}
fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("agency upgrade address overflow")
}
fn array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    Array::read(memory, at(pointer, offset)?, stride)
        .ok_or("agency upgrade array header unreadable or invalid")
}

#[cfg(test)]
#[path = "agency_upgrade_levels/tests.rs"]
mod tests;
