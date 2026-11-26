use super::*;
use super::at_hour::ForceContext;
use dashboard_world::products::group_members::GroupsAt;
use replay_data::rows::TableId;

#[test]
fn parts_split_a_total_only_where_their_rows_add_up_to_it() {
    let held = [(0, Some(30)), (0, Some(20)), (1, Some(10))];
    assert_eq!(split(Some(60), &held, 2), (vec![Some(50), Some(10)], true));
    assert_eq!(split(Some(60), &[(0, Some(50)), (1, None)], 2), (vec![Some(50), None], false), "a witnessed part keeps its count beside an unread one");
    assert_eq!(split(Some(40), &held, 2), (vec![Some(50), Some(10)], false), "parts that do not add up keep their counts; only their shares are withheld");
    assert_eq!(split(None, &held, 2), (vec![Some(50), Some(10)], false), "a missing total does not clear the parts");
    assert_eq!(split(Some(0), &[], 3), (vec![Some(0), Some(0), Some(0)], true));
}

#[test]
fn recorded_parts_group_under_their_reply_part_in_key_order() {
    let resolved = BTreeMap::from([
        (3u128, ForcePart { key: "fighter".into(), name: Some("Fighter".into()), unresolved: false }),
        (1u128, ForcePart { key: "cas".into(), name: None, unresolved: false }),
        (2u128, ForcePart { key: "fighter".into(), name: Some("Fighter".into()), unresolved: false }),
    ]);
    let named = Named::group(resolved);
    assert_eq!(named.parts.iter().map(|part| part.key.as_str()).collect::<Vec<_>>(), vec!["cas", "fighter"]);
    assert_eq!((named.of[&1], named.of[&2], named.of[&3]), (0, 1, 1));
    let named = named.in_content_order(["bomber", "fighter", "bomber", "transport"].map(String::from));
    assert_eq!(named.ordinals(), vec![None, Some(1)], "the content's own order, first listing kept; a part it does not list has no ordinal");
}

