use std::io::{Read, Write};

use super::*;

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(tag: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "hoi4_spool_trim_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Decoded bytes that look like a stream but barely compress.
///
/// The storage prefix a trim releases has to be large enough for NTFS to have
/// whole 64 KiB sparse allocation units to give back; a degenerately
/// compressible fixture would store a few kilobytes and make the reclamation
/// assertion vacuous.
fn decoded_fixture(len: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(len + 256);
    let mut i = 0u32;
    let mut state = 0x9e37_79b9u32;
    while bytes.len() < len {
        bytes.extend_from_slice(b"H4CR");
        bytes.extend_from_slice(&i.to_le_bytes());
        for _ in 0..120 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            bytes.push(state as u8);
        }
        i = i.wrapping_add(1);
    }
    bytes.truncate(len);
    bytes
}

struct Fixture {
    _dir: TmpDir,
    /// A retained read handle on every artifact the fixture just wrote.
    ///
    /// `trim_spool_prefix` and the receipt verifier reopen the stream, the
    /// hour index and the checkpoint cache immediately after this returns, and
    /// on Windows that is exactly when a real-time scanner still holds them —
    /// `os error 32` at the reader. Holding them on the FIXTURE means every
    /// test in this file is covered by construction, including ones added
    /// later, rather than by a list that goes stale.
    _scanner_guards: Vec<std::fs::File>,
    stream_path: PathBuf,
    decoded: Vec<u8>,
    frames: SpoolFrames,
    receipt: DurableHourReceipt,
    /// What the artifacts on disk actually prove, for a test that needs to
    /// reason about the watermark without asserting it into existence.
    durable_decoded_end: u64,
}

/// A spool plus a REAL certified artifact pair built by the same
/// `snapshot::ReplayCacheEncoder` production uses.
///
/// The encoder refuses an index that does not cover its source through the
/// final record, so a certified cache always reaches the end of the decoded
/// stream. That is why no fixture can hand the trim a short watermark: the
/// watermark is read out of the parsed index, and a real index cannot claim
/// less than the whole stream. `SpoolFrames::releasable_boundary` is tested
/// against partial watermarks directly, below.
fn fixture(tag: &str, decoded_len: usize, frame_bytes: u64) -> Fixture {
    let dir = TmpDir::new(tag);
    let stream_path = dir.0.join("collector_stream.bin");
    let decoded = decoded_fixture(decoded_len);

    let mut writer = compact_stream::StreamWriter::create_with_frames(
        &stream_path,
        compact_stream::StreamCompression::Zstd,
        Some(frame_bytes),
    )
    .unwrap();
    for chunk in decoded.chunks(65_536) {
        writer.write_all(chunk).unwrap();
    }
    let write_receipt = writer.finish().unwrap();
    let frames = SpoolFrames::from_receipt(&write_receipt);
    let decoded_sha256 = Sha256::hex(&decoded);

    let (cache_bytes, index_bytes) = build_cache(&decoded_sha256, decoded.len() as u64);
    let artifact_dir = dir.0.join("replay_decoder");
    fs::create_dir_all(&artifact_dir).unwrap();
    let cache_path = artifact_dir.join("checkpoints.h4cp");
    let index_path = artifact_dir.join("hour_index.h4ix");
    fs::write(&cache_path, &cache_bytes).unwrap();
    fs::write(&index_path, &index_bytes).unwrap();
    // These fixtures publish every hour, so the certifier's coverage is the
    // whole stream. The case where it is SHORTER -- a terminal FRAME_REFUSED --
    // needs a real fold and lives in
    // `replay_decoder::tests::a_terminal_frame_refusal_keeps_its_bytes_in_the_spool`.
    let coverage_path = artifact_dir.join("published_coverage.json");
    write_coverage(
        &coverage_path,
        &decoded_sha256,
        decoded.len() as u64,
        &cache_bytes,
        &index_bytes,
        3,
        decoded.len() as u64,
    );

    let receipt = DurableHourReceipt {
        checkpoint_cache_path: cache_path,
        hour_index_path: index_path,
        decoded_len: decoded.len() as u64,
        decoded_sha256,
        published_coverage_path: coverage_path,
        fold_covered_decoded_end: decoded.len() as u64,
    };

    let _scanner_guards = portable_test_fs_retry::tree_read_guards(&dir.0)
        .expect("hold the spool fixture open across the scanner's window");
    Fixture {
        _dir: dir,
        _scanner_guards,
        stream_path,
        decoded,
        frames,
        receipt,
        durable_decoded_end: decoded_len as u64,
    }
}

/// Model the boundary coarsening a very long capture performs: keep only the
/// first `keep` frame starts, so the greatest releasable boundary lands well
/// before the end of the stream and a non-empty live suffix survives.
fn coarsen(frames: &SpoolFrames, keep: usize) -> SpoolFrames {
    let mut coarse = frames.clone();
    coarse.boundaries.truncate(keep);
    coarse
}

