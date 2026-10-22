//! The politics ledger prepared on its own: political and command power
//! balances and spending by country.
//!
//! Each captured country's interval extends to every hour it is read, so no
//! hour is quiet for the ledger's observer. An hour at which the captures only
//! extend, and nothing else the observer joins changes, takes its advance
//! path: the captures' last hours and expiries move in place, and the
//! accounting, which would select no country there, visits none. Every other
//! hour takes the whole accounting. The only evidence that a capture extended
//! rather than stopped is its last hour, the field that moves, so an hour
//! cannot be read as quiet by its changes (`Replay::read_block_changes`)
//! without its rows; and the advance path, like the joins it follows, reads
//! the hour's changed rows, which a block read by its changes does not keep.
//! Its carried state (published event keys, tag history, open days, coverage
//! and hold intervals) decides what each hour adds, so a block's observation
//! cannot run apart from the block before. The family's own thread therefore
//! observes each block in order, as the hourly walk observes it: on the block
//! read alone with the tables the observer reads (its native tables, the
//! census, the naming tables, and the shared trigger kit an item's text
//! evaluates at its event hour), with the change facts and recorded updates of
//! the block alone, which are the walk's at every hour (every block starts with
//! no change facts of the block before).
//!
//! What a block's join does follows from the join before it and the block's
//! tables alone, so a pre-pass worker joins each block on its own
//! (`prepared::BlockFacts`, on the join's tables only): it keeps the join the
//! block's first hour leaves and certifies each later hour at which the
//! hour's changed captures only extend and nothing else joins a country. The
//! fold observes the block's first hour; when the join it leaves is the
//! worker's, every later hour is the worker's join too, and a certified hour
//! that only advances captures for the observer (no row it has not yet
//! recorded is observed again, no capture is due, no day boundary, nothing
//! dirty) takes the advance path without visiting its captures: each one's
//! expiry moves, and its witness reaches the last hour it extended to once,
//! before any other join reads it. Every other hour is observed as the walk
//! observes it. At an hour that presents many countries, they are presented
//! on several threads, each as presenting it in turn would make it.
//!
//! Hourly preparation binds the ledger's content and the recording at every
//! block and, where it resumes a branch (at a run's start and wherever it loads
//! the branch's saved state again), resumes the ledger from the frame its
//! stream holds: the last hour of the recording's block before, when that block
//! is on the same branch. The family does the same at the same blocks, before
//! binding, as the walk does. It saves its state where hourly preparation does
//! (`politics-ledger-state-<branch>`), after its flush.

use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::{
        frame_changes::FrameChangeTracker,
        prepass::{self, Folded},
    },
    products::{politics_content::PoliticsContent, politics_ledger::prepared},
};
use replay::{container::Replay, container::SeekResult};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use std::{
    any::Any,
    io,
    sync::{Arc, LazyLock, atomic::AtomicBool},
};

/// The ledger's family: its refusal, progress and coverage name is the hourly
/// walk's.
static FAMILY: prepass::Family = prepass::Family {
    name: "politics_ledger",
    extract: "politics_ledger.extract",
    sequential: "politics_ledger.sequential",
    fallback: "politics_ledger.fallback",
    // The extract joins the block; the observation reads its own tables.
    tables: prepared::BLOCK_TABLES,
    fields: None,
    start: prepared::block_extract,
};

/// Each hour the family's own thread observes, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const OBSERVED: &str = "politics_ledger.observed";

/// Each of those hours that took the observer's advance path, without the
/// whole accounting, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const ADVANCED: &str = "politics_ledger.advanced";

/// Each of those advance hours a pre-pass worker's certificate advanced
/// without the fold joining its captures, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const CERTIFIED: &str = "politics_ledger.certified";

/// Each observed hour whose countries were presented on several threads,
/// counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const PARALLEL: &str = "politics_ledger.parallel";

/// The tables the ledger's observer reads: its native tables, the census and
/// capture coverage, the holders' and items' naming tables
/// (`installed_content_country_names::country_names::TABLES` and the government, war and dictionary
/// operands of a country's look), and the shared trigger kit an item's text
/// evaluates at its event hour (`triggers::RECORDED_TABLES`).
fn tables() -> &'static [TableId] {
    static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = vec![
            TableId::Campaign,
            TableId::CountryPowerDaily,
            TableId::CountryPowerChanges,
            TableId::CountryPowerDailyCosts,
            TableId::GovernmentChanges,
            TableId::PoliticsCaptureCoverage,
            TableId::CharacterPortraits,
            TableId::AdvisorRoles,
            TableId::Characters,
            TableId::UnitLeaders,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::LexicalTokens,
            TableId::ModifierDefinitions,
            TableId::CountryAppearance,
            TableId::Politics,
            TableId::PoliticalParties,
            TableId::Autonomy,
            TableId::CountryCivilWars,
            TableId::CountryCounterInputs,
            TableId::CountryWars,
            TableId::WarRelations,
            TableId::CountryCommandPower,
            TableId::IndustrialOrganisations,
            TableId::States,
        ];
        tables.extend(installed_content_country_names::country_names::TABLES);
        tables.extend_from_slice(crate::triggers::RECORDED_TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &TABLES
}

/// The section hourly preparation saves a branch's ledger state in.
fn state_section(branch: u64) -> String {
    format!("politics-ledger-state-{branch}")
}

/// The politics ledger.
pub(in crate::prepared::pipeline) struct Ledger;

pub(in crate::prepared::pipeline) static LEDGER: Ledger = Ledger;

