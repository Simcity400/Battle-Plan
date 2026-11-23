use content_reader_defines::defines::Defines;
use content_reader_map::map::MapData;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use viewer_foundation::color_inheritance::inheritance;

use super::air_assignment::Field;

#[derive(Clone, Copy, Debug)]
pub struct Alpha(f32);

impl Alpha {
    pub fn read(defines: &Defines) -> std::io::Result<Self> {
        let value = defines.number("NMapMode", "OCCUPATION_MAP_MODE_COUNTRY_STRIPE_ALPHA")? as f32;
        Ok(Self(if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, 1.0)
        }))
    }
}

fn note(out: &mut Field, why: impl Into<String>, count: usize) {
    *out.refused.entry(why.into()).or_default() += count;
}

pub fn read(
    frame: &DecodedFrame,
    viewer: u32,
    map: &MapData,
    state_provinces: &BTreeMap<u32, Vec<u32>>,
    alpha: Result<Alpha, &str>,
) -> Field {
    match prepare(frame, viewer, map, state_provinces, alpha) {
        Ok(field) => field,
        Err(why) => Field {
            refused: [(why, 1)].into(),
            ..Default::default()
        },
    }
}

fn prepare(
    frame: &DecodedFrame,
    viewer: u32,
    map: &MapData,
    state_provinces: &BTreeMap<u32, Vec<u32>>,
    alpha: Result<Alpha, &str>,
) -> Result<Field, String> {
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|error| error.to_string())?;
    let country = countries
        .iter()
        .find(|country| country.canonical_slot == viewer)
        .ok_or_else(|| format!("Occupation stripe viewer slot {viewer} has no Countries row"))?;
    let inputs = frame
        .complete::<tables::CountryCounterInputs>()
        .map_err(|error| error.to_string())?;
    let input = inputs
        .binary_search_by_key(&country.id, |row| row.country)
        .ok()
        .map(|at| &inputs[at])
        .ok_or("Occupation stripe viewing country has no CountryCounterInputs row")?;
    let roster = input
        .controlled_states
        .require("Occupation stripe controlled-state roster")?
        .as_ref()
        .ok_or("Occupation stripe controlled-state roster is unavailable")?;
    let mut out = Field::default();
    if roster.is_empty() {
        return Ok(out);
    }
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|error| error.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect::<BTreeMap<_, _>>();
    let provinces = frame
        .complete::<tables::Provinces>()
        .map_err(|error| error.to_string())?;
    let resistance = frame
        .complete::<tables::StateResistance>()
        .map_err(|error| error.to_string())?;
    let mut admitted = Vec::new();
    for state in roster {
        let Ok(at) = resistance.binary_search_by_key(state, |row| row.state) else {
            note(
                &mut out,
                format!("Occupation stripe state {state} has no StateResistance row"),
                1,
            );
            continue;
        };
        let occupied = resistance[at].country;
        if occupied as i32 <= 0 {
            continue;
        }
        let Some(members) = state_provinces.get(state) else {
            note(
                &mut out,
                format!("Occupation stripe state {state} has no content province roster"),
                1,
            );
            continue;
        };
        let mut eligible = Vec::new();
        for id in members {
            let Ok(at) = provinces.binary_search_by_key(id, |row| row.id) else {
                note(
                    &mut out,
                    format!("Occupation stripe province {id} has no Provinces row"),
                    1,
                );
                continue;
            };
            let controller = provinces[at].controller;
            if controller == 0 {
                continue;
            }
            match links.get(&controller) {
                Some(&slot) if slot == viewer => eligible.push(*id),
                Some(_) => {}
                None => note(
                    &mut out,
                    format!("Occupation stripe controller {controller} has no CountryLinks row"),
                    1,
                ),
            }
        }
        if !eligible.is_empty() {
            admitted.push((occupied, eligible));
        }
    }
    if admitted.is_empty() {
        return Ok(out);
    }
    let alpha = match alpha {
        Ok(alpha) => alpha,
        Err(why) => {
            let count = admitted.iter().map(|(_, ids)| ids.len()).sum();
            note(&mut out, why, count);
            return Ok(out);
        }
    };
    let inheritance = inheritance(map, frame, &links).map_err(|error| error.to_string())?;
    let appearance = frame
        .complete::<tables::CountryAppearance>()
        .map_err(|error| error.to_string())?;
    for (occupied, eligible) in admitted {
        let color = (|| {
            let slot = links.get(&occupied).copied().ok_or_else(|| {
                format!("Occupation stripe occupied country {occupied} has no CountryLinks row")
            })?;
            let source = inheritance.source(slot)?;
            let country = countries
                .iter()
                .find(|country| country.canonical_slot == source)
                .ok_or_else(|| {
                    format!("Occupation stripe colour source slot {source} has no Countries row")
                })?;
            let at = appearance
                .binary_search_by_key(&country.id, |row| row.country)
                .map_err(|_| {
                    format!(
                        "Occupation stripe colour source country {} has no CountryAppearance row",
                        country.id
                    )
                })?;
            let row = &appearance[at];
            Ok::<_, String>(super::map_mode::pack([
                f32::from_bits(row.red),
                f32::from_bits(row.green),
                f32::from_bits(row.blue),
                alpha.0,
            ]))
        })();
        match color {
            Ok(color) => {
                for id in eligible {
                    out.provinces.insert(id, color);
                }
            }
            Err(why) => note(&mut out, why, eligible.len()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::air_assignment::Field;
    use super::*;
    use content_reader_map::map::CountryColors;
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    use viewer_foundation::test_support::{test_frame, test_put as put};

    fn map() -> MapData {
        MapData {
            width: 1,
            height: 1,
            province_ids: vec![1],
            definitions: BTreeMap::new(),
            country_colors: CountryColors {
                use_overlord_color: [(b"inherits".to_vec(), true)].into(),
            },
        }
    }

    fn members() -> BTreeMap<u32, Vec<u32>> {
        [(10, vec![1, 2, 5]), (20, vec![3]), (30, vec![4])].into()
    }

    fn fixture() -> DecodedFrame {
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::Countries(
                (1..=4)
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id - 1,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::CountryLinks(
                [(1, 0), (2, 1), (3, 2), (4, 3), (11, 0), (22, 1)]
                    .into_iter()
                    .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 1,
                controlled_states: Some(vec![10, 20]).into(),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::Provinces(
                [(1, 10, 11), (2, 10, 2), (3, 20, 1), (4, 30, 1), (5, 10, 0)]
                    .into_iter()
                    .map(|(id, state, controller)| ProvinceRow {
                        id,
                        state,
                        owner: 4,
                        controller,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::StateResistance(vec![
                StateResistanceRow {
                    state: 10,
                    country: 22,
                    ..Default::default()
                },
                StateResistanceRow {
                    state: 20,
                    ..Default::default()
                },
                StateResistanceRow {
                    state: 30,
                    country: 2,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::CountryAppearance(
                [
                    (1, [0.0, 0.0, 1.0]),
                    (2, [1.0, 0.0, 0.0]),
                    (3, [0.75, 0.5, 0.25]),
                ]
                .into_iter()
                .map(|(country, [red, green, blue])| CountryAppearanceRow {
                    country,
                    red: f32::to_bits(red),
                    green: f32::to_bits(green),
                    blue: f32::to_bits(blue),
                    ..Default::default()
                })
                .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::Autonomy(vec![AutonomyRow {
                subject: 2,
                state: b"inherits".to_vec(),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::Subjects(vec![SubjectRow {
                subject: 2,
                overlord: 3,
                ..Default::default()
            }]),
        );
        frame
    }

    fn field(frame: &DecodedFrame) -> Field {
        read(frame, 0, &map(), &members(), Ok(Alpha(0.5)))
    }

    #[test]
    fn stripes_follow_occupied_inherited_colour_and_canonical_province_control() {
        let actual = field(&fixture());
        assert_eq!(actual.provinces, [(1, [191, 127, 63, 127])].into());
        assert!(actual.refused.is_empty());
    }

    #[test]
    fn each_view_uses_its_own_country_and_exact_controlled_state_roster() {
        let mut frame = fixture();
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 1,
                    controlled_states: Some(vec![10, 20]).into(),
                    ..Default::default()
                },
                CountryCounterInputRow {
                    country: 2,
                    controlled_states: Some(vec![10]).into(),
                    ..Default::default()
                },
            ]),
        );
        let first = field(&frame);
        let other = read(&frame, 1, &map(), &members(), Ok(Alpha(0.5)));
        assert_eq!(first.provinces.keys().copied().collect::<Vec<_>>(), [1]);
        assert_eq!(other.provinces, [(2, [191, 127, 63, 127])].into());
        assert_eq!(field(&frame), first);
    }

    #[test]
    fn alpha_uses_content_replacements_native_clamps_and_truncated_bytes() {
        for (source, expected) in [
            ("-1", 0),
            ("0", 0),
            ("0.5", 127),
            ("1", 255),
            ("2", 255),
            ("0/0", 0),
        ] {
            let defines = Defines::new().unwrap();
            defines
                .load(
                    "synthetic.lua",
                    format!("NDefines={{NMapMode={{OCCUPATION_MAP_MODE_COUNTRY_STRIPE_ALPHA={source}}}}}")
                        .as_bytes(),
                )
                .unwrap();
            let actual = read(
                &fixture(),
                0,
                &map(),
                &members(),
                Ok(Alpha::read(&defines).unwrap()),
            );
            assert_eq!(actual.provinces[&1][3], expected, "{source}");
            defines
                .load(
                    "replacement.lua",
                    b"NDefines.NMapMode.OCCUPATION_MAP_MODE_COUNTRY_STRIPE_ALPHA=0.25",
                )
                .unwrap();
            let actual = read(
                &fixture(),
                0,
                &map(),
                &members(),
                Ok(Alpha::read(&defines).unwrap()),
            );
            assert_eq!(actual.provinces[&1][3], 63);
        }
        assert!(Alpha::read(&Defines::new().unwrap()).is_err());
    }

    #[test]
    fn an_empty_roster_needs_neither_colour_inputs_nor_alpha() {
        let mut frame = fixture();
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 1,
                controlled_states: Some(vec![]).into(),
                ..Default::default()
            }]),
        );
        for table in [
            TableId::Autonomy,
            TableId::CountryAppearance,
            TableId::StateResistance,
        ] {
            frame.tables[table.index()] = TableView::Unavailable {
                reason: "unreached".into(),
            }
            .into();
        }
        assert_eq!(
            read(&frame, 0, &map(), &members(), Err("missing alpha")),
            Field::default()
        );
    }

    #[test]
    fn nonpositive_occupation_references_do_not_reach_colour_inputs() {
        let mut frame = fixture();
        put(
            &mut frame,
            TableRows::StateResistance(vec![
                StateResistanceRow {
                    state: 10,
                    country: u32::MAX,
                    ..Default::default()
                },
                StateResistanceRow {
                    state: 20,
                    ..Default::default()
                },
            ]),
        );
        frame.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
            reason: "unreached".into(),
        }
        .into();
        assert_eq!(
            read(&frame, 0, &map(), &members(), Err("missing alpha")),
            Field::default()
        );
    }

    #[test]
    fn a_missing_roster_is_refused_without_reconstructing_state_control() {
        let mut frame = fixture();
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 1,
                controlled_states: None.into(),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::States(vec![StateRow {
                id: 10,
                controller: 1,
                ..Default::default()
            }]),
        );
        let actual = field(&frame);
        assert!(actual.provinces.is_empty());
        assert_eq!(
            actual.refused["Occupation stripe controlled-state roster is unavailable"],
            1
        );
    }

    #[test]
    fn unknown_occupied_identity_and_missing_colour_never_use_owner_or_viewer_colour() {
        let mut frame = fixture();
        put(
            &mut frame,
            TableRows::StateResistance(vec![
                StateResistanceRow {
                    state: 10,
                    country: 99,
                    ..Default::default()
                },
                StateResistanceRow {
                    state: 20,
                    ..Default::default()
                },
            ]),
        );
        let actual = field(&frame);
        assert!(actual.provinces.is_empty());
        assert_eq!(
            actual.refused["Occupation stripe occupied country 99 has no CountryLinks row"],
            1
        );
        let mut frame = fixture();
        put(&mut frame, TableRows::CountryAppearance(vec![]));
        let actual = field(&frame);
        assert!(actual.provinces.is_empty());
        assert_eq!(
            actual.refused["Occupation stripe colour source country 3 has no CountryAppearance row"],
            1
        );
    }

    #[test]
    fn invalid_state_and_province_memberships_are_named_without_erasing_known_stripes() {
        let mut frame = fixture();
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 1,
                controlled_states: Some(vec![10, 99]).into(),
                ..Default::default()
            }]),
        );
        let mut members = members();
        members.get_mut(&10).unwrap().push(999);
        let actual = read(&frame, 0, &map(), &members, Ok(Alpha(0.5)));
        assert_eq!(actual.provinces, [(1, [191, 127, 63, 127])].into());
        assert_eq!(
            actual.refused,
            [
                (
                    "Occupation stripe province 999 has no Provinces row".into(),
                    1
                ),
                (
                    "Occupation stripe state 99 has no StateResistance row".into(),
                    1
                ),
            ]
            .into()
        );
    }

    #[test]
    fn unresolved_controllers_and_autonomy_refuse_the_reached_provinces() {
        let mut frame = fixture();
        put(
            &mut frame,
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 1,
                    state: 10,
                    controller: 999,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 2,
                    state: 10,
                    controller: 2,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 3,
                    state: 20,
                    controller: 1,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 5,
                    state: 10,
                    ..Default::default()
                },
            ]),
        );
        let actual = field(&frame);
        assert!(actual.provinces.is_empty());
        assert_eq!(
            actual.refused["Occupation stripe controller 999 has no CountryLinks row"],
            1
        );
        let mut frame = fixture();
        frame.tables[TableId::Autonomy.index()] = TableView::Unavailable {
            reason: "synthetic missing autonomy".into(),
        }
        .into();
        let actual = field(&frame);
        assert!(actual.provinces.is_empty());
        assert_eq!(actual.refused.values().sum::<usize>(), 1);
        assert!(
            actual
                .refused
                .keys()
                .next()
                .unwrap()
                .contains("current autonomy status is unknown")
        );
    }
}
