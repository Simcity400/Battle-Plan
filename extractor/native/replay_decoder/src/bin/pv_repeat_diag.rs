//! Lane PV diagnostic: framing-level equivalence check between an original
//! H4CR stream and its gated re-encoding. Pure byte arithmetic — no fold.
//!
//! Streams both spools (zstd containers decode through compact_stream) in
//! lockstep over their DOMAIN_DATA / DOMAIN_DATA_REPEAT records, which must
//! appear 1:1 in order, and checks for every REPEAT in the gated stream:
//!   1. its recorded digest equals SHA-256 of the simulated single-slot
//!      baseline's tail (the slot rule the gate and the fold both run);
//!   2. fresh prefix ++ baseline tail reproduces exactly the corresponding
//!      ORIGINAL payload (reconstruction equivalence).
//! Reports the first failures with domain, hour, and data-record ordinal.
//! Memory stays at one 8 MiB chunk per stream plus per-domain slots.

use std::collections::BTreeMap;
use std::io::Read;

use sha2::{Digest, Sha256};

const CHUNK: usize = 8 * 1024 * 1024;

struct StreamRecords {
    reader: Box<dyn Read>,
    carry: Vec<u8>,
    consumed_bytes: u64,
}

impl StreamRecords {
    fn open(path: &str) -> Self {
        let reader: Box<dyn Read> = Box::new(
            compact_stream::open(path).unwrap_or_else(|error| panic!("open {path}: {error}")),
        );
        Self {
            reader,
            carry: Vec::with_capacity(CHUNK),
            consumed_bytes: 0,
        }
    }

    /// Next whole (header, payload) record, refilling from the reader as
    /// needed. None at clean EOF; panics on framing damage.
    fn next(&mut self) -> Option<(compact_contract::CompactRecordHeader, Vec<u8>)> {
        let header_len = usize::from(compact_contract::COMPACT_RECORD_HEADER_LEN);
        loop {
            if self.carry.len() >= header_len {
                if let Ok(header) = compact_contract::parse_compact_record_header(&self.carry) {
                    let total = usize::try_from(header.record_len()).expect("record length");
                    if self.carry.len() >= total {
                        let mut record = self.carry[..total].to_vec();
                        self.carry.drain(..total);
                        self.consumed_bytes += total as u64;
                        return Some((
                            header,
                            record.split_off(header_len),
                        ));
                    }
                } else {
                    panic!("framing damage at decoded offset {}", self.consumed_bytes);
                }
            }
            let start = self.carry.len();
            self.carry.resize(start + CHUNK, 0);
            let read = self
                .reader
                .read(&mut self.carry[start..])
                .unwrap_or_else(|error| panic!("decode: {error}"));
            self.carry.truncate(start + read);
            if read == 0 {
                if self.carry.is_empty() {
                    return None;
                }
                panic!("stream ends mid-record at decoded offset {}", self.consumed_bytes);
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: pv_repeat_diag <original.spool> <gated.spool>");
        std::process::exit(2);
    }
    let mut original = StreamRecords::open(&args[1]);
    let mut gated = StreamRecords::open(&args[2]);

    // Simulated single-slot baselines, updated by ORIGINAL payloads exactly
    // like the fold's framing store.
    let mut slots: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
    let mut index = 0u64;
    let mut failures = 0u64;
    loop {
        let (o_header, o_payload) = match original.next() {
            Some((header, payload))
                if header.record_type
                    == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA =>
            {
                (header, payload)
            }
            Some(_) => continue,
            None => match gated.next() {
                Some((header, _))
                    if header.record_type
                        == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA =>
                {
                    println!(
                        "FAIL at #{index}: gated stream has extra data record domain {} hour {}",
                        header.domain_id, header.game_hour_raw
                    );
                    std::process::exit(1);
                }
                Some(_) => continue,
                None => break,
            },
        };
        // Gated side: skip forward to its next data record.
        let (g_header, g_payload) = loop {
            match gated.next() {
                Some((header, payload))
                    if header.record_type
                        == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA
                        || header.record_type
                            == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT =>
                {
                    break (header, payload)
                }
                Some(_) => continue,
                None => {
                    println!("FAIL at #{index}: gated stream ends before original");
                    std::process::exit(1);
                }
            }
        };
        if o_header.domain_id != g_header.domain_id
            || o_header.game_hour_raw != g_header.game_hour_raw
        {
            println!(
                "FAIL at #{index}: alignment broke (domain {}/{}, hour {}/{})",
                o_header.domain_id,
                g_header.domain_id,
                o_header.game_hour_raw,
                g_header.game_hour_raw
            );
            std::process::exit(1);
        }
        let domain = o_header.domain_id;
        let hour = o_header.game_hour_raw;
        let span = compact_contract::domain_declared_volatile_prefix_len(domain);
        let is_repeat =
            g_header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT;
        if is_repeat {
            match slots.get(&domain) {
                None => {
                    println!(
                        "FAIL at #{index}: REPEAT domain {domain} hour {hour} has no simulated baseline"
                    );
                    failures += 1;
                }
                Some(previous) => {
                    let recorded = &g_payload[span..];
                    let expected = Sha256::digest(&previous[span..]);
                    if expected.as_slice() != recorded {
                        println!(
                            "FAIL at #{index}: REPEAT domain {domain} hour {hour} digest mismatch against simulated slot"
                        );
                        failures += 1;
                    }
                    let mut reconstructed = g_payload[..span].to_vec();
                    reconstructed.extend_from_slice(&previous[span..]);
                    if reconstructed != o_payload {
                        println!(
                            "FAIL at #{index}: REPEAT domain {domain} hour {hour} reconstructs {} bytes vs original {} bytes",
                            reconstructed.len(),
                            o_payload.len()
                        );
                        failures += 1;
                    }
                }
            }
        } else if g_payload != o_payload {
            println!(
                "FAIL at #{index}: FULL record domain {domain} hour {hour} differs between streams"
            );
            failures += 1;
        }
        slots.insert(domain, o_payload);
        index += 1;
        if failures >= 10 {
            println!("stopping after 10 failures at data record #{index}");
            break;
        }
    }
    if failures == 0 {
        println!(
            "EQUIVALENT: all {index} data records align; every repeat reconstructs its original payload exactly"
        );
    } else {
        std::process::exit(1);
    }
}
