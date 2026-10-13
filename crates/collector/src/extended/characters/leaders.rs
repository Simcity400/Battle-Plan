//! CountryLeaderRoles rows held between hours ([`crate::character_changes`]).
//!
//! The leader role job walks the roster job's admitted census. A character's
//! rows are carried, without reading its map, roles or ideologies, only when
//! its census pointer, id and country-leader map header (head node and size)
//! equal the held read's and no hooked writer touched the character since that
//! read registered it; the country comes from this hour's census. The map,
//! its roles' parent links and current ideologies (`CCountryLeader+0x140`)
//! are written only at hooked sites ([`engine::character_role_mutations`],
//! [`engine::advisor_mutations`]); set_country_leader_ideology names no
//! character and revokes every certificate, and a `CIdeology` token is a
//! database value. Verify mode reads every carryable character afresh,
//! publishes the fresh rows and poisons reuse on a difference. Without an
//! admitted census the job walks the roster and the cache forgets its rows.
//!
//! A fresh empty map is checked from its batch-gathered sentinel, as the
//! census acquisition checks it; every other fresh character is read by
//! `ccountry_leader::walk_entry` through the census view.
//!
//! A quiet hour carries the whole table without a lookup: when the last
//! complete capture held every occurrence, the character domain's quiet
//! token ([`Changes::quiet`]) equals the one taken before that capture's
//! first registration, and the census has that capture's generation
//! ([`census::Admitted::generation`]), no observed writer ran and every
//! occurrence, identity, country and map header is the one read then, so
//! each held certificate still matches and the held rows are what a fresh
//! read returns. Only the sampled slice is read, from the census, and
//! compared; a difference poisons reuse and the hour reads every character
//! afresh, and a slice read that fails reads every character too.
use super::*;
use crate::{
    character_changes::{Changes, Counts, Table},
    sampled_verify::Check,
};
use std::ops::Range;

const ROWS_ALLOCATION: &str = "country leader row allocation failed";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Certificate {
    epoch: u64,
    sequence: u64,
    pointer: usize,
    id: u32,
    leaders: [u8; 16],
}

#[derive(Debug)]
struct Entry {
    certificate: Certificate,
    rows: Range<usize>,
}

#[derive(Debug, Default)]
struct Held {
    /// Sorted by character pointer once the capture finishes.
    entries: Vec<Entry>,
    rows: Vec<ccountry_leader::Row>,
}

impl Held {
    fn clear(&mut self) {
        self.entries.clear();
        self.rows.clear();
    }

    fn position(&self, pointer: usize) -> Option<usize> {
        self.entries
            .binary_search_by_key(&pointer, |entry| entry.certificate.pointer)
            .ok()
    }

    /// `false` when storage could not grow, which only withholds the
    /// certificate.
    fn hold(&mut self, certificate: Certificate, rows: &[ccountry_leader::Row]) -> bool {
        let stored = engine::allocation::advisory(|| {
            engine::allocation::reserve(&mut self.entries, 1)?;
            engine::allocation::reserve(&mut self.rows, rows.len())?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        let first = self.rows.len();
        self.rows.extend_from_slice(rows);
        self.entries.push(Entry {
            certificate,
            rows: first..self.rows.len(),
        });
        true
    }

    fn allocated(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.entries) + crate::raw_memory::vector_bytes(&self.rows)
    }
}

#[derive(Debug)]
pub(crate) struct LeaderCache {
    current: Held,
    previous: Held,
    /// This capture's rows in roster order.
    rows: Vec<ccountry_leader::Row>,
    pub(super) outcome: Outcome,
    scope: (usize, usize),
    /// The last capture finished; otherwise nothing it held certifies.
    finished: bool,
    /// What lets a quiet hour carry `rows` whole: the domain's quiet token
    /// before the last complete capture and its census generation, when it
    /// held every occurrence.
    quiet: Option<((u64, u64), u64)>,
    /// A sampled character's fresh rows.
    scratch: Vec<ccountry_leader::Row>,
    pub(super) counts: Counts,
}

impl Default for LeaderCache {
    fn default() -> Self {
        Self {
            current: Held::default(),
            previous: Held::default(),
            rows: Vec::new(),
            outcome: Outcome::Rows { len: 0 },
            scope: (0, 0),
            finished: false,
            quiet: None,
            scratch: Vec::new(),
            counts: Counts::default(),
        }
    }
}

/// What a quiet hour's sampled slice found.
enum Slice {
    /// Every sampled character read as held; how many were compared.
    Same(u64),
    /// A sampled character differs from its held rows.
    Differs(u64),
    /// A sampled read did not complete; the hour reads afresh.
    Failed,
}

/// A view that serves the one map sentinel this callback already gathered;
/// every other read goes to `inner`.
struct Gathered<'a, M: Memory> {
    inner: &'a M,
    sentinel: Option<(usize, [u8; census::SENTINEL])>,
}

