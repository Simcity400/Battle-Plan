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

    let ranked = rank_regions(&[1, 2], &midnight, &measures, Show::Conditions, &[2], &HashSet::new(), Some(1));
    assert_eq!(ranked, vec![(2, true), (1, false)], "pinned regions come first followed by automatic rank");
    let legend = visible_legend(&[1], &midnight[4..5], &[("light_rain".to_owned(), [(1, vec![Some(0)]), (2, vec![Some(1)])].into())], Show::Conditions, &choices);
    assert_eq!(legend.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["clear"], "off-chart region 2 rain does not enter legend");
}
