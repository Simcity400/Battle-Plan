//! Lane CE measurement instrument: decompose an already-built replay
//! checkpoint cache and price candidate encodings in CONTAINER bytes.
//!
//! Read-only with respect to its inputs. It writes nothing anywhere: every
//! candidate encoding is streamed straight into the production zstd encoder
//! and only the resulting frame length is kept.
//!
//! Usage:
//!   ce_cache_anatomy <checkpoints.h4cp> <hour_index.h4ix> <mode>[,<mode>...]
//!
//! Modes:
//!   baseline     the cache exactly as built (reproduces the container entry)
//!   classes      per-class byte counts and per-class separate frames
//!   split        candidate: per-record class-split streams
//!   gkeys        candidate: global key dictionary
//!   gtokens      candidate: global key + string + number-token dictionaries
//!   gkeys_split  candidate: global key dictionary + class split
//!   gall_split   candidate: all three dictionaries + class split
//!   perrecord    diagnostic: every record in its own frame (no shared window)
//!   window31     diagnostic: baseline at a window the reader contract forbids

use snapshot::replay_cache::ce_anatomy::{
    compressed_len, compressed_len_oversized_window, decode_dictionary_payload, records,
    walk_record, Class, ClassCounts, CountSink, DictionarySink, FrequencySink, ParitySink,
    ArrayDeltaDecoder, ArrayDeltaSink, PathSink, TokenDictionary, CLASS_COUNT,
};
use snapshot::replay_cache::ReplayHourIndex;

/// mimalloc, because the process this instrument stands in for uses mimalloc.
///
/// Every candidate encoding here walks a whole-world `CanonValue` tree and
/// rebuilds millions of small `String`s per record, which is exactly the load
/// the Windows system heap serializes badly under; the production recorder
/// installs mimalloc in its own `main` for that reason. The reported BYTE
/// counts are allocator-independent, but the mode-to-mode wall times an owner
/// reads off a run are not, so measure on the production heap. Allocator
/// choice cannot change a produced byte.
#[global_allocator]
static GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        eprintln!("usage: ce_cache_anatomy <checkpoints.h4cp> <hour_index.h4ix> <modes>");
        std::process::exit(2);
    }
    let cache = std::fs::read(&args[0]).expect("read cache");
    let index_bytes = std::fs::read(&args[1]).expect("read hour index");
    let index = ReplayHourIndex::from_bytes(&index_bytes).expect("parse hour index");

    eprintln!(
        "cache_len={} hours={} interval={} checkpoints={}",
        cache.len(),
        index.hours().len(),
        index.checkpoint_interval_hours(),
        index
            .hours()
            .iter()
            .filter(|hour| hour.delta_steps() == 0)
            .count()
    );

    let mut frequencies: Option<FrequencySink> = None;
    for mode in args[2].split(',') {
        let started = std::time::Instant::now();
        match mode {
            "baseline" => {
                let bytes = compressed_len(27, Some(cache.len() as u64), |sink| sink.write_all(&cache)).expect("compress");
                report(mode, cache.len() as u64, bytes, started);
            }
            "window31" => {
                let bytes = compressed_len_oversized_window(31, Some(cache.len() as u64), |sink| sink.write_all(&cache))
                    .expect("compress");
                report(mode, cache.len() as u64, bytes, started);
            }
            "perrecord" => {
                let mut total = 0u64;
                for record in records(&cache, &index) {
                    let record = record.expect("record");
                    total += compressed_len(
                        27,
                        Some((record.header.len() + record.payload.len()) as u64),
                        |sink| {
                            sink.write_all(record.header)?;
                            sink.write_all(record.payload)
                        },
                    )
                    .expect("compress");
                }
                report(mode, cache.len() as u64, total, started);
            }
            "classes" => class_report(&cache, &index, started),
            "paths" => path_report(&cache, &index, started),
            "arraydelta" => array_delta_report(&cache, &index, started),
            "classmajor" => {
                class_major_report(&cache, &index, mode, [None; CLASS_COUNT], started)
            }
            "best" => {
                let frequencies = frequencies.get_or_insert_with(|| scan(&cache, &index));
                let keys = TokenDictionary::build(&frequencies.keys, 2);
                let strings = TokenDictionary::build(&frequencies.strings, 2);
                let numbers = TokenDictionary::build(&frequencies.numbers, 2);
                class_major_report(
                    &cache,
                    &index,
                    mode,
                    [None, Some(&keys), Some(&numbers), Some(&strings)],
                    started,
                )
            }
            "parity" => {
                // Prove the grammar walk is total on the real cache before any
                // number derived from it is quoted.
                let mut checked = 0u64;
                for record in records(&cache, &index) {
                    let record = record.expect("record");
                    let mut sink = ParitySink::default();
                    walk_record(record.kind, record.payload, &mut sink).expect("walk");
                    assert_eq!(
                        sink.bytes, record.payload,
                        "walk did not reproduce hour {}",
                        record.hour_index
                    );
                    checked += 1;
                }
                println!(
                    "{{\"mode\":\"parity\",\"records_reproduced\":{checked},\"elapsed_s\":{:.1}}}",
                    started.elapsed().as_secs_f64()
                );
            }
            "split" => candidate(&cache, &index, mode, None, true, started),
            "gkeys" => {
                let frequencies = frequencies.get_or_insert_with(|| scan(&cache, &index));
                candidate(&cache, &index, mode, Some((frequencies, false)), false, started)
            }
            "gtokens" => {
                let frequencies = frequencies.get_or_insert_with(|| scan(&cache, &index));
                candidate(&cache, &index, mode, Some((frequencies, true)), false, started)
            }
            "gkeys_split" => {
                let frequencies = frequencies.get_or_insert_with(|| scan(&cache, &index));
                candidate(&cache, &index, mode, Some((frequencies, false)), true, started)
            }
            "gall_split" => {
                let frequencies = frequencies.get_or_insert_with(|| scan(&cache, &index));
                candidate(&cache, &index, mode, Some((frequencies, true)), true, started)
            }
            other => {
                eprintln!("unknown mode {other}");
                std::process::exit(2);
            }
        }
    }
}

