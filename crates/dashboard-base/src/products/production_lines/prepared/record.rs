//! What a fold records of a chunk ([`Record`]): a history's events, as the ordered emit holds
//! them.

use super::*;

impl Events {
    fn unread(&mut self, first: i32, last: i32) {
        let overview = &mut self.overview;
        // A span after a wake joins none before it: the wake's spans fold in between.
        if overview
            .wakes
            .last()
            .is_some_and(|wake| wake.at == overview.unread_output.len() as u64)
        {
            overview.unread_output.push(HourSpan {
                first_hour: first,
                last_hour: last,
            });
        } else {
            mark_span(&mut overview.unread_output, first, last);
        }
    }

    fn wake(&mut self, from: u64, to: u64, hour: i32) {
        let at = self.overview.unread_output.len() as u64;
        self.overview.wakes.push(index::Wake { at, from, to, hour });
    }

    fn organisation(&mut self, id: u64, raw: RawOrganisation, named: bool) {
        if named {
            self.overview.names.push((id, raw.clone()));
        }
        self.versions.organisations.push((id, raw));
    }

    fn close_modifier(&mut self, lifetime: u64, at: At, last: i32) {
        let modifiers = self.modifiers.entry(lifetime).or_default();
        match at {
            At::New(at) => modifiers.close(at, last),
            At::Carried(_) => modifiers.carried_last = Some(last),
        }
    }

    fn close_step(&mut self, at: At, id: u64, last: i32) {
        match at {
            At::New(at) => {
                let step = &mut self.overview.steps[at];
                step.last_hour = last;
                step.closed = true;
            }
            At::Carried(_) => self.overview.step_closes.push((id, last)),
        }
    }

    fn close_run(&mut self, at: At, last: i32) {
        match at {
            At::New(at) => self.detail.runs.close(at, last),
            At::Carried(at) => self.detail.carried_runs.close(at, last),
        }
    }

    fn close_output(&mut self, at: At, last: i32) {
        let record = match at {
            At::New(at) => &mut self.detail.outputs[at],
            At::Carried(at) => &mut self.detail.carried_outputs[at],
        };
        record.output.last_hour = last;
        record.closed = true;
    }

    /// Ends the chunk: what it held, with `counters` and the produced runs `produced` ended
    /// last, the next beginning with the same counters.
    fn cut(&mut self, counters: &index::Counters, produced: Vec<(u64, i32, i32)>) -> Events {
        let mut taken = std::mem::take(self);
        self.overview.counters = counters.clone();
        taken.overview.counters = counters.clone();
        taken.overview.produced.extend(produced);
        taken
    }
}

/// What a fold records of the chunk being built: a history's events.
#[derive(Clone, Default)]
pub(super) struct Record(Events);

impl Record {
    #[cfg(test)]
    pub(super) fn held(&self) -> &Events {
        &self.0
    }

