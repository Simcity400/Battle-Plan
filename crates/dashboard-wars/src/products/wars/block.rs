//! Wars prepared block by block, on their own. A pre-pass worker reads a
//! block alone with the wars inputs, only at the hours a field the observer
//! compares changes ([`FAMILY`]'s fields), and runs the observer on it from
//! the block's first hour with no state of earlier blocks: it observes each
//! kept hour and authenticates each hour of the spans between (which its
//! recording keeps as one span, made again in constant time), recording the
//! calls it makes on its series after the first hour, the
//! change inputs it publishes and the relation lifetimes it numbers. A
//! recorded capitulation date needs no field to change: it is the extract's
//! deadline, and that hour is observed on the last hour's rows. Casualty
//! counters move every hour of a war, so wartime hours are observed there, on
//! the workers.
//!
//! The family's own thread folds the blocks in order. It observes each
//! block's first hour on the carried observer, as hourly preparation does,
//! and then continues from the extract. After that hour both observers hold
//! inputs equal in every compared field, a join of the same relations and
//! the same last value of every counter of those relations; they differ only
//! in the numbering of relation lifetimes and the count of change inputs. So
//! the fold makes the recorded calls again under the carried numbering
//! (lifetimes the extract met first are numbered as continued observation
//! numbers them), observes each change-input ordinal on the carried count,
//! takes each input view the extract replaced after the first hour, its join
//! and its next capitulation date, and walks the published inputs with the
//! recorded viewpoint's walk after the hour observed before each. No hour but
//! the first is observed on the carried observer. When the carried observer
//! does not continue the first hour as the extract's did, or the extract's
//! observer took its relations from its open counters for want of a join,
//! the rest of the block is observed hour by hour.

use super::Hour;
use super::native::World;
use super::prepared::Observer;
use super::published::{Prepared as Walk, Saved};
use dashboard_base::{
    Source,
    prepared::{
        frame_changes::{FrameChangeTracker, FrameChanges},
        prepass::{Extract, Family, Folded, Held, HeldBytes},
        series::{self, Accumulator, Recording, ReplayTarget},
    },
    products::LoadContext,
};
use replay::{
    container::BlockIndex, container::QuietHours, container::Replay, container::TableFields,
};
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::{frame::DecodedFrame, frame::SharedView, frame::TableView};
use std::{
    any::Any,
    io,
    ops::Range,
    sync::{Arc, atomic::AtomicBool},
};

/// The tables the wars observer reads: the wars inputs without the clock.
pub const INPUTS: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WarRelations,
    TableId::WarOwnedRelations,
    TableId::WarRelationGoals,
    TableId::WarOverviewCountries,
    TableId::WarNameRules,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::LocalPlayer,
    TableId::Capitulation,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::States,
];

pub static FAMILY: Family = Family {
    name: "wars",
    extract: "wars.extract",
    sequential: "wars.sequential",
    fallback: "wars.fallback",
    tables: INPUTS,
    fields: Some(fields),
    start,
};

/// The fields the observer compares ([`super::prepared::same_inputs`]), so
/// that a block is read only at the hours they change. Every other table is
/// compared whole.
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Compared whole: any row change is a change input.
        TableFields::all(TableId::Campaign),
        TableFields::all(TableId::CountryLinks),
        // Every field, casualties included: their counters are published
        // per relation lifetime, and the rest is compared whole.
        TableFields::all(TableId::WarRelations),
        TableFields::all(TableId::WarOwnedRelations),
        TableFields::all(TableId::WarRelationGoals),
        TableFields::all(TableId::WarOverviewCountries),
        TableFields::all(TableId::WarNameRules),
        TableFields::all(TableId::FactionMembers),
        TableFields::all(TableId::LocalPlayer),
        TableFields::all(TableId::LexicalTokens),
        TableFields::all(TableId::ReservedCountryNaming),
        // Country keys: the id (the row key), slot and tag. Balances change
        // every hour and no input reads them.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // A complete Factions view always compares equal: only its rows'
        // keys and availability.
        TableFields::keys(TableId::Factions),
        // Capitulation flags and dates; legitimacy and the other dates are
        // not compared.
        TableFields::named(TableId::Capitulation, &["capitulated", "capitulated_hour"])?,
        // The naming operands `war_casualties::same_name_rows` compares.
        TableFields::named(
            TableId::CountryAppearance,
            &["cosmetic_tag", "name_tag", "leader_ideology", "overlord"],
        )?,
        TableFields::named(TableId::Politics, &["ruling_party"])?,
        TableFields::named(TableId::PoliticalParties, &["token"])?,
        TableFields::named(TableId::Autonomy, &["state"])?,
        // State names; owners, controllers and manpower are not compared.
        TableFields::named(TableId::States, &["custom_name", "owner_name"])?,
    ])
}

