use super::*;
use assignment::{FactionHistory, Groups, Identity, Run, WarRun};
use dashboard_follow::live::GroupState;
use dashboard_query::identity::HolderRun;
use replay_data::rows::{AirRegionWingRow, TableRows};
use replay_frame::frame::{Metrics, TableView, not_selected};

const TAGS: [(&str, u32); 4] = [("ENG", 1), ("FRA", 2), ("GER", 3), ("USA", 4)];

fn run(tag: &str, first_hour: i32, last_hour: i32) -> Run {
    Run { tag: tag.into(), first_hour, last_hour }
}

fn groups(last: i32) -> Groups {
    Groups {
        factions: [(7, FactionHistory { first_hour: 0, members: vec![run("ENG", 0, last), run("FRA", 0, last)], leaders: vec![run("ENG", 0, last)], incomplete: vec![] })].into(),
        wars: ["ENG", "FRA"].map(|tag| WarRun { first: tag.into(), second: "GER".into(), first_hour: 0, last_hour: last }).into(),
        unobserved_factions: vec![],
        unobserved_wars: vec![],
        identities: TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![Identity { first_hour: 0, last_hour: last, slot: *id, country_id: *id }])).collect(),
    }
}

fn holders(last: i32) -> Holders {
    Holders::new(
        TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![HolderRun { first: 0, last, holder: Holder::Found { slot: *id, id: *id }, starts_segment: true }])).collect(),
        vec![],
    )
}

fn score(country: u32, region: u32, friendly: i64, enemy: i64) -> RaidAirScoreRow {
    RaidAirScoreRow { country, region, friendly, enemy, ..Default::default() }
}

fn scores() -> Vec<RaidAirScoreRow> {
    vec![score(1, 10, 500, 100), score(2, 10, 300, 50), score(3, 10, 200, 600), score(4, 11, 0, 0), score(3, 11, 0, 0)]
}

fn names() -> Names {
    Names { catalogue: Err("no content in this test".into()), regions: Err("no content in this test".into()), warnings: Vec::new() }
}

fn frame(rows: Vec<RaidAirScoreRow>, complete: bool) -> DecodedFrame {
    let mut frame = DecodedFrame { branch: 1, hour: 50, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
    let rows = TableRows::RaidAirScores(rows);
    *frame.tables[TableId::RaidAirScores.index()] = if complete { TableView::Complete(rows) } else { TableView::Truncated { rows, missing: 1 } };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(TAGS.iter().map(|(_, id)| CountryLinkRow { id: *id, canonical_slot: *id }).collect()));
    *frame.tables[TableId::AirRegionWings.index()] = TableView::Complete(TableRows::AirRegionWings(vec![AirRegionWingRow { region: 10, wings: 3 }]));
    frame
}

fn moment_of(frame: &DecodedFrame, params: &AirSuperiorityParams) -> (AirSuperiorityReply, Cells) {
    let context = AirContext { names: names(), index: Ok(Arc::new(index::build(groups(100), 0, 100))) };
    let cells = Cells::default();
    let reply = moment(frame, params, &holders(100), 50, &context, &cells).unwrap();
    (reply, cells)
}

fn params(mode: AirMode) -> AirSuperiorityParams {
    AirSuperiorityParams { branch_id: "1".into(), mode: Some(mode), ..AirSuperiorityParams::default() }
}

fn sides_cell(cell: &AirCell) -> &AirSidesCell {
    match cell {
        AirCell::Sides(cell) => cell,
        other => panic!("not a sides cell: {other:?}"),
    }
}

