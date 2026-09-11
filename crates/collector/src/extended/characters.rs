//! Persistent character state. Native walks use pooled rows and literal-byte
//! arenas; sorting, duplicate checks and owned replay strings run off-thread.

use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, active_ability_record, cadvisor, carmy_leader, ccountry_leader, ccharacter,
    ccharacter_manager, ccountry, cgame_state, character_advisor_node, character_leader_node,
    cnavy_leader, coperative_leader, cpdx_array::Array,
    cunit_leader, cunit_leader_skill, cunit_leader_trait, msvc_string, msvc_tree_node, read,
    unit_leader_trait_experience_entry, walk::Outcome,
};
use replay::{
    frame::{TABLE_COUNT, TableView},
    rows::{
        ActiveAbilityRow, AdvisorRoleRow, CharacterRow, CountryLeaderRoleRow, TableId, TableSchema,
        UnitLeaderRow, UnitLeaderTraitExperienceRow, UnitLeaderTraitRow, tables,
    },
};
use std::collections::HashMap;

pub const N: usize = 7;
pub const TABLES: [TableId; N] = [
    TableId::Characters,
    TableId::CountryLeaderRoles,
    TableId::AdvisorRoles,
    TableId::UnitLeaders,
    TableId::UnitLeaderTraits,
    TableId::UnitLeaderTraitExperience,
    TableId::ActiveAbilities,
];

/// The first table in each group carries its full QPC cost, including requested
/// comparison reads. Companion slots are zero: their shared traversal has one
/// measured cost, not an invented allocation of time between its outputs.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    (
        "characters, persistent roles and traits",
        0,
        &[0, 1, 2, 3, 4, 5],
    ),
    ("active abilities", 6, &[6]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub bytes: [usize; N],
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            // First full capture contained 13,095 characters, 2,019 leader
            // roles, 1,271 unit leaders and 19,555 trait-experience entries.
            rows: [16_384, 4096, 4096, 2048, 4096, 32_768, 128],
            bytes: [524_288, 0, 262_144, 131_072, 131_072, 524_288, 8192],
        }
    }
}

struct Buffer<R> {
    rows: CaptureRows<R>,
    bytes: Arena<u8>,
}

impl<R: Default + Clone + PartialEq> Buffer<R> {
    fn new(capacities: &Capacities, index: usize, successful: &[u16]) -> Self {
        let successful = successful.get(index).copied().unwrap_or(0);
        Self {
            rows: CaptureRows::new(capacities.rows[index], successful),
            bytes: Arena::new(capacities.bytes[index], successful),
        }
    }

    fn reset(&mut self) {
        self.rows.reset();
        self.bytes.reset();
    }

    fn grow(&mut self, capacities: &mut Capacities, index: usize, successful: &[u16]) {
        let successful = successful.get(index).copied().unwrap_or(0);
        self.rows.grow(&mut capacities.rows[index], successful);
        self.bytes.grow(&mut capacities.bytes[index], successful);
    }

    fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.bytes.allocated_bytes()
    }

    fn batch(&mut self, second: bool, enabled: bool) -> Batch<'_, R> {
        if second {
            Batch::new(&mut self.rows.second, &mut self.bytes.second, enabled)
        } else {
            Batch::new(&mut self.rows.rows, &mut self.bytes.first, enabled)
        }
    }

    fn primary(&mut self, result: ReadResult) {
        self.rows.outcome = result.outcome;
        self.bytes.len = result.bytes_len;
        self.bytes.required = result.required_bytes;
    }

    fn compare(&mut self, result: ReadResult) -> bool {
        self.rows.checked = true;
        let rows_match = self.rows.compare(result.outcome);
        let bytes_match = self.bytes.compare(result.bytes_len, result.required_bytes);
        self.rows.mismatches += usize::from(!bytes_match);
        rows_match && bytes_match
    }

    fn capture(
        &mut self,
        check: bool,
        mut walk: impl FnMut(&mut [R], &mut [u8]) -> ReadResult,
    ) -> (bool, u64) {
        let begin = qpc();
        let first = walk(&mut self.rows.rows, &mut self.bytes.first);
        self.primary(first);
        let matched = if check {
            let second = walk(&mut self.rows.second, &mut self.bytes.second);
            self.compare(second)
        } else {
            false
        };
        (matched, elapsed(begin))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct LeaderRow {
    native: cunit_leader::Row,
    text_a_start: usize,
    text_a_len: usize,
    text_b_start: usize,
    text_b_len: usize,
}

pub struct Captured {
    characters: Buffer<ccharacter::Row>,
    leaders: Buffer<ccountry_leader::Row>,
    advisors: Buffer<cadvisor::Row>,
    units: Buffer<LeaderRow>,
    traits: Buffer<cunit_leader::TraitRow>,
    trait_experience: Buffer<cunit_leader::TraitExperienceRow>,
    abilities: Buffer<active_ability_record::Row>,
    /// Raw-copy split gate for the Tier-4 slot-0 family (tables 0..6). Set by
    /// the game thread on refresh hours with a valid raw payload; `capture`
    /// then skips the typed family walks and the decode worker fills these
    /// rows before the ready handoff. Preserved across `reset` (set per
    /// refresh, cleared by the worker after decode). ActiveAbilities stays
    /// hourly and is never gated by this flag.
    pub(crate) raw_mode: bool,
}

impl Captured {
    fn family(&mut self, second: bool, enabled: [bool; 6]) -> Family<'_> {
        Family {
            characters: self.characters.batch(second, enabled[0]),
            leaders: self.leaders.batch(second, enabled[1]),
            advisors: self.advisors.batch(second, enabled[2]),
            units: self.units.batch(second, enabled[3]),
            traits: self.traits.batch(second, enabled[4]),
            experience: self.trait_experience.batch(second, enabled[5]),
        }
    }

    pub fn with_capacities(capacities: &Capacities, successful: &[u16]) -> Self {
        Self {
            characters: Buffer::new(capacities, 0, successful),
            leaders: Buffer::new(capacities, 1, successful),
            advisors: Buffer::new(capacities, 2, successful),
            units: Buffer::new(capacities, 3, successful),
            traits: Buffer::new(capacities, 4, successful),
            trait_experience: Buffer::new(capacities, 5, successful),
            abilities: Buffer::new(capacities, 6, successful),
            raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.characters.reset();
        self.leaders.reset();
        self.advisors.reset();
        self.units.reset();
        self.traits.reset();
        self.trait_experience.reset();
        self.abilities.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.characters.allocated_bytes()
            + self.leaders.allocated_bytes()
            + self.advisors.allocated_bytes()
            + self.units.allocated_bytes()
            + self.traits.allocated_bytes()
            + self.trait_experience.allocated_bytes()
            + self.abilities.allocated_bytes()
    }

    pub fn grow(&mut self, capacities: &mut Capacities, successful: &[u16]) {
        self.characters.grow(capacities, 0, successful);
        self.leaders.grow(capacities, 1, successful);
        self.advisors.grow(capacities, 2, successful);
        self.units.grow(capacities, 3, successful);
        self.traits.grow(capacities, 4, successful);
        self.trait_experience.grow(capacities, 5, successful);
        self.abilities.grow(capacities, 6, successful);
    }

    /// Every selected check is independent. The unit family shares its roster
    /// visit, but one failed or complete comparison window cannot mask another.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
        hour: i32,
        force_full: bool,
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        let check = |index| needed.get(index).copied().unwrap_or(false);
        let mut matched = [false; N];
        let mut ticks = [0; N];
        // Tier-4 family shares one traversal; off-hours skip the whole walk.
        // Raw-copy split: on refresh hours the game thread stages raw bytes
        // via `copy_characters_refresh` and the decode worker fills the typed
        // rows before the ready handoff, so the typed family walk is skipped
        // here. Matched flags and costs for the family come from the raw
        // compare, applied by the caller after this returns. ActiveAbilities
        // stays hourly and is never gated.
        let walk = replay::cadence::should_walk(
            replay::rows::TableId::Characters,
            hour,
            force_full,
        ) && !self.raw_mode;
        if self.raw_mode {
            // Refresh payload staged: leave the reset outcomes for the decode
            // worker, which installs identical rows, outcomes and arenas.
        } else if !walk {
            let skipped = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.characters.rows.outcome = skipped;
            self.leaders.rows.outcome = skipped;
            self.advisors.rows.outcome = skipped;
            self.units.rows.outcome = skipped;
            self.traits.rows.outcome = skipped;
            self.trait_experience.rows.outcome = skipped;
        } else {
            let begin = qpc();
            let first = walk_family(memory, base, game_state, self.family(false, [true; 6]));
            self.characters.primary(first[0]);
            self.leaders.primary(first[1]);
            self.advisors.primary(first[2]);
            self.units.primary(first[3]);
            self.traits.primary(first[4]);
            self.trait_experience.primary(first[5]);
            if (0..6).any(check) {
                let second = walk_family(
                    memory,
                    base,
                    game_state,
                    self.family(true, std::array::from_fn(check)),
                );
                if check(0) {
                    matched[0] = self.characters.compare(second[0]);
                }
                if check(1) {
                    matched[1] = self.leaders.compare(second[1]);
                }
                if check(2) {
                    matched[2] = self.advisors.compare(second[2]);
                }
                if check(3) {
                    matched[3] = self.units.compare(second[3]);
                }
                if check(4) {
                    matched[4] = self.traits.compare(second[4]);
                }
                if check(5) {
                    matched[5] = self.trait_experience.compare(second[5]);
                }
            }
            ticks[0] = elapsed(begin);
        }
        (matched[6], ticks[6]) = self.abilities.capture(check(6), |rows, bytes| {
            walk_abilities(memory, base, game_state, rows, bytes)
        });
        (matched, ticks)
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.characters.rows.outcome,
            self.leaders.rows.outcome,
            self.advisors.rows.outcome,
            self.units.rows.outcome,
            self.traits.rows.outcome,
            self.trait_experience.rows.outcome,
            self.abilities.rows.outcome,
        ]
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.characters.rows.mismatches,
            self.leaders.rows.mismatches,
            self.advisors.rows.mismatches,
            self.units.rows.mismatches,
            self.traits.rows.mismatches,
            self.trait_experience.rows.mismatches,
            self.abilities.rows.mismatches,
        ]
    }

    /// Called only by the spool thread after the buffer has left the callback.
    pub fn tables(&self) -> [TableView; N] {
        [
            view::<tables::Characters, _>(
                &self.characters,
                |row, bytes| {
                    valid(bytes, row.name_start, row.name_len)
                        && valid(bytes, row.token_start, row.token_len)
                },
                |row, bytes| CharacterRow {
                    id: row.id,
                    country: row.country,
                    name_token_id: row.name_token_id,
                    roster: row.roster,
                    ordinal: row.ordinal,
                    name: owned(bytes, row.name_start, row.name_len),
                    token: owned(bytes, row.token_start, row.token_len),
                },
            ),
            view::<tables::CountryLeaderRoles, _>(
                &self.leaders,
                |_, _| true,
                |row, _| CountryLeaderRoleRow {
                    character: row.character,
                    country: row.country,
                    ideology: row.ideology,
                },
            ),
            view::<tables::AdvisorRoles, _>(
                &self.advisors,
                |row, bytes| {
                    valid(bytes, row.key_start, row.key_len)
                        && valid(bytes, row.idea_start, row.idea_len)
                        && valid(bytes, row.slot_start, row.slot_len)
                },
                |row, bytes| AdvisorRoleRow {
                    character: row.character,
                    country: row.country,
                    ordinal: row.ordinal,
                    key: owned(bytes, row.key_start, row.key_len),
                    idea: owned(bytes, row.idea_start, row.idea_len),
                    slot: owned(bytes, row.slot_start, row.slot_len),
                },
            ),
            view::<tables::UnitLeaders, _>(
                &self.units,
                |row, bytes| {
                    valid(bytes, row.text_a_start, row.text_a_len)
                        && valid(bytes, row.text_b_start, row.text_b_len)
                },
                leader_row,
            ),
            view::<tables::UnitLeaderTraits, _>(
                &self.traits,
                |row, bytes| valid(bytes, row.token_start, row.token_len),
                |row, bytes| UnitLeaderTraitRow {
                    leader_reference: leader_reference(row.leader_id),
                    ordinal: row.ordinal,
                    token_id: row.token_id,
                    definition_ordinal: row.definition_ordinal,
                    token: owned(bytes, row.token_start, row.token_len),
                },
            ),
            view::<tables::UnitLeaderTraitExperience, _>(
                &self.trait_experience,
                |row, bytes| valid(bytes, row.trait_row.token_start, row.trait_row.token_len),
                |row, bytes| UnitLeaderTraitExperienceRow {
                    leader_reference: leader_reference(row.trait_row.leader_id),
                    ordinal: row.trait_row.ordinal,
                    token_id: row.trait_row.token_id,
                    definition_ordinal: row.trait_row.definition_ordinal,
                    token: owned(bytes, row.trait_row.token_start, row.trait_row.token_len),
                    experience: row.experience,
                },
            ),
            view::<tables::ActiveAbilities, _>(
                &self.abilities,
                |row, bytes| valid(bytes, row.token_start, row.token_len),
                ability_row,
            ),
        ]
    }
}

fn elapsed(begin: i64) -> u64 {
    (qpc() - begin).max(0) as u64
}

#[derive(Clone, Copy)]
struct ReadResult {
    outcome: Outcome,
    bytes_len: usize,
    required_bytes: usize,
}

impl ReadResult {
    fn rows(outcome: Outcome) -> Self {
        Self {
            outcome,
            bytes_len: 0,
            required_bytes: 0,
        }
    }
}

impl From<ccharacter::NameWalk> for ReadResult {
    fn from(result: ccharacter::NameWalk) -> Self {
        Self {
            outcome: result.outcome,
            bytes_len: result.bytes_len,
            required_bytes: result.required_bytes,
        }
    }
}

impl From<cunit_leader::TraitWalk> for ReadResult {
    fn from(result: cunit_leader::TraitWalk) -> Self {
        Self {
            outcome: result.outcome,
            bytes_len: result.bytes_len,
            required_bytes: result.required_bytes,
        }
    }
}

impl From<active_ability_record::TokenWalk> for ReadResult {
    fn from(result: active_ability_record::TokenWalk) -> Self {
        Self {
            outcome: result.outcome,
            bytes_len: result.bytes_len,
            required_bytes: result.required_bytes,
        }
    }
}

/// Aggregates exact native counts across owners. After the first capacity miss
/// later owners contribute counts only; no later row fills a hole in the prefix.
struct Batch<'a, R> {
    rows: &'a mut [R],
    bytes: &'a mut [u8],
    len: usize,
    bytes_len: usize,
    required_bytes: usize,
    unread: u32,
    failure: Option<&'static str>,
    enabled: bool,
}

impl<'a, R> Batch<'a, R> {
    fn new(rows: &'a mut [R], bytes: &'a mut [u8], enabled: bool) -> Self {
        Self {
            rows,
            bytes,
            len: 0,
            bytes_len: 0,
            required_bytes: 0,
            unread: 0,
            failure: None,
            enabled,
        }
    }

    fn active(&self) -> bool {
        self.enabled && self.failure.is_none()
    }

    fn fail(&mut self, reason: &'static str) {
        if self.enabled && self.failure.is_none() {
            self.failure = Some(reason);
        }
    }

