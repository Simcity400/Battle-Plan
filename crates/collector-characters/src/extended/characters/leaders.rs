use super::*;
use collector_observers::character_changes::{Changes, Counts, Table};
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
        collector_primitives::raw_memory::vector_bytes(&self.entries)
            + collector_primitives::raw_memory::vector_bytes(&self.rows)
    }
}

#[derive(Debug)]
pub struct LeaderCache {
    current: Held,
    previous: Held,
    rows: Vec<ccountry_leader::Row>,
    pub(super) outcome: Outcome,
    scope: (usize, usize),
    finished: bool,
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
    pub(super) fn forget(&mut self) {
        self.finished = false;
        self.quiet = None;
    }

    pub(super) fn allocated_bytes(&self) -> u64 {
        self.current.allocated()
            + self.previous.allocated()
            + collector_primitives::raw_memory::vector_bytes(&self.rows)
    }

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
            let memory = Gathered {
                inner: &view,
                sentinel: gathered.zip(head).map(|(bytes, (head, _))| (head, bytes)),
            };
            Ok(ccountry_leader::walk_entry(
                &memory,
                base,
                entry,
                engine_native_memory::walk::Buffer::Exact(&mut rows[start..]),
            ))
        })
        .unwrap_or_else(|reason| Outcome::Unavailable { reason }))
    }

    fn plan(&self, changes: &dyn Changes, occurrence: &census::Occurrence) -> Plan {
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
            collector_observers::character_changes::record(Table::CountryLeaderRoles, &self.counts);
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
        self.previous = Held::default();
        collector_observers::character_changes::record(Table::CountryLeaderRoles, &self.counts);
    }

    pub(super) fn decode(&self, captured: &mut Captured) {
        let len = self.outcome.len();
        let rows = &mut captured.leaders.rows.rows;
        if let Err(reason) = engine_native_memory::walk::Buffer::Growing(rows).ensure(len) {
            captured
                .leaders
                .primary(ReadResult::rows(Outcome::Unavailable { reason }));
            return;
        }
        rows[..len].copy_from_slice(&self.rows[..len]);
        captured.leaders.primary(ReadResult::rows(self.outcome));
    }
}
