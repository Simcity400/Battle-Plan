use super::*;

const DAY: i32 = DAY_HOURS;

#[test]
fn days_are_sampled_at_their_last_known_hour_and_fold_into_whole_day_columns() {
    let known = [(0, 0, 2 * DAY + 10), (0, 4 * DAY, 5 * DAY - 1)];
    let reads = [(0, 0), (DAY + 5, DAY + 5), (2 * DAY + 5, 2 * DAY + 5), (4 * DAY + 5, 4 * DAY + 5)];
    let days = days(0, 5 * DAY - 1, &known, &reads);
    assert_eq!(days.iter().map(|day| day.at.map(|(_, at)| at)).collect::<Vec<_>>(), [Some(DAY - 1), Some(2 * DAY - 1), Some(2 * DAY + 10), None, Some(5 * DAY - 1)]);
    assert_eq!(days.iter().map(|day| day.read).collect::<Vec<_>>(), [Some(0), Some(DAY + 5), Some(2 * DAY + 5), None, Some(4 * DAY + 5)]);
    let region = |values: [Option<i128>; 5]| -> Daily { [(7, values.to_vec())].into() };
    let measures = vec![("light_rain".to_owned(), region([Some(1), Some(0), Some(1), None, Some(0)])), ("heavy_rain".to_owned(), region([Some(0), Some(0), Some(0), None, Some(0)]))];
    let columns = fold(&days, 3, &measures, true).unwrap();
    assert_eq!(columns.days, 2);
    assert_eq!(columns.columns.iter().map(|column| (column.first_hour, column.last_hour, column.recorded)).collect::<Vec<_>>(), [(0, 2 * DAY - 1, 2), (2 * DAY, 4 * DAY - 1, 1), (4 * DAY, 5 * DAY - 1, 1)]);
    assert_eq!(columns.columns[1].sampled_hour, Some(2 * DAY + 10));
    let rain = &columns.regions[0];
    assert_eq!(rain.measures.len(), 1, "a flag set on no sampled day is left out");
    assert_eq!(rain.measures[0].days, [1, 1, 0]);
    assert_eq!(rain.observed.as_deref(), Some(&[2, 1, 1][..]));
    assert_eq!(rain.clear.as_deref(), Some(&[1, 0, 1][..]), "a day not read is never clear");
    let modifier = fold(&days, 5, &[("mod:air_attack".to_owned(), region([Some(-3), None, Some(5), None, Some(2)]))], false).unwrap();
    assert_eq!(modifier.regions[0].observed, None);
    assert_eq!(modifier.regions[0].measures[0].values.as_ref().unwrap()[0], Some(["-3".to_owned(), "-3".to_owned(), "-3".to_owned()]));
    assert_eq!(modifier.regions[0].measures[0].values.as_ref().unwrap()[1], None, "a day without a value is none, never zero");
    let wide = fold(&days, 2, &[("mod:air_attack".to_owned(), region([Some(-3), None, Some(5), None, Some(2)]))], false).unwrap();
    assert_eq!(wide.regions[0].measures[0].days, [2, 1]);
    assert_eq!(wide.regions[0].measures[0].values.as_ref().unwrap()[0], Some(["-3".to_owned(), "2".to_owned(), "5".to_owned()]));
}

#[test]
fn modifiers_follow_their_witness_and_a_mean_needs_every_province() {
    let mut row = WeatherRegionRow { id: 7, air_detection: Recorded::Value(Some(None)), map_modifiers_known: Recorded::Value(false), naval_detection: Recorded::Value(Some(4)), air_defence: Recorded::NotRecorded, ..Default::default() };
    let values = modifiers(&row);
    assert_eq!(values[0], Some(0));
    assert_eq!(values[1], None, "map modifiers not known: no naval detection");
    assert!(values[4..].iter().all(Option::is_none), "no combat witness");
    row.map_modifiers_known = Recorded::Value(true);
    row.air_attack = Recorded::Value(None);
    assert_eq!(&modifiers(&row)[1..4], &[Some(4), Some(0), None]);
    let state = |temperature| State { temperature, ..Default::default() };
    let states = [state(100), state(-50), state(31)];
    assert_eq!(mean(&[0, 1, 2], &states), Some(27));
    assert_eq!(mean(&[0, 1], &states), Some(25));
    assert_eq!(mean(&[0, 9], &states), None, "a province without a state leaves no mean");
    assert_eq!(mean(&[], &states), None);
}