/// The 21-byte v2 record header, rebuilt for the length the CANDIDATE payload
/// actually has.
///
/// Copying the original record header would carry the v2 payload length into an
/// artifact whose payloads are a different size, so the emitted framing would
/// not describe its own bytes. That is not a size the candidate could ever be
/// read at.
fn write_record_header(out: &mut Vec<u8>, kind: u8, hour_index: i64, payload_len: usize) {
    out.push(kind);
    out.extend_from_slice(&[0; 3]);
    out.extend_from_slice(&hour_index.to_le_bytes());
    out.extend_from_slice(&(payload_len as u64).to_le_bytes());
    out.push(0);
}

/// Read a framed record back the way a reader would: kind, hour and the payload
/// length that says where the payload ends.
fn read_record_header(bytes: &[u8]) -> (u8, i64, usize) {
    assert!(bytes.len() >= 21, "framed record is shorter than a header");
    assert_eq!(&bytes[1..4], &[0, 0, 0], "record header reserved bytes");
    assert_eq!(bytes[20], 0, "record header trailing reserved byte");
    let hour = i64::from_le_bytes(bytes[4..12].try_into().unwrap());
    let payload_len = u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
    (bytes[0], hour, payload_len)
}

/// One record under a dictionary/split candidate, framed exactly as the
/// candidate artifact carries it. The verification pass and the compression
/// pass both go through this, so they cannot describe different bytes.
fn frame_candidate_record(
    record: &snapshot::replay_cache::ce_anatomy::CacheRecord<'_>,
    dictionaries: [Option<&TokenDictionary>; CLASS_COUNT],
    split: bool,
) -> Vec<u8> {
    let mut writer = DictionarySink::new(dictionaries, split);
    walk_record(record.kind, record.payload, &mut writer).expect("walk");
    let payload = writer.payload();
    let mut framed = Vec::with_capacity(21 + payload.len());
    write_record_header(&mut framed, record.kind, record.hour_index, payload.len());
    framed.extend_from_slice(&payload);
    framed
}

