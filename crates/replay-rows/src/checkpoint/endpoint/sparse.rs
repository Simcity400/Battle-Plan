//! Each row's sparse endpoint fixture, for the registry's tests of every table.
use super::*;
use crate::rows::checkpoint::Native;
use crate::rows::{Kind, Recorded, RowCodec};

pub struct SparseFixture<T> {
    pub baseline: T,
    pub source: T,
    pub expected: T,
    pub payload: Vec<u8>,
}

pub trait SparseReference: Row + Fields {
    fn sparse_fixture(&self, field: usize) -> SparseFixture<Self>;
}

pub fn native_bytes<T: Native>(value: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    Native::encode(value, &mut bytes).unwrap();
    bytes
}

pub fn native_copy<T: Native>(value: &T, kind: Kind) -> T {
    let bytes = native_bytes(value);
    let mut input = Decoder::new(&bytes);
    let value = T::decode(kind, &mut input).unwrap();
    input.finish().unwrap();
    value
}

pub fn different<T: Native>(value: &T, kind: Kind, field: usize) -> T {
    let bytes = native_bytes(value);
    for index in 0..bytes.len() {
        for byte in [((field + 1) % 127) as u8, bytes[index] ^ 1]
            .into_iter()
            .chain(0..=255)
        {
            let mut candidate = bytes.clone();
            candidate[index] = byte;
            candidate.resize(candidate.len().max(256), 0);
            if let Ok(value) = T::decode(kind, &mut Decoder::new(&candidate))
                && native_bytes(&value) != bytes
            {
                return value;
            }
        }
    }
    // Zero-width retired values have only one possible prepared representation.
    assert!(
        bytes.is_empty(),
        "no distinct fixture for {} field {field}",
        std::any::type_name::<T>()
    );
    native_copy(value, kind)
}

fn row_bytes<T: Row>(row: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    row.encode_checkpoint_fields(&mut bytes).unwrap();
    bytes
}

pub fn check_sparse_masks<T: SparseReference>(row: &T) {
    for field in 0..T::SCHEMA.len() {
        let SparseFixture {
            baseline,
            source,
            expected,
            payload,
        } = row.sparse_fixture(field);
        let mask = 1 << field;
        let mut encoded = Vec::new();
        source.encode_endpoint_fields(mask, &mut encoded).unwrap();
        assert_eq!(
            encoded,
            payload,
            "{} schema payload {field}",
            std::any::type_name::<T>()
        );
        let mut input = Decoder::new(&encoded);
        let decoded = T::decode_endpoint_fields(row.key(), mask, &mut input).unwrap();
        input.finish().unwrap();
        let mut actual = RowCodec::try_clone_validated(&baseline).unwrap();
        actual.install_endpoint_fields(decoded, mask).unwrap();
        assert_eq!(
            row_bytes(&actual),
            row_bytes(&expected),
            "{} selected assignment and unselected preservation {field}",
            std::any::type_name::<T>()
        );
        if !payload.is_empty() {
            assert_ne!(
                row_bytes(&expected),
                row_bytes(&baseline),
                "{} selected field {field} must change",
                std::any::type_name::<T>()
            );
        }
    }
}

impl<T: SparseReference> SparseReference for std::sync::Arc<T>
where
    std::sync::Arc<T>: Row + Fields,
{
    fn sparse_fixture(&self, field: usize) -> SparseFixture<Self> {
        let fixture = self.as_ref().sparse_fixture(field);
        SparseFixture {
            baseline: Self::new(fixture.baseline),
            source: Self::new(fixture.source),
            expected: Self::new(fixture.expected),
            payload: fixture.payload,
        }
    }
}

impl SparseReference for crate::rows::EventFeedHistoryRow {
    fn sparse_fixture(&self, field: usize) -> SparseFixture<Self> {
        use crate::{event_feed_rows::*, value::Value};
        let baseline = Self {
            generation: 7,
            read_hour: 100,
            coverage: EventFeedCoverage::Partial,
            counts: None,
            outstanding: 3,
            failure: Some(b"before".to_vec()),
            script_refusals: 1,
            events: vec![],
            script_tokens: b"old".to_vec(),
            script_scopes: vec![],
        };
        let generation = if field == 6 { 7 } else { 9 };
        let source = Self {
            generation,
            read_hour: 200,
            coverage: EventFeedCoverage::Unavailable,
            counts: (field == 3).then_some(EventFeedCounts {
                acquired: 11,
                settled: 8,
                failed: 0,
                rejected: 1,
            }),
            outstanding: if field == 3 { 3 } else { 5 },
            failure: Some(b"after".to_vec()),
            script_refusals: 2,
            events: vec![EventFeedEvent {
                sequence: 11,
                generation,
                hour: 50,
                source: EventFeedSource::Annex,
                actor: EventFeedCountry::default(),
                target: EventFeedCountry::default(),
                cause: Recorded::NotRecorded,
                script: None,
            }],
            script_tokens: b"new".to_vec(),
            script_scopes: vec![EventFeedScope {
                country: 2,
                ..Default::default()
            }],
        };
        baseline.validate().unwrap();
        source.validate().unwrap();
        let mut expected = baseline.clone();
        let mut payload = Vec::new();
        macro_rules! assign {
            ($member:ident) => {{
                expected.$member = source.$member.clone();
                Value::encode(&source.$member, &mut payload).unwrap();
            }};
        }
        match field {
            0 => assign!(generation),
            1 => assign!(read_hour),
            2 => assign!(coverage),
            3 => assign!(counts),
            4 => assign!(outstanding),
            5 => assign!(failure),
            6 => assign!(events),
            7 => assign!(script_tokens),
            8 => assign!(script_scopes),
            9 => assign!(script_refusals),
            _ => unreachable!(),
        }
        expected.validate().unwrap();
        SparseFixture {
            baseline,
            source,
            expected,
            payload,
        }
    }
}