#[test]
fn a_side_shows_its_leader_s_read_and_a_co_belligerent_member_adds_no_part() {
    let (reply, cells) = moment_of(&frame(scores(), true), &params(AirMode::Sides));
    let keys: Vec<_> = reply.sides.iter().map(|side| side.key.as_str()).collect();
    assert_eq!(keys, ["f:7", "c:GER"], "the sides scoring somewhere; USA scores nowhere");
    let region = reply.regions.iter().find(|region| region.id == 10).unwrap();
    assert_eq!((region.name.as_deref(), region.name_missing.as_deref(), region.active, region.wings), (None, Some("no content in this test"), true, AirWings::Live), "a name not known is named missing");
    let parts: Vec<_> = sides_cell(&region.cell).parts.iter().map(|part| (part.side, part.tag.as_str(), part.live.as_str())).collect();
    assert_eq!(parts, [(0, "ENG", "ENG@10"), (1, "GER", "GER@10")], "FRA is ENG's member: its own read is not a part");
    let quiet = reply.regions.iter().find(|region| region.id == 11).unwrap();
    assert!(!quiet.active && sides_cell(&quiet.cell).parts.is_empty(), "a zero read adds no part");
    assert_eq!(quiet.wings, AirWings::None, "a complete wings table without the region's row: none");
    let live = cells.finish();
    let frame = frame(scores(), true);
    let json = live.json(&live.values(&frame).into_iter().map(GroupState::Values).collect::<Vec<_>>());
    assert_eq!(json["rows"], serde_json::json!(["10", "ENG@10", "GER@10"]));
    let values: Vec<i64> = json["groups"].as_array().unwrap().iter().flat_map(|group| group["values"].as_array().unwrap().iter().filter_map(|value| value.as_i64())).collect();
    assert_eq!(values, [500, 200, 3], "friendly scores and the wings; no enemy column in the sides view");
}

#[test]
fn a_viewpoint_reads_friendly_and_enemy_of_its_side_s_leader_or_its_country() {
    let mut side = params(AirMode::Viewpoint);
    side.viewpoint = Some("f:7".into());
    let (reply, _) = moment_of(&frame(scores(), true), &side);
    let region = reply.regions.iter().find(|region| region.id == 10).unwrap();
    assert_eq!(region.cell, AirCell::View { side: Some(0), leader: Some("ENG".into()), live: Some("ENG@10".into()) });
    assert_eq!(reply.viewpoint, Some(0));
    let quiet = reply.regions.iter().find(|region| region.id == 11).unwrap();
    assert_eq!(quiet.cell, AirCell::View { side: Some(0), leader: Some("ENG".into()), live: None }, "a witnessed owner without the pair: a known zero");
    let mut country = params(AirMode::Viewpoint);
    country.viewpoint = Some("USA".into());
    country.viewpoint_country = Some(true);
    let (reply, cells) = moment_of(&frame(scores(), true), &country);
    let quiet = reply.regions.iter().find(|region| region.id == 11).unwrap();
    assert_eq!(quiet.cell, AirCell::View { side: None, leader: Some("USA".into()), live: Some("USA@11".into()) });
    let live = cells.finish();
    let kinds: BTreeSet<_> = live.json(&[])["kinds"].as_array().unwrap().iter().map(|kind| kind["id"].as_str().unwrap().to_owned()).collect();
    assert!(kinds.contains("enemy"), "a viewpoint shows both scores: {kinds:?}");
    country.viewpoint = Some(dashboard_base::products::Country::Slot(4));
    let (by_slot, _) = moment_of(&frame(scores(), true), &country);
    assert_eq!(by_slot.regions.iter().find(|region| region.id == 11).unwrap().cell, quiet.cell);
    let mut absent = params(AirMode::Viewpoint);
    absent.viewpoint = Some("f:99".into());
    let (reply, _) = moment_of(&frame(scores(), true), &absent);
    assert_eq!(reply.regions[0].cell, AirCell::Reason { reason: "side-not-present".into() });
}

#[test]
fn scores_not_known_are_unknown_sides_never_zero() {
    let (reply, _) = moment_of(&frame(scores(), false), &params(AirMode::Sides));
    let region = reply.regions.iter().find(|region| region.id == 10).unwrap();
    let cell = sides_cell(&region.cell);
    let unknown: Vec<_> = cell.unknown.iter().map(|&index| reply.sides[index as usize].key.as_str()).collect();
    assert_eq!(unknown, ["c:USA"], "USA has no row and the table is cut short: not a zero");
    let mut empty = frame(Vec::new(), false);
    *empty.tables[TableId::RaidAirScores.index()] = TableView::Unavailable { reason: "withheld".into() };
    let (reply, _) = moment_of(&empty, &params(AirMode::Sides));
    assert!(reply.missing.is_some() && reply.regions.is_empty());
}

