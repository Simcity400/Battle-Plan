use super::*;

fn named(key: Key, hour: i32, title: &str) -> (Battle, Result<String, String>) {
    let date = match key { Key::Land(_, date, _) | Key::Naval(_, date) => date, Key::Report(_) => hour };
    (
        Battle { key, date, observed: hour, province: Some(1), port_strike: false, reports: Vec::new(), live_hours: None },
        Ok(title.into()),
    )
}

#[test]
fn repeats_and_singletons_use_the_full_ordinal_sequence() {
    let mut battles: Vec<_> = (1..=111)
        .map(|id| named(Key::Land(id, 12, None), 12, "Battle of Copper Hollow"))
        .collect();
    battles.push(named(Key::Naval(8, 14), 14, "Battle of Glass Shore"));
    let names = number(battles);
    for (id, ordinal) in [(1, "First"), (2, "Second"), (10, "Tenth"), (11, "11th"), (12, "12th"), (13, "13th"), (21, "21st"), (111, "111th")] {
        let key = Key::Land(id, 12, None).id();
        assert_eq!(names.get(&key).unwrap().as_deref(), Ok(format!("{ordinal} Battle of Copper Hollow").as_str()));
    }
    assert_eq!(names.get("combat:8:14").unwrap().as_deref(), Ok("Battle of Glass Shore"));
}

#[test]
fn land_and_naval_share_titles_in_order_and_a_linked_report_does_not_count_twice() {
    let mut live = named(Key::Naval(3, 10), 10, "Battle of Copper Hollow");
    live.0.reports.push(98);
    live.0.live_hours = Some((10, 13));
    let names = number(vec![
        named(Key::Report(8), 10, "Battle of Copper Hollow"),
        live,
        named(Key::Land(7, 10, Some(11)), 11, "Battle of Copper Hollow"),
        named(Key::Land(7, 10, None), 10, "Battle of Copper Hollow"),
        named(Key::Report(9), 15, "Port Strike in Copper Hollow"),
    ]);
    for (key, title) in [
        ("battle:7:10", "First Battle of Copper Hollow"),
        ("battle:7:10:11", "Second Battle of Copper Hollow"),
        ("combat:3:10", "Third Battle of Copper Hollow"),
        ("result:98", "Third Battle of Copper Hollow"),
        ("result:8", "Fourth Battle of Copper Hollow"),
        ("result:9", "Port Strike in Copper Hollow"),
    ] {
        assert_eq!(names.get(key).unwrap().as_deref(), Ok(title));
    }
    assert_eq!(names.naval(3, 12).unwrap().as_deref(), Ok("Third Battle of Copper Hollow"));
    assert!(names.naval(3, 14).is_none());
}

#[test]
fn a_retained_naval_report_keeps_its_ordinal_when_its_date_changes() {
    use dashboard_base::test_fixtures::{source, source_with_content, visual_mods};
    use replay_data::rows::{NavalCombatRow, NavalResultRow, NavalResultSourceRow, TableId, TableRows};
    use replay_frame::frame::TableView;

    let game = visual_mods::Machine::new(&uuid::Uuid::new_v4().to_string());
    game.write("map/strategicregions/1.txt", b"strategic_region={id=1 name=\"COPPER_COAST\" provinces={17 18}}\n");
    game.write("localisation/battles_l_english.yml", "\u{feff}l_english:\n COPPER_COAST:0 \"Copper Coast\"\n NAVAL_PORT_STRIKE_TITLE:0 \"Port Strike in $NAME$\"\n".as_bytes());
    for linked in [false, true] {
        let frames = [99, 100, 101, 102].map(|hour| {
            let mut frame = source(1, hour, None);
            let strike = |id, province, result_hour| NavalResultRow { id, province, result_hour, retire_hour: 200, flag_a: 1, ..Default::default() };
            let results = match hour {
                99 => vec![],
                100 => vec![strike(77, 17, 100)],
                101 => vec![strike(77, 18, 101)],
                _ => vec![strike(77, 18, 101), strike(78, 17, 102)],
            };
            let combats = (linked && hour == 99).then(|| NavalCombatRow { id: 7, province: 17, ..Default::default() });
            let sources = (linked && hour == 100).then(|| NavalResultSourceRow {
                id: 1, game_hour: 100, result: 77, combat: 7, combat_created_hour: Some(99), ..Default::default()
            });
            for (table, rows) in [
                (TableId::NavalResults, TableRows::NavalResults(results)),
                (TableId::NavalCombats, TableRows::NavalCombats(combats.into_iter().collect())),
                (TableId::NavalResultSources, TableRows::NavalResultSources(sources.into_iter().collect())),
            ] {
                *frame.tables[table.index()] = TableView::Complete(rows);
            }
            frame
        });
        let (source, load) = source_with_content(frames.into(), &game.root);
        let names = source.in_scope(|| read(&source, 1, &load, &false.into())).unwrap();
        let first = "First Port Strike in Copper Coast";
        assert_eq!(names.get("result:77").unwrap().as_deref(), Ok(first));
        assert_eq!(names.get("result:78").unwrap().as_deref(), Ok("Second Port Strike in Copper Coast"));
        if linked {
            assert_eq!(names.get("combat:7:99").unwrap().as_deref(), Ok(first));
            assert_eq!(names.naval(7, 99).unwrap().as_deref(), Ok(first));
        }
    }
}

#[test]
fn a_gap_keeps_the_sequence_and_each_lineage_counts_its_own_battles() {
    let first = || named(Key::Land(1, 10, None), 10, "Battle of Copper Hollow");
    let rename = named(Key::Land(2, 11, None), 11, "Battle of Glass Shore");
    let mut unknown = named(Key::Land(3, 29, None), 29, "Battle of Copper Hollow");
    unknown.1 = Err("Province names were truncated".into());
    let after_gap = named(Key::Land(4, 30, None), 30, "Battle of Copper Hollow");
    let main = number(vec![first(), rename, unknown, after_gap]);
    let fork = number(vec![first(), named(Key::Land(9, 32, None), 32, "Battle of Glass Shore")]);
    assert_eq!(main.get("battle:1:10").unwrap().as_deref(), Ok("First Battle of Copper Hollow"));
    assert_eq!(main.get("battle:4:30").unwrap().as_deref(), Ok("Second Battle of Copper Hollow"));
    assert_eq!(main.get("battle:2:11").unwrap().as_deref(), Ok("Battle of Glass Shore"));
    assert_eq!(main.get("battle:3:29").unwrap().as_ref().unwrap_err().as_ref(), "Province names were truncated");
    assert_eq!(fork.get("battle:1:10").unwrap().as_deref(), Ok("Battle of Copper Hollow"));
    assert!(fork.get("battle:4:30").is_none());
}
