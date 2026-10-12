//! Synthetic, Rust-owned binary conformance fixtures shared with TypeScript.
//! Every successful fixture is written and validated before it can be exported.
use crate::{errors::Result, model::*, reader, shapes, writer};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const CENTURY_HOURS: u64 = 100 * 365 * 24;
pub const CENTURY_COUNTRY_RUNS: u64 = CENTURY_HOURS * 5_000;

#[derive(Clone, Debug)]
pub struct Fixture {
    pub name: String,
    pub bytes: Vec<u8>,
    pub shape_id: u32,
    pub shape_version: u32,
    pub expected_error: Option<&'static str>,
    pub expected_bounds: Option<Bounds>,
    pub notes: String,
}

#[derive(Default)]
struct Values {
    offsets: BTreeMap<String, Vec<u64>>,
    data: BTreeMap<String, Vec<u8>>,
    states: BTreeMap<(String, u8), Vec<u8>>,
    refs: BTreeMap<String, DictionaryRef>,
    unbound: BTreeSet<String>,
}
fn u16s(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn u32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn u64s(v: &[u64]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn i32s(v: &[i32]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn i64s(v: &[i64]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn f64s(v: &[f64]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn packed(v: &[u64], width: u32) -> Vec<u8> {
    let mut out = vec![0; (v.len() * width as usize).div_ceil(8)];
    for (i, value) in v.iter().enumerate() {
        for bit in 0..width as usize {
            if value & (1u64 << bit) != 0 {
                let position = i * width as usize + bit;
                out[position / 8] |= 1 << (position % 8);
            }
        }
    }
    out
}
fn limbs(values: &[i128]) -> Vec<u8> {
    let mut out = values
        .iter()
        .flat_map(|v| (*v as u128 as u64).to_le_bytes())
        .collect::<Vec<_>>();
    out.extend(
        values
            .iter()
            .flat_map(|v| ((*v as u128 >> 64) as u64).to_le_bytes()),
    );
    out
}
fn dictionary_ref(kind: DictionaryKind, identity: u8, count: u64) -> DictionaryRef {
    DictionaryRef {
        dictionary_identity: [identity; 32],
        shard_identity: [identity.wrapping_add(64); 32],
        first_entry: 0,
        entry_count: count,
        kind,
        flags: 0,
    }
}
fn buffer(reply: &mut Reply, bytes: Vec<u8>) -> u32 {
    let index = reply.buffers.len() as u32;
    reply.buffers.push(bytes);
    index
}
fn default_scalar(d: &Descriptor, length: u64) -> Vec<u8> {
    let n = usize::try_from(length).expect("bounded fixture length");
    if d.physical_encoding == Encoding::BitPacked {
        return packed(&vec![0; n], d.type_param_b);
    }
    let physical_n = if d.physical_encoding == Encoding::RunLength {
        usize::from(n > 0)
    } else {
        n
    };
    if d.logical_type == LogicalType::Safe53 {
        return f64s(&vec![0.0; physical_n]);
    }
    if d.logical_type == LogicalType::WideInt
        || (d.logical_type == LogicalType::FixedPoint && d.type_param_a & 0xff == 128)
    {
        return limbs(&vec![0; physical_n]);
    }
    let width = match d.logical_type {
        LogicalType::Hour | LogicalType::NativeDate => 64,
        LogicalType::FixedPoint => d.type_param_a & 0xff,
        _ => d.type_param_a,
    };
    vec![0; physical_n * (width as usize / 8)]
}

fn fill(reply: &mut Reply, shape: &Shape, values: &Values, index: u32, length: u64, top: bool) {
    let i = index as usize;
    let path = &shape.names[i];
    reply.descriptors[i].length = if top { TOP_LEVEL_LENGTH } else { length };
    let spec = &shape.columns[i];
    let d = spec.descriptor.clone();
    if let Some(kind) = spec.dictionary_kind
        && !values.unbound.contains(path)
    {
        let reference = values
            .refs
            .get(path)
            .cloned()
            .unwrap_or_else(|| dictionary_ref(kind, kind as u8 + 1, 1));
        let r = reply
            .dictionary_refs
            .iter()
            .position(|r| r == &reference)
            .unwrap_or_else(|| {
                reply.dictionary_refs.push(reference);
                reply.dictionary_refs.len() - 1
            });
        reply.descriptors[i].dictionary_ref = r as u32;
    }
    if d.state_layers & 8 != 0 {
        let reference = values
            .refs
            .get(&format!("{path}#refusal"))
            .cloned()
            .unwrap_or_else(|| dictionary_ref(DictionaryKind::Refusal, 40, 301));
        let r = reply
            .dictionary_refs
            .iter()
            .position(|r| r == &reference)
            .unwrap_or_else(|| {
                reply.dictionary_refs.push(reference);
                reply.dictionary_refs.len() - 1
            });
        reply.descriptors[i].refusal_ref = r as u32;
    }
    if d.logical_type == LogicalType::Struct {
        state_buffers(reply, values, path, i, length);
        for child in d.child_first..d.child_first + d.child_count {
            fill(reply, shape, values, child, length, false);
        }
        return;
    }
    if d.logical_type == LogicalType::List {
        let offsets = values.offsets.get(path).cloned().unwrap_or_else(|| {
            let stride = spec.fixed_list_length.unwrap_or(1);
            (0..=length).map(|i| i * stride).collect()
        });
        let child_length = *offsets.last().expect("at least zero offset");
        reply.descriptors[i].aux_buffer = buffer(reply, u64s(&offsets));
        state_buffers(reply, values, path, i, length);
        fill(reply, shape, values, d.child_first, child_length, false);
        return;
    }
    if matches!(d.logical_type, LogicalType::Bytes | LogicalType::Text) {
        let bytes = values.data.get(path).cloned().unwrap_or_else(|| {
            if d.logical_type == LogicalType::Text {
                b"synthetic".repeat(length as usize)
            } else {
                vec![0xa5; length as usize]
            }
        });
        let offsets = values.offsets.get(path).cloned().unwrap_or_else(|| {
            let stride = (bytes.len() as u64).checked_div(length).unwrap_or(0);
            (0..=length).map(|v| v * stride).collect()
        });
        reply.descriptors[i].aux_buffer = buffer(reply, u64s(&offsets));
        reply.descriptors[i].data_buffer = buffer(reply, bytes);
    } else {
        let bytes = values
            .data
            .get(path)
            .cloned()
            .unwrap_or_else(|| default_scalar(&d, length));
        reply.descriptors[i].data_buffer = buffer(reply, bytes);
        if d.physical_encoding == Encoding::RunLength {
            let runs = if length == 0 {
                vec![]
            } else {
                u32s(&[u32::try_from(length).expect("unit-local run")])
            };
            reply.descriptors[i].aux_buffer = buffer(reply, runs);
        }
    }
    state_buffers(reply, values, path, i, length);
}
fn state_buffers(reply: &mut Reply, values: &Values, path: &str, index: usize, length: u64) {
    let layers = reply.descriptors[index].state_layers;
    if layers == 0 {
        return;
    }
    reply.descriptors[index].state_buffer = reply.buffers.len() as u32;
    for layer in 0..4 {
        if layers & (1 << layer) == 0 {
            continue;
        }
        let width = (reply.descriptors[index].state_widths >> (4 * layer)) & 0xf;
        let byte_count = if width == 0 {
            (length as usize).div_ceil(8)
        } else {
            length as usize * (1usize << (width - 1))
        };
        let bytes = values
            .states
            .get(&(path.into(), layer))
            .cloned()
            .unwrap_or_else(|| vec![0; byte_count]);
        buffer(reply, bytes);
    }
}

fn reply(registry: &Vec<Shape>, id: u32, rows: u64, values: Values) -> Reply {
    let s = registry.shape(id).expect("fixture shape exists");
    let mut reply = Reply {
        header: Header {
            query_id: 17,
            campaign_id: [0x51; 16],
            revision: 3,
            generation: 5,
            reply_kind: s.reply_kind,
            shape_id: s.id,
            shape_version: s.version,
            unit_kind: s.unit_kinds[0],
            unit_end: rows,
            row_count: rows,
            total_count: rows,
            ..Header::default()
        },
        descriptors: s.columns.iter().map(|c| c.descriptor.clone()).collect(),
        top_level: s.top_level.clone(),
        bounds: if s.bounds == BoundsRule::Required {
            Some(Bounds {
                kind: if s.reply_kind == ReplyKind::Select {
                    BoundsKind::Rank
                } else {
                    BoundsKind::Hours
                },
                start: 100,
                end: 200,
            })
        } else {
            None
        },
        ..Reply::default()
    };
    for &index in &s.top_level {
        fill(&mut reply, s, &values, index, rows, true);
    }
    reply
}
fn context(reply: &Reply) -> ReadContext {
    ReadContext {
        expected_bounds: reply.bounds,
        require_bounds_match: reply.bounds.is_some(),
    }
}
fn valid(registry: &Vec<Shape>, name: &str, reply: Reply, notes: &str) -> Result<Fixture> {
    Ok(Fixture {
        name: name.into(),
        bytes: writer::write(&reply, registry, context(&reply))?,
        shape_id: reply.header.shape_id,
        shape_version: reply.header.shape_version,
        expected_error: None,
        expected_bounds: reply.bounds,
        notes: notes.into(),
    })
}

/// The exact 14-descriptor, 22-buffer, 1,832-byte embedded legacy warning.
pub fn legacy_warning(registry: &Vec<Shape>, rows: u64) -> Reply {
    let s = registry.shape(shapes::DIAGNOSTICS).expect("diagnostics");
    let mut values = Values::default();
    for (i, column) in s.columns.iter().enumerate() {
        if column.descriptor.state_layers & 2 != 0 {
            values
                .states
                .insert((s.names[i].clone(), 1), vec![2; rows as usize]);
        }
    }
    values.unbound.insert("entity.code".into());
    let message = b"division template names unavailable";
    assert_eq!(message.len(), 35);
    values
        .data
        .insert("message".into(), message.repeat(rows as usize));
    values
        .offsets
        .insert("message".into(), (0..=rows).map(|i| i * 35).collect());
    values.data.insert("detail".into(), vec![]);
    values
        .offsets
        .insert("detail".into(), vec![0; rows as usize + 1]);
    reply(registry, shapes::DIAGNOSTICS, rows, values)
}

fn totals(registry: &Vec<Shape>, diagnostics: u64) -> Result<Reply> {
    let mut values = Values::default();
    values.data.insert("created".into(), u64s(&[12]));
    values.data.insert("lost".into(), u64s(&[3]));
    values.data.insert("transferred".into(), u64s(&[1]));
    let mut out = reply(registry, shapes::DIVISION_LOG_TOTALS, 1, values);
    let diag = legacy_warning(registry, diagnostics);
    out.diagnostics = Some(writer::write(&diag, registry, ReadContext::default())?);
    out.header.diagnostic_total = diagnostics;
    Ok(out)
}
fn chart(registry: &Vec<Shape>, runs: usize, first_run: u64, start: i32) -> Reply {
    let mut values = Values::default();
    values.offsets.insert("runs".into(), vec![0, runs as u64]);
    values.data.insert(
        "runs.$item.first".into(),
        i32s(&(0..runs).map(|i| start + i as i32).collect::<Vec<_>>()),
    );
    values.data.insert(
        "runs.$item.last".into(),
        i32s(&(0..runs).map(|i| start + i as i32).collect::<Vec<_>>()),
    );
    values.data.insert(
        "runs.$item.value".into(),
        f64s(&(0..runs).map(|i| i as f64 * 0.25).collect::<Vec<_>>()),
    );
    values.data.insert(
        "runs.$item.break_before".into(),
        packed(&(0..runs).map(|i| u64::from(i == 0)).collect::<Vec<_>>(), 1),
    );
    values.data.insert(
        "runs.$item.exact".into(),
        limbs(&(0..runs).map(|i| i as i128).collect::<Vec<_>>()),
    );
    values.data.insert("first_run".into(), u64s(&[first_run]));
    // Opaque bytes are interpreted by the separately versioned chart runtime.
    values.data.insert("tree".into(), vec![0x7c; runs * 8]);
    values
        .offsets
        .insert("tree".into(), vec![0, (runs * 8) as u64]);
    values
        .data
        .insert("directory".into(), b"synthetic-directory-v1".to_vec());
    values.offsets.insert("directory".into(), vec![0, 22]);
    reply(registry, shapes::CHART_SERIES, 1, values)
}

fn game_score(registry: &Vec<Shape>) -> Reply {
    let mut v = Values::default();
    v.data.insert("scale".into(), f64s(&[100000.0; 2]));
    v.data.insert("tag".into(), u32s(&[0, 1]));
    v.refs
        .insert("tag".into(), dictionary_ref(DictionaryKind::Tag, 10, 2));
    v.offsets.insert("identity".into(), vec![0, 2, 3]);
    v.data
        .insert("identity.$item.first_hour".into(), i64s(&[100, 150, 100]));
    v.data
        .insert("identity.$item.last_hour".into(), i64s(&[149, 199, 199]));
    v.data
        .insert("identity.$item.value.name".into(), u32s(&[0, 1, 0]));
    v.refs.insert(
        "identity.$item.value.name".into(),
        dictionary_ref(DictionaryKind::Text, 11, 2),
    );
    v.refs.insert(
        "identity.$item.value.name#refusal".into(),
        dictionary_ref(DictionaryKind::Refusal, 12, 1),
    );
    v.states
        .insert(("identity.$item.value.name".into(), 1), vec![0, 0, 0]);
    v.states
        .insert(("identity.$item.value.name".into(), 3), u16s(&[0, 0, 1]));
    v.states.insert(
        ("identity.$item.value.name_missing".into(), 1),
        vec![2, 2, 0],
    );
    v.data.insert(
        "identity.$item.value.name_missing.kind".into(),
        vec![0, 0, 1],
    );
    v.refs.insert(
        "identity.$item.value.name_missing.reason".into(),
        dictionary_ref(DictionaryKind::Refusal, 12, 1),
    );
    v.refs.insert(
        "identity.$item.value.faction.name#refusal".into(),
        dictionary_ref(DictionaryKind::Refusal, 13, 1),
    );
    v.refs.insert(
        "identity.$item.value.faction.name".into(),
        dictionary_ref(DictionaryKind::Text, 11, 2),
    );
    // Canonical country/faction identities, missing structs, colour, and explicit
    // source lists are present rather than a reduced demonstration projection.
    reply(registry, shapes::GAME_SCORE_CATALOGUE, 2, v)
}
fn constructs(registry: &Vec<Shape>) -> Reply {
    let mut v = Values::default();
    v.data.insert("bool".into(), packed(&[0, 1, 1], 1));
    v.data.insert("u8".into(), vec![0, 127, 255]);
    v.data.insert("u16".into(), u16s(&[0, 32768, u16::MAX]));
    v.data.insert("u32".into(), u32s(&[0, 1 << 31, u32::MAX]));
    v.data.insert("u64".into(), u64s(&[0, 1 << 53, u64::MAX]));
    v.data
        .insert("i8".into(), vec![i8::MIN as u8, 0, i8::MAX as u8]);
    v.data.insert(
        "i16".into(),
        [-32768i16, 0, 32767]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect(),
    );
    v.data.insert("i32".into(), i32s(&[i32::MIN, 0, i32::MAX]));
    v.data.insert("i64".into(), i64s(&[i64::MIN, 0, i64::MAX]));
    v.data.insert(
        "safe53".into(),
        f64s(&[-9007199254740992.0, 0.0, 9007199254740992.0]),
    );
    v.data.insert("u128".into(), limbs(&[0, 1i128 << 100, -1]));
    v.data
        .insert("i128".into(), limbs(&[i128::MIN, -1, i128::MAX]));
    v.data.insert("fixed10".into(), i64s(&[-150001, 0, 150001]));
    v.data.insert("fixed2".into(), limbs(&[-32769, 0, 32769]));
    v.data.insert(
        "f32".into(),
        [0x80000000u32, 0x7fc01234, 0x3f800000]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect(),
    );
    v.data.insert(
        "f64".into(),
        [
            0x8000000000000000u64,
            0x7ff8000000001234,
            0x3ff0000000000000,
        ]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect(),
    );
    v.data.insert(
        "hour".into(),
        i64s(&[-1, CENTURY_HOURS as i64, i64::MAX - 1]),
    );
    v.data
        .insert("native_date".into(), i64s(&[i64::MIN, 0, i64::MAX]));
    v.data
        .insert("bytes".into(), vec![0, 0xff, 0x80, 0x41, 0x00, 0xfe]);
    v.offsets.insert("bytes".into(), vec![0, 2, 4, 6]);
    v.data
        .insert("text".into(), "\u{e9}\u{2205}synthetic".as_bytes().to_vec());
    v.offsets.insert("text".into(), vec![0, 2, 5, 14]);
    v.data.insert(
        "wide_dictionary".into(),
        u64s(&[0, (1 << 32) + 1, (1 << 32) + 2]),
    );
    let mut reference = dictionary_ref(DictionaryKind::Plain, 20, (1 << 32) + 3);
    reference.first_entry = (1 << 40) + 7;
    v.refs.insert("wide_dictionary".into(), reference);
    v.data.insert("packed".into(), packed(&[0, 3, 7], 3));
    v.data.insert("rle".into(), i64s(&[-7]));
    v.states.insert(("recorded_known".into(), 0), vec![0, 1, 2]);
    v.states.insert(("recorded_known".into(), 2), vec![0, 0, 1]);
    v.states.insert(("native_option".into(), 1), vec![0, 1, 0]);
    v.states
        .insert(("computed_option".into(), 1), vec![0, 2, 0]);
    v.states
        .insert(("refused".into(), 3), u64s(&[0, 301, (1 << 32) + 1]));
    v.refs.insert(
        "refused#refusal".into(),
        dictionary_ref(DictionaryKind::Refusal, 21, (1 << 32) + 1),
    );
    v.data.insert("union.$kind".into(), vec![0, 1, 0]);
    // Deliberately invalid code in inactive payloads must be ignored.
    v.data
        .insert("union.text".into(), u32s(&[u32::MAX, 0, u32::MAX]));
    reply(registry, shapes::CONSTRUCTS, 3, v)
}

/// Complete successful corpus, including every registered slice projection.
pub fn valid_fixtures() -> Result<Vec<Fixture>> {
    let r = shapes::registry();
    let mut out = vec![];
    let total = valid(
        &r,
        "c12-1-totals-diagnostics",
        totals(&r, 1)?,
        "Exact Appendix C.12.1 byte walk: 2384 bytes, embedded diagnostics 1832 bytes.",
    )?;
    assert_eq!(total.bytes.len(), 2384);
    out.push(total);
    out.push(valid(&r, "c12-2-game-score-refusal", game_score(&r), "Complete catalogue columns; two synthetic countries, three identity runs, separately bound name and faction refusal shards.")?);
    out.push(valid(
        &r,
        "c12-chart-two-runs-directory",
        chart(&r, 2, 0, 100),
        "Native i32 run first/last and mandatory series directory; one chunk row, two exact runs.",
    )?);
    out.push(valid(
        &r,
        "c12-unequal-data-diagnostics",
        totals(&r, 3)?,
        "One data row and three independent diagnostic records.",
    )?);
    let mut error = legacy_warning(&r, 1);
    error.header.flags |= FLAG_ERROR;
    let diagnostics = r.shape(shapes::DIAGNOSTICS).expect("diagnostics shape");
    let message = diagnostics
        .names
        .iter()
        .position(|name| name == "message")
        .expect("message column");
    let message_bytes = b"generation lease is no longer held".to_vec();
    error.buffers[error.descriptors[message].data_buffer as usize] = message_bytes.clone();
    error.buffers[error.descriptors[message].aux_buffer as usize] =
        u64s(&[0, message_bytes.len() as u64]);
    let code_shard = crate::dictionary::write_shard(
        DictionaryKind::DiagnosticCode,
        0,
        &[crate::dictionary::Page {
            entries: vec![b"stale-generation".to_vec()],
        }],
    )?;
    error.dictionary_refs[0] = DictionaryRef {
        dictionary_identity: *blake3::hash(b"synthetic-wire-fixture-diagnostic-codes").as_bytes(),
        shard_identity: code_shard.identity,
        first_entry: 0,
        entry_count: 1,
        kind: DictionaryKind::DiagnosticCode,
        flags: 0,
    };
    out.push(valid(&r, "c12-diagnostics-only-error", error, "Standalone complete shared diagnostics: code0 resolves to the shared stale-generation shard, with an exact synthetic lease message and no invented witnesses.")?);
    for page in 0..2 {
        let mut v = Values::default();
        v.offsets.insert("nested".into(), vec![0, 2]);
        v.offsets.insert("nested.$item".into(), vec![0, 2, 4]);
        v.offsets
            .insert("nested.$item.$item".into(), vec![0, 1, 2, 3, 4]);
        let mut p = reply(&r, shapes::CONSTRUCTS, 1, v);
        p.header.unit_start = 40;
        p.header.unit_end = 41;
        p.header.total_count = 50;
        p.bounds = Some(Bounds {
            kind: BoundsKind::Hours,
            start: 100,
            end: 200,
        });
        let s = r.shape(shapes::CONSTRUCTS).expect("constructs");
        let column = s.names.iter().position(|s| s == "nested").expect("nested") as u32;
        p.extents.push(Extent {
            column,
            coordinate: Coordinate::Element,
            final_extent: page == 1,
            index: 0,
            first: page * 2,
            end: page * 2 + 2,
            total: 4,
        });
        let inner = s
            .names
            .iter()
            .position(|s| s == "nested.$item")
            .expect("nested item") as u32;
        p.extents.push(Extent {
            column: inner,
            coordinate: Coordinate::Element,
            final_extent: false,
            index: 1,
            first: 8 + page * 2,
            end: 10 + page * 2,
            total: 15,
        });
        if page == 0 {
            p.header.flags = FLAG_CONTINUATION | FLAG_BOUNDED;
            p.continuation = Some(b"synthetic-authenticated-token".to_vec());
        }
        out.push(valid(&r, &format!("c12-window-nested-page-{page}"), p,
            "Both pages retain request bounds [100,200); nested child extents continue one original row 40.")?);
    }
    out.push(valid(&r, "all-constructs-exact-values-states-unions", constructs(&r), "All logical types, all encodings, all optional layers, inactive unions, exact 64/128-bit limbs, fixed-point, NaN payloads, wide codes and refusal codes above 255.")?);
    for s in &r {
        if matches!(
            s.id,
            shapes::DIAGNOSTICS
                | shapes::CHART_SERIES
                | shapes::DIVISION_LOG_TOTALS
                | shapes::GAME_SCORE_CATALOGUE
                | shapes::CONSTRUCTS
        ) {
            continue;
        }
        out.push(valid(
            &r,
            &format!("slice-{}", s.name.replace('.', "-")),
            reply(&r, s.id, 2, Values::default()),
            "Every field in this product projection, including nested descendants, round-trips.",
        )?);
    }
    let mut century = chart(&r, 2, CENTURY_COUNTRY_RUNS, CENTURY_HOURS as i32 - 2);
    century.bounds = Some(Bounds {
        kind: BoundsKind::Hours,
        start: CENTURY_HOURS as i64 - 2,
        end: CENTURY_HOURS as i64,
    });
    century.header.unit_start = CENTURY_COUNTRY_RUNS;
    century.header.unit_end = CENTURY_COUNTRY_RUNS + 1;
    century.header.total_count = CENTURY_COUNTRY_RUNS + 1;
    out.push(valid(&r,"century-5000-country-original-run-positions",century,
        "100 years Ã— 365 days Ã— 24 hours Ã— 5000 countries = 4,380,000,000 run positions; small requested window, u64 original row and run positions.")?);
    let mut maximum = chart(&r, 2, u64::MAX - 2, i32::MAX - 1);
    maximum.header.total_count = UNKNOWN_TOTAL;
    maximum.bounds = Some(Bounds {
        kind: BoundsKind::Hours,
        start: i32::MAX as i64 - 1,
        end: i32::MAX as i64 + 1,
    });
    out.push(valid(&r,"native-hour-extreme-and-u64-run-count",maximum,
        "Native i32 chart hour domain and exclusive i64 bounds; first_run near u64 maximum, independent of reply size.")?);
    let mut minimum = chart(&r, 2, 0, i32::MIN);
    minimum.bounds = Some(Bounds {
        kind: BoundsKind::Hours,
        start: i32::MIN as i64,
        end: i32::MIN as i64 + 2,
    });
    out.push(valid(&r,"native-chart-hour-signed-minimum",minimum,
        "Minor77: negative native chart hours use signed Int32Array views without truncation or BigInt boxing.")?);
    let mut countries = Values::default();
    countries
        .data
        .insert("tag".into(), u32s(&(0..5000).collect::<Vec<_>>()));
    countries
        .refs
        .insert("tag".into(), dictionary_ref(DictionaryKind::Tag, 30, 5000));
    countries
        .data
        .insert("rank".into(), u64s(&(0..5000).collect::<Vec<_>>()));
    countries.data.insert(
        "identities.$item.slot".into(),
        u32s(&(0..5000).collect::<Vec<_>>()),
    );
    countries.data.insert(
        "identities.$item.country_id".into(),
        u32s(&(5000..10000).collect::<Vec<_>>()),
    );
    out.push(valid(
        &r,
        "5000-country-rank-window",
        reply(&r, shapes::EQUIPMENT_PRODUCED_RANK, 5000, countries),
        "5000 countries are requested; shape is sized from rows, with no chosen country cap.",
    )?);
    out.push(valid(&r,"viewport-chart-16384-runs",chart(&r,16384,0,100),
        "Large bounded chart chunk for decode-cost measurement; viewport/window sizing is a caller policy.")?);
    let n = 16_384u64;
    let mut focus = Values::default();
    focus.offsets.insert("history.segments".into(), vec![0, n]);
    focus.offsets.insert("history.progress".into(), vec![0, n]);
    focus.data.insert(
        "history.segments.$item.first_hour".into(),
        i64s(&(100..100 + n as i64).collect::<Vec<_>>()),
    );
    focus.data.insert(
        "history.segments.$item.last_hour".into(),
        i64s(&(100..100 + n as i64).collect::<Vec<_>>()),
    );
    focus.data.insert(
        "history.segments.$item.order.completed.$item.ordinal".into(),
        u64s(&(0..n).collect::<Vec<_>>()),
    );
    focus.data.insert(
        "history.progress.$item.first_hour".into(),
        i64s(&(100..100 + n as i64).collect::<Vec<_>>()),
    );
    focus.data.insert(
        "history.progress.$item.last_hour".into(),
        i64s(&(100..100 + n as i64).collect::<Vec<_>>()),
    );
    let mut focus = reply(&r, shapes::NATIONAL_FOCUS_SERIES, 1, focus);
    focus.bounds = Some(Bounds {
        kind: BoundsKind::Hours,
        start: 100,
        end: 100 + n as i64,
    });
    out.push(valid(&r,"window-focus-16384-segments",focus,
        "Bounded window with 16384 segments, completion entries and exact progress runs for product-shape decode measurement.")?);
    let mut exact = Values::default();
    exact.offsets.insert("runs".into(), vec![0, n]);
    exact.data.insert(
        "runs.$item.first".into(),
        i32s(&(100..100 + n as i32).collect::<Vec<_>>()),
    );
    exact.data.insert(
        "runs.$item.last".into(),
        i32s(&(100..100 + n as i32).collect::<Vec<_>>()),
    );
    for path in [
        "operands.first_hours",
        "operands.last_hours",
        "operands.reads",
        "operands.observations",
        "operands.values.$item.values.deltas",
        "operands.total.deltas",
    ] {
        exact.offsets.insert(path.into(), vec![0, n]);
    }
    for path in [
        "operands.values.$item.values.deltas.$item",
        "operands.total.deltas.$item",
    ] {
        exact.data.insert(
            path.into(),
            limbs(
                &(0..n)
                    .map(|i| {
                        if i % 2 == 0 {
                            1i128 << 80
                        } else {
                            -(1i128 << 81)
                        }
                    })
                    .collect::<Vec<_>>(),
            ),
        );
    }
    let mut produced = reply(&r, shapes::EQUIPMENT_PRODUCED_SERIES, 1, exact);
    produced.bounds = Some(Bounds {
        kind: BoundsKind::Hours,
        start: 100,
        end: 100 + n as i64,
    });
    out.push(valid(&r,"window-produced-16384-exact-samples",produced,
        "Registered version2 product shape with exact totals above2^53 and signed full-width differences; geometry and supplying/read operands remain separate.")?);
    let mut byte_page = reply(&r, shapes::FOCUS_ARTWORK_DESCRIPTION, 1, Values::default());
    let body = r
        .shape(shapes::FOCUS_ARTWORK_DESCRIPTION)
        .expect("description")
        .names
        .iter()
        .position(|n| n == "description")
        .expect("body") as u32;
    byte_page.extents.push(Extent {
        column: body,
        coordinate: Coordinate::Byte,
        final_extent: false,
        index: 0,
        first: CENTURY_COUNTRY_RUNS,
        end: CENTURY_COUNTRY_RUNS + 9,
        total: CENTURY_COUNTRY_RUNS + 100,
    });
    out.push(valid(&r,"century-byte-extent-original-offset",byte_page,
        "Nine requested UTF8 bytes at an original byte offset exceedingu32; total and extents stayu64.")?);
    let mut raw_page = reply(&r, shapes::CONSTRUCTS, 3, Values::default());
    let raw_shape = r.shape(shapes::CONSTRUCTS).expect("constructs");
    let raw = raw_shape
        .names
        .iter()
        .position(|n| n == "bytes")
        .expect("bytes") as u32;
    raw_page.extents.push(Extent {
        column: raw,
        coordinate: Coordinate::Byte,
        final_extent: false,
        index: 1,
        first: CENTURY_COUNTRY_RUNS,
        end: CENTURY_COUNTRY_RUNS + 1,
        total: UNKNOWN_TOTAL,
    });
    out.push(valid(&r,"century-arbitrary-byte-extent-unknown-total",raw_page,
        "Arbitrary byte value continuation with a u64 original byte offset and an explicitly undefined total.")?);
    for page in 0..2u64 {
        let mut p = reply(&r, shapes::EQUIPMENT_PRODUCED_RANK, 50, Values::default());
        p.header.unit_start = page * 50;
        p.header.unit_end = (page + 1) * 50;
        p.header.total_count = 100;
        if page == 0 {
            p.header.flags = FLAG_CONTINUATION | FLAG_BOUNDED;
            p.continuation = Some(b"synthetic-next-row-page".to_vec());
        }
        out.push(valid(&r,&format!("hour-window-row-page-{page}"),p,
            "Two distinct row pages [0,50) and [50,100) share normalized hour request bounds [100,200).")?);
    }
    Ok(out)
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("fixture field"))
}
fn le64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("fixture field"))
}
fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn section(bytes: &[u8], kind: u32) -> (usize, usize) {
    for i in 0..le32(bytes, 60) as usize {
        let at = 128 + i * 24;
        if le32(bytes, at) == kind {
            return (at, le64(bytes, at + 8) as usize);
        }
    }
    panic!("fixture section missing: {kind}")
}
fn descriptor(bytes: &[u8], index: usize) -> usize {
    section(bytes, 1).1 + index * 64
}
fn data_at(bytes: &[u8], index: usize) -> usize {
    let d = descriptor(bytes, index);
    let b = le32(bytes, d + 28) as usize;
    let s = section(bytes, 3).1;
    s + le64(bytes, s + b * 16) as usize
}
fn aux_at(bytes: &[u8], index: usize) -> usize {
    let d = descriptor(bytes, index);
    let b = le32(bytes, d + 40) as usize;
    let s = section(bytes, 3).1;
    s + le64(bytes, s + b * 16) as usize
}
fn checksum(bytes: &mut [u8]) {
    let hash = xxhash_rust::xxh3::xxh3_64(&bytes[128..]);
    put64(bytes, 120, hash);
}
fn mutation(
    out: &mut Vec<Fixture>,
    base: &Fixture,
    name: &str,
    error: &'static str,
    change: impl FnOnce(&mut Vec<u8>),
) {
    let mut fixture = base.clone();
    fixture.name = name.into();
    fixture.expected_error = Some(error);
    fixture.notes = "Malformed bytes: validation must refuse before publication.".into();
    change(&mut fixture.bytes);
    checksum(&mut fixture.bytes);
    out.push(fixture);
}

/// Malformed corpus is generated from valid Rust-written bytes and has stable
/// refusal names. All body mutations retain a correct checksum except the
/// dedicated checksum fixture.
pub fn fixtures() -> Result<Vec<Fixture>> {
    let mut out = valid_fixtures()?;
    let total = out
        .iter()
        .find(|f| f.name == "c12-1-totals-diagnostics")
        .expect("totals")
        .clone();
    mutation(&mut out, &total, "bad-magic", "invalid-magic", |b| b[0] = 0);
    mutation(
        &mut out,
        &total,
        "bad-wire-version",
        "unknown-schema-version",
        |b| put16(b, 4, 2),
    );
    mutation(&mut out, &total, "bad-flags", "invalid-flags", |b| {
        put16(b, 6, 0x1001)
    });
    mutation(
        &mut out,
        &total,
        "bad-header-reserved",
        "invalid-flags",
        |b| put32(b, 68, 1),
    );
    mutation(
        &mut out,
        &total,
        "bad-total-length",
        "invalid-length",
        |b| put64(b, 112, 1),
    );
    let mut bad_checksum = total.clone();
    bad_checksum.name = "bad-checksum".into();
    bad_checksum.expected_error = Some("checksum-mismatch");
    bad_checksum.bytes[120] ^= 1;
    out.push(bad_checksum);
    mutation(
        &mut out,
        &total,
        "bad-section-kind",
        "unknown-section-kind",
        |b| put32(b, 128, 99),
    );
    mutation(
        &mut out,
        &total,
        "bad-section-duplicate",
        "duplicate-section",
        |b| put32(b, 152, 1),
    );
    mutation(
        &mut out,
        &total,
        "bad-section-alignment",
        "section-alignment",
        |b| put64(b, 136, 249),
    );
    mutation(
        &mut out,
        &total,
        "bad-section-overlap",
        "section-overlap",
        |b| put64(b, 160, 248),
    );
    mutation(
        &mut out,
        &total,
        "bad-section-bounds",
        "section-bounds",
        |b| put64(b, 144, u64::MAX),
    );
    mutation(
        &mut out,
        &total,
        "bad-descriptor-reserved",
        "descriptor-reserved",
        |b| {
            let d = descriptor(b, 0);
            put32(b, d + 60, 1)
        },
    );
    mutation(
        &mut out,
        &total,
        "bad-shape-version",
        "unknown-shape-version",
        |b| put32(b, 56, 9),
    );
    mutation(&mut out, &total, "bad-shape-id", "unknown-shape", |b| {
        put32(b, 52, 999999)
    });
    mutation(&mut out, &total, "bad-row-unit", "unit-range", |b| {
        put64(b, 80, 2)
    });
    mutation(&mut out, &total, "bad-total-count", "unit-range", |b| {
        put64(b, 96, 0)
    });
    mutation(
        &mut out,
        &total,
        "bad-bounds-order",
        "invalid-bounds",
        |b| {
            let s = section(b, 9).1;
            put64(b, s + 8, 201)
        },
    );
    mutation(&mut out, &total, "bad-bounds-kind", "invalid-bounds", |b| {
        let s = section(b, 9).1;
        put32(b, s, 9)
    });
    mutation(
        &mut out,
        &total,
        "bad-bounds-padding",
        "invalid-bounds",
        |b| {
            let s = section(b, 9).1;
            put32(b, s + 4, 1)
        },
    );
    mutation(
        &mut out,
        &total,
        "bad-bounds-request-match",
        "bounds-mismatch",
        |b| {
            let s = section(b, 9).1;
            put64(b, s + 8, 101)
        },
    );
    mutation(
        &mut out,
        &total,
        "bad-bounds-length",
        "invalid-bounds",
        |b| {
            let (directory, payload) = section(b, 9);
            put64(b, directory + 16, 16);
            b[payload + 16..payload + 24].fill(0)
        },
    );
    mutation(
        &mut out,
        &total,
        "bad-buffer-alignment",
        "buffer-alignment",
        |b| {
            let s = section(b, 3).1;
            put64(b, s, 49)
        },
    );
    mutation(
        &mut out,
        &total,
        "bad-buffer-bounds",
        "buffer-bounds",
        |b| {
            let s = section(b, 3).1;
            put64(b, s + 8, u64::MAX)
        },
    );
    mutation(
        &mut out,
        &total,
        "bad-column-buffer-length",
        "buffer-length",
        |b| {
            let s = section(b, 3).1;
            put64(b, s + 8, 7)
        },
    );
    mutation(&mut out, &total, "bad-top-level", "descriptor-index", |b| {
        let s = section(b, 2).1;
        put32(b, s, 99)
    });
    mutation(
        &mut out,
        &total,
        "bad-diagnostics-identity",
        "diagnostics-identity",
        |b| {
            let s = section(b, 5).1;
            put64(b, s + 8, 18);
            let hash = xxhash_rust::xxh3::xxh3_64(&b[s + 128..]);
            put64(b, s + 120, hash)
        },
    );
    mutation(
        &mut out,
        &total,
        "bad-diagnostics-kind",
        "invalid-diagnostics",
        |b| {
            let s = section(b, 5).1;
            put32(b, s + 48, ReplyKind::Series as u32)
        },
    );
    mutation(
        &mut out,
        &total,
        "bad-diagnostics-short-embedding",
        "invalid-diagnostics",
        |b| {
            let (directory, payload) = section(b, 5);
            put64(b, directory + 16, 64);
            b.truncate(payload + 64);
            put64(b, 112, (payload + 64) as u64)
        },
    );
    let all = out
        .iter()
        .find(|f| f.name == "all-constructs-exact-values-states-unions")
        .expect("constructs")
        .clone();
    let registry = shapes::registry();
    let shape = registry.shape(shapes::CONSTRUCTS).expect("shape");
    let named = |name: &str| {
        shape
            .names
            .iter()
            .position(|n| n == name)
            .expect("named field")
    };
    mutation(
        &mut out,
        &all,
        "bad-safe53-fraction",
        "invalid-safe53",
        |b| {
            let at = data_at(b, named("safe53"));
            put64(b, at, 0.5f64.to_bits())
        },
    );
    mutation(
        &mut out,
        &all,
        "bad-safe53-overflow",
        "invalid-safe53",
        |b| {
            let at = data_at(b, named("safe53"));
            put64(b, at, 18014398509481984.0f64.to_bits())
        },
    );
    mutation(&mut out, &all, "bad-safe53-nan", "invalid-safe53", |b| {
        let at = data_at(b, named("safe53"));
        put64(b, at, f64::NAN.to_bits())
    });
    mutation(&mut out, &all, "bad-utf8", "invalid-utf8", |b| {
        let at = data_at(b, named("text"));
        b[at] = 0xff
    });
    mutation(&mut out, &all, "bad-offset-order", "offsets-order", |b| {
        let at = aux_at(b, named("bytes"));
        put64(b, at + 16, 1)
    });
    mutation(&mut out, &all, "bad-offset-bounds", "offsets-bounds", |b| {
        let at = aux_at(b, named("bytes"));
        put64(b, at + 24, 7)
    });
    mutation(
        &mut out,
        &all,
        "bad-dictionary-code",
        "dictionary-code",
        |b| {
            let at = data_at(b, named("wide_dictionary"));
            put64(b, at, u64::MAX)
        },
    );
    mutation(
        &mut out,
        &all,
        "bad-dictionary-canonical-overflow",
        "dictionary-overflow",
        |b| {
            let d = descriptor(b, named("wide_dictionary"));
            let r = le32(b, d + 16) as usize;
            let at = section(b, 4).1 + r * 88;
            put64(b, at + 64, u64::MAX - 1)
        },
    );
    mutation(
        &mut out,
        &all,
        "bad-state-code",
        "invalid-state-code",
        |b| {
            let d = descriptor(b, named("native_option"));
            let index = le32(b, d + 44) as usize;
            let at = section(b, 3).1;
            let offset = le64(b, at + index * 16) as usize;
            b[at + offset] = 3
        },
    );
    mutation(
        &mut out,
        &all,
        "bad-refusal-state",
        "dictionary-code",
        |b| {
            let d = descriptor(b, named("refused"));
            let index = le32(b, d + 44) as usize;
            let at = section(b, 3).1;
            let offset = le64(b, at + index * 16) as usize;
            put64(b, at + offset, u64::MAX)
        },
    );
    mutation(
        &mut out,
        &all,
        "bad-refusal-binding",
        "refusal-reference",
        |b| {
            let d = descriptor(b, named("refused"));
            put32(b, d + 56, NONE_INDEX)
        },
    );
    mutation(&mut out, &all, "bad-rle-sum", "run-length", |b| {
        let at = aux_at(b, named("rle"));
        put32(b, at, 2)
    });
    mutation(
        &mut out,
        &all,
        "bad-union-active-code",
        "dictionary-code",
        |b| {
            let at = data_at(b, named("union.text"));
            put32(b, at + 4, 9)
        },
    );
    mutation(&mut out, &all, "bad-union-gate", "union-gate", |b| {
        let d = descriptor(b, named("union.text"));
        put16(b, d + 50, 1)
    });
    mutation(&mut out, &all, "bad-child-cycle", "descriptor-cycle", |b| {
        let d = descriptor(b, named("nested"));
        put32(b, d + 20, named("nested") as u32)
    });
    mutation(
        &mut out,
        &all,
        "bad-child-length",
        "descriptor-length",
        |b| {
            let d = descriptor(b, named("union.$kind"));
            put64(b, d + 32, 2)
        },
    );
    let extent = out
        .iter()
        .find(|f| f.name == "c12-window-nested-page-0")
        .expect("extents")
        .clone();
    mutation(
        &mut out,
        &extent,
        "bad-extent-column",
        "invalid-extent",
        |b| {
            let s = section(b, 8).1;
            put32(b, s, 99999)
        },
    );
    mutation(
        &mut out,
        &extent,
        "bad-extent-coordinate",
        "invalid-extent",
        |b| {
            let s = section(b, 8).1;
            b[s + 4] = 2
        },
    );
    mutation(
        &mut out,
        &extent,
        "bad-extent-index",
        "invalid-extent",
        |b| {
            let s = section(b, 8).1;
            put64(b, s + 8, 99)
        },
    );
    mutation(
        &mut out,
        &extent,
        "bad-extent-length",
        "invalid-extent",
        |b| {
            let s = section(b, 8).1;
            put64(b, s + 24, 3)
        },
    );
    mutation(
        &mut out,
        &extent,
        "bad-extent-duplicate",
        "duplicate-extent",
        |b| {
            let s = section(b, 8).1;
            let first = b[s..s + 40].to_vec();
            b[s + 40..s + 80].copy_from_slice(&first)
        },
    );
    mutation(
        &mut out,
        &extent,
        "bad-continuation-flag",
        "continuation-flags",
        |b| put16(b, 6, FLAG_FINAL),
    );
    mutation(
        &mut out,
        &total,
        "bad-section-padding",
        "nonzero-padding",
        |b| b[452] = 1,
    );
    mutation(
        &mut out,
        &all,
        "bad-logical-type",
        "unknown-logical-type",
        |b| {
            let d = descriptor(b, 0);
            put16(b, d + 4, 99)
        },
    );
    mutation(
        &mut out,
        &all,
        "bad-physical-encoding",
        "unknown-encoding",
        |b| {
            let d = descriptor(b, 0);
            b[d + 6] = 99
        },
    );
    mutation(
        &mut out,
        &all,
        "bad-state-width",
        "invalid-state-width",
        |b| {
            let d = descriptor(b, named("native_option"));
            put16(b, d + 48, 0x50)
        },
    );
    mutation(
        &mut out,
        &all,
        "bad-dictionary-kind",
        "dictionary-kind",
        |b| {
            let d = descriptor(b, named("wide_dictionary"));
            let index = le32(b, d + 16) as usize;
            let s = section(b, 4).1;
            put32(b, s + index * 88 + 80, DictionaryKind::Text as u32)
        },
    );
    mutation(
        &mut out,
        &all,
        "bad-refusal-shard-kind",
        "refusal-reference",
        |b| {
            let d = descriptor(b, named("refused"));
            let index = le32(b, d + 56) as usize;
            let s = section(b, 4).1;
            put32(b, s + index * 88 + 80, DictionaryKind::Text as u32)
        },
    );
    mutation(
        &mut out,
        &extent,
        "bad-extent-padding",
        "invalid-extent",
        |b| {
            let s = section(b, 8).1;
            put16(b, s + 6, 1)
        },
    );
    mutation(
        &mut out,
        &extent,
        "bad-extent-final",
        "invalid-extent",
        |b| {
            let s = section(b, 8).1;
            b[s + 5] = 1
        },
    );
    mutation(
        &mut out,
        &extent,
        "bad-extent-total",
        "invalid-extent",
        |b| {
            let s = section(b, 8).1;
            put64(b, s + 32, 1)
        },
    );
    mutation(
        &mut out,
        &total,
        "bad-error-reply-kind",
        "invalid-diagnostics",
        |b| put16(b, 6, FLAG_FINAL | FLAG_ERROR),
    );
    Ok(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Exact buffer bytes are included for meaningful TypeScript parity assertions;
/// the successful corpus also reserializes the borrowed Rust reader result.
pub fn manifest(fixtures: &[Fixture]) -> Result<Value> {
    let registry = shapes::registry();
    let mut records = vec![];
    for fixture in fixtures {
        if let Some(expected) = fixture.expected_error {
            match reader::read(
                &fixture.bytes,
                &registry,
                ReadContext {
                    expected_bounds: fixture.expected_bounds,
                    require_bounds_match: fixture.expected_bounds.is_some(),
                },
            ) {
                Err(error) if error.name == expected => {}
                Err(error) => {
                    return Err(crate::errors::WireError::new(
                        "fixture-error-mismatch",
                        format!("{}: expected {expected}, got {}", fixture.name, error.name),
                    ));
                }
                Ok(_) => {
                    return Err(crate::errors::WireError::new(
                        "fixture-malformed-accepted",
                        fixture.name.clone(),
                    ));
                }
            }
        }
        let expected = if fixture.expected_error.is_none() {
            let read = reader::read(
                &fixture.bytes,
                &registry,
                ReadContext {
                    expected_bounds: fixture.expected_bounds,
                    require_bounds_match: fixture.expected_bounds.is_some(),
                },
            )?;
            let rewritten = writer::write(
                &read.to_owned(),
                &registry,
                ReadContext {
                    expected_bounds: fixture.expected_bounds,
                    require_bounds_match: fixture.expected_bounds.is_some(),
                },
            )?;
            assert_eq!(
                fixture.bytes, rewritten,
                "Rust round-trip: {}",
                fixture.name
            );
            let mut header = serde_json::to_value(&read.header).expect("header JSON");
            for (key, value) in [
                ("query_id", read.header.query_id),
                ("revision", read.header.revision),
                ("generation", read.header.generation),
                ("unit_start", read.header.unit_start),
                ("unit_end", read.header.unit_end),
                ("row_count", read.header.row_count),
                ("total_count", read.header.total_count),
                ("diagnostic_total", read.header.diagnostic_total),
            ] {
                header[key] = json!(value.to_string());
            }
            let descriptors = read
                .descriptors
                .iter()
                .map(|d| {
                    let mut v = serde_json::to_value(d).expect("descriptor JSON");
                    v["length"] = json!(d.length.to_string());
                    v
                })
                .collect::<Vec<_>>();
            let extents = read
                .extents
                .iter()
                .map(|e| {
                    let mut v = serde_json::to_value(e).expect("extent JSON");
                    for (key, value) in [
                        ("index", e.index),
                        ("first", e.first),
                        ("end", e.end),
                        ("total", e.total),
                    ] {
                        v[key] = json!(value.to_string());
                    }
                    v
                })
                .collect::<Vec<_>>();
            let mut occurrences = vec![];
            for (column, d) in read.descriptors.iter().enumerate() {
                if d.logical_type == LogicalType::List || read.column_length(column) == 0 {
                    continue;
                }
                for index in [0, read.column_length(column) - 1] {
                    occurrences.push(json!({"column":column,"index":index.to_string(),
                        "path":read.original_occurrence(column,index)?.into_iter().map(|(column,index)|json!({"column":column,"index":index.to_string()})).collect::<Vec<_>>()}));
                }
            }
            Some(
                json!({"header":header,"buffers":read.buffers.iter().map(|b|hex(b)).collect::<Vec<_>>(),
                "descriptors":descriptors,"extents":extents,"occurrences":occurrences,
                "embeddedBuffers":read.diagnostics.as_ref().map(|d|d.buffers.iter().map(|b|hex(b)).collect::<Vec<_>>())}),
            )
        } else {
            None
        };
        records.push(json!({"name":fixture.name,"file":format!("{}.h4rq",fixture.name),
            "shapeId":fixture.shape_id,"shapeVersion":fixture.shape_version,"expectedError":fixture.expected_error,
            "benchmark":matches!(fixture.name.as_str(),"5000-country-rank-window"|"viewport-chart-16384-runs"|"window-focus-16384-segments"|"window-produced-16384-exact-samples"),
            "expectedBounds":fixture.expected_bounds.map(|b|json!({"kind":b.kind as u32,"start":b.start.to_string(),"end":b.end.to_string()})),
            "notes":fixture.notes,"expected":expected}));
    }
    let mut dictionary_records = vec![];
    for fixture in crate::dictionary::synthetic_fixtures()? {
        let decoded = crate::dictionary::read_shard(&fixture.bytes, fixture.identity);
        let expected = if let Some(expected) = fixture.expected_error {
            match decoded {
                Err(error) if error.name == expected => {}
                Err(error) => {
                    return Err(crate::errors::WireError::new(
                        "fixture-error-mismatch",
                        format!("{}: expected {expected}, got {}", fixture.name, error.name),
                    ));
                }
                Ok(_) => {
                    return Err(crate::errors::WireError::new(
                        "fixture-malformed-accepted",
                        fixture.name.clone(),
                    ));
                }
            }
            None
        } else {
            let decoded = decoded?;
            for (code, entry) in fixture.expected_entries.iter().enumerate() {
                assert_eq!(
                    decoded.entry(code as u64)?,
                    entry,
                    "{} entry {code}",
                    fixture.name
                );
            }
            Some(
                json!({"kind":decoded.kind as u32,"firstEntry":decoded.first_entry.to_string(),
                "entryCount":decoded.entry_count.to_string(),"entries":fixture.expected_entries.iter().map(|e|hex(e)).collect::<Vec<_>>()}),
            )
        };
        dictionary_records.push(json!({"name":fixture.name,"file":format!("{}.h4rd",fixture.name),
            "identity":hex(&fixture.identity),"expectedError":fixture.expected_error,"expected":expected,
            "expectedEntries":fixture.expected_entries.iter().map(|e|hex(e)).collect::<Vec<_>>(),
            "byteExtents":fixture.byte_extents.iter().map(|(code,first,end)|json!({"code":code.to_string(),"first":first.to_string(),"end":end.to_string()})).collect::<Vec<_>>()}));
    }
    Ok(
        json!({"schemaVersion":SCHEMA_VERSION,"shapes":shapes::contracts_json(),"fixtures":records,"dictionaryFixtures":dictionary_records}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_score_fixed_color_lists_accept_first_and_final_slices_by_occurrence() {
        let registry = shapes::registry();
        let shape = registry.shape(shapes::GAME_SCORE_CATALOGUE).unwrap();
        let color = shape
            .names
            .iter()
            .position(|n| n == "identity.$item.value.color")
            .unwrap();
        assert_eq!(shape.columns[color].fixed_list_length, Some(3));
        let mut reply = game_score(&registry);
        let child = reply.descriptors[color].child_first as usize;
        let offsets = reply.descriptors[color].aux_buffer as usize;
        reply.buffers[offsets] = u64s(&[0, 1, 4, 6]);
        reply.descriptors[child].length = 6;
        let data = reply.descriptors[child].data_buffer as usize;
        reply.buffers[data].truncate(6);
        reply.extents = vec![
            Extent {
                column: color as u32,
                coordinate: Coordinate::Element,
                final_extent: true,
                index: 2,
                first: 1,
                end: 3,
                total: 3,
            },
            Extent {
                column: color as u32,
                coordinate: Coordinate::Element,
                final_extent: false,
                index: 0,
                first: 0,
                end: 1,
                total: 3,
            },
        ];
        let bytes = writer::write(&reply, &registry, context(&reply)).expect("fixed color slices");
        let read = reader::read(&bytes, &registry, context(&reply)).expect("fixed color slices");
        assert_eq!(read.column_length(child), 6);
        assert_eq!(read.buffers[offsets], u64s(&[0, 1, 4, 6]));
        assert_eq!(
            read.original_occurrence(child, 5)
                .unwrap()
                .last()
                .unwrap()
                .1,
            2
        );

        // Local slice lengths are still checked independently of the complete total.
        reply.buffers[offsets] = u64s(&[0, 2, 4, 6]);
        assert_eq!(
            writer::write(&reply, &registry, context(&reply))
                .unwrap_err()
                .name,
            "invalid-extent"
        );
        reply.extents.clear();
        assert_eq!(
            writer::write(&reply, &registry, context(&reply))
                .unwrap_err()
                .name,
            "descriptor-length"
        );
    }

    #[test]
    fn game_score_fixed_color_lists_refuse_incompatible_totals_in_writer_and_reader() {
        let registry = shapes::registry();
        let shape = registry.shape(shapes::GAME_SCORE_CATALOGUE).unwrap();
        let color = shape
            .names
            .iter()
            .position(|n| n == "identity.$item.value.color")
            .unwrap();
        let mut unchecked = registry.clone();
        unchecked
            .iter_mut()
            .find(|s| s.id == shape.id)
            .unwrap()
            .columns[color]
            .fixed_list_length = None;
        for total in [4, 5, UNKNOWN_TOTAL] {
            let mut reply = game_score(&registry);
            reply.extents.push(Extent {
                column: color as u32,
                coordinate: Coordinate::Element,
                final_extent: total == 4,
                index: 0,
                first: 1,
                end: 4,
                total,
            });
            assert_eq!(
                writer::write(&reply, &registry, context(&reply))
                    .unwrap_err()
                    .name,
                "descriptor-length"
            );
            // Emit structurally valid bytes without a fixed-length rule to test the reader directly.
            let bytes = writer::write(&reply, &unchecked, context(&reply)).unwrap();
            assert_eq!(
                reader::read(&bytes, &registry, context(&reply))
                    .unwrap_err()
                    .name,
                "descriptor-length"
            );
        }
    }

    #[test]
    fn constructs_text_offsets_cover_complete_utf8_values() {
        let registry = shapes::registry();
        let reply = constructs(&registry);
        let shape = registry
            .shape(shapes::CONSTRUCTS)
            .expect("constructs shape");
        let text = shape.names.iter().position(|n| n == "text").expect("text");
        let descriptor = &reply.descriptors[text];
        assert_eq!(
            reply.buffers[descriptor.data_buffer as usize].as_slice(),
            b"\xc3\xa9\xe2\x88\x85synthetic".as_slice()
        );
        assert_eq!(
            reply.buffers[descriptor.aux_buffer as usize],
            u64s(&[0, 2, 5, 14])
        );
        let bytes = writer::write(&reply, &registry, context(&reply)).expect("constructs fixture");
        let read = reader::read(&bytes, &registry, context(&reply)).expect("constructs text");
        for (index, expected) in ["\u{e9}", "\u{2205}", "synthetic"].iter().enumerate() {
            assert_eq!(
                read.value_bytes(text, index as u64).unwrap(),
                expected.as_bytes()
            );
        }
    }
}
