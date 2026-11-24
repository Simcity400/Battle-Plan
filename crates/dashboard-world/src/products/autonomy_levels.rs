use crate::products::subjects::{SubjectLevel, content_levels};
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour}}};
use dashboard_declare::autonomy_progress_milli;
use dashboard_text::metadata::LinkTags;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, collections::HashMap, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const WHOLE: i32 = 100_000;

pub struct AutonomyLevels;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutonomyLevelsParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AutonomySubject {
    pub overlord_id: u32,
    #[doc = " The overlord's tag; null where it is not recorded."]
    pub overlord: Option<String>,
    pub subject_id: u32,
    #[doc = " The subject's tag; null where it is not recorded."]
    pub subject: Option<String>,
    #[doc = " The subject's autonomy level; null where it is not recorded at the hour."]
    pub level: Option<String>,
    #[doc = " The level a falling score reaches, in the content's order; null at the least free level or where unknown."]
    pub level_below: Option<String>,
    #[doc = " The level a rising score reaches, in the content's order; null at the freest level or where unknown."]
    pub level_above: Option<String>,
    #[doc = " Why the neighbouring levels are not known, where they are not."]
    pub levels_unknown: Option<String>,
    #[doc = " Progress from the level's lower score to its upper score, in thousandths of a percent"]
    #[doc = " (100000 is the upper score); null where it is not known."]
    pub progress_milli: Option<i32>,
    #[doc = " Why the progress is not known, where it is not."]
    pub progress_unknown: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AutonomyLevelsReply {
    #[doc = " Every level the content defines, least free first; a level it does not order is listed after."]
    pub levels: Vec<SubjectLevel>,
    #[doc = " Every subject at the hour, closest to a change of level first (the nearer end of its range),"]
    #[doc = " then by subject; a subject whose progress is not known is listed last with its reason."]
    pub subjects: Vec<AutonomySubject>,
    #[doc = " Why the hour's subjects are not known, when they are not."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AutonomyLevelsReply);

pub struct AutonomyLevelsContext {
    levels: Vec<SubjectLevel>,
    warnings: Vec<String>,
}

fn neighbours(levels: &[SubjectLevel], level: Option<&str>) -> (Option<String>, Option<String>, Option<String>) {
    let Some(level) = level else {
        return (None, None, Some("The subject's level is not recorded at this hour".into()));
    };
    let ordered: Vec<&SubjectLevel> = levels.iter().filter(|item| item.ordered).collect();
    match ordered.iter().position(|item| item.token == level) {
        Some(at) => (
            at.checked_sub(1).map(|below| ordered[below].token.clone()),
            ordered.get(at + 1).map(|above| above.token.clone()),
            None,
        ),
        None if ordered.is_empty() => (None, None, Some("The content's order of autonomy levels is not known".into())),
        None => (None, None, Some("The content gives this level no place in its order".into())),
    }
}

fn closeness(progress: i32) -> i32 {
    let clamped = progress.clamp(0, WHOLE);
    clamped.min(WHOLE - clamped)
}

fn order(a: &AutonomySubject, b: &AutonomySubject) -> Ordering {
    let tag = |item: &AutonomySubject| (item.subject.is_none(), item.subject.clone(), item.subject_id);
    a.progress_milli.is_none().cmp(&b.progress_milli.is_none())
        .then_with(|| a.progress_milli.map(closeness).cmp(&b.progress_milli.map(closeness)))
        .then_with(|| tag(a).cmp(&tag(b)))
        .then_with(|| a.overlord_id.cmp(&b.overlord_id))
}

fn subjects(frame: &DecodedFrame, levels: &[SubjectLevel]) -> io::Result<Vec<AutonomySubject>> {
    let relations = frame.complete::<tables::Subjects>()?;
    let mut tags = LinkTags::default();
    let tags_known = tags.update(frame).is_ok();
    let tag = |raw: u32| if tags_known { tags.get(raw).cloned() } else { None };
    let autonomy = frame.complete::<tables::Autonomy>().map(|rows| rows.iter().map(|row| (row.subject, row)).collect::<HashMap<_, _>>());
    let mut out = Vec::new();
    for row in relations.iter() {
        let ended = row.end_hour > row.start_hour && frame.hour >= row.end_hour;
        if row.cancel != 0 || ended {
            continue;
        }
        let (level, progress) = match &autonomy {
            Ok(rows) => match rows.get(&row.subject) {
                Some(held) => {
                    let level = std::str::from_utf8(&held.state).ok().filter(|text| !text.is_empty()).map(str::to_owned);
                    let progress = match autonomy_progress_milli(held.score, held.lower_score, held.upper_score) {
                        Ok(Some(value)) => i32::try_from(value).map_err(|_| "The subject's autonomy progress is out of range".to_owned()),
                        Ok(None) => Err("The subject's autonomy progress is out of range".to_owned()),
                        Err(reason) => Err(reason.to_owned()),
                    };
                    (level, progress)
                }
                None => (None, Err("The subject's autonomy is not recorded at this hour".to_owned())),
            },
            Err(error) => (None, Err(format!("Autonomy is not recorded at this hour: {error}"))),
        };
        let (level_below, level_above, levels_unknown) = neighbours(levels, level.as_deref());
        out.push(AutonomySubject {
            overlord_id: row.overlord,
            overlord: tag(row.overlord),
            subject_id: row.subject,
            subject: tag(row.subject),
            level,
            level_below,
            level_above,
            levels_unknown,
            progress_milli: progress.as_ref().ok().copied(),
            progress_unknown: progress.err(),
        });
    }
    out.sort_by(order);
    Ok(out)
}

impl AtHour for AutonomyLevels {
    const KEY: &'static str = "autonomy-levels";
    const TITLE: &'static str = "Autonomy levels";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = AutonomyLevelsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = AutonomyLevelsReply;
    type Context = AutonomyLevelsContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AutonomyLevelsParams::decl(), AutonomySubject::decl(), AutonomyLevelsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Subjects, TableId::Autonomy, TableId::Countries, TableId::CountryLinks]
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let mut warnings = Vec::new();
        let levels = content_levels(source, load, &mut warnings);
        Ok(AutonomyLevelsContext { levels, warnings })
    }

