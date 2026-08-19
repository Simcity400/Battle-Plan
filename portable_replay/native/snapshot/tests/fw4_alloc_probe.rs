use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

static PEAK_REQUEST: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    static REFUSE_ABOVE: Cell<usize> = const { Cell::new(0) };
}

fn set_refuse_above(ceiling: usize) {
    REFUSE_ABOVE.with(|value| value.set(ceiling));
}

fn refused(size: usize) -> bool {
    let ceiling = REFUSE_ABOVE.try_with(Cell::get).unwrap_or(0);
    ceiling != 0 && size > ceiling
}

struct Probe;

unsafe impl GlobalAlloc for Probe {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        PEAK_REQUEST.fetch_max(layout.size(), Ordering::Relaxed);
        if refused(layout.size()) {
            return std::ptr::null_mut();
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        PEAK_REQUEST.fetch_max(new_size, Ordering::Relaxed);
        if refused(new_size) {
            return std::ptr::null_mut();
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOC: Probe = Probe;

use portable_replay_format::replay_cache::{
    CacheBuildHour, CacheIdentityMode, CertifiedReplayCache, ReplayCacheEncoder, ReplayHourIndex,
};
use portable_replay_format::sha256::Sha256;

const INDEX_MAGIC: &[u8; 4] = b"H4IX";
const CACHE_MAGIC: &[u8; 4] = b"H4CP";

fn format_version() -> u16 {
    let mut encoder = ReplayCacheEncoder::new(Vec::new(), &hex32(0xab), 8).expect("encoder");
    encoder
        .push(CacheBuildHour {
            hour_index: 0,
            date: "1936.1.1.1".to_string(),
            h4cr_start_offset: 0,
            h4cr_end_offset: 100,
            world_json: br#"{"date":"1936.1.1.1","game_id":"g","hour_index":0,"schema_version":"1","world":{}}"#
                .to_vec(),
        })
        .expect("push");
    let (cache, _) = encoder
        .finish("g", "1", CacheIdentityMode::IdentityV1, 100)
        .expect("finish");
    u16::from_le_bytes([cache[4], cache[5]])
}
const CACHE_HEADER_LEN: u64 = 40;

fn hex32(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

fn digest_of(hex: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |c: u8| match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => unreachable!(),
        };
        out[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    out
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        out.push(if value == 0 { byte } else { byte | 0x80 });
        if value == 0 {
            return;
        }
    }
}

fn hostile_cache(tag: u8, count: u32) -> (Vec<u8>, Vec<u8>, u64, String) {
    let mut payload = vec![tag];
    put_varint(&mut payload, u64::from(count));
    cache_around_payload(&payload)
}

fn cache_around_payload(payload: &[u8]) -> (Vec<u8>, Vec<u8>, u64, String) {
    let source_sha = hex32(0xcd);
    let source_len = 100u64;

    let mut cache = Vec::new();
    cache.extend_from_slice(CACHE_MAGIC);
    cache.extend_from_slice(&format_version().to_le_bytes());
    cache.extend_from_slice(&0u16.to_le_bytes());
    cache.extend_from_slice(&digest_of(&source_sha));
    let record_offset = cache.len() as u64;
    cache.push(1);
    cache.extend_from_slice(&[0, 0, 0]);
    cache.extend_from_slice(&0i64.to_le_bytes());
    cache.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    cache.push(0);
    cache.extend_from_slice(payload);
    let record_len = cache.len() as u64 - record_offset;
    let cache_len = cache.len() as u64;
    let cache_sha = Sha256::hex(&cache);

    let mut index = Vec::new();
    index.extend_from_slice(INDEX_MAGIC);
    index.extend_from_slice(&format_version().to_le_bytes());
    index.extend_from_slice(&0u16.to_le_bytes());
    index.push(1);
    index.extend_from_slice(&[0, 0, 0]);
    index.extend_from_slice(&2u32.to_le_bytes());
    index.extend_from_slice(&source_len.to_le_bytes());
    index.extend_from_slice(&cache_len.to_le_bytes());
    index.extend_from_slice(&digest_of(&source_sha));
    index.extend_from_slice(&digest_of(&cache_sha));
    put_string(&mut index, "g");
    put_string(&mut index, "1");
    index.extend_from_slice(&1u64.to_le_bytes());
    index.extend_from_slice(&0i64.to_le_bytes());
    index.extend_from_slice(&0u64.to_le_bytes());
    index.extend_from_slice(&source_len.to_le_bytes());
    index.extend_from_slice(&record_offset.to_le_bytes());
    index.extend_from_slice(&record_len.to_le_bytes());
    index.extend_from_slice(&0u32.to_le_bytes());
    index.extend_from_slice(&0u32.to_le_bytes());
    index.extend_from_slice(&digest_of(&hex32(0x00)));
    index.extend_from_slice(&digest_of(&Sha256::hex(
        &cache[record_offset as usize..(record_offset + record_len) as usize],
    )));
    put_string(&mut index, "1936.1.1.1");
    index.extend_from_slice(&0u64.to_le_bytes());

    (index, cache, source_len, source_sha)
}

fn truncated_hour_index(hour_count: u64) -> Vec<u8> {
    let source_sha = hex32(0xcd);
    let mut hostile = Vec::new();
    hostile.extend_from_slice(INDEX_MAGIC);
    hostile.extend_from_slice(&format_version().to_le_bytes());
    hostile.extend_from_slice(&0u16.to_le_bytes());
    hostile.push(1);
    hostile.extend_from_slice(&[0, 0, 0]);
    hostile.extend_from_slice(&2u32.to_le_bytes());
    hostile.extend_from_slice(&100u64.to_le_bytes());
    hostile.extend_from_slice(&CACHE_HEADER_LEN.to_le_bytes());
    hostile.extend_from_slice(&digest_of(&source_sha));
    hostile.extend_from_slice(&digest_of(&hex32(0x11)));
    put_string(&mut hostile, "g");
    put_string(&mut hostile, "1");
    hostile.extend_from_slice(&hour_count.to_le_bytes());
    hostile
}

const PROPORTIONALITY_CEILING: usize = 1024 * 1024;

#[test]
fn hostile_counts_never_drive_a_disproportionate_allocation_request() {
    println!(
        "size_of::<serde_json::Value>() = {}",
        std::mem::size_of::<serde_json::Value>()
    );

    let (index_bytes, cache, source_len, source_sha) = hostile_cache(5, 0x1000_0000);
    let input_bytes = index_bytes.len() + cache.len();
    PEAK_REQUEST.store(0, Ordering::Relaxed);
    let outcome = CertifiedReplayCache::open(&index_bytes, cache, source_len, &source_sha)
        .and_then(|certified| certified.reconstruct_json(0).map(|_| ()));
    let peak = PEAK_REQUEST.load(Ordering::Relaxed);
    println!(
        "R2-1 array: input {input_bytes} bytes -> peak single request {peak} bytes; outcome {outcome:?}"
    );
    assert!(outcome.is_err(), "hostile array count must be refused");
    assert!(
        peak < PROPORTIONALITY_CEILING,
        "a {input_bytes}-byte cache requested {peak} bytes (pre-fix: 8,589,934,592)"
    );

    let hostile = truncated_hour_index(2_000_000);
    let input_bytes = hostile.len();
    PEAK_REQUEST.store(0, Ordering::Relaxed);
    let outcome = ReplayHourIndex::from_bytes(&hostile).map(|_| ());
    let peak = PEAK_REQUEST.load(Ordering::Relaxed);
    println!(
        "R2-2 hour index: input {input_bytes} bytes -> peak single request {peak} bytes; outcome {outcome:?}"
    );
    assert!(outcome.is_err(), "hostile hour count must be refused");
    assert!(
        peak < PROPORTIONALITY_CEILING,
        "a {input_bytes}-byte index requested {peak} bytes (pre-fix: 192,000,000)"
    );
}

#[test]
fn an_allocator_refusal_becomes_a_typed_error_not_an_abort() {
    const ELEMENTS: u32 = 40_000;
    const CEILING: usize = 1024 * 1024;

    let mut payload = vec![5u8];
    put_varint(&mut payload, u64::from(ELEMENTS));
    payload.extend(std::iter::repeat(0u8).take(ELEMENTS as usize));
    let (index_bytes, cache, source_len, source_sha) = cache_around_payload(&payload);
    assert!(
        (ELEMENTS as usize) * std::mem::size_of::<serde_json::Value>() > CEILING,
        "the reserve must exceed the ceiling for this test to mean anything"
    );

    set_refuse_above(CEILING);
    let outcome = CertifiedReplayCache::open(&index_bytes, cache, source_len, &source_sha)
        .and_then(|certified| certified.reconstruct_json(0).map(|_| ()));
    set_refuse_above(0);

    let error = outcome.expect_err("an allocator refusal must surface as a typed refusal");
    assert!(
        error.message().contains("cannot allocate"),
        "refusal must name the allocation failure, got {error}"
    );
}