#[test]
fn weather_acceptance_current_day_at_midnight_terminal_split_since_and_scoped_extrema() {
    let known = [(0, 2160 - 4 * DAY, 2160 + 2 * DAY)];
    let reads = [(2160 - 4 * DAY, 2160 + 2 * DAY)];
    let midnight = days(2160 - 4 * DAY, 2160, &known, &reads);
    assert_eq!(midnight.len(), 5, "midnight hour 2160 opens its own day");
    assert_eq!(midnight.last().unwrap().first, 2160);
    assert_eq!(midnight.last().unwrap().last, 2160);
    let next = days(2160 - 4 * DAY, 2161, &known, &reads);
    assert_eq!(next.last().unwrap().last, 2161);
    assert_eq!(next.last().unwrap().at, Some((0, 2161)));
    assert_eq!(midnight.last().unwrap().at, Some((0, 2160)), "no hour after 2160 is sampled");

    let rain: Daily = [(1, vec![Some(1), Some(1), Some(1), Some(1), Some(0)]), (2, vec![Some(0), Some(0), Some(1), Some(1), Some(1)])].into();
    let measures = vec![("light_rain".to_owned(), rain)];
    let folded = fold_preserving_terminal(&midnight, 2, &measures, true).unwrap();
    let last_col = folded.columns.last().unwrap();
    assert_eq!((last_col.first_hour, last_col.last_hour), (2160, 2160), "current day at midnight is never folded with yesterday");
    let r1 = folded.regions.iter().find(|r| r.id == "1").unwrap();
    assert_eq!(*r1.clear.as_ref().unwrap().last().unwrap(), 1, "clear midnight stays clear in terminal slot");

    let choices = vec![Choice { id: "light_rain".into(), name: "Light rain".into() }];
    let r1_readout = region_readout(1, &midnight, &measures, Show::Conditions, &choices, "available");
    assert_eq!(
        r1_readout,
        WeatherReadout::Condition { id: "clear".into(), name: "Clear".into(), since_hour: Some(2160), since_bounded: true, as_of_hour: 2160 }
    );
    let r2_readout = region_readout(2, &midnight, &measures, Show::Conditions, &choices, "available");
    assert_eq!(
        r2_readout,
        WeatherReadout::Condition { id: "light_rain".into(), name: "Light rain".into(), since_hour: Some(2160 - DAY - 1), since_bounded: true, as_of_hour: 2160 }
    );

    let gapped = days(2160 - 2 * DAY, 2160, &[(0, 2160 - 2 * DAY, 2160 - 1)], &[(2160 - 2 * DAY, 2160 - 1)]);
    assert!(matches!(region_readout(1, &gapped, &measures, Show::Conditions, &choices, "available"), WeatherReadout::NotRecorded { .. }));

    let temps: Daily = [(1, vec![Some(600_000)]), (2, vec![Some(9_000_000)])].into();
    let temp_measures = vec![("temp".to_owned(), temps)];
    assert_eq!(visible_extrema(&[1], &temp_measures, Show::Temperature), Some(["600000".into(), "600000".into()]), "unselected region 2 never pollutes extrema");

    let rank = |days: &[Day], measures: &[(String, Daily)], show, whole| rank_at_hour(&[1, 2], days, measures, show, whole);
    assert_eq!(rank(&midnight, &measures, Show::Conditions, true), HourRank { ranked: vec![2], missing: None }, "the hour's reading decides: region 1 rained all week but is clear at the hour");
    let snow: Daily = [(1, vec![Some(0), Some(0), Some(0), Some(0), Some(1)])].into();
    let both = vec![measures[0].clone(), ("blizzard".to_owned(), snow)];
    assert_eq!(rank(&midnight, &both, Show::Conditions, true).ranked, vec![1, 2], "a blizzard outranks light rain");
    let unread: Daily = [(1, vec![Some(1), Some(1), Some(1), Some(1), None]), (2, vec![Some(1); 5])].into();
    let part = rank(&midnight, &[("light_rain".to_owned(), unread)], Show::Conditions, true);
    assert_eq!((part.ranked, part.missing.is_some()), (vec![2], true), "a region not read at the hour leaves the rank unproven; the witnessed active region stays known");
    assert!(rank(&midnight, &measures, Show::Conditions, false).missing.is_some(), "a part read ranks nothing, even when every region it reached is known");
    assert!(rank(&gapped, &measures, Show::Conditions, true).missing.is_some(), "no reading at the hour ranks nothing");
    let earlier = days(2160 - 4 * DAY, 2165, &[(0, 2160 - 4 * DAY, 2163)], &[(2160 - 4 * DAY, 2163)]);
    assert!(rank(&earlier, &measures, Show::Conditions, true).missing.is_some(), "a reading earlier in the day than the hour does not rank the hour");
    assert_eq!(rank(&midnight, &temp_measures_at(&midnight), Show::Temperature, true).ranked, vec![2, 1], "the largest magnitude at the hour first");
    let legend = visible_legend(&[1], &midnight[4..5], &[("light_rain".to_owned(), [(1, vec![Some(0)]), (2, vec![Some(1)])].into())], Show::Conditions, &choices);
    assert_eq!(legend.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["clear"], "off-chart region 2 rain does not enter legend");
}

