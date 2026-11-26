use super::*;
use crate::script_test_game as game;
use replay_frame::frame::{DecodedSource, RecordedSource};

#[test]
fn decoded_source_keeps_trigger_short_circuits_and_native_scope_order() {
    game::enter();
    let mut frame = frame(
        &[
            (100, 1, "AAA", 1, "AAA", "costume"),
            (200, 2, "GER", 2, "GER", ""),
        ],
        &[100],
    );
    *frame.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "synthetic appearance gap".into(),
    };
    *frame.tables[TableId::Provinces.index()] = TableView::Truncated {
        rows: TableRows::Provinces(vec![
            replay_data::rows::ProvinceRow {
                id: 4,
                owner: 100,
                state: 1,
                ..Default::default()
            },
            replay_data::rows::ProvinceRow {
                id: 4,
                owner: 200,
                state: 2,
                ..Default::default()
            },
        ]),
        missing: 3,
    };
    let source: &dyn DecodedSource = &frame;
    let operands: &dyn RecordedSource = source;
    assert_eq!(
        world::core_gap(operands).as_deref(),
        Some("provinces: read only in part (3 rows missing)")
    );
    with_world_traits(source, "", Err("none"), Err("none"), None, |world| {
        for (body, expected) in [
            ("always=yes", Eval::True),
            ("always=no", Eval::False),
            ("has_country_flag=x", Eval::True),
            ("OR={always=yes has_cosmetic_tag=costume}", Eval::True),
            ("AND={always=no has_cosmetic_tag=costume}", Eval::False),
            ("GER={tag=GER}", Eval::True),
        ] {
            assert_eq!(
                world.evaluate(&items(body), Scopes::of(Scope::Country(1))),
                expected,
                "{body}"
            );
        }
        assert!(matches!(
            world.evaluate(
                &items("has_cosmetic_tag=costume"),
                Scopes::of(Scope::Country(1))
            ),
            Eval::Unknown(_)
        ));
        let last = world.provinces().get(&4).unwrap();
        assert_eq!((last.owner, last.state), (200, 2));
    });
}

#[test]
fn core_admission_keeps_the_original_first_gap_without_acquiring_rows() {
    let mut frame = frame(&[(100, 1, "AAA", 1, "AAA", "")], &[]);
    *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "synthetic links gap".into(),
    };
    *frame.tables[TableId::Countries.index()] = TableView::Truncated {
        rows: TableRows::Countries(Vec::new()),
        missing: 9,
    };
    let source: &dyn RecordedSource = &frame;
    assert_eq!(
        world::core_gap(source).as_deref(),
        Some("country links: not available: synthetic links gap")
    );
}
