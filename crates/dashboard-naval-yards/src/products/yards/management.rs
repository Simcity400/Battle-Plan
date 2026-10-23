//! The two yard views' management projection; later hours project only what changed.
use super::*;
use crate::products::{naval_refits, naval_repairs};
use dashboard_base::{
    Source,
    prepared::{
        frame_changes::FrameChanges,
        kept::consecutive,
        prepass::{Extract, Family, Held, HeldBytes},
    },
};
use dashboard_keyed::{Delta, Observer};
use replay::container::{QuietHours, Replay};
use replay_data::cadence::{Cadence, cadence_in};
use std::{any::Any, collections::HashSet, ops::Range};

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

#[derive(Default)]
pub(super) struct Shared {
    binding: Option<dashboard_base::SourceKey>,
    replay: Option<Arc<Replay>>,
}

impl Shared {
    fn bind(&mut self, source: &Source) {
        if self.binding.as_ref() == Some(source.key()) {
            return;
        }
        self.binding = Some(source.key().clone());
        self.replay = Some(source.replay());
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
            self.engine = None;
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
        self.engine = None;
        if frame.complete::<tables::Countries>().is_err()
            || frame.complete::<tables::CountryLinks>().is_err()
        {
            self.dates.clear();
            self.last_hour = None;
            return Ok(false);
        }
        let projection = Projection::read(frame);
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
        self.engine = changes::Engine::build(frame, refits);
        Ok(true)
    }

    /// The hour as changes from the last observation, when the kept projection finds them.
    pub(crate) fn observe_changes(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        sources: &[TableId],
        names: &dashboard_keyed::delta::Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<Option<bool>> {
        let Some(mut engine) = self.engine.take() else {
            return Ok(None);
        };
        let Some(changes) = engine.changes(frame) else {
            return Ok(None);
        };
        // Two refusals one owner first names this hour could intern in either order.
        let mut fresh = HashSet::new();
        for (owner, why) in changes.first_held(engine.totals()) {
            let key = serde_json::to_string(&Operand::Refusal(why.to_owned()))
                .expect("serializable naming operand");
            let new = self
                .interned(&key)
                .is_none_or(|id| !names.numbered(&format!("{owner}|refusal|{id}")));
            if new && !fresh.insert(owner) {
                return Ok(None);
            }
        }
        // Every source is known at both hours: only supplying read dates move.
        let before = self.dates.clone();
        self.date(format, frame, sources);
        for &table in sources {
            let now = self.dates.get(&table.index()).copied();
            if !dated(table, format) || before.get(&table.index()).copied() == now {
                continue;
            }
            for owner in engine.owners() {
                let name = format!("{owner}|read|{table:?}");
                match now {
                    Some(read) => delta.set(name, i128::from(read)),
                    None => delta.remove(name),
                }
            }
        }
        engine.publish(changes, self, delta);
        self.engine = Some(engine);
        Ok(Some(true))
    }

    pub(crate) fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .add(self.tokens.capacity() * size_of::<Operand>())
            .table::<String, usize>(self.index.capacity())
            .tree::<usize, i32>(self.dates.len());
        for operand in &self.tokens {
            operand.held(bytes);
        }
        for key in self.index.keys() {
            bytes.heap(key);
        }
        bytes.table::<(String, usize), usize>(self.refusal_counts.capacity());
        for (owner, _) in self.refusal_counts.keys() {
            bytes.heap(owner);
        }
        if let Some(Ok(designs)) = &self.designs {
            bytes.table::<u64, Operand>(designs.capacity());
            for operand in designs.values() {
                operand.held(bytes);
            }
        }
        bytes.table::<u64, (Values, Vec<(usize, usize)>)>(self.ships.capacity());
        for ship in self.ships.values() {
            ship.values.held(bytes);
            bytes.heap(&ship.refusals);
        }
        bytes
            .heap(&self.ship_views)
            .add(self.ship_refusals.as_ref().map_or(0, |held| held.capacity()) * size_of::<(usize, usize)>());
        if let Some(engine) = &self.engine {
            changes::Engine::held(engine, bytes);
        }
    }

    pub(crate) fn unpublished(&self) -> usize {
        self.unpublished
    }

    pub(crate) fn published(&mut self) {
        self.unpublished = 0;
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
