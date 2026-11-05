mod binary;
#[cfg(test)]
mod columnar_parity;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
mod keys;
#[cfg(test)]
mod parity;
#[cfg(test)]
mod tests;

use super::index::{self, Events, OutputRecord};
use super::input::{HourInput, ReadEvidence, World};
use super::read;
use super::*;
use crate::prepared::pages::Pages;
use crate::products::country_groups::HourSpan;
use keys::{Ctx, Keys, Trees};
use replay_data::rows::{CountryProductionModifiersRow, ProductionLineModifiersRow, TableRows};

/// A full ledger record after this many deltas, so a resume reads a bounded number. Each
/// one also sweeps the keyed values no read asked for since the one before into their trees.
const LEDGER_FULL_EVERY: usize = 16;
/// Keyed values the loaded histories hold together, past which a flush sweeps too.
const RESIDENT_KEYS: usize = 1 << 20;

/// A writer seen as `dyn Pages`, whatever its own type.
struct Through<'a, W: ?Sized>(&'a mut W);

impl<W: Pages + ?Sized> Pages for Through<'_, W> {
    #[cfg(any(test, feature = "test-support"))]
    fn note_framing(&mut self, section: &str, range: u64, bytes: usize) {
        self.0.note_framing(section, range, bytes);
    }
    fn write_at(&mut self, section: &str, range: u64, data: &[u8]) -> io::Result<()> {
        self.0.write_at(section, range, data)
    }
    fn write(&mut self, section: &str, data: &[u8]) -> io::Result<u64> {
        self.0.write(section, data)
    }
    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        self.0.read_at(section, range)
    }
    fn read_slice(
        &mut self,
        section: &str,
        range: u64,
        offset: u64,
        output: &mut [u8],
    ) -> io::Result<()> {
        self.0.read_slice(section, range, offset, output)
    }
    fn range_ids(
        &mut self,
        section: &str,
        count: u64,
    ) -> io::Result<Option<Vec<Option<store::RangeId>>>> {
        self.0.range_ids(section, count)
    }
}

/// Where a live interval's record lies in its history's events: begun in the chunk being
/// built, or carried into it.
#[derive(Clone, Copy, Debug)]
enum At {
    New(usize),
    Carried(usize),
}

#[derive(Clone, Debug)]
struct LiveRun {
    id: u64,
    run: RawRun,
    at: At,
}

#[derive(Clone, Copy, Debug)]
struct LiveStep {
    id: u64,
    variant: u64,
    active_factories: i32,
    requested_factories: i32,
    at: At,
}

#[derive(Clone, Debug)]
struct LiveOutput {
    id: u64,
    output: RawOutput,
    at: At,
}

#[derive(Clone, Copy, Debug)]
struct LiveModifier {
    last: i32,
    at: At,
}

/// A line open or broken at the last read: its lifetime and what it still carries.
#[derive(Clone, Debug)]
struct Line {
    lifetime: u64,
    handle: u64,
    appearance: u32,
    run: Option<LiveRun>,
    step: Option<LiveStep>,
    output: Option<LiveOutput>,
    modifier: Option<LiveModifier>,
    /// Its last modifier read's line operands, which an equal next read does not repeat.
    operands: Option<ProductionLineModifiersRow>,
}

impl Line {
    fn new(lifetime: u64, handle: u64, appearance: u32) -> Self {
        Self {
            lifetime,
            handle,
            appearance,
            run: None,
            step: None,
            output: None,
            modifier: None,
            operands: None,
        }
    }
}

/// A handle's latest lifetime and how many it has had.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Handle {
    lifetime: u64,
    appearance: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Ids {
    lifetime: u64,
    run: u64,
    step: u64,
    output: u64,
}

/// Keys whose latest value the next ledger delta holds.
#[derive(Clone, Debug, Default)]
struct Dirty {
    handles: BTreeSet<u64>,
    designs: BTreeSet<u64>,
    organisations: BTreeSet<u64>,
    icons: BTreeSet<u64>,
}

#[derive(Clone, Default)]
pub struct CountryState {
    events: Events,
    last: Option<((u32, u32), i32)>,
    open: BTreeMap<u64, Line>,
    carried: bool,
    broken: BTreeMap<u64, Line>,
    last_read_identity: Option<(u32, u32)>,
    broken_identity: Option<(u32, u32)>,
    first_read: bool,
    output_open: bool,
    organisations_open: bool,
    last_census: Option<i32>,
    active_variants: BTreeSet<u64>,
    carry: Option<Carry>,
    last_output_due: Option<i32>,
    last_modifier_country: Option<Arc<CountryProductionModifiersRow>>,
    /// The last modifier country's place among the chunk's country rows.
    country_at: Option<u32>,
    last_design_revision: Option<u64>,
    /// A handle's latest lifetime once its line closed (an open or broken line names its
    /// own), each variant's last design read and icon, each organisation's last version.
    keys: Keys,
    counters: index::Counters,
    /// The absence's unread spans: what it adds to a country waking from it.
    stream: Option<keys::Stream>,
    ids: Ids,
    /// The last hour it followed: where what it still carries ends.
    through: Option<i32>,
    /// The last chunk holding its overview.
    overview: Option<u64>,
    dirty: Dirty,
    /// Observed or changed since the last ledger record.
    changed: bool,
    /// The next ledger record holds all of it.
    fresh: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Carry {
    first: i32,
    last: i32,
    modifiers: bool,
    output: bool,
}

fn same_design(a: &DesignRead, b: &DesignRead) -> bool {
    DesignRead {
        first_hour: 0,
        ..*a
    } == DesignRead {
        first_hour: 0,
        ..*b
    }
}

fn same_organisation(a: &RawOrganisation, b: &RawOrganisation) -> bool {
    same_naming(a, b) && a.state == b.state
}

/// The operands an organisation's reply name and icon read.
fn same_naming(a: &RawOrganisation, b: &RawOrganisation) -> bool {
    (
        &a.token,
        &a.name_key,
        &a.icon,
        &a.owner_tag,
        a.owner_unknown,
    ) == (
        &b.token,
        &b.name_key,
        &b.icon,
        &b.owner_tag,
        b.owner_unknown,
    )
}

fn organisation_bytes(raw: &RawOrganisation) -> usize {
    std::mem::size_of::<RawOrganisation>()
        + raw.token.len()
        + raw.name_key.len()
        + raw.icon.len()
        + raw.owner_tag.as_ref().map_or(0, String::len)
        + raw.state.as_ref().map_or(0, |state| {
            std::mem::size_of::<OrganisationState>()
                + state.funds_raw.len()
                + state
                    .unlocked
                    .iter()
                    .chain(&state.planned)
                    .map(String::len)
                    .sum::<usize>()
        })
}

fn icon_bytes(icon: &IconChoice) -> usize {
    std::mem::size_of::<IconChoice>()
        + icon.probes.len() * 48
        + icon.otherwise.as_ref().err().map_or(0, String::len)
}

fn output_bytes(output: &RawOutput) -> usize {
    std::mem::size_of::<OutputRecord>() + output.parts.len() * std::mem::size_of::<RawPart>()
}

fn modifier_country_bytes(country: &CountryProductionModifiersRow) -> usize {
    std::mem::size_of_val(country)
        + country
            .trees
            .iter()
            .map(|tree| {
                std::mem::size_of_val(tree)
                    + tree.failure.len()
                    + tree.nodes.len()
                        * std::mem::size_of::<replay_data::rows::ModifierBreakdownNode>()
                    + tree.nodes.iter().map(|node| node.name.len()).sum::<usize>()
            })
            .sum::<usize>()
}

fn line_operand_bytes(line: &ProductionLineModifiersRow) -> usize {
    std::mem::size_of_val(line) + line.factory_efficiencies.len() * std::mem::size_of::<i64>()
}

fn close_run(events: &mut Events, run: LiveRun) {
    let (runs, at) = match run.at {
        At::New(at) => (&mut events.detail.runs, at),
        At::Carried(at) => (&mut events.detail.carried_runs, at),
    };
    runs.close(at, run.run.last_hour);
    events.touched = true;
}

fn close_step(events: &mut Events, step: LiveStep, last: i32) {
    match step.at {
        At::New(at) => {
            let step = &mut events.overview.steps[at];
            step.last_hour = last;
            step.closed = true;
        }
        At::Carried(_) => events.overview.step_closes.push((step.id, last)),
    }
    events.touched = true;
}

fn close_output(events: &mut Events, output: LiveOutput) {
    let record = match output.at {
        At::New(at) => &mut events.detail.outputs[at],
        At::Carried(at) => &mut events.detail.carried_outputs[at],
    };
    record.output.last_hour = output.output.last_hour;
    record.closed = true;
    events.touched = true;
}

fn close_modifier(events: &mut Events, lifetime: u64, modifier: LiveModifier) {
    let modifiers = events.modifiers.entry(lifetime).or_default();
    match modifier.at {
        At::New(at) => {
            let record = &mut modifiers.records[at];
            record.last_hour = modifier.last;
            record.closed = true;
        }
        At::Carried(_) => modifiers.carried_last = Some(modifier.last),
    }
    events.touched = true;
}

/// Ends everything `line` carries where it reached.
fn close_line(events: &mut Events, line: &mut Line) {
    if let Some(run) = line.run.take() {
        let last = run.run.last_hour;
        close_run(events, run);
        if let Some(step) = line.step.take() {
            close_step(events, step, last);
        }
    }
    if let Some(output) = line.output.take() {
        close_output(events, output);
    }
    if let Some(modifier) = line.modifier.take() {
        close_modifier(events, line.lifetime, modifier);
    }
}

impl CountryState {
    pub fn new() -> Self {
        Self {
            first_read: true,
            fresh: true,
            ..Self::default()
        }
    }

    fn touch(&mut self, bytes: usize) {
        self.events.touched = true;
        self.events.bytes = self.events.bytes.saturating_add(bytes);
    }

    fn pending_bytes(&self) -> usize {
        self.events.bytes
    }

    /// The events since the flush before, leaving the next chunk's carried entries: the
    /// runs and output reads open now, with their values, so each chunk reads alone, and
    /// where the versions those runs overlay were pushed. A dormant history writes them
    /// only once it records more.
    fn take_events(&mut self, live: bool, ctx: &mut Ctx<'_>) -> io::Result<Events> {
        let mut next = Events::default();
        let mut carried = Vec::new();
        for line in self.open.values_mut() {
            if let Some(run) = &mut line.run {
                let at = next
                    .detail
                    .carried_runs
                    .push(run.id, line.lifetime, &run.run);
                run.at = At::Carried(at);
                next.detail
                    .lives
                    .insert(line.lifetime, (line.handle, line.appearance));
                carried.push((run.run.variant, run.run.mio));
            }
            if let Some(step) = &mut line.step {
                step.at = At::Carried(0);
            }
            if let Some(output) = &mut line.output {
                next.detail.carried_outputs.push(OutputRecord {
                    id: output.id,
                    lifetime: line.lifetime,
                    output: output.output.clone(),
                    closed: false,
                });
                output.at = At::Carried(next.detail.carried_outputs.len() - 1);
                next.detail
                    .lives
                    .insert(line.lifetime, (line.handle, line.appearance));
            }
            if let Some(modifier) = &mut line.modifier {
                modifier.at = At::Carried(0);
            }
        }
        next.overview.counters = self.counters.clone();
        next.touched = live
            && (next.detail.carried_runs.len() != 0 || !next.detail.carried_outputs.is_empty());
        self.country_at = None;
        let mut taken = std::mem::replace(&mut self.events, next);
        taken.overview.counters = self.counters.clone();
        for (variant, mio) in carried {
            self.anchor_design(variant, ctx)?;
            if mio != 0 {
                self.anchor_organisation(mio, ctx)?;
            }
        }
        Ok(taken)
    }

    fn materialize(&mut self) {
        let Some(carry) = self.carry.take() else {
            return;
        };
        let events = &mut self.events;
        for line in self.open.values_mut() {
            crate::prepared::prepass::note("production-lines.carried_line");
            let extends = line.modifier.is_some_and(|modifier| {
                carry.modifiers && modifier.last.checked_add(1) == Some(carry.first)
            });
            match line.modifier.take() {
                Some(mut modifier) if extends => {
                    modifier.last = carry.last;
                    line.modifier = Some(modifier);
                }
                Some(modifier) => close_modifier(events, line.lifetime, modifier),
                None => {}
            }
            if let Some(run) = &mut line.run
                && run.run.last_hour.checked_add(1) == Some(carry.first)
            {
                run.run.last_hour = carry.last;
            }
            let extends = line.output.as_ref().is_some_and(|output| {
                carry.output && output.output.last_hour.checked_add(1) == Some(carry.first)
            });
            match line.output.take() {
                Some(mut output) if extends => {
                    output.output.last_hour = carry.last;
                    line.output = Some(output);
                }
                Some(output) => close_output(events, output),
                None => {}
            }
        }
    }

    /// Follows the country into `hours`: what it carried ends where it or its lines broke off.
    fn enter(&mut self, hours: &Hours, identity: Option<(u32, u32)>, lines_read: bool) {
        self.changed = true;
        self.through = Some(hours.last);
        let continues = identity.is_some_and(|identity| {
            self.last.is_some_and(|(seen, at)| {
                seen == identity && at.checked_add(1) == Some(hours.first)
            })
        });
        if self.carry.as_ref().is_some_and(|span| {
            !continues
                || lines_read
                || !hours.lines_complete
                || span.modifiers != hours.modifiers_complete
                || span.output != (self.output_open && hours.output_complete)
        }) {
            self.materialize();
        }
        self.last = identity.map(|identity| (identity, hours.last));
        if self.carried && (!continues || !hours.lines_complete) {
            self.broken = std::mem::take(&mut self.open);
            for line in self.broken.values_mut() {
                close_line(&mut self.events, line);
            }
            self.broken_identity = self.last_read_identity;
            self.carried = false;
            self.output_open = false;
            self.organisations_open = false;
        }
    }

    /// Carries the open lines through `hours`, which read none.
    fn carry_through(&mut self, hours: &Hours) {
        if !self.carried {
            return;
        }
        let _phase = replay_frame::cpu::scope_phase("production-lines:carry");
        mark_span(
            &mut self.events.overview.line_hours,
            hours.first,
            hours.last,
        );
        self.events.touched = true;
        let output = self.output_open && hours.output_complete;
        if self.output_open && !output {
            for line in self.open.values_mut() {
                if let Some(output) = line.output.take() {
                    close_output(&mut self.events, output);
                }
            }
        }
        self.output_open = output;
        self.organisations_open &= hours.organisations_complete;
        if self.organisations_open {
            mark_span(
                &mut self.events.overview.organisation_hours,
                hours.first,
                hours.last,
            );
        }
        match &mut self.carry {
            Some(span) => span.last = hours.last,
            None => {
                self.carry = Some(Carry {
                    first: hours.first,
                    last: hours.last,
                    modifiers: hours.modifiers_complete,
                    output: self.output_open,
                })
            }
        }
        if self.output_open {
            mark_span(
                &mut self.events.overview.output_hours,
                hours.first,
                hours.last,
            );
        }
    }

