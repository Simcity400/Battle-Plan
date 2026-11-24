use super::Content;
use replay_data::rows::{OperativeMembershipRow, Recorded};
use serde::Serialize;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

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

pub fn working(row: &OperativeMembershipRow) -> bool {
    if row.roster_kind != ACTIVE || row.assignment == CAPTURED {
        return false;
    }
    match row.assignment {
        MISSION => row.action_variant.is_some_and(|variant| variant != 0),
        OPERATION => matches!(row.operation, Recorded::Value(Some(operation)) if operation != 0),
        _ => false,
    }
}

pub fn target(row: &OperativeMembershipRow) -> Option<u32> {
    let Recorded::Value(Some(target)) = row.target_country else { return None };
    working(row).then_some(target)
}