impl<M: Memory> Memory for Gathered<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        match self.sentinel {
            Some((head, bytes)) if address == head && output.len() == bytes.len() => {
                output.copy_from_slice(&bytes);
                true
            }
            _ => self.inner.copy(address, output),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Plan {
    certificate: Option<Certificate>,
    held: Option<usize>,
    carry: bool,
}

impl LeaderCache {
    /// Nothing held certifies anything: the next capture reads afresh.
    pub(super) fn forget(&mut self) {
        self.finished = false;
        self.quiet = None;
    }

    pub(super) fn allocated_bytes(&self) -> u64 {
        self.current.allocated()
            + self.previous.allocated()
            + crate::raw_memory::vector_bytes(&self.rows)
            + crate::raw_memory::vector_bytes(&self.scratch)
    }

    /// One occurrence's rows read afresh into `rows[start..]`, as the walk
    /// reads them: an empty map from its gathered sentinel, any other through
    /// the census view, which serves the gathered sentinel. With `room` its
    /// map's size in rows is reserved for it. `Err` is an allocation failure.
    #[allow(clippy::too_many_arguments)]
    fn read_fresh(
        memory: &impl Memory,
        base: usize,
        historical: usize,
        occurrence: &census::Occurrence,
        gathered: Option<[u8; census::SENTINEL]>,
        rows: &mut Vec<ccountry_leader::Row>,
        start: usize,
        room: bool,
    ) -> Result<Outcome, &'static str> {
        let head = census::map_head(occurrence.leaders());
        let empty = gathered.zip(head).is_some_and(|(bytes, (head, count))| {
            count == 0 && tree_sentinel(head, 0, &bytes).is_some()
        });
        if empty {
            return Ok(Outcome::Rows { len: 0 });
        }
        let count = head.map_or(0, |(_, count)| count as usize);
        let room = if room { count } else { 0 };
        if engine::allocation::reserve(rows, (start + room).saturating_sub(rows.len())).is_err() {
            return Err(ROWS_ALLOCATION);
        }
        rows.resize(start + room, ccountry_leader::Row::default());
        Ok(ccharacter::View::acquired(
            memory,
            base,
            occurrence.identity(historical),
            occurrence.fields([false, true, false, false]),
        )
        .and_then(|view| {
            let entry = view.entry()?;
            // The walk's own sentinel read is served from the batch that
            // already copied it.
            let memory = Gathered {
                inner: &view,
                sentinel: gathered.zip(head).map(|(bytes, (head, _))| (head, bytes)),
            };
            Ok(ccountry_leader::walk_entry(
                &memory,
                base,
                entry,
                &mut rows[start..],
            ))
        })
        .unwrap_or_else(|reason| Outcome::Unavailable { reason }))
    }