/// Every dictionary the candidate emits, parsed back out of its own serialized
/// table bytes, so the decode side never borrows the encode side's state.
fn rebuild_tables(
    dictionaries: [Option<&TokenDictionary>; CLASS_COUNT],
) -> [Option<TokenDictionary>; CLASS_COUNT] {
    let mut out: [Option<TokenDictionary>; CLASS_COUNT] = Default::default();
    for (slot, source) in out.iter_mut().zip(dictionaries) {
        *slot = source.map(|dictionary| {
            let rebuilt = TokenDictionary::decode(&dictionary.encode())
                .expect("dictionary table must parse from its own bytes");
            assert_eq!(
                rebuilt.order, dictionary.order,
                "dictionary table did not survive its own encoding"
            );
            rebuilt
        });
    }
    out
}

/// Minimal base-128 varint, matching the cache format's own spelling.
fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// The varint at the head of `bytes`, with the width it occupied.
fn read_varint(bytes: &[u8]) -> (u64, usize) {
    let mut value = 0u64;
    let mut shift = 0u32;
    for (width, byte) in bytes.iter().enumerate() {
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return (value, width + 1);
        }
        shift += 7;
    }
    panic!("directory varint is truncated");
}

fn report(mode: &str, raw: u64, compressed: u64, started: std::time::Instant) {
    println!(
        "{{\"mode\":{mode:?},\"raw_bytes\":{raw},\"compressed_bytes\":{compressed},\
         \"ratio\":{:.2},\"elapsed_s\":{:.1}}}",
        raw as f64 / compressed.max(1) as f64,
        started.elapsed().as_secs_f64()
    );
}

fn scan(cache: &[u8], index: &ReplayHourIndex) -> FrequencySink {
    let mut sink = FrequencySink::default();
    for record in records(cache, index) {
        let record = record.expect("record");
        walk_record(record.kind, record.payload, &mut sink).expect("walk");
    }
    eprintln!(
        "token census: distinct keys={} strings={} number_tokens={}",
        sink.keys.len(),
        sink.strings.len(),
        sink.numbers.len()
    );
    sink
}

