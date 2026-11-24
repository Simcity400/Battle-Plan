use super::*;
use crate::rows::round_trip;

#[test]
fn land_casualty_hourly_deltas_keep_unchanged_pools_and_clear_empty_ones() {
    let first = LandCombatProgressRow {
        combat: 7 << 32 | 62,
        attacker_manpower_loss: 11.into(),
        attacker_equipment_losses: vec![LandEquipmentLoss {
            variant: 1 << 32 | 70,
            amount_raw: 100_001,
        }]
        .into(),
        ..Default::default()
    };
    assert!(first.delta(Some(&first)).unwrap().is_none());
    round_trip(&first);
    let mut next = first.clone();
    *next.attacker_manpower_loss.require_mut("fixture").unwrap() += 1;
    let manpower = next.delta(Some(&first)).unwrap().unwrap();
    assert_eq!(manpower.mask, 1 << 4);
    assert_eq!(
        manpower
            .apply::<LandCombatProgressRow>(Some(first))
            .unwrap(),
        next
    );
    let prior = next.clone();
    next.attacker_equipment_losses
        .require_mut("fixture")
        .unwrap()
        .push(LandEquipmentLoss {
            variant: 2 << 32 | 70,
            amount_raw: 200_003,
        });
    let new_variant = next.delta(Some(&prior)).unwrap().unwrap();
    assert_eq!(new_variant.mask, 1 << 6);
    assert_eq!(
        new_variant
            .apply::<LandCombatProgressRow>(Some(prior))
            .unwrap(),
        next
    );
    let prior = next.clone();
    next.attacker_equipment_losses
        .require_mut("fixture")
        .unwrap()
        .clear();
    let empty = next.delta(Some(&prior)).unwrap().unwrap();
    assert_eq!(empty.mask, 1 << 6);
    assert_eq!(
        empty.apply::<LandCombatProgressRow>(Some(prior)).unwrap(),
        next
    );
    let mut encoder = crate::frame::FrameEncoder::new();
    let mut decoder = crate::frame::FrameDecoder::new();
    for (hour, available) in [(1, true), (2, false), (3, true)] {
        let mut tables = std::array::from_fn(|_| {
            crate::frame::TableView::Unavailable {
                reason: "fixture omitted".into(),
            }
            .into()
        });
        tables[TableId::LandCombatProgress.index()] = if available {
            crate::frame::TableView::Complete(TableRows::LandCombatProgress(vec![next.clone()]))
        } else {
            crate::frame::TableView::Unavailable {
                reason: "read gap".into(),
            }
        }
        .into();
        let encoded = encoder
            .encode(
                crate::frame::DecodedFrame {
                    branch: 1,
                    hour,
                    tables,
                    metrics: Default::default(),
                },
                false,
            )
            .unwrap();
        let decoded = decoder.advance(&encoded).unwrap();
        if available {
            assert_eq!(
                decoded.complete::<tables::LandCombatProgress>().unwrap(),
                &[next.clone()]
            );
        } else {
            assert!(decoded.complete::<tables::LandCombatProgress>().is_err());
        }
    }
}