#[test]
fn force_at_hour_ranks_top_and_withholds_unreconciled_parts() {
    use dashboard_base::products::at_hour::{At, AtHour};
    use dashboard_follow::live::Cells;
    use dashboard_query::identity::{Holder, HolderRun, Holders};
    use replay_data::rows::{AirForcePartRow, AirForceTotalRow, Recorded, TableRows};
    use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TABLE_COUNT, TableView};
    use std::sync::atomic::AtomicBool;

    let mut holders = Holders::default();
    holders.tags.insert("GER".into(), vec![HolderRun { first: 0, last: 100, holder: Holder::Found { slot: 1, id: 10 }, starts_segment: true }]);
    holders.tags.insert("ENG".into(), vec![HolderRun { first: 0, last: 100, holder: Holder::Found { slot: 2, id: 20 }, starts_segment: true }]);
    holders.tags.insert("FRA".into(), vec![HolderRun { first: 0, last: 100, holder: Holder::Found { slot: 3, id: 30 }, starts_segment: true }]);

    let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| TableView::Unavailable { reason: "outside test".into() }.into());
    *tables[TableId::AirForceTotals.index()] = TableView::Complete(TableRows::AirForceTotals(vec![
        AirForceTotalRow { slot: 1, roster_planes: Recorded::Value(50), ..AirForceTotalRow::default() },
        AirForceTotalRow { slot: 2, roster_planes: Recorded::Value(80), ..AirForceTotalRow::default() },
        AirForceTotalRow { slot: 3, roster_planes: Recorded::Value(80), ..AirForceTotalRow::default() },
    ]));
    *tables[TableId::AirForceParts.index()] = TableView::Complete(TableRows::AirForceParts(vec![
        AirForcePartRow { slot: 2, ownership: 0, variant: Some(100), planes: Recorded::Value(80) },
        AirForcePartRow { slot: 3, ownership: 0, variant: Some(100), planes: Recorded::Value(60) },
    ]));
    let frame = DecodedFrame { branch: 1, hour: 10, tables, metrics: Metrics::default() };
    let live = Cells::default();
    let cancel = AtomicBool::new(false);
    let ctx = ForceContext { catalogue: None, warnings: Vec::new(), groups: GroupsAt::default() };
    let at = At { owner: None, holders: &holders, hour: 10, context: &ctx, cancelled: &cancel, live: &live };
    let params = AirForceAtParams {
        branch_id: "1".into(),
        countries: Vec::new(),
        top: Some(2),
        group: None,
        measure: AirForceMeasure::Planes,
        ownership: AirForceOwnership::Roster,
    };
    let reply = AirForceAt::reply(&frame, &params, &at).unwrap();
    assert_eq!(reply.missing, None);
    assert_eq!(reply.rows.iter().map(|row| (row.tag.as_str(), row.slot, row.total)).collect::<Vec<_>>(), vec![("ENG", Some(2), Some(80)), ("FRA", Some(3), Some(80))]);
    assert_eq!(reply.rows[0].parts_missing, None);
    assert_eq!(reply.rows[0].values, vec![Some(80)]);
    assert!(reply.rows[1].parts_missing.is_some(), "parts that do not add up to the total are not its shares");
    assert_eq!(reply.rows[1].values, vec![Some(60)], "and each witnessed count is kept");

    let truncated_params = AirForceAtParams { countries: vec![Country::from("ENG"), Country::from("GER")], top: None, ..params };
    let mut trunc_tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| TableView::Unavailable { reason: "outside test".into() }.into());
    *trunc_tables[TableId::AirForceTotals.index()] = TableView::Truncated {
        rows: TableRows::AirForceTotals(vec![AirForceTotalRow { slot: 2, roster_planes: Recorded::Value(80), ..AirForceTotalRow::default() }]),
        missing: 1,
    };
    *trunc_tables[TableId::AirForceParts.index()] = TableView::NotRecorded { mode: replay_frame::scope::CaptureMode::Map };
    let trunc_frame = DecodedFrame { branch: 1, hour: 10, tables: trunc_tables, metrics: Metrics::default() };
    let trunc_reply = AirForceAt::reply(&trunc_frame, &truncated_params, &at).unwrap();
    assert_eq!(trunc_reply.rows[0].tag, "ENG");
    assert_eq!(trunc_reply.rows[0].total, Some(80), "witnessed country keeps its total from a truncated table");
    assert!(trunc_reply.rows[0].parts_missing.is_some());
    assert_eq!(trunc_reply.rows[1].tag, "GER");
    assert_eq!(trunc_reply.rows[1].total, None);
    assert!(trunc_reply.rows[1].missing.is_some());

    use dashboard_base::span::HourSpan;
    use dashboard_query::series::ranked::{GroupKind, GroupMembership, GroupSelection};
    use dashboard_world::products::groups::{CountryGroupReply, FactionGroup, MembershipRun, UnobservedHours};
    let run = |tag: &str, first_hour, last_hour| MembershipRun { tag: tag.into(), first_hour, last_hour };
    let groups = GroupsAt::new(
        std::sync::Arc::new(CountryGroupReply {
            branch_id: "1".into(),
            humans: Vec::new(),
            factions: vec![FactionGroup {
                id: "42".into(),
                name: None,
                name_reason: None,
                names: Vec::new(),
                recorded_name: String::new(),
                first_hour: 0,
                last_hour: 100,
                members: vec![run("GER", 0, 100), run("FRA", 20, 100)],
                leaders: Vec::new(),
                incomplete: vec![HourSpan { first_hour: 50, last_hour: 60 }],
            }],
            wars: Vec::new(),
            war_starts: Vec::new(),
            capitulations: Vec::new(),
            identity_changes: Vec::new(),
            country_identities: Vec::new(),
            unobserved: UnobservedHours::default(),
            warnings: Vec::new(),
        }),
        0,
    );
    let grouped_ctx = ForceContext { catalogue: None, warnings: Vec::new(), groups };
    let faction = GroupSelection { kind: GroupKind::Faction, id: Some("42".into()), tag: None, membership: GroupMembership::Hour };
    let grouped = AirForceAtParams { countries: Vec::new(), top: None, group: Some(faction), ..truncated_params.clone() };
    assert!(AirForceAt::validate(&AirForceAtParams { top: Some(2), ..grouped.clone() }).is_err(), "a group is never sent with a top");
    let grouped_at = |hour: i32| At { owner: None, holders: &holders, hour, context: &grouped_ctx, cancelled: &cancel, live: &live };
    let at_hour = |hour: i32| DecodedFrame { hour, ..frame.clone() };
    let before = AirForceAt::reply(&at_hour(10), &grouped, &grouped_at(10)).unwrap();
    assert_eq!(before.rows.iter().map(|row| (row.tag.as_str(), row.slot)).collect::<Vec<_>>(), vec![("GER", Some(1))], "FRA joins at 20: it is not a member at 10");
    assert_eq!(AirForceAt::valid_through(&at_hour(10), &grouped, &grouped_at(10)), 19, "the reply holds until the hour before FRA joins");
    let after = AirForceAt::reply(&at_hour(20), &grouped, &grouped_at(20)).unwrap();
    assert_eq!(after.rows.iter().map(|row| row.tag.as_str()).collect::<Vec<_>>(), vec!["FRA", "GER"]);
    let unknown = AirForceAt::reply(&at_hour(55), &grouped, &grouped_at(55)).unwrap();
    assert_eq!((unknown.rows.len(), unknown.missing.as_deref()), (0, Some("Membership of this group is not recorded at this hour")), "unknown membership is never a guess");
    let twice = AirForceAtParams { countries: vec![Country::from("GER"), Country::Slot(1)], ..truncated_params };
    assert_eq!(AirForceAt::reply(&frame, &twice, &at).unwrap().rows.len(), 1, "a holder asked by tag and by slot is one row");
}

