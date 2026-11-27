use crate::products::subjects::{SubjectLevel, content_levels};
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour, Edits, Owner, Read, ReadsAt, field_columns, warnings_through}}};
use dashboard_declare::autonomy_progress_milli;
use dashboard_text::metadata::LinkTags;
use replay_data::rows::{SubjectRow, TableId, tables};
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AutonomySubject {
    pub overlord_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub overlord_slot: Option<u32>,
    #[doc = " The overlord's tag; null where it is not recorded."]
    pub overlord: Option<String>,
    pub subject_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub subject_slot: Option<u32>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub lower_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub upper_score: Option<f64>,
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

fn active_relations<'a>(relations: &'a [SubjectRow], hour: i32) -> Vec<&'a SubjectRow> {
    relations.iter().filter(|row| row.cancel == 0 && !(row.end_hour > row.start_hour && hour >= row.end_hour)).collect()
}

fn subjects_valid_through(frame: &DecodedFrame, hour: i32) -> i32 {
    frame.complete::<tables::Subjects>().map_or(i32::MAX, |rows| {
        rows.iter()
            .filter(|row| row.cancel == 0 && row.end_hour > row.start_hour && row.end_hour > hour)
            .map(|row| row.end_hour - 1)
            .min()
            .unwrap_or(i32::MAX)
    })
}

fn subjects(frame: &DecodedFrame, levels: &[SubjectLevel]) -> io::Result<Vec<AutonomySubject>> {
    let relations = frame.complete::<tables::Subjects>()?;
    let active = active_relations(relations, frame.hour);
    if active.is_empty() {
        return Ok(Vec::new());
    }
    let mut tags = LinkTags::default();
    let tags_known = tags.update(frame).is_ok();
    let tag = |raw: u32| if tags_known { tags.get(raw).cloned() } else { None };
    let slots: HashMap<u32, u32> = frame
        .complete::<tables::CountryLinks>()
        .map(|rows| rows.iter().map(|link| (link.id, link.canonical_slot)).collect())
        .unwrap_or_default();
    let autonomy = frame.complete::<tables::Autonomy>().map(|rows| rows.iter().map(|row| (row.subject, row)).collect::<HashMap<_, _>>());
    let mut out = Vec::with_capacity(active.len());
    for row in active {
        let (level, progress, score, lower_score, upper_score) = match &autonomy {
            Ok(rows) => match rows.get(&row.subject) {
                Some(held) => {
                    let level = std::str::from_utf8(&held.state).ok().filter(|text| !text.is_empty()).map(str::to_owned);
                    let progress = match autonomy_progress_milli(held.score, held.lower_score, held.upper_score) {
                        Ok(Some(value)) => i32::try_from(value).map_err(|_| "The subject's autonomy progress is out of range".to_owned()),
                        Ok(None) => Err("The subject's autonomy progress is out of range".to_owned()),
                        Err(reason) => Err(reason.to_owned()),
                    };
                    (
                        level,
                        progress,
                        Some(held.score as f64 / f64::from(WHOLE)),
                        Some(held.lower_score as f64 / f64::from(WHOLE)),
                        Some(held.upper_score as f64 / f64::from(WHOLE)),
                    )
                }
                None => (None, Err("The subject's autonomy is not recorded at this hour".to_owned()), None, None, None),
            },
            Err(error) => (None, Err(format!("Autonomy is not recorded at this hour: {error}")), None, None, None),
        };
        let (level_below, level_above, levels_unknown) = neighbours(levels, level.as_deref());
        out.push(AutonomySubject {
            overlord_id: row.overlord,
            overlord_slot: slots.get(&row.overlord).copied(),
            overlord: tag(row.overlord),
            subject_id: row.subject,
            subject_slot: slots.get(&row.subject).copied(),
            subject: tag(row.subject),
            level,
            level_below,
            level_above,
            levels_unknown,
            progress_milli: progress.as_ref().ok().copied(),
            progress_unknown: progress.err(),
            score,
            lower_score,
            upper_score,
        });
    }
    out.sort_by(order);
    Ok(out)
}

impl AtHour for AutonomyLevels {
    const KEY: &'static str = "autonomy-levels";
    const TITLE: &'static str = "Autonomy levels";
    const CONTENT_DEPENDENT: bool = true;
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

    fn columns(table: TableId) -> Option<Vec<usize>> {
        match table {
            TableId::Subjects => Some(field_columns(table, &["start_hour", "end_hour", "cancel"])),
            TableId::Autonomy => Some(field_columns(table, &["score", "lower_score", "upper_score", "state"])),
            TableId::Countries => Some(field_columns(table, &["canonical_slot", "tag"])),
            _ => None,
        }
    }

