use super::*;

fn token(tokens: &TokenNames<'_>, id: u32) -> io::Result<Vec<u8>> {
    tokens
        .get(&id)
        .map(|bytes| content_reader_parser::parser::c_string(bytes).to_vec())
        .ok_or_else(|| {
            invalid(format!(
                "doctrine token {id} missing from recorded dictionary"
            ))
        })
}
fn optional_token(tokens: &TokenNames<'_>, id: Option<u32>) -> io::Result<Option<Vec<u8>>> {
    id.map(|id| token(tokens, id)).transpose()
}
fn ordinal(index: usize) -> io::Result<u32> {
    u32::try_from(index).map_err(|_| invalid("saved doctrine ordinal outside native width"))
}

pub(super) fn compare_doctrines(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let statuses = pack
        .doctrine
        .as_ref()
        .ok_or_else(|| invalid("doctrine comparison requires the saved doctrine block"))?;
    let countries = complete::<tables::DoctrineCountries>(frame)?;
    let folders = complete::<tables::DoctrineFolders>(frame)?;
    let tracks = complete::<tables::DoctrineTracks>(frame)?;
    let leaders = complete::<tables::DoctrineLeaderMastery>(frame)?;
    let costs = complete::<tables::DoctrineCostReductions>(frame)?;
    let grants = complete::<tables::DoctrineMasteryGrants>(frame)?;
    let equipment = complete::<tables::DoctrineEquipmentBonuses>(frame)?;
    report.limitation("doctrine effective daily mastery is an unsaved native cache; saved daily_mastery is the unit-source contribution", "DoctrineTracks.effective_daily_mastery");
    let mut expected_countries = Vec::new();
    let mut expected_folders = Vec::new();
    let mut expected_tracks = Vec::new();
    let mut expected_leaders = Vec::new();
    let mut expected_costs = Vec::new();
    let mut expected_grants = Vec::new();
    let mut expected_equipment = Vec::new();
    for (slot, status) in statuses.iter().enumerate() {
        let slot = ordinal(slot)?;
        expected_countries.push((slot, status.country.clone()));
        for (i, folder) in status.folders.iter().enumerate() {
            let i = ordinal(i)?;
            expected_folders.push((
                slot,
                i,
                folder.folder.clone(),
                folder.grand_doctrine.clone(),
                folder.completed_subdoctrines.clone(),
            ));
            for (j, track) in folder.tracks.iter().enumerate() {
                let j = ordinal(j)?;
                expected_tracks.push((
                    slot,
                    i,
                    j,
                    track.sub_doctrine.clone(),
                    track.rewards,
                    track.mastery.raw,
                    track.mastery_bank.raw,
                    track.daily_mastery.raw,
                ));
                for (k, leader) in track.leaders_daily_mastery.iter().enumerate() {
                    expected_leaders.push((
                        slot,
                        i,
                        j,
                        ordinal(k)?,
                        leader.leader,
                        leader.mastery.raw,
                    ));
                }
            }
        }
        for (i, cost) in status.cost_reductions.iter().enumerate() {
            expected_costs.push((
                slot,
                ordinal(i)?,
                cost.as_ref().map(|cost| {
                    (
                        cost.uses,
                        cost.name.clone(),
                        cost.cost_factor.raw,
                        cost.folder.clone(),
                    )
                }),
            ));
        }
        for (i, grant) in status.daily_mastery.iter().enumerate() {
            expected_grants.push((
                slot,
                ordinal(i)?,
                grant.as_ref().map(|grant| {
                    let filter = &grant.relevant_tracks;
                    (
                        grant.name.clone(),
                        (
                            filter.track.clone(),
                            filter.folder.clone(),
                            filter.grand_doctrine.clone(),
                            filter.sub_doctrine.clone(),
                            filter.track_index,
                        ),
                        grant.daily_mastery.raw,
                        grant.bonus.raw,
                        grant.days,
                    )
                }),
            ));
        }
        for (i, bonus) in status.equipment_bonuses.iter().enumerate() {
            expected_equipment.push((
                slot,
                ordinal(i)?,
                bonus.id.clone(),
                bonus.index,
                bonus.equipment_bonus,
            ));
        }
    }
    let actual_countries = countries
        .iter()
        .map(|row| {
            Ok((
                row.slot,
                if row.country as i32 > 0 {
                    Some(required_country_tag(row.country, links)?.to_vec())
                } else {
                    None
                },
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let actual_folders = folders
        .iter()
        .map(|row| {
            Ok((
                row.slot,
                row.ordinal,
                token(tokens, row.folder)?,
                optional_token(tokens, row.grand_doctrine)?,
                row.completed_subdoctrines
                    .iter()
                    .map(|&id| token(tokens, id))
                    .collect::<io::Result<Vec<_>>>()?,
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let actual_tracks = tracks
        .iter()
        .map(|row| {
            Ok((
                row.slot,
                row.folder,
                row.ordinal,
                optional_token(tokens, row.sub_doctrine)?,
                row.rewards,
                row.mastery,
                row.mastery_bank,
                row.daily_mastery,
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let actual_leaders = leaders
        .iter()
        .map(|row| {
            (
                row.slot,
                row.folder,
                row.track,
                row.ordinal,
                ((row.leader >> 32) as u32, row.leader as u32),
                row.mastery,
            )
        })
        .collect();
    let actual_costs = costs
        .iter()
        .map(|row| {
            let valid = row.folder.is_some() && row.uses > 0 && row.cost_factor != 0;
            Ok((
                row.slot,
                row.ordinal,
                if valid {
                    Some((
                        row.uses,
                        content_reader_parser::parser::c_string(&row.name).to_vec(),
                        row.cost_factor,
                        token(tokens, row.folder.expect("valid folder"))?,
                    ))
                } else {
                    None
                },
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let expiry_by_slot: BTreeMap<_, _> = countries
        .iter()
        .map(|country| (country.slot, country.expiry_events))
        .collect();
    let actual_grants = grants
        .iter()
        .map(|row| {
            let filter = &row.relevant_tracks;
            let expiry_events = *expiry_by_slot
                .get(&row.slot)
                .ok_or_else(|| invalid("doctrine grant lacks its country expiry anchor"))?;
            let days = row.days_at(expiry_events);
            Ok((
                row.slot,
                row.ordinal,
                if days > 0 && (row.daily_mastery != 0 || row.bonus != 0) {
                    Some((
                        row.name
                            .as_deref()
                            .map(content_reader_parser::parser::c_string)
                            .map(<[u8]>::to_vec),
                        (
                            optional_token(tokens, filter.track)?,
                            optional_token(tokens, filter.folder)?,
                            optional_token(tokens, filter.grand_doctrine)?,
                            optional_token(tokens, filter.sub_doctrine)?,
                            filter.track_index,
                        ),
                        row.daily_mastery,
                        row.bonus,
                        days,
                    ))
                } else {
                    None
                },
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let actual_equipment = equipment
        .iter()
        .map(|row| Ok((row.slot, row.ordinal, token(tokens, row.id)?, row.index, row.equipment_bonus)))
        .collect::<io::Result<Vec<_>>>()?;
    equal(
        report,
        "doctrine.countries",
        actual_countries,
        expected_countries,
    );
    equal(report, "doctrine.folders", actual_folders, expected_folders);
    equal(report, "doctrine.tracks", actual_tracks, expected_tracks);
    equal(
        report,
        "doctrine.leaders_daily_mastery",
        actual_leaders,
        expected_leaders,
    );
    equal(
        report,
        "doctrine.cost_reduction",
        actual_costs,
        expected_costs,
    );
    equal(
        report,
        "doctrine.daily_mastery",
        actual_grants,
        expected_grants,
    );
    equal(
        report,
        "doctrine.equipment_bonus",
        actual_equipment,
        expected_equipment,
    );
    for (name, count) in [
        ("DoctrineCountries", countries.len()),
        ("DoctrineFolders", folders.len()),
        ("DoctrineTracks", tracks.len()),
        ("DoctrineLeaderMastery", leaders.len()),
        ("DoctrineCostReductions", costs.len()),
        ("DoctrineMasteryGrants", grants.len()),
        ("DoctrineEquipmentBonuses", equipment.len()),
    ] {
        report.compared(name, count);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::*;
    use replay_frame::frame::TableView;

    #[test]
    fn doctrine_oracle_projects_native_leader_type_and_id_after_codec() {
        use engine_land_inputs::doctrine_state as native;

        struct NativeLeader([u8; native::LEADER_STRIDE]);
        impl engine_native_memory::Memory for NativeLeader {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(offset) = address.checked_sub(16) else {
                    return false;
                };
                let Some(end) = offset.checked_add(output.len()) else {
                    return false;
                };
                let Some(bytes) = self.0.get(offset..end) else {
                    return false;
                };
                output.copy_from_slice(bytes);
                true
            }
        }

        let (mut frame, _, _) = super::super::tests::fixture();
        let save = br"countries={} doctrine={countries={{
            folder={{folder=mod_folder tracks={{
                rewards=0 mastery=0 mastery_bank=0 daily_mastery=0
                leaders_daily_mastery={
                    {leader={id=2309737967 type=287454020} mastery=-0.00001}
                    {leader={id=1985229328 type=4294967280} mastery=-0.00002}
                    {leader={id=0 type=305419896} mastery=-0.00003}
                    {leader={id=305419896 type=0} mastery=-0.00004}
                }
            }}}}
        }}}";
        let pack = saved::parse(&content_reader_parser::parser::parse(save).unwrap().items).unwrap();
        let mut leaders = Vec::new();
        for (ordinal, (reference_type, id)) in [
            (0x1122_3344u32, 0x89ab_cdefu32),
            (0xffff_fff0, 0x7654_3210),
            (0x1234_5678, 0),
            (0, 0x1234_5678),
        ]
        .into_iter()
        .enumerate()
        {
            let mastery = -(ordinal as i64 + 1);
            let mut memory = NativeLeader([0; native::LEADER_STRIDE]);
            memory.0[..8].copy_from_slice(&(native::LEADER_VTABLE.0 as u64).to_le_bytes());
            memory.0[8..12].copy_from_slice(&reference_type.to_le_bytes());
            memory.0[12..16].copy_from_slice(&id.to_le_bytes());
            memory.0[16..24].copy_from_slice(&mastery.to_le_bytes());
            let (leader, mastery) = native::leader(&memory, 0, 16).unwrap();
            let row = DoctrineLeaderMasteryRow {
                ordinal: ordinal as u32,
                leader,
                mastery,
                ..Default::default()
            };
            let mut encoded = Vec::new();
            row.encode_fields(&mut encoded).unwrap();
            let mut decoder = replay_data::codec::Decoder::new(&encoded);
            let decoded = DoctrineLeaderMasteryRow::decode_fields(row.key(), &mut decoder).unwrap();
            assert_eq!(decoded, row);
            assert_eq!(decoder.remaining(), 0);
            leaders.push(decoded);
        }
        for rows in [
            TableRows::DoctrineCountries(vec![DoctrineCountryRow::default()]),
            TableRows::DoctrineFolders(vec![DoctrineFolderRow {
                folder: 1,
                ..Default::default()
            }]),
            TableRows::DoctrineTracks(vec![DoctrineTrackRow::default()]),
            TableRows::DoctrineLeaderMastery(leaders),
        ] {
            let id = rows.table();
            *frame.tables[id.index()] = TableView::Complete(rows);
        }
        let tokens = BTreeMap::from([(1, b"mod_folder".as_slice())]);
        let mut report = Report::default();
        compare_doctrines(&frame, &pack, &BTreeMap::new(), &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);

        for changed_word in [1u64, 1u64 << 32] {
            if let TableView::Complete(TableRows::DoctrineLeaderMastery(rows)) =
                &mut *frame.tables[TableId::DoctrineLeaderMastery.index()]
            {
                rows[0].leader ^= changed_word;
            }
            let mut report = Report::default();
            compare_doctrines(&frame, &pack, &BTreeMap::new(), &tokens, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1);
            if let TableView::Complete(TableRows::DoctrineLeaderMastery(rows)) =
                &mut *frame.tables[TableId::DoctrineLeaderMastery.index()]
            {
                rows[0].leader ^= changed_word;
            }
        }
    }

    #[test]
    fn doctrine_oracle_compares_all_saved_operands_and_empty_slots() {
        let (mut frame, _, country) = super::super::tests::fixture();
        let save = br#"countries={} doctrine={countries={{} {country=AAA
            folder={{folder=mod_folder grand_doctrine=mod_grand tracks={{sub_doctrine=mod_sub rewards=2 mastery=1.25 mastery_bank=-0.00001 daily_mastery=0.5 leaders_daily_mastery={{leader={id=4294967295 type=4713} mastery=-0.25}}}} completed_subdoctrines={mod_sub mod_sub}}}
            cost_reduction={{uses=2 name="" cost_factor=-0.35 folder=mod_folder} {}}
            daily_mastery={{name=grant relevant_tracks={folder=mod_folder grand_doctrine=mod_grand sub_doctrine=mod_sub track=mod_track track_index=0} daily_mastery=0.125 bonus=-0.5 days=5} {}}
            equipment_bonus={{id=mod_sub index=-1 equipment_bonus=4}}
        }}}"#;
        let pack = saved::parse(&content_reader_parser::parser::parse(save).unwrap().items).unwrap();
        let tokens = BTreeMap::from([
            (1, b"mod_folder".as_slice()),
            (2, b"mod_grand".as_slice()),
            (3, b"mod_sub".as_slice()),
            (4, b"mod_track".as_slice()),
        ]);
        let rows = [
            TableRows::DoctrineCountries(vec![
                DoctrineCountryRow {
                    slot: 0,
                    country: u32::MAX,
                    ..Default::default()
                },
                DoctrineCountryRow {
                    slot: 1,
                    country: country.id,
                    expiry_events: 2,
                },
            ]),
            TableRows::DoctrineFolders(vec![DoctrineFolderRow {
                slot: 1,
                folder: 1,
                grand_doctrine: Some(2),
                completed_subdoctrines: vec![3, 3],
                ..Default::default()
            }]),
            TableRows::DoctrineTracks(vec![DoctrineTrackRow {
                slot: 1,
                sub_doctrine: Some(3),
                rewards: 2,
                mastery: 125_000,
                mastery_bank: -1,
                daily_mastery: 50_000,
                effective_daily_mastery: 75_000,
                ..Default::default()
            }]),
            TableRows::DoctrineLeaderMastery(vec![DoctrineLeaderMasteryRow {
                slot: 1,
                leader: (u64::from(u32::MAX) << 32) | 4713,
                mastery: -25_000,
                ..Default::default()
            }]),
            TableRows::DoctrineCostReductions(vec![
                DoctrineCostReductionRow {
                    slot: 1,
                    uses: 2,
                    cost_factor: -35_000,
                    folder: Some(1),
                    ..Default::default()
                },
                DoctrineCostReductionRow {
                    slot: 1,
                    ordinal: 1,
                    ..Default::default()
                },
            ]),
            TableRows::DoctrineMasteryGrants(vec![
                DoctrineMasteryGrantRow {
                    slot: 1,
                    name: Some(b"grant".to_vec()),
                    relevant_tracks: DoctrineTrackFilter {
                        track: Some(4),
                        folder: Some(1),
                        grand_doctrine: Some(2),
                        sub_doctrine: Some(3),
                        track_index: Some(0),
                    },
                    daily_mastery: 12_500,
                    bonus: -50_000,
                    days_at_anchor: 7,
                    ..Default::default()
                },
                DoctrineMasteryGrantRow {
                    slot: 1,
                    ordinal: 1,
                    days_at_anchor: 1,
                    ..Default::default()
                },
            ]),
            TableRows::DoctrineEquipmentBonuses(vec![DoctrineEquipmentBonusRow {
                slot: 1,
                id: 3,
                index: -1,
                equipment_bonus: 4,
                ..Default::default()
            }]),
        ];
        for rows in rows {
            let id = rows.table();
            *frame.tables[id.index()] = TableView::Complete(rows);
        }
        let links = BTreeMap::from([(country.id, &country)]);
        let mut report = Report::default();
        compare_doctrines(&frame, &pack, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let mut other_tokens = tokens.clone();
        other_tokens.insert(4, b"mod_other_track".as_slice());
        let mut mismatch = Report::default();
        compare_doctrines(&frame, &pack, &links, &other_tokens, &mut mismatch).unwrap();
        assert_eq!(
            mismatch.differences.len(),
            1,
            "track token spelling must compare exactly"
        );
        for id in [
            TableId::DoctrineCountries,
            TableId::DoctrineFolders,
            TableId::DoctrineTracks,
            TableId::DoctrineLeaderMastery,
            TableId::DoctrineCostReductions,
            TableId::DoctrineMasteryGrants,
            TableId::DoctrineEquipmentBonuses,
        ] {
            let original = frame.tables[id.index()].clone();
            *frame.tables[id.index()] = TableView::Complete(TableRows::empty(id));
            let mut report = Report::default();
            let compared = compare_doctrines(&frame, &pack, &links, &tokens, &mut report);
            if id == TableId::DoctrineCountries {
                assert_eq!(
                    compared.unwrap_err().to_string(),
                    "doctrine grant lacks its country expiry anchor"
                );
            } else {
                compared.unwrap();
                assert_eq!(
                    report.differences.len(),
                    1,
                    "{id:?}: {:?}",
                    report.differences
                );
            }
            frame.tables[id.index()] = original;
        }
        if let TableView::Complete(TableRows::DoctrineTracks(rows)) =
            &mut *frame.tables[TableId::DoctrineTracks.index()]
        {
            rows[0].mastery_bank = 0;
        }
        let mut report = Report::default();
        compare_doctrines(&frame, &pack, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
}