    fn append(
        &mut self,
        walk: impl FnOnce(&mut [R], &mut [u8]) -> ReadResult,
        relocate: impl Fn(&mut R, usize) -> bool,
    ) {
        if !self.active() {
            return;
        }
        let rows_end = if self.unread == 0 {
            self.rows.len()
        } else {
            self.len
        };
        let bytes_end = if self.unread == 0 {
            self.bytes.len()
        } else {
            self.bytes_len
        };
        let result = walk(
            &mut self.rows[self.len..rows_end],
            &mut self.bytes[self.bytes_len..bytes_end],
        );
        let (len, unread) = match result.outcome {
            Outcome::Rows { len } => (len, 0),
            Outcome::Truncated { len, unread_rows } if unread_rows != 0 => (len, unread_rows),
            Outcome::Truncated { .. } => {
                self.fail("character walk returned truncation without missing rows");
                return;
            }
            Outcome::Unavailable { reason } => {
                self.fail(reason);
                return;
            }
        };
        if len > rows_end - self.len
            || result.bytes_len > bytes_end - self.bytes_len
            || result.required_bytes < result.bytes_len
        {
            self.fail("character walk exceeded its pooled buffer");
            return;
        }
        let bytes_start = self.bytes_len;
        if !self.rows[self.len..self.len + len]
            .iter_mut()
            .all(|row| relocate(row, bytes_start))
        {
            self.fail("character byte offset overflow");
            return;
        }
        let Some(required) = self.required_bytes.checked_add(result.required_bytes) else {
            self.fail("character byte count overflow");
            return;
        };
        let Some(unread) = self.unread.checked_add(unread) else {
            self.fail("character missing row count overflow");
            return;
        };
        self.len += len;
        self.bytes_len += result.bytes_len;
        self.required_bytes = required;
        self.unread = unread;
    }

    fn result(&self) -> ReadResult {
        if !self.enabled {
            return ReadResult::rows(Outcome::Unavailable {
                reason: "character comparison not requested",
            });
        }
        if let Some(reason) = self.failure {
            return ReadResult::rows(Outcome::Unavailable { reason });
        }
        ReadResult {
            outcome: if self.unread == 0 {
                Outcome::Rows { len: self.len }
            } else {
                Outcome::Truncated {
                    len: self.len,
                    unread_rows: self.unread,
                }
            },
            bytes_len: self.bytes_len,
            required_bytes: self.required_bytes,
        }
    }
}

fn relocate(start: &mut usize, base: usize) -> bool {
    let Some(value) = start.checked_add(base) else {
        return false;
    };
    *start = value;
    true
}

struct Family<'a> {
    characters: Batch<'a, ccharacter::Row>,
    leaders: Batch<'a, ccountry_leader::Row>,
    advisors: Batch<'a, cadvisor::Row>,
    units: Batch<'a, LeaderRow>,
    traits: Batch<'a, cunit_leader::TraitRow>,
    experience: Batch<'a, cunit_leader::TraitExperienceRow>,
}

fn walk_family(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    family: Family<'_>,
) -> [ReadResult; 6] {
    let Family {
        mut characters,
        mut leaders,
        mut advisors,
        mut units,
        mut traits,
        mut experience,
    } = family;
    let result = (|| {
        let roster = ccharacter_manager::Roster::read(memory, base, game_state)?;
        let mut cursor = roster.cursor();
        loop {
            if !characters.active()
                && !leaders.active()
                && !advisors.active()
                && !units.active()
                && !traits.active()
                && !experience.active()
            {
                break;
            }
            let Some(view) = cursor.next(
                memory,
                [
                    characters.active(),
                    leaders.active(),
                    units.active() || traits.active() || experience.active(),
                    advisors.active(),
                ],
            )?
            else {
                break;
            };
            let (character_pointer, character_id) = view.reference();
            match view.entry() {
                Ok(character) => {
                    characters.append(
                        |rows, bytes| ccharacter::walk_entry(&view, character, rows, bytes).into(),
                        |row, offset| {
                            relocate(&mut row.name_start, offset)
                                && relocate(&mut row.token_start, offset)
                        },
                    );
                    leaders.append(
                        |rows, _| {
                            ReadResult::rows(ccountry_leader::walk_entry(
                                &view, base, character, rows,
                            ))
                        },
                        |_, _| true,
                    );
                    advisors.append(
                        |rows, bytes| {
                            cadvisor::walk_entry(&view, base, character, rows, bytes).into()
                        },
                        |row, offset| {
                            relocate(&mut row.key_start, offset)
                                && relocate(&mut row.idea_start, offset)
                                && relocate(&mut row.slot_start, offset)
                        },
                    );
                }
                Err(reason) => {
                    characters.fail(reason);
                    leaders.fail(reason);
                    advisors.fail(reason);
                }
            }
            if !units.active() && !traits.active() && !experience.active() {
                continue;
            }
            let pointer = character_pointer
                .checked_add(ccharacter::UNIT_LEADER)
                .and_then(|address| read::<u64>(&view, address))
                .and_then(|pointer| usize::try_from(pointer).ok());
            let Some(pointer) = pointer else {
                units.fail("character unit leader pointer unreadable");
                traits.fail("character unit leader pointer unreadable");
                experience.fail("character unit leader pointer unreadable");
                continue;
            };
            if pointer == 0 {
                continue;
            }
            let previous_len = units.len;
            units.append(
                |rows, bytes| read_unit(memory, base, pointer, character_id, rows, bytes),
                |row, offset| {
                    relocate(&mut row.text_a_start, offset)
                        && relocate(&mut row.text_b_start, offset)
                },
            );
            // Reuse the authenticated scalar header on the normal path. The
            // trait vectors retain independent availability when scalar capacity
            // or a skill/text fault requires reading only the parent's identity.
            let identity = if traits.active() || experience.active() {
                Some(if units.len > previous_len {
                    Ok(units.rows[previous_len].native.id)
                } else {
                    unit_identity(memory, pointer, character_id)
                })
            } else {
                None
            };
            match identity {
                Some(Ok(id)) => {
                    traits.append(
                        |rows, bytes| {
                            cunit_leader::walk_traits(memory, base, pointer, id, rows, bytes).into()
                        },
                        |row, offset| relocate(&mut row.token_start, offset),
                    );
                    experience.append(
                        |rows, bytes| {
                            cunit_leader::walk_trait_experience(
                                memory, base, pointer, id, rows, bytes,
                            )
                            .into()
                        },
                        |row, offset| relocate(&mut row.trait_row.token_start, offset),
                    );
                }
                Some(Err(reason)) => {
                    traits.fail(reason);
                    experience.fail(reason);
                }
                None => {}
            }
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result {
        characters.fail(reason);
        leaders.fail(reason);
        advisors.fail(reason);
        units.fail(reason);
        traits.fail(reason);
        experience.fail(reason);
    }
    [
        characters.result(),
        leaders.result(),
        advisors.result(),
        units.result(),
        traits.result(),
        experience.result(),
    ]
}

fn unit_identity(
    memory: &impl Memory,
    pointer: usize,
    character: u32,
) -> Result<u32, &'static str> {
    let mut reference = [0; 8];
    let mut parent = [0; 8];
    let reference_address = pointer
        .checked_add(cunit_leader::TYPE)
        .ok_or("unit leader identity address overflow")?;
    let parent_address = pointer
        .checked_add(cunit_leader::CHARACTER_TYPE)
        .ok_or("unit leader parent address overflow")?;
    if memory.copy_scattered(
        [reference_address, parent_address],
        &mut [&mut reference, &mut parent],
    ) != 2
    {
        return Err("unit leader identity or parent unreadable");
    }
    if u32::from_le_bytes(reference[..4].try_into().unwrap()) != cunit_leader::REFERENCE_TYPE
        || u32::from_le_bytes(parent[..4].try_into().unwrap()) != ccharacter::REFERENCE_TYPE
        || u32::from_le_bytes(parent[4..].try_into().unwrap()) != character
    {
        return Err("unit leader native reference or character backlink mismatch");
    }
    Ok(u32::from_le_bytes(reference[4..].try_into().unwrap()))
}

fn read_unit(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    character: u32,
    output: &mut [LeaderRow],
    bytes: &mut [u8],
) -> ReadResult {
    let Some(output) = output.first_mut() else {
        return ReadResult::rows(Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        });
    };
    let native = match cunit_leader::read(memory, base, pointer, character) {
        Ok(row) => row,
        Err(reason) => return ReadResult::rows(Outcome::Unavailable { reason }),
    };
    let text = cunit_leader::read_text(memory, pointer, bytes);
    if matches!(text.outcome, Outcome::Rows { len: 1 }) {
        *output = LeaderRow {
            native,
            // Native +20 is saved as name; +40 is a separate unsaved string.
            text_a_start: text.token_start,
            text_a_len: text.token_len,
            text_b_start: text.name_start,
            text_b_len: text.name_len,
        };
    }
    ReadResult {
        outcome: text.outcome,
        bytes_len: text.bytes_len,
        required_bytes: text.required_bytes,
    }
}

fn walk_abilities(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut [active_ability_record::Row],
    bytes: &mut [u8],
) -> ReadResult {
    let mut output = Batch::new(rows, bytes, true);
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let address = country
                .pointer
                .checked_add(ccountry::ACTIVE_ABILITIES)
                .ok_or("country active ability array address overflow")?;
            output.append(
                |rows, bytes| {
                    active_ability_record::walk(memory, base, country.id, address, rows, bytes)
                        .into()
                },
                |row, offset| relocate(&mut row.token_start, offset),
            );
            if !output.active() {
                break;
            }
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result {
        output.fail(reason);
    }
    output.result()
}

fn view<S: TableSchema, R>(
    buffer: &Buffer<R>,
    valid_row: impl Fn(&R, &[u8]) -> bool,
    convert: impl Fn(&R, &[u8]) -> S::Row,
) -> TableView {
    if buffer.rows.checked && buffer.rows.mismatches != 0 {
        return unavailable("character table changed during double read");
    }
    let len = match buffer.rows.outcome {
        Outcome::Rows { len } | Outcome::Truncated { len, .. } => len,
        Outcome::Unavailable { reason } => return unavailable(reason),
    };
    let Some(rows) = buffer.rows.rows.get(..len) else {
        return unavailable("character row count exceeds allocated buffer");
    };
    let Some(bytes) = buffer.bytes.first.get(..buffer.bytes.len) else {
        return unavailable("character byte count exceeds allocated arena");
    };
    if rows.iter().any(|row| !valid_row(row, bytes)) {
        return unavailable("character string lies outside the captured byte arena");
    }
    table::<S, _>(buffer.rows.outcome, rows, |row| convert(row, bytes))
}

fn valid(bytes: &[u8], start: usize, len: usize) -> bool {
    start
        .checked_add(len)
        .is_some_and(|end| bytes.get(start..end).is_some())
}

/// Used only after `view` validates every range for this table.
fn owned(bytes: &[u8], start: usize, len: usize) -> Vec<u8> {
    bytes[start..start + len].to_vec()
}

fn leader_reference(id: u32) -> u64 {
    (u64::from(id) << 32) | u64::from(cunit_leader::REFERENCE_TYPE)
}

fn leader_row(row: &LeaderRow, bytes: &[u8]) -> UnitLeaderRow {
    let native = row.native;
    UnitLeaderRow {
        reference_type: native.reference_type,
        id: native.id,
        character_type: native.character_type,
        character_id: native.character_id,
        category: native.category,
        experience: native.experience,
        kind: native.kind,
        skill_status: native.skill_status,
        skill_valid: native.skill_valid,
        skill_level: native.skill_level,
        skill_required_experience: native.skill_required_experience,
        skill_category: native.skill_category,
        components: native.components,
        inherited_text_a: owned(bytes, row.text_a_start, row.text_a_len),
        inherited_text_b: owned(bytes, row.text_b_start, row.text_b_len),
    }
}

fn ability_row(row: &active_ability_record::Row, bytes: &[u8]) -> ActiveAbilityRow {
    ActiveAbilityRow {
        country: row.country,
        ordinal: row.ordinal,
        scope_country: row.scope_country,
        token: owned(bytes, row.token_start, row.token_len),
        caster: row.caster,
        strategic_region: row.strategic_region,
        division_count: row.division_count,
        activation_hour: row.activation_hour,
        phase: row.phase,
        allocate_cost: row.allocate_cost,
        allocated_command_power: row.allocated_command_power,
        duration_hours: row.duration_hours,
        cooldown_hours: row.cooldown_hours,
        target_kind: row.target_kind,
    }
}

// ===========================================================================
// Tier-4 slot-0 raw-copy split (Characters, leader/advisor roles, unit
// leaders, traits and trait experience; ActiveAbilities stays hourly).
//
// The game thread copies raw bytes only: the manager census, the per-entry
// view spans, heap string payloads by header length, role-map nodes with
// batched gathers, and unit/trait/experience nesting. No row is decoded on
// the game thread. The single FIFO decode worker replays the existing
// `walk_family` against the recorded spans (`RecordedMemory`), so typed
// rows, ordinals, skips, truncation counts and failure reasons are identical
// by construction: every read the walk performs on its success paths is in
// the log, and every recorded failure reproduces the walk's own reason at
// the same read site.
//
// Read-set rules the mirror follows (log order is irrelevant; the log is a
// set keyed by exact address and length):
// - reads served locally by `ccharacter::View` need no log entry beyond the
//   seven view spans themselves;
// - pooled capacities are ignored while copying (a superset is recorded and
//   the replay truncates identically against the real pooled buffers);
// - on any failed span the mirror aborts that sub-path exactly where the
//   typed walk would fail it, and failed batch flags (`Dead`) skip later
//   sub-copies exactly where `Batch::append` would skip the typed walk;
// - intra-copy tearing is the documented double-read limitation: the mirror
//   reads each site once and shares the parsed values, so the log is always
//   self-consistent.
// Conservative rule (as the research pilot): any live failure attributed to
// a table blocks that table's check even when both copies agree, so
// same-hour tearing never advances the windows. A readable-but-invalid
// refresh (deterministic fault, both copies equal) still counts its check
// while emitting the identical Unavailable outcome.
// ===========================================================================

/// Fault-attribution bits, one per family table in `TABLES[0..6]` order.
const MASK_CHARACTERS: u8 = 0x01;
const MASK_LEADERS: u8 = 0x02;
const MASK_ADVISORS: u8 = 0x04;
const MASK_UNITS: u8 = 0x08;
const MASK_TRAITS: u8 = 0x10;
const MASK_EXPERIENCE: u8 = 0x20;
const MASK_ALL: u8 = 0x3f;
const MASK_COUNTRY: u8 = MASK_CHARACTERS | MASK_LEADERS | MASK_ADVISORS;
const MASK_UNIT_LINK: u8 = MASK_UNITS | MASK_TRAITS | MASK_EXPERIENCE;
/// Identity-fallback spans feed only the trait vectors: the units outcome is
/// already decided when the typed fallback runs. The overlapping identity
/// span itself already attributes units through `MASK_UNIT_LINK`.
const MASK_TRAIT_FALLBACK: u8 = MASK_TRAITS | MASK_EXPERIENCE;

/// View span offsets, lengths and fault masks in `View::read` order:
/// identity, country, name token id, name/token headers, leader map head,
/// unit leader pointer, advisor map head.
const VIEW_OFFSETS: [usize; 7] = [
    0,
    ccharacter::COUNTRY,
    ccharacter::NAME_TOKEN_ID,
    ccharacter::NAME,
    ccharacter::LEADER_MAP_HEAD,
    ccharacter::UNIT_LEADER,
    ccharacter::ADVISOR_MAP_HEAD,
];
const VIEW_LENS: [usize; 7] = [16, 4, 4, 64, 16, 8, 16];
const VIEW_MASKS: [u8; 7] = [
    MASK_ALL,
    MASK_COUNTRY,
    MASK_CHARACTERS,
    MASK_CHARACTERS,
    MASK_LEADERS,
    MASK_UNIT_LINK,
    MASK_ADVISORS,
];

/// Roster pointer pages are read in 32-entry chunks, exactly like the typed
/// cursor, so the replay requests the identical page spans.
const ROSTER_PAGE: u32 = 32;

/// One recorded guarded read: exact address and length, success flag, table
/// mask, and byte-pool offset for successful spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub addr: usize,
    pub len: u32,
    pub ok: bool,
    pub mask: u8,
    pub start: u32,
}