    fn valid_through(frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>) -> i32 {
        frame.complete::<tables::Subjects>().map_or(i32::MAX, |rows| {
            rows.iter()
                .filter(|row| row.cancel == 0 && row.end_hour > row.start_hour && row.end_hour > frame.hour)
                .map(|row| row.end_hour - 1)
                .min()
                .unwrap_or(i32::MAX)
        })
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let levels = at.context.levels.clone();
        let warnings = at.context.warnings.clone();
        Ok(match subjects(frame, &levels) {
            Ok(subjects) => AutonomyLevelsReply { levels, subjects, missing: None, warnings },
            Err(error) => AutonomyLevelsReply { levels, subjects: Vec::new(), missing: Some(error.to_string()), warnings },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(token: &str, ordered: bool) -> SubjectLevel {
        SubjectLevel { token: token.into(), name: None, ordered }
    }

    fn subject(tag: &str, progress: Result<i32, &str>) -> AutonomySubject {
        AutonomySubject {
            overlord_id: 1,
            overlord: Some("ENG".into()),
            subject_id: tag.len() as u32,
            subject: Some(tag.into()),
            level: None,
            level_below: None,
            level_above: None,
            levels_unknown: None,
            progress_milli: progress.ok(),
            progress_unknown: progress.err().map(str::to_owned),
        }
    }

    #[test]
    fn the_neighbouring_levels_come_from_the_contents_order_and_are_named_unknown_otherwise() {
        let levels = [level("puppet", true), level("dominion", true), level("free", true), level("mod_odd", false)];
        assert_eq!(neighbours(&levels, Some("dominion")), (Some("puppet".into()), Some("free".into()), None));
        assert_eq!(neighbours(&levels, Some("puppet")), (None, Some("dominion".into()), None));
        assert!(neighbours(&levels, Some("mod_odd")).2.is_some());
        assert!(neighbours(&levels, None).2.is_some());
        assert!(neighbours(&[], Some("puppet")).2.is_some());
    }

    #[test]
    fn subjects_closest_to_a_change_come_first_and_an_unread_one_is_last_not_zero() {
        let mut list = [
            subject("MID", Ok(50_000)),
            subject("UNREAD", Err("not recorded")),
            subject("NEARTOP", Ok(97_000)),
            subject("NEARLOW", Ok(1_000)),
            subject("OVER", Ok(150_000)),
        ];
        list.sort_by(order);
        let order: Vec<&str> = list.iter().map(|item| item.subject.as_deref().unwrap()).collect();
        assert_eq!(order, ["OVER", "NEARLOW", "NEARTOP", "MID", "UNREAD"]);
    }
}
