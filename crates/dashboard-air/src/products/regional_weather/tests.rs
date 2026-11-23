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