/// One refresh of raw character payload. Flat pools keep every span
/// fixed-size so `clear` recycles capacities and the steady state allocates
/// on neither thread; growth happens on the decode side only (see
/// `crate::decode::recycle_characters_job`).
#[derive(Debug)]
pub(crate) struct CharactersJob {
    pub spans: Vec<Span>,
    pub bytes: Vec<u8>,
    pub game_state: usize,
    pub fault: [bool; 6],
    pub raw_bytes: [u64; 6],
    pub overflowed: bool,
}

impl CharactersJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is about 13k characters with a few
            // hundred thousand spans and under 10 MB of payload bytes; both
            // pools start above that so steady state never overflows.
            spans: Vec::with_capacity(1 << 19),
            bytes: Vec::with_capacity(24 << 20),
            game_state: 0,
            fault: [false; 6],
            raw_bytes: [0; 6],
            overflowed: false,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.spans.clear();
        self.bytes.clear();
        self.game_state = 0;
        self.fault = [false; 6];
        self.raw_bytes = [0; 6];
        self.overflowed = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.overflowed
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed refreshes fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.spans.reserve_exact(self.spans.capacity().max(1));
        self.bytes.reserve_exact(self.bytes.capacity().max(1));
    }

    /// Guarded copy of one span into the pool. Returns the pool offset of the
    /// copied bytes on success and records failures with the affected-table
    /// mask. Empty spans read nothing (matching `Memory`), null or
    /// overflowing addresses fail without touching memory. On pool overflow
    /// sets `overflowed` and returns `None`; the caller aborts the copy and
    /// the frame falls back to the typed walk.
    fn take(
        &mut self,
        memory: &impl Memory,
        addr: usize,
        len: usize,
        mask: u8,
    ) -> Option<usize> {
        if self.overflowed {
            return None;
        }
        if len == 0 {
            return Some(self.bytes.len());
        }
        let Ok(len32) = u32::try_from(len) else {
            self.overflowed = true;
            return None;
        };
        if addr == 0 || addr.checked_add(len).is_none() {
            self.note_fail(addr, len32, mask);
            return None;
        }
        if self.spans.len() == self.spans.capacity()
            || self
                .bytes
                .len()
                .checked_add(len)
                .is_none_or(|end| end > self.bytes.capacity())
        {
            self.overflowed = true;
            return None;
        }
        let start = self.bytes.len();
        self.bytes.resize(start + len, 0);
        if !memory.copy(addr, &mut self.bytes[start..start + len]) {
            self.bytes.truncate(start);
            self.note_fail(addr, len32, mask);
            return None;
        }
        self.spans.push(Span {
            addr,
            len: len32,
            ok: true,
            mask,
            start: start as u32,
        });
        self.raw_bytes[lowest_table(mask)] += len as u64;
        Some(start)
    }

    fn note_fail(&mut self, addr: usize, len32: u32, mask: u8) {
        if self.spans.len() == self.spans.capacity() {
            self.overflowed = true;
            return;
        }
        self.spans.push(Span {
            addr,
            len: len32,
            ok: false,
            mask,
            start: 0,
        });
        for (slot, fault) in self.fault.iter_mut().enumerate() {
            if mask & (1 << slot) != 0 {
                *fault = true;
            }
        }
    }

    fn logs_equal(&self, other: &Self) -> bool {
        self.spans == other.spans && self.bytes == other.bytes
    }

    /// Take failure from an overflowing pool aborts the whole copy (typed
    /// fallback); a live read failure kills only the enclosing batch.
    fn fate(&self) -> TreeFate {
        if self.overflowed {
            TreeFate::Abort
        } else {
            TreeFate::Dead
        }
    }
}

/// Attribute shared spans to the anchor table; private spans carry a single
/// bit already. Only `take` calls this, so byte accounting never allocates.
fn lowest_table(mask: u8) -> usize {
    for slot in 0..6 {
        if mask & (1 << slot) != 0 {
            return slot;
        }
    }
    0
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CharactersCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Whole-log raw equality gated by live-fault conservatism, in
    /// [characters, leaders, advisors, units, traits, experience] order.
    pub matched: [bool; 6],
    pub copy_ticks: [u64; 6],
    pub raw_bytes: [u64; 6],
}

/// Game-thread copy parameters, bundled to keep the refresh entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CharactersCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; TABLE_COUNT],
}

/// Decode-side address space: serves exactly the recorded spans and fails
/// every other read, so the replayed walk observes the identical bytes and
/// faults the copy observed. Empty reads succeed and null or overflowing
/// addresses fail without a lookup, matching `Memory`.
struct RecordedMemory<'a> {
    map: HashMap<(usize, u32), usize>,
    bytes: &'a [u8],
}

impl<'a> RecordedMemory<'a> {
    fn build(job: &'a CharactersJob) -> Self {
        let mut map = HashMap::with_capacity(job.spans.len());
        for span in &job.spans {
            if span.ok {
                map.insert((span.addr, span.len), span.start as usize);
            }
        }
        Self {
            map,
            bytes: &job.bytes,
        }
    }
}

impl Memory for RecordedMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        if address == 0 || address.checked_add(destination.len()).is_none() {
            return false;
        }
        let Ok(len) = u32::try_from(destination.len()) else {
            return false;
        };
        match self.map.get(&(address, len)) {
            Some(&start) => {
                let Some(source) = self.bytes.get(start..start + destination.len()) else {
                    return false;
                };
                destination.copy_from_slice(source);
                true
            }
            None => false,
        }
    }
}

/// Per-batch failure flags mirroring `Batch::active`: once a typed batch
/// fails, its later appends are skipped without reads, and so are the
/// mirror's later sub-copies.
#[derive(Debug, Default)]
struct Dead {
    characters: bool,
    leaders: bool,
    advisors: bool,
    units: bool,
    traits: bool,
    experience: bool,
}

/// Seven copied view spans in `View::read` order for local parsing.
struct ViewCopy {
    bytes: [[u8; 64]; 7],
}

/// Mirror of `View::read`'s scatter plus single-resume sequence with no
/// decoding. Returns `None` only when the identity span fails, which aborts
/// the whole copy exactly like the typed cursor error.
fn copy_view(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Option<ViewCopy> {
    let mut view = ViewCopy {
        bytes: [[0; 64]; 7],
    };
    let mut completed = 0;
    while completed < 7 {
        let addr = pointer.checked_add(VIEW_OFFSETS[completed]).unwrap_or(0);
        let len = VIEW_LENS[completed];
        match job.take(memory, addr, len, VIEW_MASKS[completed]) {
            Some(start) => {
                view.bytes[completed][..len].copy_from_slice(&job.bytes[start..start + len]);
                completed += 1;
            }
            None => break,
        }
    }
    if completed == 0 {
        return None;
    }
    // Resume only later independent fields, each at most once; the failed
    // field is never retried.
    for index in completed + 1..7 {
        let addr = pointer.checked_add(VIEW_OFFSETS[index]).unwrap_or(0);
        let len = VIEW_LENS[index];
        if let Some(start) = job.take(memory, addr, len, VIEW_MASKS[index]) {
            view.bytes[index][..len].copy_from_slice(&job.bytes[start..start + len]);
        }
    }
    // Local vtable and reference-type authentication, checked after the
    // scatter exactly like the typed view.
    let expected = base.checked_add(ccharacter::VTABLE_RVA)? as u64;
    if u64::from_le_bytes(view.bytes[0][..8].try_into().unwrap()) != expected
        || u32::from_le_bytes(view.bytes[0][ccharacter::TYPE..ccharacter::TYPE + 4].try_into().unwrap())
            != ccharacter::REFERENCE_TYPE
    {
        return None;
    }
    Some(view)
}

/// Parsed native string header shared by both string domains: payload length,
/// payload source address and inline flag. `None` means the typed walk fails
/// the enclosing row with its own header reason from the same header bytes.
struct Lit {
    len: usize,
    source: usize,
    inline: bool,
}

/// Mirror of `ccharacter::Literal::from_header` (character name and token).
fn parse_character_literal(header: &[u8], header_addr: usize) -> Option<Lit> {
    let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if capacity < msvc_string::INLINE_CAPACITY || len > capacity {
        return None;
    }
    let len = usize::try_from(len).ok()?;
    let inline = capacity == msvc_string::INLINE_CAPACITY;
    let source = if inline {
        header_addr
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&pointer| pointer != 0)?
    };
    if inline && header[len] != 0 {
        return None;
    }
    Some(Lit {
        len,
        source,
        inline,
    })
}

/// Mirror of `cunit_leader`'s string header validation: heap strings read
/// their terminator (and payload when nonempty) from live memory, inline
/// strings are served from the already-copied header.
fn copy_unit_literal(
    job: &mut CharactersJob,
    memory: &impl Memory,
    header: &[u8],
    mask: u8,
) -> Option<usize> {
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return None;
    }
    let len = usize::try_from(len).ok()?;
    if capacity == msvc_string::INLINE_CAPACITY {
        if header[len] != 0 {
            return None;
        }
        return Some(len);
    }
    let source = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .ok()
        .filter(|&pointer| pointer != 0)?;
    let terminator = source.checked_add(len)?;
    job.take(memory, terminator, 1, mask)?;
    if len != 0 {
        job.take(memory, source, len, mask)?;
    }
    Some(len)
}

/// Heap payload and terminator spans for one parsed character-domain string.
/// Inline strings need no live reads beyond the copied header.
fn copy_character_literal_bytes(
    job: &mut CharactersJob,
    memory: &impl Memory,
    literal: &Lit,
    mask: u8,
) -> bool {
    if literal.inline {
        return true;
    }
    let Some(terminator) = literal.source.checked_add(literal.len) else {
        return false;
    };
    if job.take(memory, terminator, 1, mask).is_none() {
        return false;
    }
    if literal.len != 0 && job.take(memory, literal.source, literal.len, mask).is_none() {
        return false;
    }
    true
}

/// Traversal state mirroring `ccharacter::RoleTree`'s read set: the mirror
/// issues the identical sentinel, link and node reads in the identical
/// successor order, parsing link values locally to choose branches.
struct TreeWalk {
    head: usize,
    next: usize,
    remaining: u32,
    count: u32,
}

/// Abort the whole copy (`false`) or kill only this map's batch (`true`).
enum TreeFate {
    Abort,
    Dead,
}

/// Mirror of `RoleTree::read`: sentinel plus size/coherence parses.
fn tree_read(
    job: &mut CharactersJob,
    memory: &impl Memory,
    head_bytes: &[u8],
    mask: u8,
) -> Result<TreeWalk, TreeFate> {
    let head = usize::try_from(u64::from_le_bytes(head_bytes[..8].try_into().unwrap()))
        .ok()
        .filter(|&pointer| pointer != 0)
        .ok_or_else(|| job.fate())?;
    let count = u32::try_from(u64::from_le_bytes(head_bytes[8..16].try_into().unwrap()))
        .map_err(|_| job.fate())?;
    let start = job
        .take(memory, head, 26, mask)
        .ok_or_else(|| job.fate())?;
    let mut sentinel = [0u8; 26];
    sentinel.copy_from_slice(&job.bytes[start..start + 26]);
    if sentinel[msvc_tree_node::NIL] != 1 {
        return Err(TreeFate::Dead);
    }
    let link = |offset: usize| {
        usize::try_from(u64::from_le_bytes(
            sentinel[offset..offset + 8].try_into().unwrap(),
        ))
        .ok()
        .filter(|&pointer| pointer != 0)
        .ok_or(TreeFate::Dead)
    };
    let next = link(msvc_tree_node::LEFT)?;
    if (count == 0) != (next == head) {
        return Err(TreeFate::Dead);
    }
    if count == 0 && (link(msvc_tree_node::PARENT)? != head || link(msvc_tree_node::RIGHT)? != head)
    {
        return Err(TreeFate::Dead);
    }
    Ok(TreeWalk {
        head,
        next,
        remaining: count,
        count,
    })
}

fn tree_link(
    job: &mut CharactersJob,
    memory: &impl Memory,
    node: usize,
    offset: usize,
    mask: u8,
) -> Result<usize, TreeFate> {
    let addr = node.checked_add(offset).ok_or(TreeFate::Dead)?;
    let start = job
        .take(memory, addr, 8, mask)
        .ok_or_else(|| job.fate())?;
    usize::try_from(u64::from_le_bytes(
        job.bytes[start..start + 8].try_into().unwrap(),
    ))
    .ok()
    .filter(|&pointer| pointer != 0)
    .ok_or(TreeFate::Dead)
}

/// Mirror of `RoleTree::next`: the identical bounded successor sequence,
/// returning native node addresses with no decoding.
fn tree_next(
    job: &mut CharactersJob,
    memory: &impl Memory,
    walk: &mut TreeWalk,
    mask: u8,
) -> Result<Option<usize>, TreeFate> {
    if walk.remaining == 0 {
        if walk.next != walk.head {
            return Err(TreeFate::Dead);
        }
        return Ok(None);
    }
    let current = walk.next;
    if current == walk.head {
        return Err(TreeFate::Dead);
    }
    let nil_addr = current.checked_add(msvc_tree_node::NIL).ok_or(TreeFate::Dead)?;
    let nil = job
        .take(memory, nil_addr, 1, mask)
        .ok_or_else(|| job.fate())?;
    if job.bytes[nil] != 0 {
        return Err(TreeFate::Dead);
    }
    let right = tree_link(job, memory, current, msvc_tree_node::RIGHT, mask)?;
    let mut next = right;
    let mut steps = 0_u64;
    if right != walk.head {
        loop {
            steps += 1;
            if steps > u64::from(walk.count) {
                return Err(TreeFate::Dead);
            }
            let nil_addr = next.checked_add(msvc_tree_node::NIL).ok_or(TreeFate::Dead)?;
            let nil = job
                .take(memory, nil_addr, 1, mask)
                .ok_or_else(|| job.fate())?;
            if job.bytes[nil] != 0 {
                return Err(TreeFate::Dead);
            }
            let left = tree_link(job, memory, next, msvc_tree_node::LEFT, mask)?;
            if left == walk.head {
                break;
            }
            if tree_link(job, memory, left, msvc_tree_node::PARENT, mask)? != next {
                return Err(TreeFate::Dead);
            }
            next = left;
        }
        if tree_link(job, memory, right, msvc_tree_node::PARENT, mask)? != current {
            return Err(TreeFate::Dead);
        }
    } else {
        let mut child = current;
        next = tree_link(job, memory, child, msvc_tree_node::PARENT, mask)?;
        while next != walk.head {
            steps += 1;
            if steps > u64::from(walk.count) {
                return Err(TreeFate::Dead);
            }
            if tree_link(job, memory, next, msvc_tree_node::LEFT, mask)? == child {
                break;
            }
            if tree_link(job, memory, next, msvc_tree_node::RIGHT, mask)? != child {
                return Err(TreeFate::Dead);
            }
            child = next;
            next = tree_link(job, memory, child, msvc_tree_node::PARENT, mask)?;
        }
    }
    walk.next = next;
    walk.remaining -= 1;
    Ok(Some(current))
}

/// One mirrored span read: `None` kills the enclosing batch. The caller maps
/// `None` to a whole-copy abort when the job overflowed, matching the staged
/// fallback rule.
fn take_or_kill(
    job: &mut CharactersJob,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u8,
    dead: &mut bool,
) -> Option<usize> {
    match addr.and_then(|addr| job.take(memory, addr, len, mask)) {
        Some(start) => Some(start),
        None => {
            *dead = true;
            None
        }
    }
}

