//! CountryLeaderRoles rows held between hours ([`crate::character_changes`]).
//!
//! The leader role job walks the roster job's admitted census. A character's rows are carried,
//! without reading its map, roles or ideologies, only when its census pointer, id and
//! country-leader map header (head node and size) equal the held read's and no hooked writer
//! touched the character since that read registered it; the country comes from this hour's census.
//! The map, its roles' parent links and current ideologies (`ccountry::LEADER_IDEOLOGY`) are
//! written only at hooked sites ([`engine_mutations::character_role_mutations`],
//! [`engine_mutations::advisor_mutations`]); set_country_leader_ideology names no character and revokes every
//! certificate, and a `CIdeology` token is a database value. Without an admitted census the job
//! walks the roster and the cache forgets its rows.
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
//! read returns.
use super::*;
use crate::character_changes::{Changes, Counts, Table};
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
        let stored = engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.entries, 1)?;
            engine_allocation::reserve(&mut self.rows, rows.len())?;
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
            counts: Counts::default(),
        }
    }
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
        if engine_allocation::reserve(rows, (start + room).saturating_sub(rows.len())).is_err() {
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
                engine_native::walk::Buffer::Exact(&mut rows[start..]),
            ))
        })
        .unwrap_or_else(|reason| Outcome::Unavailable { reason }))
    }

    fn plan(&self, changes: &dyn Changes, occurrence: &census::Occurrence) -> Plan {
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
        Plan { certificate, held }
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
    ) {
        use census::{SENTINEL, SENTINEL_BATCH};
        let scope = (base, game_state);
        // Taken before any registration or read this capture certifies.
        let token = changes.quiet();
        if self.finished
            && self.scope == scope
            && census.generation != 0
            && token.is_some_and(|token| self.quiet == Some((token, census.generation)))
        {
            self.counts = Counts {
                reused: census.len() as u64,
                read: 0,
                quiet: true,
            };
            crate::character_changes::record(Table::CountryLeaderRoles, &self.counts);
            return;
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
                let plan = self.plan(changes, &occurrence);
                slots[index - next] = usize::MAX;
                if plan.held.is_none()
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
                match plan.held {
                    Some(held) => {
                        let rows = self.previous.entries[held].rows.clone();
                        if unread == 0 {
                            if engine_allocation::reserve(&mut self.rows, rows.len()).is_err() {
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
            crate::extended::characters::sort_dropping_repeated(
                &mut self.current.entries,
                |entry| entry.certificate.pointer,
            );
            self.finished = true;
            // Carried whole only when every occurrence holds a certificate;
            // a duplicated pointer dropped its entries and is read afresh.
            self.quiet = token
                .filter(|_| {
                    held_all
                        && census.generation != 0
                        && self.current.entries.len() == census.len()
                })
                .map(|token| (token, census.generation));
        } else {
            self.current.clear();
        }
        self.previous.clear();
        crate::character_changes::record(Table::CountryLeaderRoles, &self.counts);
    }

    /// Decode worker: this capture's rows into the pooled table.
    pub(super) fn decode(&self, captured: &mut Captured) {
        let len = self.outcome.len();
        let rows = &mut captured.leaders.rows.rows;
        if let Err(reason) = engine_native::walk::Buffer::Growing(rows).ensure(len) {
            captured
                .leaders
                .primary(ReadResult::rows(Outcome::Unavailable { reason }));
            return;
        }
        rows[..len].copy_from_slice(&self.rows[..len]);
        captured.leaders.primary(ReadResult::rows(self.outcome));
    }
}
