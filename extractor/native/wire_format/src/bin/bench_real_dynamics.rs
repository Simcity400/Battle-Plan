//! Measure the wire format against the shipping compact stream on REAL
//! division dynamics — the measurement §13 says is missing.
//!
//! Input: a real `collector_stream.bin` from a d20-scheduled capture
//! (`runs/crashrate/minimal_t*`). The stream's own d20 DATA records are the
//! shipping format's cost for the domain: baseline + row-granular deltas,
//! 120-byte rows, exactly what the recorder stores today. This tool replays
//! those records into per-hour division states — the same fold
//! `world_fold::fold_divisions` performs — and re-encodes every hour with this
//! crate's grammar: a BASELINE at each 48-hour block start, field-granular
//! stored-minimal DELTAs between.
//!
//! Outputs, for the compression step (`scripts/bench_real_dynamics.py`):
//!   ship_d20.bin   — the d20 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision), which is the number the §13 layout model had to assume.

use std::collections::BTreeMap;
use std::io::Write;

use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, key, KeyedState,
};
use wire_format::d20;
use wire_format::record::Record;
use wire_format::value::Value;

// H4CR compact record framing (compact_contract::record_header).
const H4CR_MAGIC: &[u8; 4] = b"H4CR";
const H4CR_HEADER_LEN: usize = 32;
const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
const D20: u16 = 20;

// Division table payload (world_fold.rs fold_divisions).
const TABLE_HEADER_LEN: usize = 32;
const ROW_LEN: usize = 120;
const LEGACY_ROW_LEN: usize = 104;
const STATUS_DELTA_DATA: u32 = 1 << 10;
const STATUS_FAILURE_MASK_APPROX: u32 = 0; // failure statuses abort below via row math
const FIXED_SCALE: f64 = 100_000.0;

// Row flag bits (world_fold.rs).
const F_MOVING: u32 = 1 << 0;
const F_HAS_TARGET: u32 = 1 << 2;
const F_IN_COMBAT: u32 = 1 << 4;
const F_HAS_OWNER: u32 = 1 << 5;
const F_HAS_ARMY: u32 = 1 << 6;
const F_HAS_EXPERIENCE: u32 = 1 << 7;
const F_HAS_MAX_SUPPLY: u32 = 1 << 8;
const F_HAS_CURRENT_SUPPLY: u32 = 1 << 9;
const F_HAS_SUPPLY_GAIN: u32 = 1 << 10;
const F_HAS_FUEL: u32 = 1 << 11;
const F_DELETED: u32 = 1 << 12;
const F_HAS_TEMPLATE: u32 = 1 << 13;
const F_ORDER_ARMY: u32 = 1 << 14;
const F_ORDER_ARMY_GROUP: u32 = 1 << 15;
const F_ORDER_UNRESOLVED: u32 = 1 << 16;
const F_HAS_DISRUPTED_SUPPLY: u32 = 1 << 17;
const F_HAS_STRATEGIC_REDEPLOYMENT: u32 = 1 << 18;
const F_HAS_CURRENT_MANPOWER: u32 = 1 << 19;
const F_HAS_CUMULATIVE_LOSSES: u32 = 1 << 20;
const F_HAS_MAX_ORGANISATION: u32 = 1 << 21;
const F_HAS_DIG_IN: u32 = 1 << 23;
const F_HAS_DIG_IN_CAP: u32 = 1 << 24;

const BLOCK_HOURS: usize = 48;

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[derive(Clone)]
struct ShipRow {
    bytes: [u8; ROW_LEN],
}

impl ShipRow {
    fn flags(&self) -> u32 {
        u32_at(&self.bytes, 8)
    }
    fn fixed(&self, off: usize) -> f64 {
        u32_at(&self.bytes, off) as f64 / FIXED_SCALE
    }
    fn opt_fixed(&self, flag: u32, off: usize) -> Value {
        if self.flags() & flag != 0 {
            Value::Continuous(self.fixed(off))
        } else {
            Value::Null
        }
    }
    fn opt_unsigned(&self, flag: u32, off: usize) -> Value {
        if self.flags() & flag != 0 {
            Value::Unsigned(u32_at(&self.bytes, off) as u64)
        } else {
            Value::Null
        }
    }