fn class_report(cache: &[u8], index: &ReplayHourIndex, started: std::time::Instant) {
    let mut checkpoint = ClassCounts::default();
    let mut delta = ClassCounts::default();
    let mut checkpoint_payload = 0u64;
    let mut delta_payload = 0u64;
    for record in records(cache, index) {
        let record = record.expect("record");
        let mut sink = CountSink::default();
        let counts = walk_record(record.kind, record.payload, &mut sink).expect("walk");
        let mut totals = sink.counts;
        totals.inline_uint_bytes = counts.inline_uint_bytes;
        totals.replace_array_bytes = counts.replace_array_bytes;
        totals.replace_array_count = counts.replace_array_count;
        totals.replace_array_elements = counts.replace_array_elements;
        totals.replace_object_bytes = counts.replace_object_bytes;
        totals.replace_object_count = counts.replace_object_count;
        totals.replace_scalar_bytes = counts.replace_scalar_bytes;
        totals.replace_scalar_count = counts.replace_scalar_count;
        if record.kind == 1 {
            checkpoint.add(&totals);
            checkpoint_payload += record.payload.len() as u64;
        } else {
            delta.add(&totals);
            delta_payload += record.payload.len() as u64;
        }
    }
    // Each class as its OWN frame over the whole cache. The four frames do not
    // sum to the baseline frame — separating them removes cross-class matches
    // and adds three frame headers — so this is an attribution of where the
    // remaining bytes live, not a decomposition of the baseline.
    let mut class_frames = [0u64; CLASS_COUNT];
    for class in [Class::Struct, Class::Key, Class::Number, Class::String] {
        let raw = class_byte_total(cache, index, class);
        class_frames[class.index()] = compressed_len(27, Some(raw), |sink| {
            for record in records(cache, index) {
                let record = record.expect("record");
                let mut split = snapshot::replay_cache::ce_anatomy::SplitSink::default();
                walk_record(record.kind, record.payload, &mut split).expect("walk");
                sink.write_all(&split.streams[class.index()])?;
            }
            Ok(())
        })
        .expect("compress");
    }
    let checkpoint_frame = compressed_len(27, Some(checkpoint_payload), |sink| {
        for record in records(cache, index) {
            let record = record.expect("record");
            if record.kind == 1 {
                sink.write_all(record.payload)?;
            }
        }
        Ok(())
    })
    .expect("compress");
    let delta_frame = compressed_len(27, Some(delta_payload), |sink| {
        for record in records(cache, index) {
            let record = record.expect("record");
            if record.kind == 2 {
                sink.write_all(record.payload)?;
            }
        }
        Ok(())
    })
    .expect("compress");
    println!(
        "{{\"mode\":\"classes\",\
         \"checkpoint_payload_bytes\":{checkpoint_payload},\"delta_payload_bytes\":{delta_payload},\
         \"checkpoint_frame_bytes\":{checkpoint_frame},\"delta_frame_bytes\":{delta_frame},\
         \"checkpoint_class_bytes\":{{\"struct\":{},\"key\":{},\"number\":{},\"string\":{}}},\
         \"delta_class_bytes\":{{\"struct\":{},\"key\":{},\"number\":{},\"string\":{}}},\
         \"delta_inline_uint_bytes\":{},\"checkpoint_inline_uint_bytes\":{},\
         \"delta_key_occurrences\":{},\"delta_string_occurrences\":{},\
         \"delta_number_token_occurrences\":{},\
         \"delta_replace_array_bytes\":{},\"delta_replace_array_count\":{},\
         \"delta_replace_array_elements\":{},\
         \"delta_replace_object_bytes\":{},\"delta_replace_object_count\":{},\
         \"delta_replace_scalar_bytes\":{},\"delta_replace_scalar_count\":{},\
         \"class_frame_bytes\":{{\"struct\":{},\"key\":{},\"number\":{},\"string\":{}}},\
         \"elapsed_s\":{:.1}}}",
        checkpoint.bytes[0],
        checkpoint.bytes[1],
        checkpoint.bytes[2],
        checkpoint.bytes[3],
        delta.bytes[0],
        delta.bytes[1],
        delta.bytes[2],
        delta.bytes[3],
        delta.inline_uint_bytes,
        checkpoint.inline_uint_bytes,
        delta.key_occurrences,
        delta.string_occurrences,
        delta.number_token_occurrences,
        delta.replace_array_bytes,
        delta.replace_array_count,
        delta.replace_array_elements,
        delta.replace_object_bytes,
        delta.replace_object_count,
        delta.replace_scalar_bytes,
        delta.replace_scalar_count,
        class_frames[0],
        class_frames[1],
        class_frames[2],
        class_frames[3],
        started.elapsed().as_secs_f64()
    );
}

/// Which member paths the whole-array replacements sit under.
///
/// `diff_value` restates an array whole whenever its length changed, so this
/// names the parts of the world whose arrays resize, which is where a
/// length-aware array diff would have to work.
fn path_report(cache: &[u8], index: &ReplayHourIndex, started: std::time::Instant) {
    let mut sink = PathSink::new(4);
    let mut delta_payload = 0u64;
    for record in records(cache, index) {
        let record = record.expect("record");
        if record.kind != 2 {
            continue;
        }
        delta_payload += record.payload.len() as u64;
        walk_record(record.kind, record.payload, &mut sink).expect("walk");
    }
    let mut rows: Vec<(String, (u64, u64, u64))> = sink
        .totals
        .iter()
        .map(|(path, totals)| (String::from_utf8_lossy(path).into_owned(), *totals))
        .collect();
    rows.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(&b.0)));
    println!("{{\"mode\":\"paths\",\"delta_payload_bytes\":{delta_payload},\"rows\":[");
    for (position, (path, (bytes, count, elements))) in rows.iter().take(40).enumerate() {
        println!(
            "  {}{{\"path\":{path:?},\"replace_array_bytes\":{bytes},\"replacements\":{count},\"elements\":{elements}}}",
            if position == 0 { "" } else { "," }
        );
    }
    println!(
        "],\"distinct_paths\":{},\"elapsed_s\":{:.1}}}",
        rows.len(),
        started.elapsed().as_secs_f64()
    );
}