/// The inputs compared by some of their fields only. A truncated view of one
/// is compared whole, and its unwatched rows can change at hours a read by
/// changes does not keep, so a block holding one is observed hour by hour.
const PARTIAL: [TableId; 8] = [
    TableId::Countries,
    TableId::Factions,
    TableId::Capitulation,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::States,
];

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        observer: Box::default(),
        recording: Recording::default(),
        first: None,
        held: Vec::new(),
        steps: Vec::new(),
        hour: None,
        #[cfg(any(test, feature = "test-support"))]
        frames: 0,
    })
}

/// The block's first hour, which the fold observes on the carried observer.
struct First {
    frame: DecodedFrame,
    changes: FrameChanges,
    /// Whether the extract's observer held a join of that hour.
    joined: bool,
}

/// The change inputs the extract's observer published at `hour`, which the
/// recorded viewpoint's walk follows after the hour observed before.
struct Step {
    before: i32,
    hour: i32,
    inputs: Vec<(i32, Vec<u8>)>,
}

struct BlockFacts {
    first: First,
    /// The calls the extract's observer made on its series after the first
    /// hour.
    recording: Recording,
    /// The input views it held right after the first hour.
    held: Vec<SharedView>,
    steps: Vec<Step>,
    /// Its relation lifetimes in key order, and the hour each was first read.
    names: Vec<String>,
    first_reads: Vec<i32>,
    /// The change-input ordinals it observed after the first hour.
    recorded: Vec<(i32, bool)>,
    /// What it held at the block's end: its inputs, its join and its next
    /// capitulation date.
    frame: Option<DecodedFrame>,
    inputs: Option<Hour>,
    next_capitulation: Option<i32>,
    /// Whether it took its relations from its open counters after the first
    /// hour.
    unjoined: bool,
    /// The last hour observed or quiet.
    last: i32,
    /// Frames the extract was given or built.
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

struct BlockExtract {
    observer: Box<Observer>,
    recording: Recording,
    first: Option<First>,
    held: Vec<SharedView>,
    steps: Vec<Step>,
    /// The last hour observed or quiet.
    hour: Option<i32>,
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

impl Held for World {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.owned)
            .heap(&self.relations)
            .heap(&self.adjacent)
            .heap(&self.factions)
            .heap(&self.scores);
    }
}

/// The inputs the observer holds at the block's end: its world and every
/// country and relation map beside it.
impl Held for Hour {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.world)
            .heap(&self.relation_order)
            .heap(&self.tags)
            .heap(&self.identities)
            .heap(&self.raw_to_slot)
            .heap(&self.goal_owned);
    }
}

