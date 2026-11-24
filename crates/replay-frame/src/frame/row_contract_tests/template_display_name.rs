use super::*;
use crate::value::Value;
use crate::{
    format::FormatVersion,
    frame::{DecodedFrame, FrameDecoder, FrameEncoder, Metrics, TableView},
    rows::{Fields, RowCodec, TableId, TableRows, tables},
};

fn icon(value: TemplateNameValue) -> DivisionTemplateIconRow {
    DivisionTemplateIconRow {
        template: (7000_u64 << 32) | 52,
        display_name: Recorded::Value(Some(TemplateDisplayName {
            sample_hour: PacketHour::Host,
            value,
        })),
        formation_division_count: Recorded::Value(Some(-7)),
        ..Default::default()
    }
}
fn daily() -> DivisionTemplateRow {
    DivisionTemplateRow {
        template: (7000_u64 << 32) | 52,
        name: b"Dictionary".to_vec(),
        name_key: Recorded::Value(b"OLD_KEY".to_vec()),
        ..Default::default()
    }
}

#[test]
fn canonical_nested_tags_and_demand_round_trip_losslessly() {
    for value in [
        TemplateNameValue::SameAsDivisionTemplates,
        TemplateNameValue::Changed {
            literal: vec![b'A', 0, 255],
            fallback_key: None,
        },
        TemplateNameValue::Changed {
            literal: Vec::new(),
            fallback_key: Some(Vec::new()),
        },
        TemplateNameValue::Changed {
            literal: Vec::new(),
            fallback_key: Some(b"KEY\0tail".to_vec()),
        },
    ] {
        let mut bytes = Vec::new();
        Value::encode(&value, &mut bytes).unwrap();
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(TemplateNameValue::decode(&mut decoder).unwrap(), value);
        decoder.finish().unwrap();
    }
    assert!(TemplateNameValue::decode(&mut Decoder::new(&[2])).is_err());
    for value in [
        TemplateNameValue::Changed {
            literal: b"literal".to_vec(),
            fallback_key: Some(Vec::new()),
        },
        TemplateNameValue::Changed {
            literal: Vec::new(),
            fallback_key: None,
        },
    ] {
        assert!(Value::encode(&value, &mut Vec::new()).is_err());
    }
}

#[test]
fn historical_prefix_preserves_every_format99_icon_field() {
    let row = icon(TemplateNameValue::SameAsDivisionTemplates);
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let format = FormatVersion::read(crate::rows::compatibility::EFFECT_INPUTS_FORMAT).unwrap();
    assert_eq!(format.fields(TableId::DivisionTemplateIcons), 14);
    let mut scan = Decoder::new(&bytes);
    for kind in &DivisionTemplateIconRow::SCHEMA[..14] {
        kind.decode(&mut scan).unwrap();
    }
    let prefix = &bytes[..bytes.len() - scan.remaining()];
    let mut decoder = Decoder::new(prefix);
    decoder.format = format.version();
    decoder.recorded_fields = 14;
    let decoded = DivisionTemplateIconRow::decode_fields(row.key(), &mut decoder).unwrap();
    decoder.finish().unwrap();
    assert_eq!(decoded.priority_unit, row.priority_unit);
    assert_eq!(decoded.priority_unit_name, row.priority_unit_name);
    assert_eq!(decoded.custom_icon, row.custom_icon);
    assert_eq!(decoded.custom_icon_nato, row.custom_icon_nato);
    assert_eq!(decoded.representative, row.representative);
    assert_eq!(decoded.representative_local, row.representative_local);
    assert_eq!(decoded.subunits, row.subunits);
    assert_eq!(decoded.country, row.country);
    assert_eq!(decoded.foreign_country, row.foreign_country);
    assert_eq!(decoded.model_category, row.model_category);
    assert_eq!(decoded.model_entity, row.model_entity);
    assert_eq!(decoded.excluded_variants, row.excluded_variants);
    assert_eq!(decoded.model_unreadable, row.model_unreadable);
    assert_eq!(
        decoded.formation_division_count,
        row.formation_division_count
    );
    assert_eq!(decoded.display_name, Recorded::NotRecorded);
    assert_eq!(decoded.grids, Recorded::NotRecorded);
    assert_eq!(decoded.base_width, Recorded::NotRecorded);
    let absent = DivisionTemplateIconRow::NAMES
        .iter()
        .enumerate()
        .skip(14)
        .fold(0_u64, |mask, (field, _)| mask | (1_u64 << field));
    assert_eq!(decoded.not_recorded_fields(), absent);
}