/// Price the class-major layout: every record still contributes the same four
/// class streams, but the streams of ALL records are concatenated class by
/// class instead of record by record.
///
/// This is seekable. The cache is authenticated and held whole
/// (`CertifiedReplayCache` owns the decompressed bytes), and the per-record
/// class lengths are carried in a directory at the head, so reaching a record
/// is four slices instead of one. What it changes is only which bytes sit next
/// to which.
fn class_major_report(
    cache: &[u8],
    index: &ReplayHourIndex,
    mode: &str,
    dictionaries: [Option<&TokenDictionary>; CLASS_COUNT],
    started: std::time::Instant,
) {
    let header_len = index.hours()[0].cache_record_offset() as usize;
    let rebuilt = rebuild_tables(dictionaries);
    let decoding: [Option<&TokenDictionary>; CLASS_COUNT] = [
        rebuilt[0].as_ref(),
        rebuilt[1].as_ref(),
        rebuilt[2].as_ref(),
        rebuilt[3].as_ref(),
    ];
    let mut streams: [Vec<u8>; CLASS_COUNT] = Default::default();
    let mut directory: Vec<u8> = Vec::new();
    for record in records(cache, index) {
        let record = record.expect("record");
        let mut writer = DictionarySink::new(dictionaries, true);
        walk_record(record.kind, record.payload, &mut writer).expect("walk");
        let payload_len: usize = writer.streams.iter().map(Vec::len).sum();
        write_record_header(&mut directory, record.kind, record.hour_index, payload_len);
        for class in 0..CLASS_COUNT {
            put_varint(&mut directory, writer.streams[class].len() as u64);
            streams[class].extend_from_slice(&writer.streams[class]);
        }
    }
    // Rebuild every record from the EMITTED directory and the class-major
    // streams, then decode it. Reading the lengths back out of the artifact
    // rather than out of the loop above is what makes the directory's own bytes
    // part of what is proven sufficient.
    let mut cursors = [0usize; CLASS_COUNT];
    let mut at = 0usize;
    let mut verified = 0u64;
    for record in records(cache, index) {
        let record = record.expect("record");
        let (kind, hour, payload_len) = read_record_header(&directory[at..]);
        assert_eq!(
            (kind, hour),
            (record.kind, record.hour_index),
            "directory lost a record identity"
        );
        at += 21;
        let mut row = [0usize; CLASS_COUNT];
        for slot in &mut row {
            let (value, width) = read_varint(&directory[at..]);
            *slot = value as usize;
            at += width;
        }
        assert_eq!(
            row.iter().sum::<usize>(),
            payload_len,
            "directory class lengths disagree with the record payload length"
        );
        let mut payload = Vec::new();
        for length in row {
            put_varint(&mut payload, length as u64);
        }
        for class in 0..CLASS_COUNT {
            payload.extend_from_slice(&streams[class][cursors[class]..cursors[class] + row[class]]);
            cursors[class] += row[class];
        }
        let decoded = decode_dictionary_payload(kind, &payload, decoding, true)
            .expect("decode class-major payload");
        assert_eq!(
            decoded, record.payload,
            "class-major layout lost hour {}",
            record.hour_index
        );
        verified += 1;
    }
    assert_eq!(at, directory.len(), "directory has unread trailing bytes");
    for (class, cursor) in cursors.iter().enumerate() {
        assert_eq!(
            *cursor,
            streams[class].len(),
            "class-major stream {class} has unread trailing bytes"
        );
    }
    let table_bytes: u64 = dictionaries
        .iter()
        .filter_map(|slot| slot.map(|dictionary| dictionary.encode().len() as u64))
        .sum();
    let raw = header_len as u64
        + table_bytes
        + directory.len() as u64
        + streams.iter().map(|s| s.len() as u64).sum::<u64>();
    let compressed = compressed_len(27, Some(raw), |sink| {
        sink.write_all(&cache[..header_len])?;
        for slot in dictionaries.iter().flatten() {
            sink.write_all(&slot.encode())?;
        }
        sink.write_all(&directory)?;
        for stream in &streams {
            sink.write_all(stream)?;
        }
        Ok(())
    })
    .expect("compress");
    println!(
        "{{\"mode\":{mode:?},\"raw_bytes\":{raw},\"compressed_bytes\":{compressed},\
         \"dictionary_table_bytes\":{table_bytes},\"dictionary_entries\":{{\"key\":{},\"number\":{},\"string\":{}}},\
         \"directory_bytes\":{},\"stream_bytes\":{{\"struct\":{},\"key\":{},\"number\":{},\"string\":{}}},\
         \"records_round_tripped\":{verified},\"elapsed_s\":{:.1}}}",
        dictionaries[Class::Key.index()].map_or(0, TokenDictionary::len),
        dictionaries[Class::Number.index()].map_or(0, TokenDictionary::len),
        dictionaries[Class::String.index()].map_or(0, TokenDictionary::len),
        directory.len(),
        streams[0].len(),
        streams[1].len(),
        streams[2].len(),
        streams[3].len(),
        started.elapsed().as_secs_f64()
    );
}