impl Extract for BlockExtract {
    /// What the block's facts hold: its first frame, its recorded calls,
    /// the views it holds, each step's inputs and the observer's state (its
    /// series, its relation lifetimes by name and the hour each was first
    /// read, the change-input ordinals it recorded for the fold, and its
    /// frame and inputs at the block's end).
    fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        let observer = &self.observer;
        // The relation lifetimes by number, as the facts name them.
        held.add(observer.indices.len() * size_of::<String>())
            .add(observer.indices.keys().map(String::capacity).sum())
            .heap(&observer.first_reads)
            .heap(&observer.recorded)
            .heap(&observer.counter_keys)
            .heap(&observer.inputs);
        if let Some(first) = &self.first {
            held.frame(&first.frame).changes(&first.changes);
        }
        held.add(self.recording.bytes());
        for view in &self.held {
            held.view(view);
        }
        held.add(self.steps.capacity() * size_of::<Step>());
        for step in &self.steps {
            held.add(
                step.inputs.capacity() * size_of::<(i32, Vec<u8>)>()
                    + step
                        .inputs
                        .iter()
                        .map(|(_, input)| input.capacity())
                        .sum::<usize>(),
            );
        }
        if let Some(frame) = &self.observer.frame {
            held.frame(frame);
        }
        held.add(self.observer.bytes());
        held.total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.frames += 1;
        }
        if PARTIAL
            .iter()
            .any(|table| matches!(&*frame.tables[table.index()], TableView::Truncated { .. }))
        {
            return Err(io::Error::other(
                "A wars input compared by some of its fields is truncated",
            ));
        }
        if self.first.is_none() {
            self.observer.observe_changed(frame, changes)?;
            self.first = Some(First {
                frame: frame.clone(),
                changes: FrameChanges {
                    rows_changed: changes.rows_changed,
                    table_read: changes.table_read,
                    slot_read: changes.slot_read,
                    updates: None,
                },
                joined: self.observer.inputs.is_some(),
            });
            self.held = self
                .observer
                .frame
                .as_ref()
                .map(|held| held.tables.to_vec())
                .unwrap_or_default();
            self.observer.record_calls();
        } else {
            let before = self
                .hour
                .ok_or_else(|| io::Error::other("Wars hours observed before the block's first"))?;
            let observer = &mut self.observer;
            series::with_recording(&mut self.recording, || {
                observer.observe_changed(frame, changes)
            })?;
            let inputs = self.observer.take_inputs();
            if !inputs.is_empty() {
                self.steps.push(Step {
                    before,
                    hour: frame.hour,
                    inputs,
                });
            }
        }
        self.hour = Some(frame.hour);
        Ok(())
    }

    /// The next recorded capitulation date, at which the observer publishes
    /// a change input with no field changing.
    fn deadline(&self) -> Option<i32> {
        self.observer.next_capitulation
    }

    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let span = &hours.hours()[span];
        let (Some(&first), Some(&last)) = (span.first(), span.last()) else {
            return Ok(());
        };
        if !self.observer.quiet_continues(first, last) {
            return Err(io::Error::other(
                "Wars quiet hours do not continue the hour observed last",
            ));
        }
        let observer = &mut self.observer;
        series::with_recording(&mut self.recording, || observer.advance_quiet(first, last))?;
        self.hour = Some(last);
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        let BlockExtract {
            observer,
            recording,
            first,
            held,
            steps,
            hour,
            #[cfg(any(test, feature = "test-support"))]
            frames,
        } = *self;
        let first = first.ok_or_else(|| io::Error::other("Wars block has no first hour"))?;
        let last = hour.ok_or_else(|| io::Error::other("Wars block has no hours"))?;
        let observer = *observer;
        let names = series::catalogue_names(&observer.indices)?;
        Ok(Box::new(BlockFacts {
            first,
            recording,
            held,
            steps,
            names,
            first_reads: observer.first_reads,
            recorded: observer.recorded.unwrap_or_default(),
            frame: observer.frame,
            inputs: observer.inputs,
            next_capitulation: observer.next_capitulation,
            unjoined: observer.unjoined,
            last,
            #[cfg(any(test, feature = "test-support"))]
            frames,
        }))
    }
}

