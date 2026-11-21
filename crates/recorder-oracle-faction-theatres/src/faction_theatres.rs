
use std::{collections::BTreeMap, io};

use installed_content_save::Save;
use replay_data::{rows::FactionTheatreRow, rows::Recorded, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{CountryContext, Report, complete, invalid, required_country_tag};

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut captured: BTreeMap<u64, BTreeMap<u32, &FactionTheatreRow>> = BTreeMap::new();
    for row in complete::<tables::FactionTheatres>(frame)? {
        if row.faction == 0
            || captured
                .entry(row.faction)
                .or_default()
                .insert(row.ordinal, row)
                .is_some()
        {
            return Err(invalid(
                "FactionTheatres has a null faction or a duplicate ordinal",
            ));
        }
    }
    let label = |id: u64| format!("faction type={} id={}", id as u32, id >> 32);
    let mut compared = 0;
    for (&id, faction) in &save.factions {
        let mut rows = captured.remove(&id).unwrap_or_default();
        for (ordinal, expected) in faction.theatres.iter().enumerate() {
            let field = format!("{}.theatres[{ordinal}]", label(id));
            let key = u32::try_from(ordinal)
                .map_err(|_| invalid("saved faction theatres exceed u32 ordinals"))?;
            let Some(row) = rows.remove(&key) else {
                report.difference(&field, "absent", "present");
                continue;
            };
            compared += 1;
            let Some(icon) = &faction.icon else {
                return Err(invalid(
                    "saved faction with theatres lacks the icon its writer always emits",
                ));
            };
            check(report, &field, "icon", &row.icon, icon, |bytes| {
                String::from_utf8_lossy(bytes).into_owned()
            });
            check(
                report,
                &field,
                "regions",
                &row.regions,
                &expected.regions,
                |ids| format!("{ids:?}"),
            );
            let saved_count = u32::try_from(expected.countries.len())
                .map_err(|_| invalid("saved theatre country list exceeds u32"))?;
            check(
                report,
                &field,
                "country_count",
                &row.country_count,
                &saved_count,
                u32::to_string,
            );
            let tags = |links: &[u32]| {
                links
                    .iter()
                    .map(|&link| required_country_tag(link, &countries.links).map(<[u8]>::to_vec))
                    .collect::<io::Result<Vec<_>>>()
            };
            let tag_text = |tags: &Vec<Vec<u8>>| {
                let tags: Vec<_> = tags.iter().map(|t| String::from_utf8_lossy(t)).collect();
                format!("{tags:?}")
            };
            check(
                report,
                &field,
                "hidden_for",
                &tags(&row.hidden_for)?,
                &expected.hidden,
                tag_text,
            );
            let Some(saved_name) = &expected.name else {
                return Err(invalid(
                    "saved faction theatre lacks the name its text writer always emits",
                ));
            };
            match &row.name {
                Recorded::Value(name) => check(report, &field, "name", name, saved_name, |bytes| {
                    String::from_utf8_lossy(bytes).into_owned()
                }),
                Recorded::NotRecorded => not_recorded(report, &field, "name"),
            }
            match row.owner {
                Recorded::Value(owner) => {
                    let owner = Some(required_country_tag(owner, &countries.links)?.to_vec());
                    check(report, &field, "owner", &owner, &expected.tag, |tag| {
                        tag.as_deref()
                            .map_or("absent".into(), |t| String::from_utf8_lossy(t).into_owned())
                    });
                }
                Recorded::NotRecorded => not_recorded(report, &field, "owner"),
            }
            match row.commander {
                Recorded::Value(commander) => check(
                    report,
                    &field,
                    "commander",
                    &commander,
                    &expected.commander,
                    |handle| {
                        handle.map_or("none".into(), |handle| {
                            format!("type={} id={}", handle as u32, handle >> 32)
                        })
                    },
                ),
                Recorded::NotRecorded => not_recorded(report, &field, "commander"),
            }
            match &row.countries {
                Recorded::Value(links) => check(
                    report,
                    &field,
                    "countries",
                    &tags(links)?,
                    &expected.countries,
                    tag_text,
                ),
                Recorded::NotRecorded => not_recorded(report, &field, "countries"),
            }
        }
        for ordinal in rows.keys() {
            report.difference(
                format!("{}.theatres[{ordinal}]", label(id)),
                "present",
                "absent",
            );
        }
    }
    for (id, rows) in captured {
        for ordinal in rows.keys() {
            report.difference(
                format!("{}.theatres[{ordinal}] (faction not saved)", label(id)),
                "present",
                "absent",
            );
        }
    }
    report.compared("FactionTheatres", compared);
    report.limitation(
        "the theatre hidden-pin set saves as token 0x2C8C, read as `hidden` (the spelling of that token for the event, idea-category and achievement readers); no inspected save had a nonempty set",
        "FactionTheatres.hidden_for",
    );
    Ok(())
}