/// Price the array-diff repair: every whole-array restatement is stored as a
/// byte delta against the array last restated at the same world location.
///
/// The decode pass is not optional. It runs the inverse transform over the
/// rewritten records and asserts each one returns the original payload, so the
/// compressed number below is a size for the same information rather than a
/// size for less of it.
fn array_delta_report(cache: &[u8], index: &ReplayHourIndex, started: std::time::Instant) {
    let header_len = index.hours()[0].cache_record_offset() as usize;
    let mut writer = ArrayDeltaSink::new();
    let mut reader = ArrayDeltaDecoder::new();
    let mut raw = header_len as u64;
    let mut verified = 0u64;
    // Every record is FRAMED first and read back out of that framing, so the
    // 21 bytes the candidate pays for are bytes a reader could actually use.
    let mut framed_records: Vec<Vec<u8>> = Vec::with_capacity(index.hours().len());
    for record in records(cache, index) {
        let record = record.expect("record");
        writer.begin_record();
        walk_record(record.kind, record.payload, &mut writer).expect("walk");
        let mut framed = Vec::with_capacity(21 + writer.out.len());
        write_record_header(&mut framed, record.kind, record.hour_index, writer.out.len());
        framed.extend_from_slice(&writer.out);
        let (kind, hour, payload_len) = read_record_header(&framed);
        assert_eq!(
            (kind, hour),
            (record.kind, record.hour_index),
            "framed record identity"
        );
        assert_eq!(21 + payload_len, framed.len(), "framed record has slack");
        let decoded = reader
            .decode_record(kind, &framed[21..21 + payload_len])
            .expect("decode array-delta payload");
        assert_eq!(
            decoded, record.payload,
            "array delta did not round-trip hour {}",
            record.hour_index
        );
        verified += 1;
        raw += framed.len() as u64;
        framed_records.push(framed);
    }
    let compressed = compressed_len(27, Some(raw), |sink| {
        sink.write_all(&cache[..header_len])?;
        for framed in &framed_records {
            sink.write_all(framed)?;
        }
        Ok(())
    })
    .expect("compress");
    println!(
        "{{\"mode\":\"arraydelta\",\"raw_bytes\":{raw},\"compressed_bytes\":{compressed},\
         \"array_replacements_delta_coded\":{},\"array_replacements_first_sighting\":{},\
         \"raw_array_bytes\":{},\"delta_array_bytes\":{},\"shared_edge_bytes\":{},\
         \"distinct_locations\":{},\"records_round_tripped\":{verified},\"elapsed_s\":{:.1}}}",
        writer.rewritten,
        writer.first_sighting,
        writer.raw_array_bytes,
        writer.delta_array_bytes,
        writer.shared_bytes,
        writer.last.len(),
        started.elapsed().as_secs_f64()
    );
}