/// Three hours of certified cache, the last of which ends at
/// `durable_decoded_end`. Built with the production encoder, so the reader the
/// trim uses accepts it for exactly the reasons production artifacts are
/// accepted — and rejects a tampered copy for the same reasons.
fn build_cache(decoded_sha256: &str, decoded_len: u64) -> (Vec<u8>, Vec<u8>) {
    let mut encoder = snapshot::ReplayCacheEncoder::new(Vec::new(), decoded_sha256, 168).unwrap();
    let step = decoded_len / 3;
    for hour in 0..3i64 {
        let start = step * hour as u64;
        let end = if hour == 2 {
            decoded_len
        } else {
            step * (hour as u64 + 1)
        };
        let world = serde_json::to_vec(&json!({
            "game_id": "spool_trim_fixture",
            "schema_version": "1",
            "hour_index": hour,
            "date": format!("1936.1.1.{hour}"),
            "countries": {"GER": {"stability": 0.5 + hour as f64 / 100.0}},
            "states": {},
        }))
        .unwrap();
        encoder
            .push(snapshot::CacheBuildHour {
                hour_index: hour,
                date: format!("1936.1.1.{hour}"),
                h4cr_start_offset: start,
                h4cr_end_offset: end,
                world_json: world,
            })
            .unwrap();
    }
    let (cache_bytes, replay_index) = encoder
        .finish(
            "spool_trim_fixture",
            "1",
            snapshot::CacheIdentityMode::Legacy,
            decoded_len,
        )
        .unwrap();
    let index_bytes = replay_index.to_bytes().unwrap();
    (cache_bytes, index_bytes)
}

/// Write a coverage receipt bound exactly the way the certifier binds one.
#[allow(clippy::too_many_arguments)]
fn write_coverage(
    path: &Path,
    decoded_sha256: &str,
    decoded_len: u64,
    cache_bytes: &[u8],
    index_bytes: &[u8],
    hour_count: u64,
    published_decoded_end_offset: u64,
) {
    let document = json!({
        "schema": snapshot::PUBLISHED_COVERAGE_SCHEMA,
        "source_len": decoded_len,
        "source_sha256": decoded_sha256,
        "checkpoint_cache_sha256": Sha256::hex(cache_bytes),
        "hour_index_sha256": Sha256::hex(index_bytes),
        "hour_count": hour_count,
        "published_decoded_end_offset": published_decoded_end_offset,
    });
    fs::write(path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
}

fn allocated_bytes(path: &Path) -> u64 {
    fs::metadata(path).unwrap().len()
}

fn read_live_region(fixture: &Fixture, journal: &TrimJournal) -> Vec<u8> {
    let (offset, compression) = journal.live_region().unwrap();
    let mut reader =
        compact_stream::open_with_compression_at(&fixture.stream_path, compression, offset).unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    bytes
}

fn released(outcome: TrimOutcome) -> (TrimJournal, u64) {
    match outcome {
        TrimOutcome::Released {
            journal,
            reclaimed_stored_bytes,
        } => (journal, reclaimed_stored_bytes),
        other => panic!("expected a release, got {other:?}"),
    }
}

/// `releasable_boundary` is the arithmetic that keeps a release inside what
/// the cache owns. It is exercised directly against partial watermarks, which
/// is the shape a concurrent fold will produce.
#[test]
fn a_release_never_reaches_past_the_durable_watermark() {
    let fixture = fixture("boundary_arithmetic", 8 << 20, 1 << 20);
    let frames = &fixture.frames;
    assert!(frames.boundaries.len() >= 8);

    for watermark in [0u64, 1, (1 << 20) - 1, 1 << 20, 3 << 20, (3 << 20) + 1, 8 << 20] {
        match frames.releasable_boundary(watermark) {
            None => assert!(
                watermark < 1 << 20,
                "a watermark of {watermark} covers at least one whole frame"
            ),
            Some(boundary) => {
                assert!(
                    boundary.decoded_offset <= watermark,
                    "boundary at decoded {} is past the watermark {watermark}",
                    boundary.decoded_offset
                );
                assert!(boundary.stored_offset > 0);
                // Nothing releasable is skipped: the next frame really is past
                // the watermark.
                let next = frames
                    .boundaries
                    .iter()
                    .find(|candidate| candidate.decoded_offset > boundary.decoded_offset);
                if let Some(next) = next {
                    assert!(
                        next.decoded_offset > watermark,
                        "boundary at decoded {} was releasable but not chosen",
                        next.decoded_offset
                    );
                }
            }
        }
    }
}

#[test]
fn a_trim_releases_only_storage_the_cache_already_owns() {
    // 8 MiB of stream, 1 MiB frames, with only the first four frame starts
    // retained: the release must stop at the greatest boundary it knows about
    // and leave the rest of the stream readable.
    let fixture = fixture("partial", 8 << 20, 1 << 20);
    let frames = coarsen(&fixture.frames, 4);
    let before = allocated_bytes(&fixture.stream_path);

    let (journal, reclaimed) =
        released(trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt).unwrap());

    assert_eq!(
        journal.durable_decoded_end_offset, fixture.durable_decoded_end,
        "the watermark must come from the cache, not from the caller"
    );
    assert_eq!(journal.durable_hour_count, 3);
    assert_eq!(
        journal.live_decoded_offset,
        3 << 20,
        "the fourth retained boundary starts at 3 MiB"
    );
    assert!(journal.live_stored_offset > 0);
    assert_eq!(journal.released_stored_bytes, journal.live_stored_offset);
    assert_eq!(journal.decoded_len, fixture.decoded.len() as u64);

    // The logical file is untouched: same path, same length, same offsets.
    assert_eq!(allocated_bytes(&fixture.stream_path), before);

    // And every decoded byte the cache does NOT own is still readable from the
    // spool, byte for byte.
    assert_eq!(
        read_live_region(&fixture, &journal),
        fixture.decoded[journal.live_decoded_offset as usize..],
        "the surviving suffix must be exactly the un-checkpointed stream"
    );

    eprintln!(
        "trim released={} reclaimed={} live_stored={} live_decoded={}",
        journal.released_stored_bytes,
        reclaimed,
        journal.live_stored_offset,
        journal.live_decoded_offset
    );
    // The reclamation is real, not a logical mark. It can never exceed what was
    // released, and the shortfall is bounded by NTFS's 64 KiB sparse allocation
    // granularity plus the deliberately conservative "before" measurement.
    assert!(reclaimed <= journal.released_stored_bytes);
    assert!(
        reclaimed >= journal.released_stored_bytes.saturating_sub(4 * 64 * 1024),
        "released {} storage bytes but the volume gave back only {reclaimed}",
        journal.released_stored_bytes
    );
}