fn not_recorded(report: &mut Report, theatre: &str, field: &str) {
    report.limitation(&format!("FactionTheatres.{field} not recorded"), theatre);
}

fn check<T: PartialEq + ?Sized>(
    report: &mut Report,
    theatre: &str,
    field: &str,
    captured: &T,
    saved: &T,
    text: impl Fn(&T) -> String,
) {
    report.scalar_checks += 1;
    if captured != saved {
        report.difference(format!("{theatre}.{field}"), text(captured), text(saved));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;

    const AXIS: u64 = (1 << 32) | 88;
    const ALLIES: u64 = (2 << 32) | 88;
    const ENG: u32 = 201;
    const COMMANDER: u64 = (844 << 32) | 4713;

    fn fixture() -> (DecodedFrame, Save) {
        let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
        super::super::fixtures::add_country(&mut frame, &mut save, ENG, 8, b"ENG");
        let faction = |id, icon: &[u8], theatres| installed_content_save::Faction {
            goals: Default::default(),
            companions: Default::default(),
            advisor_slots: Vec::new(),
            id,
            name: None,
            members: Vec::new(),
            icon: Some(icon.to_vec()),
            template: None,
            color: None,
            color_overridden: None,
            theatres,
        };
        save.factions
            .insert(AXIS, faction(AXIS, b"GFX_axis", Vec::new()));
        save.factions.insert(
            ALLIES,
            faction(
                ALLIES,
                b"GFX_allies",
                vec![
                    installed_content_save::FactionTheatre {
                        name: Some(b"Western Front".to_vec()),
                        tag: Some(b"ENG".to_vec()),
                        commander: Some(COMMANDER),
                        regions: vec![29, 48],
                        countries: vec![b"GER".to_vec(), b"ENG".to_vec()],
                        hidden: Vec::new(),
                    },
                    installed_content_save::FactionTheatre {
                        name: Some(Vec::new()),
                        tag: Some(b"GER".to_vec()),
                        commander: None,
                        regions: vec![80],
                        countries: Vec::new(),
                        hidden: vec![b"GER".to_vec()],
                    },
                ],
            ),
        );
        (frame, save)
    }

    fn rows() -> Vec<FactionTheatreRow> {
        vec![
            FactionTheatreRow {
                faction: ALLIES,
                ordinal: 0,
                icon: b"GFX_allies".to_vec(),
                regions: vec![29, 48],
                country_count: 2,
                hidden_for: Vec::new(),
                name: b"Western Front".to_vec().into(),
                owner: Recorded::Value(ENG),
                commander: Some(COMMANDER).into(),
                countries: vec![101, ENG].into(),
            },
            FactionTheatreRow {
                faction: ALLIES,
                ordinal: 1,
                icon: b"GFX_allies".to_vec(),
                regions: vec![80],
                country_count: 0,
                hidden_for: vec![102],
                name: Vec::new().into(),
                owner: Recorded::Value(101),
                commander: None.into(),
                countries: Vec::new().into(),
            },
        ]
    }

    fn run(frame: &DecodedFrame, save: &Save, rows: Vec<FactionTheatreRow>) -> Report {
        let mut frame = frame.clone();
        *frame.tables[TableId::FactionTheatres.index()] =
            TableView::Complete(TableRows::FactionTheatres(rows));
        let context = super::super::country_context(&frame).unwrap();
        let mut report = Report::default();
        compare(&frame, save, &context, &mut report).unwrap();
        report
    }

    #[test]
    fn saved_theatres_match_by_ordinal_with_resolved_hidden_tags() {
        let (frame, save) = fixture();
        let report = run(&frame, &save, rows());
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["FactionTheatres"], 2);
        assert_eq!(report.scalar_checks, 16);
    }

    #[test]
    fn each_field_missing_and_extra_theatres_are_differences() {
        let (frame, save) = fixture();
        for (edit, expected) in [
            (
                (|rows: &mut Vec<FactionTheatreRow>| rows[0].icon = b"GFX_axis".to_vec())
                    as fn(&mut Vec<FactionTheatreRow>),
                1,
            ),
            (|rows| rows[0].regions.reverse(), 1),
            (|rows| rows[1].country_count = 1, 1),
            (|rows| rows[1].hidden_for.clear(), 1),
            (|rows| rows[0].name = b"Eastern Front".to_vec().into(), 1),
            (|rows| rows[1].owner = Recorded::Value(ENG), 1),
            (|rows| rows[0].commander = None.into(), 1),
            (|rows| rows[1].commander = Some(COMMANDER).into(), 1),
            (|rows| rows[0].countries = vec![ENG, 101].into(), 1),
            (|rows| drop(rows.pop()), 1),
            (
                |rows| {
                    let mut extra = rows[0].clone();
                    extra.faction = AXIS;
                    rows.push(extra);
                },
                1,
            ),
            (
                |rows| {
                    let mut extra = rows[0].clone();
                    extra.faction = (9 << 32) | 88;
                    rows.push(extra);
                },
                1,
            ),
        ] {
            let mut rows = rows();
            edit(&mut rows);
            let report = run(&frame, &save, rows);
            assert_eq!(
                report.differences.len(),
                expected,
                "{:?}",
                report.differences
            );
        }
    }

    #[test]
    fn unrecorded_fields_are_limitations_never_matches() {
        let (frame, save) = fixture();
        let mut rows = rows();
        rows[0].name = Recorded::NotRecorded;
        rows[0].owner = Recorded::NotRecorded;
        rows[0].commander = Recorded::NotRecorded;
        rows[0].countries = Recorded::NotRecorded;
        let report = run(&frame, &save, rows);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        for field in ["name", "owner", "commander", "countries"] {
            let reason = format!("FactionTheatres.{field} not recorded");
            assert_eq!(report.limitations[&reason].len(), 1, "{reason}");
        }
    }

    #[test]
    fn duplicate_ordinals_and_unresolved_hidden_links_fail_the_comparison() {
        let (frame, save) = fixture();
        let mut frame = frame;
        let mut duplicate = rows();
        duplicate[1].ordinal = 0;
        let mut unknown = rows();
        unknown[1].hidden_for = vec![999];
        let mut unknown_owner = rows();
        unknown_owner[0].owner = Recorded::Value(999);
        let mut unknown_country = rows();
        unknown_country[0].countries = vec![999].into();
        for rows in [duplicate, unknown, unknown_owner, unknown_country] {
            *frame.tables[TableId::FactionTheatres.index()] =
                TableView::Complete(TableRows::FactionTheatres(rows));
            let context = super::super::country_context(&frame).unwrap();
            assert!(compare(&frame, &save, &context, &mut Report::default()).is_err());
        }
    }
}
