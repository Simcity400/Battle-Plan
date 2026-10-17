use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirParams {
    pub branch_id: String,
    pub view: AirView,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub mode: Option<AirMode>,
    pub viewpoint: Option<String>,
    pub viewpoint_country: Option<bool>,
    pub sides: Option<Vec<String>>,
    pub regions: Option<Vec<u32>>,
    pub top: Option<usize>,
    pub theatres: Option<Vec<String>>,
    pub windows_only: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
pub enum AirView {
    Timeline,
    Moment,
    Choices,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
pub enum AirMode {
    Sides,
    Viewpoint,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirSpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

pub type AirSideRead = (
    i32,
    String,
    Option<String>,
    Option<String>,
    usize,
    Vec<(String, String)>,
    Option<String>,
);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirSide {
    pub key: String,
    pub faction: Option<String>,
    pub colour_faction: Option<String>,
    pub tag: Option<String>,
    pub name: String,
    pub leader: Option<String>,
    pub co_belligerents: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<(String, String)>>", optional)]
    pub factions: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<AirSideRead>>", optional)]
    pub reads: Vec<AirSideRead>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirSideCell {
    pub scores: Vec<(u32, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<(u32, String)>>", optional)]
    pub viewpoints: Vec<(u32, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<u32>>", optional)]
    pub unknown: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unknown_set: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<u32>>", optional)]
    pub no_air_object: Vec<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub filtered: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    #[ts(as = "Option<usize>", optional)]
    pub excluded: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(untagged)]
pub enum AirValue {
    Reason(String),
    Viewpoint((String, String)),
    Scores(Vec<(u32, String)>),
    SideViewpoint {
        view: (String, String),
        side: u32,
        leader: Option<String>,
    },
    Sides(AirSideCell),
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
pub struct AirCell(pub Option<AirValue>);

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirTimelineRow {
    pub region: u32,
    pub name: String,
    pub cells: Vec<(usize, AirCell)>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirMomentRegion {
    pub id: u32,
    pub name: String,
    pub active: Vec<AirSpan>,
    pub runs: Vec<(i32, i32, AirCell)>,
    pub wings: Vec<(i32, i32, u32)>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirRegionChoice {
    pub id: u32,
    pub name: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirTheatreChoice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirTimelineReply {
    #[ts(type = "\"timeline\"")]
    pub view: AirView,
    pub branch_id: String,
    pub available: bool,
    pub first_hour: i32,
    pub last_hour: i32,
    pub days: Vec<Option<i32>>,
    /// Supplying keyframe hours for approximate daily cells, None for exact cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sample_hours: Option<Vec<Option<i32>>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<AirSpan>>", optional)]
    pub approximate: Vec<AirSpan>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub rows_approximate: bool,
    pub sides: Vec<AirSide>,
    pub segments: std::collections::BTreeMap<String, Vec<AirSpan>>,
    pub anchor_reads: std::collections::BTreeMap<String, (String, i32)>,
    pub rows: Vec<AirTimelineRow>,
    pub excluded: Vec<(i32, i32, usize)>,
    pub theatre_reads: Vec<(i32, i32, i32)>,
    pub scale: i32,
    pub day_hours: i32,
    pub bad_below: i32,
    pub good_above: i32,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirMomentReply {
    #[ts(type = "\"moment\"")]
    pub view: AirView,
    pub branch_id: String,
    pub available: bool,
    pub first_hour: i32,
    pub last_hour: i32,
    #[serde(with = "moment_sides")]
    #[ts(as = "Vec<AirMomentSide>")]
    pub sides: Vec<AirSide>,
    pub segments: std::collections::BTreeMap<String, Vec<AirSpan>>,
    pub anchor_reads: std::collections::BTreeMap<String, (String, i32)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<Vec<u32>>>", optional)]
    pub unknown_sets: Vec<Vec<u32>>,
    pub regions: Vec<AirMomentRegion>,
    pub fallback: Vec<(i32, i32, AirCell)>,
    pub recorded: Vec<AirSpan>,
    pub complete: Vec<AirSpan>,
    pub wings_known: Vec<AirSpan>,
    pub excluded: Vec<(i32, i32, usize)>,
    pub theatre_reads: Vec<(i32, i32, i32)>,
    pub scale: i32,
    pub day_hours: i32,
    pub bad_below: i32,
    pub good_above: i32,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirChoicesReply {
    #[ts(type = "\"choices\"")]
    pub view: AirView,
    pub branch_id: String,
    pub regions: Vec<AirRegionChoice>,
    pub sides: Vec<AirSide>,
    pub theatres: Vec<AirTheatreChoice>,
    pub countries: Vec<String>,
    pub windows: Vec<AirSpan>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum AirReply {
    Timeline(AirTimelineReply),
    Moment(AirMomentReply),
    Choices(AirChoicesReply),
}

impl dashboard_base::products::ReplyWarnings for AirReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::Timeline(reply) => &mut reply.warnings,
            Self::Moment(reply) => &mut reply.warnings,
            Self::Choices(reply) => &mut reply.warnings,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum AirMomentSide {
    Country((String, i32)),
    Side(AirSide),
}

mod moment_sides {
    use super::*;
    use serde::ser::SerializeSeq;

    #[derive(Serialize)]
    #[serde(untagged)]
    enum SideRef<'a> {
        Country((&'a str, i32)),
        Side(&'a AirSide),
    }

    pub fn serialize<S: serde::Serializer>(
        sides: &[AirSide],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(sides.len()))?;
        for side in sides {
            let compact = side
                .tag
                .as_deref()
                .zip(side.reads.first())
                .filter(|(tag, read)| {
                    side.key.strip_prefix("c:") == Some(*tag)
                        && side.name == *tag
                        && side.leader.as_deref() == Some(*tag)
                        && side.faction.is_none()
                        && side.colour_faction.is_none()
                        && side.co_belligerents == 0
                        && side.factions.is_empty()
                        && side.reads.len() == 1
                        && read.1 == *tag
                        && read.2.as_deref() == Some(*tag)
                        && read.3.as_deref() == Some(*tag)
                        && read.4 == 0
                        && read.5.is_empty()
                        && read.6.is_none()
                });
            sequence.serialize_element(&match compact {
                Some((tag, read)) => SideRef::Country((tag, read.0)),
                None => SideRef::Side(side),
            })?;
        }
        sequence.end()
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<AirSide>, D::Error> {
        Ok(Vec::<AirMomentSide>::deserialize(deserializer)?
            .into_iter()
            .map(|side| match side {
                AirMomentSide::Side(side) => side,
                AirMomentSide::Country((tag, hour)) => AirSide {
                    key: format!("c:{tag}"),
                    faction: None,
                    colour_faction: None,
                    tag: Some(tag.clone()),
                    name: tag.clone(),
                    leader: Some(tag.clone()),
                    co_belligerents: 0,
                    factions: vec![],
                    reads: vec![(
                        hour,
                        tag.clone(),
                        Some(tag.clone()),
                        Some(tag),
                        0,
                        vec![],
                        None,
                    )],
                },
            })
            .collect())
    }
}

pub(super) fn types() -> Vec<String> {
    vec![
        AirParams::decl(),
        AirView::decl(),
        AirMode::decl(),
        AirSpan::decl(),
        AirSide::decl(),
        AirSideCell::decl(),
        AirValue::decl(),
        AirCell::decl(),
        AirTimelineRow::decl(),
        AirMomentRegion::decl(),
        AirRegionChoice::decl(),
        AirTheatreChoice::decl(),
        AirTimelineReply::decl(),
        AirMomentReply::decl(),
        AirMomentSide::decl(),
        AirChoicesReply::decl(),
        AirReply::decl(),
    ]
}