#[test]
fn technology_occurrences_roundtrip_delta_and_refuse_duplicate_indexes() {
    use crate::frame::{DecodedFrame, FrameDecoder, FrameEncoder, TableView};
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 0..3 {
        let technologies: Vec<_> = (1..=2)
            .map(|database_index| ResearchTechnologyRow {
                country: 7,
                database_index,
                technology_handle: 0xffff_fffe,
                level: database_index,
                progress_raw: i64::from(hour * database_index),
                ..Default::default()
            })
            .collect();
        let definitions: Vec<_> = (1..=2)
            .map(|database_index| TechnologyDefinitionRow {
                database_index,
                technology_handle: 0xffff_fffe,
                id: b"repeat".to_vec(),
                year: 1936 + database_index,
                ..Default::default()
            })
            .collect();
        let mut tables = std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture omitted".into(),
            }
            .into()
        });
        tables[TableId::ResearchTechnologies.index()] =
            TableView::Complete(TableRows::ResearchTechnologies(technologies.clone())).into();
        tables[TableId::TechnologyDefinitions.index()] =
            TableView::Complete(TableRows::TechnologyDefinitions(definitions.clone())).into();
        let frame = encoder
            .encode(
                DecodedFrame {
                    branch: 1,
                    hour,
                    tables,
                    metrics: Default::default(),
                },
                false,
            )
            .unwrap();
        let decoded = decoder.advance(&frame).unwrap();
        assert_eq!(
            decoded.complete::<tables::ResearchTechnologies>().unwrap(),
            technologies
        );
        assert_eq!(
            decoded.complete::<tables::TechnologyDefinitions>().unwrap(),
            definitions
        );
    }
    let duplicate = ResearchTechnologyRow {
        country: 7,
        database_index: 1,
        technology_handle: 9,
        ..Default::default()
    };
    let mut tables = std::array::from_fn(|_| {
        TableView::Unavailable {
            reason: "fixture omitted".into(),
        }
        .into()
    });
    tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(vec![
            duplicate.clone(),
            duplicate,
        ]))
        .into();
    assert!(
        encoder
            .encode(
                DecodedFrame {
                    branch: 1,
                    hour: 3,
                    tables,
                    metrics: Default::default()
                },
                false
            )
            .is_err()
    );
}

