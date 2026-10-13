//! Quiet roster hours of the roster job ([`crate::roster_changes`]).
//!
//! The roster is the `CCharacterManager`'s historical (`+0x10`) and dynamic
//! (`+0x28`) `CPdxArray<CCharacter*>`. An element is only appended, by
//! `roster push(array, unique_ptr)`, and freed only with its whole array, by
//! `roster clear(array)`; nothing erases, swaps or reorders one. The manager is
//! rebuilt only by its constructor, and a rostered character's id
//! (`+0xC`) is written after its push only by
//! character identity repair. Every one of them moves the
//! roster domain's quiet token ([`engine::character_roster_mutations`]), so
//! while the token stands still the roster's occurrences (pointers), their
//! order (roster and ordinal) and their ids are those of the job's latest
//! complete full pass.
//!
//! The roster job takes the token at the start of each hourly pass, before
//! any registration or read, and holds it with the scope only when that pass
//! was a complete full pass: a complete portrait census, no staging overflow
//! and no failed read. Pooled jobs rotate; each holds its own token, census
//! and carry, and equal tokens since that job's own pass prove that no
//! roster writer ran since. A later role hour whose token equals the held
//! one, for the same module and game state and outside a full hour, walks no
//! roster (headers, pointer pages or character views):
//!
//! 1. Characters rows: the carry is certified as on every hour
//!    ([`carry::Carry::begin_hour`], [`carry::Carry::sample`]); every
//!    occurrence of the held portrait census must be a held character with
//!    the same pointer and id whose certificate holds, and all of them are
//!    carried this pass.
//! 2. The roster claim's sampled slice ([`crate::sampled_verify`], keyed by
//!    character id): both array headers' counts, then each sliced
//!    occurrence's roster slot and its character's id, read afresh. A
//!    difference or a failed read poisons the domain and the hour runs the
//!    full pass.
//! 3. The role census is gathered from the held portrait census, as after a
//!    full pass ([`gather_role_census`]): each occurrence's country, unit
//!    leader pointer and role headers afresh in one guarded call, keeping
//!    its generation when byte-identical so role jobs keep their quiet hours.
//! 4. UnitLeaders rows, which change at any hour without a hook, are read as
//!    a census role job reads them: the unit leader of every occurrence with
//!    one, in roster order ([`copy_unit`]), then the appended formation
//!    fields. These reads are the job's prepared read program, keyed by the
//!    role census's generation ([`program_key`]): while the census keeps its
//!    generation, every read runs afresh in one guarded execution that
//!    compares each selecting byte before the read it selects, and a changed
//!    byte continues the pass from that read. Any other hour runs the reads
//!    one by one and prepares the program for the next.
//!
//! The decode walks the role census with the carry ([`walk`]): each
//! Characters row is the held row with this hour's id, country, roster and
//! ordinal, exactly as a full pass builds a carried row, and each UnitLeaders
//! row comes from the recorded reads. Those Characters rows are a function of
//! the role census's generation and the carry's version ([`RowsKey`]); the
//! job keeps the rows of its latest quiet decode ([`Published`]), and a quiet
//! hour with the same key publishes a copy of them instead of rebuilding
//! every row. The rows, the loaned portrait census
//! and the role census equal what the full pass publishes over the same
//! memory. Any step that cannot complete runs the full pass instead; a failed
//! gather is not repeated after it, so no failed read is retried.
//!
//! `roster_verify`, on by default during burn-in, takes no quiet hour: the
//! full pass runs, and when the token matched it compares the fresh portrait
//! census with the held one. A completed prepared program checked every
//! pointer page and identity header against them; a rediscovery compares its
//! census with the preceding complete capture
//! ([`ccharacter_manager::PortraitCensus::unchanged`]). A difference poisons
//! the domain. Each roster-job hour records its use of the domain once.
use super::*;
use crate::{
    roster_changes::{Counts, Roster, Table},
    sampled_verify::Check,
};
use engine::{
    ccharacter_manager::PortraitIdentity,
    memory::{RecordField, RecordLayout, RecordSources},
};

