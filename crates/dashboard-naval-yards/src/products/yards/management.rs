//! One management projection for the two keyed yard views. Native hull and refit
//! operands stay owned by dashboard-forces; the ordinary observation LRU shares
//! this history adaptation between their independently published windows.
use super::*;
use crate::products::{naval_refits, naval_repairs};
use dashboard_base::{
    Source,
    prepared::{
        frame_changes::FrameChanges,
        kept::consecutive,
        prepass::{Extract, Family, Held, HeldBytes},
        query::observations::{self, Scope},
    },
};
use dashboard_keyed::Observer;
use replay::container::{QuietHours, Replay};
use replay_data::cadence::{Cadence, cadence_in};
use replay_frame::frame::ViewIdentity;
use std::{any::Any, ops::Range, sync::atomic::AtomicBool};

pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::LexicalTokens,
    TableId::TaskForceCounterInputs,
    TableId::VariantDefinitions,
    TableId::NavalRepairCountries,
    TableId::CountryFactories,
    TableId::NavalBases,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::NavalRefitShips,
    TableId::NavalRefitProgress,
    TableId::ProductionLines,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::NavalRepairTaskForces,
    TableId::NavalRepairChildren,
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
];

pub(crate) const CACHE: &str = "naval-management-yards";

#[derive(Default)]
pub(super) struct Shared {
    binding: Option<dashboard_base::SourceKey>,
    scope: Option<Scope>,
    replay: Option<Arc<Replay>>,
    current: Option<(FormatVersion, Vec<ViewIdentity>, Arc<Projection>)>,
    pending: Option<Pending>,
}

struct Pending {
    block: (u64, i32, i32),
    projections: BlockProjection,
}

struct BlockProjection(Vec<(i32, Arc<Projection>)>);

impl BlockProjection {
    fn at(&self, hour: i32) -> io::Result<Arc<Projection>> {
        let through = self.0.partition_point(|(first, _)| *first <= hour);
        through
            .checked_sub(1)
            .map(|position| Arc::clone(&self.0[position].1))
            .ok_or_else(|| io::Error::other("Naval management block has no preceding projection"))
    }

    fn bytes(&self) -> usize {
        let mut held = HeldBytes::default();
        held.add(size_of::<Self>()).heap(&self.0);
        held.total() as usize
    }
}

impl Shared {
    fn bind(&mut self, source: &Source) {
        if self.binding.as_ref() == Some(source.key()) {
            return;
        }
        self.binding = Some(source.key().clone());
        self.scope = Some(observations::scope(source, CACHE, None));
        self.replay = Some(source.replay());
        self.current = None;
        self.pending = None;
    }

    pub(super) fn continues(&self, last: Option<i32>, branch: u64, hour: i32) -> bool {
        last.is_some_and(|last| {
            last.checked_add(1) == Some(hour)
                || (last < hour
                    && self.replay.as_ref().is_some_and(|replay| {
                        replay.index.iter().any(|block| {
                            block.branch == branch
                                && block.first_hour <= last
                                && hour <= block.last_hour
                        }) && !replay.gaps.iter().any(|gap| {
                            gap.branch == branch && gap.first_hour <= hour && last <= gap.last_hour
                        })
                    }))
        })
    }