/// Rewrite every record under a candidate encoding, prove the rewrite decodes
/// back to the original payload, and report the container frame length.
fn candidate(
    cache: &[u8],
    index: &ReplayHourIndex,
    mode: &str,
    dictionary_source: Option<(&FrequencySink, bool)>,
    split: bool,
    started: std::time::Instant,
) {
    let built;
    let dictionaries: [Option<&TokenDictionary>; CLASS_COUNT] = match dictionary_source {
        None => [None, None, None, None],
        Some((frequencies, all_tokens)) => {
            built = (
                TokenDictionary::build(&frequencies.keys, 2),
                TokenDictionary::build(&frequencies.strings, 2),
                TokenDictionary::build(&frequencies.numbers, 2),
            );
            if all_tokens {
                [None, Some(&built.0), Some(&built.2), Some(&built.1)]
            } else {
                [None, Some(&built.0), None, None]
            }
        }
    };
    let table_bytes: u64 = dictionaries
        .iter()
        .filter_map(|slot| slot.map(|dictionary| dictionary.encode().len() as u64))
        .sum();
    // The candidate pays for its tables, so the verification must decode with
    // tables rebuilt FROM THOSE BYTES. Decoding with the in-memory tables would
    // pass even if the emitted ones were short or wrong.
    let rebuilt = rebuild_tables(dictionaries);
    let decoding: [Option<&TokenDictionary>; CLASS_COUNT] = [
        rebuilt[0].as_ref(),
        rebuilt[1].as_ref(),
        rebuilt[2].as_ref(),
        rebuilt[3].as_ref(),
    ];
    // The 40-byte cache header is carried verbatim so a candidate's frame is
    // over the same artifact the baseline frame is over.
    let header_len = index.hours()[0].cache_record_offset() as usize;
    // Pass 1 sizes the candidate artifact and proves it is lossless. The frame
    // can then pledge its length, exactly as the container does for a real
    // entry, so this number and the baseline differ only in the encoding.
    let mut raw = table_bytes + header_len as u64;
    let mut verified = 0u64;
    for record in records(cache, index) {
        let record = record.expect("record");
        let framed = frame_candidate_record(&record, dictionaries, split);
        // Read the record back the way a reader would: the header says where
        // the payload ends, and nothing outside the framing is consulted.
        let (kind, hour, payload_len) = read_record_header(&framed);
        assert_eq!(
            (kind, hour),
            (record.kind, record.hour_index),
            "framed record identity"
        );
        assert_eq!(21 + payload_len, framed.len(), "framed record has slack");
        let decoded =
            decode_dictionary_payload(kind, &framed[21..21 + payload_len], decoding, split)
                .expect("decode candidate payload");
        assert_eq!(
            decoded, record.payload,
            "candidate {mode} did not round-trip hour {}",
            record.hour_index
        );
        verified += 1;
        raw += framed.len() as u64;
    }
    let compressed = compressed_len(27, Some(raw), |sink| {
        sink.write_all(&cache[..header_len])?;
        for slot in dictionaries.iter().flatten() {
            sink.write_all(&slot.encode())?;
        }
        for record in records(cache, index) {
            let record = record.expect("record");
            sink.write_all(&frame_candidate_record(&record, dictionaries, split))?;
        }
        Ok(())
    })
    .expect("compress");
    println!(
        "{{\"mode\":{mode:?},\"raw_bytes\":{raw},\"compressed_bytes\":{compressed},\
         \"dictionary_table_bytes\":{table_bytes},\
         \"dictionary_entries\":{{\"key\":{},\"number\":{},\"string\":{}}},\
         \"records_round_tripped\":{verified},\"elapsed_s\":{:.1}}}",
        dictionaries[Class::Key.index()].map_or(0, TokenDictionary::len),
        dictionaries[Class::Number.index()].map_or(0, TokenDictionary::len),
        dictionaries[Class::String.index()].map_or(0, TokenDictionary::len),
        started.elapsed().as_secs_f64()
    );
}

/// Total bytes of one class across the whole cache, so the class frame can
/// pledge its size exactly as a container entry would.
fn class_byte_total(cache: &[u8], index: &ReplayHourIndex, class: Class) -> u64 {
    let mut total = 0u64;
    for record in records(cache, index) {
        let record = record.expect("record");
        let mut sink = CountSink::default();
        walk_record(record.kind, record.payload, &mut sink).expect("walk");
        total += sink.counts.bytes[class.index()];
    }
    total
}
