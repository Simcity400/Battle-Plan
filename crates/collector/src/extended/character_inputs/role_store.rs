//! Country-leader and advisor role rows held between hours for
//! CharacterRoleTraits ([`crate::character_changes`]).
//!
//! The census walk visits every occurrence of the roster job's admitted
//! census in roster order. A character's held country-leader and advisor rows
//! are carried, without reading its maps, roles or trait arrays, only when all
//! of these hold at this boundary: the same character pointer and id as the
//! held read; the same country-leader and advisor map headers (head node and
//! size, freshly gathered by the roster job this callback); and no hooked
//! writer touched the character since the read registered it (same watch
//! epoch and sequence; a load, gap, recovery, table change, poison, an
//! unnamed receiver or a running writer revokes it). Every writer of those
//! maps, their roles' parent links and trait arrays is a hooked site
//! ([`engine::character_role_mutations`], [`engine::advisor_mutations`]), and
//! trait definitions (`+0x8` token, `+0x1A0` ordinal) are database objects
//! nothing rewrites, so the fresh read would return the held rows. Scientist
//! rows and operative links are read every hour. Every hour also reads the
//! carryable characters of its sampled-verification slice afresh
//! ([`crate::sampled_verify`], keyed by character id; `character_verify`
//! takes every one), publishes the fresh rows and poisons reuse on a
//! difference.
//!
//! A quiet hour carries every country-leader and advisor row without a
//! lookup: when the latest full walk held every occupant, the character
//! domain's quiet token ([`Changes::quiet`]) equals the one taken before that
//! walk's first registration, and the census has that walk's generation
//! ([`Admitted::generation`]), no observed writer ran and every occupant,
//! identity and map header is the one read then, so every held certificate
//! still matches. The held rows are appended in runs between the occupants
//! with a scientist or operative, whose operative links are read as every
//! hour; only the sampled slice of carried characters is read and compared
//! first. A difference poisons reuse and the hour walks every occupant
//! afresh; a slice read that fails walks every occupant too.
//!
//! A quiet hour also carries the scientist rows that walk held when the
//! scientist domain's token ([`Changes::scientists`],
//! [`crate::roster_changes`]) equals the one taken before it: the census's
//! scientist pointers are that walk's (same generation), and every writer of
//! A scientist role or its trait array  (scientist build, scientist attach,
//! scientist clear, scientist trait add, scientist trait remove, scientist trait reset) moves that token.
//! Scientists whose character falls in the hour's slice are read afresh and
//! compared; under `roster_verify` every one is. A difference poisons the
//! scientist domain and publishes the fresh row. Otherwise the scientist rows
//! are read as every hour.
//!
//! The store follows the Gated job's loan and holds only the characters of
//! the latest complete census walk, so its size follows the live roster.
//! Allocation is advisory: a character whose rows cannot be held is read
//! afresh next hour.
use crate::{
    character_changes::{Changes, Counts},
    extended::characters::census::Admitted,
    sampled_verify::Check,
};
use engine::{
    Memory, allocation,
    character_inputs::{
        CENSUS_PAGE, CensusWalk, GrowingBuffers, MAP_SENTINEL, Occupant, OperativeRow, Trait,
        TraitRow,
    },
};
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

/// One walk's held rows; each row's `start` indexes `values`.
#[derive(Default)]
struct Held {
    /// Sorted by character pointer once the walk finishes.
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