    pub(super) fn touch(&mut self, bytes: usize) {
        self.0.touched = true;
        self.0.bytes = self.0.bytes.saturating_add(bytes);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// What the chunk holds, as the batch budget counts it.
    pub(super) fn bytes(&self) -> usize {
        self.0.bytes
    }

    pub(super) fn has_design_anchor(&self, variant: u64) -> bool {
        self.0.versions.design_anchors.contains_key(&variant)
    }

    pub(super) fn has_organisation_anchor(&self, id: u64) -> bool {
        self.0.versions.organisation_anchors.contains_key(&id)
    }

    pub(super) fn read(&mut self, hour: i32) {
        self.0.overview.reads.push(hour);
    }

    pub(super) fn mark_lines(&mut self, hour: i32) {
        mark(&mut self.0.overview.line_hours, hour);
    }

    pub(super) fn mark_output(&mut self, hour: i32) {
        mark(&mut self.0.overview.output_hours, hour);
    }

    pub(super) fn mark_organisations(&mut self, hour: i32) {
        mark(&mut self.0.overview.organisation_hours, hour);
    }

    pub(super) fn span_lines(&mut self, first: i32, last: i32) {
        mark_span(&mut self.0.overview.line_hours, first, last);
    }

    pub(super) fn span_output(&mut self, first: i32, last: i32) {
        mark_span(&mut self.0.overview.output_hours, first, last);
    }

    pub(super) fn span_organisations(&mut self, first: i32, last: i32) {
        mark_span(&mut self.0.overview.organisation_hours, first, last);
    }

    pub(super) fn unread(&mut self, first: i32, last: i32) {
        self.0.unread(first, last);
    }

    pub(super) fn wake(&mut self, from: u64, to: u64, hour: i32) {
        self.0.wake(from, to, hour);
    }

    pub(super) fn life(&mut self, life: index::Life) {
        self.0.overview.lives.push(life);
    }

    pub(super) fn end(&mut self, lifetime: u64, hour: i32) {
        self.0.overview.ends.push((lifetime, hour));
    }

    pub(super) fn final_output(&mut self, lifetime: u64, output: RawOutput) {
        self.0.overview.finals.push((lifetime, output));
    }

    pub(super) fn produced(&mut self, variant: u64, first: i32, last: i32) {
        self.0.overview.produced.push((variant, first, last));
    }

    pub(super) fn design(&mut self, variant: u64, read: DesignRead) {
        self.0.versions.designs.push((variant, read));
    }

    pub(super) fn organisation(&mut self, id: u64, raw: RawOrganisation, named: bool) {
        self.0.organisation(id, raw, named);
    }

    pub(super) fn design_anchor(&mut self, variant: u64, at: Option<u64>) {
        self.0.versions.design_anchors.insert(variant, at);
    }

    pub(super) fn organisation_anchor(&mut self, id: u64, at: Option<u64>) {
        self.0.versions.organisation_anchors.insert(id, at);
    }

    /// A modifier country row: its place among the chunk's.
    pub(super) fn country(&mut self, row: &Arc<CountryProductionModifiersRow>) -> io::Result<u32> {
        self.0.countries.push(Arc::clone(row));
        u32::try_from(self.0.countries.len() - 1)
            .map_err(|_| crate::invalid("A production chunk holds too many countries"))
    }

    /// A modifier read of lifetime `lifetime`: its place among the lifetime's.
    pub(super) fn modifier(
        &mut self,
        lifetime: u64,
        hour: i32,
        country: u32,
        line: Option<&ProductionLineModifiersRow>,
        run: &RawRun,
        organisation: Option<String>,
    ) -> io::Result<usize> {
        Ok(self
            .0
            .modifiers
            .entry(lifetime)
            .or_default()
            .push(hour, country, line, run, organisation))
    }

    pub(super) fn close_modifier(&mut self, lifetime: u64, at: At, last: i32) {
        self.0.close_modifier(lifetime, at, last);
    }

    /// A step: its place among the chunk's.
    pub(super) fn step(&mut self, step: index::Step) -> usize {
        self.0.overview.steps.push(step);
        self.0.overview.steps.len() - 1
    }

    pub(super) fn close_step(&mut self, at: At, id: u64, last: i32) {
        self.0.close_step(at, id, last);
    }

    /// A run: its place among the chunk's.
    pub(super) fn run(&mut self, id: u64, lifetime: u64, run: &RawRun) -> usize {
        self.0.detail.runs.push(id, lifetime, run)
    }

    /// A run carried into the chunk: its place among those carried.
    pub(super) fn carried_run(&mut self, id: u64, lifetime: u64, run: &RawRun) -> usize {
        self.0.detail.carried_runs.push(id, lifetime, run)
    }

    pub(super) fn live(&mut self, lifetime: u64, handle: u64, appearance: u32) {
        self.0.detail.lives.insert(lifetime, (handle, appearance));
    }

    pub(super) fn close_run(&mut self, at: At, last: i32) {
        self.0.close_run(at, last);
    }

    pub(super) fn output_variants(&mut self, variants: impl Iterator<Item = u64>) {
        self.0.overview.output_variants.extend(variants);
    }

    /// An output read: its place among the chunk's.
    pub(super) fn output(&mut self, record: OutputRecord) -> usize {
        self.0.detail.outputs.push(record);
        self.0.detail.outputs.len() - 1
    }

    /// An output read carried into the chunk: its place among those carried.
    pub(super) fn carried_output(&mut self, record: OutputRecord) -> usize {
        self.0.detail.carried_outputs.push(record);
        self.0.detail.carried_outputs.len() - 1
    }

    pub(super) fn close_output(&mut self, at: At, last: i32) {
        self.0.close_output(at, last);
    }

    /// Ends the chunk ([`Events::cut`]).
    pub(super) fn cut(
        &mut self,
        counters: &index::Counters,
        produced: Vec<(u64, i32, i32)>,
    ) -> Events {
        self.0.cut(counters, produced)
    }
}