/// The observation of one hour's outcome: `Ok(true)` while the family goes
/// on, `Ok(false)` once it refused the branch.
pub type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// The recorded viewpoint's walk, when preparation publishes it, follows
/// the inputs the wars published through `hour`.
fn walk(walk: &mut Option<Walk>, observer: &mut Observer, hour: i32) -> io::Result<()> {
    match walk {
        Some(walk) => walk.after(observer.take_inputs(), hour),
        None => Ok(()),
    }
}

/// The carried series the extract's calls are made again on: its relation
/// lifetime `k`'s counters are the carried lifetime `keys[k]`'s.
struct Target<'a> {
    batch: &'a mut Accumulator,
    keys: &'a [u64],
    recorded: &'a [(i32, bool)],
}

impl ReplayTarget for Target<'_> {
    fn series(&mut self, _: u8) -> &mut Accumulator {
        &mut *self.batch
    }

    fn key(&self, _: u8, key: u64) -> u64 {
        match key.checked_sub(1) {
            // The change inputs.
            None => 0,
            Some(key) => 1 + self.keys[(key / 4) as usize] * 4 + key % 4,
        }
    }

    /// The change inputs' ordinal, observed on the carried count.
    fn family(&mut self, index: usize) -> io::Result<()> {
        let &(hour, gap) = self
            .recorded
            .get(index)
            .ok_or_else(|| io::Error::other("Wars fold has no recorded change-input ordinal"))?;
        let event = self
            .batch
            .state(0)
            .and_then(|state| state.events.checked_sub(1))
            .ok_or_else(|| io::Error::other("Wars fold has no carried change input"))?;
        self.batch.observe(0, hour, Some(i128::from(event)), gap);
        Ok(())
    }
}

/// Whether the carried observer, having observed the block's first hour,
/// continues it as the extract's did: it holds that hour's inputs and a join
/// exactly when the extract's does, its change inputs' ordinal is the last it
/// counted, and the extract's never took its relations from its open
/// counters.
fn continues(observer: &Observer, facts: &BlockFacts) -> bool {
    let hour = facts.first.frame.hour;
    !facts.unjoined
        && observer
            .frame
            .as_ref()
            .is_some_and(|frame| frame.hour == hour)
        && observer.inputs.is_some() == facts.first.joined
        && observer.batch.state(0).is_some_and(|state| {
            state
                .events
                .checked_sub(1)
                .is_some_and(|event| state.last.is_some_and(|run| run.value == i128::from(event)))
        })
}

/// Continue the carried observer and its walk through the extract's block
/// after the first hour, which both observed.
fn carry(
    observer: &mut Observer,
    published: &mut Option<Walk>,
    facts: BlockFacts,
) -> io::Result<()> {
    let BlockFacts {
        recording,
        held,
        steps,
        names,
        first_reads,
        recorded,
        frame,
        inputs,
        next_capitulation,
        last,
        ..
    } = facts;
    if names.len() != first_reads.len() {
        return Err(io::Error::other(
            "Wars block relation lifetimes and first reads disagree",
        ));
    }
    // The extract's lifetimes in the order it first read them: a carried one
    // keeps its number, a new one takes the next, with its first read, as
    // continued observation numbers it.
    let mut keys = Vec::with_capacity(names.len());
    for (name, first_read) in names.into_iter().zip(first_reads) {
        let next = observer.indices.len() as u64;
        let key = *observer.indices.entry(name).or_insert_with(|| {
            observer.first_reads.push(first_read);
            next
        });
        keys.push(key);
    }
    recording.replay(&mut Target {
        batch: &mut observer.batch,
        keys: &keys,
        recorded: &recorded,
    })?;
    // The carried observer keeps each held view the extract's kept since the
    // first hour (equal in every compared field) and takes each it replaced.
    if let (Some(carried), Some(frame)) = (observer.frame.as_mut(), frame) {
        let hour = frame.hour;
        for ((carried, held), view) in carried.tables.iter_mut().zip(&held).zip(frame.tables) {
            if !SharedView::ptr_eq(held, &view) {
                *carried = view;
            }
        }
        carried.hour = hour;
    }
    observer.inputs = inputs;
    // The extract's counter numbers are local to its block.
    observer.counter_keys.clear();
    observer.next_capitulation = next_capitulation;
    if let Some(walk) = published {
        for Step {
            before,
            hour,
            inputs,
        } in steps
        {
            walk.after(Vec::new(), before)?;
            walk.after(inputs, hour)?;
        }
        walk.after(Vec::new(), last)?;
    }
    Ok(())
}

