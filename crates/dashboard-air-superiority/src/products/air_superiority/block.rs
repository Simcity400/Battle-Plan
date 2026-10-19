//! Air superiority prepared on its own, block by block, with its own read of
//! the country groups: hourly preparation observes the groups and then air
//! superiority at every hour, and air superiority chooses each day's sides
//! from the groups' state of that hour, so this family keeps a groups scan and
//! change publisher of its own and makes the groups' calls on the groups'
//! frames at the same hours, before its own. Its groups are saved under its
//! own names (`air-superiority-groups-<branch>`, the change publisher's pages
//! as `air-superiority-group-changes-<branch>` and its continuation as
//! `air-superiority-group-state-<branch>`), never published as the groups'.
//!
//! A pre-pass worker reads a block alone, only at the hours a field the
//! groups or air superiority read changes ([`FAMILY`]'s fields: the groups'
//! own, the countries' keys, every link, every score field, each wing's
//! mission region, the theatres' names, the theatre groups' theatres and the
//! theatre rosters' keys), at each day's first hour (where the day's choice
//! is published) and at the next recorded war end (where the groups change
//! with no field changing, as their own extract names it). The worker keeps
//! each observed hour's frame (countries reduced to the keys both read, wings
//! to their mission regions) with the hour's score projection made through
//! the score identity it read there, and for each hour between, whether the
//! theatres were read there (their slot read) and whether a table the day's
//! choice reads was decoded again (the war casualties move every hour of a
//! war without changing the groups).
//!
//! The fold observes each kept hour on the carried state as hourly
//! preparation does (the groups, then air superiority with the worker's
//! projection when the worker's score identity is the carried one), and
//! takes each hour between in constant time: the groups' unchanged-hour step
//! and air superiority's ([`Prepared::quiet_hour`]). At the block's end the
//! groups are sealed and the day's choice is held open, as hourly preparation
//! does at every block's end. When the carried state does not continue into
//! an hour between, the rest of the block is observed hour by hour.
//!
//! A groups refusal refuses air superiority at that hour with the groups'
//! error, named as coming from the groups ([`GroupsRefusal`]).

use super::{
    DAY_HOURS,
    prepared::{CHOICE_TABLES, PairDiff, Prepared, ProjectionMark, ScoreIdentity, Scored},
    scan, scores,
};
use dashboard_base::prepared::kept::{Observed, Reduced, consecutive, kept_changes, observe_hours};
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::prepared::kept::{note_frames, note_observed};
use dashboard_base::{
    Source,
    prepared::{
        frame_changes::FrameChanges,
        prepass::{Extract, Family, Folded, Held, HeldBytes},
    },
    products::{
        LoadContext,
        country_groups::{self, Scan as Groups, changes::Prepared as Changes},
    },
};
use dashboard_text::metadata::CountryKeys;
use replay::{
    container::BlockIndex, container::QuietHours, container::Replay, container::TableFields,
};
use replay_data::{
    format::FormatVersion, rows::AirWingRow, rows::CountryRow, rows::TableId, rows::TableRows,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{
    any::Any,
    collections::BTreeMap,
    fmt, io,
    ops::Range,
    sync::{Arc, atomic::AtomicBool},
};

/// The tables air superiority and its groups read: the groups' own
/// (`country_groups::prepared::TABLES`), the scores, the wings and the
/// theatre rosters.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::HumanControllers,
    TableId::Factions,
    TableId::LexicalTokens,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Capitulation,
    TableId::RaidAirScores,
    TableId::AirWings,
    TableId::AirTheatres,
    TableId::AirTheatreGroups,
    TableId::AirTheatreMembers,
];

