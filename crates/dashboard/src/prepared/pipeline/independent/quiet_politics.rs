//! The quiet politics families prepared on their own: subjects, peace
//! outcomes and idea history. Each reads its blocks by the fields its
//! observer reads, folds them on its carried state and saves the
//! continuation hourly preparation keeps in its branch state under its own
//! name. No other family reads their state in the same hour, so the hourly
//! walk skips them.

use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::{
        prepass::{self, Folded},
        series::{self, Accumulator, LateIndex},
    },
    products::{
        idea_history::{self, PreparedChanges},
        peace_deals::prepared::{self as peace, Prepared as Peace},
        politics_content::{PoliticsContent, politics_content},
        subjects,
    },
};
use replay::container::Replay;
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// Subject relations and their levels.
pub(in crate::prepared::pipeline) struct Subjects;
pub(in crate::prepared::pipeline) static SUBJECTS: Subjects = Subjects;

/// Peace conferences, settlements and their outcomes.
pub(in crate::prepared::pipeline) struct PeaceOutcomes;
pub(in crate::prepared::pipeline) static PEACE: PeaceOutcomes = PeaceOutcomes;

/// Ideas and advisors.
pub(in crate::prepared::pipeline) struct Ideas;
pub(in crate::prepared::pipeline) static IDEAS: Ideas = Ideas;

/// The subject change publisher's continuation, which hourly preparation
/// keeps in its branch state under these names.
pub(in crate::prepared::pipeline) fn subject_state(branch: u64) -> String {
    format!("subject-state-{branch}")
}

/// The peace observer's continuation, which hourly preparation keeps in its
/// branch state as `peace`.
pub(in crate::prepared::pipeline) fn peace_state(branch: u64) -> String {
    format!("peace-state-{branch}")
}

/// The idea change publisher's continuation, which hourly preparation keeps
/// in its branch state as `idea_changes`.
pub(in crate::prepared::pipeline) fn idea_state(branch: u64) -> String {
    format!("idea-state-{branch}")
}

#[derive(Default, Serialize, Deserialize)]
struct SubjectState {
    subject_changes: Accumulator,
    subject_late: LateIndex,
}

impl Own for Subjects {
    fn family(&self) -> &'static prepass::Family {
        &subjects::prepared::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.subjects"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["subjects"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let Some(file) = load.read(&format!("subjects-{branch}"))? else {
            return Ok(Box::new(SubjectsBranch {
                scan: Box::new(subjects::Scan::new(load.replay, branch)),
                state: Box::default(),
            }));
        };
        let scan: subjects::Scan = serde_json::from_reader(zstd::stream::Decoder::new(file)?)?;
        let file = load
            .read(&subject_state(branch))?
            .ok_or_else(|| super::super::invalid("Prepared subject changes are missing"))?;
        let state: Box<SubjectState> = serde_json::from_reader(io::BufReader::new(file))?;
        Ok(Box::new(SubjectsBranch {
            scan: Box::new(scan),
            state,
        }))
    }
}

/// One branch's subject scan and change publisher.
struct SubjectsBranch {
    scan: Box<subjects::Scan>,
    state: Box<SubjectState>,
}

impl Branch for SubjectsBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let SubjectState {
            subject_changes,
            subject_late,
        } = &mut *self.state;
        subjects::prepared::fold(
            &mut self.scan,
            subject_changes,
            subject_late,
            facts,
            block.replay,
            block.index,
            observed,
            check,
        )
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let SubjectState {
            subject_changes,
            subject_late,
        } = &mut *self.state;
        subjects::prepared::observe_block(
            &mut self.scan,
            subject_changes,
            subject_late,
            block.replay,
            block.index,
            None,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn pending(&self) -> usize {
        self.state.subject_changes.bytes()
    }

    /// The change events, then their catalogue, as hourly preparation's
    /// flush writes them; the first error stops the second.
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let SubjectState {
            subject_changes,
            subject_late,
        } = &mut *self.state;
        subject_changes.flush(writer, &format!("subject-changes-{branch}"), 0, cancelled)?;
        series::write_catalogue(
            writer,
            "subject-changes",
            branch,
            &BTreeMap::new(),
            &(
                self.scan.observed,
                self.scan.change_warnings(),
                &*subject_late,
            ),
        )
    }

    /// The sealed scan, which is also its continuation.
    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        self.scan.seal();
        let bytes = series::compress(serde_json::to_vec(&*self.scan)?.as_slice(), 3)?;
        Ok(writer.write_at(&format!("subjects-{branch}"), 0, bytes.as_slice()))
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        super::super::json(writer, &subject_state(branch), &*self.state)
    }
}

