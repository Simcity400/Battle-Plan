use super::*;

// ===========================================================================
// Scheduled Focus and CompletedFocuses owned native acquisition.
//
// The callback-bounded World job runs the existing native walker once per pass.
// Rows and referenced token bytes are owned here; the decode worker only installs
// admitted prefixes. No raw index, mirrored census or replayed walker remains.
// Exact frame grants are checked before any source read, so storage refusal
// can use the typed fallback without retrying a consumed read fault.
// ===========================================================================

#[derive(Debug, Default)]
pub(crate) struct FocusJob {
    pub active: bool,
    pub(super) rows: Vec<cnational_focus_progress::Row>,
    pub(super) completed: Vec<cnational_focus_progress::CompletedRow>,
    pub(super) bytes: Vec<u8>,
    pub(super) outcomes: [Option<Outcome>; 2],
    pub(super) bytes_len: usize,
    pub(super) required_bytes: usize,
    pub(super) row_grants: [usize; 2],
    pub(super) byte_grant: usize,
    pub(super) provisioned: bool,
    pub(super) capacity_charge: crate::raw_memory::Charge,
}

impl FocusJob {
    pub fn new() -> Self {
        let capacities = Capacities::default();
        let mut job = Self {
            rows: vec![Default::default(); capacities.rows[0]],
            completed: vec![Default::default(); capacities.rows[1]],
            bytes: vec![0; capacities.focus_text],
            row_grants: [capacities.rows[0], capacities.rows[1]],
            byte_grant: capacities.focus_text,
            provisioned: true,
            ..Default::default()
        };
        job.update_capacity_charge();
        job
    }

    pub fn is_provisioned(&self) -> bool {
        self.provisioned
    }

    #[cfg(test)]
    pub(crate) fn capacities(&self) -> ([usize; 2], usize) {
        ([self.rows.len(), self.completed.len()], self.bytes.len())
    }

    pub fn clear(&mut self) {
        self.active = false;
        self.outcomes = [None; 2];
        self.bytes_len = 0;
        self.required_bytes = 0;
        self.row_grants = [0; 2];
        self.byte_grant = 0;
    }

    pub(super) fn prepare(&mut self, rows: [usize; 2], bytes: usize) -> bool {
        self.clear();
        self.active = true;
        self.row_grants = rows;
        self.byte_grant = bytes;
        self.provisioned && !self.grow_for_next()
    }

    pub fn grow_for_next(&self) -> bool {
        self.rows.len() < self.row_grants[0]
            || self.completed.len() < self.row_grants[1]
            || self.bytes.len() < self.byte_grant
    }

    /// Called only after callback ownership ends, on the decode side.
    pub fn grow(&mut self) {
        grow_vec(&mut self.rows, self.row_grants[0]);
        grow_vec(&mut self.completed, self.row_grants[1]);
        grow_vec(&mut self.bytes, self.byte_grant);
        self.update_capacity_charge();
    }