#[test]
fn a_fully_checkpointed_spool_releases_everything_up_to_the_last_frame() {
    let decoded_len = 8 << 20;
    let fixture = fixture("whole", decoded_len, 1 << 20);
    let (journal, _) = released(
        trim_spool_prefix(&fixture.stream_path, &fixture.frames, &fixture.receipt).unwrap(),
    );
    assert_eq!(journal.live_decoded_offset, decoded_len as u64);
    // The only storage left live is the empty terminal frame `finish()` writes
    // after the last rotation, so the surviving decoded suffix is empty.
    assert!(journal.live_stored_offset > 0);
    assert!(journal.live_stored_offset <= journal.stored_len);
    assert!(read_live_region(&fixture, &journal).is_empty());
}

#[test]
fn nothing_is_released_when_no_frame_boundary_is_durable_yet() {
    // 4 MiB spool in a single frame: there is no interior boundary, so even a
    // fully durable cache cannot release a prefix.
    let fixture = fixture("single_frame", 4 << 20, u64::MAX);
    assert_eq!(fixture.frames.boundaries.len(), 1);
    match trim_spool_prefix(&fixture.stream_path, &fixture.frames, &fixture.receipt).unwrap() {
        TrimOutcome::NothingReleasable { reason } => {
            assert!(reason.contains("no storage frame boundary"), "{reason}");
        }
        other => panic!("a boundary-less spool must not be trimmed: {other:?}"),
    }
    assert!(read_journal(&fixture.stream_path).unwrap().is_none());
    let mut whole = Vec::new();
    compact_stream::open(&fixture.stream_path)
        .unwrap()
        .read_to_end(&mut whole)
        .unwrap();
    assert_eq!(whole, fixture.decoded);
}

#[test]
fn a_receipt_whose_cache_is_not_on_disk_cannot_release_anything() {
    let fixture = fixture("missing_cache", 4 << 20, 1 << 20);
    portable_test_fs_retry::remove_file(&fixture.receipt.checkpoint_cache_path).unwrap();
    let error = trim_spool_prefix(&fixture.stream_path, &fixture.frames, &fixture.receipt)
        .expect_err("a missing checkpoint cache must never authorise a release");
    assert!(error.contains("checkpoints.h4cp"), "{error}");
    assert!(read_journal(&fixture.stream_path).unwrap().is_none());
    let mut whole = Vec::new();
    compact_stream::open(&fixture.stream_path)
        .unwrap()
        .read_to_end(&mut whole)
        .unwrap();
    assert_eq!(whole, fixture.decoded, "the spool must be untouched");
}