fn pool_u64(job: &CharactersJob, start: usize) -> u64 {
    u64::from_le_bytes(job.bytes[start..start + 8].try_into().unwrap())
}

fn pool_u32(job: &CharactersJob, start: usize) -> u32 {
    u32::from_le_bytes(job.bytes[start..start + 4].try_into().unwrap())
}

/// Mirror of `ccharacter::walk_entry`'s string reads: header parses are local
/// (the headers ride in the view span), heap payloads and terminators are
/// copied by length. Any failure kills only the characters batch.
fn copy_character_text(
    job: &mut CharactersJob,
    memory: &impl Memory,
    view: &ViewCopy,
    pointer: usize,
    dead: &mut Dead,
) {
    if dead.characters {
        return;
    }
    let name_addr = pointer.checked_add(ccharacter::NAME).unwrap_or(0);
    let token_addr = pointer.checked_add(ccharacter::TOKEN).unwrap_or(0);
    let name = parse_character_literal(&view.bytes[3][..32], name_addr);
    let token = parse_character_literal(&view.bytes[3][32..64], token_addr);
    let (Some(name), Some(token)) = (name, token) else {
        dead.characters = true;
        return;
    };
    if !copy_character_literal_bytes(job, memory, &name, MASK_CHARACTERS)
        || !copy_character_literal_bytes(job, memory, &token, MASK_CHARACTERS)
    {
        dead.characters = true;
    }
}

/// Mirror of `ccountry_leader::walk_entry`'s read set: the role-map traversal
/// plus one ideology/role-header pair per node. Order, duplicate and auth
/// checks are local parses; any failure kills only the leaders batch.
fn copy_leaders(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    view: &ViewCopy,
    pointer: usize,
    dead: &mut Dead,
) -> bool {
    if dead.leaders {
        return true;
    }
    let expected = match base.checked_add(ccountry_leader::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => {
            dead.leaders = true;
            return true;
        }
    };
    let mut walk = match tree_read(job, memory, &view.bytes[4][..16], MASK_LEADERS) {
        Ok(walk) => walk,
        Err(TreeFate::Abort) => return false,
        Err(TreeFate::Dead) => {
            dead.leaders = true;
            return true;
        }
    };
    let mut previous = None;
    loop {
        let node = match tree_next(job, memory, &mut walk, MASK_LEADERS) {
            Ok(node) => node,
            Err(TreeFate::Abort) => return false,
            Err(TreeFate::Dead) => {
                dead.leaders = true;
                return true;
            }
        };
        let Some(node) = node else { break };
        let Some(ideo) = take_or_kill(
            job,
            memory,
            node.checked_add(character_leader_node::IDEOLOGY),
            16,
            MASK_LEADERS,
            &mut dead.leaders,
        ) else {
            return !job.overflowed;
        };
        let ideology = i32::from_le_bytes(job.bytes[ideo..ideo + 4].try_into().unwrap());
        if previous.is_some_and(|key| key >= ideology) {
            dead.leaders = true;
            return true;
        }
        previous = Some(ideology);
        let role = usize::try_from(pool_u64(job, ideo + 8)).ok().filter(|&role| role != 0);
        let Some(header) =
            take_or_kill(job, memory, role, 16, MASK_LEADERS, &mut dead.leaders)
        else {
            return !job.overflowed;
        };
        if pool_u64(job, header) != expected
            || pool_u64(job, header + ccountry_leader::CHARACTER) != pointer as u64
        {
            dead.leaders = true;
            return true;
        }
    }
    true
}

/// Mirror of `cadvisor::walk_entry`'s read set: the role-map traversal plus
/// node, role and key/idea/slot string spans per node. Hash order and auth
/// checks are local parses; any failure kills only the advisors batch.
fn copy_advisors(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    view: &ViewCopy,
    pointer: usize,
    dead: &mut Dead,
) -> bool {
    if dead.advisors {
        return true;
    }
    let expected = match base.checked_add(cadvisor::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => {
            dead.advisors = true;
            return true;
        }
    };
    let mut walk = match tree_read(job, memory, &view.bytes[6][..16], MASK_ADVISORS) {
        Ok(walk) => walk,
        Err(TreeFate::Abort) => return false,
        Err(TreeFate::Dead) => {
            dead.advisors = true;
            return true;
        }
    };
    let mut previous_hash = None;
    loop {
        let node = match tree_next(job, memory, &mut walk, MASK_ADVISORS) {
            Ok(node) => node,
            Err(TreeFate::Abort) => return false,
            Err(TreeFate::Dead) => {
                dead.advisors = true;
                return true;
            }
        };
        let Some(node) = node else { break };
        let Some(role_ptr) = take_or_kill(
            job,
            memory,
            node.checked_add(character_advisor_node::ADVISOR),
            8,
            MASK_ADVISORS,
            &mut dead.advisors,
        ) else {
            return !job.overflowed;
        };
        let Some(key_header) = take_or_kill(
            job,
            memory,
            node.checked_add(character_advisor_node::KEY),
            32,
            MASK_ADVISORS,
            &mut dead.advisors,
        ) else {
            return !job.overflowed;
        };
        let Some(hash) = take_or_kill(
            job,
            memory,
            node.checked_add(character_advisor_node::KEY_HASH),
            4,
            MASK_ADVISORS,
            &mut dead.advisors,
        ) else {
            return !job.overflowed;
        };
        let role = usize::try_from(pool_u64(job, role_ptr))
            .ok()
            .filter(|&role| role != 0);
        if role.is_none() {
            dead.advisors = true;
            return true;
        }
        let role = role.unwrap();
        let Some(vtable) = take_or_kill(job, memory, Some(role), 8, MASK_ADVISORS, &mut dead.advisors)
        else {
            return !job.overflowed;
        };
        let Some(parent) = take_or_kill(
            job,
            memory,
            role.checked_add(cadvisor::CHARACTER),
            8,
            MASK_ADVISORS,
            &mut dead.advisors,
        ) else {
            return !job.overflowed;
        };
        let Some(idea_header) = take_or_kill(
            job,
            memory,
            role.checked_add(cadvisor::IDEA_TOKEN),
            32,
            MASK_ADVISORS,
            &mut dead.advisors,
        ) else {
            return !job.overflowed;
        };
        let Some(slot_header) = take_or_kill(
            job,
            memory,
            role.checked_add(cadvisor::SLOT),
            32,
            MASK_ADVISORS,
            &mut dead.advisors,
        ) else {
            return !job.overflowed;
        };
        if pool_u64(job, vtable) != expected
            || pool_u64(job, parent) != pointer as u64
        {
            dead.advisors = true;
            return true;
        }
        let hash = pool_u32(job, hash);
        if previous_hash.is_some_and(|previous| previous > hash)
            || previous_hash == Some(hash)
        {
            dead.advisors = true;
            return true;
        }
        previous_hash = Some(hash);
        let key_addr = node
            .checked_add(character_advisor_node::KEY)
            .unwrap_or(0);
        let idea_addr = role.checked_add(cadvisor::IDEA_TOKEN).unwrap_or(0);
        let slot_addr = role.checked_add(cadvisor::SLOT).unwrap_or(0);
        let literal = |start: usize, addr: usize| {
            let mut header = [0u8; 32];
            header.copy_from_slice(&job.bytes[start..start + 32]);
            parse_character_literal(&header, addr)
        };
        let (Some(key), Some(idea), Some(slot)) = (
            literal(key_header, key_addr),
            literal(idea_header, idea_addr),
            literal(slot_header, slot_addr),
        ) else {
            dead.advisors = true;
            return true;
        };
        if !copy_character_literal_bytes(job, memory, &key, MASK_ADVISORS)
            || !copy_character_literal_bytes(job, memory, &idea, MASK_ADVISORS)
            || !copy_character_literal_bytes(job, memory, &slot, MASK_ADVISORS)
        {
            dead.advisors = true;
            return true;
        }
    }
    true
}

/// Authenticate a unit leader header locally, mirroring `cunit_leader`'s
/// identity read. Returns the kind and leader id for the copy below.
fn parse_unit_identity(
    job: &CharactersJob,
    base: usize,
    start: usize,
) -> Option<(u8, u32)> {
    let vtable = pool_u64(job, start);
    let relative = usize::try_from(vtable).ok()?.checked_sub(base)?;
    let kind = match relative {
        cunit_leader::VTABLE_RVA => cunit_leader::KIND_BASE,
        carmy_leader::VTABLE_RVA => cunit_leader::KIND_ARMY,
        cnavy_leader::VTABLE_RVA => cunit_leader::KIND_NAVY,
        coperative_leader::VTABLE_RVA => cunit_leader::KIND_OPERATIVE,
        _ => return None,
    };
    if pool_u32(job, start + cunit_leader::TYPE) != cunit_leader::REFERENCE_TYPE {
        return None;
    }
    Some((kind, pool_u32(job, start + cunit_leader::ID)))
}

/// Mirror of `read_unit` plus `cunit_leader::read`/`read_text`: identity,
/// state, skill, kind components and both text strings. The identity-fallback
/// spans are always recorded so the trait replay finds them when the scalar
/// read fails. Any failure kills only the units batch.
fn copy_unit(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    view: &ViewCopy,
    _pointer: usize,
    dead: &mut Dead,
) -> bool {
    let unit = usize::try_from(u64::from_le_bytes(view.bytes[5][..8].try_into().unwrap()))
        .ok()
        .filter(|&unit| unit != 0);
    let Some(unit) = unit else {
        // Null role pointers are handled by the caller: no rows, no reads.
        return true;
    };
    if dead.units && dead.traits && dead.experience {
        // The typed walk `continue`s without reads once all three batches
        // are dead, and so does the mirror.
        return true;
    }
    // The replay calls the identity fallback while its traits or experience
    // batch is still alive at this character, which is exactly the entry
    // state here: later kills in this function happen after the replay's
    // call point, so they must not close this gate.
    let need_fallback = !dead.traits || !dead.experience;
    // Identity feeds the scalar walk and both trait vectors. Its failure
    // kills all three batches because every path re-reads it.
    let identity = take_or_kill(
        job,
        memory,
        Some(unit),
        16,
        MASK_UNIT_LINK,
        &mut dead.units,
    );
    if identity.is_none() {
        dead.traits = true;
        dead.experience = true;
    }
    let parsed = identity.and_then(|start| parse_unit_identity(job, base, start));
    if identity.is_some() && parsed.is_none() {
        dead.units = true;
        dead.traits = true;
        dead.experience = true;
    }
    let kind = parsed.map(|(kind, _)| kind);
    if !dead.units {
        // State scatter in `cunit_leader::read` order: character backlink,
        // category, experience, skill pointer.
        let mut state = [0usize; 4];
        let mut states_ok = true;
        for (slot, (offset, len)) in [
            (cunit_leader::CHARACTER_TYPE, 8),
            (cunit_leader::CATEGORY, 4),
            (cunit_leader::EXPERIENCE, 8),
            (cunit_leader::SKILL, 8),
        ]
        .iter()
        .enumerate()
        {
            match take_or_kill(
                job,
                memory,
                unit.checked_add(*offset),
                *len,
                MASK_UNITS,
                &mut dead.units,
            ) {
                Some(start) => state[slot] = start,
                None => {
                    states_ok = false;
                    break;
                }
            }
        }
        if !job.overflowed && states_ok {
            let character_id = pool_u32(job, identity.unwrap() + cunit_leader::ID);
            if pool_u32(job, state[0]) != ccharacter::REFERENCE_TYPE
                || pool_u32(job, state[0] + 4) != character_id
            {
                dead.units = true;
            } else {
                copy_unit_skill(job, memory, base, state[3], dead);
            }
        }
        if job.overflowed {
            return false;
        }
        if !dead.units {
            if let Some(kind) = kind {
                copy_unit_components(job, memory, unit, kind, dead);
            }
            if job.overflowed {
                return false;
            }
        }
        if !dead.units {
            copy_unit_text(job, memory, unit, dead);
            if job.overflowed {
                return false;
            }
        }
    }
    // Identity-fallback spans for the trait replay. Recorded whenever the
    // replay could still call the fallback (entry gate above); a live
    // failure here kills only traits and experience, exactly like the
    // typed fallback error, and never the units outcome.
    if need_fallback {
        let mut fallback_dead = false;
        let first = take_or_kill(
            job,
            memory,
            unit.checked_add(cunit_leader::TYPE),
            8,
            MASK_TRAIT_FALLBACK,
            &mut fallback_dead,
        );
        let second = if first.is_some() {
            take_or_kill(
                job,
                memory,
                unit.checked_add(cunit_leader::CHARACTER_TYPE),
                8,
                MASK_TRAIT_FALLBACK,
                &mut fallback_dead,
            )
        } else {
            None
        };
        if job.overflowed {
            return false;
        }
        if first.is_none() || second.is_none() {
            dead.traits = true;
            dead.experience = true;
        }
    }
    true
}

/// Mirror of the skill block in `cunit_leader::read`. Kills only the units
/// batch; the vtable predicate is a local parse.
fn copy_unit_skill(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    skill_start: usize,
    dead: &mut Dead,
) {
    let skill = usize::try_from(pool_u64(job, skill_start)).ok();
    match skill {
        None => {
            dead.units = true;
        }
        Some(0) => {}
        Some(skill) => {
            let skill_expected = match base.checked_add(cunit_leader_skill::VTABLE_RVA) {
                Some(expected) => expected as u64,
                None => {
                    dead.units = true;
                    return;
                }
            };
            let vtable = take_or_kill(job, memory, Some(skill), 8, MASK_UNITS, &mut dead.units);
            let _valid = take_or_kill(
                job,
                memory,
                skill.checked_add(cunit_leader_skill::VALID),
                1,
                MASK_UNITS,
                &mut dead.units,
            );
            let _scalars = take_or_kill(
                job,
                memory,
                skill.checked_add(cunit_leader_skill::LEVEL),
                12,
                MASK_UNITS,
                &mut dead.units,
            );
            // The typed walk authenticates the vtable from the first span
            // and the remaining fields from the later spans; a completed
            // zero prefix still fails here with the vtable reason.
            if let Some(vtable) = vtable
                && pool_u64(job, vtable) != skill_expected
            {
                dead.units = true;
            }
        }
    }
}

/// Mirror of the kind-component scatter in `cunit_leader::read`.
fn copy_unit_components(
    job: &mut CharactersJob,
    memory: &impl Memory,
    unit: usize,
    kind: u8,
    dead: &mut Dead,
) {
    let components = match kind {
        cunit_leader::KIND_ARMY => Some([
            carmy_leader::ATTACK,
            carmy_leader::DEFENSE,
            carmy_leader::PLANNING,
            carmy_leader::LOGISTICS,
        ]),
        cunit_leader::KIND_NAVY => Some([
            cnavy_leader::ATTACK,
            cnavy_leader::DEFENSE,
            cnavy_leader::MANEUVERING,
            cnavy_leader::COORDINATION,
        ]),
        _ => None,
    };
    if let Some(offsets) = components {
        for offset in offsets {
            if take_or_kill(
                job,
                memory,
                unit.checked_add(offset),
                4,
                MASK_UNITS,
                &mut dead.units,
            )
            .is_none()
            {
                break;
            }
        }
    }
}

