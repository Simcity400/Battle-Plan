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