#[test]
fn current_template_icon_name_layout_and_settings_round_trip_independently() {
    let mut row = icon(TemplateNameValue::SameAsDivisionTemplates);
    row.layout = Recorded::Value(Some(crate::rows::TemplateLayout::default().into()));
    row.settings = Recorded::Value(Some(
        crate::rows::TemplateSettings {
            priority: -7,
            forbidden_equipment: Some(vec![19, 23]),
            ..Default::default()
        }
        .into(),
    ));
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut decoder = Decoder::new(&bytes);
    let decoded = DivisionTemplateIconRow::decode_fields(row.key(), &mut decoder).unwrap();
    decoder.finish().unwrap();
    assert_eq!(decoded, row);
    assert_eq!(decoded.not_recorded_fields(), 0);
}

#[test]
fn fresh_same_frame_certificate_has_no_guessed_daily_origin() {
    let row = icon(TemplateNameValue::SameAsDivisionTemplates);
    let dictionary = daily();
    assert_eq!(
        row.display_name_inputs(77, Some(&dictionary))
            .unwrap()
            .literal,
        b"Dictionary"
    );
    assert!(row.display_name_inputs(78, Some(&dictionary)).is_ok());
    let mut historical = row.clone();
    if let Recorded::Value(Some(packet)) = &mut historical.display_name {
        packet.sample_hour = PacketHour::Stamped(77);
    }
    assert!(
        historical
            .display_name_inputs(77, Some(&dictionary))
            .is_ok()
    );
    assert!(
        historical
            .display_name_inputs(78, Some(&dictionary))
            .unwrap_err()
            .contains("not current")
    );
    assert!(row.display_name_inputs(77, None).is_err());
    let foreign = DivisionTemplateRow {
        template: 52,
        ..dictionary
    };
    assert!(row.display_name_inputs(77, Some(&foreign)).is_err());
    let changed = icon(TemplateNameValue::Changed {
        literal: b"New".to_vec(),
        fallback_key: None,
    });
    assert_eq!(
        changed.display_name_inputs(77, None).unwrap().literal,
        b"New"
    );
}

#[test]
fn keyframe_direct_seek_keeps_dictionary_dependency_and_override() {
    for value in [
        TemplateNameValue::SameAsDivisionTemplates,
        TemplateNameValue::Changed {
            literal: b"Rename at hour77".to_vec(),
            fallback_key: None,
        },
    ] {
        let row = icon(value);
        let mut frame = DecodedFrame {
            branch: 7,
            hour: 77,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "unused fixture table".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(vec![row.clone()]));
        *frame.tables[TableId::DivisionTemplates.index()] =
            TableView::Complete(TableRows::DivisionTemplates(vec![daily()]));
        let wire = FrameEncoder::new().encode(frame, true).unwrap();
        let decoded = FrameDecoder::new().apply(&wire).unwrap();
        let rows = decoded.complete::<tables::DivisionTemplateIcons>().unwrap();
        let daily = decoded.complete::<tables::DivisionTemplates>().unwrap();
        assert_eq!(
            rows[0].display_name_inputs(77, Some(&daily[0])),
            row.display_name_inputs(77, Some(&daily[0]))
        );
    }
}