/// Mirror of `cunit_leader::read_text`: both string headers plus heap
/// payloads and terminators by header length.
fn copy_unit_text(
    job: &mut CharactersJob,
    memory: &impl Memory,
    unit: usize,
    dead: &mut Dead,
) {
    let name_header = take_or_kill(
        job,
        memory,
        unit.checked_add(cunit_leader::NAME),
        32,
        MASK_UNITS,
        &mut dead.units,
    );
    let token_header = take_or_kill(
        job,
        memory,
        unit.checked_add(cunit_leader::TOKEN),
        32,
        MASK_UNITS,
        &mut dead.units,
    );
    let (Some(name_header), Some(token_header)) = (name_header, token_header) else {
        return;
    };
    let mut header = [0u8; 32];
    header.copy_from_slice(&job.bytes[name_header..name_header + 32]);
    let name = copy_unit_literal(job, memory, &header, MASK_UNITS);
    header.copy_from_slice(&job.bytes[token_header..token_header + 32]);
    let token = copy_unit_literal(job, memory, &header, MASK_UNITS);
    if name.is_none() || token.is_none() {
        dead.units = true;
    }
}

/// Trait-vector copy parameters, bundled to keep the mirror entry small.
struct TraitVectorCopy {
    unit: usize,
    leader_id: u32,
    array_offset: usize,
    stride: usize,
    has_experience: bool,
    mask: u8,
}

/// Mirror of `walk_definitions`' read set for one trait vector: identity,
/// array header, entry pointers and definition/token spans per element.
/// Order and capacity rules are local; any failure kills only this table.
fn copy_trait_vector(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    params: TraitVectorCopy,
    dead: &mut bool,
) -> bool {
    let TraitVectorCopy {
        unit,
        leader_id,
        array_offset,
        stride,
        has_experience,
        mask,
    } = params;
    if *dead {
        return true;
    }
    let Some(identity) =
        take_or_kill(job, memory, Some(unit), 16, mask, dead)
    else {
        return !job.overflowed;
    };
    if pool_u32(job, identity + cunit_leader::ID) != leader_id {
        *dead = true;
        return true;
    }
    let Some(array_header) = take_or_kill(
        job,
        memory,
        unit.checked_add(array_offset),
        16,
        mask,
        dead,
    ) else {
        return !job.overflowed;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&job.bytes[array_header..array_header + 16]);
    let array = match Array::from_header(raw, stride) {
        Some(array) => array,
        None => {
            *dead = true;
            return true;
        }
    };
    let expected = match base.checked_add(cunit_leader_trait::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => {
            *dead = true;
            return true;
        }
    };
    for ordinal in 0..array.count {
        let entry = match (ordinal as usize)
            .checked_mul(stride)
            .and_then(|offset| array.data.checked_add(offset))
        {
            Some(entry) => entry,
            None => {
                *dead = true;
                return true;
            }
        };
        let entry_len = if has_experience {
            unit_leader_trait_experience_entry::STRIDE
        } else {
            8
        };
        let Some(entry_start) =
            take_or_kill(job, memory, Some(entry), entry_len, mask, dead)
        else {
            return !job.overflowed;
        };
        let definition = usize::try_from(pool_u64(job, entry_start)).ok();
        let Some(definition) = definition.filter(|&definition| definition != 0) else {
            // A null definition pointer fails the typed element read.
            *dead = true;
            return true;
        };
        let Some(definition_header) =
            take_or_kill(job, memory, Some(definition), 12, mask, dead)
        else {
            return !job.overflowed;
        };
        let Some(definition_ordinal) = take_or_kill(
            job,
            memory,
            definition.checked_add(cunit_leader_trait::ORDINAL),
            4,
            mask,
            dead,
        ) else {
            return !job.overflowed;
        };
        let Some(token_header) = take_or_kill(
            job,
            memory,
            definition.checked_add(cunit_leader_trait::TOKEN),
            32,
            mask,
            dead,
        ) else {
            return !job.overflowed;
        };
        if pool_u64(job, definition_header) != expected {
            *dead = true;
            return true;
        }
        let mut header = [0u8; 32];
        header.copy_from_slice(&job.bytes[token_header..token_header + 32]);
        if copy_unit_literal(job, memory, &header, mask).is_none() {
            *dead = true;
            return true;
        }
        let _ = (definition_ordinal, ordinal);
    }
    true
}

/// Mirror of one `walk_family` loop iteration's read set across all six
/// batches. Returns `false` only to abort the whole copy (pool overflow or a
/// census-level fault); batch kills are recorded in `dead` and the loop
/// continues exactly like the typed walk.
fn copy_character(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    dead: &mut Dead,
) -> bool {
    let Some(view) = copy_view(job, memory, base, pointer) else {
        return false;
    };
    if job.overflowed {
        return false;
    }
    let id = u32::from_le_bytes(
        view.bytes[0][ccharacter::ID..ccharacter::ID + 4]
            .try_into()
            .unwrap(),
    );
    copy_character_text(job, memory, &view, pointer, dead);
    if job.overflowed {
        return false;
    }
    if !copy_leaders(job, memory, base, &view, pointer, dead) {
        return false;
    }
    if !copy_advisors(job, memory, base, &view, pointer, dead) {
        return false;
    }
    if !copy_unit(job, memory, base, &view, pointer, dead) {
        return false;
    }
    // Trait vectors share the unit leader pointer resolved view-locally
    // above; a null role skips them silently like the typed walk.
    let unit = usize::try_from(u64::from_le_bytes(view.bytes[5][..8].try_into().unwrap()))
        .ok()
        .filter(|&unit| unit != 0);
    if let Some(unit) = unit {
        if !copy_trait_vector(
            job,
            memory,
            base,
            TraitVectorCopy {
                unit,
                leader_id: id,
                array_offset: cunit_leader::TRAIT_DATA,
                stride: 8,
                has_experience: false,
                mask: MASK_TRAITS,
            },
            &mut dead.traits,
        ) {
            return false;
        }
        if !copy_trait_vector(
            job,
            memory,
            base,
            TraitVectorCopy {
                unit,
                leader_id: id,
                array_offset: cunit_leader::TRAIT_EXPERIENCE_DATA,
                stride: unit_leader_trait_experience_entry::STRIDE,
                has_experience: true,
                mask: MASK_EXPERIENCE,
            },
            &mut dead.experience,
        ) {
            return false;
        }
    }
    true
}

/// Mirror of the roster census plus the cursor's 32-entry page loop. Page and
/// pointer parses are local; any census failure stops the copy and the replay
/// fails identically from the same recorded spans.
fn copy_pass(memory: &impl Memory, base: usize, game_state: usize, job: &mut CharactersJob) {
    job.game_state = game_state;
    let mut dead = Dead::default();
    let Some(manager_addr) = game_state.checked_add(cgame_state::CHARACTER_MANAGER) else {
        return;
    };
    let Some(manager_start) = job.take(memory, manager_addr, 8, MASK_ALL) else {
        return;
    };
    let manager = match usize::try_from(pool_u64(job, manager_start)).ok().filter(|&manager| manager != 0) {
        Some(manager) => manager,
        None => return,
    };
    let Some(vtable_start) = job.take(memory, manager, 8, MASK_ALL) else {
        return;
    };
    let expected = match base.checked_add(ccharacter_manager::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    if pool_u64(job, vtable_start) != expected {
        return;
    }
    let mut arrays = [Array { data: 0, count: 0 }; 2];
    for (slot, offset) in [
        ccharacter_manager::HISTORICAL_CHARACTERS,
        ccharacter_manager::DYNAMIC_CHARACTERS,
    ]
    .iter()
    .enumerate()
    {
        let Some(addr) = manager.checked_add(*offset) else {
            return;
        };
        let Some(header_start) = job.take(memory, addr, 16, MASK_ALL) else {
            return;
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&job.bytes[header_start..header_start + 16]);
        let Some(array) = Array::from_header(raw, 8) else {
            return;
        };
        arrays[slot] = array;
    }
    if arrays[0].count.checked_add(arrays[1].count).is_none() {
        return;
    }
    for array in arrays.iter() {
        let mut ordinal = 0u32;
        while ordinal < array.count {
            let chunk = (array.count - ordinal).min(ROSTER_PAGE);
            let Some(page_addr) = array.element(ordinal, 8) else {
                return;
            };
            let Some(page_start) = job.take(
                memory,
                page_addr,
                chunk as usize * 8,
                MASK_ALL,
            ) else {
                return;
            };
            for index in 0..chunk {
                let at = page_start + index as usize * 8;
                let pointer = match usize::try_from(u64::from_le_bytes(
                    job.bytes[at..at + 8].try_into().unwrap(),
                ))
                .ok()
                .filter(|&pointer| pointer != 0)
                {
                    Some(pointer) => pointer,
                    // A null roster pointer fails the typed cursor for this
                    // and every later character, so the copy stops here and
                    // the replay fails identically from the same page bytes.
                    None => return,
                };
                if !copy_character(job, memory, base, pointer, &mut dead) {
                    return;
                }
            }
            ordinal += chunk;
        }
    }
}

impl Captured {
    /// Game-thread raw copy for a slot-0 refresh hour. Copies `first` always
    /// and `second` while any family table still needs double-read checks,
    /// compares the raw logs, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is
    /// complete (`raw_ok`); on staging overflow the caller falls back to the
    /// typed walk and enqueues nothing.
    pub(crate) fn copy_characters_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: CharactersCopyContext,
        first: &mut CharactersJob,
        second: Option<&mut CharactersJob>,
    ) -> CharactersCopyReport {
        let needed = |local: usize| ctx.needed[TABLES[local].index()];
        let started = qpc();
        copy_pass(memory, ctx.base, ctx.game_state, first);
        let mut ticks = [0u64; 6];
        ticks[0] = elapsed(started);
        let mut matched = [true; 6];
        if let Some(second_job) = second {
            let second_started = qpc();
            copy_pass(memory, ctx.base, ctx.game_state, second_job);
            ticks[0] += elapsed(second_started);
            let equal =
                first.logs_equal(second_job) && !first.overflowed && !second_job.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                *matched = equal && !first.fault[slot] && !second_job.fault[slot];
            }
        }
        self.characters.rows.checked = needed(0);
        self.characters.rows.mismatches = usize::from(needed(0) && !matched[0]);
        self.leaders.rows.checked = needed(1);
        self.leaders.rows.mismatches = usize::from(needed(1) && !matched[1]);
        self.advisors.rows.checked = needed(2);
        self.advisors.rows.mismatches = usize::from(needed(2) && !matched[2]);
        self.units.rows.checked = needed(3);
        self.units.rows.mismatches = usize::from(needed(3) && !matched[3]);
        self.traits.rows.checked = needed(4);
        self.traits.rows.mismatches = usize::from(needed(4) && !matched[4]);
        self.trait_experience.rows.checked = needed(5);
        self.trait_experience.rows.mismatches = usize::from(needed(5) && !matched[5]);
        let raw_ok = !first.overflowed;
        CharactersCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw refresh payload into the pooled
    /// typed rows. Outcomes, arena bytes and counts match the typed walk
    /// exactly because the identical walk runs against the recorded spans;
    /// checked/mismatches set by the raw compare survive. Clears `raw_mode`
    /// so the next capture starts clean.
    pub(crate) fn decode_characters_payload(
        &mut self,
        base: usize,
        job: &CharactersJob,
        decode_ticks: &mut [u64; 6],
    ) {
        let started = qpc();
        let recorded = RecordedMemory::build(job);
        let results = walk_family(
            &recorded,
            base,
            job.game_state,
            self.family(false, [true; 6]),
        );
        self.characters.primary(results[0]);
        self.leaders.primary(results[1]);
        self.advisors.primary(results[2]);
        self.units.primary(results[3]);
        self.traits.primary(results[4]);
        self.trait_experience.primary(results[5]);
        decode_ticks[0] += elapsed(started);
        self.raw_mode = false;
    }
}

/// Shared synthetic address space for the characters copy/decode split
/// harness: role maps with leader/advisor nodes, heap and inline strings,
/// skill definitions, army and navy kinds, and a third navy character.
#[cfg(test)]
pub(crate) mod characters_test_kit {
    use super::{
        Buffer, Capacities, Captured, CharactersCopyContext, CharactersCopyReport, CharactersJob,
        N, walk_family,
    };
    use super::tests::{
        ADDRESS, CHARACTERS, IMAGE, MANAGER, ROSTER, UNITS, fixture, literal, pointer, put,
    };
    use engine::{
        cadvisor, carmy_leader, ccharacter, ccharacter_manager, ccountry_leader, cnavy_leader,
        cunit_leader, cunit_leader_skill, character_advisor_node, character_leader_node,
        msvc_string, msvc_tree_node,
    };
    use replay::frame::TABLE_COUNT;

    pub(crate) const BASE: usize = IMAGE;
    pub(crate) const GAME_STATE: usize = ADDRESS;

    pub(crate) const LEAD_NODE: usize = 0x8100;
    pub(crate) const ADV_NODE: usize = 0x8200;
    pub(crate) const HEAP_AREA: usize = 0x8300;
    pub(crate) const SKILL0: usize = 0x7900;
    const LEAD_ROLE: usize = 0x8140;
    const ADV_ROLE: usize = 0x8260;
    const CHAR2: usize = 0x2300;
    const UNIT2: usize = 0x5200;
    const HEAD2L: usize = 0x2500;
    const HEAD2A: usize = 0x2530;

