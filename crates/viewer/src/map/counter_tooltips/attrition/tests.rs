use super::*;
use replay::{
    frame::TableView,
    rows::{
        LexicalTokenRow, ModifierDefinitionRow, ProvinceRow, Recorded, StateFlagRow, TableId,
        TableRows,
    },
};

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let table = rows.table();
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn status(frame: &mut DecodedFrame) -> &mut DivisionStatusRow {
    let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *frame.tables[TableId::DivisionStatus.index()]
    else {
        unreachable!("the fixture holds complete status rows");
    };
    &mut rows[0]
}

/// Division 1 at `province` with a cached attrition total of 2.5% and no
/// upkeep (vanilla's UNIT_UPKEEP_ATTRITION is zero), on `path`, in full
/// supply: a whole received ratio with a positive maximum grace.
fn frame(province: u32, path_total: u32, path: Vec<u32>) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 0,
        metrics: Default::default(),
        tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
    };
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: 1,
            province,
            logical_owner: 100,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: 1,
            attrition_value: 2_500,
            attrition_threshold: 0,
            supply_ratio: 100_000,
            supply_grace_maximum: Recorded::Value(100_000),
            supply_grace_remaining: Recorded::Value(0),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionMovement(vec![DivisionMovementRow {
            division: 1,
            path_total,
            path,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::Provinces(
            [(5, 3), (7, 0), (8, 0)]
                .into_iter()
                .map(|(id, state)| ProvinceRow {
                    id,
                    state,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::StateFlags(vec![StateFlagRow {
            state: 3,
            border_conflict_active: Recorded::Value(0),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::ModifierDefinitions(vec![ModifierDefinitionRow {
            index: engine::modifier_keys::ATTRITION,
            token: 9,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 9,
            literal: b"attrition".to_vec(),
        }]),
    );
    frame
}

fn content() -> Content {
    Content {
        training: 5_000,
        border_factor: 10_000,
        supply_threshold: 35_000,
        terrains: vec![
            Terrain {
                name: b"plains".to_vec(),
                attrition: None,
            },
            Terrain {
                name: b"mountain".to_vec(),
                attrition: Some(30_000),
            },
        ],
    }
}

fn render_with(frame: &DecodedFrame, lookups: Result<&Lookups<'_>, String>) -> Tooltip {
    let loc: Localisation = [
        ("ATTRITION_TOTAL", "total $TOT|%2$\n"),
        ("ATTRITION_UPKEEP", "upkeep $VALUE|%2$"),
        ("ATTRITION_TERRAIN", "terrain $TERR$ $ATT|%2$"),
        ("TRAINING_ATTRITION", "train $VALUE$"),
        ("BORDER_WAR_HEADER", "border war"),
        ("BORDER_WAR_ATTRITION", "border $VAL|%1$"),
        ("mountain", "Mountain"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    let vision = Vision::sharing(1, []);
    let names = installed_content::division_names::Database::default();
    let units = Err("unused".into());
    let context = Context {
        nato: false,
        content: None,
        dig_in_factor: Err("unused".into()),
        loc: &loc,
        recorded_language: "english",
        icons: &crate::combat::binding::NoIcons,
        names: &names,
        units: &units,
        supply_grace: Ok(72),
        comms_max_distance: Err("unused".into()),
        frame,
        vision: &vision,
        links: Ok(&[]),
    };
    let mut out = Tooltip::default();
    append_with(&context, lookups, 1, &mut out);
    out
}

fn render(frame: &DecodedFrame) -> Tooltip {
    render_with(frame, Err("the counter content is unavailable".into()))
}

/// With every lookup available: province 7 is a mountain (ordinal 2), 8
/// plains, anything else the null terrain; state 3's border war gives the
/// unit's logical owner (100) a 40% enemy share.
fn rendered(frame: &DecodedFrame) -> Tooltip {
    let content = content();
    let terrain = |province: u32| -> Result<i32, String> {
        Ok(match province {
            7 => 2,
            8 => 1,
            _ => 0,
        })
    };
    let border = |country: u32, state: u32| -> Result<i64, String> {
        assert_eq!((country, state), (100, 3));
        Ok(40_000)
    };
    let lookups = Lookups {
        content: &content,
        terrain: &terrain,
        border: &border,
    };
    render_with(frame, Ok(&lookups))
}

fn refused(out: &Tooltip, line: &str) -> bool {
    out.refused
        .keys()
        .any(|key| key.starts_with(&format!("counter {line}:")))
}

/// A moving unit's upkeep and terrain lines: the route's first province is
/// the mountain, whose
/// `attrition` adds to the upkeep; the location's state has no line.
/// Retired-capture builds showed the cached total alone.
#[test]
fn a_moving_unit_shows_its_upkeep_and_route_terrain() {
    let mut frame = frame(5, 2, vec![7, 8]);
    status(&mut frame).attrition_threshold = 10_000;
    let out = rendered(&frame);
    assert_eq!(
        out.title,
        "total 2.50%\nupkeep 10.00%\nterrain Mountain 40.00%"
    );
    for line in [
        "attrition equipment",
        "attrition upkeep modifiers",
        "attrition weather",
        "attrition country modifiers",
    ] {
        assert!(refused(&out, line), "{line}: {:?}", out.refused);
    }
    for line in ["attrition state", "attrition supply", "attrition terrain"] {
        assert!(!refused(&out, line), "{line}: {:?}", out.refused);
    }
    // Plains has no attrition entry: upkeep alone is its nonzero sum.
    let mut plains = frame.clone();
    put(
        &mut plains,
        TableRows::DivisionMovement(vec![DivisionMovementRow {
            division: 1,
            path_total: 1,
            path: vec![8],
            ..Default::default()
        }]),
    );
    assert_eq!(
        rendered(&plains).title,
        "total 2.50%\nupkeep 10.00%\nterrain plains 10.00%"
    );
    // Strategic redeployment and the null terrain skip the branch.
    let mut redeploying = frame.clone();
    let TableView::Complete(TableRows::DivisionMovement(rows)) =
        &mut *redeploying.tables[TableId::DivisionMovement.index()]
    else {
        unreachable!("the fixture holds complete movement rows");
    };
    rows[0].strategic_redeployment = 1;
    assert_eq!(rendered(&redeploying).title, "total 2.50%\nupkeep 10.00%");
}

/// A stationary exercising unit in a border-war state: the training line,
/// then the border war heading and its share times the factor; the state
/// term itself is refused by name.
#[test]
fn training_and_border_war_lines_follow_the_native_order() {
    let mut frame = frame(5, 0, vec![]);
    status(&mut frame).exercising = 1;
    put(
        &mut frame,
        TableRows::StateFlags(vec![StateFlagRow {
            state: 3,
            border_conflict_active: Recorded::Value(1),
            ..Default::default()
        }]),
    );
    let out = rendered(&frame);
    assert_eq!(
        out.title,
        "total 2.50%\ntrain \u{11}R+5.0%\u{11}!\nborder war\nborder 4.0%"
    );
    assert!(refused(&out, "attrition state"));
    assert!(!refused(&out, "attrition weather"));
    assert!(!refused(&out, "attrition training"));
}

/// The supply penalty is zero whenever the threshold-normalized
/// availability is whole; the stepped recorded values bound it from below.
/// Below the threshold the out-of-supply days decide, which are not
/// recorded.
#[test]
fn supply_is_absent_only_when_the_recorded_supply_proves_it() {
    let row = |ratio: i64, maximum: i64, remaining: i64| DivisionStatusRow {
        supply_ratio: ratio,
        supply_grace_maximum: Recorded::Value(maximum),
        supply_grace_remaining: Recorded::Value(remaining),
        ..Default::default()
    };
    assert_eq!(supply_absent(&row(35_000, 1, 0), Ok(72), 35_000), Ok(true));
    assert_eq!(supply_absent(&row(34_999, 1, 0), Ok(72), 35_000), Ok(false));
    // A full remaining grace is whole availability.
    assert_eq!(
        supply_absent(&row(0, 1, 72 * 100_000), Ok(72), 35_000),
        Ok(true)
    );
    // No maximum grace: nothing is available.
    assert_eq!(
        supply_absent(&row(100_000, 0, 0), Ok(72), 35_000),
        Ok(false)
    );
    // A zero threshold normalizes to whole.
    assert_eq!(supply_absent(&row(0, 0, 0), Ok(72), 0), Ok(true));
    let mut unrecorded = row(100_000, 1, 0);
    unrecorded.supply_grace_remaining = Recorded::NotRecorded;
    assert!(supply_absent(&unrecorded, Ok(72), 35_000).is_err());
    let mut frame = frame(5, 0, vec![]);
    status(&mut frame).supply_ratio = 10_000;
    assert!(refused(&rendered(&frame), "attrition supply"));
}

/// Without the content, the total stands and every line that needs it is
/// refused by name.
#[test]
fn missing_content_refuses_the_lines_it_feeds() {
    let mut frame = frame(5, 2, vec![7]);
    status(&mut frame).exercising = 0;
    let out = render(&frame);
    assert_eq!(out.title, "total 2.50%");
    assert!(refused(&out, "attrition terrain"));
    assert!(refused(&out, "attrition supply"));
}

/// The game writes nothing without a route province or a location; a
/// route province alone still words the total.
#[test]
fn the_total_needs_a_location_or_a_route_province() {
    let out = render(&frame(0, 0, vec![]));
    assert!(out.title.is_empty());
    assert!(out.refused.is_empty());
    assert_eq!(render(&frame(0, 1, vec![4])).title, "total 2.50%");
    let clipped = render(&frame(0, 2, vec![]));
    assert!(clipped.title.is_empty());
    assert_eq!(clipped.refused.len(), 1);
    assert!(
        clipped
            .refused
            .keys()
            .all(|key| key.starts_with("counter attrition total:"))
    );
}

#[test]
fn a_missing_status_row_refuses_the_total_by_name() {
    let mut frame = frame(5, 0, vec![]);
    put(&mut frame, TableRows::DivisionStatus(vec![]));
    let out = render(&frame);
    assert!(out.title.is_empty());
    assert_eq!(
        out.refused
            .get("counter attrition total: DivisionStatus division 1 is absent")
            .copied(),
        Some(1)
    );
}
