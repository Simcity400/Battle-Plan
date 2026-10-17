//! What preparing a branch's newest block window did, for measurement and
//! preparation status: the branch's blocks read by their changes before the
//! look-back, every look-back tried (where it started, the blocks it read,
//! its time, and the check that failed with the hour it names), the earlier
//! blocks read for the window's diagnostics, and the outcome.

use std::time::Duration;

/// A look-back's failed proof: the fact it could not prove, and the hour the
/// look-back must start by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unproven {
    pub check: &'static str,
    pub hour: i32,
}

/// One look-back tried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tried {
    /// Its first block, by position in the branch.
    pub from: usize,
    /// The blocks it observed, the last included.
    pub blocks: usize,
    pub took: Duration,
    /// Its failed proof; None when it was proven.
    pub unproven: Option<Unproven>,
}

/// How preparing the window ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    /// Not finished: cancelled or stopped by an error preparation reports.
    #[default]
    Unfinished,
    /// Proven and published.
    Proven,
    /// The look-back would start at the branch's first block, as the failed
    /// proof that moved it there names: the window is left to forward
    /// preparation.
    BranchStart(Option<Unproven>),
    /// The look-back tried is not proven: the window is left to forward
    /// preparation.
    Unproven(Unproven),
    /// No hour the look-back read is available.
    Unavailable,
    /// Naming the variants read on the branch refuses text.
    NamingRefuses,
    /// A block could not be read or forward preparation would refuse: the
    /// reason.
    Unreadable(String),
}

impl Outcome {
    /// A short name, for timing rows.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unfinished => "unfinished",
            Self::Proven => "proven",
            Self::BranchStart(_) => "branch-start",
            Self::Unproven(_) => "unproven",
            Self::Unavailable => "unavailable",
            Self::NamingRefuses => "naming-refuses",
            Self::Unreadable(_) => "unreadable",
        }
    }
}

/// What preparing one branch's newest window did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The branch's blocks.
    pub blocks: usize,
    /// Blocks before the last read by their changes for the look-back's
    /// proof, and their time.
    pub earlier: usize,
    pub earlier_took: Duration,
    /// Every look-back tried, in order.
    pub tried: Vec<Tried>,
    /// Blocks before the proven look-back read for the window's diagnostics,
    /// and their time.
    pub naming: usize,
    pub naming_took: Duration,
    pub outcome: Outcome,
}

impl Report {
    /// The report as one line: the outcome, then each look-back tried.
    pub fn describe(&self) -> String {
        let from = self.tried.last().map_or(0, |tried| tried.from);
        let mut line = match &self.outcome {
            Outcome::Unfinished => "not finished".to_owned(),
            Outcome::Proven => format!("proven from block {from} of {}", self.blocks),
            Outcome::BranchStart(why) => format!(
                "left to forward preparation: the look-back would start at the branch's first block{}",
                why.as_ref().map_or(String::new(), |why| format!(
                    " ({} at hour {})",
                    why.check, why.hour
                ))
            ),
            Outcome::Unproven(why) => format!(
                "left to forward preparation: the look-back from block {from} of {} is not proven ({} at hour {})",
                self.blocks, why.check, why.hour
            ),
            Outcome::Unavailable => format!(
                "left to forward preparation: no hour of the look-back from block {from} of {} is available",
                self.blocks
            ),
            Outcome::NamingRefuses => {
                "left to forward preparation: naming the variants read on the branch refuses text"
                    .to_owned()
            }
            Outcome::Unreadable(why) => format!("left to forward preparation: {why}"),
        };
        if self.earlier > 0 {
            line.push_str(&format!(
                "; {} earlier blocks read by their changes in {} ms",
                self.earlier,
                self.earlier_took.as_millis()
            ));
        }
        for tried in &self.tried {
            line.push_str(&format!(
                "; look-back from block {} ({} blocks, {} ms): {}",
                tried.from,
                tried.blocks,
                tried.took.as_millis(),
                tried
                    .unproven
                    .as_ref()
                    .map_or("proven".to_owned(), |why| format!(
                        "{} at hour {}",
                        why.check, why.hour
                    ))
            ));
        }
        if self.naming > 0 {
            line.push_str(&format!(
                "; {} earlier blocks' naming read in {} ms",
                self.naming,
                self.naming_took.as_millis()
            ));
        }
        line
    }
}