fn temp_measures_at(days: &[Day]) -> Vec<(String, Daily)> {
    let at = |value| { let mut values = vec![None; days.len()]; values[days.len() - 1] = Some(value); values };
    vec![("temp".to_owned(), [(1, at(-200_000)), (2, at(900_000))].into())]
}

#[test]
fn unavailable_and_truncated_weather_days_carry_their_reasons() {
    use dashboard_base::test_fixtures::{played, recorded_mod_defines, source, source_with_mods};
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;

    let (machine, cat_source, load) = recorded_mod_defines("", "", &[]);
    let cat = catalogue::catalogue(&cat_source, load.content().unwrap()).unwrap();
    let weather = Weather {
        members: BTreeMap::new(),
        names: [(1, Ok("Region One".into())), (2, Ok("Region Two".into())), (3, Ok("Region Three".into()))].into(),
        pass: Err("no pass".into()),
        cat,
        warnings: Vec::new(),
    };
    let make_row = |id: u32, rain: u8, det: i64| WeatherRegionRow {
        id,
        light_rain: rain,
        air_detection: Recorded::Value(Some(Some(det))),
        ..Default::default()
    };
    let mut frames = Vec::new();
    for hour in (0..4 * DAY).filter(|hour| !(10..12).contains(hour)) {
        let mut frame = source(1, hour, Some(i64::from(hour)));
        *frame.tables[TableId::WeatherRegions.index()] = if hour < 5 {
            TableView::Complete(TableRows::WeatherRegions(vec![make_row(1, 0, 1), make_row(2, 0, 2), make_row(3, 0, 3)]))
        } else if hour < DAY {
            TableView::Unavailable { reason: "mid-day outage".into() }
        } else if hour < 2 * DAY {
            TableView::Unavailable { reason: "weather sensor failed".into() }
        } else if hour < 3 * DAY {
            TableView::Truncated {
                rows: TableRows::WeatherRegions(vec![
                    make_row(1, 1, 5),
                    WeatherRegionRow { id: 3, light_rain: 0, air_detection: Recorded::Value(None), ..Default::default() },
                ]),
                missing: 2,
            }
        } else {
            TableView::NotRecorded { mode: replay_data::scope::CaptureMode::Full }
        };
        *frame.tables[TableId::WeatherProvinces.index()] = if (DAY..2 * DAY).contains(&hour) {
            TableView::Unavailable { reason: "provinces sensor failed".into() }
        } else {
            TableView::Complete(TableRows::WeatherProvinces(Vec::new()))
        };
        frames.push(frame);
    }
    let (source, load) = source_with_mods(frames, &machine.root, &["mod/probe.mod"]);
    let source = Arc::new(source);
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let params = |show, last_hour| RegionalWeatherParams {
        branch_id: "1".into(),
        first_hour: Some(0),
        window_days: None,
        last_hour,
        width: 10,
        show,
        modifier: Some("air_detection".into()),
        regions: None,
        pinned_regions: Some(vec![1, 2, 3]),
        hidden_regions: None,
        top: Some(5),
    };

    for show in [Show::Conditions, Show::Modifier] {
        let before_outage = history(&query, &params(show, 0), &weather, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(before_outage.valid_through, 4, "{show:?}: reply must expire before the status change at hour 5");

        let unavailable = history(&query, &params(show, 2 * DAY - 1), &weather, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(unavailable.columns.columns.last().unwrap().recorded, 0);
        assert!(unavailable.warnings.iter().any(|why| why.contains("weather sensor failed")), "{show:?}: {:?}", unavailable.warnings);
        assert!(unavailable.rank_missing.as_deref().is_some_and(|why| why.contains("weather sensor failed")), "{show:?}: {:?}", unavailable.rank_missing);
        let r1 = unavailable.columns.regions.iter().find(|region| region.id == "1").unwrap();
        assert!(matches!(&r1.readout, Some(WeatherReadout::NotRecorded { reason }) if reason.contains("weather sensor failed")), "{show:?}: {:?}", r1.readout);

        let truncated = history(&query, &params(show, 3 * DAY - 1), &weather, None, &AtomicBool::new(false)).unwrap();
        assert!(truncated.warnings.iter().any(|why| why.contains("2 rows not read")), "{show:?}: {:?}", truncated.warnings);
        assert!(truncated.warnings.iter().any(|why| why.contains("weather sensor failed")), "{show:?}: {:?}", truncated.warnings);
        assert!(truncated.rank_missing.as_deref().is_some_and(|why| why.contains("2 rows not read")), "{show:?}: {:?}", truncated.rank_missing);
        let r1 = truncated.columns.regions.iter().find(|region| region.id == "1").unwrap();
        match show {
            Show::Conditions => assert!(matches!(&r1.readout, Some(WeatherReadout::Condition { id, .. }) if id == "light_rain"), "{:?}", r1.readout),
            Show::Modifier => assert!(matches!(&r1.readout, Some(WeatherReadout::Value { value, .. }) if value == "5"), "{:?}", r1.readout),
            Show::Temperature => unreachable!(),
        }
        let r2 = truncated.columns.regions.iter().find(|region| region.id == "2").unwrap();
        assert!(matches!(&r2.readout, Some(WeatherReadout::NotRecorded { reason }) if reason.contains("2 rows not read")), "{show:?}: {:?}", r2.readout);
        if show == Show::Modifier {
            let r3 = truncated.columns.regions.iter().find(|region| region.id == "3").unwrap();
            assert!(
                matches!(&r3.readout, Some(WeatherReadout::NotRecorded { reason }) if reason == "Weather reading was not recorded at this hour"),
                "witnessed row with unrecorded modifier must keep its field-level reason: {:?}",
                r3.readout
            );
        }

        let not_recorded = history(&query, &params(show, 3 * DAY), &weather, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(not_recorded.valid_through, 3 * DAY, "{show:?}: hour-stamped NotRecorded reason must not be reused on later hours");
        let r1 = not_recorded.columns.regions.iter().find(|region| region.id == "1").unwrap();
        assert!(matches!(&r1.readout, Some(WeatherReadout::NotRecorded { reason }) if reason.contains(&format!("hour {}", 3 * DAY))), "{show:?}: {:?}", r1.readout);
    }

    let temp_unavailable = history(&query, &params(Show::Temperature, 2 * DAY - 1), &weather, None, &AtomicBool::new(false)).unwrap();
    let r1 = temp_unavailable.columns.regions.iter().find(|region| region.id == "1").unwrap();
    assert!(
        matches!(&r1.readout, Some(WeatherReadout::NotRecorded { reason }) if reason.contains("provinces sensor failed")),
        "unsampled WeatherProvinces day should preserve the recorded table reason even when temperature content fails: {:?}",
        r1.readout
    );

    let request = serde_json::json!({"panel": "regional-weather", "branchId": "1", "firstHour": 0, "width": 10, "show": "conditions", "top": 5});
    let open = crate::registrations::REGIONAL_WEATHER_REGIONALWEATHER.panel.unwrap().open;
    let panel = open(&source, &load, &request).unwrap();
    let exact = panel.exact().unwrap();
    let advance = exact.advance.clone();
    let with_error = dashboard_follow::panel::AtHour {
        cold: exact.cold.clone(),
        read: exact.read.clone(),
        advance: Arc::new(move |hour, seed, token, step| {
            if hour == 9 {
                assert!(advance(hour, seed, &true.into(), step).value.is_err());
            }
            advance(hour, seed, token, step)
        }),
    };
    let reads = played(source.replay_ref(), &with_error, 4, 2 * DAY + 1);
    for (hour, rows) in [(4, 3), (5, 0), (9, 3), (10, 3), (11, 3), (12, 3), (13, 0), (DAY, 0), (2 * DAY, 2), (2 * DAY + 1, 2)] {
        let read = reads.iter().find(|read| read.hour == hour).unwrap();
        assert!(read.value.is_ok(), "hour {hour}: {:?}", read.value);
        assert_eq!(read.rows, rows, "weather sample reads at hour {hour}");
        assert_eq!(*read.value, (exact.cold)(hour, &false.into()));
    }
    let sought = (exact.advance)(6, true, &false.into(), None);
    assert_eq!(*sought.value, (exact.cold)(6, &false.into()));
    assert_eq!(sought.rows, 3);
}