    /// Holds `rows` (indexing `values`) for `certificate`; `false` when the
    /// storage could not grow, which only withholds the certificate.
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

/// Rows of the latest complete walk and the walk being made.
#[derive(Default)]
pub(crate) struct RoleStore {
    current: Held,
    previous: Held,
    scope: (usize, usize),
    counts: Counts,
    /// Allocated bytes as of the last finish, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next walk drops every held row.
    forgotten: bool,
    /// What lets a quiet hour carry every held row: the domain's quiet token
    /// before the latest full walk and its census generation, when that walk
    /// held every occupant.
    quiet: Option<((u64, u64), u64)>,
    /// Of that walk, in roster order: each occupant with a scientist or an
    /// operative link, the held rows up to and including its own, and its
    /// scientist rows.
    breaks: Vec<Break>,
    /// The scientist rows of `breaks`; each row's `start` indexes
    /// `scientist_values`.
    scientist_rows: Vec<TraitRow>,
    scientist_values: Vec<Trait>,
    /// The scientist domain's token taken before that walk.
    scientists: Option<(u64, u64)>,
    /// A quiet hour's published scientist rows, held in place of
    /// `scientist_rows` once complete.
    fresh_rows: Vec<TraitRow>,
    fresh_values: Vec<Trait>,
    /// A sampled character's fresh read.
    scratch: Scratch,
    /// Names the CharacterRoleTraits table a quiet hour publishes from the
    /// held rows and scientist rows: a fresh value whenever either changes;
    /// zero for none.
    content: u64,
    /// The operative links of the latest walk ([`Operatives`]).
    operatives: Operatives,
}

/// A content version no other store has held.
fn fresh_content() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// What carries an operative link: the certificates of the unit leader its
/// reference resolves to, from the unit leader domain (owner, skill,
/// category, names and codename writers, [`crate::unit_leader_changes`])
/// and the trait domain (its assigned traits,
/// [`crate::unit_leader_trait_changes`]); both see the leader constructors
/// and destructor. A CharacterOperatives row reads nothing else of the
/// leader, and the skill level and trait ordinals it reads are the content's.
pub(crate) trait LeaderChanges: Sync {
    /// Both certificates of `leader`, registering it; `None` when either
    /// domain cannot observe it.
    fn certificate(&self, leader: usize) -> Option<[(u64, u64); 2]>;
    /// A slice read differs from a carried row: neither domain carries.
    fn mismatch(&self);
}

/// The hooks' domains.
pub(crate) struct LeaderObserver;
impl LeaderChanges for LeaderObserver {
    fn certificate(&self, leader: usize) -> Option<[(u64, u64); 2]> {
        use crate::unit_leader_changes::Changes;
        Some([
            crate::unit_leader_changes::Observer.sequence(leader)?,
            crate::unit_leader_trait_changes::Observer.sequence(leader)?,
        ])
    }
    fn mismatch(&self) {
        use crate::unit_leader_changes::Changes;
        crate::unit_leader_changes::Observer.mismatch();
        crate::unit_leader_trait_changes::Observer.mismatch();
    }
}

/// Tests: one domain stands in for both.
#[cfg(test)]
impl LeaderChanges for crate::unit_leader_changes::Watches {
    fn certificate(&self, leader: usize) -> Option<[(u64, u64); 2]> {
        use crate::unit_leader_changes::Changes;
        let sequence = self.sequence(leader)?;
        Some([sequence; 2])
    }
    fn mismatch(&self) {
        crate::unit_leader_changes::Changes::mismatch(self);
    }
}

/// One held operative link: the occupant's census index and reference, the
/// object it resolved to with its certificates taken before the read, and
/// the row with its definitions and text.
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

/// One walk's operative links, held for the next walk ([`LeaderChanges`]).
#[derive(Default)]
struct Operatives {
    /// By census index.
    held: Vec<HeldOperative>,
    definitions: Vec<u32>,
    text: Vec<u8>,
    /// This walk's links, held again.
    next: Vec<HeldOperative>,
    next_definitions: Vec<u32>,
    next_text: Vec<u8>,
    /// A fresh read's text.
    scratch: Vec<u8>,
    counts: OperativeCounts,
}

/// One walk's operative links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OperativeCounts {
    pub carried: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
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

    /// Before a walk's first link.
    fn begin(&mut self) {
        self.next.clear();
        self.next_definitions.clear();
        self.next_text.clear();
        self.counts = OperativeCounts::default();
    }

    /// After the walk: its links are held when the CharacterOperatives table
    /// completed; otherwise nothing is.
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

