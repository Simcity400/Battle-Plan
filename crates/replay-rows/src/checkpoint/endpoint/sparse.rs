use super::*;
use crate::rows::checkpoint::Native;
use crate::rows::{Kind, RowCodec};

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