#[test]
fn a_chosen_country_whose_holder_is_not_known_at_the_hour_is_kept_as_not_recorded_with_the_recorded_reason() {
    use dashboard_query::identity::{Country, Holder, HolderRun, Holders};
    use replay_data::rows::tables;
    use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TABLE_COUNT, TableView};

    let mut holders = Holders::default();
    holders.tags.insert("GER".into(), vec![HolderRun { first: 0, last: 9, holder: Holder::Found { slot: 1, id: 10 }, starts_segment: true }]);
    holders.unknown.push((10, 12, "Countries was not complete at these hours".into()));
    let tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| TableView::Unavailable { reason: "outside test".into() }.into());
    let frame = DecodedFrame { branch: 1, hour: 11, tables, metrics: Metrics::default() };
    let rows = super::at_hour::select_explicit::<tables::AirForceTotals>(&frame, &[Country::from("GER")], |_| None, |_| None, None, &holders, 11, "planes");
    assert_eq!(rows.len(), 1, "the chosen country is kept");
    assert_eq!((rows[0].slot, rows[0].total, rows[0].missing.as_deref()), (None, None, Some("Who holds GER is not known: Countries was not complete at these hours")));
}

mod unlinked_slot {
    use super::*;
    use crate::products::army::ArmyAt;
    use dashboard_base::{
        products::at_hour::{AtHour, Read, exact_panel, frame},
        test_fixtures::{CAMPAIGN, ownership_frame, records},
    };
    use dashboard_query::identity::TAG_UNLINKED;
    use replay::campaign::Replay;
    use replay_data::rows::{AirWingRow, ShipRow, TableRows, TaskForceRow, WingEquipmentRow, tables};
    use replay_frame::frame::TableView;
    use serde_json::{Value, json};
    use std::sync::atomic::AtomicBool;

