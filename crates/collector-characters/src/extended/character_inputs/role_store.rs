use crate::extended::characters::census::Admitted;
use collector_observers::character_changes::{Changes, Counts};
use engine_allocation as allocation;
use engine_characters::{
    character_inputs::CENSUS_PAGE, character_inputs::CensusWalk, character_inputs::MAP_SENTINEL,
    character_inputs::Occupant, character_inputs::OperativeRow, character_inputs::Trait,
    character_inputs::TraitRow,
};
use engine_native::Memory;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Certificate {
    epoch: u64,
    sequence: u64,
    pointer: usize,
    character: u32,
    leaders: [u8; 16],
    advisors: [u8; 16],
}

#[derive(Clone, Debug)]
struct Entry {
    certificate: Certificate,
    rows: Range<usize>,
}

#[derive(Default)]
struct Held {
    entries: Vec<Entry>,
    rows: Vec<TraitRow>,
    values: Vec<Trait>,
}

impl Held {
    fn clear(&mut self) {
        self.entries.clear();
        self.rows.clear();
        self.values.clear();
    }

    fn position(&self, pointer: usize) -> Option<usize> {
        self.entries
            .binary_search_by_key(&pointer, |entry| entry.certificate.pointer)
            .ok()
    }

    fn hold(&mut self, certificate: Certificate, rows: &[TraitRow], values: &[Trait]) -> bool {
        let count: usize = rows.iter().map(|row| row.len).sum();
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.entries, 1)?;
            allocation::reserve(&mut self.rows, rows.len())?;
            allocation::reserve(&mut self.values, count)?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        let first = self.rows.len();
        for row in rows {
            let start = self.values.len();
            self.values
                .extend_from_slice(&values[row.start..row.start + row.len]);
            self.rows.push(TraitRow { start, ..*row });
        }
        self.entries.push(Entry {
            certificate,
            rows: first..self.rows.len(),
        });
        true
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.rows.capacity() * size_of::<TraitRow>()
            + self.values.capacity() * size_of::<Trait>()
    }
}

#[derive(Default)]
pub struct RoleStore {
    current: Held,
    previous: Held,
    scope: (usize, usize),
    counts: Counts,
    bytes: usize,
    forgotten: bool,
    quiet: Option<((u64, u64), u64)>,
    breaks: Vec<Break>,
    scientist_rows: Vec<TraitRow>,
    scientist_values: Vec<Trait>,
    scientists: Option<(u64, u64)>,
    fresh_rows: Vec<TraitRow>,
    fresh_values: Vec<Trait>,
    content: u64,
    operatives: Operatives,
}

fn fresh_content() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub trait LeaderChanges: Sync {
    fn certificate(&self, leader: usize) -> Option<[(u64, u64); 2]>;
}

pub struct LeaderObserver;
impl LeaderChanges for LeaderObserver {
    fn certificate(&self, leader: usize) -> Option<[(u64, u64); 2]> {
        use collector_observers::unit_leader_changes::Changes;
        Some([
            collector_observers::unit_leader_changes::Observer.sequence(leader)?,
            collector_observers::unit_leader_trait_changes::Observer.sequence(leader)?,
        ])
    }
}

#[cfg(any(test, feature = "test-support"))]
impl LeaderChanges for collector_observers::unit_leader_changes::Watches {
    fn certificate(&self, leader: usize) -> Option<[(u64, u64); 2]> {
        use collector_observers::unit_leader_changes::Changes;
        let sequence = self.sequence(leader)?;
        Some([sequence; 2])
    }
}

#[derive(Clone, Debug)]
struct HeldOperative {
    index: usize,
    reference: u64,
    object: usize,
    certificate: [(u64, u64); 2],
    row: OperativeRow,
    definitions: Range<usize>,
    text: Range<usize>,
}

#[derive(Default)]
struct Operatives {
    held: Vec<HeldOperative>,
    definitions: Vec<u32>,
    text: Vec<u8>,
    next: Vec<HeldOperative>,
    next_definitions: Vec<u32>,
    next_text: Vec<u8>,
    scratch: Vec<u8>,
    counts: OperativeCounts,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OperativeCounts {
    pub carried: u64,
    pub read: u64,
}

impl Operatives {
    fn measure(&self) -> usize {
        (self.held.capacity() + self.next.capacity()) * size_of::<HeldOperative>()
            + (self.definitions.capacity() + self.next_definitions.capacity()) * size_of::<u32>()
            + self.text.capacity()
            + self.next_text.capacity()
            + self.scratch.capacity()
    }