pub static FAMILY: Family = Family {
    name: "air_superiority",
    extract: "air_superiority.extract",
    sequential: "air_superiority.sequential",
    fallback: "air_superiority.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The groups' sections air superiority keeps under its own names: its read
/// of the groups, the change publisher's pages and catalogue, and the change
/// publisher's continuation.
pub fn groups_section(branch: u64) -> String {
    format!("air-superiority-groups-{branch}")
}
pub const GROUP_CHANGES: &str = "air-superiority-group-changes";
pub fn state_section(branch: u64) -> String {
    format!("air-superiority-group-state-{branch}")
}

/// The fields air superiority and its groups read, so that a block is read
/// only at the hours they change. Campaign is checked on every read and read
/// by nothing else here.
fn fields() -> io::Result<Vec<TableFields>> {
    // The groups' own fields, as their own family watches them: what decides
    // an hour that changes the groups.
    let groups = country_groups::prepared::FAMILY
        .fields
        .ok_or_else(|| io::Error::other("Country groups declare no fields"))?;
    let mut fields = groups()?;
    fields.extend([
        // Country keys (`CountryKeys`, `tagged_countries_indexed`): the id
        // (the row key), canonical slot and tag. The balances move every
        // hour and neither reads them.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // Every link: the score owners are resolved through them, and a link
        // change reads the score identity again.
        TableFields::all(TableId::CountryLinks),
        // The scores: any row change projects them again. Each pair's
        // movement impact is rewritten with the pair, so watching it adds no
        // hour; scores move every hour of an air war.
        TableFields::all(TableId::RaidAirScores),
        // Wings counted by mission region and joined to theatres by id (the
        // row key). Plane counts move every hour and are not read.
        TableFields::named(TableId::AirWings, &["mission_region"])?,
        // Theatre names, each theatre group's theatre and the rosters' keys
        // (wing and group); owners and ordinals are not read. A theatre read
        // with equal rows only dates the held theatres: each hour's slot read
        // is kept with the quiet hours.
        TableFields::named(TableId::AirTheatres, &["name"])?,
        TableFields::named(TableId::AirTheatreGroups, &["theatre"])?,
        TableFields::keys(TableId::AirTheatreMembers),
    ]);
    Ok(fields)
}

fn start(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract::new(format))
}

/// One hour between observed hours: whether the theatres were read there,
/// and whether a table the day's choice reads was decoded again.
#[derive(Clone, Copy)]
struct Quiet {
    theatres_read: bool,
    choices_changed: bool,
}

/// One hour of a block's facts, in hour order.
#[allow(clippy::large_enum_variant)] // Each observed hour owns its frame.
enum Hour {
    /// An hour the groups and air superiority observe, with its frame, change
    /// facts and the worker's score projection where the scores or their
    /// identity changed.
    Observed {
        frame: DecodedFrame,
        changes: FrameChanges,
        scored: Option<Scored>,
    },
    /// Consecutive recorded hours from `first`, after the hour before them,
    /// at which no field either reads changed, within one day and before the
    /// next war end.
    Quiet { first: i32, hours: Vec<Quiet> },
}

/// A block's hours as the extract kept them.
struct BlockFacts {
    hours: Vec<Hour>,
    /// Frames the extract was given or built.
    #[cfg_attr(not(test), allow(dead_code))]
    frames: usize,
}

type PreviousProjection = (Arc<ProjectionMark>, BTreeMap<(String, u32), (i64, i64)>);

/// The score identity air superiority reads, followed as its observer
/// follows it (`Prepared::observe_scored`), so a worker projects an hour's
/// scores exactly where and as the observer would, and diffs each projection's
/// pairs against its previous one in the block, so the fold observes only the
/// pairs that changed.
#[derive(Default)]
pub(super) struct ScoreTracker {
    keys: Option<CountryKeys>,
    identity: Option<Arc<ScoreIdentity>>,
    /// The previous projection's mark and pairs.
    previous: Option<PreviousProjection>,
}

