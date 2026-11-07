//! What a fold records of a chunk ([`Record`]): a history's events, as the ordered emit holds
//! them, or a lens's journal of the same changes ([`Op`]), which its walk hands the emit as it
//! makes them and the emit makes again into the lens's events ([`Rebuilt`]). A lens keeps only
//! what its fold reads back of its events: their counts, the anchors noted, and their size.

use super::*;

/// The overview's spans of hours a change marks.
const LINE_HOURS: u8 = 0;
const OUTPUT_HOURS: u8 = 1;
const ORGANISATION_HOURS: u8 = 2;

fn spans(events: &mut Events, which: u8) -> io::Result<&mut Vec<HourSpan>> {
    let overview = &mut events.overview;
    Ok(match which {
        LINE_HOURS => &mut overview.line_hours,
        OUTPUT_HOURS => &mut overview.output_hours,
        ORGANISATION_HOURS => &mut overview.organisation_hours,
        _ => return Err(crate::invalid("A production lens marks spans of no kind")),
    })
}

fn at_of(carried: bool, at: u64) -> io::Result<At> {
    let at = usize::try_from(at).map_err(|_| crate::invalid("A production record lies past memory"))?;
    Ok(if carried { At::Carried(at) } else { At::New(at) })
}

fn split(at: At) -> (bool, u64) {
    match at {
        At::New(at) => (false, at as u64),
        At::Carried(at) => (true, at as u64),
    }
}

product_codec::product! {
    /// One change to a chunk's events, in the order a lens made it.
    #[derive(Clone, Debug)]
    pub(super) enum Op {
        0 => Read { hour: i32 },
        1 => Mark { which: u8, hour: i32 },
        2 => MarkSpan { which: u8, first: i32, last: i32 },
        3 => Unread { first: i32, last: i32 },
        4 => Wake { from: u64, to: u64, hour: i32 },
        5 => Life { life: index::Life },
        6 => End { lifetime: u64, hour: i32 },
        7 => Final { lifetime: u64, output: RawOutput },
        8 => Produced { variant: u64, first: i32, last: i32 },
        9 => Design { variant: u64, read: DesignRead },
        10 => Organisation { id: u64, raw: RawOrganisation, named: bool },
        11 => DesignAnchor { variant: u64, at: Option<u64> },
        12 => OrganisationAnchor { id: u64, at: Option<u64> },
        13 => Country { row: Vec<u8> },
        14 => Modifier {
            lifetime: u64,
            hour: i32,
            country: u32,
            line: Option<Vec<u8>>,
            run: RawRun,
            organisation: Option<String>,
        },
        15 => CloseModifier { lifetime: u64, carried: bool, at: u64, last: i32 },
        16 => Step { step: index::Step },
        17 => CloseStep { carried: bool, at: u64, id: u64, last: i32 },
        18 => Run { id: u64, lifetime: u64, run: RawRun },
        19 => Live { lifetime: u64, handle: u64, appearance: u32 },
        20 => CloseRun { carried: bool, at: u64, last: i32 },
        21 => OutputVariants { variants: Vec<u64> },
        22 => Output { record: OutputRecord },
        23 => CloseOutput { carried: bool, at: u64, last: i32 },
        24 => Cut { counters: index::Counters, produced: Vec<(u64, i32, i32)> },
        25 => CarriedRun { id: u64, lifetime: u64, run: RawRun },
        26 => CarriedOutput { record: OutputRecord },
        27 => Piece {
            marks: index::Marks,
            conds: Vec<(u32, u8, u64, u32)>,
            handles: Vec<u64>,
        },
    }
}

