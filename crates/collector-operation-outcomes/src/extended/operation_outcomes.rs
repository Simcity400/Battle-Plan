use crate::operation_outcomes as hooks;
use collector_primitives::table_capture::{grow_capacity, grow_vec};
use collector_projection::table_conversion::unavailable;
use engine_allocation::{copy_vec, map_slice, owned_vec};
use engine_native::walk::Outcome;
use replay_data::{rows::OperationCompletionRow, rows::TableId, rows::TableRows};
use replay_frame::{frame::TableInput, frame::TableView};
use std::sync::atomic::{AtomicUsize, Ordering};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::OperationCompletions];
const ASSEMBLED: &str = "operation completion observations assembled by spool";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    entries: usize,
    bytes: usize,
}

collector_primitives::capacity_max!(Capacities { entries, bytes });
impl Capacities {
    pub fn raise_for_countries(&mut self, slots: usize) {
        grow_capacity(&mut self.entries, slots);
    }
    pub fn raise_for_operations(&mut self, rows: usize) {
        grow_capacity(&mut self.entries, rows);
    }
    pub fn raise_for_descriptions(&mut self, bytes: usize) {
        grow_capacity(&mut self.bytes, bytes);
    }
    /// # Safety
    /// Run only while operation-log storage remains live for the active session (or an isolated
    /// test owning the global log) and `operation_outcomes::release` is excluded; once attached,
    /// concurrent growth calls rely on the log's internal `LOGS` and `byte_rings` locks.
    pub unsafe fn prepare(&self) {
        hooks::presize(self.entries, self.bytes);
    }
}
pub struct Captured {
    handoff: Option<Result<hooks::Handoff, &'static str>>,
    cursors: Vec<hooks::LaneCursor>,
    drained_pending: AtomicUsize,
}
impl Captured {
    pub fn with_capacities(_capacity: &Capacities) -> Self {
        Self {
            handoff: None,
            cursors: Vec::new(),
            drained_pending: AtomicUsize::new(0),
        }
    }
    pub fn reset(&mut self) {
        self.handoff = None;
        self.drained_pending.store(0, Ordering::Relaxed);
    }
    pub fn allocated_bytes(&self) -> usize {
        self.cursors.capacity() * size_of::<hooks::LaneCursor>()
    }
    /// # Safety
    /// Caller holds exclusive `&mut` access to this frame while operation-log storage stays live
    /// for the active session (or an isolated test) and `operation_outcomes::release` is excluded.
    /// Different frames may grow concurrently across spool and sizing threads via internal locks.
    pub unsafe fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(
            &mut capacity.entries,
            self.drained_pending.load(Ordering::Relaxed),
        );
        // SAFETY: Caller guarantees active-session liveness with `release` excluded.
        unsafe { capacity.prepare() };
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [Outcome::Unavailable { reason: ASSEMBLED }]
    }
    pub fn tables(&self) -> [TableView; N] {
        [unavailable(ASSEMBLED)]
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }
    /// # Safety
    /// Caller must own the active session's capture or publication failure path when invalidating
    /// live operation-completion observation.
    pub unsafe fn fail(&mut self, reason: &'static str) {
        // SAFETY: Caller owns the operation-completion capture or publication failure path.
        unsafe { hooks::invalidate() };
        self.handoff = Some(Err(reason));
    }
    /// # Safety
    /// Only the admitted hour's sole observer boundary (or an isolated test owning the global
    /// operation log) may seal an offered generation and capture a handoff while the log stays live.
    pub unsafe fn capture(&mut self) -> u64 {
        let started = collector_primitives::clock::qpc();
        self.handoff = Some(hooks::handoff_growing(&mut self.cursors));
        collector_primitives::clock::qpc()
            .saturating_sub(started)
            .max(0) as u64
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn drained_pending(&self) -> usize {
        self.drained_pending.load(Ordering::Relaxed)
    }
    #[cfg(test)]
    pub(crate) fn sealed(handoff: hooks::Handoff, cursors: Vec<hooks::LaneCursor>) -> Self {
        Self {
            handoff: Some(Ok(handoff)),
            cursors,
            drained_pending: AtomicUsize::new(0),
        }
    }
}

#[derive(Default)]
pub struct Ledger {
    events: Vec<hooks::Event>,
    bytes: Vec<u8>,
}

pub struct Publication {
    handoff: hooks::Handoff,
    cursors: Vec<hooks::LaneCursor>,
}
impl Publication {
    pub fn allocated_bytes(&self) -> usize {
        self.cursors.capacity() * size_of::<hooks::LaneCursor>()
    }

    /// # Safety
    /// Acknowledge only actual spool queue admission for the originating session, while its
    /// operation log remains live and before `operation_outcomes::release`.
    pub unsafe fn admitted(&self) {
        self.handoff.admitted(&self.cursors);
    }
    /// # Safety
    /// Acknowledge only a confirmed writer receipt for the originating session, while its
    /// operation log remains live and before `operation_outcomes::release`.
    pub unsafe fn commit(self) {
        self.handoff.commit(&self.cursors);
    }
}

/// # Safety
/// Only the originating session's spool may reset the admitted frontier after discarding
/// uncommitted output, while its operation log remains live and before `operation_outcomes::release`.
pub unsafe fn reset_admitted() {
    hooks::reset_admitted();
}
impl Ledger {
    /// # Safety
    /// Only the originating session's sole spool consumer may assemble this captured handoff, while
    /// the operation log retained by `captured` remains live and before `operation_outcomes::release`.
    pub unsafe fn assemble_owned(
        &mut self,
        captured: &Captured,
        tables: &mut [TableInput],
    ) -> Option<Publication> {
        let publication = captured
            .handoff
            .and_then(Result::ok)
            .map(|handoff| Publication {
                handoff,
                cursors: owned_vec(copy_vec(&captured.cursors)),
            });
        let result = captured
            .handoff
            .unwrap_or(Err("operation completion handoff absent"))
            .and_then(|handoff| {
                let pending = handoff.pending();
                captured.drained_pending.store(pending, Ordering::Relaxed);
                grow_vec(&mut self.events, pending);
                grow_vec(
                    &mut self.bytes,
                    handoff.description_capacity(&captured.cursors)?,
                );
                handoff.prepare(&captured.cursors, &mut self.events, &mut self.bytes)
            });
        let view = match result {
            Err(reason) => unavailable(reason),
            Ok(drained) => {
                let mut rows = owned_vec(map_slice(&self.events[..drained.copied], |event| {
                    OperationCompletionRow {
                        owner: event.owner,
                        sequence: event.sequence,
                        id: event.id,
                        target: event.target,
                        definition_token: event.definition_token,
                        completion_hour: event.completion_hour,
                        terminal: event.terminal,
                        bonus: event.bonus,
                        detected: event.detected,
                        description_error: event.description_error,
                        description_required: event.description_required,
                        effect_description: owned_vec(copy_vec(
                            &self.bytes[event.description_start
                                ..event.description_start + event.description_len],
                        )),
                    }
                }));
                rows.sort_unstable_by_key(|row| (row.owner, row.sequence));
                TableView::Complete(TableRows::OperationCompletions(rows))
            }
        };
        tables[TableId::OperationCompletions.index()] = TableInput::Sampled(view);
        publication
    }
}