impl ScoreTracker {
    /// The hour's projection where the scores or the score identity changed;
    /// None at other hours, where the observer projects nothing.
    pub(super) fn scored(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<Option<Scored>> {
        let countries = frame.complete::<tables::Countries>().unwrap_or(&[]);
        let restoring = self.identity.is_none();
        let country_changed = changes.rows_changed[TableId::Countries.index()]
            && self.keys.as_ref().is_none_or(|keys| !keys.same(countries));
        let identity_changed =
            restoring || country_changed || changes.rows_changed[TableId::CountryLinks.index()];
        if identity_changed {
            self.identity = Some(Arc::new(scan::score_countries(frame)?));
            self.keys = Some(CountryKeys::read(countries));
        }
        if !identity_changed && !changes.rows_changed[TableId::RaidAirScores.index()] {
            return Ok(None);
        }
        let identity = Arc::clone(self.identity.as_ref().expect("score identity read"));
        let rows = dashboard_text::metadata::recorded_rows::<tables::RaidAirScores>(frame)?;
        let (operands, owners) = &*identity;
        let projection = scan::score_values(
            scores::ScoreView {
                rows,
                complete: frame.complete::<tables::RaidAirScores>().is_ok(),
                owners: None,
                active: None,
            },
            operands.clone(),
            owners,
        );
        let mark = Arc::new(ProjectionMark);
        let diff = self.previous.take().map(|(after, before)| PairDiff {
            after,
            changed: projection
                .pairs
                .iter()
                .filter(|(pair, scores)| before.get(*pair) != Some(*scores))
                .map(|(pair, &scores)| (pair.clone(), scores))
                .collect(),
            removed: before
                .into_keys()
                .filter(|pair| !projection.pairs.contains_key(pair))
                .collect(),
        });
        self.previous = Some((Arc::clone(&mark), projection.pairs.clone()));
        Ok(Some(Scored {
            identity,
            projection,
            mark,
            diff,
        }))
    }
}

/// The groups' next recorded war end after `frame`'s hour, as their scan
/// finds its next expiry from the hour's war operands: a war whose recorded
/// end is after its start. None while the war table is not complete.
fn next_war_end(frame: &DecodedFrame) -> Option<i32> {
    frame
        .complete::<tables::WarRelations>()
        .ok()?
        .iter()
        .filter(|row| row.end_hour > row.start_hour && row.end_hour > frame.hour)
        .map(|row| row.end_hour)
        .min()
}

struct BlockExtract {
    format: FormatVersion,
    hours: Vec<Hour>,
    countries: Reduced<CountryRow>,
    wings: Reduced<AirWingRow>,
    scores: ScoreTracker,
    /// The last hour observed.
    last: Option<i32>,
    /// The groups' next war end after it.
    war_end: Option<i32>,
    frames: usize,
}

impl BlockExtract {
    fn new(format: FormatVersion) -> Self {
        Self {
            format,
            hours: Vec::new(),
            countries: Reduced::new(TableId::Countries),
            wings: Reduced::new(TableId::AirWings),
            scores: ScoreTracker::default(),
            last: None,
            war_end: None,
            frames: 0,
        }
    }
}

impl Held for scan::ScoreOperands {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.present)
            .heap(&self.witnessed)
            .heap(&self.invalid);
    }
}

impl Held for scan::CountryScores {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.operands)
            .heap(&self.pairs)
            .heap(&self.active);
    }
}

impl Held for ProjectionMark {
    fn held(&self, _: &mut HeldBytes) {}
}

impl Held for PairDiff {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.after)
            .heap(&self.changed)
            .heap(&self.removed);
    }
}

/// A worker projection: its own pairs, sets and diff, and the score identity
/// and marks it shares with the block's other projections, once.
impl Held for Scored {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.identity)
            .heap(&self.projection)
            .heap(&self.mark)
            .heap(&self.diff);
    }
}

