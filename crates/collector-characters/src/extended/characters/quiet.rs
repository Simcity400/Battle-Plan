use super::*;
use collector_observers::roster_changes::{Counts, Roster, Table};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    token: (u64, u64),
    scope: (usize, usize),
}

pub struct QuietRoster {
    domain: &'static dyn Roster,
    held: Option<Held>,
    pub(super) quiet: bool,
    pub(super) carry_begun: bool,
    gathered: bool,
    counts: Counts,
}

impl std::fmt::Debug for QuietRoster {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QuietRoster")
            .field("held", &self.held)
            .field("quiet", &self.quiet)
            .finish()
    }
}

#[cfg(any(test, feature = "test-support"))]
struct Unobserved;

#[cfg(any(test, feature = "test-support"))]
impl Roster for Unobserved {
    fn token(&self) -> Option<(u64, u64)> {
        None
    }
    fn record(&self, _table: Table, _counts: &Counts) {}
}

impl Default for QuietRoster {
    fn default() -> Self {
        Self::with(&collector_observers::roster_changes::Observer)
    }
}

impl QuietRoster {
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn unobserved() -> Self {
        Self::with(&Unobserved)
    }

    pub(super) fn with(domain: &'static dyn Roster) -> Self {
        Self {
            domain,
            held: None,
            quiet: false,
            carry_begun: false,
            gathered: false,
            counts: Counts::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> Counts {
        self.counts
    }

    pub(super) fn token(&self) -> Option<(u64, u64)> {
        self.domain.token()
    }
}

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
    let mut counts = Counts::default();
    if matched && gather && quiet_pass(memory, ctx, job) {
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
        job.roster.held = token
            .filter(|_| complete)
            .map(|token| Held { token, scope });
    }
    job.roster.counts = counts;
    job.roster.domain.record(Table::Characters, &counts);
}

fn quiet_pass(memory: &impl Memory, ctx: CharactersCopyContext, job: &mut CharactersJob) -> bool {
    job.carry
        .begin_hour(ctx.active[0] && !ctx.force_full && cfg!(feature = "character_changes"));
    job.roster.carry_begun = true;
    let ready = job
        .portrait_census
        .as_deref()
        .and_then(|census| census.rows(ctx.base, ctx.game_state))
        .is_some_and(|rows| job.carry.certifies(rows));
    if !ready || !job.carry.carry_certified() {
        return false;
    }
    let _reads = read_accounting::table(TABLES[anchor(ctx.active)]);
    job.log.clear();
    job.leader_rows = 0;
    job.advisor_demand = AdvisorDemand::default();
    job.appended_overflowed = false;
    job.active = ctx.active;
    job.traits.begin();
    job.game_state = ctx.game_state;
    job.advisor_section_complete = false;
    job.census_visited.clear();
    job.census_required = 0;
    job.census_current = true;
    job.role_census_current = false;
    job.roster.gathered = true;
    gather_role_census(memory, ctx, job);
    if !job.role_census_current {
        job.census_current = false;
        return false;
    }
    job.census_capture = true;
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
    let planned = (!ctx.active[3] || ctx.active[5])
        && !carrying
        && key.is_some_and(|key| job.log.begin_planned_capture(memory, key));
    if !planned {
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

pub(super) fn unit(occurrence: census::Occurrence) -> Option<usize> {
    usize::try_from(u64::from_le_bytes(occurrence.unit()))
        .ok()
        .filter(|&unit| unit != 0)
}

pub(super) fn leaders(
    census: census::Admitted<'_>,
) -> impl Iterator<Item = (usize, usize, u32, usize)> + '_ {
    (0..census.len()).filter_map(move |index| {
        let occurrence = census.occurrence(index);
        Some((index, unit(occurrence)?, occurrence.id(), occurrence.pointer))
    })
}

const QUIET_PROGRAM: usize = 1 << (usize::BITS - 1);

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

fn copy_units(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
    read: bool,
    carrying: bool,
) {
    const LOOKAHEAD: usize = 16;
    let mut dead = Dead {
        characters: !ctx.active[0],
        leaders: true,
        advisors: true,
        units: !ctx.active[3],
        traits: true,
    };
    job.character_demand = AdvisorDemand::default();
    let roles = job.role_census.take();
    if let Some(census) = roles
        .as_deref()
        .and_then(|census| census.admitted(ctx.base, ctx.game_state))
    {
        let (rows, bytes) = job.carry.census_demand(census);
        job.character_demand.add(rows, bytes);
        if carrying {
            for (_, pointer, character, _) in leaders(census) {
                if job.unit_carry.carries(pointer) {
                    job.experience.touch(pointer, character);
                }
            }
            let (rows, bytes) = job.unit_carry.carried_demand();
            job.unit_demand.add(rows, bytes);
            for at in 0..job.unit_carry.visit_count() {
                let Some((pointer, id, character)) = job.unit_carry.fresh_visit(at) else {
                    continue;
                };
                let mut unit_id = None;
                if !copy_unit(
                    job,
                    memory,
                    ctx.base,
                    Some(pointer),
                    character,
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
                occurrence.pointer,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RowsKey {
    scope: (usize, usize),
    census: u64,
    carry: u64,
}

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

#[derive(Debug, Default)]
pub struct Published {
    key: Option<RowsKey>,
    rows: Vec<ccharacter::Row>,
    bytes: Vec<u8>,
    required_bytes: usize,
}

impl Published {
    pub(super) fn matches(&self, key: Option<RowsKey>) -> bool {
        key.is_some() && key == self.key
    }

    pub(super) fn forget(&mut self) {
        self.key = None;
        self.rows.clear();
        self.bytes.clear();
    }

    pub(super) fn allocated_bytes(&self) -> u64 {
        collector_primitives::raw_memory::vector_bytes(&self.rows)
            + collector_primitives::raw_memory::vector_bytes(&self.bytes)
    }

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
        let stored = engine_allocation::advisory(|| {
            engine_allocation::reserve_exact(&mut self.rows, rows.len())?;
            engine_allocation::reserve_exact(&mut self.bytes, bytes.len())
        });
        if stored.is_ok() {
            self.rows.extend_from_slice(rows);
            self.bytes.extend_from_slice(bytes);
            self.required_bytes = result.required_bytes;
            self.key = Some(key);
        }
    }

    pub(super) fn result(&self) -> ReadResult {
        ReadResult {
            outcome: Outcome::Rows {
                len: self.rows.len(),
            },
            bytes_len: self.bytes.len(),
            required_bytes: self.required_bytes,
        }
    }

    pub(super) fn copy_into(
        &self,
        rows: &mut Vec<ccharacter::Row>,
        bytes: &mut Vec<u8>,
    ) -> ReadResult {
        let room = engine_native_memory::walk::Buffer::Growing(rows)
            .ensure(self.rows.len())
            .and_then(|_| engine_native_memory::walk::Buffer::Growing(bytes).ensure(self.bytes.len()));
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

pub(super) fn census_generation(job: &CharactersJob) -> u64 {
    job.role_census_current
        .then_some(job.role_census.as_deref())
        .flatten()
        .map_or(0, census::RoleCensus::generation)
}

pub(super) fn fresh_leaders_unchanged<M: Memory>(
    memory: &M,
    base: usize,
    job: &CharactersJob,
) -> bool {
    let carry = &job.unit_carry;
    let mut bytes = Vec::new();
    for at in 0..carry.visit_count() {
        let Some((pointer, character, character_pointer)) = carry.fresh_visit(at) else {
            continue;
        };
        let Ok(read) = read_unit_selected(
            memory,
            base,
            pointer,
            character_pointer,
            character,
            &mut bytes,
            0,
            job.active[5],
        ) else {
            return false;
        };
        if !carry.held_equals(at, &[read.row], &bytes[..read.bytes_len]) {
            return false;
        }
    }
    true
}

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