    /// The census occurrence `index`'s operative link: carried from its held
    /// read while the leader's certificates hold and `check` does not cover
    /// the character, read afresh (and compared, when it was carryable)
    /// otherwise; held again either way when it can be certified and its
    /// read refused no name.
    #[allow(clippy::too_many_arguments)]
    fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        index: usize,
        occupant: &Occupant,
        walk: &mut CensusWalk<'_>,
        leaders: &dyn LeaderChanges,
        check: Check,
    ) {
        if occupant.operative == 0 {
            return;
        }
        // Taken before any read of this link.
        let held = self.holding(index, occupant.operative).cloned();
        let held_certificate = held
            .as_ref()
            .and_then(|held| leaders.certificate(held.object));
        let carryable = held
            .as_ref()
            .is_some_and(|held| held_certificate == Some(held.certificate));
        if let Some(held) = held.as_ref().filter(|_| carryable)
            && !check.covers(u64::from(occupant.character))
        {
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
        // A refused name is not the leader's: the row is neither compared
        // nor held, and the next walk reads it afresh as an uncarried walk
        // would.
        let whole = row.name_reason.is_none() && row.codename_reason.is_none();
        if let Some(held) = held.as_ref().filter(|_| carryable && whole) {
            self.counts.verified += 1;
            let same = held.row == row
                && self.definitions[held.definitions.clone()] == *definitions
                && self.text[held.text.clone()] == text[..];
            if !same {
                self.counts.mismatches += 1;
                leaders.mismatch();
            }
        }
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

    /// Holds a link for the next walk: its own definitions and text, or the
    /// held ones. Storage is advisory: a link it cannot hold is read next
    /// walk.
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

/// Storage of a sampled character's fresh read.
#[derive(Default)]
struct Scratch {
    rows: Vec<TraitRow>,
    values: Vec<Trait>,
    operatives: Vec<OperativeRow>,
    definitions: Vec<u32>,
    bytes: Vec<u8>,
}

impl Scratch {
    fn measure(&self) -> usize {
        self.rows.capacity() * size_of::<TraitRow>()
            + self.values.capacity() * size_of::<Trait>()
            + self.operatives.capacity() * size_of::<OperativeRow>()
            + self.definitions.capacity() * size_of::<u32>()
            + self.bytes.capacity()
    }
}

/// One walk's view of the store and the observation that certifies reuse.
pub(crate) struct RoleReuse<'a> {
    pub store: &'a mut RoleStore,
    pub changes: &'a dyn Changes,
    /// The carryable characters this walk reads afresh and compares.
    pub check: Check,
    /// The store content whose CharacterRoleTraits table the frame's buffers
    /// hold; a quiet hour publishing that table leaves them as they are.
    pub held: Option<u64>,
    /// What carries each operative link.
    pub leaders: &'a dyn LeaderChanges,
}

impl RoleStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Nothing held certifies anything: the next walk reads afresh.
    /// Allocation-free for the game thread.
    pub(crate) fn forget(&mut self) {
        self.forgotten = true;
        self.content = 0;
        self.operatives.forget();
    }

    /// Before a walk: no operative link of it is held yet.
    pub(crate) fn begin_operatives(&mut self) {
        self.operatives.begin();
    }

    /// After a walk: hold its operative links when CharacterOperatives
    /// completed.
    pub(crate) fn finish_operatives(&mut self, complete: bool) {
        self.operatives.finish(complete);
        let counts = self.operatives.counts;
        crate::character_changes::record_operatives(
            counts.carried,
            counts.read,
            counts.verified,
            counts.mismatches,
        );
    }

    #[cfg(test)]
    pub(crate) fn operative_counts(&self) -> OperativeCounts {
        self.operatives.counts
    }

    /// The content of the CharacterRoleTraits table the latest walk
    /// published from or into the held rows; zero for none.
    pub(crate) fn content(&self) -> u64 {
        self.content
    }

    /// As of the last finish, so the game thread need not measure.
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> Counts {
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
            + self.scratch.measure()
            + self.operatives.measure();
    }

    /// Holds one break's scientist rows (indexing `values`); `false` when
    /// the storage could not grow.
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

    /// Whether this walk may carry every held row: the latest full walk held
    /// every occupant of this census, and no observed writer ran since.
    fn quiet_hour(
        &self,
        scope: (usize, usize),
        token: Option<(u64, u64)>,
        census: &Admitted<'_>,
        check: Check,
    ) -> bool {
        !self.forgotten
            && self.scope == scope
            && check != Check::Every
            && census.generation != 0
            && token.is_some_and(|token| self.quiet == Some((token, census.generation)))
    }

    /// Keeps this walk's rows only when every trait row was read or carried
    /// completely, and publishes its counts.
    fn finish(&mut self, complete: bool) {
        self.content = if complete { fresh_content() } else { 0 };
        if complete {
            self.current
                .entries
                .sort_unstable_by_key(|entry| entry.certificate.pointer);
            // A pointer occurring twice names no single held read.
            let entries = &mut self.current.entries;
            let mut index = 0;
            while index < entries.len() {
                let pointer = entries[index].certificate.pointer;
                let end = entries[index..]
                    .iter()
                    .position(|entry| entry.certificate.pointer != pointer)
                    .map_or(entries.len(), |offset| index + offset);
                if end - index > 1 {
                    entries.drain(index..end);
                } else {
                    index = end;
                }
            }
        } else {
            self.current.clear();
        }
        self.previous.clear();
        self.measure();
        crate::character_changes::record(
            crate::character_changes::Table::CharacterRoleTraits,
            &self.counts,
        );
    }

    /// A quiet hour's slice: each carried character `check` covers is read
    /// afresh from the census, as a full walk reads it, and compared with
    /// its held rows.
    fn sample(
        &mut self,
        memory: &impl Memory,
        base: usize,
        census: &Admitted<'_>,
        check: Check,
    ) -> Slice {
        if !check.any() {
            return Slice::Same(0);
        }
        let Self {
            current, scratch, ..
        } = self;
        let mut walk = CensusWalk::growing(GrowingBuffers {
            traits: &mut scratch.rows,
            trait_values: &mut scratch.values,
            operatives: &mut scratch.operatives,
            definitions: &mut scratch.definitions,
            bytes: &mut scratch.bytes,
        });
        let mut compared = 0;
        let mut page = [(0_usize, Occupant::default()); CENSUS_PAGE];
        let mut headers = [[0_u8; 16]; CENSUS_PAGE * 2];
        let mut sentinels = [0_u8; CENSUS_PAGE * 2 * MAP_SENTINEL];
        let mut covered = census.covered(check);
        loop {
            let mut len = 0;
            for next in covered.by_ref().take(CENSUS_PAGE) {
                let occupant = census.occurrence(next).occupant();
                page[len] = (next, occupant);
                headers[len * 2] = occupant.leaders;
                headers[len * 2 + 1] = occupant.advisors;
                len += 1;
            }
            if len == 0 {
                break;
            }
            walk.sentinels(
                memory,
                &headers[..len * 2],
                &mut sentinels[..len * 2 * MAP_SENTINEL],
            );
            for (slot, (_, occupant)) in page[..len].iter().enumerate() {
                let Some(held) = current.position(occupant.pointer) else {
                    return Slice::Failed;
                };
                let at = slot * 2 * MAP_SENTINEL;
                let sentinel = |map: usize| -> [u8; MAP_SENTINEL] {
                    sentinels[at + map * MAP_SENTINEL..][..MAP_SENTINEL]
                        .try_into()
                        .unwrap()
                };
                let from = walk.stored();
                walk.roles(memory, base, occupant, [sentinel(0), sentinel(1)]);
                if !walk.traits_complete() {
                    return Slice::Failed;
                }
                compared += 1;
                let (rows, values) = walk.stored_since(from);
                let old = &current.rows[current.entries[held].rows.clone()];
                if !same(rows, values, old, &current.values) {
                    return Slice::Differs(compared);
                }
            }
        }
        Slice::Same(compared)
    }

    /// A quiet hour's rows: every held row in runs, with the scientist row
    /// and operative link of each occupant that has one after its own rows,
    /// in roster order as a full walk appends them. The operative link is
    /// carried or read afresh ([`Operatives::visit`]); the scientist rows are carried when `scientists` (the
    /// scientist token matched), except those `check` covers and every one
    /// when `verify`, which are read afresh and compared. The scientist rows
    /// published are held again, so a quiet hour after a scientist write
    /// carries the rows it read.
    #[allow(clippy::too_many_arguments)]
    fn carry_all(
        &mut self,
        memory: &impl Memory,
        base: usize,
        census: &Admitted<'_>,
        walk: &mut CensusWalk<'_>,
        scientists: bool,
        verify: bool,
        check: Check,
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
            if scientists && !verify && !check.covers(u64::from(occupant.character)) {
                walk.carry(rows, scientist_values);
                counts.carried += 1;
            } else {
                walk.scientist(memory, base, &occupant);
                if scientists && walk.traits_complete() {
                    counts.verified += 1;
                    let (fresh, values) = walk.stored_since(from);
                    if !same(fresh, values, rows, scientist_values) {
                        counts.mismatches += 1;
                    }
                }
            }
            // Held again as published.
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
            operatives.visit(memory, base, brk.index, &occupant, walk, leaders, check);
            at = brk.end;
        }
        walk.carry(&held.rows[at..], &held.values);
        if counts.held {
            std::mem::swap(scientist_rows, fresh_rows);
            std::mem::swap(scientist_values, fresh_values);
        }
        // Scientist rows read afresh may differ from the held ones.
        if counts.carried as usize != self.breaks.len() || !counts.held {
            self.content = fresh_content();
        }
        counts
    }

    /// A quiet hour whose CharacterRoleTraits table is the held content's
    /// and already in the frame: whether every scientist row would be
    /// carried, and if so the operative link of each break, carried or read
    /// afresh ([`Operatives::visit`]).
    fn operatives_only(
        &mut self,
        memory: &impl Memory,
        base: usize,
        census: &Admitted<'_>,
        walk: &mut CensusWalk<'_>,
        check: Check,
        leaders: &dyn LeaderChanges,
    ) -> bool {
        let Self {
            breaks, operatives, ..
        } = self;
        if breaks
            .iter()
            .any(|brk| check.covers(u64::from(census.occurrence(brk.index).occupant().character)))
        {
            return false;
        }
        for brk in breaks.iter() {
            let occupant = census.occurrence(brk.index).occupant();
            operatives.visit(memory, base, brk.index, &occupant, walk, leaders, check);
        }
        true
    }
}