/// A cache whose bytes have moved cannot authorise deleting the spool bytes it
/// claims to hold — and since the reader gained per-record confinement (owner
/// ruling 2026-08-12, task #191) it says so at a finer resolution: the flipped
/// byte lands in one record, the reader admits the cache while refusing the
/// hours that record serves, and the TRIM refuses because those hours are not
/// durable anywhere.
///
/// kills: deleting the `confined_hours()` refusal in `verify_durable_hours`.
/// The reader no longer refuses a damaged cache outright, so without that check
/// the trim releases the source bytes of every confined hour — the only
/// surviving copy of them.
///
/// The published-coverage receipt is re-signed over the DAMAGED cache here on
/// purpose (Codex review round 1, finding 3). Left untouched it names the
/// original digest and stops the trim on its own, so the confinement guard
/// would be unfalsifiable behind it and this test would pin only which sentence
/// comes out. Re-signing models the case that actually needs the guard: a
/// recording whose receipt honestly describes the artifacts it has, one of
/// whose records has rotted.
#[test]
fn a_rewritten_cache_cannot_release_anything() {
    let fixture = fixture("rewritten_cache", 4 << 20, 1 << 20);
    let mut bytes = portable_test_fs_retry::read(&fixture.receipt.checkpoint_cache_path).unwrap();
    let flip = bytes.len() / 2;
    bytes[flip] ^= 0xFF;
    fs::write(&fixture.receipt.checkpoint_cache_path, &bytes).unwrap();
    let coverage_path = &fixture.receipt.published_coverage_path;
    let mut coverage: serde_json::Value =
        serde_json::from_slice(&portable_test_fs_retry::read(coverage_path).unwrap()).unwrap();
    coverage["checkpoint_cache_sha256"] = json!(Sha256::hex(&bytes));
    fs::write(coverage_path, serde_json::to_vec_pretty(&coverage).unwrap()).unwrap();

    let error = trim_spool_prefix(&fixture.stream_path, &fixture.frames, &fixture.receipt)
        .expect_err("a rewritten checkpoint cache must never authorise a release");
    assert!(
        error.contains("cannot serve") && error.contains("not durable"),
        "{error}"
    );
    assert!(read_journal(&fixture.stream_path).unwrap().is_none());
}

#[test]
fn a_truncated_hour_index_cannot_release_anything() {
    let fixture = fixture("short_index", 4 << 20, 1 << 20);
    let bytes = portable_test_fs_retry::read(&fixture.receipt.hour_index_path).unwrap();
    fs::write(&fixture.receipt.hour_index_path, &bytes[..bytes.len() - 1]).unwrap();
    let _scanner_guard = portable_test_fs_retry::open(&fixture.receipt.hour_index_path)
        .expect("hold the REWRITTEN index open across the scanner's window");
    let error = trim_spool_prefix(&fixture.stream_path, &fixture.frames, &fixture.receipt)
        .expect_err("a truncated hour index must never authorise a release");
    assert!(error.contains("not bound to this"), "{error}");
    assert!(read_journal(&fixture.stream_path).unwrap().is_none());
}

/// The defect this closes: before the receipt was bound to the decoded stream,
/// any pair of files whose hashes matched what the caller claimed could
/// authorise punching an unrelated spool.
#[test]
fn a_real_cache_from_a_different_capture_cannot_release_this_spool() {
    let fixture = fixture("foreign_cache", 8 << 20, 1 << 20);

    // A perfectly well-formed, internally consistent artifact pair — just
    // built from a DIFFERENT decoded stream of the same length.
    let foreign = decoded_fixture(8 << 20);
    let mut foreign = foreign.clone();
    foreign[0] ^= 0xFF;
    let (cache_bytes, index_bytes) = build_cache(&Sha256::hex(&foreign), foreign.len() as u64);
    fs::write(&fixture.receipt.checkpoint_cache_path, &cache_bytes).unwrap();
    fs::write(&fixture.receipt.hour_index_path, &index_bytes).unwrap();

    let error = trim_spool_prefix(&fixture.stream_path, &fixture.frames, &fixture.receipt)
        .expect_err("artifacts from another capture must never authorise a release");
    assert!(error.contains("not bound to this"), "{error}");
    assert!(read_journal(&fixture.stream_path).unwrap().is_none());
    let mut whole = Vec::new();
    compact_stream::open(&fixture.stream_path)
        .unwrap()
        .read_to_end(&mut whole)
        .unwrap();
    assert_eq!(whole, fixture.decoded, "the spool must be untouched");
}

/// Fabricated bytes cannot be laundered into authority either: the cache and
/// index are parsed by the production reader, not merely hashed.
#[test]
fn fabricated_artifact_bytes_cannot_release_anything() {
    let fixture = fixture("fabricated", 4 << 20, 1 << 20);
    fs::write(&fixture.receipt.checkpoint_cache_path, b"cache".repeat(16)).unwrap();
    fs::write(&fixture.receipt.hour_index_path, b"index".repeat(16)).unwrap();
    let error = trim_spool_prefix(&fixture.stream_path, &fixture.frames, &fixture.receipt)
        .expect_err("fabricated artifacts must never authorise a release");
    assert!(error.contains("not bound to this"), "{error}");
    assert!(read_journal(&fixture.stream_path).unwrap().is_none());
}

#[test]
fn a_receipt_for_a_different_length_spool_is_refused() {
    let mut fixture = fixture("length_mismatch", 4 << 20, 1 << 20);
    fixture.receipt.decoded_len = (4 << 20) + 1;
    let error = trim_spool_prefix(&fixture.stream_path, &fixture.frames, &fixture.receipt)
        .expect_err("a receipt for a different stream length must be refused");
    assert!(error.contains("not bound to this"), "{error}");
    assert!(read_journal(&fixture.stream_path).unwrap().is_none());
}

