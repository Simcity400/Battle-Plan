use super::*;
pub fn eq<T: PartialEq + std::fmt::Debug>(r: &mut Report, label: &str, a: T, b: T) {
    r.scalar_checks += 1;
    if a != b {
        r.difference(label, a, b);
    }
}
pub fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::CountryCounterInputs, |r| {
        sent_expeditionaries(frame, save, r)
    });
    report.attempt_table(TableId::SentVolunteers, |r| sent_volunteers(frame, save, r));
    report.attempt_table(TableId::ProvinceUnitOrder, |r| provinces(frame, save, r));
    report.attempt_table(TableId::CharacterPortraits, |r| characters(frame, save, r));
    report.attempt_table(TableId::LeaderPortraits, |r| leaders(frame, save, r));
}
fn sent_expeditionaries(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::CountryCounterInputs>()?;
    let context = country_context(frame)?;
    for row in rows {
        let tag = required_country_tag(row.country, &context.links)?;
        let label = format!(
            "CountryCounterInputs {} sent expeditionaries",
            String::from_utf8_lossy(tag)
        );
        let expected = save
            .counter_inputs
            .sent_expeditionaries
            .get(tag)
            .map_or(&[][..], Vec::as_slice);
        let actual = row
            .sent_expeditionaries
            .require(&label)
            .map_err(invalid)?
            .as_ref()
            .map(|values| &values[..]);
        eq(r, &label, actual, Some(expected));
    }
    r.compared("CountryCounterInputs.sent_expeditionaries", rows.len());
    Ok(())
}
fn sent_volunteers(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::SentVolunteers>()?;
    let context = country_context(frame)?;
    let mut actual = BTreeMap::new();
    for row in rows {
        let tag = required_country_tag(row.country, &context.links)?;
        let handles: &mut Vec<u64> = actual.entry(tag.to_vec()).or_default();
        if handles.len() != row.ordinal as usize {
            return Err(invalid("SentVolunteers ordinals are not contiguous"));
        }
        handles.push(row.division);
    }
    compare_sent_volunteers(&actual, &save.counter_inputs.sent_volunteers, r);
    r.compared("SentVolunteers", rows.len());
    Ok(())
}
fn compare_sent_volunteers(
    actual: &BTreeMap<Vec<u8>, Vec<u64>>,
    expected: &BTreeMap<Vec<u8>, Vec<u64>>,
    r: &mut Report,
) {
    let tags: BTreeSet<_> = actual.keys().chain(expected.keys()).collect();
    for tag in tags {
        eq(
            r,
            &format!("SentVolunteers {} order", String::from_utf8_lossy(tag)),
            actual.get(tag).map_or(&[][..], Vec::as_slice),
            expected.get(tag).map_or(&[][..], Vec::as_slice),
        );
    }
}
fn provinces(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::ProvinceUnitOrder>()?;
    let province_ids: BTreeSet<_> = frame
        .complete::<tables::Provinces>()?
        .iter()
        .map(|p| p.id)
        .collect();
    eq(
        r,
        "ProvinceUnitOrder province census",
        rows.iter().map(|p| p.province).collect::<BTreeSet<_>>(),
        province_ids,
    );
    let mut seen_divisions = BTreeSet::new();
    let mut seen_tasks = BTreeSet::new();
    let mut seen_guns = BTreeSet::new();
    for row in rows {
        for &handle in &row.mixed {
            if let Some(d) = save.divisions.get(&handle) {
                if !seen_divisions.insert(handle) {
                    return Err(invalid("division repeated in province mixed roster"));
                }
                eq(
                    r,
                    &format!("ProvinceUnitOrder {handle} division location"),
                    Some(row.province),
                    d.province,
                );
            } else if let Some(t) = save.task_forces.get(&handle) {
                if !seen_tasks.insert(handle) {
                    return Err(invalid("task force repeated in province mixed roster"));
                }
                eq(
                    r,
                    &format!("ProvinceUnitOrder {handle} task location"),
                    Some(row.province),
                    t.province,
                );
            } else if let Some(g) = save.railway_guns.get(&handle) {
                if !seen_guns.insert(handle) {
                    return Err(invalid("railway gun repeated in province mixed roster"));
                }
                eq(
                    r,
                    &format!("ProvinceUnitOrder {handle} railway gun location"),
                    Some(row.province),
                    g.province,
                );
            } else {
                r.difference(
                    format!("ProvinceUnitOrder {handle} mixed occupant"),
                    "present",
                    "no saved division, task force or railway gun",
                );
            }
        }
        let expected: BTreeSet<_> = save
            .task_forces
            .values()
            .filter(|t| t.province == Some(row.province))
            .map(|t| t.id)
            .collect();
        eq(
            r,
            &format!("ProvinceUnitOrder {} naval membership", row.province),
            row.naval.iter().copied().collect::<BTreeSet<_>>(),
            expected,
        );
        if row.naval.iter().collect::<BTreeSet<_>>().len() != row.naval.len() {
            return Err(invalid("duplicate naval province occupant"));
        }
    }
    eq(
        r,
        "ProvinceUnitOrder located division census",
        seen_divisions,
        save.divisions
            .values()
            .filter(|d| d.province.is_some())
            .map(|d| d.id)
            .collect(),
    );
    eq(
        r,
        "ProvinceUnitOrder located task-force census",
        seen_tasks,
        save.task_forces
            .values()
            .filter(|t| t.province.is_some())
            .map(|t| t.id)
            .collect(),
    );
    eq(
        r,
        "ProvinceUnitOrder located railway-gun census",
        seen_guns,
        save.railway_guns
            .values()
            .filter(|g| g.province.is_some())
            .map(|g| g.id)
            .collect(),
    );
    r.compared("ProvinceUnitOrder", rows.len());
    r.limitation("The province writer omits occupant arrays; the unit writer saves location but no province insertion ordinal. Country unit serialization order is not an independent witness of the current province order. Membership/location are checked; mixed/naval ordinals remain unverified", "ProvinceUnitOrder order");
    Ok(())
}
fn characters(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::CharacterPortraits>()?;
    let manager = save
        .characters
        .manager
        .as_ref()
        .ok_or_else(|| invalid("save lacks character manager"))?;
    let expected: BTreeSet<_> = manager
        .characters
        .keys()
        .map(|id| (id >> 32) as u32)
        .collect();
    eq(
        r,
        "CharacterPortraits character census",
        rows.iter().map(|p| p.character).collect::<BTreeSet<_>>(),
        expected,
    );
    const ROLES: [&[u8]; 6] = [
        b"civilian",
        b"army",
        b"navy",
        b"air",
        b"operative",
        b"scientist",
    ];
    const SIZES: [&[u8]; 2] = [b"small", b"large"];
    for row in rows {
        let Some(character) = manager
            .characters
            .get(&((u64::from(row.character) << 32) | 73))
        else {
            continue;
        };
        let saved = character
            .portraits
            .as_ref()
            .ok_or_else(|| invalid("character save lacks portrait holder"))?;
        let mut projected = Vec::new();
        for entry in &row.portraits {
            let role = ROLES
                .get(entry.category as usize)
                .ok_or_else(|| invalid("portrait category outside serializer mapping"))?;
            let size = SIZES
                .get(entry.size as usize)
                .ok_or_else(|| invalid("portrait size outside serializer mapping"))?;
            if entry.kind > 1 {
                return Err(invalid("portrait kind has no serialized scalar"));
            }
            projected.push(save::characters::Portrait {
                category: role.to_vec(),
                size: size.to_vec(),
                identity: content_reader_parser::parser::c_string(&entry.identity).to_vec(),
            });
        }
        eq(
            r,
            &format!("CharacterPortraits {} ordered identities", row.character),
            &projected,
            saved,
        );
    }
    r.compared("CharacterPortraits", rows.len());
    r.limitation("The portrait writers serialize category, size and the identity string for kinds0/1; the set byte and kind distinction are not serialized. Text identity comparison ends at embedded NUL; trailing resource/cache pointers are not part of the contract", "CharacterPortraits descriptor");
    Ok(())
}
pub fn hq_leader_scope(
    frame: &DecodedFrame,
    save: &Save,
    r: &mut Report,
    label: &str,
    actual: BTreeSet<u64>,
) -> io::Result<()> {
    let recorded: BTreeSet<_> = frame
        .complete::<tables::OrdersGroups>()?
        .iter()
        .filter(|group| group.hq_provider != 0 && group.leader != 0)
        .map(|group| group.leader)
        .collect();
    eq(r, &format!("{label} HQ census"), &actual, &recorded);
    let countries = save
        .naval_history
        .theatre_countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks theatre countries for HQ portrait scope"))?;
    let saved: BTreeSet<_> = countries
        .values()
        .flat_map(|country| &country.land)
        .flat_map(|theatre| &theatre.definitions)
        .filter(|group| group.leader_unit.is_some_and(|unit| unit != 0))
        .filter_map(|group| group.leader.filter(|leader| *leader != 0))
        .collect();
    for leader in saved {
        eq(
            r,
            &format!("{label} {leader} saved HQ leader"),
            actual.contains(&leader),
            true,
        );
    }
    Ok(())
}

