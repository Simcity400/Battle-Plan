use super::*;
use dashboard_base::prepared::codec;
use serde::Serialize;
use std::collections::BTreeMap;

/// Bytes as MessagePack's binary.
struct Bytes(Vec<u8>);

impl Serialize for Bytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

#[derive(Serialize)]
struct Inner {
    name: String,
    raw: i128,
    bytes: Bytes,
}

#[derive(Serialize)]
struct Record {
    country: u32,
    realized: i128,
    charge: Option<i128>,
    late: bool,
    ratio: f64,
    inner: Inner,
    list: Vec<Inner>,
    named: BTreeMap<String, u64>,
    numbered: BTreeMap<u32, String>,
    empty: BTreeMap<String, u64>,
    pair: (String, String),
}

fn record(seed: i128) -> Record {
    let inner = |at: i128| Inner {
        name: format!("item {at}"),
        raw: at * -7_000_000_000_000_000_000_000,
        bytes: Bytes(vec![at as u8; (at as usize) % 20]),
    };
    Record {
        country: seed as u32,
        realized: seed - 3,
        charge: (seed % 2 == 0).then_some(i128::MIN + seed),
        late: seed % 3 == 0,
        ratio: seed as f64 / 3.0,
        inner: inner(seed),
        list: (0..seed % 4).map(inner).collect(),
        named: (0..seed % 3).map(|at| (format!("reason {at}"), at as u64)).collect(),
        numbered: (0..seed % 3).map(|at| (at as u32, format!("{at}"))).collect(),
        empty: BTreeMap::new(),
        pair: ("upkeep".into(), String::new()),
    }
}

#[test]
fn records_unpack_to_their_bytes() {
    let mut shapes = Shapes::default();
    let mut named = Named::default();
    let mut out = Vec::new();
    for part in 0..3_i128 {
        let mut payload = Vec::new();
        let mut kept = Vec::new();
        for seed in part * 7..part * 7 + 7 {
            let bytes = codec::encode(&record(seed)).unwrap();
            let start = payload.len();
            shapes.pack(&bytes, &mut payload).unwrap();
            kept.push((bytes, start..payload.len()));
        }
        named.extend(shapes.take()).unwrap();
        for (bytes, range) in kept {
            assert!(range.len() < bytes.len());
            named.unpack(&payload[range], &mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }
    // Later parts name no shape anew: each record's maps were named before.
    let bytes = codec::encode(&record(30)).unwrap();
    shapes.pack(&bytes, &mut Vec::new()).unwrap();
    assert!(shapes.take().is_empty());
}

#[test]
fn damaged_records_are_refused() {
    let named = Named::default();
    let mut out = Vec::new();
    assert!(named.unpack(&[MARK, 3], &mut out).is_err());
    assert!(named.unpack(&[0x92, 0x01], &mut out).is_err());
    assert!(Shapes::default().pack(&[0xc1], &mut Vec::new()).is_err());
}