    fn project(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
    ) -> io::Result<Arc<Projection>> {
        let unchanged = self
            .current
            .as_ref()
            .and_then(|(held_format, views, projection)| {
                (*held_format == format
                    && views
                        .iter()
                        .zip(TABLES)
                        .all(|(view, table)| view.is(&frame.tables[table.index()])))
                .then(|| Arc::clone(projection))
            });
        let projection = if let Some(scope) = &self.scope {
            let block = self
                .replay
                .as_ref()
                .expect("bound naval management replay")
                .index
                .iter()
                .find(|block| {
                    block.branch == frame.branch
                        && block.first_hour <= frame.hour
                        && frame.hour <= block.last_hour
                })
                .ok_or_else(|| {
                    io::Error::other("Naval management frame is outside the recorded blocks")
                })?;
            let cancelled = AtomicBool::new(false);
            if let Some(held) = scope.get_cached::<BlockProjection>(block, &(), &cancelled)? {
                self.pending = None;
                held.at(frame.hour)?
            } else {
                let projection = match unchanged {
                    Some(projection) => projection,
                    None => scope.get_query(
                        block,
                        &frame.hour,
                        &cancelled,
                        Projection::bytes,
                        || Ok(Projection::read(frame)),
                    )?,
                };
                let key = (block.branch, block.first_hour, block.last_hour);
                if self
                    .pending
                    .as_ref()
                    .is_none_or(|pending| pending.block != key)
                {
                    self.pending = Some(Pending {
                        block: key,
                        projections: BlockProjection(Vec::new()),
                    });
                }
                let pending = self.pending.as_mut().expect("current management block");
                if pending
                    .projections
                    .0
                    .last()
                    .is_none_or(|(_, previous)| !Arc::ptr_eq(previous, &projection))
                {
                    pending
                        .projections
                        .0
                        .push((frame.hour, Arc::clone(&projection)));
                }
                if frame.hour == block.last_hour && block.first_hour != block.last_hour {
                    let pending = self.pending.take().expect("completed management block");
                    if pending
                        .projections
                        .0
                        .first()
                        .is_some_and(|(hour, _)| *hour == block.first_hour)
                    {
                        // Families fold a whole block separately. Retain its
                        // immutable changed projections as one ordinary LRU
                        // observation, not competing per-hour entries.
                        scope.get_query(block, &(), &cancelled, BlockProjection::bytes, || {
                            Ok(pending.projections)
                        })?;
                    }
                }
                projection
            }
        } else {
            unchanged.unwrap_or_else(|| Arc::new(Projection::read(frame)))
        };
        self.current = Some((
            format,
            TABLES
                .iter()
                .map(|table| frame.tables[table.index()].identity())
                .collect(),
            Arc::clone(&projection),
        ));
        Ok(projection)
    }
}

struct Projection {
    tokens: Vec<Operand>,
    owners: Vec<String>,
    repairs: Vec<(String, i128)>,
    refits: Vec<(String, i128)>,
}

impl Projection {
    fn read(frame: &DecodedFrame) -> Self {
        dashboard_base::prepared::prepass::note("naval-yards.project");
        let countries = Countries::read(frame).expect("yard identity tables checked");
        let owners = super::owners(frame, &countries);
        let mut state = State::default();
        let mut repairs = Values::default();
        let mut refits = Values::default();
        naval_repairs::project(&mut state, frame, &countries, &owners, &mut repairs);
        state.refusal_counts.clear();
        naval_refits::project(&mut state, frame, &countries, &owners, &mut refits);
        Self {
            tokens: state.tokens,
            owners,
            repairs: repairs
                .iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
            refits: refits
                .iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        }
    }

    fn bytes(&self) -> usize {
        let mut held = HeldBytes::default();
        held.value(self);
        held.total() as usize
    }
}

impl Held for Projection {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.tokens)
            .heap(&self.owners)
            .heap(&self.repairs)
            .heap(&self.refits);
    }
}

impl State {
    pub(crate) fn bind(&mut self, source: &Source) {
        if self
            .management
            .binding
            .as_ref()
            .is_some_and(|binding| binding != source.key())
        {
            self.dates.clear();
            self.last_hour = None;
        }
        self.management.bind(source);
    }

    pub(crate) fn observe(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        sources: &[TableId],
        refits: bool,
        values: &mut Values,
    ) -> io::Result<bool> {
        if frame.complete::<tables::Countries>().is_err()
            || frame.complete::<tables::CountryLinks>().is_err()
        {
            self.dates.clear();
            self.last_hour = None;
            return Ok(false);
        }
        let projection = self.management.project(format, frame)?;
        self.sources(format, frame, sources, &projection.owners, values);
        let mut tokens = HashMap::new();
        for (name, amount) in if refits {
            &projection.refits
        } else {
            &projection.repairs
        } {
            if let Some((owner, id)) = name.split_once("|refusal|") {
                let id = id.parse::<usize>().map_err(io::Error::other)?;
                let mapped = *tokens
                    .entry(id)
                    .or_insert_with(|| self.intern(projection.tokens[id].clone()));
                self.refusal(
                    values,
                    owner,
                    mapped,
                    usize::try_from(*amount).map_err(io::Error::other)?,
                );
            } else if is_label(name.rsplit('|').next().unwrap_or_default()) {
                let id = usize::try_from(*amount).map_err(io::Error::other)?;
                let mapped = *tokens
                    .entry(id)
                    .or_insert_with(|| self.intern(projection.tokens[id].clone()));
                values.push(name.clone(), mapped as i128);
            } else {
                values.push(name.clone(), *amount);
            }
        }
        Ok(true)
    }
}

/// Retain the shared keyed extract and fold, forwarding sibling reads and the
/// quiet block endpoint from their read evidence. No campaign walk or series is added.
pub const fn family<O: Observer>(
    name: &'static str,
    extract: &'static str,
    sequential: &'static str,
    fallback: &'static str,
) -> Family {
    Family {
        start: start::<O>,
        ..dashboard_keyed::block::family::<O>(name, extract, sequential, fallback)
    }
}