fn leaders(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LeaderPortraits>()?;
    let by_id: BTreeMap<_, _> = rows.iter().map(|p| (p.leader, p)).collect();
    hq_leader_scope(
        frame,
        save,
        r,
        "LeaderPortraits",
        by_id.keys().copied().collect(),
    )?;
    for (&id, row) in &by_id {
        let Some(saved) = save.characters.portraits.get(&id) else {
            r.limitation(
                "HQ leader has no serialized portrait wrapper",
                format!("LeaderPortraits {id}"),
            );
            continue;
        };
        for (index, actual) in [&row.portrait_c0, &row.portrait_e0, &row.portrait_100]
            .into_iter()
            .enumerate()
        {
            eq(
                r,
                &format!("LeaderPortraits {id} string{index}"),
                content_reader_parser::parser::c_string(actual),
                saved[index].as_slice(),
            );
        }
    }
    r.compared("LeaderPortraits", rows.len());
    r.limitation("the unit-leader writer omits empty portrait_path/picture/gfx; comparison uses their proven empty-string default and text bytes through NUL. Only current HQ leaders are captured; all saved HQ leaders are required. Unsaved HQ leaders and raw character references cannot be verified by these strings", "LeaderPortraits");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::CharacterPortrait, rows::CharacterPortraitRow, rows::LeaderPortraitRow,
    };
    fn set_hq_scope(frame: &mut DecodedFrame, saved: &mut Save, leader: u64) {
        use content_reader_save::naval_history::{LandTheatre, OrdersGroup, TheatreCountry};
        *frame.tables[TableId::OrdersGroups.index()] =
            TableView::Complete(TableRows::OrdersGroups(vec![
                replay_data::rows::OrdersGroupRow {
                    handle: 53,
                    leader,
                    hq_provider: 51,
                    ..Default::default()
                },
            ]));
        saved.naval_history.theatre_countries = Some(BTreeMap::from([(
            b"GER".to_vec(),
            TheatreCountry {
                land: vec![LandTheatre {
                    id: 1,
                    volunteers_country: None,
                    groups: vec![],
                    units: vec![],
                    area_representatives: vec![],
                    definitions: vec![OrdersGroup {
                        id: 53,
                        leader: Some(leader),
                        leader_unit: Some(51),
                        ..Default::default()
                    }],
                }],
                air: vec![],
            },
        )]));
    }
    #[test]
    fn ordered_portraits_and_noncharacter_strings_compare_and_mismatches_fail() {
        let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
        let doc=content_reader_parser::parser::parse(br#"countries={GER={units={corps_commander={id={id=8 type=4713} portrait_path="x.dds" picture="p" gfx="GFX_z"}}}} character_manager={historical={character={id={id=7 type=73} token=x portraits={civilian={small="GFX_a"} civilian={large="GFX_b"}}}}}"#).unwrap();
        saved.characters = save::characters::parse(&doc.items).unwrap();
        set_hq_scope(&mut frame, &mut saved, (8 << 32) | 4713);
        *frame.tables[TableId::CharacterPortraits.index()] =
            TableView::Complete(TableRows::CharacterPortraits(vec![CharacterPortraitRow {
                character: 7,
                portraits: vec![
                    CharacterPortrait {
                        category: 0,
                        size: 0,
                        is_set: 255,
                        kind: 0,
                        identity: b"GFX_a".to_vec(),
                    },
                    CharacterPortrait {
                        category: 0,
                        size: 1,
                        is_set: 1,
                        kind: 0,
                        identity: b"GFX_b".to_vec(),
                    },
                ],
            }]));
        *frame.tables[TableId::LeaderPortraits.index()] =
            TableView::Complete(TableRows::LeaderPortraits(vec![LeaderPortraitRow {
                leader: (8 << 32) | 4713,
                character: 0,
                portrait_c0: b"x.dds".to_vec(),
                portrait_e0: b"p".to_vec(),
                portrait_100: b"GFX_z".to_vec(),
            }]));
        let mut report = Report::default();
        characters(&frame, &saved, &mut report).unwrap();
        leaders(&frame, &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 7);
        if let TableView::Complete(TableRows::CharacterPortraits(rows)) =
            &mut *frame.tables[TableId::CharacterPortraits.index()]
        {
            rows[0].portraits.swap(0, 1);
        }
        saved
            .characters
            .portraits
            .get_mut(&((8 << 32) | 4713))
            .unwrap()[1] = b"different".to_vec();
        let mut report = Report::default();
        characters(&frame, &saved, &mut report).unwrap();
        leaders(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
    }

    #[test]
    fn hq_scope_checks_saved_leaders_even_if_recorded_groups_and_portraits_both_omit_them() {
        let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
        set_hq_scope(&mut frame, &mut saved, (8 << 32) | 4713);
        *frame.tables[TableId::OrdersGroups.index()] =
            TableView::Complete(TableRows::OrdersGroups(vec![]));
        let mut report = Report::default();
        hq_leader_scope(
            &frame,
            &saved,
            &mut report,
            "LeaderPortraits",
            BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        let mut report = Report::default();
        hq_leader_scope(
            &frame,
            &saved,
            &mut report,
            "LeaderPortraitPaths",
            BTreeSet::from([(8 << 32) | 4713]),
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }
}

#[cfg(test)]
mod sent_volunteer_tests {
    use super::*;
    #[test]
    fn oracle_rejects_reordering_missing_and_extra_references() {
        let expected = BTreeMap::from([(b"AAA".to_vec(), vec![11, 0, 22])]);
        let mut report = Report::default();
        compare_sent_volunteers(&expected, &expected, &mut report);
        assert!(report.differences.is_empty());
        for handles in [vec![22, 0, 11], vec![11, 22], vec![11, 0, 22, 33]] {
            let mut report = Report::default();
            compare_sent_volunteers(
                &BTreeMap::from([(b"AAA".to_vec(), handles)]),
                &expected,
                &mut report,
            );
            assert!(!report.differences.is_empty());
        }
    }
}

#[cfg(test)]
mod manpower_tests {
    use super::*;
    #[test]
    fn manpower_expeditionary_oracle_rejects_changed_order_and_missing_capture() {
        use replay_data::rows::CountryCounterInputRow;
        use replay_data::rows::Recorded;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let (mut frame, mut saved, _, _) = crate::fixtures::fixture();
        let country = frame.complete::<tables::Countries>().unwrap()[0].clone();
        let references = vec![0, 7, 7];
        saved
            .counter_inputs
            .sent_expeditionaries
            .insert(country.tag.clone(), references.clone());
        for (actual, differences) in [(Some(references), 0), (Some(vec![7, 0, 7]), 1), (None, 1)] {
            *frame.tables[TableId::CountryCounterInputs.index()] =
                TableView::Complete(TableRows::CountryCounterInputs(vec![
                    CountryCounterInputRow {
                        country: country.id,
                        sent_expeditionaries: Recorded::Value(actual.map(Into::into)),
                        ..Default::default()
                    },
                ]));
            let mut report = Report::default();
            sent_expeditionaries(&frame, &saved, &mut report).unwrap();
            assert_eq!(
                report.differences.len(),
                differences,
                "{:?}",
                report.differences
            );
        }
    }
}
