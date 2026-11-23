
use super::{Content, Country, IDENTITY, Tags, branch, missing};
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour}}};
use replay_data::rows::{OperativeMembershipRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: [TableId; 3] = [TableId::OperativeMemberships, IDENTITY[0], IDENTITY[1]];

pub const ACTIVE: u8 = 0;
pub const CAPTURED: u32 = 0;
pub const MISSION: u32 = 3;
pub const OPERATION: u32 = 4;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Operative {
    #[doc = " The full handle, as a decimal string."]
    pub id: String,
    pub name: Option<String>,
    #[doc = " Why the name is not shown, when it is not."]
    pub name_missing: Option<String>,
}

impl Operative {
    pub fn of(row: &OperativeMembershipRow, content: &Content) -> Self {
        let (name, name_missing) = match &row.name {
            Recorded::NotRecorded => (None, Some("The operative's name is not recorded".to_owned())),
            Recorded::Value(bytes) => match content.text(content_reader_parser::parser::c_string(bytes)) {
                Some(name) => (Some(name), None),
                None => (None, Some("The operative has no saved name".to_owned())),
            },
        };
        Self { id: row.id.to_string(), name, name_missing }
    }
}

pub fn target(row: &OperativeMembershipRow) -> Option<u32> {
    if row.roster_kind != ACTIVE || row.assignment == CAPTURED {
        return None;
    }
    let working = match row.assignment {
        MISSION => row.action_variant.is_some_and(|variant| variant != 0),
        OPERATION => matches!(row.operation, Recorded::Value(Some(operation)) if operation != 0),
        _ => false,
    };
    let Recorded::Value(Some(target)) = row.target_country else { return None };
    working.then_some(target)
}

pub struct OperativePlacements;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperativePlacementParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub owner: Country,
    pub target: Country,
    pub operatives: Vec<Operative>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativePlacementReply {
    #[doc = " By owner, then target id."]
    pub placements: Vec<Placement>,
    #[doc = " The census's read hour the rows hold (it is taken daily)."]
    pub read_hour: Option<i32>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(OperativePlacementReply);

pub fn placements(frame: &DecodedFrame, content: &Content) -> io::Result<Vec<Placement>> {
    let tags = Tags::read(frame)?;
    let rows = frame.complete::<tables::OperativeMemberships>()?;
    let mut pairs: BTreeMap<(u32, u32), Vec<Operative>> = BTreeMap::new();
    for row in rows {
        if let Some(target) = target(row) {
            pairs.entry((row.owner_country, target)).or_default().push(Operative::of(row, content));
        }
    }
    Ok(pairs
        .into_iter()
        .map(|((owner, target), operatives)| Placement { owner: Country::of(&tags, owner), target: Country::of(&tags, target), operatives })
        .collect())
}

impl AtHour for OperativePlacements {
    const KEY: &'static str = "operative-placements";
    const TITLE: &'static str = "Operative placements";
    const CONTENT_DEPENDENT: bool = true;
    type Params = OperativePlacementParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = OperativePlacementReply;
    type Context = Content;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![OperativePlacementParams::decl(), Country::decl(), Operative::decl(), Placement::decl(), OperativePlacementReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(Content::load(source, load))
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let read_hour = frame.tables[TableId::OperativeMemberships.index()].sampled_at();
        let missing = missing(frame, TableId::OperativeMemberships);
        Ok(match placements(frame, at.context) {
            Ok(placements) => OperativePlacementReply { placements, read_hour, missing, warnings: Vec::new() },
            Err(error) => OperativePlacementReply { placements: Vec::new(), read_hour, missing: Some(missing.unwrap_or_else(|| error.to_string())), warnings: Vec::new() },
        })
    }
}