/// One roster slot pointer.
const SLOT_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const SLOT_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&SLOT_FIELDS, 8) {
    Some(layout) => layout,
    None => panic!("roster slot layout"),
};
/// One character's id.
const ID_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: ccharacter::ID,
    destination_offset: 0,
    len: 4,
}];
const ID_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&ID_FIELDS, 4) {
    Some(layout) => layout,
    None => panic!("character id layout"),
};
/// Bytes of scratch per sliced occurrence: its slot pointer, then its id.
const SAMPLE: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    token: (u64, u64),
    scope: (usize, usize),
}

/// The roster job's held token and its quiet-hour state.
pub(crate) struct QuietRoster {
    domain: &'static dyn Roster,
    /// Tests: the hours' sampled slice; otherwise the hour's.
    check: Option<Check>,
    /// Verify mode: never quiet; compare each matched full pass.
    verify: bool,
    /// The token taken before this job's latest complete full pass.
    held: Option<Held>,
    /// This payload came from a quiet hour: decode walks the role census.
    pub(super) quiet: bool,
    /// The quiet attempt already certified this hour's carry.
    pub(super) carry_begun: bool,
    /// The quiet attempt gathered the role census; the full pass does not
    /// gather it again.
    gathered: bool,
    counts: Counts,
    /// Sampled slice scratch: slot addresses, held pointers, fresh bytes.
    slots: Vec<usize>,
    pointers: Vec<usize>,
    fresh: Vec<u8>,
}

impl std::fmt::Debug for QuietRoster {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QuietRoster")
            .field("held", &self.held)
            .field("quiet", &self.quiet)
            .field("verify", &self.verify)
            .finish()
    }
}

/// Tests: a domain that never observes, so the shared hooks' domain never
/// leaks between parallel tests. Quiet-hour tests choose their own.
#[cfg(test)]
struct Unobserved;

#[cfg(test)]
impl Roster for Unobserved {
    fn token(&self) -> Option<(u64, u64)> {
        None
    }
    fn mismatch(&self) {}
    fn record(&self, _table: Table, _counts: &Counts) {}
}

impl Default for QuietRoster {
    fn default() -> Self {
        #[cfg(not(test))]
        let domain: &'static dyn Roster = &crate::roster_changes::Observer;
        #[cfg(test)]
        let domain: &'static dyn Roster = &Unobserved;
        Self::with(domain, None)
    }
}

impl QuietRoster {
    /// `check` overrides the hour's sampled slice; [`Check::Every`] is verify
    /// mode.
    pub(super) fn with(domain: &'static dyn Roster, check: Option<Check>) -> Self {
        Self {
            domain,
            check,
            verify: check.map_or(crate::roster_changes::VERIFY, |check| check == Check::Every),
            held: None,
            quiet: false,
            carry_begun: false,
            gathered: false,
            counts: Counts::default(),
            slots: Vec::new(),
            pointers: Vec::new(),
            fresh: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn set_check(&mut self, check: Check) {
        self.check = Some(check);
    }

    /// This job's latest roster-job hour.
    #[cfg(test)]
    pub(super) fn counts(&self) -> Counts {
        self.counts
    }

    /// The roster domain's token.
    pub(super) fn token(&self) -> Option<(u64, u64)> {
        self.domain.token()
    }

    pub(super) fn allocated_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.slots)
            + crate::raw_memory::vector_bytes(&self.pointers)
            + crate::raw_memory::vector_bytes(&self.fresh)
    }

    /// A difference or failed read of the roster claim.
    fn differs(&self, counts: &mut Counts, mismatches: u64) -> bool {
        counts.mismatches += mismatches;
        self.domain.mismatch();
        false
    }

