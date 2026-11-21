use super::*;
use crate::{
    cadence::{Cadence, cadence},
    rows::compatibility::{EFFECT_INPUTS_FORMAT, INTEL_INPUTS_FORMAT},
};

fn old_prefix<T: RowCodec + Fields>(table: TableId, row: &T) -> T {
    let format = FormatVersion::read(EFFECT_INPUTS_FORMAT).unwrap();
    let fields = format.fields(table);
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &T::SCHEMA[..fields] {
        kind.decode(&mut input).unwrap();
    }
    let prefix = &bytes[..bytes.len() - input.remaining()];
    let mut input = Decoder::new(prefix);
    input.format = format.version();
    input.recorded_fields = fields;
    let decoded = T::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.recorded_fields_bytes().unwrap(), prefix.len());
    decoded
}

#[test]
fn intel_inputs_append_without_reinterpreting_membership_or_operation_prefixes() {
    let row = OperativeMembershipRow {
        id: u64::MAX,
        roster_country: 5000,
        roster_kind: 3,
        owner_country: 4999,
        legacy_description_length: 0xfd,
        assignment: 1,
        ordinal: Recorded::Value(5000),
        target_country: Recorded::Value(Some(42)),
        operation: Recorded::Value(Some(u64::MAX)),
        name: Recorded::Value(vec![0xff, 0]),
        cooldown_kind: Recorded::Value(Some(u32::MAX)),
        cooldown_start_hour: Recorded::Value(Some(i32::MIN)),
        cooldown_end_hour: Recorded::Value(Some(i32::MAX)),
        ..Default::default()
    };
    let old = old_prefix(TableId::OperativeMemberships, &row);
    assert_eq!(old.legacy_description_length, row.legacy_description_length);
    assert_eq!(old.ordinal, row.ordinal);
    assert_eq!(old.assignment, 1);
    assert_eq!(old.target_country, Recorded::NotRecorded);
    assert_eq!(old.operation, Recorded::NotRecorded);
    assert_eq!(old.name, Recorded::NotRecorded);
    assert_eq!(old.cooldown_kind, Recorded::NotRecorded);
    assert_eq!(old.cooldown_start_hour, Recorded::NotRecorded);
    assert_eq!(old.cooldown_end_hour, Recorded::NotRecorded);
    assert_eq!(old.not_recorded_fields(), 0b111111 << 9);
    assert!(old.encode_fields(&mut Vec::new()).is_err());

    let old = FormatVersion::read(EFFECT_INPUTS_FORMAT).unwrap();
    assert!(
        old.table_refusal(TableId::Operations)
            .unwrap()
            .starts_with("Operations ")
    );
    assert!(
        FormatVersion::CURRENT
            .table_refusal(TableId::Operations)
            .is_none()
    );
}

#[test]
fn intel_tables_do_not_change_existing_captured_roster_or_snapshot_cadences() {
    let old = FormatVersion::read(EFFECT_INPUTS_FORMAT).unwrap();
    for table in [TableId::AgencyUpgradeLevels, TableId::OperationCompletions] {
        assert_eq!(table.last_change().introduced, INTEL_INPUTS_FORMAT);
        assert!(old.table_refusal(table).unwrap().contains("not recorded"));
        assert!(FormatVersion::CURRENT.table_refusal(table).is_none());
        assert_eq!(cadence(table), Cadence::Hourly);
    }
    assert_eq!(cadence(TableId::OperativeMemberships), Cadence::Daily(16));
    assert_eq!(
        cadence(TableId::CapturedOperativeReferences),
        Cadence::Daily(16)
    );
    assert_eq!(cadence(TableId::Operations), Cadence::Hourly);
    assert!(
        old.table_refusal(TableId::CapturedOperativeReferences)
            .is_none()
    );
}