/// One break of a held walk: the census occurrence with a scientist or an
/// operative link, the end of the held rows up to and including its own, and
/// its scientist rows in [`RoleStore::scientist_rows`].
#[derive(Clone, Debug)]
struct Break {
    index: usize,
    end: usize,
    scientist: Range<usize>,
}

/// A quiet hour's scientist rows.
#[derive(Clone, Copy, Debug, Default)]
struct ScientistCounts {
    carried: u64,
    verified: u64,
    mismatches: u64,
    /// Every break's published scientist rows are held again.
    held: bool,
}

/// What a quiet hour's sampled slice found.
enum Slice {
    /// Every sampled character read as held; how many were compared.
    Same(u64),
    /// A sampled character differs from its held rows.
    Differs(u64),
    /// A sampled read did not complete; the hour walks afresh.
    Failed,
}

/// What [`visit`] wrote into the walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Visited {
    /// Both tables.
    Walked,
    /// Only CharacterOperatives: the CharacterRoleTraits table is the held
    /// content's ([`RoleReuse::held`]).
    Held,
}

/// Every occupant of `census` into `walk` in roster order: each character's
/// country-leader and advisor rows carried when `reuse` proves them current
/// and read afresh otherwise, then its scientist row and operative link.
pub(crate) fn visit(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: Admitted<'_>,
    walk: &mut CensusWalk<'_>,
    mut reuse: Option<RoleReuse<'_>>,
) -> Visited {
    // Taken before any registration or read this walk certifies.
    let token = reuse.as_ref().and_then(|reuse| reuse.changes.quiet());
    let scientist_token = reuse.as_ref().and_then(|reuse| reuse.changes.scientists());
    let mut sampled = Counts::default();
    if let Some(reuse) = reuse.as_mut()
        && reuse
            .store
            .quiet_hour((base, game_state), token, &census, reuse.check)
    {
        match reuse.store.sample(memory, base, &census, reuse.check) {
            Slice::Same(compared) => {
                let scientists =
                    scientist_token.is_some() && reuse.store.scientists == scientist_token;
                let verify = reuse.changes.scientist_verify();
                if scientists
                    && !verify
                    && reuse.store.content != 0
                    && reuse.held == Some(reuse.store.content)
                    && reuse.store.operatives_only(
                        memory,
                        base,
                        &census,
                        walk,
                        reuse.check,
                        reuse.leaders,
                    )
                {
                    crate::roster_changes::record(
                        crate::roster_changes::Table::CharacterRoleTraits,
                        &crate::roster_changes::Counts {
                            quiet: true,
                            ..Default::default()
                        },
                    );
                    let store = &mut *reuse.store;
                    store.counts = Counts {
                        reused: census.len() as u64 - compared,
                        read: compared,
                        verified: compared,
                        mismatches: 0,
                        quiet: true,
                    };
                    store.measure();
                    crate::character_changes::record(
                        crate::character_changes::Table::CharacterRoleTraits,
                        &store.counts,
                    );
                    return Visited::Held;
                }
                let counts = reuse.store.carry_all(
                    memory,
                    base,
                    &census,
                    walk,
                    scientists,
                    verify,
                    reuse.check,
                    reuse.leaders,
                );
                if counts.mismatches != 0 {
                    // A scientist write no hook saw: the fresh rows are
                    // published, and scientists are read afresh from now on.
                    reuse.changes.scientist_mismatch();
                }
                // The published scientist rows were read or carried after
                // this hour's token was taken.
                reuse.store.scientists = scientist_token.filter(|_| counts.held);
                crate::roster_changes::record(
                    crate::roster_changes::Table::CharacterRoleTraits,
                    &crate::roster_changes::Counts {
                        quiet: scientists && !verify,
                        read: !scientists || verify,
                        verified: counts.verified,
                        mismatches: counts.mismatches,
                    },
                );
                if walk.traits_complete() {
                    let store = &mut *reuse.store;
                    store.counts = Counts {
                        reused: census.len() as u64 - compared,
                        read: compared,
                        verified: compared,
                        mismatches: 0,
                        quiet: true,
                    };
                    store.measure();
                    crate::character_changes::record(
                        crate::character_changes::Table::CharacterRoleTraits,
                        &store.counts,
                    );
                    return Visited::Walked;
                }
                // A scientist row failed or the rows did not fit: the
                // table publishes that outcome, and nothing is held.
                reuse.store.forget();
                reuse.store.begin((base, game_state));
                reuse.store.finish(false);
                return Visited::Walked;
            }
            Slice::Differs(compared) => {
                // A missed write: nothing certifies reuse any more, and this
                // hour reads and publishes every character afresh.
                sampled.verified = compared;
                sampled.mismatches = 1;
                reuse.changes.mismatch();
            }
            Slice::Failed => {}
        }
    }
    if let Some(reuse) = reuse.as_mut() {
        reuse.store.begin((base, game_state));
    }
    // Every occupant so far holds a certificate.
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
            // Registration precedes every read it certifies.
            let plan = reuse
                .as_ref()
                .map_or_else(Plan::default, |reuse| plan(reuse, &occupant));
            if !plan.carry {
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
                (Some(reuse), Some(index)) if plan.carry => {
                    let previous = &reuse.store.previous;
                    let rows = previous.entries[index].rows.clone();
                    walk.carry(&previous.rows[rows], &previous.values);
                    reuse.store.counts.reused += 1;
                }
                (reuse, held) => {
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
                        // A failed fresh read fails the table and holds nothing.
                        if let Some(index) = held.filter(|_| walk.traits_complete()) {
                            reuse.store.counts.verified += 1;
                            let previous = &reuse.store.previous;
                            let (rows, values) = walk.stored_since(from);
                            let old = &previous.rows[previous.entries[index].rows.clone()];
                            if !same(rows, values, old, &previous.values) {
                                reuse.store.counts.mismatches += 1;
                                reuse.changes.mismatch();
                            }
                        }
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
                Some(reuse) => reuse.store.operatives.visit(
                    memory,
                    base,
                    index,
                    occupant,
                    walk,
                    reuse.leaders,
                    reuse.check,
                ),
                None => walk.operative(memory, base, occupant),
            }
            index += 1;
        }
    }
    if let Some(reuse) = reuse {
        let store = reuse.store;
        let complete = walk.traits_complete();
        store.counts.verified += sampled.verified;
        store.counts.mismatches += sampled.mismatches;
        store.finish(complete);
        // Every occupant held under one certificate each (a duplicated
        // pointer dropped its entries and is read afresh), with this walk's
        // census and no mismatch: a quiet hour carries the whole table.
        store.quiet = token
            .filter(|_| {
                complete
                    && held_all
                    && census.generation != 0
                    && reuse.check != Check::Every
                    && store.counts.mismatches == 0
                    && store.current.entries.len() == census.len()
            })
            .map(|token| (token, census.generation));
        if store.quiet.is_none() {
            store.breaks.clear();
            store.scientist_rows.clear();
            store.scientist_values.clear();
        }
        // Every break's scientist rows were held with the walk.
        store.scientists = store.quiet.and(scientist_token);
        crate::roster_changes::record(
            crate::roster_changes::Table::CharacterRoleTraits,
            &crate::roster_changes::Counts {
                read: true,
                ..Default::default()
            },
        );
    }
    Visited::Walked
}

/// What a walk does with one occupant.
#[derive(Clone, Copy, Default)]
struct Plan {
    certificate: Option<Certificate>,
    /// The previous walk's entry this certificate matches.
    held: Option<usize>,
    /// Carry that entry instead of reading.
    carry: bool,
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
    Plan {
        certificate,
        held,
        carry: held.is_some() && !reuse.check.covers(u64::from(occupant.character)),
    }
}

/// Fresh rows equal held ones, positions aside.
fn same(rows: &[TraitRow], values: &[Trait], held: &[TraitRow], held_values: &[Trait]) -> bool {
    rows.len() == held.len()
        && rows.iter().zip(held).all(|(row, old)| {
            (row.character, row.role, row.len) == (old.character, old.role, old.len)
                && values[row.start..row.start + row.len]
                    == held_values[old.start..old.start + old.len]
        })
}