    fn record_designs(
        &mut self,
        input: &HourInput,
        at: Dated,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<()> {
        let variants: Vec<u64> = self.active_variants.iter().copied().collect();
        for variant in variants {
            self.record_design(input, at, variant, ctx)?;
        }
        Ok(())
    }

    /// Follows deferred runs as inputs: identities as last followed, designs recorded as they came.
    fn follow(&mut self, runs: &[Hours]) {
        let identity = self.last.map(|(identity, _)| identity);
        for hours in runs {
            crate::prepared::prepass::note("production-lines.country_carry");
            self.enter(hours, identity, false);
            self.last_design_revision = Some(hours.design_revision);
            if identity.is_some() && hours.lines_complete {
                self.carry_through(hours);
            }
        }
    }

    /// Whether a design change records this country's variants.
    fn designs_carried(&self) -> bool {
        self.carried && !self.active_variants.is_empty()
    }

    /// The design read a census or design change takes for `variant`, a new version where
    /// it differs, and its author's naming where none is recorded yet.
    fn record_design(
        &mut self,
        input: &HourInput,
        at: Dated,
        variant: u64,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<()> {
        let read = DesignRead {
            first_hour: at.hour,
            read_hour: input
                .designs
                .get(&variant)
                .and_then(|_| at.design_read_hour),
            ..input.designs.get(&variant).copied().unwrap_or_default()
        };
        self.anchor_design(variant, ctx)?;
        let root = self.keys.root;
        if self
            .keys
            .designs
            .get(variant, root, ctx)?
            .is_none_or(|(last, _)| !same_design(last, &read))
        {
            self.keys
                .designs
                .put(variant, (read, ctx.chunk), ctx.generation);
            self.dirty.designs.insert(variant);
            self.events.versions.designs.push((variant, read));
            self.touch(std::mem::size_of::<(u64, DesignRead)>());
        }
        if let Some(author) = read.author.filter(|id| *id != 0)
            && at.author_names_due
            && self.keys.organisations.get(author, root, ctx)?.is_none()
            && let Some(raw) = input.author_names.get(&author)
        {
            self.organisation_version(
                author,
                RawOrganisation {
                    first_hour: at.hour,
                    ..raw.clone()
                },
                ctx,
            )?;
        }
        Ok(())
    }

    /// A new organisation version where it differs from the last, whatever hour that is from.
    fn organisation_version(
        &mut self,
        id: u64,
        raw: RawOrganisation,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<()> {
        self.anchor_organisation(id, ctx)?;
        let root = self.keys.root;
        let previous = self.keys.organisations.get(id, root, ctx)?;
        if previous.is_some_and(|(last, _)| same_organisation(last, &raw)) {
            return Ok(());
        }
        let named = previous.is_none_or(|(last, _)| !same_naming(last, &raw));
        let bytes = organisation_bytes(&raw);
        if named {
            self.events.overview.names.push((id, raw.clone()));
        }
        self.events.versions.organisations.push((id, raw.clone()));
        self.keys
            .organisations
            .put(id, (raw, ctx.chunk), ctx.generation);
        self.dirty.organisations.insert(id);
        self.touch(bytes * (1 + usize::from(named)));
        Ok(())
    }

    fn icon_version(
        &mut self,
        id: u64,
        hour: i32,
        icon: &IconChoice,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<()> {
        let root = self.keys.root;
        if self.keys.icons.get(id, root, ctx)? == Some(icon) {
            return Ok(());
        }
        self.events.overview.icons.push((id, hour, icon.clone()));
        self.keys.icons.put(id, icon.clone(), ctx.generation);
        self.dirty.icons.insert(id);
        self.touch(icon_bytes(icon));
        Ok(())
    }

    /// Notes, once a chunk, where the design of `variant` holding as it began was pushed.
    fn anchor_design(&mut self, variant: u64, ctx: &mut Ctx<'_>) -> io::Result<()> {
        if !self.events.versions.design_anchors.contains_key(&variant) {
            let root = self.keys.root;
            let at = self
                .keys
                .designs
                .get(variant, root, ctx)?
                .map(|(_, chunk)| *chunk);
            self.events.versions.design_anchors.insert(variant, at);
        }
        Ok(())
    }

    /// [`Self::anchor_design`] of an organisation's version.
    fn anchor_organisation(&mut self, id: u64, ctx: &mut Ctx<'_>) -> io::Result<()> {
        if !self.events.versions.organisation_anchors.contains_key(&id) {
            let root = self.keys.root;
            let at = self
                .keys
                .organisations
                .get(id, root, ctx)?
                .map(|(_, chunk)| *chunk);
            self.events.versions.organisation_anchors.insert(id, at);
        }
        Ok(())
    }

    /// A handle's latest lifetime and appearance: its open or broken line's, or the last
    /// its closed lines left.
    fn handle(&mut self, handle: u64, ctx: &mut Ctx<'_>) -> io::Result<Option<Handle>> {
        if let Some(line) = self.open.get(&handle).or_else(|| self.broken.get(&handle)) {
            return Ok(Some(Handle {
                lifetime: line.lifetime,
                appearance: line.appearance,
            }));
        }
        let root = self.keys.root;
        Ok(self.keys.handles.get(handle, root, ctx)?.copied())
    }

    /// Keeps the lifetime of a line no longer open or broken under its handle.
    fn closed(&mut self, line: &Line, ctx: &Ctx<'_>) {
        self.keys.handles.put(
            line.handle,
            Handle {
                lifetime: line.lifetime,
                appearance: line.appearance,
            },
            ctx.generation,
        );
        self.dirty.handles.insert(line.handle);
    }

    fn unread_span(&mut self, first: i32, last: i32) -> io::Result<()> {
        let overview = &mut self.events.overview;
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
        if let Some(stream) = &mut self.stream {
            stream.push(first, last)?;
        }
        self.touch(std::mem::size_of::<HourSpan>());
        Ok(())
    }

    fn output_due(&mut self, available: bool, hour: i32) -> io::Result<()> {
        if !available {
            self.counters.unread_output_reads += 1;
            let first = self
                .last_output_due
                .and_then(|due| due.checked_add(1))
                .unwrap_or(hour);
            self.unread_span(first.min(hour), hour)?;
        }
        self.last_output_due = Some(hour);
        Ok(())
    }

    fn lost(&mut self, first: i32, last: i32) -> io::Result<()> {
        self.changed = true;
        self.counters.unreadable_stretches += 1;
        self.unread_span(first.min(last), last)
    }

    /// What `absent` recorded since this country went dormant at `sleep`. Its unread spans
    /// from then on stay in the absence's pages: the wake names them, and a query folds
    /// them in where it stands among this country's own.
    fn wake(&mut self, sleep: Option<Sleep>, absent: &CountryState) -> io::Result<()> {
        let Some(sleep) = sleep else {
            return Ok(());
        };
        self.changed = true;
        self.counters.unread_output_reads += absent
            .counters
            .unread_output_reads
            .saturating_sub(sleep.reads);
        self.counters.unreadable_stretches += absent
            .counters
            .unreadable_stretches
            .saturating_sub(sleep.stretches);
        let recorded = absent
            .stream
            .as_ref()
            .ok_or_else(|| crate::invalid("The production absence keeps no unread spans"))?
            .count;
        if sleep.spans > recorded {
            return Err(crate::invalid(
                "A dormant production country names unread spans the absence never recorded",
            ));
        }
        if sleep.spans < recorded {
            let at = self.events.overview.unread_output.len() as u64;
            self.events.overview.wakes.push(index::Wake {
                at,
                from: sleep.spans,
                to: recorded,
                hour: sleep.hour,
            });
        }
        self.touch(std::mem::size_of::<index::Wake>());
        self.last_output_due = absent.last_output_due;
        Ok(())
    }

    /// Follows `evidence`'s reads, each read as `input` with the read's hour and flags.
    fn observe_evidence(
        &mut self,
        input: &HourInput,
        country: Country,
        evidence: &[ReadEvidence],
        ctx: &mut Ctx<'_>,
    ) -> io::Result<()> {
        self.changed = true;
        let named: BTreeSet<_> = country
            .found()
            .filter(|_| evidence.iter().any(|read| read.organisation_changed))
            .into_iter()
            .flat_map(|(slot, _)| {
                input
                    .lines
                    .get(&slot)
                    .into_iter()
                    .flatten()
                    .flat_map(|row| {
                        std::iter::once(row.mio)
                            .chain(input.designs.get(&row.variant).and_then(|read| read.author))
                    })
            })
            .filter(|id| *id != 0)
            .collect();
        for read in evidence {
            if let Some(available) =
                input.output_due_at(read.output_read, read.lines_read, Some(country))
            {
                self.output_due(available, read.hour)?;
            }
            let changed = self.last_design_revision != Some(read.design_revision);
            self.last_design_revision = Some(read.design_revision);
            if !self.carried {
                continue;
            }
            if changed {
                self.record_designs(
                    input,
                    Dated {
                        hour: read.hour,
                        design_read_hour: read.design_read_hour,
                        author_names_due: read.author_names_due,
                    },
                    ctx,
                )?;
            }
            if !read.lines_read {
                continue;
            }
            self.events.overview.reads.push(read.hour);
            self.touch(std::mem::size_of::<i32>());
            self.last_census = Some(read.hour);
            if read.modifiers_read
                && let Some(reason) = &input.modifier_refusal
            {
                *self.counters.refusals.entry(reason.clone()).or_default() += 1;
            }
            if read.organisation_changed {
                for id in &named {
                    if let Some(raw) = input.organisations.get(id) {
                        let mut raw = RawOrganisation {
                            first_hour: read.hour,
                            ..raw.clone()
                        };
                        if let Some(state) = &mut raw.state {
                            state.read_hour = read.organisation_read_hour;
                        }
                        self.organisation_version(*id, raw, ctx)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn observe_input(
        &mut self,
        input: &HourInput,
        country: Option<Country>,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<()> {
        crate::prepared::prepass::note("production-lines.country_fold");
        let hours = Hours::of(input);
        let identity = country.and_then(Country::found);
        self.enter(&hours, identity, input.lines_read);
        if let Some(read) = input.output_due(country) {
            self.output_due(read, input.hour)?;
        }
        let designs_changed = self.last_design_revision != Some(input.design_revision);
        self.last_design_revision = Some(input.design_revision);
        let (Some(identity), true) = (identity, input.lines_complete) else {
            return Ok(());
        };
        if self.carried && designs_changed {
            self.record_designs(input, Dated::of(input), ctx)?;
        }
        if !input.lines_read {
            self.carry_through(&hours);
            return Ok(());
        }
        self.census(input, identity, ctx)
    }

    fn census(
        &mut self,
        input: &HourInput,
        identity: (u32, u32),
        ctx: &mut Ctx<'_>,
    ) -> io::Result<()> {
        let hour = input.hour;
        let slot = identity.0;
        let rows = input
            .lines
            .get(&slot)
            .map(Vec::as_slice)
            .unwrap_or_default();
        self.events.overview.reads.push(hour);
        mark(&mut self.events.overview.line_hours, hour);
        self.touch(std::mem::size_of::<i32>());
        let previous_read = self.last_census;
        self.last_census = Some(hour);
        if self.broken_identity != Some(identity) {
            for line in std::mem::take(&mut self.broken).into_values() {
                self.closed(&line, ctx);
            }
        }
        self.last_read_identity = Some(identity);
        // A census ends what the one before's output and modifier reads carried.
        for line in self.open.values_mut() {
            if let Some(output) = line.output.take() {
                close_output(&mut self.events, output);
            }
            if let Some(modifier) = line.modifier.take() {
                close_modifier(&mut self.events, line.lifetime, modifier);
            }
        }
        let mut still: BTreeMap<u64, Line> = BTreeMap::new();
        self.counters.unlinked_rows += input.unlinked_lines.len();
        let mut mine: Vec<&ProductionLineRow> = rows
            .iter()
            .chain(input.unlinked_lines.iter().filter(|row| {
                self.open.contains_key(&row.handle) || self.broken.contains_key(&row.handle)
            }))
            .collect();
        mine.sort_by_key(|row| row.handle);
        let modifier_country = match input.modifier_countries.get(&identity.1) {
            Some(country) => {
                if self
                    .last_modifier_country
                    .as_ref()
                    .is_none_or(|last| !Arc::ptr_eq(last, country) && **last != **country)
                {
                    self.events.countries.push(Arc::clone(country));
                    self.country_at = Some((self.events.countries.len() - 1) as u32);
                    self.touch(modifier_country_bytes(country));
                    self.last_modifier_country = Some(Arc::clone(country));
                }
                self.last_modifier_country.clone()
            }
            None => None,
        };
        if input.modifiers_read
            && let Some(reason) = &input.modifier_refusal
        {
            *self.counters.refusals.entry(reason.clone()).or_default() += 1;
            self.events.touched = true;
        }
        for row in &mine {
            let Some(kind) = LineKind::of(row.kind) else {
                self.counters.unknown_kinds += 1;
                self.events.touched = true;
                continue;
            };
            let (mut line, starts_segment) = if let Some(line) = self.open.remove(&row.handle) {
                (line, false)
            } else if let Some(line) = self.broken.remove(&row.handle) {
                (line, true)
            } else {
                // A handle this census already listed counts from the lifetime it began.
                let last = match still.get(&row.handle) {
                    Some(line) => Some(line.appearance),
                    None => {
                        let root = self.keys.root;
                        self.keys
                            .handles
                            .get(row.handle, root, ctx)?
                            .map(|handle| handle.appearance)
                    }
                };
                let appearance = last.unwrap_or(0) + 1;
                let lifetime = self.ids.lifetime;
                self.ids.lifetime += 1;
                self.events.overview.lives.push(index::Life {
                    id: lifetime,
                    handle: row.handle,
                    appearance,
                    kind: Some(kind),
                    before_recording: self.first_read,
                    between_reads: None,
                    variants: Vec::new(),
                });
                self.touch(std::mem::size_of::<index::Life>());
                (Line::new(lifetime, row.handle, appearance), true)
            };
            self.record_design(input, Dated::of(input), row.variant, ctx)?;
            let values = RawRun {
                first_hour: hour,
                last_hour: hour,
                starts_segment,
                variant: row.variant,
                mio: row.mio,
                active_factories: row.active_factories,
                requested_factories: row.requested_factories,
                max_factories: row.max_factories,
                cost_raw: row.cost_raw,
                speed_raw: row.speed_raw,
                efficiency_raw: row.efficiency_raw,
                priority: row.priority,
                amount: row.amount,
            };
            if let Some(country) = modifier_country
                .as_ref()
                .filter(|country| country.country == row.country)
                && let Some(operands) = input.modifier_lines.get(&row.handle)
                && (
                    operands.country,
                    operands.variant,
                    operands.organisation,
                    operands.kind,
                    operands.active_factories,
                ) == (
                    row.country,
                    row.variant,
                    row.mio,
                    row.kind,
                    row.active_factories,
                )
            {
                let organisation = input.organisation_names.get(&row.mio).cloned().flatten();
                let country = match self.country_at {
                    Some(at) => at,
                    None => {
                        self.events.countries.push(Arc::clone(country));
                        let at = (self.events.countries.len() - 1) as u32;
                        self.country_at = Some(at);
                        self.touch(modifier_country_bytes(country));
                        at
                    }
                };
                let repeated = line.operands.as_ref() == Some(operands);
                if !repeated {
                    line.operands = Some(operands.clone());
                }
                let bytes = std::mem::size_of::<index::ModifierRecord>()
                    + usize::from(!repeated) * line_operand_bytes(operands)
                    + organisation.as_ref().map_or(0, String::len);
                let records = &mut self
                    .events
                    .modifiers
                    .entry(line.lifetime)
                    .or_default()
                    .records;
                records.push(index::ModifierRecord {
                    first_hour: hour,
                    last_hour: hour,
                    closed: false,
                    country,
                    line: (!repeated).then(|| operands.clone()),
                    run: values,
                    organisation,
                });
                line.modifier = Some(LiveModifier {
                    last: hour,
                    at: At::New(records.len() - 1),
                });
                self.counters.modified = true;
                self.touch(bytes);
            }
            match line.run.take() {
                Some(mut run)
                    if !starts_segment
                        && run.run.last_hour.checked_add(1) == Some(hour)
                        && same_values(&run.run, &values) =>
                {
                    run.run.last_hour = hour;
                    line.run = Some(run);
                }
                previous => {
                    let previous_last = previous.as_ref().map(|run| run.run.last_hour);
                    let joins = !starts_segment
                        && previous_last.and_then(|last| last.checked_add(1)) == Some(hour)
                        && line.step.is_some_and(|step| {
                            (
                                step.variant,
                                step.active_factories,
                                step.requested_factories,
                            ) == (
                                values.variant,
                                values.active_factories,
                                values.requested_factories,
                            )
                        });
                    if let Some(run) = previous {
                        close_run(&mut self.events, run);
                    }
                    if !joins {
                        if let (Some(step), Some(last)) = (line.step.take(), previous_last) {
                            close_step(&mut self.events, step, last);
                        }
                        let id = self.ids.step;
                        self.ids.step += 1;
                        self.events.overview.steps.push(index::Step {
                            id,
                            lifetime: line.lifetime,
                            first_hour: hour,
                            last_hour: hour,
                            closed: false,
                            starts_segment,
                            variant: values.variant,
                            active_factories: values.active_factories,
                            requested_factories: values.requested_factories,
                        });
                        line.step = Some(LiveStep {
                            id,
                            variant: values.variant,
                            active_factories: values.active_factories,
                            requested_factories: values.requested_factories,
                            at: At::New(self.events.overview.steps.len() - 1),
                        });
                    }
                    let id = self.ids.run;
                    self.ids.run += 1;
                    let at = self.events.detail.runs.push(id, line.lifetime, &values);
                    self.events
                        .detail
                        .lives
                        .insert(line.lifetime, (line.handle, line.appearance));
                    if values.mio != 0 {
                        self.anchor_organisation(values.mio, ctx)?;
                    }
                    line.run = Some(LiveRun {
                        id,
                        run: values,
                        at: At::New(at),
                    });
                    self.touch(std::mem::size_of::<RawRun>() + 32);
                }
            }
            still.insert(row.handle, line);
        }
        let ended = std::mem::take(&mut self.open)
            .into_values()
            .chain(std::mem::take(&mut self.broken).into_values());
        for mut line in ended {
            self.events.overview.ends.push((line.lifetime, hour));
            close_line(&mut self.events, &mut line);
            self.closed(&line, ctx);
            self.events.touched = true;
        }
        self.active_variants = still
            .values()
            .filter_map(|line| line.run.as_ref().map(|run| run.run.variant))
            .collect();
        self.open = still;
        self.carried = true;
        self.first_read = false;
        self.output_open = false;
        let mut produced_between: BTreeSet<u64> = BTreeSet::new();
        if input.output_complete {
            self.output_open = true;
            mark(&mut self.events.overview.output_hours, hour);
            let mut by_line: BTreeMap<u64, Vec<(&ProductionLineOutputRow, bool)>> = BTreeMap::new();
            self.counters.unlinked_rows += input.unlinked_output.len();
            for (row, linked) in input
                .output
                .get(&slot)
                .into_iter()
                .flatten()
                .map(|row| (row, true))
                .chain(input.unlinked_output.iter().map(|row| (row, false)))
            {
                if !linked && self.handle(row.line, ctx)?.is_none() {
                    continue;
                }
                by_line.entry(row.line).or_default().push((row, linked));
                produced_between.insert(row.variant);
            }
            for (handle, mut rows) in by_line {
                rows.sort_by_key(|(row, _)| row.ordinal);
                let lifetime = match self.handle(handle, ctx)? {
                    Some(found) => found.lifetime,
                    None if rows.iter().all(|(row, _)| row.ended) => {
                        let whole_life = rows.iter().all(|(row, _)| row.observed_since_creation);
                        let since = rows
                            .iter()
                            .map(|(row, _)| row.since_hour)
                            .min()
                            .unwrap_or(hour);
                        let (first_hour, before) = match previous_read {
                            Some(read) if whole_life => (read + 1, false),
                            _ => (since.min(hour), true),
                        };
                        let lifetime = self.ids.lifetime;
                        self.ids.lifetime += 1;
                        let variants: Vec<u64> = rows.iter().map(|(row, _)| row.variant).collect();
                        produced_between.extend(variants.iter().copied());
                        self.events.overview.lives.push(index::Life {
                            id: lifetime,
                            handle,
                            appearance: 1,
                            kind: None,
                            before_recording: before,
                            between_reads: Some(HourSpan {
                                first_hour,
                                last_hour: hour,
                            }),
                            variants,
                        });
                        self.events.overview.ends.push((lifetime, hour));
                        self.keys.handles.put(
                            handle,
                            Handle {
                                lifetime,
                                appearance: 1,
                            },
                            ctx.generation,
                        );
                        self.dirty.handles.insert(handle);
                        self.touch(std::mem::size_of::<index::Life>());
                        lifetime
                    }
                    None => {
                        self.counters.unlisted_output += 1;
                        self.events.touched = true;
                        continue;
                    }
                };
                let output = output_of(&rows, hour);
                let bytes = output_bytes(&output);
                let ended = rows.iter().any(|(row, _)| row.ended);
                match self.open.get_mut(&handle).filter(|_| !ended) {
                    Some(line) => {
                        self.events
                            .overview
                            .output_variants
                            .extend(output.parts.iter().map(|part| part.variant));
                        let id = self.ids.output;
                        self.ids.output += 1;
                        self.events.detail.outputs.push(OutputRecord {
                            id,
                            lifetime,
                            output: output.clone(),
                            closed: false,
                        });
                        self.events
                            .detail
                            .lives
                            .insert(lifetime, (line.handle, line.appearance));
                        line.output = Some(LiveOutput {
                            id,
                            output,
                            at: At::New(self.events.detail.outputs.len() - 1),
                        });
                    }
                    None => self.events.overview.finals.push((lifetime, output)),
                }
                self.touch(bytes);
            }
        }
        self.organisations_open = false;
        if input.organisations_named {
            self.organisations_open = true;
            mark(&mut self.events.overview.organisation_hours, hour);
            let named: BTreeSet<u64> = mine
                .iter()
                .flat_map(|row| {
                    std::iter::once(row.mio)
                        .chain(input.designs.get(&row.variant).and_then(|read| read.author))
                })
                .filter(|mio| *mio != 0)
                .collect();
            for id in named {
                if let Some(raw) = input.organisations.get(&id) {
                    let mut raw = RawOrganisation {
                        first_hour: hour,
                        ..raw.clone()
                    };
                    if let Some(state) = &mut raw.state {
                        state.read_hour = input.organisation_read_hour;
                    }
                    self.organisation_version(id, raw, ctx)?;
                }
            }
        }
        let produced: BTreeSet<u64> = mine
            .iter()
            .map(|row| row.variant)
            .chain(produced_between)
            .collect();
        for id in produced {
            if let Some(icon) = input.icons.get(&id) {
                self.icon_version(id, hour, icon, ctx)?;
            }
        }
        Ok(())
    }
}

/// What a design read is dated by: its hour, the hour the designs were read, and whether its
/// authors' names are due.
#[derive(Clone, Copy)]
struct Dated {
    hour: i32,
    design_read_hour: Option<i32>,
    author_names_due: bool,
}

impl Dated {
    fn of(input: &HourInput) -> Self {
        Self {
            hour: input.hour,
            design_read_hour: input.design_read_hour,
            author_names_due: input.author_names_due,
        }
    }
}

/// Hours one input covers, with what was complete through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hours {
    first: i32,
    last: i32,
    lines_complete: bool,
    output_complete: bool,
    organisations_complete: bool,
    modifiers_complete: bool,
    /// The design revision at `last`.
    design_revision: u64,
}

impl Hours {
    fn of(input: &HourInput) -> Self {
        Self {
            first: input.hour,
            last: input.last_hour,
            lines_complete: input.lines_complete,
            output_complete: input.output_complete,
            organisations_complete: input.organisations_complete,
            modifiers_complete: input.modifiers_complete,
            design_revision: input.design_revision,
        }
    }

    /// Joins `next` when it follows on with the same availability.
    fn join(&mut self, next: &Self) -> bool {
        let joins = self.last.checked_add(1) == Some(next.first)
            && (
                self.lines_complete,
                self.output_complete,
                self.organisations_complete,
                self.modifiers_complete,
            ) == (
                next.lines_complete,
                next.output_complete,
                next.organisations_complete,
                next.modifiers_complete,
            );
        if joins {
            self.last = next.last;
            self.design_revision = next.design_revision;
        }
        joins
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NativeCountry {
    pub slot: u32,
    pub id: u32,
    pub epoch: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Alias {
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub country: NativeCountry,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Directory {
    pub branch: u64,
    pub coverage: crate::prepared::coverage::Coverage,
    pub last_block: Option<(i32, i32, u64)>,
    pub pictured: bool,
    /// Whether every file the icon content read is one the store key stamps
    /// ([`crate::products::equipment_icon::inputs_stamped`]).
    pub stamped: bool,
    pub visual: Option<String>,
    pub executable: bool,
    /// Whether modifier reads were prepared with the content's localisation.
    pub localised: bool,
    /// How many commits `production-commits` holds.
    pub commits: u64,
    /// Every tag the branch prepared a place for.
    pub tags: Vec<String>,
    /// Prepared from a native recording: its publisher's frontier, not `tags`, says when it is
    /// current, and the tags a later preparation names need no marking here.
    #[serde(default)]
    pub native: bool,
}

/// What icon rules read beyond the catalogue: only an hour that chooses icons depends on them.
pub(super) const PICTURED: [TableId; 5] = [
    TableId::PoliticalParties,
    TableId::Politics,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::States,
];

pub fn directory_section(branch: u64) -> String {
    format!("production-lines-directory-v3-{branch}")
}
pub fn coverage_section(branch: u64) -> String {
    format!("preparation-family-production-lines-{branch}")
}

/// When a dormant history stopped being followed, with the absence's counts then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sleep {
    pub hour: i32,
    pub reads: usize,
    pub stretches: usize,
    /// How many unread spans the absence had recorded.
    pub spans: u64,
}

/// One country's fold and where its events belong: a native lifetime, a tag's own
/// projection, or the absence every new one begins from. A fork reads its parent's chunks
/// before `parent.1` and its own from there.
#[derive(Clone)]
struct History {
    id: u64,
    parent: Option<(u64, u64)>,
    state: CountryState,
    sleep: Option<Sleep>,
    /// Where its whole state lies in `production-ledger` while it is put away; `state`
    /// then holds only its horizon and last overview.
    blob: Option<u64>,
}

impl History {
    fn dormant(&self, hour: i32) -> Sleep {
        Sleep {
            hour,
            reads: self.state.counters.unread_output_reads,
            stretches: self.state.counters.unreadable_stretches,
            spans: self.state.stream.as_ref().map_or(0, |stream| stream.count),
        }
    }

    fn wake(&mut self, absent: &History) -> io::Result<()> {
        let sleep = self.sleep.take();
        self.state.wake(sleep, &absent.state)
    }

    /// Brings a put-away history's state back from the ledger.
    fn load(&mut self, ctx: &mut Ctx<'_>) -> io::Result<()> {
        let Some(blob) = self.blob else {
            return Ok(());
        };
        let (pages, trees) = ctx.local()?;
        let bytes = pages
            .read_at(&trees.section, blob)?
            .ok_or_else(|| crate::invalid("Missing put-away production history"))?;
        let ledger: HistoryLedger = index::unpack(&bytes)?;
        if !matches!(ledger.state, StateLedger::Loaded { .. }) {
            return Err(crate::invalid(
                "A put-away production history names another",
            ));
        }
        let mut history = RestoredHistory::from(ledger).restore(self.id, false, ctx)?;
        history.state.fresh = true;
        history.state.changed = true;
        *self = history;
        Ok(())
    }

    /// Leaves only what a query reads of it, the rest lying at `blob`.
    fn put_away(&mut self, blob: u64) {
        self.state = CountryState {
            through: self.state.through,
            overview: self.state.overview,
            changed: true,
            fresh: true,
            ..CountryState::default()
        };
        self.blob = Some(blob);
    }
}

/// A copy of `from` (held, not put away) that goes on with its own events from the chunk
/// being built.
fn fork(next: &mut u64, cut: u64, from: &History) -> History {
    let mut history = from.clone();
    history.id = *next;
    *next += 1;
    history.parent = Some((from.id, cut));
    history.blob = None;
    history.state.stream = None;
    history.state.fresh = true;
    history.state.changed = true;
    history
}

#[derive(Clone)]
enum TagCountry {
    Absent,
    Native(NativeCountry),
    Projected(Box<History>),
}

impl TagCountry {
    fn projection(&self) -> Option<&History> {
        match self {
            Self::Projected(country) => Some(country),
            _ => None,
        }
    }

    fn projection_mut(&mut self) -> Option<&mut History> {
        match self {
            Self::Projected(country) => Some(country),
            _ => None,
        }
    }
}

/// One hour as the fold reads it: a row frame with its changes, or a native walk's facts of
/// the frame of its hour.
enum Observed<'a> {
    Frame {
        replay: &'a Replay,
        frame: &'a DecodedFrame,
        changes: &'a crate::prepared::frame_changes::FrameChanges,
    },
    Native(&'a mut super::columnar::Hour),
}

impl Observed<'_> {
    fn hour(&self) -> i32 {
        match self {
            Self::Frame { frame, .. } => frame.hour,
            Self::Native(hour) => hour.hour,
        }
    }

    fn quiet(&mut self, world: &mut World, evidence: bool) -> bool {
        match self {
            Self::Frame {
                replay,
                frame,
                changes,
            } => world.quiet(replay, frame, changes, evidence),
            Self::Native(hour) => world.quiet_native(hour, evidence),
        }
    }

    fn evidence(&self, world: &mut World) -> ReadEvidence {
        match self {
            Self::Frame { replay, frame, .. } => world.evidence(replay, frame),
            Self::Native(hour) => world.evidence_native(hour),
        }
    }

    fn identity_changed(&self, world: &World) -> bool {
        match self {
            Self::Frame { frame, changes, .. } => world.identity_changed(frame, changes),
            Self::Native(hour) => world.identity_changed_native(hour),
        }
    }

    fn extract(
        &mut self,
        world: &mut World,
        icons: Option<&IconContent>,
        previous: Option<HourInput>,
    ) -> io::Result<HourInput> {
        match self {
            Self::Frame {
                replay,
                frame,
                changes,
            } => world.extract_cached(replay, frame, icons, &BTreeSet::new(), previous, changes),
            Self::Native(hour) => world.extract_native(hour, icons, previous),
        }
    }
}

/// How a query reads one history: where its view begins and where what it carries ends.
#[derive(Clone, Debug)]
pub(super) struct HistoryView {
    pub id: u64,
    pub parent: Option<(u64, u64)>,
    pub sleep: Option<Sleep>,
    pub through: Option<i32>,
    pub overview: Option<u64>,
}

/// One publication: how many chunks it holds, the chunk table's entries after its last
/// full page (each chunk's range and hours), how many unread spans the absence recorded
/// and those after its last full page, and where the identities it publishes and the
/// ledger a resume applies lie.
#[derive(Clone, Debug)]
pub(super) struct Commit {
    pub chunks: u64,
    pub tail: Vec<(u64, i32, i32)>,
    pub spans: u64,
    pub span_tail: Vec<HourSpan>,
    pub through: Option<i32>,
    /// The last hour the variant catalogue observed.
    pub variants_hour: Option<i32>,
    /// The [`Identity`] record's range in `production-ledger`.
    pub identity: u64,
    /// The ledger records a resume applies, from the last full one.
    pub ledger: Vec<u64>,
}

/// What a query resolves a tag or a native country to, as a publication leaves it.
#[derive(Clone, Debug)]
pub(super) struct Identity {
    pub absent: u64,
    pub histories: Vec<HistoryView>,
    pub natives: Vec<(NativeCountry, u64)>,
    pub tags: Vec<(String, u64)>,
}

pub struct Prepared {
    world: World,
    absent: History,
    native: BTreeMap<NativeCountry, History>,
    retired: BTreeMap<NativeCountry, History>,
    aliases: BTreeMap<String, TagCountry>,
    dormant: BTreeMap<String, TagCountry>,
    active: BTreeMap<(u32, u32), NativeCountry>,
    links: BTreeMap<(u32, u32), NativeCountry>,
    epochs: BTreeMap<(u32, u32), u64>,
    tag_names: BTreeSet<String>,
    initial_tags: BTreeSet<String>,
    quiet: Option<(i32, i32)>,
    last_input: Option<HourInput>,
    evidence_ready: bool,
    evidence: Vec<ReadEvidence>,
    evidence_bytes: usize,
    evidence_base_bytes: usize,
    evidence_design_bytes: usize,
    evidence_organisation_bytes: usize,
    evidence_organisation_hour: Option<i32>,
    directory: Directory,
    aliases_spans: Vec<Alias>,
    alias_last: BTreeMap<String, usize>,
    processed: Option<BlockIndex>,
    block_failed: bool,
    /// Hours that read nothing and change no identity, which the countries have yet to follow.
    deferred: Vec<Hours>,
    /// Whether a deferred hour ended every country's carried lines.
    deferred_breaks: bool,
    /// The last hour and design revision the countries followed or deferred.
    followed: Option<(i32, u64)>,
    /// The countries a design change records while they carry the same lines: natives, and
    /// tags' own projections.
    designing: Option<(BTreeSet<NativeCountry>, BTreeSet<String>)>,
    next_history: u64,
    /// Chunks written.
    chunks: u64,
    /// The chunk table's entries after its last full page.
    table: Vec<(u64, i32, i32)>,
    trees: Trees,
    /// The last hour any country followed, and the last a written chunk covers.
    through: Option<i32>,
    flushed_through: Option<i32>,
    ledger_ranges: Vec<u64>,
    /// Something was observed since the last flush.
    pending: bool,
}

impl Prepared {
    pub fn advance_block<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        position: usize,
        icons: Option<&IconContent>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        crate::metadata::check_cancel(cancelled)?;
        let next = self.next_block(replay)?;
        let expected = replay
            .index
            .iter()
            .enumerate()
            .skip(next)
            .find(|(_, block)| block.branch == self.directory.branch);
        let Some((expected, block)) = expected else {
            return Err(crate::invalid("Production cursor is past the branch tail"));
        };
        if position != expected {
            return Err(crate::invalid(
                "Production cursor skips an unprocessed block",
            ));
        }
        let tables = Self::tables(icons.is_some());
        self.block_failed = true;
        let mut observer_failed = false;
        let pages: &mut dyn Pages = &mut Through(writer);
        let result = replay.read_block_recorded_alone(
            block,
            &tables,
            |updates, frame| {
                let _cpu = replay_frame::cpu::scope("fold observe", None);
                let changes = self
                    .world
                    .recorded_changes(replay, frame, (*updates).clone());
                if let Err(error) =
                    self.observe_frame(&mut *pages, replay, frame, icons, cancelled, &changes)
                {
                    observer_failed = true;
                    return Err(error);
                }
                Ok(())
            },
            || crate::metadata::check_cancel(cancelled),
        );
        match result {
            Ok(()) => {}
            Err(error) if observer_failed || error.kind() == io::ErrorKind::Interrupted => {
                return Err(error);
            }
            Err(error) => {
                let first = replay::container::ReplayStream::failed_hour(&error)
                    .unwrap_or(block.first_hour);
                self.drain_quiet(pages)?;
                self.lost(first, block.last_hour)?;
            }
        }
        self.processed = Some(block.clone());
        self.block_failed = false;
        Ok(())
    }

    pub fn new(
        replay: &Replay,
        branch: u64,
        tags: impl IntoIterator<Item = String>,
        icons: Option<&IconContent>,
    ) -> Self {
        let first_hour = branch_blocks(replay, branch)
            .first()
            .map_or(0, |block| block.first_hour);
        let tags: BTreeSet<_> = tags.into_iter().collect();
        Self {
            alias_last: BTreeMap::new(),
            aliases_spans: Vec::new(),
            processed: None,
            block_failed: false,
            deferred: Vec::new(),
            deferred_breaks: false,
            followed: None,
            designing: None,
            world: World::default(),
            absent: History {
                id: 1,
                parent: None,
                state: CountryState {
                    stream: Some(keys::Stream::default()),
                    ..CountryState::new()
                },
                sleep: None,
                blob: None,
            },
            native: BTreeMap::new(),
            retired: BTreeMap::new(),
            aliases: tags
                .iter()
                .map(|tag| (tag.clone(), TagCountry::Absent))
                .collect(),
            dormant: BTreeMap::new(),
            active: BTreeMap::new(),
            links: BTreeMap::new(),
            epochs: BTreeMap::new(),
            tag_names: BTreeSet::new(),
            initial_tags: tags,
            quiet: None,
            last_input: None,
            evidence_ready: false,
            evidence: Vec::new(),
            evidence_bytes: 0,
            evidence_base_bytes: 0,
            evidence_design_bytes: 0,
            evidence_organisation_bytes: 0,
            evidence_organisation_hour: None,
            directory: Directory {
                branch,
                coverage: crate::prepared::coverage::Coverage {
                    schema: crate::prepared::coverage::SCHEMA,
                    first_hour,
                    last_hour: first_hour,
                    complete: false,
                },
                last_block: None,
                pictured: icons.is_some(),
                stamped: icons.is_some_and(IconContent::stamped),
                visual: icons.map(|icons| icons.visual_inputs().to_owned()),
                executable: engine_executable::executable_tables::is_initialized(),
                localised: icons.is_some_and(|icons| icons.catalogue.is_some()),
                commits: 0,
                tags: Vec::new(),
                native: false,
            },
            next_history: 2,
            chunks: 0,
            table: Vec::new(),
            trees: Trees::new(branch),
            through: None,
            flushed_through: None,
            ledger_ranges: Vec::new(),
            pending: false,
        }
    }

    pub fn tables(pictured: bool) -> Vec<TableId> {
        let mut tables = vec![
            TableId::Campaign,
            TableId::ProductionLines,
            TableId::ProductionLineOutput,
            TableId::IndustrialOrganisations,
            TableId::IndustrialOrganisationDetails,
            TableId::IndustrialOrganisationPolicies,
            TableId::VariantStats,
            TableId::CountryProductionModifiers,
            TableId::ProductionLineModifiers,
            TableId::ModifierDefinitions,
        ];
        tables.extend(VariantCatalogue::TABLES);
        if pictured {
            tables.extend(PICTURED);
        }
        tables
    }

    #[cfg(test)]
    fn observe<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
    ) -> io::Result<bool> {
        let changes = self.world.changes(replay, frame);
        self.observe_changed(&mut Through(writer), replay, frame, icons, &changes)
    }
    #[cfg(test)]
    fn observe_changed(
        &mut self,
        pages: &mut dyn Pages,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<bool> {
        self.observe_hour(
            pages,
            &mut Observed::Frame {
                replay,
                frame,
                changes,
            },
            icons,
        )
    }

    /// Folds one hour, whatever it is read from; whether it was more than a quiet hour.
    fn observe_hour(
        &mut self,
        pages: &mut dyn Pages,
        observed: &mut Observed<'_>,
        icons: Option<&IconContent>,
    ) -> io::Result<bool> {
        let hour = observed.hour();
        if self
            .quiet
            .is_some_and(|(_, last)| last.checked_add(1) != Some(hour))
        {
            self.drain_quiet(pages)?;
        }
        if observed.quiet(&mut self.world, self.evidence_ready) {
            if self.evidence_ready {
                let mut evidence = observed.evidence(&mut self.world);
                let input = self.last_input.as_ref().expect("unchanged census operands");
                let previous = self.evidence.last();
                if evidence.lines_read
                    || evidence.output_read
                    || evidence.design_revision
                        != previous.map_or(input.design_revision, |read| read.design_revision)
                    || evidence.organisation_read_hour
                        != previous.map_or(input.organisation_read_hour, |read| {
                            read.organisation_read_hour
                        })
                {
                    let design_changed = evidence.design_revision
                        != previous.map_or(input.design_revision, |read| read.design_revision);
                    evidence.organisation_changed = evidence.lines_read
                        && input.organisations_named
                        && evidence.organisation_read_hour != self.evidence_organisation_hour;
                    let bytes = std::mem::size_of::<ReadEvidence>()
                        + usize::from(evidence.lines_read)
                            * (self.native.len() + self.aliases.len() + 1)
                            * std::mem::size_of::<i32>()
                        + usize::from(design_changed) * self.evidence_design_bytes
                        + usize::from(evidence.organisation_changed)
                            * self.evidence_organisation_bytes;
                    if crate::prepared::pages::publication_due(true, false, false, || {
                        (
                            self.evidence_base_bytes
                                .saturating_add(self.evidence_bytes)
                                .saturating_add(bytes),
                            0,
                        )
                    }) {
                        self.flush_into(pages, &AtomicBool::new(false))?;
                    }
                    self.evidence
                        .try_reserve_exact(1)
                        .map_err(io::Error::other)?;
                    self.evidence_bytes = self.evidence_bytes.saturating_add(bytes);
                    if evidence.lines_read {
                        self.evidence_organisation_hour = evidence.organisation_read_hour;
                    }
                    self.evidence.push(evidence);
                }
            }
            let first = self.quiet.map_or(hour, |span| span.0);
            self.quiet = Some((first, hour));
            return Ok(false);
        }
        if !self.evidence.is_empty() {
            self.drain_quiet(pages)?;
        }
        self.evidence_ready = false;
        let previous = self.last_input.as_ref().map(|input| {
            (
                input.lines_complete,
                input.output_complete,
                input.organisations_complete,
                input.modifiers_complete,
                input.design_revision,
            )
        });
        if observed.identity_changed(&self.world) {
            self.drain_quiet(pages)?;
        }
        let mut input = observed.extract(&mut self.world, icons, self.last_input.take())?;
        let signature = (
            input.lines_complete,
            input.output_complete,
            input.organisations_complete,
            input.modifiers_complete,
            input.design_revision,
        );
        if previous == Some(signature)
            && !input.identities_changed
            && !input.lines_read
            && !input.output_read
        {
            let first = self.quiet.map_or(input.hour, |span| span.0);
            self.quiet = Some((first, input.hour));
            self.last_input = Some(input);
            return Ok(false);
        }
        // Drain the earlier frontier before applying this hour's semantic edge.
        if let Some((first, last)) = self.quiet.take() {
            let saved = (
                input.hour,
                input.last_hour,
                input.lines_read,
                input.output_read,
                input.modifiers_read,
                input.identities_changed,
            );
            let availability = (
                input.lines_complete,
                input.output_complete,
                input.organisations_complete,
                input.modifiers_complete,
                input.design_revision,
            );
            input.hour = first;
            input.last_hour = last;
            input.lines_read = false;
            input.output_read = false;
            input.modifiers_read = false;
            input.identities_changed = false;
            if let Some(previous) = previous {
                input.lines_complete = previous.0;
                input.output_complete = previous.1;
                input.organisations_complete = previous.2;
                input.modifiers_complete = previous.3;
                input.design_revision = previous.4;
            }
            self.observe_input(&input, pages)?;
            input.hour = saved.0;
            input.last_hour = saved.1;
            input.lines_read = saved.2;
            input.output_read = saved.3;
            input.modifiers_read = saved.4;
            input.identities_changed = saved.5;
            input.lines_complete = availability.0;
            input.output_complete = availability.1;
            input.organisations_complete = availability.2;
            input.modifiers_complete = availability.3;
            input.design_revision = availability.4;
        }
        self.observe_input(&input, pages)?;
        // These censuses add dates and names, but no per-line output or modifier samples.
        self.evidence_ready = input.lines_read
            && input.lines_complete
            && input.rows_complete
            && input.countries.is_some()
            && input.unlinked_lines.is_empty()
            && input.output.is_empty()
            && input.unlinked_output.is_empty()
            && input.modifier_lines.is_empty()
            && (input.modifiers_read || !input.modifiers_complete)
            && input
                .lines
                .values()
                .flatten()
                .all(|row| LineKind::of(row.kind).is_some());
        if self.evidence_ready {
            self.evidence_base_bytes = self.pending_total();
            self.evidence_design_bytes = self
                .native
                .values()
                .chain(self.aliases.values().filter_map(TagCountry::projection))
                .flat_map(|country| &country.state.active_variants)
                .map(|id| input.design_bytes(*id))
                .sum();
            self.evidence_organisation_bytes = self
                .native
                .keys()
                .map(|key| key.slot)
                .chain(
                    self.aliases
                        .iter()
                        .filter(|(_, alias)| alias.projection().is_some())
                        .filter_map(|(tag, _)| {
                            input
                                .countries
                                .as_ref()?
                                .get(tag)?
                                .found()
                                .map(|(slot, _)| slot)
                        }),
                )
                .map(|slot| {
                    let rows = input
                        .lines
                        .get(&slot)
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let named: BTreeSet<_> = rows
                        .iter()
                        .flat_map(|row| {
                            std::iter::once(row.mio)
                                .chain(input.designs.get(&row.variant).and_then(|read| read.author))
                        })
                        .collect();
                    named
                        .iter()
                        .filter_map(|id| input.organisations.get(id))
                        .map(organisation_bytes)
                        .sum::<usize>()
                })
                .sum();
            self.evidence_organisation_hour = input.organisation_read_hour;
        }
        self.last_input = Some(input);
        Ok(true)
    }

    pub fn observe_into<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let changes = self.world.changes(replay, frame);
        self.observe_changed_into(writer, replay, frame, icons, cancelled, &changes)
    }
    pub fn observe_changed_into<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
        cancelled: &AtomicBool,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.observe_frame(
            &mut Through(writer),
            replay,
            frame,
            icons,
            cancelled,
            changes,
        )
    }

    /// The spellings a native walk read, which its resume takes up ([`super::columnar::Branch::save`]).
    pub(super) fn spellings(&self) -> &BTreeMap<u32, String> {
        self.world.spellings()
    }

    pub(super) fn take_spellings(&mut self, spellings: BTreeMap<u32, String>) {
        self.world.take_spellings(spellings);
    }

    /// Folds one hour of a native recording ([`super::columnar`]): its facts are what the fold
    /// reads of the frame of its hour.
    pub fn observe_facts<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        mut hour: super::columnar::Hour,
        icons: Option<&IconContent>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.world.native_hour(&mut hour);
        self.observe_due(
            &mut Through(writer),
            &mut Observed::Native(&mut hour),
            icons,
            cancelled,
        )
    }

    fn observe_frame(
        &mut self,
        pages: &mut dyn Pages,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
        cancelled: &AtomicBool,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.observe_due(
            pages,
            &mut Observed::Frame {
                replay,
                frame,
                changes,
            },
            icons,
            cancelled,
        )
    }

    /// Folds one hour, and flushes once what it holds is due.
    fn observe_due(
        &mut self,
        pages: &mut dyn Pages,
        observed: &mut Observed<'_>,
        icons: Option<&IconContent>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if !self.observe_hour(pages, observed, icons)? {
            if crate::prepared::pages::publication_due(true, false, false, || {
                (self.world.pending_bytes + self.evidence_bytes, 0)
            }) {
                self.flush_into(pages, cancelled)?;
            }
            return Ok(());
        }
        let bytes = self.pending_total();
        if crate::prepared::pages::publication_due(true, false, false, || (bytes, 0)) {
            self.flush_into(pages, cancelled)?;
        }
        Ok(())
    }

    /// Every history the fold holds, live or dormant.
    fn histories(&self) -> impl Iterator<Item = &History> {
        std::iter::once(&self.absent)
            .chain(self.native.values())
            .chain(self.retired.values())
            .chain(self.aliases.values().filter_map(TagCountry::projection))
            .chain(self.dormant.values().filter_map(TagCountry::projection))
    }

    /// The bytes the next chunk holds: every history's pending events and the kept aliases.
    fn pending_total(&self) -> usize {
        self.world.pending_bytes
            + self
                .histories()
                .map(|history| history.state.pending_bytes())
                .sum::<usize>()
            + self
                .aliases_spans
                .iter()
                .map(|alias| std::mem::size_of::<Alias>() + alias.tag.capacity())
                .sum::<usize>()
    }

    /// Drains the quiet hours, and the countries follow every hour deferred.
    fn drain_quiet(&mut self, pages: &mut dyn Pages) -> io::Result<()> {
        if let Some((first, last)) = self.quiet.take()
            && let Some(mut input) = self.last_input.take()
        {
            input.hour = first;
            input.last_hour = last;
            input.lines_read = false;
            input.output_read = false;
            input.modifiers_read = false;
            input.identities_changed = false;
            self.observe_input(&input, pages)?;
            let evidence = std::mem::take(&mut self.evidence);
            self.evidence_base_bytes = self.evidence_base_bytes.saturating_add(self.evidence_bytes);
            self.evidence_bytes = 0;
            if !evidence.is_empty() {
                self.follow_deferred(input.countries.as_ref());
                let reads = evidence.iter().any(|read| read.lines_read);
                let mut work = Vec::with_capacity(self.native.len() + self.aliases.len() + 1);
                for (key, country) in &mut self.native {
                    let selected = if input.linked.contains(&(key.slot, key.id)) {
                        Country::Found(key.slot, key.id)
                    } else {
                        Country::Unresolved
                    };
                    let weight = weigh(country, reads);
                    work.push((country, selected, weight));
                }
                for (tag, alias) in &mut self.aliases {
                    if let Some(country) = alias.projection_mut() {
                        let selected = input
                            .countries
                            .as_ref()
                            .and_then(|countries| countries.get(tag))
                            .copied()
                            .unwrap_or(Country::Absent);
                        let weight = weigh(country, reads);
                        work.push((country, selected, weight));
                    }
                }
                work.push((&mut self.absent, Country::Absent, 1));
                each(
                    &mut *pages,
                    &mut self.trees,
                    self.chunks,
                    work,
                    |history, selected, ctx| {
                        history
                            .state
                            .observe_evidence(&input, selected, &evidence, ctx)
                    },
                )?;
                // The input as the last read left it.
                if let Some(read) = evidence.last() {
                    input.hour = read.hour;
                    input.lines_read = read.lines_read;
                    input.output_read = read.output_read;
                    input.modifiers_read = read.modifiers_read;
                    input.design_revision = read.design_revision;
                    input.design_read_hour = read.design_read_hour;
                    input.organisation_read_hour = read.organisation_read_hour;
                    input.author_names_due = read.author_names_due;
                }
                self.followed = Some((last, input.design_revision));
            }
            input.hour = last;
            input.last_hour = last;
            input.lines_read = false;
            input.output_read = false;
            input.modifiers_read = false;
            self.last_input = Some(input);
        }
        let input = self.last_input.take();
        self.follow_deferred(input.as_ref().and_then(|input| input.countries.as_ref()));
        self.last_input = input;
        Ok(())
    }

    /// Defers an input that reads nothing and changes no identity; its design change records now.
    fn defer(&mut self, input: &HourInput, pages: &mut dyn Pages) -> io::Result<bool> {
        let Some((through, revision)) = self.followed else {
            return Ok(false);
        };
        if input.lines_read
            || input.output_read
            || input.identities_changed
            || (input.rows_complete && self.active.is_empty() && !input.holders.is_empty())
        {
            return Ok(false);
        }
        // Such an hour ends every carried country's lines; none carries any again before a read.
        self.deferred_breaks |= !input.lines_complete || through.checked_add(1) != Some(input.hour);
        if input.design_revision != revision && !self.deferred_breaks {
            let (natives, tags) = self.designing.get_or_insert_with(|| {
                let natives = self
                    .native
                    .iter()
                    .filter(|(_, country)| country.state.designs_carried())
                    .map(|(key, _)| key.clone())
                    .collect();
                let tags = self
                    .aliases
                    .iter()
                    .filter(|(_, alias)| {
                        alias
                            .projection()
                            .is_some_and(|country| country.state.designs_carried())
                    })
                    .map(|(tag, _)| tag.clone())
                    .collect();
                (natives, tags)
            });
            let mut work = Vec::with_capacity(natives.len() + tags.len());
            for (key, country) in &mut self.native {
                if natives.contains(key) {
                    let weight = 1 + country.state.active_variants.len();
                    work.push((country, (), weight));
                }
            }
            for (tag, alias) in &mut self.aliases {
                if tags.contains(tag)
                    && let Some(country) = alias.projection_mut()
                {
                    let weight = 1 + country.state.active_variants.len();
                    work.push((country, (), weight));
                }
            }
            each(
                &mut *pages,
                &mut self.trees,
                self.chunks,
                work,
                |history, (), ctx| {
                    crate::prepared::prepass::note("production-lines.country_design");
                    history
                        .state
                        .record_designs(input, Dated::of(input), ctx)
                },
            )?;
        }
        let hours = Hours::of(input);
        if !self
            .deferred
            .last_mut()
            .is_some_and(|last| last.join(&hours))
        {
            self.deferred.push(hours);
        }
        self.followed = Some((input.last_hour, input.design_revision));
        self.through = Some(input.last_hour);
        Ok(true)
    }

    /// The countries follow the hours deferred since they last did.
    fn follow_deferred(&mut self, countries: Option<&BTreeMap<String, Country>>) {
        self.designing = None;
        self.deferred_breaks = false;
        if self.deferred.is_empty() {
            return;
        }
        let runs = std::mem::take(&mut self.deferred);
        for hours in &runs {
            self.follow_tags(countries, false, hours.first, hours.last);
            self.put_dormant(countries, hours.last);
        }
        for country in self.native.values_mut().chain(
            self.aliases
                .values_mut()
                .filter_map(TagCountry::projection_mut),
        ) {
            country.state.follow(&runs);
        }
        self.absent.state.follow(&runs);
    }

    fn initialize_tags(&mut self) {
        self.tag_names.append(&mut self.initial_tags);
    }

    pub fn reconcile_tags<'a, W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        tags: impl IntoIterator<Item = &'a str>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.next_block(replay)?;
        let writer: &mut dyn Pages = &mut Through(writer);
        let result = (|| {
            crate::metadata::check_cancel(cancelled)?;
            self.directory.coverage.complete &=
                self.directory.last_block == last_block(replay, self.directory.branch);
            let mut changed = false;
            for tag in tags {
                crate::metadata::check_cancel(cancelled)?;
                changed |= self.tag_names.insert(tag.to_owned());
            }
            crate::metadata::check_cancel(cancelled)?;
            if !self.directory.coverage.complete
                && let Some(block) = self.processed.clone().filter(|block| {
                    Some((block.first_hour, block.last_hour, block.offset))
                        == last_block(replay, self.directory.branch)
                })
            {
                self.publish_into(writer, replay, &block, cancelled)?;
            } else if changed {
                self.write_directory(writer)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.block_failed = true;
        }
        result
    }

    fn observe_input(&mut self, input: &HourInput, pages: &mut dyn Pages) -> io::Result<()> {
        self.initialize_tags();
        self.pending = true;
        if self.defer(input, pages)? {
            return Ok(());
        }
        if input.identities_changed && !self.deferred.is_empty() {
            return Err(crate::invalid(
                "Production countries changed before following their deferred hours",
            ));
        }
        self.follow_deferred(input.countries.as_ref());
        self.followed = Some((input.last_hour, input.design_revision));
        self.through = Some(input.last_hour);
        let mut born = BTreeSet::new();
        if input.rows_complete && (input.identities_changed || self.active.is_empty()) {
            let mut active = BTreeMap::new();
            for (&(slot, id), tag) in &input.holders {
                let existing = self.active.get(&(slot, id)).cloned();
                let previous = if existing.is_none() {
                    self.links.get(&(slot, id)).cloned()
                } else {
                    None
                };
                let key = match existing.or(previous) {
                    Some(key) => key,
                    None => {
                        let epoch = self.epochs.entry((slot, id)).or_default();
                        *epoch += 1;
                        NativeCountry {
                            slot,
                            id,
                            epoch: *epoch,
                        }
                    }
                };
                if !self.native.contains_key(&key) {
                    let mut country = match self.retired.remove(&key) {
                        Some(country) => country,
                        None => {
                            born.insert(key.clone());
                            fork(&mut self.next_history, self.chunks, &self.absent)
                        }
                    };
                    let mut ctx = Ctx::new(&mut *pages, &mut self.trees, self.chunks);
                    country.load(&mut ctx)?;
                    country.wake(&self.absent)?;
                    self.native.insert(key.clone(), country);
                }
                self.links.insert((slot, id), key.clone());
                active.insert((slot, id), key);
                let Some(tag) = tag else {
                    continue;
                };
                if !self.aliases.contains_key(tag) {
                    let mut alias = self.dormant.remove(tag).unwrap_or(TagCountry::Absent);
                    if let Some(country) = alias.projection_mut() {
                        let mut ctx = Ctx::new(&mut *pages, &mut self.trees, self.chunks);
                        country.load(&mut ctx)?;
                        country.wake(&self.absent)?;
                    }
                    self.aliases.insert(tag.clone(), alias);
                }
                self.tag_names.insert(tag.clone());
            }
            self.active = active;
        }
        self.follow_tags(
            input.countries.as_ref(),
            input.identities_changed,
            input.hour,
            input.last_hour,
        );
        for (tag, alias) in &mut self.aliases {
            let selected = input
                .countries
                .as_ref()
                .map(|countries| countries.get(tag).copied().unwrap_or(Country::Absent));
            let native = selected
                .and_then(Country::found)
                .and_then(|identity| self.active.get(&identity));
            match &*alias {
                TagCountry::Native(key)
                    if native == Some(key)
                        || input.countries.is_none()
                        || (selected == Some(Country::Absent)
                            && self
                                .active
                                .get(&(key.slot, key.id))
                                .is_none_or(|active| active != key)) =>
                {
                    continue;
                }
                TagCountry::Absent if matches!(selected, None | Some(Country::Absent)) => continue,
                TagCountry::Absent if native.is_some_and(|key| born.contains(key)) => {
                    *alias = TagCountry::Native(native.unwrap().clone());
                    continue;
                }
                TagCountry::Projected(_) => continue,
                _ => {}
            }
            // A literal tag diverges at renames/reuse; retain its history without changing native identity.
            let mut ctx = Ctx::new(&mut *pages, &mut self.trees, self.chunks);
            let mut country = match &*alias {
                TagCountry::Absent => fork(&mut self.next_history, self.chunks, &self.absent),
                TagCountry::Native(key) => match self.native.get(key) {
                    Some(from) => fork(&mut self.next_history, self.chunks, from),
                    None => {
                        let mut from = self
                            .retired
                            .get(key)
                            .ok_or_else(|| crate::invalid("Missing retired production country"))?
                            .clone();
                        from.load(&mut ctx)?;
                        fork(&mut self.next_history, self.chunks, &from)
                    }
                },
                TagCountry::Projected(_) => unreachable!(),
            };
            country.wake(&self.absent)?;
            *alias = TagCountry::Projected(Box::new(country));
        }
        let mut work = Vec::with_capacity(self.native.len() + self.aliases.len() + 1);
        for (key, country) in &mut self.native {
            let selected = input.countries.as_ref().map(|_| {
                if self.active.get(&(key.slot, key.id)) == Some(key) {
                    if input.linked.contains(&(key.slot, key.id)) {
                        Country::Found(key.slot, key.id)
                    } else {
                        Country::Unresolved
                    }
                } else {
                    Country::Absent
                }
            });
            let weight = weigh(country, input.lines_read);
            work.push((country, selected, weight));
        }
        for (tag, alias) in &mut self.aliases {
            let Some(country) = alias.projection_mut() else {
                continue;
            };
            let selected = input
                .countries
                .as_ref()
                .map(|countries| countries.get(tag).copied().unwrap_or(Country::Absent));
            let weight = weigh(country, input.lines_read);
            work.push((country, selected, weight));
        }
        let absent = input.countries.as_ref().map(|_| Country::Absent);
        work.push((&mut self.absent, absent, 1));
        each(
            pages,
            &mut self.trees,
            self.chunks,
            work,
            |history, selected, ctx| history.state.observe_input(input, selected, ctx),
        )?;
        let retired: Vec<_> = self
            .native
            .keys()
            .filter(|key| self.active.get(&(key.slot, key.id)) != Some(*key))
            .cloned()
            .collect();
        for key in retired {
            let mut country = self.native.remove(&key).unwrap();
            country.state.materialize();
            country.sleep = Some(self.absent.dormant(input.last_hour));
            country.state.changed = true;
            self.retired.insert(key, country);
        }
        self.put_dormant(input.countries.as_ref(), input.last_hour);
        Ok(())
    }

    /// Each held tag's identity alias through `first..=last`, and a place for every tag.
    fn follow_tags(
        &mut self,
        countries: Option<&BTreeMap<String, Country>>,
        identities_changed: bool,
        first: i32,
        last: i32,
    ) {
        let Some(countries) = countries else {
            return;
        };
        for (tag, country) in countries {
            if identities_changed {
                self.tag_names.insert(tag.clone());
            }
            if !self.aliases.contains_key(tag) {
                // A tag listed again holds no earlier dormant history: its next save is absent.
                self.dormant.remove(tag);
                self.aliases.insert(tag.clone(), TagCountry::Absent);
            }
            if let Country::Found(slot, id) = country
                && let Some(native) = self.active.get(&(*slot, *id))
            {
                let previous = self
                    .alias_last
                    .get(tag)
                    .copied()
                    .and_then(|index| self.aliases_spans.get_mut(index));
                if let Some(previous) = previous.filter(|previous| {
                    previous.country == *native && previous.last_hour.checked_add(1) == Some(first)
                }) {
                    previous.last_hour = last;
                } else {
                    self.alias_last
                        .insert(tag.clone(), self.aliases_spans.len());
                    self.aliases_spans.push(Alias {
                        tag: tag.clone(),
                        first_hour: first,
                        last_hour: last,
                        country: native.clone(),
                    });
                }
            }
        }
    }

    /// Puts away the tags no country holds through `last_hour`.
    fn put_dormant(&mut self, countries: Option<&BTreeMap<String, Country>>, last_hour: i32) {
        let dormant: Vec<_> = self
            .aliases
            .keys()
            .filter(|tag| {
                countries.is_some_and(|countries| {
                    matches!(countries.get(*tag), None | Some(Country::Absent))
                })
            })
            .cloned()
            .collect();
        for tag in dormant {
            let mut alias = self.aliases.remove(&tag).unwrap();
            if let Some(country) = alias.projection_mut() {
                country.state.materialize();
                country.sleep = Some(self.absent.dormant(last_hour));
                country.state.changed = true;
            }
            self.dormant.insert(tag, alias);
        }
    }

    pub fn lost(&mut self, first: i32, last: i32) -> io::Result<()> {
        self.pending = true;
        self.absent.state.lost(first, last)?;
        for country in self.native.values_mut().chain(
            self.aliases
                .values_mut()
                .filter_map(TagCountry::projection_mut),
        ) {
            country.state.lost(first, last)?;
        }
        Ok(())
    }

    /// Appends one chunk of everything recorded since the flush before. Each history's
    /// parts are encoded on spare cores; the ordered work only moves them and appends.
    #[cfg(test)]
    fn flush<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.flush_into(&mut Through(writer), cancelled)
    }

    fn flush_into(&mut self, pages: &mut dyn Pages, cancelled: &AtomicBool) -> io::Result<()> {
        crate::metadata::check_cancel(cancelled)?;
        self.drain_quiet(pages)?;
        self.initialize_tags();
        if !std::mem::take(&mut self.pending) {
            return Ok(());
        }
        let _cpu = replay_frame::cpu::scope("fold flush", None);
        let ordinal = self.chunks;
        let taken: Vec<(u64, Events)> = {
            let _phase = replay_frame::cpu::scope_phase("production-lines:events");
            let mut work = Vec::new();
            let live = std::iter::once(&mut self.absent)
                .chain(self.native.values_mut())
                .chain(
                    self.aliases
                        .values_mut()
                        .filter_map(TagCountry::projection_mut),
                );
            for history in live {
                let weight = weigh(history, true);
                work.push((history, true, weight));
            }
            let dormant = self.retired.values_mut().chain(
                self.dormant
                    .values_mut()
                    .filter_map(TagCountry::projection_mut),
            );
            for history in dormant {
                let weight = weigh(history, true);
                work.push((history, false, weight));
            }
            // A dormant history is materialized only where it has events to write.
            each(
                &mut *pages,
                &mut self.trees,
                ordinal + 1,
                work,
                |history, live, ctx| {
                    if live {
                        history.state.materialize();
                    }
                    if history.state.events.is_empty() {
                        return Ok(None);
                    }
                    if !live {
                        history.state.materialize();
                    }
                    history.state.overview = Some(ordinal);
                    let events = history.state.take_events(live, ctx)?;
                    Ok(Some((history.id, events)))
                },
            )?
            .into_iter()
            .flatten()
            .collect()
        };
        let variants: BTreeMap<u64, Vec<equipment::VariantVersion>> = self
            .world
            .take_flushed()
            .into_iter()
            .filter_map(|id| {
                let versions = self.world.variants.versions.get_mut(&id)?;
                let kept = versions.clone();
                tail(versions);
                Some((id, kept))
            })
            .collect();
        let icons = std::mem::take(&mut self.world.icon_events);
        let mut globals = Vec::new();
        if !variants.is_empty() {
            globals.push(index::encode_part(
                index::GLOBAL,
                index::Part::Variants,
                0,
                &variants,
            )?);
        }
        if !icons.is_empty() {
            globals.push(index::encode_part(
                index::GLOBAL,
                index::Part::IconEvents,
                0,
                &icons,
            )?);
        }
        if !self.aliases_spans.is_empty() {
            globals.push(index::encode_part(
                index::GLOBAL,
                index::Part::Aliases,
                0,
                &self.aliases_spans,
            )?);
        }
        if !taken.is_empty() || !globals.is_empty() {
            let parts = {
                let _phase = replay_frame::cpu::scope_phase("production-lines:encode");
                encode_histories(taken, globals)?
            };
            let _phase = replay_frame::cpu::scope_phase("production-lines:append");
            let bytes = index::chunk(parts)?;
            let branch = self.directory.branch;
            let range = pages.write(&index::index_section(branch), &bytes)?;
            let first = self
                .flushed_through
                .map_or(self.directory.coverage.first_hour, |hour| {
                    hour.saturating_add(1)
                });
            let last = self
                .through
                .unwrap_or(first.saturating_sub(1))
                .max(first.saturating_sub(1));
            self.table.push((range, first, last));
            self.chunks += 1;
            self.flushed_through = Some(last);
            if self.table.len() == index::CHUNK_PAGE {
                // Its range is its number, whatever an earlier preparation left past it.
                let page = self.chunks / index::CHUNK_PAGE as u64 - 1;
                let table = index::encode_chunks(&self.table);
                pages.write_at(&index::chunks_section(branch), page, &table)?;
                self.table.clear();
            }
        }
        let mut last = BTreeMap::new();
        for alias in std::mem::take(&mut self.aliases_spans) {
            last.insert(alias.tag.clone(), alias);
        }
        self.aliases_spans = last
            .into_values()
            .filter(|alias| self.aliases.contains_key(&alias.tag))
            .collect();
        self.alias_last = self
            .aliases_spans
            .iter()
            .enumerate()
            .map(|(index, alias)| (alias.tag.clone(), index))
            .collect();
        self.put_away(pages, false)?;
        self.world.pending_bytes = 0;
        self.evidence_base_bytes = self.pending_total();
        Ok(())
    }

    /// Every history the fold holds, put away or not.
    fn histories_mut(&mut self) -> impl Iterator<Item = &mut History> {
        std::iter::once(&mut self.absent)
            .chain(self.native.values_mut())
            .chain(self.retired.values_mut())
            .chain(
                self.aliases
                    .values_mut()
                    .filter_map(TagCountry::projection_mut),
            )
            .chain(
                self.dormant
                    .values_mut()
                    .filter_map(TagCountry::projection_mut),
            )
    }

    /// The histories whose state the fold holds.
    fn loaded(&self) -> impl Iterator<Item = &History> {
        self.histories().filter(|history| history.blob.is_none())
    }

    /// Pages the absence's full unread spans; sweeps the keyed values no read asked for since
    /// the sweep before into their trees, when `sweep` or once the loaded histories hold more
    /// than [`RESIDENT_KEYS`]; and puts each dormant history away whole.
    fn put_away(&mut self, pages: &mut dyn Pages, sweep: bool) -> io::Result<()> {
        if let Some(stream) = &mut self.absent.state.stream {
            stream.page(pages, &self.trees.absence)?;
        }
        let generation = self.trees.generation;
        let sweep = sweep
            || self
                .loaded()
                .map(|history| history.state.keys.len())
                .sum::<usize>()
                > RESIDENT_KEYS;
        // Where the values read since the last sweep pass the bound alone, every value goes.
        let all = sweep
            && self
                .loaded()
                .map(|history| history.state.keys.warm(generation))
                .sum::<usize>()
                > RESIDENT_KEYS;
        let mut out = keys::Builder::default();
        let trees = &mut self.trees;
        if sweep {
            let live = std::iter::once(&mut self.absent)
                .chain(self.native.values_mut())
                .chain(
                    self.aliases
                        .values_mut()
                        .filter_map(TagCountry::projection_mut),
                );
            for history in live {
                if history
                    .state
                    .keys
                    .sweep(generation, all, pages, trees, &mut out)?
                {
                    // Its earlier records hold values its tree now holds instead.
                    history.state.fresh = true;
                }
            }
        }
        let dormant = self.retired.values_mut().chain(
            self.dormant
                .values_mut()
                .filter_map(TagCountry::projection_mut),
        );
        for history in dormant.filter(|history| history.blob.is_none()) {
            history
                .state
                .keys
                .sweep(generation, true, pages, trees, &mut out)?;
        }
        if let Some(range) = out.write(pages, &trees.section)? {
            for history in self.histories_mut() {
                history.state.keys.root = keys::placed(history.state.keys.root, range);
            }
        }
        let dormant = self.retired.values_mut().chain(
            self.dormant
                .values_mut()
                .filter_map(TagCountry::projection_mut),
        );
        for history in dormant.filter(|history| history.blob.is_none()) {
            let ledger = history.ledger(true)?;
            let blob = pages.write(&self.trees.section, &index::pack(&ledger)?)?;
            history.put_away(blob);
        }
        if sweep {
            self.trees.generation += 1;
        }
        Ok(())
    }

    pub fn publish<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        block: &BlockIndex,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.publish_into(&mut Through(writer), replay, block, cancelled)
    }

    fn publish_into(
        &mut self,
        pages: &mut dyn Pages,
        replay: &Replay,
        block: &BlockIndex,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.next_block(replay)?;
        if self.processed.as_ref() != Some(block) {
            return Err(crate::invalid(
                "Production publication exceeds processed blocks",
            ));
        }
        let _cpu = replay_frame::cpu::scope("fold publish", None);
        self.flush_into(pages, cancelled)?;
        let branch = self.directory.branch;
        self.directory.last_block = Some((block.first_hour, block.last_hour, block.offset));
        self.directory.coverage.last_hour = block.last_hour;
        self.directory.coverage.complete = self.directory.last_block == last_block(replay, branch);
        self.commit(pages)?;
        pages.write_at(
            &coverage_section(branch),
            0,
            &serde_json::to_vec(&self.directory.coverage)?,
        )
    }

    /// Publishes what the fold holds of a native recording's hours through `last_hour`, as
    /// [`Self::publish`] does through a row block: `last_block` is the replay's last block of
    /// the branch once its every hour is folded. The coverage page is the native publisher's.
    pub fn publish_native<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        last_hour: Option<i32>,
        last_block: Option<(i32, i32, u64)>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let pages: &mut dyn Pages = &mut Through(writer);
        self.flush_into(pages, cancelled)?;
        if let Some(hour) = last_hour {
            self.directory.coverage.last_hour = hour;
        }
        self.directory.last_block = last_block;
        self.directory.coverage.complete = last_block.is_some();
        self.directory.native = true;
        self.commit(pages)
    }

    fn write_directory(&mut self, pages: &mut dyn Pages) -> io::Result<()> {
        self.directory.tags = self.tag_names.iter().cloned().collect();
        crate::prepared::series::write_reply(
            pages,
            &directory_section(self.directory.branch),
            0,
            &self.directory,
        )
    }

    /// Flushes and commits, as a publication does without a new block.
    #[cfg(test)]
    fn save<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if self.block_failed {
            return Err(crate::invalid("Production block did not finish"));
        }
        let pages: &mut dyn Pages = &mut Through(writer);
        self.flush_into(pages, cancelled)?;
        self.commit(pages)
    }

    /// Commits what the chunks written so far hold, with the ledger record a resume
    /// continues from.
    fn commit(&mut self, pages: &mut dyn Pages) -> io::Result<()> {
        if self.block_failed {
            return Err(crate::invalid("Production block did not finish"));
        }
        let _cpu = replay_frame::cpu::scope("fold save", None);
        let branch = self.directory.branch;
        let stream = self
            .absent
            .state
            .stream
            .as_mut()
            .ok_or_else(|| crate::invalid("The production absence keeps no unread spans"))?;
        stream.page(pages, &self.trees.absence)?;
        let (spans, span_tail) = (stream.count, stream.tail.clone());
        let full = self.ledger_ranges.is_empty() || self.ledger_ranges.len() >= LEDGER_FULL_EVERY;
        if full {
            self.put_away(pages, true)?;
        }
        let record = self.ledger_record(full)?;
        let range = pages.write(&index::ledger_section(branch), &index::pack(&record)?)?;
        if full {
            self.ledger_ranges.clear();
        }
        self.ledger_ranges.push(range);
        let identity = Identity {
            absent: self.absent.id,
            histories: self
                .histories()
                .map(|history| HistoryView {
                    id: history.id,
                    parent: history.parent,
                    sleep: history.sleep,
                    through: history.state.through,
                    overview: history.state.overview,
                })
                .collect(),
            natives: self
                .native
                .iter()
                .chain(&self.retired)
                .map(|(key, history)| (key.clone(), history.id))
                .collect(),
            tags: self
                .aliases
                .iter()
                .chain(&self.dormant)
                .map(|(tag, alias)| {
                    let history = match alias {
                        TagCountry::Absent => self.absent.id,
                        TagCountry::Native(key) => self
                            .native
                            .get(key)
                            .or_else(|| self.retired.get(key))
                            .map_or(self.absent.id, |history| history.id),
                        TagCountry::Projected(history) => history.id,
                    };
                    (tag.clone(), history)
                })
                .collect(),
        };
        let identity = pages.write(&index::ledger_section(branch), &index::pack(&identity)?)?;
        let commit = Commit {
            chunks: self.chunks,
            tail: self.table.clone(),
            spans,
            span_tail,
            through: self.through,
            variants_hour: self.world.variants.last_hour(),
            identity,
            ledger: self.ledger_ranges.clone(),
        };
        let range = pages.write(&index::commits_section(branch), &index::pack(&commit)?)?;
        self.directory.commits = range + 1;
        self.write_directory(pages)
    }

    pub fn resume(
        store: &store::Store,
        key: &store::SourceKey,
        replay: &Replay,
        branch: u64,
        icons: Option<&IconContent>,
    ) -> io::Result<Option<Self>> {
        if !binary_current(store, key, branch)? {
            return Ok(None);
        }
        let Some(directory) = directory(store, key, branch)? else {
            return Ok(None);
        };
        if directory.coverage.schema != crate::prepared::coverage::SCHEMA
            || directory.executable != engine_executable::executable_tables::is_initialized()
            || !pictures_hold_with(icons, directory.pictured, directory.visual.as_deref())
            || !replay.index.iter().any(|block| {
                block.branch == branch
                    && Some((block.first_hour, block.last_hour, block.offset))
                        == directory.last_block
            })
        {
            return Ok(None);
        }
        Self::restore_from(
            directory,
            replay,
            &mut index::StoreRanges::new(store, key.clone()),
        )
        .map(Some)
    }

    fn restore_from(
        directory: Directory,
        replay: &Replay,
        pages: &mut dyn Pages,
    ) -> io::Result<Self> {
        let branch = directory.branch;
        let processed = replay
            .index
            .iter()
            .find(|block| {
                block.branch == branch
                    && Some((block.first_hour, block.last_hour, block.offset))
                        == directory.last_block
            })
            .cloned()
            .ok_or_else(|| crate::invalid("Production cursor differs from recording prefix"))?;
        Self::restored(directory, Some(processed), replay, pages)
    }

    /// The fold of a native recording's hours going on from its last commit, which `directory`
    /// names: it follows no row block.
    pub fn resume_native<W: Pages + ?Sized>(
        directory: Directory,
        replay: &Replay,
        writer: &mut W,
    ) -> io::Result<Self> {
        Self::restored(directory, None, replay, &mut Through(writer))
    }

    /// The fold `directory`'s commit left, after the row block `processed` when it followed one.
    fn restored(
        mut directory: Directory,
        processed: Option<BlockIndex>,
        replay: &Replay,
        pages: &mut dyn Pages,
    ) -> io::Result<Self> {
        let branch = directory.branch;
        let (_, commit) = read::commit(pages, branch, directory.commits)?;
        let mut restored = Restored::default();
        for &range in &commit.ledger {
            let bytes = pages
                .read_at(&index::ledger_section(branch), range)?
                .ok_or_else(|| crate::invalid("Missing production ledger record"))?;
            restored.apply(index::unpack(&bytes)?);
        }
        let mut record = restored
            .record
            .take()
            .ok_or_else(|| crate::invalid("Missing production ledger record"))?;
        if commit.chunks != record.chunks
            || commit.tail.len() as u64 != commit.chunks % index::CHUNK_PAGE as u64
        {
            return Err(crate::invalid("Production commit and ledger disagree"));
        }
        let mut trees = Trees::new(branch);
        let chunk = record.chunks;
        let mut history = |id: u64, live: bool| -> io::Result<History> {
            restored
                .histories
                .remove(&id)
                .ok_or_else(|| crate::invalid("Missing production history continuation"))?
                .restore(
                    id,
                    live,
                    &mut Ctx::new(&mut *pages, &mut trees, chunk),
                )
        };
        let absent = history(record.absent, true)?;
        if absent.state.stream.as_ref().map(|stream| stream.count) != Some(commit.spans) {
            return Err(crate::invalid(
                "Production commit and ledger disagree on the absence's unread spans",
            ));
        }
        let native = std::mem::take(&mut record.natives)
            .into_iter()
            .map(|(key, id)| Ok((key, history(id, true)?)))
            .collect::<io::Result<_>>()?;
        let retired = std::mem::take(&mut record.retired)
            .into_iter()
            .map(|(key, id)| Ok((key, history(id, false)?)))
            .collect::<io::Result<_>>()?;
        let mut tags = |aliases: Vec<(String, TagLedger)>, live: bool| {
            aliases
                .into_iter()
                .map(|(tag, alias)| {
                    Ok((
                        tag,
                        match alias {
                            TagLedger::Absent => TagCountry::Absent,
                            TagLedger::Native(key) => TagCountry::Native(key),
                            TagLedger::Projected(id) => {
                                TagCountry::Projected(Box::new(history(id, live)?))
                            }
                        },
                    ))
                })
                .collect::<io::Result<BTreeMap<_, _>>>()
        };
        let aliases = tags(std::mem::take(&mut record.aliases), true)?;
        let dormant = tags(std::mem::take(&mut record.dormant), false)?;
        directory.coverage.complete &= directory.last_block == last_block(replay, branch);
        let tag_names = directory.tags.iter().cloned().collect();
        let aliases_spans = std::mem::take(&mut record.spans);
        Ok(Self {
            processed,
            block_failed: false,
            deferred: Vec::new(),
            deferred_breaks: false,
            followed: None,
            designing: None,
            world: World::resume(restored.world)?,
            absent,
            native,
            retired,
            aliases,
            dormant,
            active: record.active.into_iter().collect(),
            links: record.links.into_iter().collect(),
            epochs: record.epochs.into_iter().collect(),
            tag_names,
            initial_tags: BTreeSet::new(),
            quiet: None,
            last_input: None,
            evidence_ready: false,
            evidence: Vec::new(),
            evidence_bytes: 0,
            evidence_base_bytes: 0,
            evidence_design_bytes: 0,
            evidence_organisation_bytes: 0,
            evidence_organisation_hour: None,
            alias_last: aliases_spans
                .iter()
                .enumerate()
                .map(|(index, alias)| (alias.tag.clone(), index))
                .collect(),
            aliases_spans,
            next_history: record.next_history,
            chunks: record.chunks,
            table: commit.tail,
            trees,
            through: record.through,
            flushed_through: record.flushed_through,
            ledger_ranges: commit.ledger,
            pending: false,
            directory,
        })
    }
    pub fn next_block(&self, replay: &Replay) -> io::Result<usize> {
        if self.block_failed {
            return Err(crate::invalid("Production block did not finish"));
        }
        match &self.processed {
            None => Ok(0),
            Some(previous) => replay
                .block_position(previous)
                .filter(|_| previous.branch == self.directory.branch)
                .map(|position| position + 1)
                .ok_or_else(|| crate::invalid("Production cursor differs from recording prefix")),
        }
    }
}

/// Keeps each variant's last version, which the next chunk's versions follow.
fn tail<T>(values: &mut Vec<T>) {
    if values.len() > 1 {
        values.drain(..values.len() - 1);
        values.shrink_to_fit();
    }
}

/// A fold step's work, in lines, past which it is shared out over spare cores.
const SHARD_WORK: usize = 8 * 1024;
/// The least work, in lines, a shard is given.
const SHARD_LEAST: usize = 1024;

/// What a fold step weighs on `history`: its open lines where the hour reads them.
fn weigh(history: &History, lines_read: bool) -> usize {
    1 + if lines_read {
        history.state.open.len()
    } else {
        0
    }
}

/// Runs `step` on each history of `work` with its operand, and gives back what each step gave,
/// in order. Large work is shared out by its weights in contiguous shards over spare cores,
/// whose tree reads this thread serves from `pages`.
fn each<'h, S, R, F>(
    pages: &mut dyn Pages,
    trees: &mut Trees,
    chunk: u64,
    work: Vec<(&'h mut History, S, usize)>,
    step: F,
) -> io::Result<Vec<R>>
where
    S: Send,
    R: Send,
    F: Fn(&mut History, S, &mut Ctx<'_>) -> io::Result<R> + Sync,
{
    let total: usize = work.iter().map(|(_, _, weight)| *weight).sum();
    let workers = std::thread::available_parallelism()
        .map_or(1, |cores| (cores.get() / 2).max(1))
        .min(total / SHARD_LEAST)
        .min(work.len());
    if total < SHARD_WORK || workers <= 1 {
        let mut ctx = Ctx::new(pages, trees, chunk);
        return work
            .into_iter()
            .map(|(history, operand, _)| step(history, operand, &mut ctx))
            .collect();
    }
    let share = total.div_ceil(workers);
    let mut shards: Vec<Vec<(&'h mut History, S)>> = vec![Vec::new()];
    let mut filled = 0;
    for (history, operand, weight) in work {
        if filled >= share && shards.len() < workers {
            shards.push(Vec::new());
            filled = 0;
        }
        filled += weight;
        if let Some(shard) = shards.last_mut() {
            shard.push((history, operand));
        }
    }
    let generation = trees.generation;
    let (requests, asked) = std::sync::mpsc::channel::<keys::Request>();
    let context = replay_frame::cpu::Context::capture();
    let step = &step;
    std::thread::scope(|scope| {
        let handles: Vec<_> = shards
            .into_iter()
            .map(|shard| {
                let remote = keys::Remote::new(requests.clone());
                let context = context.clone();
                scope.spawn(move || {
                    let _cpu = context.enter("fold observe");
                    let mut ctx = Ctx::remote(&remote, generation, chunk);
                    shard
                        .into_iter()
                        .map(|(history, operand)| step(history, operand, &mut ctx))
                        .collect::<io::Result<Vec<R>>>()
                })
            })
            .collect();
        // Every shard holds its own way here; the reads end once they all finish.
        drop(requests);
        for request in asked {
            request.serve(&mut *pages, &mut *trees);
        }
        let mut given = Vec::new();
        let mut failed = None;
        for handle in handles {
            match handle.join() {
                Ok(Ok(shard)) => given.extend(shard),
                Ok(Err(error)) => {
                    failed.get_or_insert(error);
                }
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        match failed {
            Some(error) => Err(error),
            None => Ok(given),
        }
    })
}

/// Encodes each history's parts on spare cores, in contiguous groups of about equal bytes.
fn encode_histories(
    histories: Vec<(u64, Events)>,
    mut parts: Vec<index::Encoded>,
) -> io::Result<Vec<index::Encoded>> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |cores| (cores.get() / 2).max(1))
        .min(histories.len());
    if workers <= 1 {
        for (history, events) in &histories {
            parts.extend(index::encode_history(*history, events)?);
        }
        return Ok(parts);
    }
    let total: usize = histories
        .iter()
        .map(|(_, events)| events.bytes.max(1))
        .sum();
    let share = total.div_ceil(workers);
    let mut groups: Vec<Vec<(u64, Events)>> = vec![Vec::new()];
    let mut filled = 0;
    for history in histories {
        if filled >= share && groups.len() < workers {
            groups.push(Vec::new());
            filled = 0;
        }
        filled += history.1.bytes.max(1);
        groups.last_mut().unwrap().push(history);
    }
    let context = replay_frame::cpu::Context::capture();
    std::thread::scope(|scope| {
        let handles: Vec<_> = groups
            .iter()
            .map(|group| {
                let context = context.clone();
                scope.spawn(move || {
                    let _cpu = context.enter("fold publish");
                    let mut encoded = Vec::new();
                    for (history, events) in group {
                        encoded.extend(index::encode_history(*history, events)?);
                    }
                    Ok::<_, io::Error>(encoded)
                })
            })
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(encoded) => parts.extend(encoded?),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        Ok(parts)
    })
}

/// A tag as a ledger record holds it.
#[derive(Clone, Debug)]
enum TagLedger {
    Absent,
    Native(NativeCountry),
    Projected(u64),
}

/// A line's compact continuation: its lifetime and what it carries.
#[derive(Clone, Debug)]
struct LineLedger {
    lifetime: u64,
    appearance: u32,
    run: Option<(u64, RawRun)>,
    step: Option<(u64, u64, i32, i32)>,
    output: Option<(u64, RawOutput)>,
    modifier: Option<i32>,
    operands: Option<Vec<u8>>,
}

/// A history's state without its events or keyed values.
#[derive(Clone, Debug)]
struct CoreLedger {
    last: Option<((u32, u32), i32)>,
    open: Vec<(u64, LineLedger)>,
    carried: bool,
    broken: Vec<(u64, LineLedger)>,
    last_read_identity: Option<(u32, u32)>,
    broken_identity: Option<(u32, u32)>,
    first_read: bool,
    output_open: bool,
    organisations_open: bool,
    last_census: Option<i32>,
    active_variants: BTreeSet<u64>,
    carry: Option<Carry>,
    last_output_due: Option<i32>,
    last_modifier_country: Option<Vec<u8>>,
    last_design_revision: Option<u64>,
    counters: index::Counters,
    /// The absence's unread spans: how many, and those after its last full page.
    stream: Option<(u64, Vec<HourSpan>)>,
    ids: (u64, u64, u64, u64),
    through: Option<i32>,
    overview: Option<u64>,
}

/// A history's keyed values in a ledger record: its tree, and the values it holds that the
/// tree does not (in a delta, only those changed since the record before).
#[derive(Clone, Debug, Default)]
struct KeysLedger {
    root: Option<keys::NodeRef>,
    handles: Vec<(u64, Handle)>,
    designs: Vec<(u64, (DesignRead, u64))>,
    organisations: Vec<(u64, (RawOrganisation, u64))>,
    icons: Vec<(u64, IconChoice)>,
}

product_codec::product! {
    /// A history in a ledger record: held, or put away whole at `blob`.
    #[derive(Clone, Debug)]
    enum StateLedger {
        0 => Loaded { core: CoreLedger, keys: KeysLedger },
        1 => Stored { through: Option<i32>, overview: Option<u64>, blob: u64 },
    }
}

/// One history in a ledger record: whole where `full`, else its core and the keyed values
/// changed since the record before.
#[derive(Clone, Debug)]
struct HistoryLedger {
    parent: Option<(u64, u64)>,
    sleep: Option<Sleep>,
    full: bool,
    state: StateLedger,
}

/// What a resume restores: identities and histories as the last commit left them.
#[derive(Clone, Debug)]
struct LedgerRecord {
    full: bool,
    chunks: u64,
    through: Option<i32>,
    flushed_through: Option<i32>,
    next_history: u64,
    world: input::WorldSaved,
    absent: u64,
    active: Vec<((u32, u32), NativeCountry)>,
    links: Vec<((u32, u32), NativeCountry)>,
    epochs: Vec<((u32, u32), u64)>,
    natives: Vec<(NativeCountry, u64)>,
    retired: Vec<(NativeCountry, u64)>,
    aliases: Vec<(String, TagLedger)>,
    dormant: Vec<(String, TagLedger)>,
    spans: Vec<Alias>,
    histories: Vec<(u64, HistoryLedger)>,
}

fn line_ledger(line: &Line) -> io::Result<LineLedger> {
    Ok(LineLedger {
        lifetime: line.lifetime,
        appearance: line.appearance,
        run: line.run.as_ref().map(|run| (run.id, run.run)),
        step: line.step.map(|step| {
            (
                step.id,
                step.variant,
                step.active_factories,
                step.requested_factories,
            )
        }),
        output: line
            .output
            .as_ref()
            .map(|output| (output.id, output.output.clone())),
        modifier: line.modifier.map(|modifier| modifier.last),
        operands: line
            .operands
            .as_ref()
            .map(|row| index::encode_row(TableRows::ProductionLineModifiers(vec![row.clone()])))
            .transpose()?,
    })
}

fn restore_line(handle: u64, line: LineLedger) -> io::Result<Line> {
    Ok(Line {
        lifetime: line.lifetime,
        handle,
        appearance: line.appearance,
        run: line.run.map(|(id, run)| LiveRun {
            id,
            run,
            at: At::Carried(0),
        }),
        step: line.step.map(
            |(id, variant, active_factories, requested_factories)| LiveStep {
                id,
                variant,
                active_factories,
                requested_factories,
                at: At::Carried(0),
            },
        ),
        output: line.output.map(|(id, output)| LiveOutput {
            id,
            output,
            at: At::Carried(0),
        }),
        modifier: line.modifier.map(|last| LiveModifier {
            last,
            at: At::Carried(0),
        }),
        operands: line
            .operands
            .as_deref()
            .map(index::decode_line)
            .transpose()?,
    })
}

/// The values a ledger record holds of `held`: each one its tree lacks, or those of them
/// changed since the record before.
fn listed<V: product_codec::Product + Clone>(
    held: &keys::Resident<V>,
    dirty: BTreeSet<u64>,
    full: bool,
) -> Vec<(u64, V)> {
    if full {
        held.all_unstored()
            .map(|(id, value)| (id, value.clone()))
            .collect()
    } else {
        dirty
            .into_iter()
            .filter_map(|id| Some((id, held.unstored(id)?.clone())))
            .collect()
    }
}

impl History {
    /// This history in a ledger record, and what the next record no longer repeats.
    fn ledger(&mut self, full: bool) -> io::Result<HistoryLedger> {
        if let Some(blob) = self.blob {
            self.state.fresh = false;
            self.state.changed = false;
            return Ok(HistoryLedger {
                parent: self.parent,
                sleep: self.sleep,
                full: true,
                state: StateLedger::Stored {
                    through: self.state.through,
                    overview: self.state.overview,
                    blob,
                },
            });
        }
        let state = &mut self.state;
        let full = full || state.fresh;
        let dirty = std::mem::take(&mut state.dirty);
        state.fresh = false;
        state.changed = false;
        let held = &state.keys;
        let keys = KeysLedger {
            root: held.root,
            handles: listed(&held.handles, dirty.handles, full),
            designs: listed(&held.designs, dirty.designs, full),
            organisations: listed(&held.organisations, dirty.organisations, full),
            icons: listed(&held.icons, dirty.icons, full),
        };
        let core = CoreLedger {
            last: state.last,
            open: state
                .open
                .iter()
                .map(|(handle, line)| Ok((*handle, line_ledger(line)?)))
                .collect::<io::Result<_>>()?,
            carried: state.carried,
            broken: state
                .broken
                .iter()
                .map(|(handle, line)| Ok((*handle, line_ledger(line)?)))
                .collect::<io::Result<_>>()?,
            last_read_identity: state.last_read_identity,
            broken_identity: state.broken_identity,
            first_read: state.first_read,
            output_open: state.output_open,
            organisations_open: state.organisations_open,
            last_census: state.last_census,
            active_variants: state.active_variants.clone(),
            carry: state.carry,
            last_output_due: state.last_output_due,
            last_modifier_country: state
                .last_modifier_country
                .as_ref()
                .map(|row| {
                    index::encode_row(TableRows::CountryProductionModifiers(vec![(**row).clone()]))
                })
                .transpose()?,
            last_design_revision: state.last_design_revision,
            counters: state.counters.clone(),
            stream: state
                .stream
                .as_ref()
                .map(|stream| (stream.count, stream.tail.clone())),
            ids: (
                state.ids.lifetime,
                state.ids.run,
                state.ids.step,
                state.ids.output,
            ),
            through: state.through,
            overview: state.overview,
        };
        Ok(HistoryLedger {
            parent: self.parent,
            sleep: self.sleep,
            full,
            state: StateLedger::Loaded { core, keys },
        })
    }
}

/// A history folded from ledger records, the last core over the earlier ones.
struct RestoredHistory {
    parent: Option<(u64, u64)>,
    sleep: Option<Sleep>,
    state: StateLedger,
}

impl From<HistoryLedger> for RestoredHistory {
    fn from(history: HistoryLedger) -> Self {
        Self {
            parent: history.parent,
            sleep: history.sleep,
            state: history.state,
        }
    }
}

impl RestoredHistory {
    fn restore(self, id: u64, live: bool, ctx: &mut Ctx<'_>) -> io::Result<History> {
        let (core, stored) = match self.state {
            StateLedger::Stored {
                through,
                overview,
                blob,
            } => {
                return Ok(History {
                    id,
                    parent: self.parent,
                    state: CountryState {
                        through,
                        overview,
                        ..CountryState::default()
                    },
                    sleep: self.sleep,
                    blob: Some(blob),
                });
            }
            StateLedger::Loaded { core, keys } => (core, keys),
        };
        let lines = |lines: Vec<(u64, LineLedger)>| {
            lines
                .into_iter()
                .map(|(handle, line)| Ok((handle, restore_line(handle, line)?)))
                .collect::<io::Result<BTreeMap<_, _>>>()
        };
        let mut held = Keys {
            root: stored.root,
            ..Keys::default()
        };
        held.handles.restore(stored.handles);
        held.designs.restore(stored.designs);
        held.organisations.restore(stored.organisations);
        held.icons.restore(stored.icons);
        let mut state = CountryState {
            events: Events::default(),
            last: core.last,
            open: lines(core.open)?,
            carried: core.carried,
            broken: lines(core.broken)?,
            last_read_identity: core.last_read_identity,
            broken_identity: core.broken_identity,
            first_read: core.first_read,
            output_open: core.output_open,
            organisations_open: core.organisations_open,
            last_census: core.last_census,
            active_variants: core.active_variants,
            carry: core.carry,
            last_output_due: core.last_output_due,
            last_modifier_country: core
                .last_modifier_country
                .as_deref()
                .map(index::decode_country)
                .transpose()?
                .map(Arc::new),
            country_at: None,
            last_design_revision: core.last_design_revision,
            keys: held,
            counters: core.counters,
            stream: core
                .stream
                .map(|(count, tail)| keys::Stream::restored(count, tail))
                .transpose()?,
            ids: Ids {
                lifetime: core.ids.0,
                run: core.ids.1,
                step: core.ids.2,
                output: core.ids.3,
            },
            through: core.through,
            overview: core.overview,
            dirty: Dirty::default(),
            changed: false,
            fresh: false,
        };
        // The chunk after the commit carries what is open, as the flush before left it.
        state.take_events(live, ctx)?;
        Ok(History {
            id,
            parent: self.parent,
            state,
            sleep: self.sleep,
            blob: None,
        })
    }
}

#[derive(Default)]
struct Restored {
    record: Option<LedgerRecord>,
    world: Vec<input::WorldSaved>,
    histories: BTreeMap<u64, RestoredHistory>,
}

impl Restored {
    fn apply(&mut self, mut record: LedgerRecord) {
        if record.full {
            self.world.clear();
            self.histories.clear();
        }
        self.world.push(std::mem::take(&mut record.world));
        for (id, history) in std::mem::take(&mut record.histories) {
            let delta = !history.full && matches!(history.state, StateLedger::Loaded { .. });
            match self
                .histories
                .get_mut(&id)
                .filter(|restored| delta && matches!(restored.state, StateLedger::Loaded { .. }))
            {
                Some(restored) => {
                    restored.parent = history.parent;
                    restored.sleep = history.sleep;
                    if let (
                        StateLedger::Loaded { core, keys },
                        StateLedger::Loaded {
                            core: held,
                            keys: values,
                        },
                    ) = (history.state, &mut restored.state)
                    {
                        *held = core;
                        values.root = keys.root;
                        values.handles.extend(keys.handles);
                        values.designs.extend(keys.designs);
                        values.organisations.extend(keys.organisations);
                        values.icons.extend(keys.icons);
                    }
                }
                None => {
                    self.histories.insert(id, history.into());
                }
            }
        }
        self.record = Some(record);
    }
}

impl Prepared {
    fn ledger_record(&mut self, full: bool) -> io::Result<LedgerRecord> {
        let mut histories = Vec::new();
        let absent = self.absent.id;
        for history in self.histories_mut() {
            if full || history.state.changed || history.state.fresh {
                histories.push((history.id, history.ledger(full)?));
            }
        }
        let tags = |aliases: &BTreeMap<String, TagCountry>| {
            aliases
                .iter()
                .map(|(tag, alias)| {
                    (
                        tag.clone(),
                        match alias {
                            TagCountry::Absent => TagLedger::Absent,
                            TagCountry::Native(key) => TagLedger::Native(key.clone()),
                            TagCountry::Projected(history) => TagLedger::Projected(history.id),
                        },
                    )
                })
                .collect()
        };
        Ok(LedgerRecord {
            full,
            chunks: self.chunks,
            through: self.through,
            flushed_through: self.flushed_through,
            next_history: self.next_history,
            world: self.world.save(full)?,
            absent,
            active: self
                .active
                .iter()
                .map(|(id, key)| (*id, key.clone()))
                .collect(),
            links: self
                .links
                .iter()
                .map(|(id, key)| (*id, key.clone()))
                .collect(),
            epochs: self
                .epochs
                .iter()
                .map(|(id, epoch)| (*id, *epoch))
                .collect(),
            natives: self
                .native
                .iter()
                .map(|(key, history)| (key.clone(), history.id))
                .collect(),
            retired: self
                .retired
                .iter()
                .map(|(key, history)| (key.clone(), history.id))
                .collect(),
            aliases: tags(&self.aliases),
            dormant: tags(&self.dormant),
            spans: self.aliases_spans.clone(),
            histories,
        })
    }
}
pub fn directory(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Option<Directory>> {
    crate::prepared::series::read_branch_reply(store, key, &directory_section(branch), 0, branch)
}

/// Whether the store holds a commit in this layout.
pub(super) fn binary_current(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<bool> {
    Ok(store
        .read(key, &index::commits_section(branch), 0)?
        .is_some())
}

pub fn current(
    store: &store::Store,
    key: &store::SourceKey,
    source: &Source,
    context: &LoadContext,
    branch: u64,
) -> io::Result<bool> {
    if !binary_current(store, key, branch)? {
        return Ok(false);
    }
    let Some(directory) = directory(store, key, branch)? else {
        return Ok(false);
    };
    if directory.coverage.schema != crate::prepared::coverage::SCHEMA
        || !directory.coverage.complete
        || directory.last_block != last_block(source.replay_ref(), branch)
        || directory.executable != engine_executable::executable_tables::is_initialized()
        || !pictures_serve(
            source,
            context,
            directory.pictured,
            directory.stamped,
            directory.visual.as_deref(),
        )
    {
        return Ok(false);
    }
    Ok(source.with_metadata(|metadata| {
        names_every(
            &directory,
            metadata.countries.iter().map(|country| country.tag.as_str()),
        )
    }))
}

/// Whether `directory` marks each of `tags` as its preparation's: a row preparation reconciles
/// the tags a later preparation names into it; a native one's frontier alone says it is current.
pub(super) fn names_every<'t>(
    directory: &Directory,
    tags: impl IntoIterator<Item = &'t str>,
) -> bool {
    if directory.native {
        return true;
    }
    let held: BTreeSet<&str> = directory.tags.iter().map(String::as_str).collect();
    tags.into_iter().all(|tag| held.contains(tag))
}

/// The prepared index a query reads, its content checked as the old pages were.
pub(super) fn open(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    icons: Option<&IconContent>,
    cancelled: &AtomicBool,
) -> io::Result<read::Index<'static>> {
    crate::metadata::check_cancel(cancelled)?;
    #[cfg(any(test, feature = "test-support"))]
    if let Some(index) = fixtures::open(source, branch)? {
        return Ok(index);
    }
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    if !binary_current(&store, &key, branch)? {
        return Err(crate::invalid(
            "Production lines need preparation for the current private-page schema",
        ));
    }
    let mut directory = directory(&store, &key, branch)?
        .ok_or_else(|| crate::invalid("Production lines are still being prepared"))?;
    if directory.coverage.schema != crate::prepared::coverage::SCHEMA
        || directory.executable != engine_executable::executable_tables::is_initialized()
        || !pictures_serve_with(icons, directory.pictured, directory.visual.as_deref())
    {
        return Err(dashboard_core::refused(
            "Production lines need preparation for the current content",
        ));
    }
    directory.coverage.complete &= directory.last_block == last_block(source.replay_ref(), branch);
    if !directory.coverage.complete {
        crate::prepared::coverage::note_short("production-lines");
    }
    let binding = key.clone();
    // The commit's bytes' identity names everything the publication holds.
    let publication = match directory.commits.checked_sub(1) {
        Some(range) => store
            .range_id(&key, &index::commits_section(branch), range)?
            .map(|id| id.cache_key()),
        None => None,
    };
    let mut pages = index::StoreRanges::scoped(store, key)?;
    let published = match publication {
        Some(id) => crate::prepared::query::observations::scope(
            source,
            "production-lines:published",
            Some(&binding),
        )
        .get_entry(
            branch,
            &(),
            &(index::commits_section(branch), id),
            cancelled,
            read::Published::bytes,
            || read::Published::read(&mut pages, &directory),
        )?,
        None => Arc::new(read::Published::read(&mut pages, &directory)?),
    };
    let mut index = read::Index::with(Box::new(pages), directory, published, publication);
    index.binding = Some(binding);
    Ok(index)
}

pub(super) fn country_at(
    index: &mut read::Index<'_>,
    tag: &str,
    hour: i32,
) -> io::Result<NativeCountry> {
    index.country_at(tag, hour)?.ok_or_else(|| {
        crate::invalid("The original production country identity is unavailable at its choice")
    })
}

product_codec::product! { impl Sleep {
    hour: i32,
    reads: usize,
    stretches: usize,
    spans: u64,
} default {} }
product_codec::product! { impl HistoryView {
    id: u64,
    parent: Option<(u64, u64)>,
    sleep: Option<Sleep>,
    through: Option<i32>,
    overview: Option<u64>,
} default {} }
product_codec::product! { impl Commit {
    chunks: u64,
    tail: Vec<(u64, i32, i32)>,
    spans: u64,
    span_tail: Vec<HourSpan>,
    through: Option<i32>,
    variants_hour: Option<i32>,
    identity: u64,
    ledger: Vec<u64>,
} default {} }
product_codec::product! { impl Identity {
    absent: u64,
    histories: Vec<HistoryView>,
    natives: Vec<(NativeCountry, u64)>,
    tags: Vec<(String, u64)>,
} default {} }