    fn forget(&mut self) {
        self.held.clear();
        self.definitions.clear();
        self.text.clear();
    }

    fn begin(&mut self) {
        self.next.clear();
        self.next_definitions.clear();
        self.next_text.clear();
        self.counts = OperativeCounts::default();
    }

    fn finish(&mut self, complete: bool) {
        if complete {
            std::mem::swap(&mut self.held, &mut self.next);
            std::mem::swap(&mut self.definitions, &mut self.next_definitions);
            std::mem::swap(&mut self.text, &mut self.next_text);
        } else {
            self.forget();
        }
    }

    fn holding(&self, index: usize, reference: u64) -> Option<&HeldOperative> {
        let at = self
            .held
            .binary_search_by_key(&index, |held| held.index)
            .ok()?;
        Some(&self.held[at]).filter(|held| held.reference == reference)
    }

    fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        index: usize,
        occupant: &Occupant,
        walk: &mut CensusWalk<'_>,
        leaders: &dyn LeaderChanges,
    ) {
        if occupant.operative == 0 {
            return;
        }
        let held = self.holding(index, occupant.operative).cloned();
        let held_certificate = held
            .as_ref()
            .and_then(|held| leaders.certificate(held.object));
        let carryable = held
            .as_ref()
            .is_some_and(|held| held_certificate == Some(held.certificate));
        if let Some(held) = held.as_ref().filter(|_| carryable) {
            walk.carry_operative(
                &held.row,
                &self.definitions[held.definitions.clone()],
                &self.text[held.text.clone()],
            );
            self.counts.carried += 1;
            self.keep(held.clone(), None);
            return;
        }
        self.counts.read += 1;
        let Some(object) = walk.resolve_operative(memory, base, occupant.operative) else {
            return;
        };
        let certificate = match (object, held.as_ref()) {
            (Some(object), Some(held)) if object == held.object => held_certificate,
            (Some(object), _) => leaders.certificate(object),
            (None, _) => None,
        };
        let stored = walk.operatives_stored();
        walk.operative_resolved(memory, base, occupant, object);
        if walk.operatives_stored() == stored {
            return;
        }
        let mut text = std::mem::take(&mut self.scratch);
        let last = allocation::advisory(|| walk.last_operative(&mut text));
        let Some((row, definitions)) = last else {
            self.scratch = text;
            return;
        };
        let whole = row.name_reason.is_none() && row.codename_reason.is_none();
        if let (Some(object), Some(certificate), true) = (object, certificate, whole) {
            self.keep(
                HeldOperative {
                    index,
                    reference: occupant.operative,
                    object,
                    certificate,
                    row,
                    definitions: 0..0,
                    text: 0..0,
                },
                Some((definitions, text.as_slice())),
            );
        }
        self.scratch = text;
    }

    fn keep(&mut self, mut held: HeldOperative, fresh: Option<(&[u32], &[u8])>) {
        let (definitions, text) = match fresh {
            Some(fresh) => fresh,
            None => (
                &self.definitions[held.definitions.clone()],
                &self.text[held.text.clone()],
            ),
        };
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.next, 1)?;
            allocation::reserve(&mut self.next_definitions, definitions.len())?;
            allocation::reserve(&mut self.next_text, text.len())
        });
        if stored.is_err() {
            return;
        }
        let start = self.next_definitions.len();
        self.next_definitions.extend_from_slice(definitions);
        held.definitions = start..self.next_definitions.len();
        let start = self.next_text.len();
        self.next_text.extend_from_slice(text);
        held.text = start..self.next_text.len();
        self.next.push(held);
    }
}

pub struct RoleReuse<'a> {
    pub store: &'a mut RoleStore,
    pub changes: &'a dyn Changes,
    pub held: Option<u64>,
    pub leaders: &'a dyn LeaderChanges,
}