/// The changes [`Op`] names, as a history's events take them.
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

    /// Makes `op`'s change; the place of what it adds where it adds to a list.
    fn apply(&mut self, op: Op) -> io::Result<Option<Events>> {
        match op {
            Op::Read { hour } => self.overview.reads.push(hour),
            Op::Mark { which, hour } => mark(spans(self, which)?, hour),
            Op::MarkSpan { which, first, last } => mark_span(spans(self, which)?, first, last),
            Op::Unread { first, last } => self.unread(first, last),
            Op::Wake { from, to, hour } => self.wake(from, to, hour),
            Op::Life { life } => self.overview.lives.push(life),
            Op::End { lifetime, hour } => self.overview.ends.push((lifetime, hour)),
            Op::Final { lifetime, output } => self.overview.finals.push((lifetime, output)),
            Op::Produced {
                variant,
                first,
                last,
            } => self.overview.produced.push((variant, first, last)),
            Op::Design { variant, read } => self.versions.designs.push((variant, read)),
            Op::Organisation { id, raw, named } => self.organisation(id, raw, named),
            Op::DesignAnchor { variant, at } => {
                self.versions.design_anchors.insert(variant, at);
            }
            Op::OrganisationAnchor { id, at } => {
                self.versions.organisation_anchors.insert(id, at);
            }
            Op::Country { row } => self.countries.push(Arc::new(index::decode_country(&row)?)),
            Op::Modifier {
                lifetime,
                hour,
                country,
                line,
                run,
                organisation,
            } => {
                let line = line.map(|bytes| decode_line(&bytes)).transpose()?;
                self.modifiers.entry(lifetime).or_default().push(
                    hour,
                    country,
                    line.as_ref(),
                    &run,
                    organisation,
                );
            }
            Op::CloseModifier {
                lifetime,
                carried,
                at,
                last,
            } => self.close_modifier(lifetime, at_of(carried, at)?, last),
            Op::Step { step } => self.overview.steps.push(step),
            Op::CloseStep {
                carried,
                at,
                id,
                last,
            } => self.close_step(at_of(carried, at)?, id, last),
            Op::Run { id, lifetime, run } => {
                self.detail.runs.push(id, lifetime, &run);
            }
            Op::Live {
                lifetime,
                handle,
                appearance,
            } => {
                self.detail.lives.insert(lifetime, (handle, appearance));
            }
            Op::CloseRun { carried, at, last } => self.close_run(at_of(carried, at)?, last),
            Op::OutputVariants { variants } => self.overview.output_variants.extend(variants),
            Op::Output { record } => self.detail.outputs.push(record),
            Op::CloseOutput { carried, at, last } => self.close_output(at_of(carried, at)?, last),
            Op::Cut { counters, produced } => return Ok(Some(self.cut(&counters, produced))),
            Op::CarriedRun { id, lifetime, run } => {
                self.detail.carried_runs.push(id, lifetime, &run);
            }
            Op::CarriedOutput { record } => self.detail.carried_outputs.push(record),
            Op::Piece { .. } => {
                return Err(crate::invalid("A production lens's piece is no change of events"));
            }
        }
        Ok(None)
    }
}

fn decode_line(bytes: &[u8]) -> io::Result<ProductionLineModifiersRow> {
    match index::decode_row(TableId::ProductionLineModifiers, bytes)? {
        TableRows::ProductionLineModifiers(mut rows) if rows.len() == 1 => Ok(rows.remove(0)),
        _ => Err(crate::invalid("A production line modifier row does not decode")),
    }
}

/// A lens's changes since it last handed them on, with what its fold reads back of the chunk.
#[derive(Clone, Default)]
pub(super) struct Journal {
    ops: Vec<Op>,
    runs: usize,
    carried_runs: usize,
    steps: usize,
    outputs: usize,
    carried_outputs: usize,
    countries: usize,
    designs: usize,
    organisations: usize,
    names: usize,
    modifiers: BTreeMap<u64, usize>,
    design_anchors: BTreeSet<u64>,
    organisation_anchors: BTreeSet<u64>,
    bytes: usize,
    touched: bool,
}

/// What a fold records of the chunk being built.
#[derive(Clone)]
pub(super) enum Record {
    Events(Events),
    Journal(Box<Journal>),
}

impl Default for Record {
    fn default() -> Self {
        Self::Events(Events::default())
    }
}

fn count(held: &mut usize) -> usize {
    let at = *held;
    *held += 1;
    at
}

impl Record {
    /// A lens's: a journal.
    pub(super) fn journal() -> Self {
        Self::Journal(Box::default())
    }

    /// A history's events.
    #[cfg(test)]
    pub(super) fn held(&self) -> &Events {
        match self {
            Self::Events(events) => events,
            Self::Journal(_) => panic!("A lens journals its events"),
        }
    }

    pub(super) fn touch(&mut self, bytes: usize) {
        let (touched, held) = match self {
            Self::Events(events) => (&mut events.touched, &mut events.bytes),
            Self::Journal(journal) => (&mut journal.touched, &mut journal.bytes),
        };
        *touched = true;
        *held = held.saturating_add(bytes);
    }

    pub(super) fn is_empty(&self) -> bool {
        match self {
            Self::Events(events) => events.is_empty(),
            Self::Journal(journal) => !journal.touched,
        }
    }

    /// What the chunk holds, as the batch budget counts it.
    pub(super) fn bytes(&self) -> usize {
        match self {
            Self::Events(events) => events.bytes,
            Self::Journal(journal) => journal.bytes,
        }
    }