    /// A quiet hour's slice: each held character `check` covers is read
    /// afresh from the census, its sentinels gathered in batches as the walk
    /// gathers them, and compared with its held rows.
    fn sample(
        &mut self,
        memory: &impl Memory,
        base: usize,
        census: census::Admitted<'_>,
        check: Check,
    ) -> Slice {
        use census::{SENTINEL, SENTINEL_BATCH};
        if !check.any() {
            return Slice::Same(0);
        }
        let mut compared = 0;
        let mut picked = [0_usize; SENTINEL_BATCH];
        let mut slots = [usize::MAX; SENTINEL_BATCH];
        let mut heads = [0; SENTINEL_BATCH];
        let mut sentinels = [0; SENTINEL_BATCH * SENTINEL];
        let mut next = 0;
        while next < census.len() {
            let (mut len, mut staged) = (0, 0);
            while len < SENTINEL_BATCH && next < census.len() {
                let occurrence = census.occurrence(next);
                if check.covers(u64::from(occurrence.id())) {
                    picked[len] = next;
                    slots[len] = usize::MAX;
                    if let Some((head, _)) = census::map_head(occurrence.leaders())
                        && !census::inside_view(occurrence.pointer, head, SENTINEL)
                    {
                        heads[staged] = head;
                        slots[len] = staged;
                        staged += 1;
                    }
                    len += 1;
                }
                next += 1;
            }
            let completed = census::gather_sentinels(memory, &heads[..staged], &mut sentinels);
            for (&index, &slot) in picked[..len].iter().zip(&slots[..len]) {
                let occurrence = census.occurrence(index);
                let Some(held) = self.current.position(occurrence.pointer) else {
                    return Slice::Failed;
                };
                let gathered = (slot < completed).then(|| -> [u8; SENTINEL] {
                    sentinels[slot * SENTINEL..][..SENTINEL].try_into().unwrap()
                });
                self.scratch.clear();
                let outcome = Self::read_fresh(
                    memory,
                    base,
                    census.historical,
                    &occurrence,
                    gathered,
                    &mut self.scratch,
                    0,
                    true,
                );
                let Ok(Outcome::Rows { len: fresh }) = outcome else {
                    return Slice::Failed;
                };
                compared += 1;
                let country = u32::from_le_bytes(occurrence.country());
                let old = &self.current.rows[self.current.entries[held].rows.clone()];
                let same = old.len() == fresh
                    && old
                        .iter()
                        .zip(&self.scratch[..fresh])
                        .all(|(old, new)| ccountry_leader::Row { country, ..*old } == *new);
                if !same {
                    return Slice::Differs(compared);
                }
            }
        }
        Slice::Same(compared)
    }

    fn plan(&self, changes: &dyn Changes, check: Check, occurrence: &census::Occurrence) -> Plan {
        // Registration precedes every read it certifies.
        let certificate = changes
            .sequence(occurrence.pointer)
            .map(|(epoch, sequence)| Certificate {
                epoch,
                sequence,
                pointer: occurrence.pointer,
                id: occurrence.id(),
                leaders: occurrence.leaders(),
            });
        let held = certificate.and_then(|certificate| {
            let index = self.previous.position(certificate.pointer)?;
            (self.previous.entries[index].certificate == certificate).then_some(index)
        });
        Plan {
            certificate,
            held,
            carry: held.is_some() && !check.covers(u64::from(occurrence.id())),
        }
    }

