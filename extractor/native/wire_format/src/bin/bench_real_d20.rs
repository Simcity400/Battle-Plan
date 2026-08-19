//! Encode the real D04 deep-war save's division set both ways: the shipping
//! 120-byte compact row (`world_fold.rs` layout) and this crate's BASELINE, in
//! both codec-measurement contract arms. `bench_layout` measures a modelled month; this measures one
//! real population — 3,168 divisions from an actual 1944 campaign — so the size
//! claim has at least one non-synthetic anchor.
//!
//! Input is the flat file `flatten_divisions.py` writes:
//! `div_id div_type owner_idx location_province template_id org strength`,
//! one line per division, `template_id == -1` when the save resolved none.
//!
//! Fields the save extract does not carry are encoded the way a real capture
//! encodes them: `Unknown` for the three A15 extension fields a legacy row
//! omits, `Null` for the `has_`-gated optionals, `Absent` for move target and
//! progress on a stationary division. The compact-row arm gets the identical
//! information: real values at their real offsets, zeros with the `has_` flag
//! clear elsewhere — zeros compress well, so the bias of that choice favors
//! the OLD format, making the comparison conservative.
//!
//! **Two bounds the stream-harness sweep enforces elsewhere are structurally
//! N/A here, stated so the sweep can tell "checked, none" from "not checked".**
//! (1) The complete-baseline-after-gap rule: this bin encodes ONE
//! baseline from ONE save snapshot — there is no hour sequence, no delta chain,
//! and no `UNCHANGED` run, so no gap can exist for a baseline to be forced
//! across. (2) The cross-domain validation bound: the input is
//! `flatten_divisions.py`'s flat text file, not an H4CR `collector_stream.bin`
//! — no domain's payload, d20's included, is walked here, and nothing is
//! excluded by status. The H4CR-stream measurement for d20 dynamics is
//! `bench_real_dynamics.rs`, which is where those two properties live for this
//! domain.

use std::collections::BTreeMap;
use std::io::Write;

use wire_format::body::{decode_baseline, encode_baseline, key, KeyedState};
use wire_format::columnar::encode_baseline_columnar;
use wire_format::d20;
use wire_format::record::Record;
use wire_format::value::Value;

// `world_fold.rs` division row constants, restated here because the compact-row
// arm writes the layout that code reads: 32-byte table header, 120-byte rows.
const HEADER_LEN: usize = 32;
const ROW_LEN: usize = 120;
const FIXED_SCALE: f64 = 100_000.0;
const FLAG_HAS_OWNER: u32 = 1 << 5;
const FLAG_HAS_TEMPLATE: u32 = 1 << 13;

struct Div {
    div_id: u64,
    div_type: u64,
    owner: u64, // 0 = no owner
    location: u64,
    template: Option<u64>,
    org: f64,
    strength: f64,
}

fn parse(path: &str) -> Vec<Div> {
    let text = std::fs::read_to_string(path).expect("flat division file");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let mut it = line.split_whitespace();
            let mut next = || it.next().expect("seven columns per line");
            let div_id = next().parse().expect("div_id");
            let div_type = next().parse().expect("div_type");
            let owner = next().parse().expect("owner");
            let location = next().parse().expect("location");
            let template: i64 = next().parse().expect("template");
            let org = next().parse().expect("org");
            let strength = next().parse().expect("strength");
            Div {
                div_id,
                div_type,
                owner,
                location,
                template: (template >= 0).then_some(template as u64),
                org,
                strength,
            }
        })
        .collect()
}