    /// Step 2: the hour's sampled slice of the roster claim over the held
    /// census `rows` and its `historical` length. False when the quiet hour
    /// cannot be taken: a difference or failed read (counted, poisoning) or
    /// scratch storage that cannot be prepared.
    fn sample(
        &mut self,
        memory: &impl Memory,
        ctx: CharactersCopyContext,
        rows: &[PortraitIdentity],
        historical: usize,
        growing: bool,
        counts: &mut Counts,
    ) -> bool {
        let check = self
            .check
            .unwrap_or_else(|| crate::sampled_verify::current_burned_in(false));
        if !check.any() {
            return true;
        }
        let covered = |row: &PortraitIdentity| check.covers(u64::from(row.character));
        let sampled = rows.iter().filter(|&row| covered(row)).count();
        let Some(bytes) = sampled.checked_mul(SAMPLE) else {
            return false;
        };
        self.slots.clear();
        self.pointers.clear();
        if growing
            && engine::allocation::advisory(|| {
                engine::allocation::reserve_exact(&mut self.slots, sampled)?;
                engine::allocation::reserve_exact(&mut self.pointers, sampled)?;
                if self.fresh.len() < bytes {
                    engine::allocation::resize(&mut self.fresh, bytes, 0)?;
                }
                std::io::Result::Ok(())
            })
            .is_err()
        {
            return false;
        }
        if self.slots.capacity() < sampled
            || self.pointers.capacity() < sampled
            || self.fresh.len() < bytes
        {
            return false;
        }
        // Both array headers, afresh (`Roster::read`: the manager pointer,
        // its vtable and both 16-byte headers).
        counts.verified += 1;
        let arrays = match ccharacter_manager::Roster::read(memory, ctx.base, ctx.game_state) {
            Ok(roster) => roster.sections().map(|(array, _)| array),
            Err(_) => return self.differs(counts, 1),
        };
        if arrays[0].count as usize != historical
            || (arrays[0].count as usize).checked_add(arrays[1].count as usize) != Some(rows.len())
        {
            return self.differs(counts, 1);
        }
        for (index, row) in rows.iter().enumerate() {
            if !covered(row) {
                continue;
            }
            let (array, ordinal) = if index < historical {
                (arrays[0], index)
            } else {
                (arrays[1], index - historical)
            };
            // The counts equal the held ones, so every slot is in range.
            let Some(slot) = u32::try_from(ordinal)
                .ok()
                .and_then(|ordinal| array.element(ordinal, 8))
            else {
                return self.differs(counts, 1);
            };
            self.slots.push(slot);
            self.pointers.push(row.pointer);
        }
        counts.verified += sampled as u64;
        if sampled == 0 {
            return true;
        }
        // Each sliced slot, then the id at each held pointer: one guarded
        // gather each. An id read through the held pointer is the slot's
        // character's id whenever the slot still holds that pointer.
        let (slots, ids) = self.fresh[..bytes].split_at_mut(sampled * 8);
        let slots_read =
            memory.copy_records(RecordSources::Pointers(&self.slots), &SLOT_LAYOUT, slots);
        let ids_read =
            memory.copy_records(RecordSources::Pointers(&self.pointers), &ID_LAYOUT, ids);
        let mut mismatches = u64::from(slots_read < sampled || ids_read < sampled);
        for (index, row) in rows.iter().filter(|&row| covered(row)).enumerate() {
            let slot = (index < slots_read)
                .then(|| u64::from_le_bytes(slots[index * 8..][..8].try_into().unwrap()));
            let id = (index < ids_read)
                .then(|| u32::from_le_bytes(ids[index * 4..][..4].try_into().unwrap()));
            if slot.is_some_and(|slot| slot != row.pointer as u64)
                || id.is_some_and(|id| id != row.character)
            {
                mismatches += 1;
            }
        }
        if mismatches != 0 {
            return self.differs(counts, mismatches);
        }
        true
    }
}

