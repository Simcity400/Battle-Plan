use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defines {
    pub army_maximum: i64,
    pub basic: i64,
    pub standard: i64,
    pub count: [i64; 3],
}

impl Defines {
    pub fn load(content: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        let maximum = content.fixed5_array("NIntel", "COUNTRY_LEVEL_INTEL_MAXIMUMS")?;
        if maximum.len() != 4 {
            return Err(invalid(
                "COUNTRY_LEVEL_INTEL_MAXIMUMS must hold four values",
            ));
        }
        Ok(Self {
            army_maximum: maximum[1],
            basic: content.fixed5("NIntel", "ARMY_MIN_INTEL_TO_SHOW_BASIC_TEMPLATE_INFO")?,
            standard: content.fixed5(
                "NIntel",
                "ARMY_MIN_INTEL_RATIO_NEEDED_FOR_DISPLAYING_FAKE_ENEMY_INTEL_IN_LEDGER",
            )?,
            count: [
                content.fixed5("NIntel", "ARMY_TEMPLATE_UNIT_COUNT_INTEL_MIN")?,
                content.fixed5("NIntel", "ARMY_TEMPLATE_UNIT_COUNT_INTEL_MAX")?,
                content.fixed5(
                    "NIntel",
                    "ARMY_TEMPLATE_UNIT_COUNT_INTEL_RANGE_AT_LOWEST_INTEL",
                )?,
            ],
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disclosure {
    pub fraction: i64,
    pub basic: bool,
    pub standard: bool,
    count: [i64; 3],
}

impl Disclosure {
    pub fn read(
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        raw_viewer: u32,
        raw_target: u32,
        fog: bool,
        defines: &Defines,
    ) -> Result<Self, &'static str> {
        let fraction = if !fog {
            100_000
        } else {
            let viewer = map_intel_slot(links, raw_viewer)?;
            let target = map_intel_slot(links, raw_target)?;
            let shared = if viewer == 0 {
                raw_target == 0 || reverse_zero_shared(frame, links, target)?
            } else if target == 0 {
                forward_zero_shared(frame, links, viewer)?
            } else {
                Relations::read(frame, links)?.shared(viewer, target)
            };
            if shared {
                100_000
            } else {
                let intel = Intel {
                    rows: complete::<tables::CountryIntel>(frame, "CountryIntel"),
                    raws: raw_references(links),
                };
                let army = if raw_target as i32 <= 0 {
                    0
                } else if viewer == 0 {
                    intel.zero_army(frame, raw_target, target)?
                } else {
                    intel.army(viewer, target)?
                };
                intel_fraction(army, defines.army_maximum)?
            }
        };
        Ok(Self {
            fraction,
            basic: defines.basic <= fraction,
            standard: defines.standard <= fraction,
            count: defines.count,
        })
    }

    pub fn template(&self, standard: u64, fake: u64) -> Option<u64> {
        self.basic.then_some(if self.standard || fake == 0 {
            standard
        } else {
            fake
        })
    }

    pub fn count(&self, template: u64, count: i32, seed: u32, hour: i32) -> ([i32; 2], bool) {
        (
            viewer_runtime::count_math::count_range_seeded(
                count,
                self.fraction,
                seed.wrapping_add((template >> 32) as u32)
                    .wrapping_add(0x4a2),
                hour,
                self.count,
            ),
            self.count[0] <= self.fraction,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub source_template: u64,
    pub template: u64,
    pub count: [i32; 2],
    pub count_known: bool,
}

pub use dashboard_text::template_facts::display_name;

fn resolve(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    template: u64,
    target: u32,
    loc: &content_reader_localisation::localisation::Localisation,
) -> Result<u64, String> {
    let icons = complete::<tables::DivisionTemplateIcons>(frame, "DivisionTemplateIcons")?;
    let source = icons
        .iter()
        .find(|row| row.template == template)
        .ok_or_else(|| format!("template {template} source country missing"))?;
    let name = display_name(frame, template, loc)?;
    let effective = if source.foreign_country > 0 {
        source.foreign_country
    } else {
        source.country
    };
    let rosters = complete::<tables::DivisionTemplates>(frame, "DivisionTemplates")?;
    for raw in [source.country as u32, effective as u32, target] {
        let country = map_intel_slot(links, raw)?;
        let mut candidates = Vec::new();
        for row in rosters {
            if map_intel_slot(links, row.owner)? == country {
                let icon = icons
                    .iter()
                    .find(|icon| icon.template == row.template)
                    .ok_or_else(|| format!("template {} roster position missing", row.template))?;
                let ordinal = icon
                    .settings
                    .require("template settings")?
                    .as_ref()
                    .ok_or("template settings acquisition refused")?
                    .roster_ordinal;
                candidates.push((ordinal, row.template));
            }
        }
        candidates.sort_unstable_by_key(|&(ordinal, _)| ordinal);
        for (_, candidate) in candidates {
            if display_name(frame, candidate, loc)? == name {
                return Ok(candidate);
            }
        }
    }
    Err(format!(
        "template {template} display name has no source, effective or target roster match"
    ))
}

pub fn entries(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    raw_viewer: u32,
    raw_target: u32,
    fog: bool,
    defines: &Defines,
    loc: &content_reader_localisation::localisation::Localisation,
) -> Result<Vec<Entry>, String> {
    let disclosure = Disclosure::read(frame, links, raw_viewer, raw_target, fog, defines)?;
    if !disclosure.basic {
        return Ok(Vec::new());
    }
    let target = map_intel_slot(links, raw_target)?;
    let divisions = complete::<tables::Divisions>(frame, "Divisions")?;
    let icons = (!disclosure.standard)
        .then(|| complete::<tables::ArmyIconInputs>(frame, "ArmyIconInputs"))
        .transpose()?;
    let mut counts = BTreeMap::<u64, i32>::new();
    for division in divisions {
        if map_intel_slot(links, division.roster_owner)? != target {
            continue;
        }
        let fake = if let Some(icons) = icons {
            icons
                .binary_search_by_key(&division.id, |row| row.division)
                .map(|at| icons[at].alternate_template)
                .map_err(|_| "ArmyIconInputs division missing")?
        } else {
            0
        };
        let template = disclosure.template(division.template, fake).unwrap();
        let count = counts.entry(template).or_default();
        *count = count.wrapping_add(1);
    }
    if counts.is_empty() {
        return Ok(Vec::new());
    }
    let [clock] = complete::<tables::Clock>(frame, "Clock")? else {
        return Err("Clock singleton".into());
    };
    counts
        .into_iter()
        .map(|(source_template, count)| {
            let template = resolve(frame, links, source_template, raw_target, loc)?;
            let (count, count_known) =
                disclosure.count(template, count, clock.seed as u32, frame.hour);
            Ok(Entry {
                source_template,
                template,
                count,
                count_known,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::ArmyIconInputRow, rows::ClockRow, rows::CountryIntelRow, rows::DivisionRow,
        rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;

    fn defines() -> Defines {
        Defines {
            army_maximum: 100_000,
            basic: 33_000,
            standard: 70_000,
            count: [40_000, 80_000, 50_000],
        }
    }
    fn intel(frame: &mut DecodedFrame, raw: i64) {
        *frame.tables[TableId::CountryIntel.index()] =
            TableView::Complete(TableRows::CountryIntel(vec![CountryIntelRow {
                owner_country: 101,
                target_index: 2,
                army: raw,
                ..Default::default()
            }]));
    }
    type RosterEntry<'a> = (u64, u32, i32, i32, u32, &'a [u8], &'a [u8]);
    fn roster(frame: &mut DecodedFrame, rows: &[RosterEntry<'_>]) {
        use replay_data::{
            rows::DivisionTemplateIconRow, rows::DivisionTemplateRow, rows::PacketHour,
            rows::TemplateDisplayName, rows::TemplateNameValue, rows::TemplateSettings,
        };
        let mut dictionaries = Vec::new();
        let mut icons = Vec::new();
        for &(template, owner, country, foreign_country, ordinal, name, key) in rows {
            dictionaries.push(DivisionTemplateRow {
                template,
                owner,
                name: name.to_vec(),
                name_key: Recorded::Value(key.to_vec()),
                ..Default::default()
            });
            icons.push(DivisionTemplateIconRow {
                template,
                country,
                foreign_country,
                settings: Recorded::Value(Some(
                    TemplateSettings {
                        roster_ordinal: ordinal,
                        ..Default::default()
                    }
                    .into(),
                )),
                display_name: Recorded::Value(Some(TemplateDisplayName {
                    sample_hour: PacketHour::Host,
                    value: TemplateNameValue::SameAsDivisionTemplates,
                })),
                ..Default::default()
            });
        }
        dictionaries.sort_by_key(|row| row.template);
        icons.sort_by_key(|row| row.template);
        *frame.tables[TableId::DivisionTemplates.index()] =
            TableView::Complete(TableRows::DivisionTemplates(dictionaries));
        *frame.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(icons));
    }

    #[test]
    fn display_name_collisions_resolve_in_native_roster_order_and_country_fallback_order() {
        let (mut frame, links, _) = super::super::tests::fixture();
        let loc: content_reader_localisation::localisation::Localisation = [
            ("KEY_A".to_owned(), "Same name".to_owned()),
            ("KEY_B".to_owned(), "Same name".to_owned()),
        ]
        .into_iter()
        .collect();
        let rows = [
            (
                52 | (9 << 32),
                102,
                101,
                103,
                1,
                b"".as_slice(),
                b"KEY_A".as_slice(),
            ),
            (
                52 | (8 << 32),
                101,
                101,
                -1,
                0,
                b"".as_slice(),
                b"KEY_B".as_slice(),
            ),
            (
                52 | (2 << 32),
                101,
                101,
                -1,
                1,
                b"".as_slice(),
                b"KEY_A".as_slice(),
            ),
            (
                52 | (7 << 32),
                103,
                103,
                -1,
                0,
                b"".as_slice(),
                b"KEY_B".as_slice(),
            ),
            (
                52 | (6 << 32),
                102,
                102,
                -1,
                0,
                b"".as_slice(),
                b"KEY_B".as_slice(),
            ),
        ];
        roster(&mut frame, &rows);
        assert_eq!(
            resolve(&frame, &links, rows[0].0, 102, &loc).unwrap(),
            rows[1].0
        );
        roster(&mut frame, &[rows[0], rows[3], rows[4]]);
        assert_eq!(
            resolve(&frame, &links, rows[0].0, 102, &loc).unwrap(),
            rows[3].0
        );
        roster(&mut frame, &[rows[0], rows[4]]);
        assert_eq!(
            resolve(&frame, &links, rows[0].0, 102, &loc).unwrap(),
            rows[4].0
        );
        let disclosure = Disclosure {
            fraction: 60_000,
            basic: true,
            standard: false,
            count: defines().count,
        };
        let resolved = resolve(&frame, &links, rows[0].0, 102, &loc).unwrap();
        intel(&mut frame, 60_000);
        *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
            (1..=19)
                .map(|id| DivisionRow {
                    id,
                    roster_owner: 102,
                    template: rows[0].0,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::ArmyIconInputs.index()] =
            TableView::Complete(TableRows::ArmyIconInputs(
                (1..=19)
                    .map(|division| ArmyIconInputRow {
                        division,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                seed: 123,
                ..Default::default()
            }]));
        let entries = entries(&frame, &links, 101, 102, true, &defines(), &loc).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source_template, rows[0].0);
        assert_eq!(entries[0].template, resolved);
        let (count, known) = disclosure.count(resolved, 19, 123, frame.hour);
        assert_eq!((entries[0].count, entries[0].count_known), (count, known));
    }
    #[test]
    fn template_reveal_boundaries_fake_fallback_and_native_sentinels() {
        let (mut frame, links, _) = super::super::tests::fixture();
        let mut d = defines();
        for (value, basic, standard) in [
            (d.basic - 1, false, false),
            (d.basic, true, false),
            (d.standard - 1, true, false),
            (d.standard, true, true),
        ] {
            intel(&mut frame, value);
            let disclosure = Disclosure::read(&frame, &links, 101, 102, true, &d).unwrap();
            assert_eq!((disclosure.basic, disclosure.standard), (basic, standard));
            assert_eq!(
                disclosure.template(52, 99),
                basic.then_some(if standard { 52 } else { 99 })
            );
            assert_eq!(disclosure.template(52, 0), basic.then_some(52));
        }
        d.army_maximum = 0;
        assert_eq!(
            Disclosure::read(&frame, &links, 101, 102, true, &d)
                .unwrap()
                .fraction,
            i64::from(u32::MAX)
        );
        d.basic = 100_001;
        assert!(
            !Disclosure::read(&frame, &links, 101, 102, false, &d)
                .unwrap()
                .basic
        );
        assert!(
            !Disclosure::read(&frame, &links, 101, 101, true, &d)
                .unwrap()
                .basic
        );
    }
    #[test]
    fn template_reveal_ledger_groups_fake_templates_and_obeys_count_gate() {
        let (mut frame, mut links, _) = super::super::tests::fixture();
        links.insert(202, 2);
        intel(&mut frame, 39_999);
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![
                DivisionRow {
                    id: 1,
                    roster_owner: 102,
                    template: (4 << 32) | 52,
                    ..Default::default()
                },
                DivisionRow {
                    id: 2,
                    roster_owner: 202,
                    template: (5 << 32) | 52,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::ArmyIconInputs.index()] =
            TableView::Complete(TableRows::ArmyIconInputs(vec![
                ArmyIconInputRow {
                    division: 1,
                    alternate_template: (9 << 32) | 52,
                    ..Default::default()
                },
                ArmyIconInputRow {
                    division: 2,
                    alternate_template: (9 << 32) | 52,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                seed: 123,
                ..Default::default()
            }]));
        roster(
            &mut frame,
            &[
                ((4 << 32) | 52, 102, 102, -1, 0, b"A", b""),
                ((5 << 32) | 52, 202, 102, -1, 1, b"B", b""),
                ((9 << 32) | 52, 102, 102, -1, 2, b"Decoy", b""),
            ],
        );
        let loc = content_reader_localisation::localisation::Localisation::default();
        let values = entries(&frame, &links, 101, 202, true, &defines(), &loc).unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].template, (9 << 32) | 52);
        assert!(!values[0].count_known);
        intel(&mut frame, 80_000);
        let values = entries(&frame, &links, 101, 102, true, &defines(), &loc).unwrap();
        assert_eq!(values.len(), 2);
        assert!(
            values
                .iter()
                .all(|entry| entry.count_known && entry.count == [1, 1])
        );
    }
}
