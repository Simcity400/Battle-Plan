//! Persistent character state. Native walks use pooled rows and literal-byte
//! arenas; sorting, duplicate checks and owned replay strings run off-thread.

use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, active_ability_record, cadvisor, ccharacter, ccharacter_manager, ccountry,
    ccountry_leader, cunit_leader, read, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{
        ActiveAbilityRow, AdvisorRoleRow, CharacterRow, CountryLeaderRoleRow, TableId, TableSchema,
        UnitLeaderRow, UnitLeaderTraitExperienceRow, UnitLeaderTraitRow, tables,
    },
};

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
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        let check = |index| needed.get(index).copied().unwrap_or(false);
        let mut matched = [false; N];
        let mut ticks = [0; N];
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::{cgame_state, cunit_leader_trait, memory::SliceMemory};
    use std::cell::Cell;

    const ADDRESS: usize = 0x10000;
    const IMAGE: usize = 0x140000000;
    const MANAGER: usize = 0x1000;
    const ROSTER: usize = 0x2000;
    const CHARACTERS: [usize; 2] = [0x2100, 0x2200];
    const UNITS: [usize; 2] = [0x3000, 0x5000];
    const TRAIT_ARRAY: usize = 0x7000;
    const EXPERIENCE_ARRAY: usize = 0x7040;
    const DEFINITION: usize = 0x7800;

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

    fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
        bytes[offset..offset + N].copy_from_slice(&value);
    }

    fn pointer(bytes: &mut [u8], offset: usize, target: usize) {
        put(bytes, offset, ((ADDRESS + target) as u64).to_le_bytes());
    }

    fn array(bytes: &mut [u8], offset: usize, data: usize, count: u32) {
        pointer(bytes, offset, data);
        put(bytes, offset + 8, count.to_le_bytes());
        put(bytes, offset + 12, count.to_le_bytes());
    }

    fn literal(bytes: &mut [u8], offset: usize, value: &[u8]) {
        assert!(value.len() < 16);
        bytes[offset..offset + 32].fill(0);
        bytes[offset..offset + value.len()].copy_from_slice(value);
        put(bytes, offset + 16, (value.len() as u64).to_le_bytes());
        put(bytes, offset + 24, 15_u64.to_le_bytes());
    }

    fn fixture() -> Vec<u8> {
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
}