/// The roster job's hourly pass (part 0 alone): a quiet hour when the roster
/// stood still since this job's latest complete full pass, otherwise the
/// full pass, followed on role hours (`gather`) by the role census. `token`
/// was taken before any registration or read of this pass.
pub(super) fn capture(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
    gather: bool,
    token: Option<(u64, u64)>,
) {
    let scope = (ctx.base, ctx.game_state);
    job.unit_carry.begin_hour();
    let roster = &mut job.roster;
    roster.quiet = false;
    roster.carry_begun = false;
    roster.gathered = false;
    let matched = cfg!(feature = "roster_changes")
        && !ctx.force_full
        && token.is_some_and(|token| roster.held == Some(Held { token, scope }));
    let verify = roster.verify;
    let held_historical = job.census_historical;
    let mut counts = Counts::default();
    if matched && !verify && gather && quiet_pass(memory, ctx, job, &mut counts) {
        job.roster.quiet = true;
        counts.quiet = true;
    } else {
        copy_pass(memory, ctx, job, None);
        if gather && !job.roster.gathered {
            gather_role_census(memory, ctx, job);
        }
        counts.read = true;
        let complete = job.census_current
            && !job.overflowed()
            && !job.requires_no_retry()
            && !job.log.fault.iter().any(|&fault| fault);
        if matched && verify && complete {
            let census = job.portrait_census.as_deref();
            counts.verified += census
                .and_then(|census| census.rows(ctx.base, ctx.game_state))
                .map_or(0, |rows| rows.len() as u64);
            // A completed prepared program compared every pointer page and
            // identity header with those the held census was read from.
            let same = job.log.planned_capture_succeeded()
                || (job.census_historical == held_historical
                    && census.is_some_and(|census| census.unchanged()));
            if !same {
                counts.mismatches += 1;
                job.roster.domain.mismatch();
            }
        }
        job.roster.held = token
            .filter(|_| complete && counts.mismatches == 0)
            .map(|token| Held { token, scope });
    }
    job.roster.counts = counts;
    job.roster.domain.record(Table::Characters, &counts);
}

/// Steps 1 to 4 of a quiet hour; false runs the full pass instead.
fn quiet_pass(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
    counts: &mut Counts,
) -> bool {
    // Step 1, exactly as the full pass begins (`copy_pass`), which then
    // skips it.
    job.carry
        .begin_hour(ctx.active[0] && !ctx.force_full && cfg!(feature = "character_changes"));
    job.carry.sample(memory, ctx.base);
    job.roster.carry_begun = true;
    let census = job.portrait_census.take();
    let ready = census
        .as_deref()
        .and_then(|census| census.rows(ctx.base, ctx.game_state))
        .is_some_and(|rows| {
            job.carry.certifies(rows)
                && job.roster.sample(
                    memory,
                    ctx,
                    rows,
                    job.census_historical,
                    job.worker_growing,
                    counts,
                )
        });
    job.portrait_census = census;
    if !ready || !job.carry.carry_certified() {
        return false;
    }
    let _reads = read_accounting::table(TABLES[anchor(ctx.active)]);
    // The payload holds this hour's census and unit reads, as a census role
    // job's does.
    job.log.clear();
    if job.worker_growing {
        job.log.enable_worker_growth();
    }
    job.leader_rows = 0;
    job.advisor_demand = AdvisorDemand::default();
    job.appended_overflowed = false;
    job.active = ctx.active;
    job.traits.begin(ctx, job.worker_growing);
    job.game_state = ctx.game_state;
    job.advisor_section_complete = false;
    job.census_visited.clear();
    job.census_required = 0;
    // Step 3: the held census is this hour's (complete for this scope).
    job.census_current = true;
    job.role_census_current = false;
    job.roster.gathered = true;
    gather_role_census(memory, ctx, job);
    if !job.role_census_current {
        job.census_current = false;
        return false;
    }
    job.census_capture = true;
    // Step 4: the leaders the unit carry holds unchanged are not read
    // ([`unit_carry`]); the others are read one by one. When it carries none,
    // as this job's prepared read program (`program_key`): while the role
    // census keeps its generation, every unit-leader read of the job's latest
    // full quiet pass runs afresh in one guarded execution, which compares
    // each byte that selected a later read (a skill pointer, a category, a
    // string header) before that read. A changed selecting byte stops it
    // there and the pass continues from that read; a changed generation or no
    // program runs the pass, which prepares the next one.
    let source_mask = job.source_mask();
    let census = job
        .role_census
        .as_deref()
        .and_then(|census| census.admitted(ctx.base, ctx.game_state));
    let carrying = ctx.active[3]
        && census.is_some_and(|census| {
            job.unit_carry
                .certify(census.generation, census.len(), || leaders(census))
        });
    let key = census.map(|census| program_key(ctx, source_mask, census));
    let planned = !carrying && key.is_some_and(|key| job.log.begin_planned_capture(memory, key));
    if !planned {
        // A completed program read exactly what its pass read, so that
        // pass's unit text demand holds; otherwise the pass counts it again.
        job.unit_demand = AdvisorDemand::default();
        job.appended_roles.clear();
        job.appended_required = 0;
        job.skills.fill(SkillCopy::default());
    }
    copy_units(memory, ctx, job, !planned, carrying);
    if !planned {
        copy_appended_roles(job, memory, ctx.base);
    }
    job.log.finish_planned_capture();
    job.traits.finish();
    true
}