    /// The row's stored form in the new grammar, field for field. Presence
    /// flags map to the schema's four states the way `fold_divisions` reads
    /// them: flag clear on an optional field is Null; movement fields are
    /// Absent (not applicable) on a stationary division.
    fn to_record(&self, collection: &wire_format::schema::Collection) -> Record {
        let flags = self.flags();
        let moving = flags & F_MOVING != 0;
        let values: Vec<Value> = collection
            .value_fields()
            .map(|f| match f.field_id {
                d20::field::OWNER => {
                    if flags & F_HAS_OWNER != 0 {
                        Value::ContentId(u32_at(&self.bytes, 56) as u64)
                    } else {
                        Value::Null
                    }
                }
                d20::field::LOCATION_PROVINCE => {
                    Value::ContentId(u32_at(&self.bytes, 12) as u64)
                }
                d20::field::TEMPLATE_ID => self.opt_unsigned(F_HAS_TEMPLATE, 52),
                d20::field::ORGANISATION => Value::Continuous(self.fixed(32)),
                d20::field::STRENGTH => Value::Continuous(self.fixed(40)),
                d20::field::MAX_ORGANISATION => self.opt_fixed(F_HAS_MAX_ORGANISATION, 104),
                d20::field::MOVING => Value::Bool(moving),
                d20::field::MOVE_TARGET_PROVINCE => {
                    if flags & F_HAS_TARGET != 0 {
                        Value::ContentId(u32_at(&self.bytes, 16) as u64)
                    } else {
                        Value::Absent
                    }
                }
                d20::field::MOVE_PROGRESS => {
                    if moving {
                        Value::Continuous(self.fixed(24))
                    } else {
                        Value::Absent
                    }
                }
                d20::field::IN_COMBAT => Value::Bool(flags & F_IN_COMBAT != 0),
                d20::field::EXPERIENCE => self.opt_fixed(F_HAS_EXPERIENCE, 64),
                d20::field::CURRENT_SUPPLY => self.opt_fixed(F_HAS_CURRENT_SUPPLY, 72),
                d20::field::MAX_SUPPLY => self.opt_fixed(F_HAS_MAX_SUPPLY, 68),
                d20::field::SUPPLY_GAIN => self.opt_fixed(F_HAS_SUPPLY_GAIN, 76),
                d20::field::FUEL => self.opt_fixed(F_HAS_FUEL, 80),
                d20::field::DIG_IN => self.opt_fixed(F_HAS_DIG_IN, 112),
                d20::field::DIG_IN_CAP => self.opt_fixed(F_HAS_DIG_IN_CAP, 116),
                d20::field::DISRUPTED_SUPPLY => self.opt_fixed(F_HAS_DISRUPTED_SUPPLY, 88),
                d20::field::STRATEGIC_REDEPLOYMENT => {
                    if flags & F_HAS_STRATEGIC_REDEPLOYMENT != 0 {
                        Value::Bool(u32_at(&self.bytes, 92) != 0)
                    } else {
                        Value::Null
                    }
                }
                d20::field::CURRENT_MANPOWER => self.opt_unsigned(F_HAS_CURRENT_MANPOWER, 96),
                d20::field::CUMULATIVE_LOSSES => {
                    self.opt_unsigned(F_HAS_CUMULATIVE_LOSSES, 100)
                }
                d20::field::ARMY_ID => self.opt_unsigned(F_HAS_ARMY, 60),
                d20::field::ORDER_INSTANCE_ID => self.opt_unsigned(F_ORDER_ARMY, 84),
                d20::field::ORDER_LINK => {
                    if flags & F_ORDER_ARMY != 0 {
                        Value::Enum(0)
                    } else if flags & F_ORDER_ARMY_GROUP != 0 {
                        Value::Enum(1)
                    } else if flags & F_ORDER_UNRESOLVED != 0 {
                        Value::Enum(2)
                    } else {
                        Value::Null
                    }
                }
                _ => Value::Null,
            })
            .collect();
        Record::new(values)
    }
}