impl RoleStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn forget(&mut self) {
        self.forgotten = true;
        self.content = 0;
        self.operatives.forget();
    }

    pub fn begin_operatives(&mut self) {
        self.operatives.begin();
    }

    pub fn finish_operatives(&mut self, complete: bool) {
        self.operatives.finish(complete);
        let counts = self.operatives.counts;
        collector_observers::character_changes::record_operatives(counts.carried, counts.read);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn operative_counts(&self) -> OperativeCounts {
        self.operatives.counts
    }

    pub fn content(&self) -> u64 {
        self.content
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    fn begin(&mut self, scope: (usize, usize)) {
        if std::mem::take(&mut self.forgotten) || self.scope != scope {
            self.current.clear();
            self.previous.clear();
            self.scope = scope;
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
        self.counts = Counts::default();
        self.quiet = None;
        self.breaks.clear();
        self.scientist_rows.clear();
        self.scientist_values.clear();
        self.scientists = None;
    }

    fn measure(&mut self) {
        self.bytes = self.current.measure()
            + self.previous.measure()
            + self.breaks.capacity() * size_of::<Break>()
            + self.scientist_rows.capacity() * size_of::<TraitRow>()
            + self.scientist_values.capacity() * size_of::<Trait>()
            + self.fresh_rows.capacity() * size_of::<TraitRow>()
            + self.fresh_values.capacity() * size_of::<Trait>()
            + self.operatives.measure();
    }

    fn hold_scientist(&mut self, rows: &[TraitRow], values: &[Trait]) -> Option<Range<usize>> {
        let count: usize = rows.iter().map(|row| row.len).sum();
        allocation::advisory(|| {
            allocation::reserve(&mut self.scientist_rows, rows.len())?;
            allocation::reserve(&mut self.scientist_values, count)
        })
        .ok()?;
        let first = self.scientist_rows.len();
        for row in rows {
            let start = self.scientist_values.len();
            self.scientist_values
                .extend_from_slice(&values[row.start..row.start + row.len]);
            self.scientist_rows.push(TraitRow { start, ..*row });
        }
        Some(first..self.scientist_rows.len())
    }

    fn quiet_hour(
        &self,
        scope: (usize, usize),
        token: Option<(u64, u64)>,
        census: &Admitted<'_>,
    ) -> bool {
        !self.forgotten
            && self.scope == scope
            && census.generation != 0
            && token.is_some_and(|token| self.quiet == Some((token, census.generation)))
    }

    fn finish(&mut self, complete: bool) {
        self.content = if complete { fresh_content() } else { 0 };
        if complete {
            crate::extended::characters::sort_dropping_repeated(
                &mut self.current.entries,
                |entry| entry.certificate.pointer,
            );
        } else {
            self.current.clear();
        }
        self.previous.clear();
        self.measure();
        collector_observers::character_changes::record(
            collector_observers::character_changes::Table::CharacterRoleTraits,
            &self.counts,
        );
    }

    fn carry_all(
        &mut self,
        memory: &impl Memory,
        base: usize,
        census: &Admitted<'_>,
        walk: &mut CensusWalk<'_>,
        scientists: bool,
        leaders: &dyn LeaderChanges,
    ) -> ScientistCounts {
        let Self {
            current: held,
            breaks,
            scientist_rows,
            scientist_values,
            fresh_rows,
            fresh_values,
            operatives,
            ..
        } = self;
        fresh_rows.clear();
        fresh_values.clear();
        let mut counts = ScientistCounts {
            held: true,
            ..Default::default()
        };
        let mut at = 0;
        for brk in breaks.iter_mut() {
            walk.carry(&held.rows[at..brk.end], &held.values);
            let occupant = census.occurrence(brk.index).occupant();
            let rows = &scientist_rows[brk.scientist.clone()];
            let from = walk.stored();
            if scientists {
                walk.carry(rows, scientist_values);
                counts.carried += 1;
            } else {
                walk.scientist(memory, base, &occupant);
            }
            let (published, values) = walk.stored_since(from);
            let count: usize = published.iter().map(|row| row.len).sum();
            let stored = walk.traits_complete()
                && allocation::advisory(|| {
                    allocation::reserve(fresh_rows, published.len())?;
                    allocation::reserve(fresh_values, count)
                })
                .is_ok();
            if stored {
                let first = fresh_rows.len();
                for row in published {
                    let start = fresh_values.len();
                    fresh_values.extend_from_slice(&values[row.start..row.start + row.len]);
                    fresh_rows.push(TraitRow { start, ..*row });
                }
                brk.scientist = first..fresh_rows.len();
            } else {
                counts.held = false;
            }
            operatives.visit(memory, base, brk.index, &occupant, walk, leaders);
            at = brk.end;
        }
        walk.carry(&held.rows[at..], &held.values);
        if counts.held {
            std::mem::swap(scientist_rows, fresh_rows);
            std::mem::swap(scientist_values, fresh_values);
        }
        if counts.carried as usize != self.breaks.len() || !counts.held {
            self.content = fresh_content();
        }
        counts
    }

    fn operatives_only(
        &mut self,
        memory: &impl Memory,
        base: usize,
        census: &Admitted<'_>,
        walk: &mut CensusWalk<'_>,
        leaders: &dyn LeaderChanges,
    ) {
        let Self {
            breaks, operatives, ..
        } = self;
        for brk in breaks.iter() {
            let occupant = census.occurrence(brk.index).occupant();
            operatives.visit(memory, base, brk.index, &occupant, walk, leaders);
        }
    }
}

#[derive(Clone, Debug)]
struct Break {
    index: usize,
    end: usize,
    scientist: Range<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ScientistCounts {
    carried: u64,
    held: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visited {
    Walked,
    Held,
}

pub fn visit(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: Admitted<'_>,
    walk: &mut CensusWalk<'_>,
    mut reuse: Option<RoleReuse<'_>>,
) -> Visited {
    let token = reuse.as_ref().and_then(|reuse| reuse.changes.quiet());
    let scientist_token = reuse.as_ref().and_then(|reuse| reuse.changes.scientists());
    if let Some(reuse) = reuse.as_mut()
        && reuse.store.quiet_hour((base, game_state), token, &census)
    {
        let scientists = scientist_token.is_some() && reuse.store.scientists == scientist_token;
        if scientists && reuse.store.content != 0 && reuse.held == Some(reuse.store.content) {
            reuse
                .store
                .operatives_only(memory, base, &census, walk, reuse.leaders);
            collector_observers::roster_changes::record(
                collector_observers::roster_changes::Table::CharacterRoleTraits,
                &collector_observers::roster_changes::Counts {
                    quiet: true,
                    ..Default::default()
                },
            );
            let store = &mut *reuse.store;
            store.counts = Counts {
                reused: census.len() as u64,
                quiet: true,
                ..Default::default()
            };
            store.measure();
            collector_observers::character_changes::record(
                collector_observers::character_changes::Table::CharacterRoleTraits,
                &store.counts,
            );
            return Visited::Held;
        }
        let counts = reuse
            .store
            .carry_all(memory, base, &census, walk, scientists, reuse.leaders);
        reuse.store.scientists = scientist_token.filter(|_| counts.held);
        collector_observers::roster_changes::record(
            collector_observers::roster_changes::Table::CharacterRoleTraits,
            &collector_observers::roster_changes::Counts {
                quiet: scientists,
                read: !scientists,
            },
        );
        if walk.traits_complete() {
            let store = &mut *reuse.store;
            store.counts = Counts {
                reused: census.len() as u64,
                quiet: true,
                ..Default::default()
            };
            store.measure();
            collector_observers::character_changes::record(
                collector_observers::character_changes::Table::CharacterRoleTraits,
                &store.counts,
            );
            return Visited::Walked;
        }
        reuse.store.forget();
        reuse.store.begin((base, game_state));
        reuse.store.finish(false);
        return Visited::Walked;
    }
    if let Some(reuse) = reuse.as_mut() {
        reuse.store.begin((base, game_state));
    }
    let mut held_all = true;
    let mut index = 0;
    let mut occupants = (0..census.len())
        .map(|index| census.occurrence(index).occupant())
        .peekable();
    let mut page = [Occupant::default(); CENSUS_PAGE];
    let mut plans = [Plan::default(); CENSUS_PAGE];
    let mut headers = [[0_u8; 16]; CENSUS_PAGE * 2];
    let mut sentinels = [0_u8; CENSUS_PAGE * 2 * MAP_SENTINEL];
    while occupants.peek().is_some() {
        let mut len = 0;
        let mut fresh = 0;
        for (slot, occupant) in page.iter_mut().zip(occupants.by_ref()) {
            *slot = occupant;
            let plan = reuse
                .as_ref()
                .map_or_else(Plan::default, |reuse| plan(reuse, &occupant));
            if plan.held.is_none() {
                headers[fresh * 2] = occupant.leaders;
                headers[fresh * 2 + 1] = occupant.advisors;
                fresh += 1;
            }
            plans[len] = plan;
            len += 1;
        }
        walk.sentinels(
            memory,
            &headers[..fresh * 2],
            &mut sentinels[..fresh * 2 * MAP_SENTINEL],
        );
        let mut next = 0;
        for (occupant, plan) in page[..len].iter().zip(&plans[..len]) {
            let from = walk.stored();
            match (reuse.as_mut(), plan.held) {
                (Some(reuse), Some(index)) => {
                    let previous = &reuse.store.previous;
                    let rows = previous.entries[index].rows.clone();
                    walk.carry(&previous.rows[rows], &previous.values);
                    reuse.store.counts.reused += 1;
                }
                (reuse, _) => {
                    let at = next * 2 * MAP_SENTINEL;
                    let sentinel = |map: usize| -> [u8; MAP_SENTINEL] {
                        sentinels[at + map * MAP_SENTINEL..][..MAP_SENTINEL]
                            .try_into()
                            .unwrap()
                    };
                    walk.roles(memory, base, occupant, [sentinel(0), sentinel(1)]);
                    next += 1;
                    if let Some(reuse) = reuse {
                        reuse.store.counts.read += 1;
                    }
                }
            }
            if let Some(reuse) = reuse.as_mut() {
                let store = &mut *reuse.store;
                match plan.certificate {
                    Some(certificate) if walk.traits_complete() => {
                        let (rows, values) = walk.stored_since(from);
                        held_all &= store.current.hold(certificate, rows, values);
                    }
                    _ => held_all = false,
                }
                if held_all && (occupant.scientist != 0 || occupant.operative != 0) {
                    held_all =
                        allocation::advisory(|| allocation::reserve(&mut store.breaks, 1)).is_ok();
                    if held_all {
                        store.breaks.push(Break {
                            index,
                            end: store.current.rows.len(),
                            scientist: 0..0,
                        });
                    }
                }
            }
            let from = walk.stored();
            walk.scientist(memory, base, occupant);
            if let Some(reuse) = reuse.as_mut()
                && held_all
                && (occupant.scientist != 0 || occupant.operative != 0)
            {
                let store = &mut *reuse.store;
                let (rows, values) = walk.stored_since(from);
                match store.hold_scientist(rows, values) {
                    Some(range) => store.breaks.last_mut().unwrap().scientist = range,
                    None => held_all = false,
                }
            }
            match reuse.as_mut() {
                Some(reuse) => {
                    reuse
                        .store
                        .operatives
                        .visit(memory, base, index, occupant, walk, reuse.leaders)
                }
                None => walk.operative(memory, base, occupant),
            }
            index += 1;
        }
    }
    if let Some(reuse) = reuse {
        let store = reuse.store;
        let complete = walk.traits_complete();
        store.finish(complete);
        store.quiet = token
            .filter(|_| {
                complete
                    && held_all
                    && census.generation != 0
                    && store.current.entries.len() == census.len()
            })
            .map(|token| (token, census.generation));
        if store.quiet.is_none() {
            store.breaks.clear();
            store.scientist_rows.clear();
            store.scientist_values.clear();
        }
        store.scientists = store.quiet.and(scientist_token);
        collector_observers::roster_changes::record(
            collector_observers::roster_changes::Table::CharacterRoleTraits,
            &collector_observers::roster_changes::Counts {
                read: true,
                ..Default::default()
            },
        );
    }
    Visited::Walked
}

#[derive(Clone, Copy, Default)]
struct Plan {
    certificate: Option<Certificate>,
    held: Option<usize>,
}

fn plan(reuse: &RoleReuse<'_>, occupant: &Occupant) -> Plan {
    let certificate = reuse
        .changes
        .sequence(occupant.pointer)
        .map(|(epoch, sequence)| Certificate {
            epoch,
            sequence,
            pointer: occupant.pointer,
            character: occupant.character,
            leaders: occupant.leaders,
            advisors: occupant.advisors,
        });
    let previous = &reuse.store.previous;
    let held = certificate.and_then(|certificate| {
        let index = previous.position(certificate.pointer)?;
        (previous.entries[index].certificate == certificate).then_some(index)
    });
    Plan { certificate, held }
}
