use super::*;
use replay_data::{
    rows::CountryLinkRow, rows::LandCombatExileInputs, rows::TableId, rows::TableRows,
};
use replay_frame::frame::TableView;

const BONUSES: [i64; 2] = [10_000, 10_000];

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 1,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "missing".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        DivisionRow {
            id: 2,
            roster_owner: 6,
            expeditionary_owner: 8,
            logical_owner: 9,
            ..Default::default()
        },
        DivisionRow {
            id: 3,
            roster_owner: 8,
            expeditionary_owner: u32::MAX,
            ..Default::default()
        },
        DivisionRow {
            id: 4,
            roster_owner: 6,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(5, 1), (7, 1), (8, 2), (6, 3), (9, 4)]
            .into_iter()
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
    frame
}

fn exiled(origin_type: i32, country: i32) -> LandCombatDivisionRow {
    LandCombatDivisionRow {
        division: 1,
        exile: Some(LandCombatExileInputs {
            origin_type,
            country,
            country_fallback: 5,
            location_cores: vec![7],
            occupiers: vec![8],
        }),
        own: Some(replay_data::rows::LandCombatMemberValues {
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn detail(opponents: Vec<u64>) -> LandCombatDetailsRow {
    let mut detail = LandCombatDetailsRow::default();
    detail.sides[1].committed = opponents;
    detail.sides[1].reserves = vec![999];
    detail
}

#[test]
fn a_host_or_subject_template_has_no_badges_and_flies_the_owner_flag() {
    let frame = frame();
    let inputs = Inputs::read(&frame).unwrap();
    let row = LandCombatDivisionRow {
        division: 2,
        own: Some(replay_data::rows::LandCombatMemberValues {
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        inputs.frames(&row, &detail(vec![3]), &BONUSES),
        [Ok(false); 2]
    );
    assert_eq!(inputs.entry_owner(&row), Ok(8));
    let reserve = LandCombatDivisionRow {
        division: 3,
        own: Some(replay_data::rows::LandCombatMemberValues {
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(inputs.entry_owner(&reserve), Ok(8));
}

#[test]
fn an_exile_template_flies_its_primary_country_or_the_fallback() {
    let frame = frame();
    let inputs = Inputs::read(&frame).unwrap();
    assert_eq!(inputs.entry_owner(&exiled(EXILE_ORIGIN, 9)), Ok(9));
    assert_eq!(inputs.entry_owner(&exiled(EXILE_ORIGIN, 0)), Ok(5));
    assert_eq!(inputs.entry_owner(&exiled(EXILE_ORIGIN, -3)), Ok(5));
}

#[test]
fn the_occupier_badge_needs_a_committed_occupying_opponent_and_a_bonus() {
    let frame = frame();
    let inputs = Inputs::read(&frame).unwrap();
    let row = exiled(EXILE_ORIGIN, 0);
    assert_eq!(
        inputs.frames(&row, &detail(vec![4, 2]), &BONUSES)[0],
        Ok(true)
    );
    assert_eq!(
        inputs.occupiers(&row, &detail(vec![2, 3, 4]), &BONUSES),
        Ok(2)
    );
    assert_eq!(
        inputs.frames(&row, &detail(vec![4]), &BONUSES)[0],
        Ok(false)
    );
    assert_eq!(inputs.frames(&row, &detail(vec![]), &BONUSES)[0], Ok(false));
    let unbonused = [0, 10_000];
    assert_eq!(
        inputs.frames(&row, &detail(vec![2]), &unbonused)[0],
        Ok(false)
    );
    let mut defending = row.clone();
    defending.side = 1;
    let mut swapped = detail(vec![]);
    swapped.sides[0].committed = vec![2];
    assert_eq!(inputs.frames(&defending, &swapped, &unbonused)[0], Ok(true));
    assert_eq!(
        inputs.frames(&row, &detail(vec![77]), &BONUSES)[0],
        Err(Refused::Division {
            division: 77,
            gap: Gap::Absent
        })
    );
    assert_eq!(
        inputs.frames(&row, &detail(vec![2, 77]), &BONUSES)[0],
        Ok(true)
    );
    assert_eq!(
        inputs.occupiers(&row, &detail(vec![2, 77]), &BONUSES),
        Err(Refused::Division {
            division: 77,
            gap: Gap::Absent
        })
    );
}

#[test]
fn a_later_occupying_opponent_decides_past_an_unknown_one() {
    let frame = frame();
    let inputs = Inputs::read(&frame).unwrap();
    let row = exiled(EXILE_ORIGIN, 0);
    assert_eq!(
        inputs.frames(&row, &detail(vec![77, 4, 2]), &BONUSES)[0],
        Ok(true)
    );
    let unknown = Err(Refused::Division {
        division: 77,
        gap: Gap::Absent,
    });
    assert_eq!(
        inputs.frames(&row, &detail(vec![77, 4]), &BONUSES)[0],
        unknown
    );
    let all = inputs.frames(&row, &detail(vec![77, 78]), &BONUSES)[0];
    assert_eq!(all, unknown);
    assert_eq!(
        all.unwrap_err().to_string(),
        "the Divisions table has no row for division 77"
    );
    let mut unlinked = row.clone();
    unlinked.exile.as_mut().unwrap().occupiers = vec![99];
    assert_eq!(
        inputs.frames(&unlinked, &detail(vec![4]), &BONUSES)[0],
        Err(Refused::CountryLink {
            country: 99,
            gap: Gap::Absent
        })
    );
    unlinked.exile.as_mut().unwrap().occupiers = vec![99, 8];
    assert_eq!(
        inputs.frames(&unlinked, &detail(vec![4, 2]), &BONUSES)[0],
        Ok(true)
    );
}

#[test]
fn the_core_badge_compares_the_location_cores_through_country_links() {
    let frame = frame();
    let inputs = Inputs::read(&frame).unwrap();
    assert_eq!(
        inputs.frames(&exiled(EXILE_ORIGIN, 0), &detail(vec![]), &BONUSES)[1],
        Ok(true)
    );
    let mut elsewhere = exiled(EXILE_ORIGIN, 0);
    elsewhere.exile.as_mut().unwrap().location_cores = vec![0, 6];
    assert_eq!(
        inputs.frames(&elsewhere, &detail(vec![]), &BONUSES)[1],
        Ok(false)
    );
    elsewhere.exile.as_mut().unwrap().location_cores = vec![99];
    let refused = inputs.frames(&elsewhere, &detail(vec![]), &BONUSES)[1];
    assert_eq!(
        refused,
        Err(Refused::CountryLink {
            country: 99,
            gap: Gap::Absent
        })
    );
    assert_eq!(
        refused.unwrap_err().to_string(),
        "the CountryLinks table has no row for country 99"
    );
    assert_eq!(inputs.contains(&[0], 0), Ok(true));
    assert_eq!(inputs.contains(&[99, 7], 5), Ok(true));
}

#[test]
fn only_origin_two_is_exiled() {
    let frame = frame();
    let inputs = Inputs::read(&frame).unwrap();
    for origin in [0, 1, 3, 5, -1, i32::MIN, i32::MAX] {
        let mut row = exiled(origin, 9);
        row.division = 2;
        assert_eq!(inputs.entry_owner(&row), Ok(8), "origin {origin}");
        assert_eq!(
            inputs.frames(&row, &detail(vec![2]), &BONUSES),
            [Ok(false); 2],
            "origin {origin}"
        );
    }
}

#[test]
fn refusals_name_the_table_key_and_gap() {
    let mut frame = frame();
    *frame.tables[TableId::Divisions.index()] = TableView::Truncated {
        rows: TableRows::Divisions(Vec::new()),
        missing: 3,
    };
    *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "country registry unreadable".into(),
    };
    let inputs = Inputs::read(&frame).unwrap();
    let owner = inputs.entry_owner(&LandCombatDivisionRow {
        division: 2,
        ..Default::default()
    });
    assert_eq!(
        owner.unwrap_err().to_string(),
        "the truncated Divisions table did not reach division 2"
    );
    let core = inputs.frames(&exiled(EXILE_ORIGIN, 0), &detail(vec![]), &BONUSES)[1];
    assert_eq!(
        core.unwrap_err().to_string(),
        "the CountryLinks table is unavailable (country registry unreadable), so country 7 is unknown"
    );
}