/// One d20 DATA record kept by the pass-1 walk: its game hour, whether the
/// record is an incremental update, its rows, and its byte span in the stream.
type ScannedHour = (i32, bool, Vec<ShipRow>, std::ops::Range<usize>);

/// Pass 1: walk the H4CR framing and keep every d20 DATA record.
fn scan_d20_hours(stream: &[u8]) -> Vec<ScannedHour> {
    let mut pos = 0usize;
    let mut hours: Vec<ScannedHour> = Vec::new();
    let mut skipped_legacy = 0usize;
    while pos + H4CR_HEADER_LEN <= stream.len() {
        assert_eq!(&stream[pos..pos + 4], H4CR_MAGIC, "framing lost at {pos}");
        let domain_id = u16::from_le_bytes([stream[pos + 20], stream[pos + 21]]);
        let record_type = u16::from_le_bytes([stream[pos + 22], stream[pos + 23]]);
        let payload_len = u32_at(stream, pos + 24) as usize;
        let game_hour = i32::from_le_bytes([
            stream[pos + 16],
            stream[pos + 17],
            stream[pos + 18],
            stream[pos + 19],
        ]);
        let record_end = pos + H4CR_HEADER_LEN + payload_len;
        assert!(record_end <= stream.len(), "record overruns stream at {pos}");

        if domain_id == D20 && record_type == RECORD_TYPE_DOMAIN_DATA {
            let payload = &stream[pos + H4CR_HEADER_LEN..record_end];
            assert!(payload.len() >= TABLE_HEADER_LEN, "short division table");
            let row_count = u32_at(payload, 8) as usize;
            let status = u32_at(payload, 16);

            // VALIDITY BEFORE CLASSIFICATION. The `delta` decision below reads
            // the status word to classify the v1 vocabulary; it is not a
            // validity predicate. `compact_contract/src/lib.rs:121`-`:123` says
            // so in as many words: "Callers handling untrusted status words must
            // first reject a non-zero `compact_status_unknown_flags_v1` result;
            // otherwise an unassigned bit has no defined classification." Bits
            // 14..=31 are unassigned (`status_flags_base.rs:21`-`:24`), and an
            // unassigned bit "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` is silently
            // read as a full baseline — the delta bit is clear — so the world map
            // is CLEARED and rebuilt from that record's rows, and the hour is
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this walk does the same rather than re-deriving a lenient rule. No
            // capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_contract::compact_status_unknown_flags_v1(status);
            assert!(
                unknown_status == 0,
                "d20 DATA at byte {pos}: hour {game_hour} carries unassigned v1 status bits \
                 {unknown_status:#x}; an unassigned bit has no defined classification"
            );

            let delta = status & STATUS_DELTA_DATA != 0;
            let body = &payload[TABLE_HEADER_LEN..];
            if body.len() == row_count * LEGACY_ROW_LEN && body.len() != row_count * ROW_LEN {
                // The capture is expected to be all-extended; a legacy table
                // would need its own mapping, so it is counted, not guessed at.
                skipped_legacy += 1;
            } else {
                assert_eq!(
                    body.len(),
                    row_count * ROW_LEN,
                    "division table length does not match its row count"
                );
                let rows = (0..row_count)
                    .map(|i| ShipRow {
                        bytes: body[i * ROW_LEN..(i + 1) * ROW_LEN].try_into().unwrap(),
                    })
                    .collect();
                hours.push((game_hour, delta, rows, pos..record_end));
            }
        }
        pos = record_end;
    }
    assert!(pos == stream.len(), "trailing bytes after the last record");
    assert!(skipped_legacy == 0, "unexpected legacy rows: {skipped_legacy}");
    hours
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("usage: bench_real_dynamics <collector_stream.bin> <out dir>");
    let out_dir = args.next().expect("usage: bench_real_dynamics <collector_stream.bin> <out dir>");
    std::fs::create_dir_all(&out_dir).expect("out dir");
    let stream = std::fs::read(&input).expect("read stream");
    let _ = STATUS_FAILURE_MASK_APPROX;

    let hours = scan_d20_hours(&stream);
    assert!(!hours.is_empty(), "stream carries no d20 DATA records");

    // ---- Pass 2: fold to per-hour states; encode both arms. ----------------
    let schema = d20::schema();
    let collection = &schema.collections[0];

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut world: BTreeMap<String, ShipRow> = BTreeMap::new();
    let mut prior: Option<KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut divisions_total = 0usize;
    let mut non_contiguous = 0usize;
    let mut last_hour: Option<i32> = None;

    for (game_hour, delta, rows, ship_span) in &hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[ship_span.clone()]);
        ship_offsets.push(ship.len());

        if let Some(prev) = last_hour {
            if *game_hour != prev + 1 {
                non_contiguous += 1;
            }
        }
        last_hour = Some(*game_hour);

        // Fold exactly as world_fold does: baseline replaces, delta upserts.
        if !*delta {
            world.clear();
        }
        for row in rows {
            let uid = format!("{}:{}", u32_at(&row.bytes, 0), u32_at(&row.bytes, 4));
            if row.flags() & F_DELETED != 0 {
                world.remove(&uid);
            } else {
                world.insert(uid, row.clone());
            }
        }

        // The hour's full state in the new grammar.
        let mut state: KeyedState = BTreeMap::new();
        for row in world.values() {
            state.insert(
                key(&[u32_at(&row.bytes, 0) as u64, u32_at(&row.bytes, 4) as u64]),
                row.to_record(collection),
            );
        }
        divisions_total += state.len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let block_start = hours_seen % BLOCK_HOURS == 0;
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(&schema, std::slice::from_ref(&state), &mut wire)
                .expect("baseline encodes");
            baselines += 1;
        } else {
            let prev = prior.as_ref().unwrap();
            let d = diff(collection, prev, &state);
            if d.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                changed_rows_total += d.changed.len();
                tombstones_total += d.tombstones.len();
                encode_delta(
                    &schema,
                    std::slice::from_ref(prev),
                    &[Some(d)],
                    &mut wire,
                )
                .expect("delta encodes");
            }
        }
        wire_offsets.push(wire.len());

        // Reader replay: decode the bytes just written against the reader's
        // OWN reconstructed prior, exactly as a conforming reader would, and
        // require the result to be stored-equal to this hour's state. This is
        // what makes the size claim a claim about decodable bytes.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if block_start || reader.is_none() {
                let (states, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader = Some(states.into_iter().next().unwrap());
            } else {
                let reader_prior = reader.take().unwrap();
                let (deltas, consumed) = decode_delta(
                    &schema,
                    std::slice::from_ref(&reader_prior),
                    hour_bytes,
                    0,
                )
                .expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                let mut next = reader_prior;
                apply(&mut next, deltas[0].as_ref().unwrap());
                reader = Some(next);
            }
        }
        let replayed = reader.as_ref().unwrap();
        assert_eq!(replayed.len(), state.len(), "hour {hours_seen}: row count");
        let residual = diff(collection, replayed, &state);
        assert!(
            residual.is_empty(),
            "hour {hours_seen}: reader state is not stored-equal to the fold"
        );

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d20.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d20.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    println!("hours                {hours_seen} ({non_contiguous} non-contiguous steps)");
    println!("divisions avg        {:.1}", divisions_total as f64 / hours_seen as f64);
    println!("baselines            {baselines} (every {BLOCK_HOURS} h)");
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d20.bin         {} bytes", ship.len());
    println!("wire_new.bin         {} bytes ({:.1}%)", wire.len(), 100.0 * wire.len() as f64 / ship.len() as f64);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One H4CR record, framed the way the recorder frames it — only the fields
    /// this walk reads are meaningful, the rest are the recorder's own shape.
    fn record(sequence: u64, game_hour: i32, domain: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(H4CR_HEADER_LEN + payload.len());
        out.extend_from_slice(H4CR_MAGIC);
        out.extend_from_slice(&1u16.to_le_bytes()); // version
        out.extend_from_slice(&(H4CR_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // record flags
        assert_eq!(out.len(), H4CR_HEADER_LEN);
        out.extend_from_slice(payload);
        out
    }

    /// A 120-byte extended division row: one live division with an owner.
    fn division_row(div_id: u32, owner: u32) -> Vec<u8> {
        let mut row = vec![0u8; ROW_LEN];
        row[0..4].copy_from_slice(&7u32.to_le_bytes()); // country slot
        row[4..8].copy_from_slice(&div_id.to_le_bytes());
        row[8..12].copy_from_slice(&F_HAS_OWNER.to_le_bytes());
        row[56..60].copy_from_slice(&owner.to_le_bytes());
        row
    }

    /// A d20 division-table payload: the 32-byte header then the rows.
    fn d20_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut p = vec![0u8; TABLE_HEADER_LEN];
        p[8..12].copy_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        p[16..20].copy_from_slice(&status_flags.to_le_bytes());
        for row in rows {
            p.extend_from_slice(row);
        }
        p
    }

    fn one_record_stream(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        record(1, 100, D20, RECORD_TYPE_DOMAIN_DATA, &d20_payload(status_flags, rows))
    }

    #[test]
    fn a_well_formed_stream_yields_its_d20_hours() {
        let hours = scan_d20_hours(&one_record_stream(0, &[division_row(1, 42)]));
        assert_eq!(hours.len(), 1);
        assert_eq!(hours[0].0, 100);
        assert!(!hours[0].1, "a clear delta bit is a full baseline");
        assert_eq!(hours[0].2.len(), 1);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the v1 status helpers classify
        // a known vocabulary and are not validity predicates, so an unassigned bit
        // must be rejected BEFORE classification. Here classification is the
        // baseline-vs-delta decision: with the bit unrecognised, `status &
        // STATUS_DELTA_DATA` is zero and the record reads as a full baseline that
        // CLEARS the world map — a measured hour built from a record with no
        // defined meaning.
        //
        // This walk reports by panicking, as everything else in it does, so the
        // refusals are caught rather than matched on a `Result`. The default hook
        // would print 108 backtraces; it is restored before the accepting half.
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        // Every unassigned bit, alone and combined with a known failure bit and
        // with the informational delta bit, with rows and without.
        for bit in 14..32u32 {
            let status = 1u32 << bit;
            for extra in [0u32, 1, STATUS_DELTA_DATA] {
                for rows in [Vec::new(), vec![division_row(1, 42)]] {
                    let stream = one_record_stream(status | extra, &rows);
                    let refused = std::panic::catch_unwind(move || scan_d20_hours(&stream)).is_err();
                    assert!(
                        refused,
                        "status bit {bit} (+{extra:#x}) with {} row(s) must be refused",
                        rows.len()
                    );
                }
            }
        }
        std::panic::set_hook(previous_hook);

        // And a clean status word and every INFORMATIONAL v1 bit are still
        // accepted, with the same baseline/delta classification as before: the
        // check rejects unassigned bits and nothing else. d20's own informational
        // bit is DELTA_DATA; bits 4, 5, 11 and 13 are inert here, exactly as they
        // were.
        //
        // The failure bits are deliberately not asserted accepted. This walk has
        // no failure exclusion at all — see `STATUS_FAILURE_MASK_APPROX` above,
        // which is zero and unused — so what it does with a failure-status record
        // is a separate question from this gate, and asserting the current
        // behaviour here would freeze a rule `fold_divisions` does not have.
        for known in [0u32, 1 << 4, 1 << 5, STATUS_DELTA_DATA, 1 << 11, 1 << 13] {
            let hours = scan_d20_hours(&one_record_stream(known, &[division_row(1, 42)]));
            assert_eq!(hours.len(), 1, "status {known:#x}");
            assert_eq!(
                hours[0].1,
                known & STATUS_DELTA_DATA != 0,
                "status {known:#x} keeps its baseline/delta classification"
            );
        }
    }
}