/// Fold one block's facts into the carried observer and its walk. Returns
/// how it folded and the last hour it reached.
fn fold(
    observer: &mut Observer,
    published: &mut Option<Walk>,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let facts = {
        let _cpu = ::replay_frame::cpu::scope("fold facts", None);
        facts
            .downcast::<BlockFacts>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
    };
    #[cfg(any(test, feature = "test-support"))]
    FRAMES.with(|count| count.set(count.get() + facts.frames));
    check()?;
    let hour = facts.first.frame.hour;
    #[cfg(any(test, feature = "test-support"))]
    OBSERVED.with(|count| count.set(count.get() + 1));
    let result = {
        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
        observer
            .observe_changed(&facts.first.frame, &facts.first.changes)
            .and_then(|()| walk(published, observer, hour))
    };
    if !observed(hour, result)? {
        return Ok((Folded::Fast, Some(hour)));
    }
    if !continues(observer, &facts) {
        let reached = observe_block(
            observer,
            published,
            replay,
            index,
            Some(hour),
            observed,
            check,
        )?;
        return Ok((Folded::Fallback, reached.or(Some(hour))));
    }
    check()?;
    let last = facts.last;
    let result = {
        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
        carry(observer, published, *facts)
    };
    observed(last, result)?;
    Ok((Folded::Fast, Some(last)))
}

/// Observe the block hour by hour after `after` (every hour without it),
/// with the change facts of the block alone, as hourly preparation does.
/// Returns the last hour observed.
fn observe_block(
    observer: &mut Observer,
    published: &mut Option<Walk>,
    replay: &Replay,
    index: &BlockIndex,
    after: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let read = replay.read_block_alone(
        index,
        INPUTS,
        |raw, frame| {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            let facts = tracker.observe_updates(frame, Arc::new(raw), replay.format);
            if stopped || after.is_some_and(|after| frame.hour <= after) {
                return Ok(());
            }
            reached = Some(frame.hour);
            #[cfg(any(test, feature = "test-support"))]
            OBSERVED.with(|count| count.set(count.get() + 1));
            let result = {
                let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                observer
                    .observe_changed(frame, &facts)
                    .and_then(|()| walk(published, observer, frame.hour))
            };
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Wars observation stopped"))
                }
            }
        },
        check,
    );
    if let Some(error) = failure {
        return Err(error);
    }
    read?;
    Ok(reached)
}

/// What the recorded viewpoint's walk starts from when the branch's first
/// block binds it.
enum Start {
    /// New, with the observer.
    New,
    /// As the last commit published it.
    Saved(Box<Saved>),
    /// Not published: the observer was restored without a published walk.
    Unpublished,
}

/// One branch's wars prepared on their own: the observer hourly preparation
/// keeps in its branch state, whose continuation is its published catalogue,
/// and the recorded viewpoint's walk over the inputs it publishes.
pub struct OwnBranch {
    observer: Box<Observer>,
    start: Option<Start>,
    walk: Option<Walk>,
}

impl OwnBranch {
    /// The branch's observer and walk as `saved` last committed them, or new
    /// without a commit to read.
    pub fn load(
        store: &store::Store,
        key: &store::SourceKey,
        saved: Option<&store::SourceKey>,
        branch: u64,
    ) -> io::Result<Self> {
        let restored = match saved {
            None => None,
            Some(_) => series::read_saved_catalogue(store, key, saved, "wars", branch)?
                .map(Observer::restore),
        };
        let start = match &restored {
            None => Start::New,
            Some(_) => Saved::read(store, key, saved, branch)?
                .map_or(Start::Unpublished, |saved| Start::Saved(Box::new(saved))),
        };
        Ok(Self {
            observer: Box::new(restored.unwrap_or_default()),
            start: Some(start),
            walk: None,
        })
    }