fn start<O: Observer>(format: FormatVersion) -> Box<dyn Extract> {
    let family = dashboard_keyed::block::family::<O>(O::NAME, "", "", "");
    Box::new(Siblings {
        format,
        inner: (family.start)(format),
        frame: None,
        last: None,
    })
}

struct Siblings {
    format: FormatVersion,
    inner: Box<dyn Extract>,
    frame: Option<DecodedFrame>,
    last: Option<i32>,
}

pub fn changed(changes: &FrameChanges) -> bool {
    // Sequential fallback visits every recorded update, stamping its endpoint
    // even when its rows are held. Sparse kept facts have no recorded updates.
    changes.recorded().is_some()
        || TABLES
            .iter()
            .any(|table| changes.rows_changed[table.index()])
        || TABLES.iter().any(|table| {
            !matches!(cadence_in(*table, FormatVersion::CURRENT), Cadence::Hourly)
                && changes.slot_read[table.index()]
        })
}

impl Extract for Siblings {
    fn read_by_changes(&mut self) {
        self.inner.read_by_changes();
    }

    fn deadline(&self) -> Option<i32> {
        self.inner.deadline()
    }

    fn observe(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        if self.last.is_none() {
            self.last = replay
                .index
                .iter()
                .find(|block| {
                    block.branch == frame.branch
                        && block.first_hour <= frame.hour
                        && frame.hour <= block.last_hour
                })
                .map(|block| block.last_hour);
        }
        self.frame = Some(frame.clone());
        self.inner.observe(replay, frame, changes)
    }

    fn quiet(&mut self, replay: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        consecutive(hours, span.clone())?;
        let mut first = span.start;
        for position in span.clone() {
            let hour = hours.hours()[position];
            let last = self.last == Some(hour);
            if !last
                && !TABLES.iter().any(|table| {
                    !matches!(cadence_in(*table, self.format), Cadence::Hourly)
                        && dashboard_text::metadata::slot_read(
                            self.format,
                            &hours.metrics()[position],
                            *table,
                            hour,
                        )
                })
            {
                continue;
            }
            self.inner.quiet(replay, hours, first..position)?;
            let frame = self.frame.as_mut().ok_or_else(|| {
                io::Error::other("Naval management quiet span has no preceding frame")
            })?;
            frame.hour = hour;
            frame.metrics = hours.metrics()[position].clone();
            for &table in TABLES {
                frame.tables[table.index()] = frame.tables[table.index()]
                    .clone()
                    .with_sampled_at(hours.sampled_at(position, table));
            }
            // Stamp a quiet block endpoint on carried state as well as the series.
            // A conservative endpoint observation cannot carry a date across a
            // later unreadable block, and reuses the unchanged row projection.
            let changes = FrameChanges::rebuilt(frame, self.format, !last, |_| false);
            self.inner.observe(replay, frame, &changes)?;
            first = position + 1;
        }
        self.inner.quiet(replay, hours, first..span.end)
    }

    fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        held.add(size_of::<Self>());
        if let Some(frame) = &self.frame {
            held.frame(frame);
        }
        held.total().saturating_add(self.inner.bytes())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        self.inner.finish()
    }
}

impl Held for Operand {
    fn held(&self, bytes: &mut HeldBytes) {
        match self {
            Operand::ShipCache { bytes: text, .. } | Operand::Task { bytes: text, .. } => {
                bytes.heap(text);
            }
            Operand::ShipRebuilt { naming, group, .. } => {
                bytes.heap(&naming.override_name).heap(group);
            }
            Operand::Design(raw) => {
                bytes
                    .heap(&raw.token)
                    .heap(&raw.display_name)
                    .heap(&raw.suffix)
                    .heap(&raw.name_tag)
                    .heap(&raw.type_tag)
                    .heap(&raw.design)
                    .heap(&raw.refusals);
            }
            Operand::Class { spelling, .. } => {
                bytes.heap(spelling);
            }
            Operand::Port(port) => {
                bytes.heap(&port.override_name).heap(&port.owner_tag);
                match &port.state {
                    Ok(state) => {
                        bytes.heap(&state.custom).heap(&state.owner);
                    }
                    Err(why) => {
                        bytes.heap(why);
                    }
                }
            }
            Operand::Refusal(why) => {
                bytes.heap(why);
            }
            Operand::Policy { .. } => {}
        }
    }
}