    pub(super) fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            payload_vectors: vector_bytes(&self.rows)
                + vector_bytes(&self.completed)
                + vector_bytes(&self.bytes),
            ..Default::default()
        });
    }

    fn outcome(&self, slot: usize) -> Outcome {
        self.outcomes[slot].unwrap_or(Outcome::Unavailable {
            reason: "focus acquisition not supplied",
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FocusCopyReport {
    /// False only for a preflight refusal, before any live source read.
    pub raw_ok: bool,
    /// Shared native traversal is charged once to Focus.
    pub copy_ticks: [u64; 2],
    /// Indexed raw payload bytes only. Owned rows/tokens are charged by capacity.
    pub raw_bytes: [u64; 2],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FocusCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub active: bool,
    pub rows: [usize; 2],
    pub bytes: usize,
}

pub(super) fn copy_focus_pass(
    job: &mut FocusJob,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; 2],
) {
    let _reads = read_accounting::table(TABLES[0]);
    let started = qpc();
    let mut focus_total = Outcome::Rows { len: 0 };
    let mut completed_total = Outcome::Rows { len: 0 };
    let mut text_len = 0usize;
    let mut text_required = 0usize;
    match ccountry::Reader::new(memory, base, game_state) {
        Err(reason) => {
            focus_total = Outcome::Unavailable { reason };
            completed_total = Outcome::Unavailable { reason };
        }
        Ok(countries) => {
            for slot in 1..countries.count() {
                let country = match countries.at(slot) {
                    Ok(Some(country)) => country,
                    Ok(None) => continue,
                    Err(reason) => {
                        focus_total = Outcome::Unavailable { reason };
                        completed_total = Outcome::Unavailable { reason };
                        break;
                    }
                };
                let first = focus_total.len();
                let completed = completed_total.len();
                let text = text_len;
                let result = cnational_focus_progress::walk(
                    memory,
                    base,
                    country.pointer,
                    country.id,
                    &mut job.rows[first..job.row_grants[0]],
                    &mut job.completed[completed..job.row_grants[1]],
                    &mut job.bytes[text..job.byte_grant],
                );
                for row in &mut job.rows[first..first + result.outcome.len()] {
                    for span in [&mut row.tree, &mut row.current, &mut row.continuous]
                        .into_iter()
                        .flatten()
                    {
                        span.start += text;
                    }
                }
                for row in &mut job.completed[completed..completed + result.completed_outcome.len()]
                {
                    row.token.start += text;
                }
                append(&mut focus_total, result.outcome);
                append(&mut completed_total, result.completed_outcome);
                text_len += result.bytes_len;
                text_required = text_required.saturating_add(result.required_bytes);
            }
        }
    }
    job.outcomes = [Some(focus_total), Some(completed_total)];
    job.bytes_len = text_len;
    job.required_bytes = text_required;
    ticks[0] += elapsed(started);
}

pub(crate) fn copy_focus_refresh(
    memory: &impl Memory,
    ctx: FocusCopyContext,
    job: &mut FocusJob,
) -> FocusCopyReport {
    if !ctx.active {
        job.clear();
        return FocusCopyReport {
            raw_ok: true,
            ..Default::default()
        };
    }
    if !job.prepare(ctx.rows, ctx.bytes) {
        return FocusCopyReport::default();
    }
    let mut report = FocusCopyReport {
        raw_ok: true,
        ..Default::default()
    };
    copy_focus_pass(
        job,
        memory,
        ctx.base,
        ctx.game_state,
        &mut report.copy_ticks,
    );
    report
}

impl Captured {
    pub(crate) fn focus_copy_context(
        &self,
        base: usize,
        game_state: usize,
        active: bool,
    ) -> FocusCopyContext {
        FocusCopyContext {
            base,
            game_state,
            active,
            rows: [self.focus.rows.len(), self.completed.rows.len()],
            bytes: self.focus_text.first.len(),
        }
    }

    /// Finish an accepted acquisition failure without retrying live memory.
    pub(crate) fn fail_focus_capture(&mut self, active: bool, reason: &'static str) {
        let reason = if active {
            reason
        } else {
            replay::cadence::SKIPPED_REASON
        };
        self.focus.outcome = Outcome::Unavailable { reason };
        self.completed.outcome = Outcome::Unavailable { reason };
        self.focus_text.reset();
        self.focus_raw_mode = false;
    }

    /// Installs only already-owned admitted prefixes. A changed frame grant is
    /// an ownership error, never a reason to touch the live source again.
    pub(crate) fn decode_focus_payload(
        &mut self,
        _base: usize,
        job: &FocusJob,
        decode_ticks: &mut [u64; 2],
    ) {
        if !job.active {
            self.fail_focus_capture(false, replay::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        let arena_ok = self.focus_text.first.len() == job.byte_grant
            && job.bytes_len <= job.byte_grant
            && job.bytes_len <= job.bytes.len();
        self.focus.outcome = job.outcome(0);
        if !arena_ok
            || self.focus.rows.len() != job.row_grants[0]
            || self.focus.outcome.len() > job.row_grants[0]
        {
            self.focus.outcome = Outcome::Unavailable {
                reason: "focus owned payload capacity mismatch",
            };
        } else {
            let len = self.focus.outcome.len();
            self.focus.rows[..len].copy_from_slice(&job.rows[..len]);
        }
        if arena_ok {
            self.focus_text.first[..job.bytes_len].copy_from_slice(&job.bytes[..job.bytes_len]);
            self.focus_text.len = job.bytes_len;
            self.focus_text.required = job.required_bytes;
        } else {
            self.focus_text.len = 0;
            self.focus_text.required = job.required_bytes.max(job.byte_grant);
        }
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        self.completed.outcome = job.outcome(1);
        if !arena_ok
            || self.completed.rows.len() != job.row_grants[1]
            || self.completed.outcome.len() > job.row_grants[1]
        {
            self.completed.outcome = Outcome::Unavailable {
                reason: "completed focus owned payload capacity mismatch",
            };
        } else {
            let len = self.completed.outcome.len();
            self.completed.rows[..len].copy_from_slice(&job.completed[..len]);
        }
        decode_ticks[1] += elapsed(started);
        self.focus_raw_mode = false;
    }
}