    /// One hour's rows from the admitted census, carrying each certified
    /// character's held rows with its census country.
    pub(super) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        census: census::Admitted<'_>,
        changes: &dyn Changes,
        check: Check,
    ) {
        use census::{SENTINEL, SENTINEL_BATCH};
        let scope = (base, game_state);
        // Taken before any registration or read this capture certifies.
        let token = changes.quiet();
        let mut sampled = Counts::default();
        if self.finished
            && self.scope == scope
            && check != Check::Every
            && census.generation != 0
            && token.is_some_and(|token| self.quiet == Some((token, census.generation)))
        {
            match self.sample(memory, base, census, check) {
                Slice::Same(compared) => {
                    self.counts = Counts {
                        reused: census.len() as u64 - compared,
                        read: compared,
                        verified: compared,
                        mismatches: 0,
                        quiet: true,
                    };
                    crate::character_changes::record(Table::CountryLeaderRoles, &self.counts);
                    return;
                }
                Slice::Differs(compared) => {
                    // A missed write: nothing certifies reuse any more, and
                    // this hour reads and publishes every character afresh.
                    sampled.verified = compared;
                    sampled.mismatches = 1;
                    changes.mismatch();
                }
                Slice::Failed => {}
            }
        }
        self.quiet = None;
        if !self.finished || self.scope != scope {
            self.current.clear();
            self.previous.clear();
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
        self.rows.clear();
        self.counts = Counts::default();
        self.scope = scope;
        self.finished = false;
        let mut failure = None;
        let mut unread = 0_u32;
        // Every occurrence so far holds a certificate.
        let mut held_all = true;
        let mut plans = [Plan::default(); SENTINEL_BATCH];
        let mut slots = [usize::MAX; SENTINEL_BATCH];
        let mut heads = [0; SENTINEL_BATCH];
        let mut sentinels = [0; SENTINEL_BATCH * SENTINEL];
        let mut next = 0;
        'batches: while next < census.len() {
            let end = census.len().min(next + SENTINEL_BATCH);
            let mut staged = 0;
            for index in next..end {
                let occurrence = census.occurrence(index);
                let plan = self.plan(changes, check, &occurrence);
                slots[index - next] = usize::MAX;
                if !plan.carry
                    && let Some((head, _)) = census::map_head(occurrence.leaders())
                    && !census::inside_view(occurrence.pointer, head, SENTINEL)
                {
                    heads[staged] = head;
                    slots[index - next] = staged;
                    staged += 1;
                }
                plans[index - next] = plan;
            }
            let completed = census::gather_sentinels(memory, &heads[..staged], &mut sentinels);
            for index in next..end {
                let occurrence = census.occurrence(index);
                let plan = plans[index - next];
                let country = u32::from_le_bytes(occurrence.country());
                let start = self.rows.len();
                match plan.held.filter(|_| plan.carry) {
                    Some(held) => {
                        let rows = self.previous.entries[held].rows.clone();
                        if unread == 0 {
                            if engine::allocation::reserve(&mut self.rows, rows.len()).is_err() {
                                failure = Some(ROWS_ALLOCATION);
                                break 'batches;
                            }
                            self.rows.extend(
                                self.previous.rows[rows]
                                    .iter()
                                    .map(|row| ccountry_leader::Row { country, ..*row }),
                            );
                        } else {
                            unread = unread.saturating_add(rows.len() as u32);
                        }
                        self.counts.reused += 1;
                    }
                    None => {
                        self.counts.read += 1;
                        let slot = slots[index - next];
                        let gathered = (slot < completed).then(|| -> [u8; SENTINEL] {
                            sentinels[slot * SENTINEL..][..SENTINEL].try_into().unwrap()
                        });
                        let outcome = match Self::read_fresh(
                            memory,
                            base,
                            census.historical,
                            &occurrence,
                            gathered,
                            &mut self.rows,
                            start,
                            unread == 0,
                        ) {
                            Ok(outcome) => outcome,
                            Err(reason) => {
                                failure = Some(reason);
                                break 'batches;
                            }
                        };
                        match outcome {
                            Outcome::Rows { len } => self.rows.truncate(start + len),
                            Outcome::Truncated { len, unread_rows } => {
                                self.rows.truncate(start + len);
                                unread = unread.saturating_add(unread_rows);
                            }
                            Outcome::Unavailable { reason } => {
                                self.rows.truncate(start);
                                failure = Some(reason);
                                break 'batches;
                            }
                        }
                        if let Some(held) = plan.held.filter(|_| unread == 0) {
                            self.counts.verified += 1;
                            let entry = &self.previous.entries[held];
                            let old = &self.previous.rows[entry.rows.clone()];
                            let same = old.len() == self.rows.len() - start
                                && old.iter().zip(&self.rows[start..]).all(|(old, new)| {
                                    ccountry_leader::Row { country, ..*old } == *new
                                });
                            if !same {
                                self.counts.mismatches += 1;
                                changes.mismatch();
                            }
                        }
                    }
                }
                if let Some(certificate) = plan.certificate
                    && unread == 0
                {
                    held_all &= self.current.hold(certificate, &self.rows[start..]);
                } else {
                    held_all = false;
                }
            }
            next = end;
        }
        let len = self.rows.len();
        self.outcome = match failure {
            Some(reason) => Outcome::Unavailable { reason },
            None if unread != 0 => Outcome::Truncated {
                len,
                unread_rows: unread,
            },
            None => Outcome::Rows { len },
        };
        if matches!(self.outcome, Outcome::Rows { .. }) {
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
            self.finished = true;
            // Carried whole only when every occurrence holds a certificate;
            // a duplicated pointer dropped its entries and is read afresh.
            self.quiet = token
                .filter(|_| {
                    held_all
                        && census.generation != 0
                        && check != Check::Every
                        && self.counts.mismatches == 0
                        && self.current.entries.len() == census.len()
                })
                .map(|token| (token, census.generation));
        } else {
            self.current.clear();
        }
        self.previous.clear();
        self.counts.verified += sampled.verified;
        self.counts.mismatches += sampled.mismatches;
        crate::character_changes::record(Table::CountryLeaderRoles, &self.counts);
    }

    /// Decode worker: this capture's rows into the pooled table.
    pub(super) fn decode(&self, captured: &mut Captured) {
        let len = self.outcome.len();
        let rows = &mut captured.leaders.rows.rows;
        if let Err(reason) = engine::walk::Buffer::Growing(rows).ensure(len) {
            captured
                .leaders
                .primary(ReadResult::rows(Outcome::Unavailable { reason }));
            return;
        }
        rows[..len].copy_from_slice(&self.rows[..len]);
        captured.leaders.primary(ReadResult::rows(self.outcome));
    }
}