/// The shipping format: one compact division table exactly as the collector
/// emits it and `fold_divisions` decodes it.
fn encode_compact_rows(divs: &[Div]) -> Vec<u8> {
    let mut out = vec![0u8; HEADER_LEN + divs.len() * ROW_LEN];
    out[8..12].copy_from_slice(&(divs.len() as u32).to_le_bytes());
    // status_flags at 16 stays 0: success, full baseline.
    for (i, d) in divs.iter().enumerate() {
        let base = HEADER_LEN + i * ROW_LEN;
        let mut put = |off: usize, v: u32| {
            out[base + off..base + off + 4].copy_from_slice(&v.to_le_bytes());
        };
        let mut flags = 0u32;
        if d.owner != 0 {
            flags |= FLAG_HAS_OWNER;
        }
        if d.template.is_some() {
            flags |= FLAG_HAS_TEMPLATE;
        }
        put(0, d.div_id as u32);
        put(4, d.div_type as u32);
        put(8, flags);
        put(12, d.location as u32);
        put(32, (d.org * FIXED_SCALE).round() as u32);
        put(40, (d.strength * FIXED_SCALE).round() as u32);
        put(52, d.template.unwrap_or(0) as u32);
        put(56, d.owner as u32);
        // Everything else: has_ flag clear, operand zero — the layout's own
        // encoding of "not carried this row".
    }
    out
}

fn build_state(collection: &wire_format::schema::Collection, divs: &[Div]) -> KeyedState {
    let mut state = BTreeMap::new();
    for d in divs {
        let values: Vec<Value> = collection
            .value_fields()
            .map(|f| match f.field_id {
                d20::field::OWNER => {
                    if d.owner != 0 {
                        Value::ContentId(d.owner)
                    } else {
                        Value::Null
                    }
                }
                d20::field::LOCATION_PROVINCE => Value::ContentId(d.location),
                d20::field::TEMPLATE_ID => match d.template {
                    Some(t) => Value::Unsigned(t),
                    None => Value::Null,
                },
                d20::field::ORGANISATION => Value::Continuous(d.org),
                d20::field::STRENGTH => Value::Continuous(d.strength),
                // A save snapshot has no movement or combat sampling; a
                // stationary row is what a paused capture would emit.
                d20::field::MOVING | d20::field::IN_COMBAT => Value::Bool(false),
                d20::field::MOVE_TARGET_PROVINCE | d20::field::MOVE_PROGRESS => Value::Absent,
                _ => {
                    if f.unknown_capable {
                        // The A15 extension trio: supported by the schema,
                        // not carried by this source — record-state contract's unknown.
                        Value::Unknown
                    } else {
                        // has_-gated optionals the source did not carry.
                        Value::Null
                    }
                }
            })
            .collect();
        state.insert(key(&[d.div_id, d.div_type]), Record::new(values));
    }
    state
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("usage: bench_real_d20 <flat file> <out dir>");
    let out_dir = args.next().expect("usage: bench_real_d20 <flat file> <out dir>");
    std::fs::create_dir_all(&out_dir).expect("out dir");

    let divs = parse(&input);
    let schema = d20::schema();
    let collection = &schema.collections[0];
    let state = build_state(collection, &divs);
    assert_eq!(state.len(), divs.len(), "every division keyed uniquely");

    let compact = encode_compact_rows(&divs);

    let mut row_major = Vec::new();
    encode_baseline(&schema, std::slice::from_ref(&state), &mut row_major)
        .expect("row-major baseline encodes");
    // The decoder is the honesty check: the bytes measured are bytes a
    // conforming reader accepts and reconstructs to the same state.
    let (back, consumed) = decode_baseline(&schema, &row_major, 0).expect("round-trips");
    assert_eq!(consumed, row_major.len());
    assert_eq!(back[0].len(), state.len());

    let mut column_major = Vec::new();
    encode_baseline_columnar(collection, &state, &mut column_major)
        .expect("columnar baseline encodes");

    for (name, bytes) in [
        ("real_compact_rows.bin", &compact),
        ("real_row_major.bin", &row_major),
        ("real_column_major.bin", &column_major),
    ] {
        let path = std::path::Path::new(&out_dir).join(name);
        let mut f = std::fs::File::create(&path).expect("create output");
        f.write_all(bytes).expect("write output");
    }

    println!("divisions            {}", divs.len());
    println!("compact rows (ship)  {} bytes ({:.1} B/div)", compact.len(), compact.len() as f64 / divs.len() as f64);
    println!("row-major baseline   {} bytes ({:.1} B/div)", row_major.len(), row_major.len() as f64 / divs.len() as f64);
    println!("columnar baseline    {} bytes ({:.1} B/div)", column_major.len(), column_major.len() as f64 / divs.len() as f64);
}