    /// Bind the walk to the branch's first hour and the store's content
    /// catalogue before the first block it observes. A catalogue that cannot
    /// be read, a moved first hour or a restored observer without a published
    /// walk publish none: the default query then walks the inputs.
    fn bind(
        &mut self,
        replay: &Replay,
        index: &BlockIndex,
        source: &Source,
        context: &LoadContext,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        let Some(start) = self.start.take() else {
            return Ok(());
        };
        let first = replay
            .branches
            .iter()
            .find(|branch| branch.id == index.branch)
            .map(|branch| dashboard_text::metadata::branch_bounds(replay, branch).0);
        let cat = match context.content.as_ref() {
            None => Some(None),
            Some(options) => match dashboard_base::catalogue::catalogue(source, options) {
                Ok(cat) => Some(Some(cat)),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(_) => None,
            },
        };
        let (Some(first), Some(cat)) = (first, cat) else {
            return Ok(());
        };
        self.walk = match start {
            Start::New => Some(Walk::new(index.branch, first, cat)),
            Start::Saved(saved) if saved.first() == first => {
                Some(Walk::load(*saved, cat, index.branch)?)
            }
            Start::Saved(_) | Start::Unpublished => None,
        };
        if self.walk.is_some() {
            self.observer.keep_inputs();
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fold_facts(
        &mut self,
        facts: Box<dyn Any + Send>,
        replay: &Replay,
        index: &BlockIndex,
        source: &Source,
        context: &LoadContext,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.bind(replay, index, source, context)?;
        fold(
            &mut self.observer,
            &mut self.walk,
            facts,
            replay,
            index,
            observed,
            check,
        )
    }

    pub fn observe_hours(
        &mut self,
        replay: &Replay,
        index: &BlockIndex,
        source: &Source,
        context: &LoadContext,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        self.bind(replay, index, source, context)?;
        observe_block(
            &mut self.observer,
            &mut self.walk,
            replay,
            index,
            None,
            observed,
            check,
        )
    }

    pub fn pending_bytes(&self) -> usize {
        self.observer.bytes() + self.walk.as_ref().map_or(0, Walk::bytes)
    }

    /// The flush hourly preparation makes at a batch's end, then the walk's
    /// publication.
    pub fn flush_batch(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.observer.flush(writer, branch, cancelled)?;
        match &mut self.walk {
            Some(walk) => walk.flush(writer, branch),
            None => Ok(()),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static OBSERVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FRAMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Hours observed on the carried observer on this thread: each folded
/// block's first hour, and every hour of a block observed hour by hour.
#[cfg(any(test, feature = "test-support"))]
pub fn observed_hours() -> usize {
    OBSERVED.with(std::cell::Cell::get)
}

/// Frames the extracts of the blocks folded on this thread were given or
/// built: every hour of a block read whole, only the hours kept of a block
/// read by its changes.
#[cfg(any(test, feature = "test-support"))]
pub fn built_frames() -> usize {
    FRAMES.with(std::cell::Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wars_inputs_are_the_wars_tables_without_the_clock() {
        let inputs: Vec<_> = super::super::TABLES
            .iter()
            .copied()
            .filter(|&table| super::super::prepared::is_input(table))
            .collect();
        assert_eq!(inputs, INPUTS);
        let watched: Vec<_> = fields().unwrap().iter().map(|watch| watch.table).collect();
        for table in INPUTS {
            assert!(watched.contains(table), "{table:?} is watched");
        }
        assert_eq!(watched.len(), INPUTS.len());
    }
}