#[test]
fn an_implausible_receipt_is_refused_before_any_file_is_read() {
    let mut fixture = fixture("implausible", 4 << 20, 1 << 20);
    fixture.receipt.decoded_sha256 = "short".to_string();
    let error = trim_spool_prefix(&fixture.stream_path, &fixture.frames, &fixture.receipt)
        .expect_err("a malformed decoded identity must be refused");
    assert!(error.contains("implausible decoded stream"), "{error}");
}

#[test]
fn a_spool_whose_length_moved_since_the_write_receipt_is_refused() {
    let fixture = fixture("length_drift", 4 << 20, 1 << 20);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&fixture.stream_path)
        .unwrap();
    file.write_all(b"appended").unwrap();
    drop(file);
    let error = trim_spool_prefix(&fixture.stream_path, &fixture.frames, &fixture.receipt)
        .expect_err("a spool that grew after its write receipt must not be trimmed");
    assert!(error.contains("bytes on disk"), "{error}");
    assert!(read_journal(&fixture.stream_path).unwrap().is_none());
}

/// The crash invariant, stated as a test.
///
/// The forbidden state is "storage released while the journal still says the
/// spool is whole". This drives the REAL `trim_spool_prefix` and fails the
/// release at the instant it would have run; swapping the two production
/// operations makes the assertion below red, because the journal would not yet
/// exist when the release ran.
#[test]
fn the_journal_is_durable_before_any_byte_is_released() {
    let fixture = fixture("ordering", 8 << 20, 1 << 20);
    let frames = coarsen(&fixture.frames, 5);

    // Before the trim: no journal, whole spool.
    assert!(read_journal(&fixture.stream_path).unwrap().is_none());
    let mut whole = Vec::new();
    compact_stream::open(&fixture.stream_path)
        .unwrap()
        .read_to_end(&mut whole)
        .unwrap();
    assert_eq!(whole, fixture.decoded);

    // Crash exactly at the release. The journal must already be on disk.
    let observed: std::cell::Cell<Option<u64>> = std::cell::Cell::new(None);
    let error = trim_spool_prefix_with_release(
        &fixture.stream_path,
        &frames,
        &fixture.receipt,
        |path, stored_offset| {
            observed.set(Some(stored_offset));
            assert!(
                read_journal(path).unwrap().is_some(),
                "the journal MUST be durable before the release runs"
            );
            Err("simulated crash inside the release".to_string())
        },
    )
    .expect_err("a failed release must surface");
    assert!(error.contains("simulated crash"), "{error}");
    let released_at = observed.get().expect("the release must have been reached");
    assert!(released_at > 0);

    // Recovery: the journal names a live region that still reads exactly.
    let journal = read_journal(&fixture.stream_path)
        .unwrap()
        .expect("the interrupted trim left its journal behind");
    assert_eq!(journal.live_stored_offset, released_at);
    assert_eq!(
        read_live_region(&fixture, &journal),
        fixture.decoded[journal.live_decoded_offset as usize..],
    );

    // And re-running is a no-op, not a second release.
    match trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt).unwrap() {
        TrimOutcome::NothingReleasable { reason } => {
            assert!(reason.contains("already trimmed"), "{reason}")
        }
        other => panic!("a re-run must not release again: {other:?}"),
    }
}

/// Every storage-blind consumer, not just the ones that remembered to ask.
///
/// The guard lives at the `compact_stream` entry points, so this asserts on the
/// codec itself: after a release, no zero-offset open or magic sniff can
/// succeed. That is what stops a raw consumer ingesting the hole plus the
/// still-compressed tail as content.
#[test]
fn a_trimmed_spool_is_refused_by_every_zero_offset_reader() {
    let fixture = fixture("guard", 8 << 20, 1 << 20);
    let frames = coarsen(&fixture.frames, 5);
    require_whole_spool(&fixture.stream_path, "container packer")
        .expect("an untrimmed spool must pass the guard");
    compact_stream::open(&fixture.stream_path).expect("an untrimmed spool must open");
    compact_stream::detect(&fixture.stream_path).expect("an untrimmed spool must sniff");

    let (journal, _) =
        released(trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt).unwrap());

    let error = require_whole_spool(&fixture.stream_path, "container packer")
        .expect_err("a trimmed spool must not be handed to a whole-spool consumer");
    assert!(error.contains("container packer"), "{error}");
    assert!(error.contains("was trimmed"), "{error}");
    assert!(
        error.contains(&journal.checkpoint_cache_sha256),
        "the refusal must name the cache that took ownership: {error}"
    );

    // The codec itself refuses, so a consumer that never heard of trimming
    // cannot decode the hole.
    let error = compact_stream::open(&fixture.stream_path)
        .err()
        .expect("open must fail closed on a released prefix");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("released"), "{error}");

    let error = compact_stream::detect(&fixture.stream_path)
        .err()
        .expect("detect must fail closed rather than call a zstd spool raw");
    assert!(error.to_string().contains("released"), "{error}");

    let error = compact_stream::open_with_compression(
        &fixture.stream_path,
        compact_stream::StreamCompression::Zstd,
    )
    .err()
    .expect("an explicitly-typed zero-offset open must fail closed too");
    assert!(error.to_string().contains("released"), "{error}");

    // Only the journal-aware path proceeds.
    assert_eq!(
        read_live_region(&fixture, &journal),
        fixture.decoded[journal.live_decoded_offset as usize..],
    );
}