    pub(crate) fn memory(bytes: &[u8]) -> engine::memory::SliceMemory<'_> {
        engine::memory::SliceMemory { base: ADDRESS, bytes }
    }

    fn heap_string(bytes: &mut [u8], header: usize, payload: usize, value: &[u8]) {
        assert!(value.len() > msvc_string::INLINE_CAPACITY as usize);
        put(bytes, header, ((ADDRESS + payload) as u64).to_le_bytes());
        put(bytes, header + 16, (value.len() as u64).to_le_bytes());
        put(bytes, header + 24, (value.len() as u64).to_le_bytes());
        bytes[payload..payload + value.len()].copy_from_slice(value);
        bytes[payload + value.len()] = 0;
    }

    fn empty_map(bytes: &mut [u8], character: usize, offset: usize, head: usize) {
        pointer(bytes, character + offset, head);
        for link in [0, 8, 16] {
            pointer(bytes, head + link, head);
        }
        bytes[head + msvc_tree_node::NIL] = 1;
    }

    /// Base fixture plus one leader role, one heap-keyed advisor role, a
    /// valid skill on unit 0, army kind on unit 1, and a third navy character
    /// with a heap name.
    pub(crate) fn mapped_fixture() -> Vec<u8> {
        let mut bytes = fixture();
        put(
            &mut bytes,
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS + 8,
            3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS + 12,
            3_u32.to_le_bytes(),
        );
        pointer(&mut bytes, ROSTER + 16, CHAR2);
        // Single leader role on character 0 with ideology key 7. The map
        // count lives in the character's map header, not the sentinel.
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::LEADER_MAP_HEAD + 8,
            1_u64.to_le_bytes(),
        );
        pointer(&mut bytes, 0x2400, LEAD_NODE);
        pointer(&mut bytes, LEAD_NODE, 0x2400);
        pointer(&mut bytes, LEAD_NODE + 8, 0x2400);
        pointer(&mut bytes, LEAD_NODE + 16, 0x2400);
        bytes[LEAD_NODE + msvc_tree_node::NIL] = 0;
        put(
            &mut bytes,
            LEAD_NODE + character_leader_node::IDEOLOGY,
            7_i32.to_le_bytes(),
        );
        pointer(
            &mut bytes,
            LEAD_NODE + character_leader_node::LEADER,
            LEAD_ROLE,
        );
        put(
            &mut bytes,
            LEAD_ROLE,
            ((IMAGE + ccountry_leader::VTABLE_RVA) as u64).to_le_bytes(),
        );
        pointer(
            &mut bytes,
            LEAD_ROLE + ccountry_leader::CHARACTER,
            CHARACTERS[0],
        );
        // Single advisor role on character 0 with a 20-byte heap key.
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD + 8,
            1_u64.to_le_bytes(),
        );
        pointer(&mut bytes, 0x2430, ADV_NODE);
        pointer(&mut bytes, ADV_NODE, 0x2430);
        pointer(&mut bytes, ADV_NODE + 8, 0x2430);
        pointer(&mut bytes, ADV_NODE + 16, 0x2430);
        bytes[ADV_NODE + msvc_tree_node::NIL] = 0;
        pointer(
            &mut bytes,
            ADV_NODE + character_advisor_node::ADVISOR,
            ADV_ROLE,
        );
        heap_string(
            &mut bytes,
            ADV_NODE + character_advisor_node::KEY,
            HEAP_AREA,
            b"twenty-byte-advisor!",
        );
        put(
            &mut bytes,
            ADV_NODE + character_advisor_node::KEY_HASH,
            1234_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            ADV_ROLE,
            ((IMAGE + cadvisor::VTABLE_RVA) as u64).to_le_bytes(),
        );
        pointer(
            &mut bytes,
            ADV_ROLE + cadvisor::CHARACTER,
            CHARACTERS[0],
        );
        literal(&mut bytes, ADV_ROLE + cadvisor::IDEA_TOKEN, b"idea");
        literal(&mut bytes, ADV_ROLE + cadvisor::SLOT, b"slot");
        // Valid skill definition on unit 0.
        put(
            &mut bytes,
            SKILL0,
            ((IMAGE + cunit_leader_skill::VTABLE_RVA) as u64).to_le_bytes(),
        );
        bytes[SKILL0 + cunit_leader_skill::VALID] = 1;
        put(
            &mut bytes,
            SKILL0 + cunit_leader_skill::LEVEL,
            3_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            SKILL0 + cunit_leader_skill::LEVEL + 4,
            100_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            SKILL0 + cunit_leader_skill::LEVEL + 8,
            5_i32.to_le_bytes(),
        );
        pointer(&mut bytes, UNITS[0] + cunit_leader::SKILL, SKILL0);
        // Army kind with components on unit 1.
        put(
            &mut bytes,
            UNITS[1],
            ((IMAGE + carmy_leader::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            UNITS[1] + carmy_leader::ATTACK,
            11_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNITS[1] + carmy_leader::DEFENSE,
            22_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNITS[1] + carmy_leader::PLANNING,
            33_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNITS[1] + carmy_leader::LOGISTICS,
            44_i32.to_le_bytes(),
        );
        // Third character with a heap name and a navy unit.
        put(
            &mut bytes,
            CHAR2,
            ((IMAGE + ccharacter::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            CHAR2 + ccharacter::TYPE,
            ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(&mut bytes, CHAR2 + ccharacter::ID, 42_u32.to_le_bytes());
        put(&mut bytes, CHAR2 + ccharacter::COUNTRY, 3_u32.to_le_bytes());
        pointer(&mut bytes, CHAR2 + ccharacter::UNIT_LEADER, UNIT2);
        heap_string(
            &mut bytes,
            CHAR2 + ccharacter::NAME,
            HEAP_AREA + 64,
            b"heap-named-character!",
        );
        literal(&mut bytes, CHAR2 + ccharacter::TOKEN, b"tok");
        empty_map(&mut bytes, CHAR2, ccharacter::LEADER_MAP_HEAD, HEAD2L);
        empty_map(&mut bytes, CHAR2, ccharacter::ADVISOR_MAP_HEAD, HEAD2A);
        put(
            &mut bytes,
            UNIT2,
            ((IMAGE + cnavy_leader::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cunit_leader::TYPE,
            cunit_leader::REFERENCE_TYPE.to_le_bytes(),
        );
        put(&mut bytes, UNIT2 + cunit_leader::ID, 42_u32.to_le_bytes());
        put(
            &mut bytes,
            UNIT2 + cunit_leader::CHARACTER_TYPE,
            ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cunit_leader::CHARACTER_ID,
            42_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cunit_leader::EXPERIENCE,
            999_i64.to_le_bytes(),
        );
        literal(&mut bytes, UNIT2 + cunit_leader::TOKEN, b"n2");
        literal(&mut bytes, UNIT2 + cunit_leader::NAME, b"m2");
        put(&mut bytes, UNIT2 + cunit_leader::TRAIT_DATA, 0_u64.to_le_bytes());
        put(
            &mut bytes,
            UNIT2 + cunit_leader::TRAIT_EXPERIENCE_DATA,
            0_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cnavy_leader::ATTACK,
            51_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cnavy_leader::DEFENSE,
            52_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cnavy_leader::MANEUVERING,
            53_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cnavy_leader::COORDINATION,
            54_i32.to_le_bytes(),
        );
        bytes
    }

    /// Reference typed walk with identical pooled capacities, outcomes and
    /// arenas installed exactly like `capture` installs them.
    pub(crate) fn direct(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> Captured {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let results = walk_family(memory, BASE, GAME_STATE, captured.family(false, [true; 6]));
        captured.characters.primary(results[0]);
        captured.leaders.primary(results[1]);
        captured.advisors.primary(results[2]);
        captured.units.primary(results[3]);
        captured.traits.primary(results[4]);
        captured.trait_experience.primary(results[5]);
        captured
    }

    /// Raw-copy split: two raw passes compared, then the first payload
    /// decoded into fresh pooled buffers of the same capacities.
    pub(crate) fn split(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> (Captured, CharactersCopyReport, CharactersJob) {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let ctx = CharactersCopyContext {
            base: BASE,
            game_state: GAME_STATE,
            needed: [true; TABLE_COUNT],
        };
        let mut first = CharactersJob::new();
        let mut second = CharactersJob::new();
        let report = captured.copy_characters_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut ticks = [0u64; 6];
        captured.decode_characters_payload(BASE, &first, &mut ticks);
        (captured, report, first)
    }

    fn assert_buffer<R: PartialEq + std::fmt::Debug>(
        name: &str,
        split: &Buffer<R>,
        direct: &Buffer<R>,
    ) {
        assert_eq!(split.rows.outcome, direct.rows.outcome, "{name} outcome");
        let len = split.rows.outcome.len();
        assert_eq!(direct.rows.outcome.len(), len, "{name} length");
        assert_eq!(&split.rows.rows[..len], &direct.rows.rows[..len], "{name} rows");
        assert_eq!(
            &split.bytes.first[..split.bytes.len],
            &direct.bytes.first[..direct.bytes.len],
            "{name} arena"
        );
        assert_eq!(
            split.bytes.required, direct.bytes.required,
            "{name} required bytes"
        );
    }

    pub(crate) fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_buffer("characters", &split.characters, &direct.characters);
        assert_buffer("leaders", &split.leaders, &direct.leaders);
        assert_buffer("advisors", &split.advisors, &direct.advisors);
        assert_buffer("units", &split.units, &direct.units);
        assert_buffer("traits", &split.traits, &direct.traits);
        assert_buffer(
            "experience",
            &split.trait_experience,
            &direct.trait_experience,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::{cgame_state, cunit_leader_trait, memory::SliceMemory};
    use std::cell::Cell;

    pub(crate) const ADDRESS: usize = 0x10000;
    pub(crate) const IMAGE: usize = 0x140000000;
    pub(crate) const MANAGER: usize = 0x1000;
    pub(crate) const ROSTER: usize = 0x2000;
    pub(crate) const CHARACTERS: [usize; 2] = [0x2100, 0x2200];
    pub(crate) const UNITS: [usize; 2] = [0x3000, 0x5000];
    pub(crate) const TRAIT_ARRAY: usize = 0x7000;
    pub(crate) const EXPERIENCE_ARRAY: usize = 0x7040;
    pub(crate) const DEFINITION: usize = 0x7800;

    fn walk_units(
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        units: Batch<'_, LeaderRow>,
        traits: Batch<'_, cunit_leader::TraitRow>,
        experience: Batch<'_, cunit_leader::TraitExperienceRow>,
    ) -> [ReadResult; 3] {
        let result = walk_family(
            memory,
            base,
            game_state,
            Family {
                characters: Batch::new(&mut [], &mut [], false),
                leaders: Batch::new(&mut [], &mut [], false),
                advisors: Batch::new(&mut [], &mut [], false),
                units,
                traits,
                experience,
            },
        );
        [result[3], result[4], result[5]]
    }

    pub(crate) fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
        bytes[offset..offset + N].copy_from_slice(&value);
    }

    pub(crate) fn pointer(bytes: &mut [u8], offset: usize, target: usize) {
        put(bytes, offset, ((ADDRESS + target) as u64).to_le_bytes());
    }

    pub(crate) fn array(bytes: &mut [u8], offset: usize, data: usize, count: u32) {
        pointer(bytes, offset, data);
        put(bytes, offset + 8, count.to_le_bytes());
        put(bytes, offset + 12, count.to_le_bytes());
    }

    pub(crate) fn literal(bytes: &mut [u8], offset: usize, value: &[u8]) {
        assert!(value.len() < 16);
        bytes[offset..offset + 32].fill(0);
        bytes[offset..offset + value.len()].copy_from_slice(value);
        put(bytes, offset + 16, (value.len() as u64).to_le_bytes());
        put(bytes, offset + 24, 15_u64.to_le_bytes());
    }

    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x9000];
        pointer(&mut bytes, cgame_state::CHARACTER_MANAGER, MANAGER);
        put(
            &mut bytes,
            MANAGER,
            ((IMAGE + ccharacter_manager::VTABLE_RVA) as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
            ROSTER,
            2,
        );
        put(
            &mut bytes,
            DEFINITION,
            ((IMAGE + cunit_leader_trait::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION + cunit_leader_trait::TOKEN_ID,
            (-22_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION + cunit_leader_trait::ORDINAL,
            i32::MIN.to_le_bytes(),
        );
        literal(
            &mut bytes,
            DEFINITION + cunit_leader_trait::TOKEN,
            b"trait\xff",
        );
        for index in 0..2 {
            let character = CHARACTERS[index];
            let unit = UNITS[index];
            let id = u32::MAX - index as u32;
            pointer(&mut bytes, ROSTER + index * 8, character);
            put(
                &mut bytes,
                character,
                ((IMAGE + ccharacter::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                character + ccharacter::TYPE,
                ccharacter::REFERENCE_TYPE.to_le_bytes(),
            );
            put(&mut bytes, character + ccharacter::ID, id.to_le_bytes());
            put(
                &mut bytes,
                character + ccharacter::COUNTRY,
                (index as u32 + 1).to_le_bytes(),
            );
            pointer(&mut bytes, character + ccharacter::UNIT_LEADER, unit);
            literal(&mut bytes, character + ccharacter::NAME, b"literal");
            literal(&mut bytes, character + ccharacter::TOKEN, b"saved");
            for (role, offset) in [ccharacter::LEADER_MAP_HEAD, ccharacter::ADVISOR_MAP_HEAD]
                .into_iter()
                .enumerate()
            {
                let head = 0x2400 + (index * 2 + role) * 0x30;
                pointer(&mut bytes, character + offset, head);
                for link in [0, 8, 16] {
                    pointer(&mut bytes, head + link, head);
                }
                bytes[head + 0x19] = 1;
            }
            put(
                &mut bytes,
                unit,
                ((IMAGE + cunit_leader::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::TYPE,
                cunit_leader::REFERENCE_TYPE.to_le_bytes(),
            );
            put(&mut bytes, unit + cunit_leader::ID, id.to_le_bytes());
            put(
                &mut bytes,
                unit + cunit_leader::CHARACTER_TYPE,
                ccharacter::REFERENCE_TYPE.to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::CHARACTER_ID,
                id.to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::EXPERIENCE,
                (i64::MIN + index as i64).to_le_bytes(),
            );
            literal(
                &mut bytes,
                unit + cunit_leader::TOKEN,
                if index == 0 { b"saved1" } else { b"saved2" },
            );
            literal(
                &mut bytes,
                unit + cunit_leader::NAME,
                if index == 0 { b"other1" } else { b"other2" },
            );
            array(
                &mut bytes,
                unit + cunit_leader::TRAIT_DATA,
                TRAIT_ARRAY + index * 8,
                1,
            );
            pointer(&mut bytes, TRAIT_ARRAY + index * 8, DEFINITION);
            array(
                &mut bytes,
                unit + cunit_leader::TRAIT_EXPERIENCE_DATA,
                EXPERIENCE_ARRAY + index * 16,
                1,
            );
            pointer(&mut bytes, EXPERIENCE_ARRAY + index * 16, DEFINITION);
            put(
                &mut bytes,
                EXPERIENCE_ARRAY + index * 16 + 8,
                (i64::MAX - index as i64).to_le_bytes(),
            );
        }
        bytes
    }

    struct CountedMemory<'a> {
        memory: SliceMemory<'a>,
        character_header_reads: Cell<usize>,
    }

    struct FaultMemory<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
        failures: Cell<usize>,
    }

    impl Memory for FaultMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                self.failures.set(self.failures.get() + 1);
                // Native guarded copies may modify a prefix before failing.
                // Tests must reject the whole failed span even if its unused
                // suffix happens to contain the native default value zero.
                let prefix = self.fault.start.saturating_sub(address).min(output.len());
                if prefix != 0 {
                    self.memory.copy(address, &mut output[..prefix]);
                }
                return false;
            }
            self.memory.copy(address, output)
        }
    }

    impl Memory for CountedMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if output.len() == 16
                && CHARACTERS
                    .iter()
                    .any(|&character| address == ADDRESS + character)
            {
                self.character_header_reads
                    .set(self.character_header_reads.get() + 1);
            }
            self.memory.copy(address, output)
        }
    }

    #[test]
    fn shared_roster_keeps_traits_when_scalar_capacity_is_exhausted() {
        let bytes = fixture();
        let memory = CountedMemory {
            memory: SliceMemory {
                base: ADDRESS,
                bytes: &bytes,
            },
            character_header_reads: Cell::new(0),
        };
        let mut units = [LeaderRow::default(); 1];
        let mut names = [0; 24];
        let mut traits = [cunit_leader::TraitRow::default(); 2];
        let mut tokens = [0; 12];
        let mut experience = [cunit_leader::TraitExperienceRow::default(); 2];
        let mut experience_tokens = [0; 12];
        let results = walk_units(
            &memory,
            IMAGE,
            ADDRESS,
            Batch::new(&mut units, &mut names, true),
            Batch::new(&mut traits, &mut tokens, true),
            Batch::new(&mut experience, &mut experience_tokens, true),
        );
        assert_eq!(memory.character_header_reads.get(), 2);
        assert_eq!(
            results[0].outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(results[1].outcome, Outcome::Rows { len: 2 });
        assert_eq!(results[2].outcome, Outcome::Rows { len: 2 });
        assert_eq!(traits[1].token_start, 6);
        assert_eq!(&tokens, b"trait\xfftrait\xff");
        assert_eq!(experience[1].experience, i64::MAX - 1);
        assert_eq!(leader_row(&units[0], &names).experience, i64::MIN);
        assert_eq!(leader_row(&units[0], &names).inherited_text_a, b"saved1");
        assert_eq!(leader_row(&units[0], &names).inherited_text_b, b"other1");
        assert_eq!(leader_reference(traits[0].leader_id), 0xffff_ffff_0000_1269);
    }

    #[test]
    fn one_trait_vector_fault_does_not_invalidate_other_family_tables() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            UNITS[1] + cunit_leader::TRAIT_COUNT,
            (-1_i32).to_le_bytes(),
        );
        let memory = SliceMemory {
            base: ADDRESS,
            bytes: &bytes,
        };
        let mut units = [LeaderRow::default(); 2];
        let mut names = [0; 24];
        let mut traits = [cunit_leader::TraitRow::default(); 2];
        let mut tokens = [0; 12];
        let mut experience = [cunit_leader::TraitExperienceRow::default(); 2];
        let mut experience_tokens = [0; 12];
        let results = walk_units(
            &memory,
            IMAGE,
            ADDRESS,
            Batch::new(&mut units, &mut names, true),
            Batch::new(&mut traits, &mut tokens, true),
            Batch::new(&mut experience, &mut experience_tokens, true),
        );
        assert_eq!(results[0].outcome, Outcome::Rows { len: 2 });
        assert!(matches!(results[1].outcome, Outcome::Unavailable { .. }));
        assert_eq!(results[1].bytes_len, 0);
        assert_eq!(results[2].outcome, Outcome::Rows { len: 2 });
        assert_eq!(leader_row(&units[1], &names).inherited_text_a, b"saved2");
    }

    #[test]
    fn all_character_tables_share_identity_and_comparison_fields_are_fresh() {
        let mut bytes = fixture();
        let capacity = Capacities {
            rows: [2; N],
            bytes: [32; N],
        };
        let mut captured = Captured::with_capacities(&capacity, &[0; N]);
        {
            let memory = CountedMemory {
                memory: SliceMemory {
                    base: ADDRESS,
                    bytes: &bytes,
                },
                character_header_reads: Cell::new(0),
            };
            let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(false, [true; 6]));
            assert_eq!(memory.character_header_reads.get(), 2);
            assert_eq!(
                results.map(|result| result.outcome),
                [
                    Outcome::Rows { len: 2 },
                    Outcome::Rows { len: 0 },
                    Outcome::Rows { len: 0 },
                    Outcome::Rows { len: 2 },
                    Outcome::Rows { len: 2 },
                    Outcome::Rows { len: 2 },
                ]
            );
            captured.characters.primary(results[0]);
        }
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::NAME_TOKEN_ID,
            (-7_i32).to_le_bytes(),
        );
        let memory = CountedMemory {
            memory: SliceMemory {
                base: ADDRESS,
                bytes: &bytes,
            },
            character_header_reads: Cell::new(0),
        };
        let second = walk_family(
            &memory,
            IMAGE,
            ADDRESS,
            captured.family(true, [true, false, false, false, false, false]),
        );
        assert_eq!(memory.character_header_reads.get(), 2);
        assert!(!captured.characters.compare(second[0]));
        assert_eq!(captured.characters.rows.second[0].name_token_id, -7);
        assert_eq!(captured.characters.rows.rows[0].name_token_id, 0);
    }

    #[test]
    fn invalid_name_header_does_not_hide_roles_or_unit_state() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::NAME + 16,
            16_u64.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: ADDRESS,
            bytes: &bytes,
        };
        let capacity = Capacities {
            rows: [2; N],
            bytes: [32; N],
        };
        let mut captured = Captured::with_capacities(&capacity, &[0; N]);
        let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(false, [true; 6]));
        assert!(matches!(results[0].outcome, Outcome::Unavailable { .. }));
        assert_eq!(results[1].outcome, Outcome::Rows { len: 0 });
        assert_eq!(results[2].outcome, Outcome::Rows { len: 0 });
        for result in &results[3..] {
            assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        }
    }

    #[test]
    fn partial_character_span_faults_are_local_and_failed_spans_are_never_retried() {
        let bytes = fixture();
        let capacity = Capacities {
            rows: [2; N],
            bytes: [32; N],
        };
        let expected = [2, 0, 0, 2, 2, 2];
        for (offset, affected) in [
            (
                ccharacter::NAME + 32,
                [true, false, false, false, false, false],
            ),
            (
                ccharacter::LEADER_MAP_HEAD + 8,
                [false, true, false, false, false, false],
            ),
            (
                ccharacter::ADVISOR_MAP_HEAD + 8,
                [false, false, true, false, false, false],
            ),
            (
                ccharacter::UNIT_LEADER + 4,
                [false, false, false, true, true, true],
            ),
            (
                ccharacter::COUNTRY + 2,
                [true, true, true, false, false, false],
            ),
        ] {
            let fault = ADDRESS + CHARACTERS[0] + offset;
            let memory = FaultMemory {
                memory: SliceMemory {
                    base: ADDRESS,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
                failures: Cell::new(0),
            };
            let mut captured = Captured::with_capacities(&capacity, &[0; N]);
            let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(false, [true; 6]));
            for (index, result) in results.into_iter().enumerate() {
                if affected[index] {
                    assert!(
                        matches!(result.outcome, Outcome::Unavailable { .. }),
                        "offset={offset:x} table={index}"
                    );
                    assert_eq!(result.bytes_len, 0);
                } else {
                    assert_eq!(
                        result.outcome,
                        Outcome::Rows {
                            len: expected[index]
                        },
                        "offset={offset:x} table={index}"
                    );
                }
            }
            assert_eq!(
                memory.failures.get(),
                1,
                "failed span retried at {offset:x}"
            );
        }
    }

    #[test]
    fn unrelated_portrait_bytes_and_unrequested_name_fields_are_not_read() {
        let bytes = fixture();
        let capacity = Capacities {
            rows: [2; N],
            bytes: [32; N],
        };
        let portrait = ADDRESS + CHARACTERS[0] + ccharacter::PORTRAITS;
        let memory = FaultMemory {
            memory: SliceMemory {
                base: ADDRESS,
                bytes: &bytes,
            },
            fault: portrait..portrait + 1,
            failures: Cell::new(0),
        };
        let mut captured = Captured::with_capacities(&capacity, &[0; N]);
        let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(false, [true; 6]));
        for result in results {
            assert!(matches!(result.outcome, Outcome::Rows { .. }));
        }
        assert_eq!(memory.failures.get(), 0);
        let name = ADDRESS + CHARACTERS[0] + ccharacter::NAME;
        let memory = FaultMemory {
            memory: SliceMemory {
                base: ADDRESS,
                bytes: &bytes,
            },
            fault: name..name + 1,
            failures: Cell::new(0),
        };
        let results = walk_family(
            &memory,
            IMAGE,
            ADDRESS,
            captured.family(true, [false, false, false, true, true, true]),
        );
        for result in &results[3..] {
            assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        }
        assert_eq!(memory.failures.get(), 0);
    }

    #[test]
    fn byte_capacity_stops_the_prefix_and_retains_exact_omitted_count() {
        let mut rows = [cunit_leader::TraitRow::default(); 3];
        let mut bytes = [0; 2];
        let mut output = Batch::new(&mut rows, &mut bytes, true);
        output.append(
            |rows, bytes| {
                rows[0].token_len = 2;
                bytes.copy_from_slice(b"ok");
                ReadResult {
                    outcome: Outcome::Truncated {
                        len: 1,
                        unread_rows: 4,
                    },
                    bytes_len: 2,
                    required_bytes: 9,
                }
            },
            |row, offset| relocate(&mut row.token_start, offset),
        );
        output.append(
            |rows, bytes| {
                assert!(rows.is_empty());
                assert!(bytes.is_empty());
                ReadResult::rows(Outcome::Truncated {
                    len: 0,
                    unread_rows: 7,
                })
            },
            |_, _| unreachable!(),
        );
        let result = output.result();
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 11
            }
        );
        assert_eq!((result.bytes_len, result.required_bytes), (2, 9));
    }

    fn small_capacities() -> Capacities {
        Capacities {
            rows: [2; N],
            bytes: [16; N],
        }
    }

    #[test]
    fn ability_conversion_retains_presence_native_widths_and_literal_bytes() {
        let native = active_ability_record::Row {
            country: u32::MAX,
            ordinal: u32::MAX,
            scope_country: 0x8000_0000,
            token_start: 1,
            token_len: 2,
            caster: Some(leader_reference(u32::MAX)),
            strategic_region: Some(u32::MAX),
            division_count: i32::MIN,
            activation_hour: i32::MIN,
            phase: 0xff,
            allocate_cost: 0xfe,
            allocated_command_power: i64::MIN,
            duration_hours: i32::MAX,
            cooldown_hours: i32::MIN,
            target_kind: i32::MIN,
        };
        let replay = ability_row(&native, b"x\xff\x80");
        assert_eq!(replay.token, b"\xff\x80");
        assert_eq!(replay.caster, Some(0xffff_ffff_0000_1269));
        assert_eq!(replay.strategic_region, Some(u32::MAX));
        assert_eq!(replay.division_count, i32::MIN);
        assert_eq!(replay.allocated_command_power, i64::MIN);
        assert_eq!(replay.phase, 0xff);
        assert_eq!(replay.allocate_cost, 0xfe);
        assert_eq!(replay.key(), u64::MAX as u128);
        let absent = ability_row(
            &active_ability_record::Row {
                caster: None,
                strategic_region: None,
                division_count: 1,
                allocated_command_power: -1,
                ..native
            },
            b"x\xff\x80",
        );
        assert_eq!(absent.caster, None);
        assert_eq!(absent.strategic_region, None);
        assert_eq!(
            (absent.division_count, absent.allocated_command_power),
            (1, -1)
        );
    }

    #[test]
    fn conversion_rejects_bad_ranges_duplicate_keys_and_changed_literal_bytes() {
        let mut captured = Captured::with_capacities(&small_capacities(), &[0; N]);
        // These walkers retain native traversal order. Their complete results
        // still cannot publish duplicate row keys through off-thread conversion.
        captured.characters.rows.rows[0].id = u32::MAX;
        captured.characters.rows.rows[1] = captured.characters.rows.rows[0];
        captured.characters.rows.outcome = Outcome::Rows { len: 2 };
        captured.leaders.rows.rows[0].character = u32::MAX;
        captured.leaders.rows.rows[0].ideology = i32::MIN;
        captured.leaders.rows.rows[1] = captured.leaders.rows.rows[0];
        captured.leaders.rows.outcome = Outcome::Rows { len: 2 };
        captured.advisors.rows.rows[0].character = u32::MAX;
        captured.advisors.rows.rows[1] = captured.advisors.rows.rows[0];
        captured.advisors.rows.outcome = Outcome::Rows { len: 2 };
        let views = captured.tables();
        for view in &views[..3] {
            assert!(matches!(view, TableView::Unavailable { .. }));
        }
        captured.abilities.rows.rows[0].token_len = 1;
        captured.abilities.rows.outcome = Outcome::Rows { len: 1 };
        assert!(matches!(
            captured.tables()[6],
            TableView::Unavailable { .. }
        ));
        captured.abilities.bytes.len = 1;
        captured.abilities.bytes.required = 1;
        captured.abilities.bytes.first[0] = 0xff;
        captured.abilities.rows.rows[1] = captured.abilities.rows.rows[0];
        captured.abilities.rows.outcome = Outcome::Rows { len: 2 };
        assert!(matches!(
            captured.tables()[6],
            TableView::Unavailable { .. }
        ));
        captured.abilities.rows.outcome = Outcome::Rows { len: 1 };
        captured.abilities.rows.second[0] = captured.abilities.rows.rows[0];
        captured.abilities.bytes.second[0] = 0xfe;
        assert!(!captured.abilities.compare(ReadResult {
            outcome: Outcome::Rows { len: 1 },
            bytes_len: 1,
            required_bytes: 1
        }));
        assert_eq!(captured.checked_mismatches()[6], 1);
        assert!(matches!(
            captured.tables()[6],
            TableView::Unavailable { .. }
        ));
        captured.reset();
        assert_eq!(captured.checked_mismatches(), [0; N]);
        assert!(!captured.abilities.rows.checked);
        assert_eq!(captured.abilities.bytes.len, 0);
    }

    // Deterministic read-cost proxy over one synthetic roster. Its proportions
    // approach the measured capture's 0.154 country-leader roles, 0.728 advisor
    // roles and 0.098 unit leaders per character: 32 characters carry 5, 22 and
    // 3 of them, that is 0.156, 0.688 and 0.094, with two traits and fifteen
    // trait experience entries per leader and heap literals throughout. The
    // counts asserted below are guarded entries into the copy shim, the spans
    // inside them, and the bytes those spans move.
    const COST_CHARACTERS: usize = 32;
    const COST_LEADER_STEP: usize = 7;
    const COST_ADVISORS_PER: usize = 2;
    const COST_UNIT_STEP: usize = 11;
    const COST_TRAITS: u32 = 2;
    const COST_EXPERIENCE: u32 = 15;
    const COST_LITERAL: &[u8] = b"seventeen_bytes__";

    const COST_MANAGER: usize = 0x1000;
    const COST_ROSTER: usize = 0x2000;
    const COST_CHARACTER: usize = 0x4000;
    const COST_LEADER_HEAD: usize = 0x8000;
    const COST_ADVISOR_HEAD: usize = 0x9000;
    const COST_LEADER_NODE: usize = 0xa000;
    const COST_ADVISOR_NODE: usize = 0xb000;
    const COST_LEADER_ROLE: usize = 0xd000;
    const COST_ADVISOR_ROLE: usize = 0xe000;
    const COST_UNIT: usize = 0x12000;
    const COST_DEFINITION: usize = 0x16000;
    const COST_TRAIT_ARRAY: usize = 0x1a000;
    const COST_EXPERIENCE_ARRAY: usize = 0x1b000;
    const COST_HEAP: usize = 0x1c000;

    struct CostHeap {
        next: usize,
    }

    impl CostHeap {
        /// Writes a heap MSVC string header, so every literal in this fixture
        /// takes the separate terminator and body read path.
        fn write(&mut self, bytes: &mut [u8], header: usize) {
            let body = self.next;
            self.next += 32;
            put(bytes, header, ((ADDRESS + body) as u64).to_le_bytes());
            put(bytes, header + 16, (COST_LITERAL.len() as u64).to_le_bytes());
            put(bytes, header + 24, (COST_LITERAL.len() as u64).to_le_bytes());
            bytes[body..body + COST_LITERAL.len()].copy_from_slice(COST_LITERAL);
            bytes[body + COST_LITERAL.len()] = 0;
        }
    }

    /// A head sentinel plus a right-leaning chain of `count` content nodes.
    fn cost_tree(
        bytes: &mut [u8],
        field: usize,
        head: usize,
        node: usize,
        stride: usize,
        count: u32,
    ) {
        pointer(bytes, field, head);
        put(bytes, field + 8, u64::from(count).to_le_bytes());
        bytes[head + 0x19] = 1;
        if count == 0 {
            for link in [0, 8, 16] {
                pointer(bytes, head + link, head);
            }
            return;
        }
        let last = node + (count as usize - 1) * stride;
        pointer(bytes, head, node);
        pointer(bytes, head + 8, node);
        pointer(bytes, head + 16, last);
        for index in 0..count as usize {
            let current = node + index * stride;
            pointer(bytes, current, head);
            pointer(
                bytes,
                current + 8,
                if index == 0 { head } else { current - stride },
            );
            pointer(
                bytes,
                current + 16,
                if index + 1 == count as usize {
                    head
                } else {
                    current + stride
                },
            );
        }
    }

    pub(crate) fn cost_fixture() -> Vec<u8> {
        use engine::{character_advisor_node, character_leader_node};
        let mut bytes = vec![0; 0x20000];
        let mut heap = CostHeap { next: COST_HEAP };
        pointer(&mut bytes, cgame_state::CHARACTER_MANAGER, COST_MANAGER);
        put(
            &mut bytes,
            COST_MANAGER,
            ((IMAGE + ccharacter_manager::VTABLE_RVA) as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            COST_MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
            COST_ROSTER,
            COST_CHARACTERS as u32,
        );
        for definition in 0..4 {
            let at = COST_DEFINITION + definition * 0x1000;
            put(
                &mut bytes,
                at,
                ((IMAGE + cunit_leader_trait::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                at + cunit_leader_trait::TOKEN_ID,
                (definition as i32).to_le_bytes(),
            );
            put(
                &mut bytes,
                at + cunit_leader_trait::ORDINAL,
                (definition as i32).to_le_bytes(),
            );
            heap.write(&mut bytes, at + cunit_leader_trait::TOKEN);
        }
        for index in 0..COST_CHARACTERS {
            let character = COST_CHARACTER + index * 0x100;
            pointer(&mut bytes, COST_ROSTER + index * 8, character);
            put(
                &mut bytes,
                character,
                ((IMAGE + ccharacter::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                character + ccharacter::TYPE,
                ccharacter::REFERENCE_TYPE.to_le_bytes(),
            );
            put(
                &mut bytes,
                character + ccharacter::ID,
                (index as u32 + 1).to_le_bytes(),
            );
            put(
                &mut bytes,
                character + ccharacter::COUNTRY,
                (index as u32 % 7 + 1).to_le_bytes(),
            );
            heap.write(&mut bytes, character + ccharacter::NAME);
            heap.write(&mut bytes, character + ccharacter::TOKEN);
            let leaders = u32::from(index % COST_LEADER_STEP == 0);
            cost_tree(
                &mut bytes,
                character + ccharacter::LEADER_MAP_HEAD,
                COST_LEADER_HEAD + index * 0x40,
                COST_LEADER_NODE + index * 0x40,
                character_leader_node::SIZE,
                leaders,
            );
            if leaders != 0 {
                let node = COST_LEADER_NODE + index * 0x40;
                let role = COST_LEADER_ROLE + index * 0x20;
                put(
                    &mut bytes,
                    node + character_leader_node::IDEOLOGY,
                    (index as i32).to_le_bytes(),
                );
                pointer(&mut bytes, node + character_leader_node::LEADER, role);
                put(
                    &mut bytes,
                    role,
                    ((IMAGE + ccountry_leader::VTABLE_RVA) as u64).to_le_bytes(),
                );
                pointer(&mut bytes, role + ccountry_leader::CHARACTER, character);
            }
            let advisors = if index % 3 == 1 {
                COST_ADVISORS_PER as u32
            } else {
                0
            };
            cost_tree(
                &mut bytes,
                character + ccharacter::ADVISOR_MAP_HEAD,
                COST_ADVISOR_HEAD + index * 0x40,
                COST_ADVISOR_NODE + index * 0x100,
                character_advisor_node::SIZE,
                advisors,
            );
            for advisor in 0..advisors as usize {
                let node =
                    COST_ADVISOR_NODE + index * 0x100 + advisor * character_advisor_node::SIZE;
                let role = COST_ADVISOR_ROLE + index * 0x200 + advisor * 0x100;
                put(
                    &mut bytes,
                    node + character_advisor_node::KEY_HASH,
                    (advisor as u32).to_le_bytes(),
                );
                heap.write(&mut bytes, node + character_advisor_node::KEY);
                pointer(&mut bytes, node + character_advisor_node::ADVISOR, role);
                put(
                    &mut bytes,
                    role,
                    ((IMAGE + cadvisor::VTABLE_RVA) as u64).to_le_bytes(),
                );
                pointer(&mut bytes, role + cadvisor::CHARACTER, character);
                heap.write(&mut bytes, role + cadvisor::IDEA_TOKEN);
                heap.write(&mut bytes, role + cadvisor::SLOT);
            }
            if index % COST_UNIT_STEP != 2 {
                continue;
            }
            let slot = index / COST_UNIT_STEP;
            let unit = COST_UNIT + slot * 0x1000;
            pointer(&mut bytes, character + ccharacter::UNIT_LEADER, unit);
            put(
                &mut bytes,
                unit,
                ((IMAGE + cunit_leader::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::TYPE,
                cunit_leader::REFERENCE_TYPE.to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::ID,
                (index as u32 + 1).to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::CHARACTER_TYPE,
                ccharacter::REFERENCE_TYPE.to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::CHARACTER_ID,
                (index as u32 + 1).to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::EXPERIENCE,
                (index as i64).to_le_bytes(),
            );
            heap.write(&mut bytes, unit + cunit_leader::TOKEN);
            heap.write(&mut bytes, unit + cunit_leader::NAME);
            let traits = COST_TRAIT_ARRAY + slot * 0x100;
            array(
                &mut bytes,
                unit + cunit_leader::TRAIT_DATA,
                traits,
                COST_TRAITS,
            );
            for entry in 0..COST_TRAITS as usize {
                pointer(
                    &mut bytes,
                    traits + entry * 8,
                    COST_DEFINITION + (entry % 4) * 0x1000,
                );
            }
            let experience = COST_EXPERIENCE_ARRAY + slot * 0x400;
            array(
                &mut bytes,
                unit + cunit_leader::TRAIT_EXPERIENCE_DATA,
                experience,
                COST_EXPERIENCE,
            );
            for entry in 0..COST_EXPERIENCE as usize {
                pointer(
                    &mut bytes,
                    experience + entry * 16,
                    COST_DEFINITION + (entry % 4) * 0x1000,
                );
                put(
                    &mut bytes,
                    experience + entry * 16 + 8,
                    (entry as i64).to_le_bytes(),
                );
            }
        }
        bytes
    }

    struct CostMemory<'a> {
        memory: SliceMemory<'a>,
        guards: Cell<usize>,
        spans: Cell<usize>,
        bytes: Cell<usize>,
    }

    impl<'a> CostMemory<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self {
                memory: SliceMemory {
                    base: ADDRESS,
                    bytes,
                },
                guards: Cell::new(0),
                spans: Cell::new(0),
                bytes: Cell::new(0),
            }
        }

        fn span(&self, address: usize, output: &mut [u8]) -> bool {
            self.spans.set(self.spans.get() + 1);
            self.bytes.set(self.bytes.get() + output.len());
            self.memory.copy(address, output)
        }
    }

    impl Memory for CostMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.guards.set(self.guards.get() + 1);
            self.span(address, output)
        }

        fn copy_scattered<const M: usize>(
            &self,
            addresses: [usize; M],
            outputs: &mut [&mut [u8]; M],
        ) -> usize {
            if outputs.iter().any(|output| !output.is_empty()) {
                self.guards.set(self.guards.get() + 1);
            }
            for (index, (address, output)) in addresses.into_iter().zip(outputs).enumerate() {
                if output.is_empty() {
                    continue;
                }
                if address == 0
                    || address.checked_add(output.len()).is_none()
                    || !self.span(address, output)
                {
                    return index;
                }
            }
            M
        }
    }

    #[test]
    fn character_family_read_cost_stays_at_its_measured_shape() {
        let bytes = cost_fixture();
        let capacity = Capacities {
            rows: [4096; N],
            bytes: [131_072; N],
        };
        let mut captured = Captured::with_capacities(&capacity, &[0; N]);
        let memory = CostMemory::new(&bytes);
        let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(false, [true; 6]));
        assert_eq!(
            results.map(|result| result.outcome),
            [
                Outcome::Rows { len: 32 },
                Outcome::Rows { len: 5 },
                Outcome::Rows { len: 22 },
                Outcome::Rows { len: 3 },
                Outcome::Rows { len: 6 },
                Outcome::Rows { len: 45 },
            ]
        );
        // The shared walk's read cost, pinned. Making the view's copied-span
        // lookup cheaper removed decoding work, not reads, so every one of
        // these three totals must stay exactly where it was.
        assert_eq!(
            (memory.guards.get(), memory.spans.get(), memory.bytes.get()),
            (588, 1204, 16_872)
        );
    }

    #[test]
    fn growth_and_comparison_release_are_independent_for_shared_walk_outputs() {
        let mut capacities = small_capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        captured.traits.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 19,
        };
        captured.traits.bytes.required = 80;
        let mut successful = [0; N];
        successful[3] = CONSISTENCY_TARGET;
        captured.grow(&mut capacities, &successful);
        assert!(captured.units.rows.second.is_empty());
        assert!(captured.units.bytes.second.is_empty());
        assert_eq!(captured.traits.rows.rows.len(), 32);
        assert_eq!(captured.traits.rows.second.len(), 32);
        assert_eq!(captured.traits.bytes.first.len(), 128);
        assert_eq!(captured.traits.bytes.second.len(), 128);
        let before = captured.allocated_bytes();
        successful.fill(CONSISTENCY_TARGET);
        captured.grow(&mut capacities, &successful);
        assert!(captured.traits.rows.second.is_empty());
        assert!(captured.traits.bytes.second.is_empty());
        assert!(captured.allocated_bytes() < before);
        let idle = Captured::with_capacities(&capacities, &successful);
        assert_eq!(idle.allocated_bytes(), captured.allocated_bytes());
    }

    fn split_capacities() -> Capacities {
        Capacities {
            rows: [4096; N],
            bytes: [131_072; N],
        }
    }

    #[test]
    fn split_copy_matches_direct_walk_on_cost_census() {
        use super::characters_test_kit as kit;
        let bytes = cost_fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.characters.rows.outcome, Outcome::Rows { len: 32 });
        assert_eq!(split.leaders.rows.outcome, Outcome::Rows { len: 5 });
        assert_eq!(split.advisors.rows.outcome, Outcome::Rows { len: 22 });
        assert_eq!(split.units.rows.outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.traits.rows.outcome, Outcome::Rows { len: 6 });
        assert_eq!(
            split.trait_experience.rows.outcome,
            Outcome::Rows { len: 45 }
        );
    }

    #[test]
    fn split_copy_matches_direct_walk_with_skills_kinds_and_heap_names() {
        use super::characters_test_kit as kit;
        let bytes = kit::mapped_fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        kit::assert_matches_direct(&split, &direct);
        // Spot-checks: leader ideology, heap advisor key, skill, army and
        // navy components, and the third character's heap name.
        assert_eq!(split.leaders.rows.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.leaders.rows.rows[0].ideology, 7);
        assert_eq!(split.advisors.rows.outcome, Outcome::Rows { len: 1 });
        let advisor = &split.advisors.rows.rows[0];
        assert_eq!(
            &split.advisors.bytes.first[advisor.key_start..advisor.key_start + advisor.key_len],
            b"twenty-byte-advisor!"
        );
        let units = &split.units.rows.rows;
        assert_eq!(units[0].native.skill_status, 2);
        assert_eq!(units[0].native.skill_level, 3);
        assert_eq!(units[0].native.skill_required_experience, 100);
        assert_eq!(units[0].native.skill_category, 5);
        assert_eq!(units[1].native.kind, cunit_leader::KIND_ARMY);
        assert_eq!(units[1].native.components, [11, 22, 33, 44]);
        assert_eq!(units[2].native.kind, cunit_leader::KIND_NAVY);
        assert_eq!(units[2].native.components, [51, 52, 53, 54]);
        let character = &split.characters.rows.rows[2];
        assert_eq!(
            &split.characters.bytes.first
                [character.name_start..character.name_start + character.name_len],
            b"heap-named-character!"
        );
    }

    #[test]
    fn split_copy_matches_direct_walk_under_truncation() {
        use super::characters_test_kit as kit;
        let bytes = cost_fixture();
        let memory = kit::memory(&bytes);
        let capacities = Capacities {
            rows: [2; N],
            bytes: [64; N],
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        kit::assert_matches_direct(&split, &direct);
        assert!(matches!(
            split.characters.rows.outcome,
            Outcome::Truncated { .. }
        ));
    }

    #[test]
    fn split_copy_matches_direct_walk_on_fault_matrix() {
        use super::characters_test_kit as kit;
        // (fault offset from the image base, expected raw-compare verdicts in
        // family order). Outcomes including reason strings must match the
        // typed walk exactly; verdicts follow the fault-attribution masks.
        for (offset, matched) in [
            (
                cgame_state::CHARACTER_MANAGER,
                [false, false, false, false, false, false],
            ),
            (ROSTER, [false, false, false, false, false, false]),
            (
                CHARACTERS[0],
                [false, false, false, false, false, false],
            ),
            (
                CHARACTERS[0] + ccharacter::COUNTRY,
                [false, false, false, true, true, true],
            ),
            (
                CHARACTERS[0] + ccharacter::NAME_TOKEN_ID,
                [false, true, true, true, true, true],
            ),
            (
                CHARACTERS[0] + ccharacter::NAME,
                [false, true, true, true, true, true],
            ),
            (
                CHARACTERS[0] + ccharacter::LEADER_MAP_HEAD,
                [true, false, true, true, true, true],
            ),
            (
                CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD,
                [true, true, false, true, true, true],
            ),
            (
                CHARACTERS[0] + ccharacter::UNIT_LEADER,
                [true, true, true, false, false, false],
            ),
            (kit::HEAP_AREA, [true, true, false, true, true, true]),
            (
                kit::LEAD_NODE + character_leader_node::IDEOLOGY,
                [true, false, true, true, true, true],
            ),
            (UNITS[0], [true, true, true, false, false, false]),
            (
                UNITS[0] + cunit_leader::TRAIT_DATA,
                [true, true, true, true, false, true],
            ),
            (DEFINITION, [true, true, true, true, false, false]),
            (
                EXPERIENCE_ARRAY,
                [true, true, true, true, true, false],
            ),
            (kit::SKILL0, [true, true, true, false, true, true]),
            (
                kit::HEAP_AREA + 64,
                [false, true, true, true, true, true],
            ),
        ] {
            let bytes = kit::mapped_fixture();
            let fault = ADDRESS + offset;
            let memory = FaultMemory {
                memory: SliceMemory {
                    base: ADDRESS,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
                failures: Cell::new(0),
            };
            let capacities = split_capacities();
            let direct = kit::direct(&memory, &capacities);
            let (split, report, _) = kit::split(&memory, &capacities);
            assert!(report.raw_ok, "offset={offset:x}");
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(report.matched, matched, "offset={offset:x}");
        }
    }

    #[test]
    fn double_copy_compare_blocks_checks_on_tearing() {
        use super::characters_test_kit as kit;
        use replay::frame::TABLE_COUNT;
        let bytes = kit::mapped_fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = CharactersCopyContext {
            base: IMAGE,
            game_state: ADDRESS,
            needed: [true; TABLE_COUNT],
        };
        let mut first = CharactersJob::new();
        let mut second = CharactersJob::new();
        let report =
            captured.copy_characters_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        // Same-hour tearing between the passes changes the log, so a copy
        // taken after the mutation disagrees with the earlier one.
        let mut torn = bytes.clone();
        put(
            &mut torn,
            CHARACTERS[0] + ccharacter::ID,
            0x1234_5678_u32.to_le_bytes(),
        );
        let torn_memory = kit::memory(&torn);
        let mut after = CharactersJob::new();
        copy_pass(&memory, IMAGE, ADDRESS, &mut after);
        let mut after_torn = CharactersJob::new();
        copy_pass(&torn_memory, IMAGE, ADDRESS, &mut after_torn);
        assert!(!after.logs_equal(&after_torn));
        // Identical images agree byte-for-byte.
        let mut repeat = CharactersJob::new();
        copy_pass(&memory, IMAGE, ADDRESS, &mut repeat);
        assert!(after.logs_equal(&repeat));
    }

    #[test]
    fn staging_overflow_falls_back_to_typed_walk() {
        use super::characters_test_kit as kit;
        use replay::frame::TABLE_COUNT;
        let bytes = kit::mapped_fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = CharactersCopyContext {
            base: IMAGE,
            game_state: ADDRESS,
            needed: [true; TABLE_COUNT],
        };
        let mut first = CharactersJob {
            spans: Vec::with_capacity(8),
            bytes: Vec::with_capacity(8),
            game_state: 0,
            fault: [false; 6],
            raw_bytes: [0; 6],
            overflowed: false,
        };
        let report = captured.copy_characters_refresh(&memory, ctx, &mut first, None);
        assert!(!report.raw_ok);
        assert!(first.overflowed);
        assert!(first.grow_for_next());
    }
}