    fn reads_in(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        match step {
            0 => vec![Read::all(TableId::Subjects)],
            1 => {
                let Ok(relations) = held.complete::<tables::Subjects>() else { return Vec::new() };
                let active = active_relations(relations, at.hour);
                if active.is_empty() {
                    return Vec::new();
                }
                vec![
                    Read::these(TableId::Autonomy, active.iter().map(|row| u128::from(row.subject))),
                    Read::these(TableId::CountryLinks, active.iter().flat_map(|row| [u128::from(row.overlord), u128::from(row.subject)])),
                    Read::all(TableId::Countries),
                ]
            }
            _ => Vec::new(),
        }
    }

    fn retained_reads(params: &Self::Params, owner: Option<Owner>, stage: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        let until = if stage == 1 { subjects_valid_through(held, at.hour) } else { i32::MAX };
        (Self::reads_in(params, owner, stage, held, at), until)
    }

    fn plan_changes(stage: usize, edits: &Edits, _held: &DecodedFrame, _at: &ReadsAt<'_, Self::Context>) -> bool {
        stage == 1 && edits.fields(TableId::Subjects, u64::MAX)
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let mut warnings = Vec::new();
        let levels = content_levels(source, load, &mut warnings);
        Ok(AutonomyLevelsContext { levels, warnings })
    }

    fn valid_through(frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>) -> i32 {
        subjects_valid_through(frame, frame.hour)
    }

    fn valid_reply_through(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, reply: &Self::Reply, _partial: bool) -> i32 {
        let reasons = reply.missing.as_deref().into_iter().chain(reply.subjects.iter().filter_map(|subject| subject.progress_unknown.as_deref()));
        Self::valid_through(frame, params, at).min(warnings_through(frame.hour, reasons))
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
            overlord_slot: None,
            overlord: Some("ENG".into()),
            subject_id: tag.len() as u32,
            subject_slot: None,
            subject: Some(tag.into()),
            level: None,
            level_below: None,
            level_above: None,
            levels_unknown: None,
            progress_milli: progress.ok(),
            progress_unknown: progress.err().map(str::to_owned),
            score: None,
            lower_score: None,
            upper_score: None,
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

    #[test]
    fn autonomy_levels_played_steps_match_whole_frame_across_edits_expiry_damage_seeks_and_branches() {
        use crate::registrations::AUTONOMY_LEVELS_AUTONOMYLEVELS;
        use dashboard_base::{SourceKey, products::at_hour::frame_panel, test_fixtures::{CAMPAIGN, materialized_frame, played, records, source}};
        use replay::campaign::Replay;
        use replay_data::rows::{AutonomyRow, CountryLinkRow, CountryRow, TableRows};
        use replay_frame::frame::TableView;
        use serde_json::json;
        use std::sync::{Arc, atomic::AtomicBool};

        let build_frame = |branch: u64, h: i32| {
            let mut f = source(branch, h, Some(i64::from(h)));
            let can_tag = if branch == 1 && h >= 107 { b"EEE".to_vec() } else { b"BBB".to_vec() };
            let countries = vec![
                CountryRow { id: 9001, canonical_slot: 7, tag: b"AAA".to_vec(), political_power_raw: Some(i64::from(h)), border_land_doctrine_level: replay_data::rows::Recorded::Value(None), ..CountryRow::default() },
                CountryRow { id: 9002, canonical_slot: 8, tag: can_tag, political_power_raw: Some(i64::from(h * 2)), border_land_doctrine_level: replay_data::rows::Recorded::Value(None), ..CountryRow::default() },
                CountryRow { id: 9003, canonical_slot: 9, tag: b"CCC".to_vec(), border_land_doctrine_level: replay_data::rows::Recorded::Value(None), ..CountryRow::default() },
                CountryRow { id: 9004, canonical_slot: 10, tag: b"DDD".to_vec(), border_land_doctrine_level: replay_data::rows::Recorded::Value(None), ..CountryRow::default() },
            ];
            let links = vec![
                CountryLinkRow { id: 9001, canonical_slot: 7 },
                CountryLinkRow { id: 9002, canonical_slot: 8 },
                CountryLinkRow { id: 9003, canonical_slot: 9 },
                CountryLinkRow { id: 9004, canonical_slot: 10 },
            ];
            *f.tables[TableId::Countries.index()] = if branch == 1 && (121..=122).contains(&h) {
                TableView::Unavailable { reason: "countries damaged".into() }
            } else {
                TableView::Complete(TableRows::Countries(countries))
            };
            *f.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links));

            let mut relations = if branch == 1 && h >= 123 {
                Vec::new()
            } else {
                vec![
                    SubjectRow { overlord: 9001, subject: 9002, start_hour: 90, end_hour: 0, cancel: 0, autonomy_ratio: i64::from(h), state: b"syn_puppet".to_vec() },
                    SubjectRow { overlord: 9001, subject: 9003, start_hour: 90, end_hour: 110, cancel: 0, autonomy_ratio: 0, state: b"syn_puppet".to_vec() },
                ]
            };
            if branch == 2 || (branch == 1 && (112..=122).contains(&h)) {
                relations.push(SubjectRow { overlord: 9001, subject: 9004, start_hour: 112, end_hour: 0, cancel: 0, autonomy_ratio: 0, state: b"syn_colony".to_vec() });
            }
            *f.tables[TableId::Subjects.index()] = if branch == 1 && (118..=120).contains(&h) {
                TableView::Unavailable { reason: "subjects damaged".into() }
            } else {
                TableView::Complete(TableRows::Subjects(relations))
            };

            let bbb_score = if branch == 2 { 800_000 } else if h >= 104 { 600_000 } else { 200_000 };
            let bbb_state = if branch == 2 || h >= 104 { b"syn_dominion".to_vec() } else { b"syn_puppet".to_vec() };
            let autonomy = vec![
                AutonomyRow { subject: 9002, score: bbb_score, lower_score: 0, upper_score: 1_000_000, last_modified_bits: h as u32, state: bbb_state, ..AutonomyRow::default() },
                AutonomyRow { subject: 9003, score: 400_000, lower_score: 0, upper_score: 1_000_000, state: b"syn_puppet".to_vec(), ..AutonomyRow::default() },
                AutonomyRow { subject: 9004, score: i64::from(h) * 1_000, lower_score: 0, upper_score: 1_000_000, state: b"syn_colony".to_vec(), ..AutonomyRow::default() },
            ];
            *f.tables[TableId::Autonomy.index()] = if branch == 1 && (114..=116).contains(&h) {
                TableView::Unavailable { reason: "autonomy damaged".into() }
            } else {
                TableView::Complete(TableRows::Autonomy(autonomy))
            };
            f
        };

