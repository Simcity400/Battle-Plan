//! The event feed prepared on its own, with its own read of the country
//! identity history. Hourly preparation observes the identity history (in
//! territory's owned-state observation) and then the feed at every hour, and
//! the feed names its countries through the history's state of that hour, so
//! this family keeps an identity history of its own and makes the history's
//! call on each frame before its own. That history is saved under the feed's
//! name (`event-feed-identities-<branch>`), never published as the
//! identities'.
//!
//! The feed reads its event pack at every hour (the pack carries its read
//! hour), so every hour is observed: a pre-pass worker reads each block whole
//! with the feed's and the history's tables and keeps every hour's frame,
//! reduced to the fields the feed and the history read
//! ([`KeptFrames`]), with its change facts. The family's own thread observes
//! them in order on the carried history and feed, as hourly preparation
//! observes them. The worker's read and decoding leave the family's thread.
//!
//! A refusal of the history's read refuses the feed at that hour with the
//! history's error, named as coming from the identities
//! ([`IdentitiesRefusal`]), as hourly preparation records an identities
//! refusal reaching the feed.
//!
//! The feed's continuation, which hourly preparation keeps in its branch
//! state, is saved as `event_feed-continuation-<branch>`; its facts series
//! and catalogue are published where hourly preparation flushes them. Its
//! query reads the pages of other families too (`independent::JOINT`).

use super::{
    Block, Branch, Load, Observed, Own,
    census::{continuation, read, save},
};
use crate::{
    prepared::{
        frame_changes::FrameChanges,
        prepass::{self, Extract, Folded},
    },
    products::{
        country_history::HistoryScan,
        event_feed::prepared::{KeptFrames, Observer},
    },
};
use dashboard_base::prepared::kept::{kept_changes, observe_hours};
use replay::container::Replay;
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::frame::DecodedFrame;
use std::{any::Any, fmt, io, sync::atomic::AtomicBool};

/// The tables the feed and its identity history read: the history's own
/// (`country_history::prepared::TABLES`), the relations and slice reads of
/// the diplomacy observer, the event packs and the nuclear strikes.
const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryCounterInputs,
    TableId::CountryExperienceGains,
    TableId::RelationTuples,
    TableId::SliceReads,
    TableId::EventFeedHistory,
    TableId::NuclearStrikes,
];

static FAMILY: prepass::Family = prepass::Family {
    name: "event_feed",
    extract: "event_feed.extract",
    sequential: "event_feed.sequential",
    fallback: "event_feed.fallback",
    tables: TABLES,
    // Every hour reads an event pack of its own, so every hour is observed.
    fields: None,
    start,
};

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(FeedExtract {
        frames: KeptFrames::default(),
        hours: Vec::new(),
    })
}

/// A block's frames, every hour, as the feed and the history read them.
struct FeedExtract {
    frames: KeptFrames,
    hours: Vec<(DecodedFrame, FrameChanges)>,
}

/// A block's kept hours.
struct Facts {
    hours: Vec<(DecodedFrame, FrameChanges)>,
}

impl Extract for FeedExtract {
    /// What the block's kept hours hold: each frame's views once (kept
    /// frames share their unchanged views) and its change facts.
    fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<(DecodedFrame, FrameChanges)>());
        for (frame, changes) in &self.hours {
            held.frame(frame).changes(changes);
        }
        held.total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let kept = self.frames.kept(frame);
        self.hours.push((kept, kept_changes(changes)));
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Event feed block has no hours"));
        }
        Ok(Box::new(Facts { hours: self.hours }))
    }
}

/// The history's section of one branch, which is also its continuation.
fn identities_section(branch: u64) -> String {
    format!("event-feed-identities-{branch}")
}

/// A refusal of the feed's own read of the identity history: the
/// identities' error, recorded for the feed as coming from them
/// ([`refused_from`]), as hourly preparation records an identities refusal
/// reaching the feed.
#[derive(Debug)]
pub(in crate::prepared::pipeline) struct IdentitiesRefusal(io::Error);

impl IdentitiesRefusal {
    const FROM: &'static str = "identities";