    fn recording() -> Source {
        let frames = (100..=102).map(|hour| {
            let mut frame = ownership_frame(1, hour, 11);
            if hour == 101 {
                let TableView::Complete(TableRows::CountryLinks(links)) = &mut *frame.tables[TableId::CountryLinks.index()] else { unreachable!() };
                links.retain(|link| link.id != 9001);
            }
            *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![
                AirWingRow { id: 1, owner: 11, planes: if hour == 101 { -1 } else { hour - 90 }, ..Default::default() },
                AirWingRow { id: 2, owner: 44, planes: 0, ..Default::default() },
            ]));
            *frame.tables[TableId::WingEquipment.index()] = TableView::Complete(TableRows::WingEquipment(vec![WingEquipmentRow { wing: 2, variant: 5, ..Default::default() }]));
            *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(vec![ShipRow { id: 1, task_force: 2, strength: 41, ..Default::default() }]));
            *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(vec![TaskForceRow { id: 2, roster_owner: 11, logical_owner: 11, ..Default::default() }]));
            *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(Vec::new()));
            *frame.tables[TableId::DivisionTemplateIcons.index()] = TableView::Complete(TableRows::DivisionTemplateIcons(Vec::new()));
            frame
        }).collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: "forces".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn selection_keeps_the_row_between_known_counts() {
        let src = recording();
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        for (hour, count) in [(100, Some(10)), (101, None), (102, Some(12))] {
            let frame = frame(&query, hour, &[Read::all(TableId::AirForceTotals)], "planes").unwrap();
            let rows = at_hour::select_explicit::<tables::AirForceTotals>(
                &frame, &[Country::Slot(7), Country::Slot(8)], |row| at_hour::recorded_value(row.roster_planes), |_| None, None, &query.holders, hour, "planes",
            );
            let chosen = rows.iter().find(|row| row.country == Country::Slot(7)).expect("the chosen slot keeps its row");
            assert_eq!(chosen.total, count);
            if hour == 101 {
                assert!(chosen.missing.as_deref().unwrap().contains(TAG_UNLINKED));
            } else {
                assert_eq!(chosen.missing, None);
            }
            let linked = rows.iter().find(|row| row.country == Country::Slot(8)).unwrap();
            assert_eq!((linked.total, linked.missing.as_deref()), (Some(0), None));
        }
    }

    fn gap_row<P: AtHour>(mut params: Value) -> (Value, Vec<Value>)
    where P::Params: Sync, P::Context: Sync {
        params["branchId"] = json!("1");
        params["countries"] = json!([7, 8, 77]);
        let panel = exact_panel::<P>(&recording(), &LoadContext { content: None }, &params).unwrap();
        let reply = (panel.exact().unwrap().cold)(101, &AtomicBool::new(false)).unwrap();
        let rows = reply["rows"].as_array().unwrap();
        let linked = rows.iter().find(|row| row["country"] == 8).unwrap();
        assert_eq!(linked["total"], 0);
        assert_eq!(linked["missing"], Value::Null);
        let row = rows.iter().find(|row| row["country"] == 7).expect("the reply keeps the chosen slot");
        assert_eq!(row["slot"], Value::Null);
        assert_eq!(row["tag"], "7");
        let absent = rows.iter().find(|row| row["country"] == 77).unwrap();
        assert_eq!(absent["total"], Value::Null);
        assert!(absent["missing"].as_str().unwrap().contains(TAG_UNLINKED));
        assert!(absent["values"].as_array().unwrap().iter().all(Value::is_null));
        (row.clone(), reply["warnings"].as_array().unwrap().clone())
    }

    #[test]
    fn air_force_keeps_the_missing_plane_count() {
        let (row, _) = gap_row::<AirForceAt>(json!({ "measure": "planes", "ownership": "roster" }));
        assert_eq!(row["total"], Value::Null);
        assert_eq!(row["missing"], format!("Who holds 7 is not known: {TAG_UNLINKED}. No total was recorded for air planes at this hour"));
        assert_eq!(row["values"], json!([null, 0]));
    }

    #[test]
    fn navy_keeps_witnessed_hull_strength_and_class() {
        let (row, warnings) = gap_row::<NavyAt>(json!({ "measure": "strength", "ownership": "roster", "grouping": "class" }));
        assert_eq!(row["total"], 41);
        assert_eq!(row["missing"], Value::Null);
        assert!(warnings.contains(&json!(format!("Who holds 7 is not known: {TAG_UNLINKED}"))));
        assert_eq!(row["values"], json!([41]));
        assert_eq!(row["referenceTotal"], Value::Null);
    }

    #[test]
    fn army_keeps_a_recorded_zero() {
        let (row, warnings) = gap_row::<ArmyAt>(json!({ "countFor": "owner" }));
        assert_eq!(row["total"], 0);
        assert_eq!(row["missing"], Value::Null);
        assert!(warnings.contains(&json!(format!("Who holds 7 is not known: {TAG_UNLINKED}"))));
    }
}