impl Own for Ledger {
    fn family(&self) -> &'static prepass::Family {
        &FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.politics_ledger"
    }

    fn shared_tables(&self, _: &super::Phase<'_>) -> &'static [TableId] {
        tables()
    }

    fn gates(&self) -> &'static [&'static str] {
        &["politics-ledger"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let ledger = match load.read(&state_section(branch))? {
            Some(file) => prepared::Prepared::read(io::BufReader::new(file))?,
            None => prepared::Prepared::default(),
        };
        Ok(Box::new(LedgerBranch {
            ledger: Box::new(ledger),
            content: None,
            resume: true,
        }))
    }
}

/// One branch's ledger.
struct LedgerBranch {
    ledger: Box<prepared::Prepared>,
    /// The ledger content, read once for this state as hourly preparation
    /// reads it once per run; its error is the warning the ledger keeps.
    content: Option<Result<Option<Arc<PoliticsContent>>, String>>,
    /// Whether the state was just loaded: hourly preparation resumes a
    /// branch's ledger wherever it loads the branch.
    resume: bool,
}

impl LedgerBranch {
    /// Before a block's hours: resume a just-loaded state as the hourly walk
    /// resumes it, then bind the content and the recording, as the walk does
    /// at every block.
    fn prepare(&mut self, block: &Block<'_>) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        if std::mem::take(&mut self.resume)
            && let Some(frame) = before(block)?
        {
            self.ledger.resume(&frame);
        }
        let content = self.content.get_or_insert_with(|| {
            prepared::prepared_content(block.source, block.context.content.as_ref())
        });
        self.ledger.configure(content);
        self.ledger.decode_previous_from(block.source.replay());
        Ok(())
    }

    /// Observe the block in order, as hourly preparation does, with the
    /// change facts of the block alone and, from a pre-pass worker, its
    /// join's `facts`. The last hour observed.
    fn observe_hours(
        &mut self,
        block: &Block<'_>,
        facts: Option<prepared::BlockFacts>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        self.prepare(block)?;
        let mut fold = facts.map(prepared::BlockFold::new);
        let format = block.replay.format;
        let mut tracker = FrameChangeTracker::new();
        // Cancellation is checked between hours; an hour's observation runs
        // whole, so its refusals are the walk's.
        let running = AtomicBool::new(false);
        let ledger = &mut self.ledger;
        let mut reached = None;
        let mut stopped = false;
        let mut failure = None;
        let read = block.read_alone(
            tables(),
            |raw, frame| {
                let _cpu = ::replay_frame::cpu::scope("fold driver", None);
                let changes = tracker.observe_updates(frame, raw, format);
                if stopped {
                    return Ok(());
                }
                reached = Some(frame.hour);
                #[cfg(test)]
                prepass::note(OBSERVED);
                #[cfg(test)]
                let counts = (
                    ledger.advanced_hours(),
                    ledger.certified_hours(),
                    ledger.parallel_hours(),
                );
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    match fold.as_mut() {
                        Some(fold) => ledger.observe_folded(frame, &changes, &running, fold),
                        None => ledger.observe(frame, &changes, &running),
                    }
                };
                #[cfg(test)]
                {
                    for (before, after, name) in [
                        (counts.0, ledger.advanced_hours(), ADVANCED),
                        (counts.1, ledger.certified_hours(), CERTIFIED),
                        (counts.2, ledger.parallel_hours(), PARALLEL),
                    ] {
                        if after > before {
                            prepass::note(name);
                        }
                    }
                }
                match observed(frame.hour, result) {
                    Ok(live) => {
                        stopped = !live;
                        Ok(())
                    }
                    Err(error) => {
                        failure = Some(error);
                        Err(io::Error::other("Politics ledger observation stopped"))
                    }
                }
            },
            check,
        );
        ledger.end_block();
        if let Some(error) = failure {
            return Err(error);
        }
        read?;
        Ok(reached)
    }
}

/// The frame hourly preparation's stream holds when it resumes `block`'s
/// branch at `block`: the last hour of the recording's block before, with
/// every table, when that block is on the same branch; none otherwise.
fn before(block: &Block<'_>) -> io::Result<Option<DecodedFrame>> {
    let index = &block.replay.index;
    let Some(position) = index
        .iter()
        .position(|at| at.branch == block.index.branch && at.first_hour == block.index.first_hour)
    else {
        return Ok(None);
    };
    let Some(previous) = position
        .checked_sub(1)
        .map(|position| &index[position])
        .filter(|previous| previous.branch == block.index.branch)
    else {
        return Ok(None);
    };
    Ok(
        match block.replay.seek(previous.branch, previous.last_hour)? {
            SeekResult::Frame(frame) => Some(*frame),
            SeekResult::Gap { .. } => None,
        },
    )
}

impl Branch for LedgerBranch {
    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        let facts = {
            let _cpu = ::replay_frame::cpu::scope("fold facts", None);
            facts
                .downcast::<prepared::BlockFacts>()
                .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
        };
        let reached = self.observe_hours(block, Some(*facts), observed, check)?;
        Ok((Folded::Fast, reached))
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        self.observe_hours(block, None, observed, check)
    }

    fn pending(&self) -> usize {
        self.ledger.bytes()
    }

    /// The ledger's pages, where hourly preparation's flush writes them.
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.ledger.flush(writer, branch, cancelled)
    }

    /// Its state, which hourly preparation's flush writes after the pages:
    /// failing to encode it stops preparation, failing to write it refuses.
    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        let bytes = self.ledger.saved_bytes()?;
        Ok(writer.write_at(&state_section(branch), 0, bytes.as_slice()))
    }

    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}
