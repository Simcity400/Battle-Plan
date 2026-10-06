use super::*;
use crate::{
    format::FormatVersion,
    rows::{
        self, RowCodec,
        compatibility::{HISTORY, LastChange, PROVEN_FROM},
    },
};

fn legacy_rows(table: TableId, hour: Hour) -> TableRows {
    match table {
        TableId::CountryAppearance => {
            TableRows::CountryAppearance(vec![rows::CountryAppearanceRow {
                country: 1,
                ..Default::default()
            }])
        }
        TableId::DivisionTemplates => {
            TableRows::DivisionTemplates(vec![rows::DivisionTemplateRow {
                template: 1,
                name: b"synthetic template".to_vec(),
                ..Default::default()
            }])
        }
        TableId::Decisions => TableRows::Decisions(vec![rows::DecisionRow {
            country: 1,
            token: b"synthetic_decision".to_vec(),
            ..Default::default()
        }]),
        TableId::DivisionMovement => TableRows::DivisionMovement(vec![rows::DivisionMovementRow {
            division: 1,
            source: 7,
            target: 8,
            ..Default::default()
        }]),
        TableId::Countries => TableRows::Countries(vec![rows::CountryRow {
            id: 1,
            tag: b"AAA".to_vec(),
            ..Default::default()
        }]),
        TableId::LandCombatDetails => TableRows::LandCombatDetails(vec![Default::default()]),
        TableId::NavalCombatDetails => {
            TableRows::NavalCombatDetails(vec![rows::NavalCombatDetailsRow {
                custom_sea: Some(Vec::new()),
                ..Default::default()
            }])
        }
        TableId::NavalCombatResultDetails => {
            TableRows::NavalCombatResultDetails(vec![Default::default()])
        }
        TableId::AdvisorRoles => TableRows::AdvisorRoles(vec![rows::AdvisorRoleRow {
            character: 1,
            country: 1,
            ..Default::default()
        }]),
        TableId::CharacterOperatives => {
            TableRows::CharacterOperatives(vec![rows::CharacterOperativeRow {
                character: 1,
                reference: 1,
                ..Default::default()
            }])
        }
        TableId::OperativeMemberships => {
            TableRows::OperativeMemberships(vec![rows::OperativeMembershipRow {
                id: 1,
                roster_country: 1,
                ..Default::default()
            }])
        }
        TableId::CountryCounterInputs => {
            TableRows::CountryCounterInputs(vec![rows::CountryCounterInputRow {
                country: 1,
                ..Default::default()
            }])
        }
        TableId::VariantDefinitions => {
            TableRows::VariantDefinitions(vec![rows::VariantDefinitionRow {
                id: 1,
                ..Default::default()
            }])
        }
        TableId::ScriptVariables => TableRows::ScriptVariables(vec![rows::ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 1,
            ..Default::default()
        }]),
        TableId::TrainInventory => TableRows::TrainInventory(vec![rows::TrainInventoryRow {
            country: 1,
            ..Default::default()
        }]),
        TableId::UnitLeaders => TableRows::UnitLeaders(vec![Default::default()]),
        TableId::WeatherProvinces => TableRows::WeatherProvinces(vec![Default::default()]),
        TableId::StateFlags => TableRows::StateFlags(vec![Default::default()]),
        TableId::DivisionStatus => TableRows::DivisionStatus(vec![rows::DivisionStatusRow {
            division: 1,
            ..Default::default()
        }]),
        TableId::RaidTooltipOperands => TableRows::RaidTooltipOperands(vec![Default::default()]),
        TableId::Ships => TableRows::Ships(vec![rows::ShipRow {
            id: 1,
            ..Default::default()
        }]),
        _ => match source(1, hour).tables[table.index()].clone().into_inner() {
            TableView::Complete(rows) => rows,
            _ => unreachable!(),
        },
    }
}

// The fixtures contain synthetic rows in the proven historical field prefixes,
// not old row decoders. Appended fields are absent; unchanged prefixes decode.
fn legacy_frame(format: FormatVersion, hour: Hour, first: bool) -> Vec<u8> {
    legacy_frame_with_history(format, hour, first, &HISTORY)
}