/// A CANONICALISABLE alias for the spool must not look past the journal.
///
/// `require_whole_spool` derives the journal from the path it is handed, so a
/// second spelling of the same file would otherwise pass the guard and then be
/// followed straight to the released bytes. This covers what
/// `fs::canonicalize` resolves: relative spellings, `.`/`..`, symlinks,
/// junctions, 8.3 short names. Hard links are NOT covered and are pinned
/// separately below.
#[test]
fn a_canonicalisable_alias_for_a_trimmed_spool_does_not_look_past_the_journal() {
    let fixture = fixture("alias", 8 << 20, 1 << 20);
    let frames = coarsen(&fixture.frames, 5);
    let (journal, _) =
        released(trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt).unwrap());
    assert!(journal.live_stored_offset > 0);

    // A different spelling of the same file: the parent directory reached
    // through a "." component. Canonicalisation collapses it, so the guard
    // must still find the journal.
    let alias = fixture
        .stream_path
        .parent()
        .unwrap()
        .join(".")
        .join("collector_stream.bin");
    assert!(alias.is_file(), "the alias must reach the same file");
    assert!(
        read_journal(&alias).unwrap().is_some(),
        "the journal must be found through an alias"
    );
    let error = require_whole_spool(&alias, "aliased consumer")
        .expect_err("an alias must not pass the whole-spool guard");
    assert!(error.contains("was trimmed"), "{error}");
    let error = compact_stream::open(&alias)
        .err()
        .expect("the codec must refuse the alias too");
    assert!(error.to_string().contains("released"), "{error}");
}

/// Releasing storage must not disturb CY's finalize-progress accounting.
///
/// `RunDirectoryMeter` proves finalize is still working by watching the run
/// directory grow, and a supervisor bounds its patience on how long that
/// number has stood still. The release is the one part of finalize that makes
/// a run directory smaller, so three things have to hold, and this test pins
/// all three rather than trusting the reading:
///
/// 1. the spool is not one of the artifacts the meter counts, so releasing it
///    cannot move the number at all;
/// 2. the journal the release publishes is not counted either, so it cannot
///    fabricate forward progress;
/// 3. the meter reads LOGICAL length, which deallocating a sparse range does
///    not change - so even if (1) were reversed the number could not fall.
///
/// Production changes that would turn this red: adding `collector_stream` to
/// `is_recorder_finalize_output`, or switching `total_bytes` from
/// `metadata.len()` to a size-on-disk query.
#[test]
fn releasing_storage_is_invisible_to_the_finalize_progress_meter() {
    use crate::finalize_progress::{RunDirectoryMeter, WorkMeter};

    let fixture = fixture("progress_meter", 8 << 20, 1 << 20);
    let frames = coarsen(&fixture.frames, 5);
    let run_dir = fixture.stream_path.parent().unwrap().to_path_buf();

    // A metered artifact, so the meter is reading a non-zero number and a
    // regression could actually show up as a change rather than as 0 == 0.
    let spill = run_dir.join(".replay_decoder_artifacts-fixture");
    fs::create_dir_all(&spill).unwrap();
    fs::write(spill.join("payload.bin"), vec![7u8; 4096]).unwrap();

    let mut meter = RunDirectoryMeter::new(&run_dir);
    let before = meter.observe();
    assert!(before > 0, "the fixture must give the meter something to count");
    let spool_len = fs::metadata(&fixture.stream_path).unwrap().len();
    assert!(
        before < spool_len,
        "the {spool_len}-byte spool must not be inside the metered total {before}"
    );

    let (journal, reclaimed) =
        released(trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt).unwrap());
    assert!(journal.released_stored_bytes > 0);
    assert!(reclaimed > 0, "the volume must actually have given storage back");

    let after = meter.observe();
    assert_eq!(
        after, before,
        "releasing {} storage bytes moved the finalize progress meter",
        journal.released_stored_bytes
    );

    // And the logical length is untouched, which is the property the meter
    // would rely on if the spool ever did become a metered artifact.
    assert_eq!(fs::metadata(&fixture.stream_path).unwrap().len(), spool_len);
    assert!(journal_path(&fixture.stream_path).is_file());
}

