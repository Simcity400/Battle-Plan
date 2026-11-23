use crate::rows::{RelationLinkRow, TransportSubjectAccess, ZeroSubjectInputs};
type Fact<T> = Result<T, String>;

pub fn current(
    rows: &[RelationLinkRow],
    owner_raw: u32,
    owner_slot: u32,
    target_slot: u32,
    hour: i32,
) -> Fact<Option<&TransportSubjectAccess>> {
    let row = |slot| -> Fact<Option<&RelationLinkRow>> {
        let mut found = rows
            .iter()
            .filter(|row| row.country == owner_raw && row.target_index == slot);
        let value = found.next();
        if found.next().is_some() {
            return Err("transport subject status key duplicated".into());
        }
        Ok(value)
    };
    let certificate = row(owner_slot)?
        .ok_or("transport subject owner coverage not recorded")?
        .transport_subject
        .require("RelationLinks.transport_subject")?
        .as_ref()
        .ok_or("transport subject owner coverage refused")?;
    if !certificate.sample_hour.fresh_at(hour) {
        return Err("transport subject owner coverage stale".into());
    }
    if !certificate.complete_owner
        || certificate.target_raw.is_some()
        || certificate.subject.is_some()
    {
        return Err("transport subject owner coverage malformed".into());
    }
    let Some(row) = row(target_slot)? else {
        return Ok(None);
    };
    let packet = row
        .transport_subject
        .require("RelationLinks.transport_subject")?
        .as_ref()
        .ok_or("transport subject status capture refused")?;
    if !packet.sample_hour.fresh_at(hour) {
        return Err("transport subject status stale".into());
    }
    if packet.complete_owner {
        return Err("transport subject reached owner certificate as a status".into());
    }
    Ok(Some(packet))
}