fn legacy_frame_with_history(
    format: FormatVersion,
    hour: Hour,
    first: bool,
    history: &[LastChange; HISTORY.len()],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&hour.to_le_bytes());
    bytes.push(u8::from(first));
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    for table in TableId::ALL {
        if history[table.index()].present(format.version()) {
            bytes.extend_from_slice(&0_u64.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    for table in TableId::ALL {
        if !history[table.index()].present(format.version()) {
            continue;
        }
        let rows = legacy_rows(table, hour);
        bytes.push(0);
        bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        bytes.extend(rows.fixture_prefix(history[table.index()].fields(table, format.version())));
    }
    bytes
}

fn legacy_replay(format: u16) -> Replay {
    let mut records = records(1, &[1000]);
    let Record::SessionStart(session) = &mut records[0] else {
        unreachable!()
    };
    session.identity.coverage_policy = 3;
    let current = Replay::from_records(CAMPAIGN.to_vec(), &[records]).unwrap();
    let payload = legacy_frame(FormatVersion::read(format).unwrap(), 1000, true);
    let replay = edited_block(&current, |raw| {
        raw.clear();
        raw.extend_from_slice(&1_u32.to_le_bytes());
        put_bytes(raw, &payload).unwrap();
    });
    let mut bytes = replay.bytes().to_vec();
    bytes[4..6].copy_from_slice(&format.to_le_bytes());
    Replay::open(bytes).unwrap()
}

#[test]
fn compatibility_format94_policy3_reads_appended_tables_with_absent_fields() {
    let replay = legacy_replay(94);
    assert_eq!(replay.format.version(), 94);
    assert_eq!(replay.identity.coverage_policy, 3);
    let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
        panic!("missing hour")
    };
    let countries = frame.complete::<rows::tables::Countries>().unwrap();
    assert_eq!(countries[0].tag, b"AAA");
    assert_eq!(
        frame.complete::<rows::tables::Clock>().unwrap()[0].hour,
        1000
    );
    let expected_gaps = TableId::ALL
        .into_iter()
        .map(|table| {
            usize::from(replay.format.table_refusal(table).is_some())
                + replay.format.field_gaps(table).count()
        })
        .sum::<usize>()
        + replay.format.nested_field_gaps().count();
    assert_eq!(replay.compatibility_gaps().len(), expected_gaps);
    for table in TableId::ALL {
        match replay.format.table_refusal(table) {
            Some(reason) => assert_eq!(
                *frame.tables[table.index()],
                TableView::Unavailable { reason }
            ),
            None => assert!(
                matches!(*frame.tables[table.index()], TableView::Complete(_)),
                "{table:?}"
            ),
        }
    }
    let selected = replay
        .read_block_selected(
            &replay.index[0],
            &[TableId::Clock, TableId::DivisionMovement],
        )
        .unwrap();
    assert!(matches!(
        selected[0].tables[TableId::Clock.index()],
        TableUpdate::Snapshot(_)
    ));
    assert!(
        matches!(&selected[0].tables[TableId::DivisionMovement.index()], TableUpdate::Snapshot(rows) if rows.rows::<rows::tables::DivisionMovement>().unwrap()[0].strategic_action == rows::Recorded::NotRecorded)
    );
    assert_eq!(
        frame.complete::<rows::tables::CountryAppearance>().unwrap()[0].subjects,
        rows::Recorded::NotRecorded
    );
    assert_eq!(
        frame.complete::<rows::tables::DivisionTemplates>().unwrap()[0].name_key,
        rows::Recorded::NotRecorded
    );
    assert_eq!(
        frame.complete::<rows::tables::Decisions>().unwrap()[0].description_seed,
        rows::Recorded::NotRecorded
    );
    assert!(
        replay
            .append(&[])
            .err()
            .unwrap()
            .to_string()
            .contains(&format!("Replay append requires format {FORMAT_VERSION}"))
    );
}

#[test]
fn compatibility_format91_framing_survives_snapshot_delta_truncation_and_unchanged() {
    let format = FormatVersion::read(91).unwrap();
    let table = TableId::DivisionStatus;
    assert_eq!(format.fields(table), 16); // old delta mask was TWO bytes, not three
    let payload = legacy_frame(format, 1000, true);
    let mut decoder = FrameDecoder::new();
    let first = Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap();
    decoder.advance(&first).unwrap();
    for (i, status) in [1, 2, 3, 2, 4].into_iter().enumerate() {
        let hour = 1001 + i as i32;
        let mut update = vec![status];
        match status {
            1 => {
                update.extend_from_slice(&(hour - 1).to_le_bytes());
                update.extend_from_slice(&1_u32.to_le_bytes());
                update.push(1); // key
                update.extend_from_slice(&[0, 0x80]); // historical 16-field mask
                update.push(2); // move_flags += 1, zigzag-encoded
                update.extend_from_slice(&0_u32.to_le_bytes()); // no deletions
            }
            2 => update.extend_from_slice(&(hour - 1).to_le_bytes()),
            3 => {
                update.extend_from_slice(&5_u32.to_le_bytes());
                let rows = legacy_rows(table, hour);
                update.extend_from_slice(&(rows.len() as u32).to_le_bytes());
                update.extend(rows.fixture_prefix(format.fields(table)));
            }
            4 => put_string(&mut update, "synthetic unavailable").unwrap(),
            _ => unreachable!(),
        }
        let mut payload = legacy_frame(format, hour, false);
        let prefix = 37
            + 8 * TableId::ALL
                .into_iter()
                .filter(|table| table.last_change().present(format.version()))
                .count();
        let mut input = Decoder::new(&payload[prefix..]);
        let mut range = 0..0;
        for id in TableId::ALL
            .into_iter()
            .filter(|table| table.last_change().present(format.version()))
        {
            let start = payload.len() - input.remaining();
            assert_eq!(input.u8().unwrap(), 0);
            id.skip_snapshot(format.fields(id), &mut input).unwrap();
            let end = payload.len() - input.remaining();
            if id == table {
                range = start..end;
            }
        }
        input.finish().unwrap();
        payload.splice(range, update);
        let frame = Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap();
        let decoded = decoder.advance(&frame).unwrap();
        assert_eq!(
            decoded.complete::<rows::tables::Clock>().unwrap()[0].hour,
            hour
        );
        match status {
            1 => {
                let rows = decoded.complete::<rows::tables::DivisionStatus>().unwrap();
                assert_eq!(rows[0].move_flags, 1);
                assert_eq!(rows[0].border_combat_stats, rows::Recorded::NotRecorded);
            }
            3 => assert!(
                matches!(&*decoded.tables[table.index()], TableView::Truncated { missing: 5, rows } if rows.rows::<rows::tables::DivisionStatus>().unwrap()[0].border_combat_stats == rows::Recorded::NotRecorded)
            ),
            2 if i == 1 => {
                let rows = decoded.complete::<rows::tables::DivisionStatus>().unwrap();
                assert_eq!(rows[0].move_flags, 1);
                assert_eq!(rows[0].border_combat_stats, rows::Recorded::NotRecorded);
            }
            2 | 4 => assert!(matches!(
                &*decoded.tables[table.index()],
                TableView::Unavailable { .. }
            )),
            _ => unreachable!(),
        }
        assert_eq!(
            format.table_refusal(TableId::RaidTooltipOperands).unwrap(),
            "RaidTooltipOperands layout or meaning changed at format 94; this recording is format 91"
        );
        assert_eq!(
            *decoded.tables[TableId::RaidTooltipOperands.index()],
            TableView::Unavailable {
                reason: format.table_refusal(TableId::RaidTooltipOperands).unwrap()
            }
        );
        assert!(matches!(
            &*decoded.tables[TableId::Ships.index()],
            TableView::Complete(_)
        ));
    }
}

#[test]
fn compatibility_historical_delta_insertion_keeps_the_appended_field_absent() {
    let format = FormatVersion::read(91).unwrap();
    let table = TableId::DivisionStatus;
    let row = rows::DivisionStatusRow {
        division: 2,
        move_flags: 7,
        ..Default::default()
    };
    let mut delta = row.delta(None).unwrap().unwrap();
    let mut input = Decoder::new(&delta.values);
    for kind in &table.schema()[..format.fields(table)] {
        kind.decode_change(&mut input).unwrap();
    }
    delta
        .values
        .truncate(delta.values.len() - input.remaining());
    delta.format = format.version();
    delta.mask = (1 << format.fields(table)) - 1;
    delta.recorded_fields = format.fields(table);
    let inserted = delta.apply::<rows::DivisionStatusRow>(None).unwrap();
    assert_eq!(inserted.move_flags, 7);
    assert_eq!(inserted.border_combat_stats, rows::Recorded::NotRecorded);
    let mut bytes = vec![0x55];
    assert_eq!(
        rows::encode_delta(table, &delta, 0, &mut bytes)
            .unwrap_err()
            .to_string(),
        "historical fields cannot be re-encoded"
    );
    assert_eq!(bytes, [0x55]);
    delta.mask &= !(1 << 15);
    assert!(delta.apply::<rows::DivisionStatusRow>(None).is_err());
}

#[test]
fn compatibility_every_appended_field_is_explicitly_not_recorded() {
    let format = FormatVersion::read(91).unwrap();
    let payload = legacy_frame(format, 1000, true);
    let frame = Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap();
    let decoded = FrameDecoder::new().apply(&frame).unwrap();
    for table in TableId::ALL {
        let appended = table.last_change().appended;
        if appended.is_empty() {
            continue;
        }
        let TableView::Complete(rows) = &*decoded.tables[table.index()] else {
            panic!("{table:?} was withheld");
        };
        assert!(!rows.is_empty(), "{table:?}");
        let expected = appended
            .iter()
            .fold(0, |mask, field| mask | (1 << field.index));
        for row in rows.iter() {
            assert_eq!(row.not_recorded_fields(), expected, "{table:?}");
        }
        assert_eq!(rows.field_gaps().count(), appended.len());
    }
    assert!(frame.table_encoded_sizes().is_ok());
    // Reader compatibility never lets a writer turn an absent field into a value.
    assert!(frame.encode(&mut Vec::new()).is_err());
}

#[test]
fn compatibility_layout_change_still_withholds_the_whole_table() {
    let mut history = HISTORY;
    let table = TableId::CountryAppearance;
    history[table.index()].kind = rows::compatibility::ChangeKind::Layout;
    let format = FormatVersion::read(94).unwrap();
    let payload = legacy_frame_with_history(format, 1000, true, &history);
    let parsed =
        Frame::decode_with_history(&mut Decoder::new(&payload), None, format, &history).unwrap();
    let decoded = FrameDecoder::new().apply(&parsed).unwrap();
    assert!(
        matches!(&*decoded.tables[table.index()], TableView::Unavailable { reason } if reason.starts_with("CountryAppearance layout or meaning changed at format 97"))
    );
    assert!(decoded.complete::<rows::tables::Countries>().is_ok());
}

#[test]
fn compatibility_table_lifetimes_keep_the_old_frame_stream_readable() {
    // Synthetic introduction/retirement independent of the actual table lifetimes.
    let mut history = HISTORY;
    let added = TableId::NavalModifierBreakdowns;
    history[added.index()].introduced = 96;
    history[added.index()].format = 96;
    let retired = TableId::Ships;
    history[retired.index()].retired = Some(96);
    for version in [94, 97] {
        let format = FormatVersion::read(version).unwrap();
        let payload = legacy_frame_with_history(format, 1000, true, &history);
        let mut input = Decoder::new(&payload);
        let frame = Frame::decode_with_history(&mut input, None, format, &history).unwrap();
        input.finish().unwrap();
        let mut decoder = FrameDecoder::new();
        let decoded = decoder.advance(&frame).unwrap();
        assert_eq!(
            decoded.complete::<rows::tables::Clock>().unwrap()[0].hour,
            1000
        );
        assert_eq!(
            *decoded.tables[retired.index()],
            TableView::Unavailable {
                reason: format!("Ships retired at format 96; this recording is format {version}")
            }
        );
        if version == 94 {
            assert_eq!(*decoded.tables[added.index()], TableView::Unavailable {
                reason: "NavalModifierBreakdowns not recorded at format 94 (introduced at format 96)".into()
            });
        } else {
            assert!(matches!(
                &*decoded.tables[added.index()],
                TableView::Complete(_)
            ));
        }
        assert!(matches!(
            &*decoded.tables[TableId::Fleets.index()],
            TableView::Complete(_)
        ));
    }
}

#[test]
fn compatibility_framing_change_refuses_the_whole_file_by_name() {
    let replay = legacy_replay(94);
    let mut bytes = replay.bytes().to_vec();
    bytes[4..6].copy_from_slice(&(PROVEN_FROM - 1).to_le_bytes());
    assert!(
        Replay::open(bytes)
            .err()
            .unwrap()
            .to_string()
            .contains("format 90 predates the proven compatibility history")
    );
    let mut bytes = replay.bytes().to_vec();
    bytes[..4].copy_from_slice(b"BAD!");
    assert_eq!(
        Replay::open(bytes).err().unwrap().to_string(),
        "invalid replay magic"
    );
    let mut records = records(1, &[1000]);
    records.push(Record::Control(ControlRecord::Partial {
        session: 1,
        reason: "synthetic failure".into(),
        failed_hour: None,
    }));
    let current = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    let mut bytes = current.bytes().to_vec();
    bytes[4..6].copy_from_slice(&91_u16.to_le_bytes());
    assert_eq!(
        Replay::open(bytes).err().unwrap().to_string(),
        "Partial layout changed at format 92; this recording is format 91"
    );
}

#[test]
fn compatibility_old_spool_is_readable_but_cannot_be_rewritten_or_mixed() {
    let format = FormatVersion::read(94).unwrap();
    let mut bytes = Vec::new();
    for record in records(1, &[1000]) {
        let mut encoded = encode_record(&record).unwrap();
        if matches!(record, Record::Frame(_)) {
            encoded.truncate(crate::spool::RECORD_HEADER_BYTES);
            encoded.push(2);
            encoded.extend(legacy_frame(format, 1000, true));
            let length = encoded.len() - crate::spool::RECORD_HEADER_BYTES;
            encoded[6..10].copy_from_slice(&(length as u32).to_le_bytes());
            let crc = crc32fast::hash(&encoded[crate::spool::RECORD_HEADER_BYTES..]);
            encoded[10..14].copy_from_slice(&crc.to_le_bytes());
        }
        encoded[4..6].copy_from_slice(&94_u16.to_le_bytes());
        bytes.extend(encoded);
    }
    let recovered = recover_prefix(&bytes);
    assert_eq!(recovered.format, Some(format));
    assert!(recovered.damage.is_none(), "{:?}", recovered.damage);
    let current = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1000])]).unwrap();
    assert!(
        current
            .append_recovered(&[(0, recover_prefix(&bytes))])
            .err()
            .unwrap()
            .to_string()
            .contains("segment 0 is format 94")
    );
    assert!(
        Replay::from_recovered(CAMPAIGN.into(), &[(0, recovered)])
            .err()
            .unwrap()
            .to_string()
            .contains("historical rows are never rewritten")
    );
    bytes.extend(
        encode_record(&Record::Control(ControlRecord::ClockWitness {
            session: 1,
            qpc: 0,
            hour: 1000,
            load_signal: None,
        }))
        .unwrap(),
    );
    assert!(
        recover_prefix(&bytes)
            .damage
            .unwrap()
            .contains("SpoolEnvelope format changed inside a segment")
    );
}