/// An occurrence's unit leader pointer; `None` for none.
pub(super) fn unit(occurrence: census::Occurrence) -> Option<usize> {
    usize::try_from(u64::from_le_bytes(occurrence.unit()))
        .ok()
        .filter(|&unit| unit != 0)
}

/// The census index, unit pointer and character id of every occurrence with
/// a unit leader, in census order.
pub(super) fn leaders(
    census: census::Admitted<'_>,
) -> impl Iterator<Item = (usize, usize, u32)> + '_ {
    (0..census.len()).filter_map(move |index| {
        let occurrence = census.occurrence(index);
        Some((index, unit(occurrence)?, occurrence.id()))
    })
}

/// Marks a quiet pass's program apart from the full pass's, whose third key
/// word is the job's five-bit source mask.
const QUIET_PROGRAM: usize = 1 << (usize::BITS - 1);

/// The key of a quiet pass's prepared program. Its reads are selected only
/// by the role census (each occurrence's unit pointer and id, in roster
/// order), whose generation names its exact content, and by the job's
/// tables; every other selecting byte is a control the program compares.
fn program_key(
    ctx: CharactersCopyContext,
    source_mask: u32,
    census: census::Admitted<'_>,
) -> [usize; 4] {
    [
        ctx.base,
        ctx.game_state,
        QUIET_PROGRAM | source_mask as usize,
        census.generation as usize,
    ]
}

/// Step 4: every occurrence's Characters demand and, when `read` (no
/// prepared program completed), the unit leader of each occurrence with one,
/// in roster order, with the full pass's reads and failure order from its
/// unit pointer on; when `carrying`, only the leaders the unit carry does not
/// carry.
fn copy_units(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
    read: bool,
    carrying: bool,
) {
    /// Occurrences ahead whose leader lines are hinted.
    const LOOKAHEAD: usize = 16;
    let mut dead = Dead {
        characters: !ctx.active[0],
        leaders: true,
        advisors: true,
        units: !ctx.active[3],
        traits: true,
    };
    job.character_demand = AdvisorDemand::default();
    // Taken while the job is written; the census itself is not.
    let roles = job.role_census.take();
    if let Some(census) = roles
        .as_deref()
        .and_then(|census| census.admitted(ctx.base, ctx.game_state))
    {
        // Decode supplies every occurrence's held row; their text joins this
        // pass's demand.
        let (rows, bytes) = job.carry.census_demand(census);
        job.character_demand.add(rows, bytes);
        if carrying {
            // And the carried leaders' held rows.
            let (rows, bytes) = job.unit_carry.carried_demand();
            job.unit_demand.add(rows, bytes);
            for at in 0..job.unit_carry.visit_count() {
                let Some((pointer, id)) = job.unit_carry.fresh_visit(at) else {
                    continue;
                };
                let mut unit_id = None;
                if !copy_unit(
                    job,
                    memory,
                    ctx.base,
                    Some(pointer),
                    |_| id,
                    &mut unit_id,
                    &mut dead,
                ) {
                    break;
                }
            }
            job.role_census = roles;
            return;
        }
        for index in (0..census.len()).filter(|_| read) {
            let occurrence = census.occurrence(index);
            if let Some(ahead) = (index + LOOKAHEAD < census.len())
                .then(|| unit(census.occurrence(index + LOOKAHEAD)))
                .flatten()
            {
                for (offset, len) in [
                    (0, 16),
                    (cunit_leader::NAME, 32),
                    (cunit_leader::SKILL, 32),
                    (cunit_leader::CHARACTER_TYPE, 8),
                    (cunit_leader::EXILE_COUNTRY, 4),
                ] {
                    if let Some(address) = ahead.checked_add(offset) {
                        // The census selects these lines, so the program
                        // keeps them as its own hints.
                        job.log.prefetch(address, len);
                    }
                }
            }
            let Some(pointer) = unit(occurrence) else {
                continue;
            };
            let id = occurrence.id();
            let mut unit_id = None;
            if !copy_unit(
                job,
                memory,
                ctx.base,
                Some(pointer),
                |_| id,
                &mut unit_id,
                &mut dead,
            ) {
                break;
            }
        }
    }
    job.role_census = roles;
}