#[test]
fn historical_research_frame_skips_changed_snapshots_and_keeps_other_tables() {
    use crate::{codec::put_string, format::FormatVersion, frame::Frame};
    let format = FormatVersion::read(crate::rows::compatibility::WIDGET_CAPTURE_FORMAT).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&10_i32.to_le_bytes());
    bytes.push(1);
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    for table in TableId::ALL {
        if table.last_change().present(format.version()) {
            bytes.extend_from_slice(&0_u64.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&[0; 16]);
    for table in TableId::ALL {
        if !table.last_change().present(format.version()) {
            continue;
        }
        let row = match table {
            TableId::Clock => {
                let row = ClockRow {
                    hour: 10,
                    ..Default::default()
                };
                let mut fields = Vec::new();
                row.encode_fields(&mut fields).unwrap();
                Some((row.key(), fields, ClockRow::SCHEMA, ClockRow::SCHEMA.len()))
            }
            TableId::ResearchTechnologies => {
                let row = ResearchTechnologyRow {
                    country: 7,
                    technology_handle: 99,
                    ..Default::default()
                };
                let mut fields = Vec::new();
                row.encode_fields(&mut fields).unwrap();
                Some((
                    (7_u128 << 32) | 99,
                    fields,
                    ResearchTechnologyRow::SCHEMA,
                    5,
                ))
            }
            TableId::TechnologyDefinitions => {
                let row = TechnologyDefinitionRow {
                    technology_handle: 3,
                    id: b"repeat".to_vec(),
                    ..Default::default()
                };
                let mut fields = Vec::new();
                row.encode_fields(&mut fields).unwrap();
                Some((99, fields, TechnologyDefinitionRow::SCHEMA, 5))
            }
            TableId::ResearchSlots => {
                let row = ResearchSlotRow {
                    country: 7,
                    technology_handle: 99,
                    database_index: Recorded::Value(3),
                    ..Default::default()
                };
                let mut fields = Vec::new();
                row.encode_fields(&mut fields).unwrap();
                Some((row.key(), fields, ResearchSlotRow::SCHEMA, 5))
            }
            TableId::ResearchInputs => {
                let row = ResearchInputRow {
                    country: 7,
                    technology_handle: 99,
                    database_index: Recorded::Value(3),
                    ..Default::default()
                };
                let mut fields = Vec::new();
                row.encode_fields(&mut fields).unwrap();
                Some((row.key(), fields, ResearchInputRow::SCHEMA, 15))
            }
            _ => None,
        };
        if let Some((key, fields, schema, count)) = row {
            bytes.push(0);
            bytes.extend_from_slice(&1_u32.to_le_bytes());
            table.encode_key(key, 0, &mut bytes).unwrap();
            let mut input = Decoder::new(&fields);
            for kind in &schema[..count] {
                kind.decode(&mut input).unwrap();
            }
            bytes.extend_from_slice(&fields[..fields.len() - input.remaining()]);
        } else {
            bytes.push(4);
            put_string(&mut bytes, "fixture omitted").unwrap();
        }
    }
    let mut input = Decoder::new(&bytes);
    let frame = Frame::decode_version(&mut input, None, format).unwrap();
    input.finish().unwrap();
    let mut decoder = crate::frame::FrameDecoder::new();
    let decoded = decoder.advance(&frame).unwrap();
    assert_eq!(decoded.complete::<tables::Clock>().unwrap()[0].hour, 10);
    for table in [
        TableId::ResearchTechnologies,
        TableId::TechnologyDefinitions,
    ] {
        assert!(
            matches!(&*decoded.tables[table.index()], crate::frame::TableView::Unavailable { reason } if reason.contains(&format!("{table:?}")))
        );
    }
    assert_eq!(
        decoded.complete::<tables::ResearchSlots>().unwrap()[0].database_index,
        Recorded::NotRecorded
    );
    assert_eq!(
        decoded.complete::<tables::ResearchInputs>().unwrap()[0].database_index,
        Recorded::NotRecorded
    );
}
#[test]
fn deletion_witness_rejects_unsupported_or_missing_loss_cause_in_table_decoder() {
    use crate::frame::{Frame, FrameDecoder, Metrics, TableUpdate, TableView};

    let row = DivisionEquipmentDeletionRow {
        division_handle: (781_u64 << 32) | 0x1234,
        ..Default::default()
    };
    for value in [27, 255] {
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        *bytes.last_mut().unwrap() = value;
        assert!(
            DivisionEquipmentDeletionRow::decode_fields(row.key(), &mut Decoder::new(&bytes))
                .and_then(|row| row.check_key_fields())
                .is_err()
        );
        let mut delta = row.delta(None).unwrap().unwrap();
        let mut bytes = Vec::new();
        Field::U8(value)
            .change(Field::Empty)
            .unwrap()
            .encode(&mut bytes)
            .unwrap();
        delta.values.truncate(delta.values.len() - 1);
        delta.values.extend(bytes);
        if value == 27 {
            assert!(delta.apply::<DivisionEquipmentDeletionRow>(None).is_err());
        } else {
            let decoded = delta.apply::<DivisionEquipmentDeletionRow>(None).unwrap();
            assert_eq!(decoded.cause, None);
            assert!(decoded.check_key_fields().is_err());
        }
        let table = TableId::DivisionEquipmentDeletions;
        let mut frame = Frame {
            branch: 1,
            hour: 1,
            keyframe: true,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| TableUpdate::Unavailable {
                reason: "isolated deletion witness fixture".into(),
            }),
        };
        frame.tables[table.index()] =
            TableUpdate::Snapshot(TableRows::DivisionEquipmentDeletions(vec![]));
        let mut decoder = FrameDecoder::for_tables(&[table]);
        decoder.advance(&frame).unwrap();
        frame.hour = 2;
        frame.keyframe = false;
        frame.tables[table.index()] = TableUpdate::Delta {
            baseline: 1,
            changed: vec![delta],
            deleted: vec![],
        };
        let rejected = decoder.apply(&frame).unwrap();
        let TableView::Unavailable { reason } = &*rejected.tables[table.index()] else {
            panic!("invalid deletion cause stayed available");
        };
        assert!(reason.contains(if value == 27 {
            "unsupported equipment destruction cause"
        } else {
            "invalid division deletion disposition"
        }));
    }
}