impl Extract for BlockExtract {
    /// What the block's facts hold: each kept frame's views once, its change
    /// facts and score projection, and the hours kept between them.
    fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            match hour {
                Hour::Observed {
                    frame,
                    changes,
                    scored,
                } => {
                    held.frame(frame).changes(changes).heap(scored);
                }
                Hour::Quiet { hours, .. } => {
                    held.add(hours.capacity() * size_of::<Quiet>());
                }
            }
        }
        held.total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.frames += 1;
        let scored = self.scores.scored(frame, changes)?;
        self.last = Some(frame.hour);
        self.war_end = next_war_end(frame);
        let mut kept = frame.clone();
        self.countries.countries(&mut kept);
        let wings = frame.complete::<tables::AirWings>().ok().map(|rows| {
            rows.iter()
                .map(|wing| AirWingRow {
                    id: wing.id,
                    mission_region: wing.mission_region,
                    ..Default::default()
                })
                .collect()
        });
        self.wings.apply(&mut kept, wings, TableRows::AirWings);
        self.hours.push(Hour::Observed {
            frame: kept,
            changes: kept_changes(changes),
            scored,
        });
        Ok(())
    }

    /// The next day's first hour, where the day's choice is published, or
    /// the groups' next war end, if sooner: hours that change what the fold
    /// does with no field changing.
    fn deadline(&self) -> Option<i32> {
        let day = self
            .last?
            .div_euclid(DAY_HOURS)
            .checked_add(1)
            .and_then(|day| day.checked_mul(DAY_HOURS));
        match (day, self.war_end) {
            (Some(day), Some(end)) => Some(day.min(end)),
            (day, end) => day.or(end),
        }
    }

    /// Hours at which no field either reads changed: each hour's slot read
    /// of the theatres, and whether the day's choice tables were decoded
    /// again, as an observed hour's change facts carry them.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let Some((first, _)) = consecutive(hours, span.clone())? else {
            return Ok(());
        };
        let format = self.format;
        let quiet = span.map(|position| Quiet {
            theatres_read: dashboard_text::metadata::slot_read(
                format,
                &hours.metrics()[position],
                TableId::AirTheatres,
                hours.hours()[position],
            ),
            choices_changed: CHOICE_TABLES
                .iter()
                .any(|&table| hours.rebuilt(position, table)),
        });
        match self.hours.last_mut() {
            Some(Hour::Quiet {
                first: held,
                hours: held_hours,
            }) if i64::from(*held) + held_hours.len() as i64 == i64::from(first) => {
                held_hours.extend(quiet);
            }
            _ => self.hours.push(Hour::Quiet {
                first,
                hours: quiet.collect(),
            }),
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Air superiority block has no hours"));
        }
        Ok(Box::new(BlockFacts {
            hours: self.hours,
            frames: self.frames,
        }))
    }
}

/// A country groups refusal reaching air superiority: the groups' own error,
/// at the hour the groups refused, recorded as coming from the groups
/// ([`refused_from`]), as hourly preparation records a refusal reaching a
/// family that reads another family's state.
#[derive(Debug)]
pub struct GroupsRefusal(io::Error);

impl GroupsRefusal {
    /// The family the refusal comes from.
    pub const FROM: &'static str = "groups";
}

impl fmt::Display for GroupsRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for GroupsRefusal {}

/// A groups outcome as air superiority's: a groups refusal refuses it
/// ([`GroupsRefusal`]); any other error is unchanged.
pub fn from_groups(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::InvalidData {
        io::Error::new(io::ErrorKind::InvalidData, GroupsRefusal(error))
    } else {
        error
    }
}

/// The family a refusal came from: the groups, for a groups refusal reaching
/// air superiority; None for a family's own refusal.
pub fn refused_from(error: &io::Error) -> Option<&'static str> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<GroupsRefusal>())
        .map(|_| GroupsRefusal::FROM)
}

/// What becomes of each groups observation and flush of the family's own
/// read, at its hour: the outcome itself, or (in tests) a refusal injected
/// for the groups at that hour.
pub type GroupsOutcome<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<()> + 'a;

/// One branch's air superiority and its own read of the country groups.
pub struct OwnBranch {
    prepared: Box<Prepared>,
    groups: Box<Groups>,
    changes: Box<Changes>,
}

impl OwnBranch {
    pub fn new(prepared: Prepared, groups: Groups, changes: Changes) -> Self {
        Self {
            prepared: Box::new(prepared),
            groups: Box::new(groups),
            changes: Box::new(changes),
        }
    }

    /// Before each block, as hourly preparation begins the groups' block.
    pub fn begin_block(&mut self) {
        self.groups.begin_block();
    }

    /// The groups' observation of `frame`, then air superiority's, as hourly
    /// preparation observes them one after the other; whether air
    /// superiority goes on.
    #[allow(clippy::too_many_arguments)]
    fn observe(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        scored: Option<Scored>,
        replay: &Replay,
        source: &Source,
        context: &LoadContext,
        groups_outcome: &mut GroupsOutcome<'_>,
        observed: &mut Observed<'_>,
    ) -> io::Result<bool> {
        let started = self.prepared.clock();
        let groups = groups_outcome(
            frame.hour,
            self.changes
                .observe_changed(&mut self.groups, frame, changes, source, context),
        );
        self.prepared.lap(started, "air_superiority.fold.groups");
        let result = match groups {
            Ok(()) => {
                self.prepared
                    .observe_scored(frame, replay, &mut self.groups, changes, scored)
            }
            Err(error) => Err(from_groups(error)),
        };
        observed(frame.hour, result)
    }