/// What a quiet roster hour's Characters rows are a function of: the role
/// census they follow (each occurrence's pointer, id, country and roster
/// position, which its nonzero generation names) and the carry's held rows,
/// which such an hour publishes for every occurrence (its version).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RowsKey {
    scope: (usize, usize),
    census: u64,
    carry: u64,
}

/// The key of this payload's Characters rows when it came from a quiet
/// roster hour; `None` otherwise.
pub(super) fn rows_key(base: usize, job: &CharactersJob) -> Option<RowsKey> {
    if !job.roster.quiet || !job.role_census_current {
        return None;
    }
    let census = job.role_census.as_deref()?.admitted(base, job.game_state)?;
    (census.generation != 0).then_some(RowsKey {
        scope: (base, job.game_state),
        census: census.generation,
        carry: job.carry.version,
    })
}

/// The Characters rows of the job's latest quiet decode, under their key.
#[derive(Debug, Default)]
pub(crate) struct Published {
    key: Option<RowsKey>,
    rows: Vec<ccharacter::Row>,
    bytes: Vec<u8>,
    required_bytes: usize,
}

impl Published {
    /// Whether a payload with `key` publishes exactly the held rows.
    pub(super) fn matches(&self, key: Option<RowsKey>) -> bool {
        key.is_some() && key == self.key
    }

    pub(super) fn forget(&mut self) {
        self.key = None;
        self.rows.clear();
        self.bytes.clear();
    }

    pub(super) fn allocated_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.rows) + crate::raw_memory::vector_bytes(&self.bytes)
    }

    /// Decode worker: holds a quiet decode's complete rows under `key`.
    /// Storage is advisory: rows that cannot be held are rebuilt next hour.
    pub(super) fn hold(
        &mut self,
        key: RowsKey,
        result: ReadResult,
        rows: &[ccharacter::Row],
        bytes: &[u8],
    ) {
        self.forget();
        let Outcome::Rows { len } = result.outcome else {
            return;
        };
        let (Some(rows), Some(bytes)) = (rows.get(..len), bytes.get(..result.bytes_len)) else {
            return;
        };
        let stored = engine::allocation::advisory(|| {
            engine::allocation::reserve_exact(&mut self.rows, rows.len())?;
            engine::allocation::reserve_exact(&mut self.bytes, bytes.len())
        });
        if stored.is_ok() {
            self.rows.extend_from_slice(rows);
            self.bytes.extend_from_slice(bytes);
            self.required_bytes = result.required_bytes;
            self.key = Some(key);
        }
    }

    /// What [`Self::copy_into`] returns, for buffers that already hold the
    /// rows.
    pub(super) fn result(&self) -> ReadResult {
        ReadResult {
            outcome: Outcome::Rows {
                len: self.rows.len(),
            },
            bytes_len: self.bytes.len(),
            required_bytes: self.required_bytes,
        }
    }

    /// Copies the held rows into a capture's buffers, as the decode would
    /// have built them.
    pub(super) fn copy_into(
        &self,
        rows: &mut Vec<ccharacter::Row>,
        bytes: &mut Vec<u8>,
    ) -> ReadResult {
        let room = engine::walk::Buffer::Growing(rows)
            .ensure(self.rows.len())
            .and_then(|_| engine::walk::Buffer::Growing(bytes).ensure(self.bytes.len()));
        if let Err(reason) = room {
            return ReadResult::rows(Outcome::Unavailable { reason });
        }
        rows[..self.rows.len()].copy_from_slice(&self.rows);
        bytes[..self.bytes.len()].copy_from_slice(&self.bytes);
        ReadResult {
            outcome: Outcome::Rows {
                len: self.rows.len(),
            },
            bytes_len: self.bytes.len(),
            required_bytes: self.required_bytes,
        }
    }
}