    /// How many design reads, organisation versions and organisation names the chunk lists.
    pub(super) fn listed(&self) -> (usize, usize, usize) {
        match self {
            Self::Events(events) => (
                events.versions.designs.len(),
                events.versions.organisations.len(),
                events.overview.names.len(),
            ),
            Self::Journal(journal) => (journal.designs, journal.organisations, journal.names),
        }
    }

    pub(super) fn has_design_anchor(&self, variant: u64) -> bool {
        match self {
            Self::Events(events) => events.versions.design_anchors.contains_key(&variant),
            Self::Journal(journal) => journal.design_anchors.contains(&variant),
        }
    }

    pub(super) fn has_organisation_anchor(&self, id: u64) -> bool {
        match self {
            Self::Events(events) => events.versions.organisation_anchors.contains_key(&id),
            Self::Journal(journal) => journal.organisation_anchors.contains(&id),
        }
    }

    pub(super) fn read(&mut self, hour: i32) {
        match self {
            Self::Events(events) => events.overview.reads.push(hour),
            Self::Journal(journal) => journal.ops.push(Op::Read { hour }),
        }
    }

    fn mark_in(&mut self, which: u8, hour: i32) {
        match self {
            Self::Events(events) => {
                if let Ok(spans) = spans(events, which) {
                    mark(spans, hour);
                }
            }
            Self::Journal(journal) => journal.ops.push(Op::Mark { which, hour }),
        }
    }

    fn span_in(&mut self, which: u8, first: i32, last: i32) {
        match self {
            Self::Events(events) => {
                if let Ok(spans) = spans(events, which) {
                    mark_span(spans, first, last);
                }
            }
            Self::Journal(journal) => journal.ops.push(Op::MarkSpan { which, first, last }),
        }
    }

    pub(super) fn mark_lines(&mut self, hour: i32) {
        self.mark_in(LINE_HOURS, hour);
    }

    pub(super) fn mark_output(&mut self, hour: i32) {
        self.mark_in(OUTPUT_HOURS, hour);
    }

    pub(super) fn mark_organisations(&mut self, hour: i32) {
        self.mark_in(ORGANISATION_HOURS, hour);
    }

    pub(super) fn span_lines(&mut self, first: i32, last: i32) {
        self.span_in(LINE_HOURS, first, last);
    }

    pub(super) fn span_output(&mut self, first: i32, last: i32) {
        self.span_in(OUTPUT_HOURS, first, last);
    }

    pub(super) fn span_organisations(&mut self, first: i32, last: i32) {
        self.span_in(ORGANISATION_HOURS, first, last);
    }

    pub(super) fn unread(&mut self, first: i32, last: i32) {
        match self {
            Self::Events(events) => events.unread(first, last),
            Self::Journal(journal) => journal.ops.push(Op::Unread { first, last }),
        }
    }

    pub(super) fn wake(&mut self, from: u64, to: u64, hour: i32) {
        match self {
            Self::Events(events) => events.wake(from, to, hour),
            Self::Journal(journal) => journal.ops.push(Op::Wake { from, to, hour }),
        }
    }

    pub(super) fn life(&mut self, life: index::Life) {
        match self {
            Self::Events(events) => events.overview.lives.push(life),
            Self::Journal(journal) => journal.ops.push(Op::Life { life }),
        }
    }

    pub(super) fn end(&mut self, lifetime: u64, hour: i32) {
        match self {
            Self::Events(events) => events.overview.ends.push((lifetime, hour)),
            Self::Journal(journal) => journal.ops.push(Op::End { lifetime, hour }),
        }
    }

    pub(super) fn final_output(&mut self, lifetime: u64, output: RawOutput) {
        match self {
            Self::Events(events) => events.overview.finals.push((lifetime, output)),
            Self::Journal(journal) => journal.ops.push(Op::Final { lifetime, output }),
        }
    }

    pub(super) fn produced(&mut self, variant: u64, first: i32, last: i32) {
        match self {
            Self::Events(events) => events.overview.produced.push((variant, first, last)),
            Self::Journal(journal) => journal.ops.push(Op::Produced {
                variant,
                first,
                last,
            }),
        }
    }

    pub(super) fn design(&mut self, variant: u64, read: DesignRead) {
        match self {
            Self::Events(events) => events.versions.designs.push((variant, read)),
            Self::Journal(journal) => {
                journal.designs += 1;
                journal.ops.push(Op::Design { variant, read });
            }
        }
    }