    /// The block's end, as hourly preparation ends every block it read: the
    /// groups sealed and the day's choice held open at `hour`.
    fn end_of_block(
        &mut self,
        replay: &Replay,
        branch: u64,
        hour: i32,
        observed: &mut Observed<'_>,
    ) -> io::Result<bool> {
        self.groups.finish(replay, branch);
        let result = self.prepared.publish_day(&self.groups, true);
        observed(hour, result)
    }

    /// Fold one block's facts into the carried state, after the block began.
    /// Returns how it folded and the last hour it reached.
    #[allow(clippy::too_many_arguments)]
    pub fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        replay: &Replay,
        index: &BlockIndex,
        source: &Source,
        context: &LoadContext,
        groups_outcome: &mut GroupsOutcome<'_>,
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
        note_frames(FAMILY.name, facts.frames);
        let mut reached = None;
        for hour in facts.hours {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            check()?;
            match hour {
                Hour::Observed {
                    frame,
                    changes,
                    scored,
                } => {
                    reached = Some(frame.hour);
                    #[cfg(any(test, feature = "test-support"))]
                    note_observed(FAMILY.name);
                    let live = {
                        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                        self.observe(
                            &frame,
                            &changes,
                            scored,
                            replay,
                            source,
                            context,
                            groups_outcome,
                            observed,
                        )?
                    };
                    if !live {
                        return Ok((Folded::Fast, reached));
                    }
                }
                Hour::Quiet { first, hours } => {
                    for (offset, quiet) in hours.into_iter().enumerate() {
                        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
                        // The span's hours are consecutive i32 hours.
                        let hour = first + offset as i32;
                        if !self.prepared.quiet_follows(hour) {
                            let reached = self.observe_block(
                                replay,
                                index,
                                source,
                                context,
                                Some(hour),
                                groups_outcome,
                                observed,
                                check,
                            )?;
                            return Ok((Folded::Fallback, reached));
                        }
                        reached = Some(hour);
                        let result = {
                            let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                            let started = self.prepared.clock();
                            let groups = groups_outcome(
                                hour,
                                self.changes.advance_quiet(&mut self.groups, hour, hour),
                            );
                            let result = match groups {
                                Ok(()) => self.prepared.quiet_hour(
                                    hour,
                                    quiet.theatres_read,
                                    quiet.choices_changed,
                                ),
                                Err(error) => Err(from_groups(error)),
                            };
                            self.prepared.lap(started, "air_superiority.fold.quiet");
                            result
                        };
                        if !observed(hour, result)? {
                            return Ok((Folded::Fast, reached));
                        }
                    }
                }
            }
        }
        self.end_of_block(
            replay,
            index.branch,
            reached.unwrap_or(index.last_hour),
            observed,
        )?;
        Ok((Folded::Fast, reached))
    }

    /// Observe the block hour by hour from `from` (every hour without it),
    /// with the change facts of the block alone, as hourly preparation does,
    /// then end it while air superiority goes on. Returns the last hour
    /// observed.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_block(
        &mut self,
        replay: &Replay,
        index: &BlockIndex,
        source: &Source,
        context: &LoadContext,
        from: Option<i32>,
        groups_outcome: &mut GroupsOutcome<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        let mut live = true;
        let reached = {
            let mut followed = |hour: i32, result: io::Result<()>| -> io::Result<bool> {
                let open = observed(hour, result)?;
                live = open;
                Ok(open)
            };
            let Self {
                prepared,
                groups,
                changes,
            } = self;
            observe_hours(
                replay,
                index,
                TABLES,
                from,
                &mut followed,
                check,
                |frame, facts| {
                    #[cfg(any(test, feature = "test-support"))]
                    note_observed(FAMILY.name);
                    let outcome = groups_outcome(
                        frame.hour,
                        changes.observe_changed(groups, frame, facts, source, context),
                    );
                    match outcome {
                        Ok(()) => prepared.observe_changed(frame, replay, groups, facts),
                        Err(error) => Err(from_groups(error)),
                    }
                },
            )?
        };
        if live {
            self.end_of_block(
                replay,
                index.branch,
                reached.unwrap_or(index.last_hour),
                observed,
            )?;
        }
        Ok(reached)
    }

    /// Measure the parts of each fold from now on ([`Self::take_parts`]).
    pub fn measure(&mut self) {
        self.prepared.measure();
    }

    /// The time each part of the folds took since last asked, by part name,
    /// with the times it took it; none unless measured.
    pub fn take_parts(&mut self) -> Vec<(&'static str, std::time::Duration, u64)> {
        self.prepared.take_parts()
    }

    /// Bytes held for the next flush.
    pub fn pending(&self) -> usize {
        self.prepared.bytes() + self.changes.bytes()
    }

    /// The batch's flush, in hourly preparation's order: air superiority's
    /// pages, then its groups sealed and their change publisher's pages, whose
    /// refusal is a groups refusal.
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.prepared.flush(writer, branch, cancelled)?;
        self.groups.finish(replay, branch);
        self.changes
            .flush_as(writer, GROUP_CHANGES, branch, &self.groups, cancelled)
            .map_err(from_groups)
    }

    /// Its read of the groups, written after the flush as hourly preparation
    /// writes the groups': the outer error stops preparation, the inner one
    /// is a groups refusal.
    pub fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        let bytes = self.groups.saved_bytes()?;
        Ok(writer
            .write_at(&groups_section(branch), 0, bytes.as_slice())
            .map_err(from_groups))
    }

    /// The change publisher's continuation, which hourly preparation keeps in
    /// its branch state.
    pub fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        writer.write_at(
            &state_section(branch),
            0,
            serde_json::to_vec(&*self.changes)?.as_slice(),
        )
    }
}