        let branch1: Vec<DecodedFrame> = (100..=124).map(|h| build_frame(1, h)).collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, branch1.clone())])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let source = Arc::new(
            Source::from_replay(
                SourceKey { source_id: "autonomy-levels-played".into(), revision: "1".into() },
                "synthetic.battleplan".into(),
                Arc::new(replay),
                &AtomicBool::new(false),
            )
            .unwrap(),
        );
        let load = LoadContext { content: None };
        let req = json!({"branchId": "1"});
        let mut whole = frame_panel::<AutonomyLevels>(&source, &load, &req).unwrap();
        let panel = (AUTONOMY_LEVELS_AUTONOMYLEVELS.panel.unwrap().open)(&source, &load, &req).unwrap();
        let reader = panel.exact().unwrap();
        let steps = played(source.replay_ref(), reader, 100, 124);
        for step in &steps {
            let expected = materialized_frame(&mut *whole, source.replay_ref(), 1, step.hour);
            assert_eq!(*step.value, expected, "autonomy-levels at {}", step.hour);
        }
        assert_eq!(steps[1].product_calls, 0, "unrelated pp, last_modified_bits, autonomy_ratio and non-subject autonomy must not rebuild");
        assert_eq!(steps[2].product_calls, 0, "quiet hour must not rebuild");
        assert_eq!(steps[3].product_calls, 0, "quiet hour must not rebuild");
        assert_eq!(steps[4].product_calls, 1, "subject autonomy level change must rebuild");
        assert_eq!(steps[5].product_calls, 0, "quiet hour after level change must not rebuild");
        assert_eq!(steps[10].product_calls, 1, "subject end_hour expiry without Subjects edit must rebuild");
        assert_eq!(steps[11].product_calls, 0, "quiet hour after expiry must not rebuild");
        assert_eq!(steps[24].product_calls, 0, "steady empty subjects holds");
        for h in [115, 102, 104, 109, 110, 119, 121, 124] {
            let read = (reader.advance)(h, true, &false.into(), None);
            assert_eq!(*read.value, materialized_frame(&mut *whole, source.replay_ref(), 1, h), "autonomy-levels seek {h}");
        }
        let branch2: Vec<DecodedFrame> = (100..=104).map(|h| build_frame(2, h)).collect();
        let multi = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, branch1), records(2, branch2)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let multi_source = Arc::new(
            Source::from_replay(
                SourceKey { source_id: "autonomy-levels-branches".into(), revision: "1".into() },
                "synthetic.battleplan".into(),
                Arc::new(multi),
                &AtomicBool::new(false),
            )
            .unwrap(),
        );
        for branch in &multi_source.metadata().unwrap().branches {
            let id: u64 = branch.id.parse().unwrap();
            let req_b = json!({"branchId": branch.id});
            let mut whole_b = frame_panel::<AutonomyLevels>(&multi_source, &load, &req_b).unwrap();
            let panel_b = (AUTONOMY_LEVELS_AUTONOMYLEVELS.panel.unwrap().open)(&multi_source, &load, &req_b).unwrap();
            let reader_b = panel_b.exact().unwrap();
            for h in 100..=104 {
                let read = (reader_b.advance)(h, h == 100, &false.into(), None);
                assert_eq!(*read.value, materialized_frame(&mut *whole_b, multi_source.replay_ref(), id, h), "branch {} at {h}", branch.id);
            }
        }
    }
}