    pub(super) fn organisation(&mut self, id: u64, raw: RawOrganisation, named: bool) {
        match self {
            Self::Events(events) => events.organisation(id, raw, named),
            Self::Journal(journal) => {
                journal.organisations += 1;
                journal.names += usize::from(named);
                journal.ops.push(Op::Organisation { id, raw, named });
            }
        }
    }

    pub(super) fn design_anchor(&mut self, variant: u64, at: Option<u64>) {
        match self {
            Self::Events(events) => {
                events.versions.design_anchors.insert(variant, at);
            }
            Self::Journal(journal) => {
                journal.design_anchors.insert(variant);
                journal.ops.push(Op::DesignAnchor { variant, at });
            }
        }
    }

    pub(super) fn organisation_anchor(&mut self, id: u64, at: Option<u64>) {
        match self {
            Self::Events(events) => {
                events.versions.organisation_anchors.insert(id, at);
            }
            Self::Journal(journal) => {
                journal.organisation_anchors.insert(id);
                journal.ops.push(Op::OrganisationAnchor { id, at });
            }
        }
    }

    /// A modifier country row: its place among the chunk's.
    pub(super) fn country(&mut self, row: &Arc<CountryProductionModifiersRow>) -> io::Result<u32> {
        let at = match self {
            Self::Events(events) => {
                events.countries.push(Arc::clone(row));
                events.countries.len() - 1
            }
            Self::Journal(journal) => {
                let row = index::encode_row(TableRows::CountryProductionModifiers(vec![
                    (**row).clone(),
                ]))?;
                journal.ops.push(Op::Country { row });
                count(&mut journal.countries)
            }
        };
        u32::try_from(at).map_err(|_| crate::invalid("A production chunk holds too many countries"))
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
        match self {
            Self::Events(events) => Ok(events.modifiers.entry(lifetime).or_default().push(
                hour,
                country,
                line,
                run,
                organisation,
            )),
            Self::Journal(journal) => {
                let line = line
                    .map(|line| {
                        index::encode_row(TableRows::ProductionLineModifiers(vec![line.clone()]))
                    })
                    .transpose()?;
                journal.ops.push(Op::Modifier {
                    lifetime,
                    hour,
                    country,
                    line,
                    run: *run,
                    organisation,
                });
                Ok(count(journal.modifiers.entry(lifetime).or_default()))
            }
        }
    }

    pub(super) fn close_modifier(&mut self, lifetime: u64, at: At, last: i32) {
        match self {
            Self::Events(events) => events.close_modifier(lifetime, at, last),
            Self::Journal(journal) => {
                let (carried, at) = split(at);
                journal.ops.push(Op::CloseModifier {
                    lifetime,
                    carried,
                    at,
                    last,
                });
            }
        }
    }

    /// A step: its place among the chunk's.
    pub(super) fn step(&mut self, step: index::Step) -> usize {
        match self {
            Self::Events(events) => {
                events.overview.steps.push(step);
                events.overview.steps.len() - 1
            }
            Self::Journal(journal) => {
                journal.ops.push(Op::Step { step });
                count(&mut journal.steps)
            }
        }
    }

    pub(super) fn close_step(&mut self, at: At, id: u64, last: i32) {
        match self {
            Self::Events(events) => events.close_step(at, id, last),
            Self::Journal(journal) => {
                let (carried, at) = split(at);
                journal.ops.push(Op::CloseStep {
                    carried,
                    at,
                    id,
                    last,
                });
            }
        }
    }

    /// A run: its place among the chunk's.
    pub(super) fn run(&mut self, id: u64, lifetime: u64, run: &RawRun) -> usize {
        match self {
            Self::Events(events) => events.detail.runs.push(id, lifetime, run),
            Self::Journal(journal) => {
                journal.ops.push(Op::Run {
                    id,
                    lifetime,
                    run: *run,
                });
                count(&mut journal.runs)
            }
        }
    }

    /// A run carried into the chunk: its place among those carried.
    pub(super) fn carried_run(&mut self, id: u64, lifetime: u64, run: &RawRun) -> usize {
        match self {
            Self::Events(events) => events.detail.carried_runs.push(id, lifetime, run),
            Self::Journal(journal) => {
                journal.ops.push(Op::CarriedRun {
                    id,
                    lifetime,
                    run: *run,
                });
                count(&mut journal.carried_runs)
            }
        }
    }