    /// The history's refusal as the feed's; any other error is unchanged.
    fn wrap(error: io::Error) -> io::Error {
        if error.kind() == io::ErrorKind::InvalidData {
            io::Error::new(io::ErrorKind::InvalidData, Self(error))
        } else {
            error
        }
    }
}

impl fmt::Display for IdentitiesRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for IdentitiesRefusal {}

/// The family a refusal came from, when it is a refusal of the feed's own
/// read of the identities ([`IdentitiesRefusal`]).
pub(in crate::prepared::pipeline) fn refused_from(error: &io::Error) -> Option<&'static str> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<IdentitiesRefusal>())
        .map(|_| IdentitiesRefusal::FROM)
}

/// The outcome of the history's observation at `hour`, with the refusals a
/// test injects for the identities there, as the hourly walk's identities
/// observation finds them. The history itself never refuses.
fn identities_outcome(hour: i32) -> io::Result<()> {
    #[cfg(test)]
    let result = super::super::lanes::inject("identities", hour, Ok(()));
    #[cfg(not(test))]
    let result = {
        let _ = hour;
        Ok(())
    };
    result.map_err(IdentitiesRefusal::wrap)
}

/// The feed's cancellation is checked between hours (the fold's `check`), so
/// an hour's observation runs whole, never cancelled inside.
static WHOLE_HOUR: AtomicBool = AtomicBool::new(false);

/// One branch's identity history and feed.
struct FeedBranch {
    identities: Box<HistoryScan>,
    feed: Box<Observer>,
}

impl FeedBranch {
    /// One hour, as hourly preparation observes it: the history, then the
    /// feed naming its countries through it.
    fn observe(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.identities
            .observe_owned_states(frame, Some(changes), |_, _, _| {});
        identities_outcome(frame.hour)?;
        self.feed
            .observe(replay, frame, &self.identities, &WHOLE_HOUR)
    }
}

impl Branch for FeedBranch {
    fn begin_block(&mut self) {}

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
                .downcast::<Facts>()
                .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
        };
        #[cfg(test)]
        dashboard_base::prepared::kept::note_frames(FAMILY.name, facts.hours.len());
        let mut reached = None;
        for (frame, changes) in facts.hours {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            check()?;
            reached = Some(frame.hour);
            #[cfg(test)]
            dashboard_base::prepared::kept::note_observed(FAMILY.name);
            let result = {
                let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                self.observe(block.replay, &frame, &changes)
            };
            if !observed(frame.hour, result)? {
                break;
            }
        }
        Ok((Folded::Fast, reached))
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        let replay = block.replay;
        observe_hours(
            replay,
            block.index,
            TABLES,
            None,
            observed,
            check,
            |frame, changes| {
                #[cfg(test)]
                dashboard_base::prepared::kept::note_observed(FAMILY.name);
                self.observe(replay, frame, changes)
            },
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn pending(&self) -> usize {
        self.feed.bytes()
    }

    /// The history sealed and written whole, as hourly preparation's flush
    /// writes its read, then the feed's facts.
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.identities.seal();
        writer.write_at(
            &identities_section(branch),
            0,
            serde_json::to_vec(&*self.identities)?.as_slice(),
        )?;
        self.feed.flush(writer, branch, cancelled)
    }

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        save(writer, &continuation(FAMILY.name, branch), &*self.feed)
    }
}

/// The event feed: native actions, guarantees, existence transitions and
/// nuclear strikes, with its own read of the identity history.
pub(in crate::prepared::pipeline) struct Feed;

pub(in crate::prepared::pipeline) static FEED: Feed = Feed;

impl Own for Feed {
    fn family(&self) -> &'static prepass::Family {
        &FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.event_feed"
    }

    /// Its query waits for the families whose pages it reads too
    /// (`independent::JOINT`).
    fn gates(&self) -> &'static [&'static str] {
        &[]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let feed: Observer = read(load, &continuation(FAMILY.name, branch))?.unwrap_or_default();
        let identities = match load.read(&identities_section(branch))? {
            Some(file) => serde_json::from_reader(io::BufReader::new(file))?,
            None => HistoryScan::new(false, &[]),
        };
        Ok(Box::new(FeedBranch {
            identities: Box::new(identities),
            feed: Box::new(feed),
        }))
    }
}