/// Pin the known residual: a HARD LINK does bypass the guard.
///
/// This asserts the gap rather than papering over it. `fs::canonicalize`
/// returns a hard link's own path -- a hard link is a second directory entry
/// for the same file record, not an alias that resolves to the first -- so the
/// sidecar beside the ORIGINAL name is never consulted. Closing it needs the
/// file's identity rather than its name (`FindFirstFileNameW`, Windows-only in
/// a portable crate) and is a documented residual.
///
/// If someone implements link enumeration this test goes red, which is the
/// point: it is the reminder to update the claim in `sidecar_candidates` at
/// the same time.
#[test]
fn a_hard_link_to_a_trimmed_spool_is_a_known_uncovered_alias() {
    let fixture = fixture("hardlink_residual", 8 << 20, 1 << 20);
    let frames = coarsen(&fixture.frames, 5);
    let (journal, _) =
        released(trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt).unwrap());
    assert!(journal.live_stored_offset > 0);

    let link = fixture.stream_path.with_file_name("hardlink_alias.bin");
    if fs::hard_link(&fixture.stream_path, &link).is_err() {
        // Not every volume supports hard links; the residual is unchanged.
        return;
    }

    // The original name is still guarded.
    require_whole_spool(&fixture.stream_path, "original name")
        .expect_err("the original name must still be refused");

    // The hard link is not, and this is the documented gap.
    assert!(
        read_journal(&link).unwrap().is_none(),
        "if this now finds the journal, hard-link coverage was implemented -- \
         update sidecar_candidates' documented residual and this test together"
    );
    require_whole_spool(&link, "hard link")
        .expect("today a hard link bypasses the guard; this pins that, it does not bless it");
}

/// The coverage receipt is EVIDENCE, so tampering with it must not authorise
/// a release. This is the authority failure the round-5 review named: passing
/// the coverage as a scalar let a caller supply EOF and defeat the bound, since
/// the index's ceiling is EOF on exactly the stream shape that needs bounding.
#[test]
fn a_tampered_coverage_receipt_cannot_release_anything() {
    let fixture = fixture("coverage_authority", 8 << 20, 1 << 20);
    let frames = coarsen(&fixture.frames, 5);
    let path = &fixture.receipt.published_coverage_path;
    let original: serde_json::Value =
        serde_json::from_slice(&portable_test_fs_retry::read(path).unwrap()).unwrap();

    // Raising the coverage: the exact move that used to work.
    let mut raised = original.clone();
    raised["published_decoded_end_offset"] = json!(fixture.decoded.len() as u64 + 1);
    fs::write(path, serde_json::to_vec_pretty(&raised).unwrap()).unwrap();
    let error = trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt)
        .expect_err("coverage past the spool must be refused");
    assert!(error.contains("past the"), "{error}");

    // A receipt for different artifacts.
    let mut foreign = original.clone();
    foreign["checkpoint_cache_sha256"] = json!("f".repeat(64));
    fs::write(path, serde_json::to_vec_pretty(&foreign).unwrap()).unwrap();
    let error = trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt)
        .expect_err("a receipt for other artifacts must be refused");
    assert!(error.contains("checkpoint_cache_sha256"), "{error}");

    // A receipt for a different capture.
    let mut other_capture = original.clone();
    other_capture["source_sha256"] = json!("0".repeat(64));
    fs::write(path, serde_json::to_vec_pretty(&other_capture).unwrap()).unwrap();
    let error = trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt)
        .expect_err("a receipt for another capture must be refused");
    assert!(error.contains("source_sha256"), "{error}");

    // THE prescribed negative: every anchor correct, only the offset raised
    // to EOF. This is the forgery the round-6 review described -- all of the
    // anchors are copyable from the artifacts, so the offset is the one thing
    // a document cannot establish on its own.
    let mut forged = original.clone();
    forged["published_decoded_end_offset"] = json!(fixture.decoded.len() as u64);
    fs::write(path, serde_json::to_vec_pretty(&forged).unwrap()).unwrap();
    let mut short_receipt = fixture.receipt.clone();
    short_receipt.fold_covered_decoded_end = 2 << 20; // what the fold really said
    let error = trim_spool_prefix(&fixture.stream_path, &frames, &short_receipt)
        .expect_err("a receipt that disagrees with its own fold must authorise nothing");
    assert!(error.contains("disagrees with its own fold"), "{error}");
    fs::write(path, serde_json::to_vec_pretty(&original).unwrap()).unwrap();

    // An unknown schema.
    let mut schema = original.clone();
    schema["schema"] = json!("something-else");
    fs::write(path, serde_json::to_vec_pretty(&schema).unwrap()).unwrap();
    let error = trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt)
        .expect_err("an unknown coverage schema must be refused");
    assert!(error.contains("unsupported schema"), "{error}");

    // Missing entirely.
    portable_test_fs_retry::remove_file(path).unwrap();
    let error = trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt)
        .expect_err("a missing coverage receipt must be refused");
    assert!(error.contains("read published coverage"), "{error}");

    // Nothing was released through any of that.
    assert!(read_journal(&fixture.stream_path).unwrap().is_none());
    let mut whole = Vec::new();
    compact_stream::open(&fixture.stream_path)
        .unwrap()
        .read_to_end(&mut whole)
        .unwrap();
    assert_eq!(whole, fixture.decoded);
}