#[test]
fn only_the_chosen_sides_and_theatres_regions_are_compared() {
    let mut chosen = params(AirMode::Sides);
    chosen.sides = Some(vec!["c:GER".into()]);
    let (reply, _) = moment_of(&frame(scores(), true), &chosen);
    let region = reply.regions.iter().find(|region| region.id == 10).unwrap();
    let cell = sides_cell(&region.cell);
    assert!(cell.filtered);
    assert_eq!(cell.parts.iter().map(|part| part.tag.as_str()).collect::<Vec<_>>(), ["GER"]);
    let mut theatres = params(AirMode::Sides);
    theatres.theatres = Some(vec!["5".into()]);
    let mut frame = frame(scores(), true);
    *frame.tables[TableId::AirTheatres.index()] = TableView::Complete(TableRows::AirTheatres(vec![AirTheatreRow { id: 5, owner: 1, ordinal: 0, name: Vec::new() }]));
    *frame.tables[TableId::AirTheatreGroups.index()] = TableView::Complete(TableRows::AirTheatreGroups(vec![AirTheatreGroupRow { id: 50, theatre: 5, ..Default::default() }]));
    *frame.tables[TableId::AirTheatreMembers.index()] = TableView::Complete(TableRows::AirTheatreMembers(vec![AirTheatreMemberRow { group: 50, wing: 900, ..Default::default() }]));
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![AirWingRow { id: 900, owner: 1, mission_region: Some(10), ..Default::default() }]));
    let (reply, _) = moment_of(&frame, &theatres);
    let reasons: Vec<_> = reply.regions.iter().map(|region| (region.id, matches!(&region.cell, AirCell::Reason { reason } if reason == "outside-theatres"))).collect();
    assert_eq!(reasons, [(10, false), (11, true)]);
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(Vec::new()));
    let (reply, _) = moment_of(&frame, &theatres);
    assert_eq!(reply.regions[0].cell, AirCell::Reason { reason: "theatres-not-recorded".into() }, "a member's wing not recorded: membership not known");
}

#[test]
fn a_tag_s_rows_keep_its_greatest_raw_id_and_a_negative_score_is_not_read() {
    let links = [CountryLinkRow { id: 1, canonical_slot: 1 }, CountryLinkRow { id: 9, canonical_slot: 1 }, CountryLinkRow { id: 3, canonical_slot: 3 }];
    let present: Arc<BTreeSet<String>> = Arc::new(["ENG", "GER"].map(String::from).into());
    let rows = [score(9, 10, 70, 1), score(1, 10, 500, 1), score(3, 10, -1, 0), score(42, 12, 5, 5)];
    let read = hour_scores(&rows, true, Some(&links), &holders(100), &present, 50);
    assert_eq!(read.pairs[&10], [Pair { tag: "ENG".into(), raw: 9, friendly: 70, enemy: 1 }]);
    assert!(read.operands.invalid.contains("GER") && read.operands.unresolved);
    assert_eq!(read.active, [10, 12].into());
}

#[test]
fn sides_are_assigned_once_per_stretch_of_unchanged_groups() {
    let mut groups = groups(100);
    groups.factions.get_mut(&7).unwrap().members = vec![run("ENG", 0, 100), run("FRA", 0, 59)];
    let index = index::build(groups, 0, 100);
    let spans: Vec<_> = index.epochs.iter().map(|epoch| (epoch.first, epoch.last)).collect();
    assert_eq!(spans, [(0, 59), (60, 100)]);
    let late = index.epoch(80).unwrap().sides.as_ref().unwrap();
    let pact = late.iter().find(|side| side.key == "f:7").unwrap();
    assert_eq!(pact.members, ["ENG", "FRA"], "FRA left the faction but fights GER beside it");
    assert_eq!(pact.co_belligerents, 1);
}