pub fn matches(
    subject: &ZeroSubjectInputs,
    target: Fact<u32>,
    mut canonical: impl FnMut(u32) -> Fact<u32>,
) -> Fact<bool> {
    let parties = match (subject.relation_type, subject.parties) {
        (None, None) => return Ok(false),
        (Some(kind), None) if kind != engine_native::crelation::subject::TYPE_VALUE => {
            return Ok(false);
        }
        (Some(kind), Some(parties)) if kind == engine_native::crelation::subject::TYPE_VALUE => {
            parties
        }
        _ => return Err("transport subject native branches malformed".into()),
    };
    let mut equal = |party: Option<u32>| -> Fact<bool> {
        let party = party.ok_or("transport subject endpoint refused or unreached")?;
        let target = target.clone()?;
        Ok(party == target
            || (party != 0 && target != 0 && canonical(party)? == canonical(target)?))
    };
    let first = equal(parties[0]);
    if first == Ok(true) {
        return Ok(true);
    }
    let second = equal(parties[1]);
    if second == Ok(true) {
        return Ok(true);
    }
    first?;
    second
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        codec::Decoder,
        rows::{Recorded, RowCodec, TableId, tables},
    };
    #[test]
    fn absent_pair_is_false_only_with_current_complete_owner_certificate() {
        let mut rows = vec![RelationLinkRow {
            country: 11,
            target_index: 1,
            transport_subject: Recorded::Value(Some(TransportSubjectAccess {
                sample_hour: crate::rows::PacketHour::Stamped(7),
                complete_owner: true,
                ..Default::default()
            })),
            ..Default::default()
        }];
        assert_eq!(current(&rows, 11, 1, 2, 7), Ok(None));
        assert!(current(&rows, 11, 1, 2, 8).unwrap_err().contains("stale"));
        rows[0].transport_subject = Recorded::NotRecorded;
        assert!(current(&rows, 11, 1, 2, 7).is_err());
        assert!(current(&[], 11, 1, 2, 7).is_err());
    }
    #[test]
    fn raw_zero_alias_and_unreached_second_preserve_native_order() {
        let subject = |parties| ZeroSubjectInputs {
            relation_type: Some(engine_native::crelation::subject::TYPE_VALUE),
            parties: Some(parties),
        };
        assert_eq!(
            matches(&subject([Some(12), None]), Ok(11), |_| Ok(1)),
            Ok(true)
        );
        assert_eq!(
            matches(&subject([Some(0), Some(12)]), Ok(11), |_| Ok(1)),
            Ok(true)
        );
        assert_eq!(
            matches(&subject([Some(0), None]), Ok(0), |_| panic!("raw equality")),
            Ok(true)
        );
        assert_eq!(
            matches(&subject([Some(12), Some(13)]), Ok(0), |_| panic!(
                "zero has no alias comparison"
            )),
            Ok(false)
        );
        assert!(matches(&subject([Some(12), None]), Ok(0), |_| Ok(0)).is_err());
        assert_eq!(
            matches(
                &ZeroSubjectInputs::default(),
                Err("target missing".into()),
                Ok
            ),
            Ok(false)
        );
    }

    #[test]
    fn subject_suffix_codec_delta_and_format99_keep_original_link_byte() {
        let baseline = RelationLinkRow {
            country: 11,
            target_index: 2,
            links: 255,
            transport_subject: Recorded::Value(None),
        };
        let packet = TransportSubjectAccess {
            sample_hour: crate::rows::PacketHour::Host,
            target_raw: Some(u32::MAX),
            subject: Some(ZeroSubjectInputs {
                relation_type: Some(engine_native::crelation::subject::TYPE_VALUE),
                parties: Some([Some(u32::MAX), None]),
            }),
            ..Default::default()
        };
        for value in [
            None,
            Some(packet),
            Some(TransportSubjectAccess {
                sample_hour: crate::rows::PacketHour::Host,
                complete_owner: true,
                ..Default::default()
            }),
        ] {
            let next = RelationLinkRow {
                transport_subject: Recorded::Value(value),
                ..baseline.clone()
            };
            let mut bytes = Vec::new();
            next.encode_fields(&mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(
                RelationLinkRow::decode_fields(next.key(), &mut input).unwrap(),
                next
            );
            input.finish().unwrap();
            if next != baseline {
                let delta = next.delta(Some(&baseline)).unwrap().unwrap();
                assert_eq!(delta.mask, 1 << 1);
                assert_eq!(delta.apply(Some(baseline.clone())).unwrap(), next);
            }
        }
        let version = crate::rows::compatibility::EFFECT_INPUTS_FORMAT;
        let fields = crate::format::FormatVersion::read(version)
            .unwrap()
            .fields(TableId::RelationLinks);
        assert_eq!(fields, 1);
        let prefix = [baseline.links];
        let mut input = Decoder::new(&prefix);
        input.format = version;
        input.recorded_fields = fields;
        let historical = RelationLinkRow::decode_fields(baseline.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(historical.links, baseline.links);
        assert_eq!(historical.transport_subject, Recorded::NotRecorded);
        assert_eq!(historical.not_recorded_fields(), 1 << 1);
        assert_eq!(historical.recorded_fields_bytes().unwrap(), 1);
        assert!(historical.encode_fields(&mut Vec::new()).is_err());
    }

    #[test]
    fn subject_suffix_history_retains_prior_hour_and_gap_drops_the_baseline() {
        use crate::frame::{
            DecodedFrame, FrameDecoder, FrameEncoder, Metrics, TableUpdate, TableView,
        };
        use crate::rows::TableRows;
        let row = |_hour: i32, subject| RelationLinkRow {
            country: 11,
            target_index: 2,
            links: 8,
            transport_subject: Recorded::Value(Some(TransportSubjectAccess {
                sample_hour: crate::rows::PacketHour::Host,
                target_raw: Some(22),
                subject,
                ..Default::default()
            })),
        };
        let source = |hour, rows| {
            let mut frame = DecodedFrame {
                branch: 1,
                hour,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|_| {
                    TableView::Unavailable {
                        reason: "undemanded".into(),
                    }
                    .into()
                }),
            };
            frame.tables[TableId::RelationLinks.index()] =
                TableView::Complete(TableRows::RelationLinks(rows)).into();
            frame
        };
        let first_row = row(7, Some(ZeroSubjectInputs::default()));
        let next_row = row(8, None);
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let first = decoder
            .apply(
                &encoder
                    .encode(source(7, vec![first_row.clone()]), true)
                    .unwrap(),
            )
            .unwrap();
        let next = decoder
            .apply(
                &encoder
                    .encode(source(8, vec![next_row.clone()]), false)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            first.complete::<tables::RelationLinks>().unwrap(),
            &[first_row]
        );
        assert_eq!(
            next.complete::<tables::RelationLinks>().unwrap(),
            &[next_row]
        );
        let mut gap = encoder
            .encode(source(9, vec![row(9, None)]), false)
            .unwrap();
        gap.tables[TableId::RelationLinks.index()] = TableUpdate::Unavailable {
            reason: "capture failed".into(),
        };
        assert!(
            decoder
                .apply(&gap)
                .unwrap()
                .complete::<tables::RelationLinks>()
                .is_err()
        );
        let lost = encoder
            .encode(source(10, vec![row(10, None)]), false)
            .unwrap();
        assert!(
            decoder
                .apply(&lost)
                .unwrap()
                .complete::<tables::RelationLinks>()
                .is_err()
        );
        let recovered = decoder
            .apply(
                &encoder
                    .encode(source(11, vec![row(11, None)]), true)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            recovered.complete::<tables::RelationLinks>().unwrap()[0].links,
            8
        );
    }

    #[test]
    fn subject_owner_and_status_refusals_stale_or_duplicate_keys_never_certify_absence() {
        let certificate = RelationLinkRow {
            country: 11,
            target_index: 1,
            transport_subject: Recorded::Value(Some(TransportSubjectAccess {
                sample_hour: crate::rows::PacketHour::Stamped(7),
                complete_owner: true,
                ..Default::default()
            })),
            ..Default::default()
        };
        let mut rows = vec![
            certificate.clone(),
            RelationLinkRow {
                country: 11,
                target_index: 2,
                transport_subject: Recorded::Value(None),
                ..Default::default()
            },
        ];
        assert!(current(&rows, 11, 1, 2, 7).unwrap_err().contains("refused"));
        rows[1].transport_subject = Recorded::Value(Some(TransportSubjectAccess {
            sample_hour: crate::rows::PacketHour::Stamped(6),
            target_raw: Some(22),
            subject: Some(ZeroSubjectInputs::default()),
            ..Default::default()
        }));
        assert!(current(&rows, 11, 1, 2, 7).unwrap_err().contains("stale"));
        rows.push(certificate);
        assert!(
            current(&rows, 11, 1, 3, 7)
                .unwrap_err()
                .contains("duplicated")
        );
    }
}