#[cfg(test)]
mod bytes_tests {
    use super::*;
    use dashboard_base::{
        prepared::{frame_changes::FrameChangeTracker, prepass::HeldBytes},
        test_fixtures::{CAMPAIGN, records, source},
    };
    use replay_data::rows::RaidAirScoreRow;
    use replay_frame::frame::TableView;

    /// What a block's extract reports, and what its kept frames and change
    /// facts alone hold, when `regions` scores move at each of `hours` hours.
    fn reported(regions: u32, hours: i32) -> (u64, u64) {
        let frames: Vec<DecodedFrame> = (100..100 + hours)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                *frame.tables[TableId::RaidAirScores.index()] =
                    TableView::Complete(TableRows::RaidAirScores(
                        (0..regions)
                            .map(|region| RaidAirScoreRow {
                                country: 9001,
                                region,
                                friendly: i64::from(hour) * 1_000 + i64::from(region),
                                enemy: 1,
                                ..Default::default()
                            })
                            .collect(),
                    ));
                frame
            })
            .collect();
        // The recording the extract is given; it reads only the frames.
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 100, Some(1))])],
        )
        .unwrap();
        let mut tracker = FrameChangeTracker::new();
        let mut extract = BlockExtract::new(replay.format);
        for frame in &frames {
            let changes = tracker.observe(frame, replay.format);
            extract.observe(&replay, frame, &changes).unwrap();
        }
        let mut frames_alone = HeldBytes::default();
        frames_alone.add(extract.hours.capacity() * size_of::<Hour>());
        for hour in &extract.hours {
            if let Hour::Observed {
                frame,
                changes,
                scored,
            } = hour
            {
                assert!(scored.is_some(), "every hour's scores moved");
                frames_alone.frame(frame).changes(changes);
            }
        }
        (extract.bytes(), frames_alone.total())
    }

    /// Every hour's projection is kept beside its frame (its pairs and their
    /// diff against the hour before), so a block of K moving pairs at each of
    /// H hours holds more than its frames by at least two pair entries per
    /// pair and hour.
    #[test]
    fn extract_bytes_grow_with_pairs_times_hours() {
        let beyond = |regions, hours| {
            let (reported, frames) = reported(regions, hours);
            assert!(reported >= frames);
            reported - frames
        };
        let pair = 2 * size_of::<((String, u32), (i64, i64))>() as u64;
        let (small, wide, long, both) =
            (beyond(8, 4), beyond(256, 4), beyond(8, 64), beyond(256, 64));
        assert!(wide - small >= (256 - 8) * 4 * pair, "{small} {wide}");
        assert!(long - small >= 8 * (64 - 4) * pair, "{small} {long}");
        assert!(both - wide >= 256 * (64 - 4) * pair, "{wide} {both}");
        assert!(both - long >= (256 - 8) * 64 * pair, "{long} {both}");
    }
}
