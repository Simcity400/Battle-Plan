//! What the family reads of an hour, for its columnar operator and its emit: the channels, the
//! lanes' kinds and choices, and the programmes' supplying reads.
use super::*;
use dashboard_base::{
    Source, catalogue,
    metadata::check_cancel,
    prepared::{
        frame_changes::FrameChanges,
        query::{frames::Walk, native, progressive},
    },
    products::faction_details::{self, FactionRuleDetail, GoalLabel, selectors},
    triggers::inputs as reuse,
};
use replay_data::rows::{RowCodec, TableId, TableSchema, tables};
use replay_frame::frame::{DecodedFrame, SharedView, TableView};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
/// A known value read as unavailable, apart from every value a lane holds.
pub(in crate::products::faction_state) const NULL: i128 = i64::MIN as i128 - 1;
pub const CHANNELS: [(&str, TableId); 8] = [
    ("influence", TableId::FactionAccounts),
    ("rules", TableId::FactionRules),
    ("programmes", TableId::FactionPrograms),
    ("minimums", TableId::FactionMinimumUpgrades),
    ("members", TableId::FactionAccounts),
    ("doctrines", TableId::FactionDoctrine),
    ("manpower", TableId::FactionManpower),
    ("goals", TableId::FactionGoalEntries),
];
/// What a lane holds; its measure in the family's publication is its position in [`Kind::ALL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) enum Kind {
    Share,
    Association,
    Upgrade,
    Known,
    Read,
    Rule,
    Programme,
    Minimum,
    Doctrine,
    Manpower,
    ManpowerDiagnostic,
    Total,
    Group,
    GoalList,
    GoalProgress,
    Roster,
}
impl Kind {
    pub(in crate::products::faction_state) const ALL: [Self; 16] = [
        Self::Share,
        Self::Association,
        Self::Upgrade,
        Self::Known,
        Self::Read,
        Self::Rule,
        Self::Programme,
        Self::Minimum,
        Self::Doctrine,
        Self::Manpower,
        Self::ManpowerDiagnostic,
        Self::Total,
        Self::Group,
        Self::GoalList,
        Self::GoalProgress,
        Self::Roster,
    ];
    pub(in crate::products::faction_state) fn measure(self) -> u64 {
        self as u64
    }
    fn selection_name(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Programme => "programme",
            Self::Minimum => "minimum-upgrade",
            Self::Doctrine => "doctrine",
            Self::Manpower => "manpower",
            _ => unreachable!("Lane kind has no selected table"),
        }
    }
}
/// Every table [`Supplying`] reads through the native hour reader: the programmes' supplying
/// read and its widest frame.
pub fn native_tables() -> Vec<TableId> {
    let mut tables = selectors::programme_text_tables();
    tables.push(TableId::ProjectPrograms);
    tables
}
/// One branch's programme supplying reads, each sought in rising hours.
pub struct Supplying<'a> {
    hours: Walk<'a>,
    frames: Walk<'a>,
}
impl<'a> Supplying<'a> {
    pub fn new(source: &'a Source, branch: u64) -> Self {
        Self {
            hours: Walk::new(source, branch),
            frames: Walk::new(source, branch),
        }
    }
    /// The hour of `frame`'s latest supplying read of programme operands.
    pub(super) fn read_hour(
        &mut self,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<i32>> {
        if self.hours.branch() != frame.branch {
            *self = Supplying::new(self.hours.source(), frame.branch);
        }
        // A native recording's row frames leave the programmes' statuses to its capture.
        match native::read_hour(
            self.hours.source(),
            frame.branch,
            frame.hour,
            TableId::ProjectPrograms,
            replay_data::cadence::STAGGER_PERIOD,
            cancelled,
        ) {
            Ok(Some(read)) => return Ok(read),
            Ok(None) => {}
            // An index refused or not holding the hour leaves the read unknown, as a lost one.
            Err(error) if error.kind() != io::ErrorKind::Interrupted => return Ok(None),
            Err(error) => return Err(error),
        }
        progressive::walked_read_hour(
            &mut self.hours,
            frame.hour,
            TableId::ProjectPrograms,
            replay_data::cadence::STAGGER_PERIOD,
            cancelled,
        )
    }
    /// The supplying read at `hour`: the programme tables, and every scripted-text table
    /// when `wide`, those its row frame leaves to the native capture read from its index.
    pub(super) fn frame(
        &mut self,
        hour: i32,
        wide: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Arc<DecodedFrame>>> {
        let tables = if wide {
            selectors::programme_text_tables()
        } else {
            selectors::PROGRAMME_TABLES.to_vec()
        };
        let Some(mut read) = progressive::walked_supplying_frame(
            &mut self.frames,
            hour,
            TableId::ProjectPrograms,
            &tables,
            cancelled,
        )?
        else {
            return Ok(None);
        };
        native::fill(self.frames.source(), &mut read, &tables, cancelled)?;
        Ok(Some(Arc::from(read)))
    }
}
/// A selection lane's value: its token and name, or why the hour has none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) struct Choice {
    pub(in crate::products::faction_state) token: Option<String>,
    pub(in crate::products::faction_state) name: Option<String>,
    pub(in crate::products::faction_state) reason: Option<String>,
}
impl Choice {
    pub(in crate::products::faction_state) fn unknown(reason: String) -> Self {
        Self {
            reason: Some(projection::timeless(reason)),
            ..Default::default()
        }
    }
}
impl From<dashboard_base::products::faction_details::FactionRuleDetail> for Choice {
    fn from(detail: dashboard_base::products::faction_details::FactionRuleDetail) -> Self {
        Self {
            token: detail.token,
            name: detail.name,
            reason: detail.reason.map(projection::timeless),
        }
    }
}
impl From<dashboard_base::products::faction_details::FactionUpgradeDetail> for Choice {
    fn from(detail: dashboard_base::products::faction_details::FactionUpgradeDetail) -> Self {
        Self {
            token: detail.token,
            name: detail.name,
            reason: detail.reason.map(projection::timeless),
        }
    }
}
impl From<dashboard_base::products::faction_details::FactionProgramDetail> for Choice {
    fn from(detail: dashboard_base::products::faction_details::FactionProgramDetail) -> Self {
        Self {
            token: Some(detail.reference),
            name: detail.project,
            reason: detail.reason.map(projection::timeless),
        }
    }
}
pub(super) mod goals;
mod projection;
use goals::GoalList;
pub(in crate::products::faction_state) use projection::{
    ACCOUNT_FIELDS, AccountOp, AccountRow, Accounts, ChannelOp, Edits, FactionOps, Frames, Ghost,
    GoalsOp, ManpowerEntry, ManpowerOp, Owner, Projected, Runtime,
};
#[cfg(test)]
pub(in crate::products::faction_state) use projection::{Whole, timeless};
#[cfg(test)]
mod columnar_parity;