#[test]
#[ignore = "read-only local recordings; BATTLE_PLAN_COMPAT_RECORDINGS supplies paths"]
fn compatibility_real_recordings() {
    let paths = std::env::var("BATTLE_PLAN_COMPAT_RECORDINGS").or_else(|_| std::env::var("HOI4_COMPAT_RECORDINGS")).expect("semicolon-separated paths");
    for path in paths.split(';') {
        let replay = Replay::open(std::fs::read(path).unwrap()).unwrap();
        println!(
            "RECORDING {path} format={} policy={} blocks={} branches={}",
            replay.format.version(),
            replay.identity.coverage_policy,
            replay.index.len(),
            replay.branches.len()
        );
        for gap in replay.compatibility_gaps() {
            println!("COMPAT_GAP {gap}");
        }
        let mut frames = 0;
        let mut gaps = BTreeMap::<TableId, BTreeSet<String>>::new();
        let mut fields = BTreeMap::<String, usize>::new();
        for index in &replay.index {
            let mut decoder = FrameDecoder::new();
            for frame in replay.read_block(index).unwrap() {
                let decoded = decoder.advance(&frame).unwrap();
                frames += 1;
                for table in TableId::ALL {
                    match &*decoded.tables[table.index()] {
                        TableView::Unavailable { reason } => {
                            gaps.entry(table).or_default().insert(reason.clone());
                        }
                        TableView::Truncated { missing, .. } => {
                            gaps.entry(table)
                                .or_default()
                                .insert(format!("{missing} missing rows"));
                        }
                        TableView::Complete(rows) => {
                            for (field, count) in rows.field_gaps() {
                                *fields.entry(format!("{table:?}.{field}")).or_default() += count;
                            }
                        }
                    }
                }
            }
        }
        println!("FRAMES {frames}");
        for (field, count) in fields {
            println!("FIELD_GAP {field} {count} row samples not recorded");
        }
        for (table, reasons) in gaps {
            println!(
                "TABLE_GAP {table:?} {}",
                reasons.into_iter().collect::<Vec<_>>().join(" | ")
            );
        }
    }
}