    pub(super) fn live(&mut self, lifetime: u64, handle: u64, appearance: u32) {
        match self {
            Self::Events(events) => {
                events.detail.lives.insert(lifetime, (handle, appearance));
            }
            Self::Journal(journal) => journal.ops.push(Op::Live {
                lifetime,
                handle,
                appearance,
            }),
        }
    }

    pub(super) fn close_run(&mut self, at: At, last: i32) {
        match self {
            Self::Events(events) => events.close_run(at, last),
            Self::Journal(journal) => {
                let (carried, at) = split(at);
                journal.ops.push(Op::CloseRun { carried, at, last });
            }
        }
    }

    pub(super) fn output_variants(&mut self, variants: impl Iterator<Item = u64>) {
        match self {
            Self::Events(events) => events.overview.output_variants.extend(variants),
            Self::Journal(journal) => journal.ops.push(Op::OutputVariants {
                variants: variants.collect(),
            }),
        }
    }

    /// An output read: its place among the chunk's.
    pub(super) fn output(&mut self, record: OutputRecord) -> usize {
        match self {
            Self::Events(events) => {
                events.detail.outputs.push(record);
                events.detail.outputs.len() - 1
            }
            Self::Journal(journal) => {
                journal.ops.push(Op::Output { record });
                count(&mut journal.outputs)
            }
        }
    }

    /// An output read carried into the chunk: its place among those carried.
    pub(super) fn carried_output(&mut self, record: OutputRecord) -> usize {
        match self {
            Self::Events(events) => {
                events.detail.carried_outputs.push(record);
                events.detail.carried_outputs.len() - 1
            }
            Self::Journal(journal) => {
                journal.ops.push(Op::CarriedOutput { record });
                count(&mut journal.carried_outputs)
            }
        }
    }

    pub(super) fn close_output(&mut self, at: At, last: i32) {
        match self {
            Self::Events(events) => events.close_output(at, last),
            Self::Journal(journal) => {
                let (carried, at) = split(at);
                journal.ops.push(Op::CloseOutput { carried, at, last });
            }
        }
    }

    /// Ends the chunk ([`Events::cut`]): a history's events; none of a lens's, which journals
    /// the cut and counts the next chunk's from nothing.
    pub(super) fn cut(
        &mut self,
        counters: &index::Counters,
        produced: Vec<(u64, i32, i32)>,
    ) -> Events {
        match self {
            Self::Events(events) => events.cut(counters, produced),
            Self::Journal(journal) => {
                let mut ops = std::mem::take(&mut journal.ops);
                ops.push(Op::Cut {
                    counters: counters.clone(),
                    produced,
                });
                **journal = Journal {
                    ops,
                    ..Journal::default()
                };
                Events::default()
            }
        }
    }

    /// Lets go of what a lens journaled before its last cut, which no chunk holds.
    pub(super) fn forget_taken(&mut self) {
        if let Self::Journal(journal) = self
            && let Some(at) = journal.ops.iter().rposition(|op| matches!(op, Op::Cut { .. }))
        {
            journal.ops.drain(..=at);
        }
    }

    /// A lens's piece of the chunk it cut last, with what its binding resolves.
    pub(super) fn piece(
        &mut self,
        marks: index::Marks,
        conds: Vec<(u32, u8, u64, u32)>,
        handles: Vec<u64>,
    ) {
        if let Self::Journal(journal) = self {
            journal.ops.push(Op::Piece {
                marks,
                conds,
                handles,
            });
        }
    }

    /// What a lens journaled since it last handed it on.
    pub(super) fn take_ops(&mut self) -> Vec<Op> {
        match self {
            Self::Events(_) => Vec::new(),
            Self::Journal(journal) => std::mem::take(&mut journal.ops),
        }
    }
}

/// A lens's chunk as the emit makes it again from the lens's journal, and the chunk it cut last
/// until its piece is made.
#[derive(Default)]
pub(super) struct Rebuilt {
    events: Events,
    taken: Option<Events>,
}

impl Rebuilt {
    /// Makes `op`'s change; the piece of lens `lens` where it ends one.
    pub(super) fn apply(&mut self, lens: u32, op: Op) -> io::Result<Option<lens::Piece>> {
        if let Op::Piece {
            marks,
            conds,
            handles,
        } = op
        {
            let events = self
                .taken
                .take()
                .ok_or_else(|| crate::invalid("A production lens's piece follows no cut"))?;
            return lens::piece(lens, events, marks, conds, handles).map(Some);
        }
        if let Some(taken) = self.events.apply(op)? {
            self.taken = Some(taken);
        }
        Ok(None)
    }
}