/// A coverage receipt that honestly reports SHORT coverage lowers the
/// watermark, and the release respects it.
#[test]
fn short_published_coverage_lowers_the_release_watermark() {
    let fixture = fixture("short_coverage", 8 << 20, 1 << 20);
    let frames = coarsen(&fixture.frames, 8);
    let path = &fixture.receipt.published_coverage_path;
    let mut short: serde_json::Value =
        serde_json::from_slice(&portable_test_fs_retry::read(path).unwrap()).unwrap();
    short["published_decoded_end_offset"] = json!(2u64 << 20);
    fs::write(path, serde_json::to_vec_pretty(&short).unwrap()).unwrap();
    // The verifier below re-reads this file, so the rewrite is the exposed edge.
    let _scanner_guard = portable_test_fs_retry::open(path).expect("hold the rewritten receipt");
    // Both halves move together, because a real short fold reports the same
    // number in memory and on disk.
    let mut receipt = fixture.receipt.clone();
    receipt.fold_covered_decoded_end = 2 << 20;

    let verified = verify_durable_receipt(&receipt).unwrap();
    assert_eq!(verified.indexed_decoded_end_offset, fixture.decoded.len() as u64);
    assert_eq!(verified.durable_decoded_end_offset, 2 << 20);

    let (journal, _) =
        released(trim_spool_prefix(&fixture.stream_path, &frames, &receipt).unwrap());
    assert!(
        journal.live_decoded_offset <= 2 << 20,
        "released to decoded {} past the {}-byte coverage",
        journal.live_decoded_offset,
        2u64 << 20
    );
    assert_eq!(
        read_live_region(&fixture, &journal),
        fixture.decoded[journal.live_decoded_offset as usize..]
    );
}

#[test]
fn a_journal_with_an_unknown_schema_fails_closed() {
    let fixture = fixture("schema", 4 << 20, 1 << 20);
    let path = journal_path(&fixture.stream_path);
    fs::write(
        &path,
        br#"{"schema":"something-else","stream_file":"collector_stream.bin","compression":"zstd","decoded_len":1,"stored_len":1,"live_stored_offset":1,"live_decoded_offset":1,"released_stored_bytes":1,"durable_hour_count":1,"durable_decoded_end_offset":1,"checkpoint_cache_sha256":"x","hour_index_sha256":"y"}"#,
    )
    .unwrap();
    let error = read_journal(&fixture.stream_path).expect_err("an unknown schema must fail closed");
    assert!(error.contains("unsupported schema"), "{error}");
    let error = require_whole_spool(&fixture.stream_path, "fold")
        .expect_err("the guard must inherit the failure");
    assert!(error.contains("unsupported schema"), "{error}");
}

/// An unrecognised storage codec in the journal is a load error, never a
/// silent decision to read the surviving suffix as raw.
#[test]
fn a_journal_with_an_unknown_compression_fails_closed() {
    let fixture = fixture("unknown_codec", 4 << 20, 1 << 20);
    let path = journal_path(&fixture.stream_path);
    let journal = serde_json::json!({
        "schema": TRIM_JOURNAL_SCHEMA,
        "stream_file": "collector_stream.bin",
        "compression": "brotli",
        "decoded_len": 1,
        "stored_len": 1,
        "live_stored_offset": 1,
        "live_decoded_offset": 1,
        "released_stored_bytes": 1,
        "durable_hour_count": 1,
        "durable_decoded_end_offset": 1,
        "checkpoint_cache_sha256": "a".repeat(64),
        "hour_index_sha256": "b".repeat(64),
    });
    fs::write(&path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();

    let error =
        read_journal(&fixture.stream_path).expect_err("an unknown codec must fail closed");
    assert!(error.contains("unknown storage encoding"), "{error}");
    let error = require_whole_spool(&fixture.stream_path, "fold")
        .expect_err("the guard must inherit the failure");
    assert!(error.contains("unknown storage encoding"), "{error}");
}

#[test]
fn the_journal_round_trips_through_its_own_serialization() {
    let fixture = fixture("round_trip", 8 << 20, 1 << 20);
    let frames = coarsen(&fixture.frames, 5);
    let (journal, _) =
        released(trim_spool_prefix(&fixture.stream_path, &frames, &fixture.receipt).unwrap());
    let reloaded = read_journal(&fixture.stream_path).unwrap().unwrap();
    assert_eq!(reloaded, journal);
    let summary = trim_summary_json(&TrimOutcome::Released {
        journal: journal.clone(),
        reclaimed_stored_bytes: 4096,
    });
    assert_eq!(summary["released"], json!(true));
    assert_eq!(summary["reclaimed_stored_bytes"], json!(4096));
    assert_eq!(
        summary["live_stored_offset"],
        json!(journal.live_stored_offset)
    );
}