impl Own for PeaceOutcomes {
    fn family(&self) -> &'static prepass::Family {
        &peace::family::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.peace"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["peace-deals"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let prepared = match load.read(&peace_state(branch))? {
            Some(file) => serde_json::from_reader(io::BufReader::new(file))?,
            None => Peace::new(load.replay, branch),
        };
        Ok(Box::new(PeaceBranch(Box::new(prepared))))
    }
}

/// One branch's peace observer.
struct PeaceBranch(Box<Peace>);

impl Branch for PeaceBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        peace::family::fold(
            &mut self.0,
            facts,
            block.replay,
            block.index,
            observed,
            check,
        )
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        peace::family::observe_block(
            &mut self.0,
            block.replay,
            block.index,
            None,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn pending(&self) -> usize {
        self.0.buffered_bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.0.flush(writer, branch, cancelled)
    }

    /// The flush writes every page, the continuation included.
    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        super::super::json(writer, &peace_state(branch), &*self.0)
    }
}

impl Own for Ideas {
    fn family(&self) -> &'static prepass::Family {
        &idea_history::prepared::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.ideas"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["idea-history"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let Some(file) = load.read(&format!("idea-history-{branch}"))? else {
            return Ok(Box::new(IdeasBranch {
                scan: Box::new(idea_history::Scan::new(load.replay, branch)),
                changes: Box::default(),
                content: None,
            }));
        };
        let scan = idea_history::Scan::read(io::BufReader::new(file))?;
        let file = load
            .read(&idea_state(branch))?
            .ok_or_else(|| super::super::invalid("Prepared idea changes are missing"))?;
        let changes: Box<PreparedChanges> = serde_json::from_reader(io::BufReader::new(file))?;
        Ok(Box::new(IdeasBranch {
            scan: Box::new(scan),
            changes,
            content: None,
        }))
    }
}

/// One branch's idea scan and change publisher, with the idea classification
/// content once read.
struct IdeasBranch {
    scan: Box<idea_history::Scan>,
    changes: Box<PreparedChanges>,
    content: Option<Option<Arc<PoliticsContent>>>,
}

impl IdeasBranch {
    /// The classification content, bound as hourly preparation binds it: a
    /// content that cannot be read classifies nothing, and a cancelled read
    /// stops preparation.
    fn content(&mut self, block: &Block<'_>) -> io::Result<Option<Arc<PoliticsContent>>> {
        let _cpu = ::replay::cpu::scope("fold binds", None);
        if let Some(content) = &self.content {
            return Ok(content.clone());
        }
        let loaded = block
            .context
            .content()
            .and_then(|options| politics_content(block.source, options));
        if let Err(error) = &loaded
            && error.kind() == io::ErrorKind::Interrupted
        {
            return Err(io::Error::new(error.kind(), error.to_string()));
        }
        let content = loaded.ok();
        self.content = Some(content.clone());
        Ok(content)
    }
}

impl Branch for IdeasBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let content = self.content(block)?;
        idea_history::prepared::fold(
            &mut self.scan,
            &mut self.changes,
            content.as_deref(),
            facts,
            block.replay,
            block.index,
            observed,
            check,
        )
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let content = self.content(block)?;
        idea_history::prepared::observe_block(
            &mut self.scan,
            &mut self.changes,
            content.as_deref(),
            block.replay,
            block.index,
            None,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn pending(&self) -> usize {
        self.changes.bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan
            .flush_changes(writer, branch, &mut self.changes, cancelled)
    }

    /// The sealed scan, which is also its continuation.
    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        let bytes = self.scan.saved_bytes()?;
        Ok(writer.write_at(&format!("idea-history-{branch}"), 0, bytes.as_slice()))
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        super::super::json(writer, &idea_state(branch), &*self.changes)
    }
}