/// The generation of the role census this payload's quiet hour gathered;
/// zero for none.
pub(super) fn census_generation(job: &CharactersJob) -> u64 {
    job.role_census_current
        .then_some(job.role_census.as_deref())
        .flatten()
        .map_or(0, census::RoleCensus::generation)
}

/// Decode of a quiet hour whose buffers hold its held leaders' UnitLeaders
/// table: whether every leader the hour read afresh equals its held row, so
/// that table is this hour's.
pub(super) fn fresh_leaders_unchanged<M: Memory>(
    memory: &M,
    base: usize,
    job: &CharactersJob,
) -> bool {
    let carry = &job.unit_carry;
    let mut row = [LeaderRow::default()];
    let mut bytes = Vec::new();
    for at in 0..carry.visit_count() {
        let Some((pointer, character)) = carry.fresh_visit(at) else {
            continue;
        };
        let read = loop {
            let read = read_unit(memory, base, pointer, character, &mut row, &mut bytes);
            match read.outcome {
                Outcome::Truncated { .. } if read.required_bytes > bytes.len() => {
                    bytes.resize(read.required_bytes, 0);
                }
                _ => break read,
            }
        };
        let Outcome::Rows { len: 1 } = read.outcome else {
            return false;
        };
        if !carry.held_equals(at, &row, &bytes[..read.bytes_len]) {
            return false;
        }
    }
    true
}

/// Decode of a quiet hour: `walk_census` over this pass's role census with
/// the carry, every occurrence in roster order; when only the unit leader
/// tables are decoded (the Characters rows republished), only the
/// occurrences with a unit leader, the others producing no row.
pub(super) fn walk<M: Memory>(
    memory: &M,
    base: usize,
    mut family: Family<'_>,
    job: &CharactersJob,
) -> [ReadResult; N] {
    let census = job
        .role_census_current
        .then_some(job.role_census.as_deref())
        .flatten()
        .and_then(|census| census.admitted(base, job.game_state));
    let Some(census) = census else {
        family.fail("quiet roster hour without its role census");
        family.finish_advisor_tooltips(memory);
        return family.results();
    };
    let [characters, leaders, _, advisors] = family.required();
    let units_only = !characters && !leaders && !advisors;
    // The unit carry's visits are the occurrences with a unit leader.
    let leaders = job.unit_carry.visited().filter(|_| units_only);
    let every = leaders.is_none().then(|| {
        (0..census.len())
            .filter(move |&index| !units_only || unit(census.occurrence(index)).is_some())
    });
    let occurrences = leaders
        .into_iter()
        .flatten()
        .chain(every.into_iter().flatten());
    walk_census(
        memory,
        base,
        family,
        Some(&job.traits),
        occurrences.map(|index| census.occurrence(index)),
        census.historical,
        Carried {
            characters: Some(&job.carry),
            units: Some(&job.unit_carry),
            ..Carried::default()
        },
    )
}
