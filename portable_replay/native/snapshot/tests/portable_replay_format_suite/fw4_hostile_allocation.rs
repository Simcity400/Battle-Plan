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
const RECORD_CHECKPOINT: u8 = 1;
const CACHE_HEADER_LEN: u64 = 4 + 2 + 2;

fn hex32(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

fn digest_of(hex: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |c: u8| match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => unreachable!("test digest is lowercase hex"),
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
    let source_sha = hex32(0xcd);
    let source_len = 100u64;

    let mut payload = Vec::new();
    payload.push(tag);
    put_varint(&mut payload, u64::from(count));

    let mut cache = Vec::new();
    cache.extend_from_slice(CACHE_MAGIC);
    cache.extend_from_slice(&format_version().to_le_bytes());
    cache.extend_from_slice(&0u16.to_le_bytes());
    assert_eq!(cache.len() as u64, CACHE_HEADER_LEN);

    let record_offset = cache.len() as u64;
    cache.push(RECORD_CHECKPOINT);
    cache.extend_from_slice(&[0, 0, 0]);
    cache.extend_from_slice(&0i64.to_le_bytes());
    let sealed = portable_replay_format::seal_cache_payload(&payload).unwrap();
    let mut payload = Vec::new();
    put_varint(&mut payload, 1);
    put_varint(&mut payload, 5);
    payload.extend_from_slice(b"world");
    payload.extend_from_slice(&[0, 1]);
    put_varint(&mut payload, sealed.len() as u64);
    payload.extend_from_slice(&sealed);
    cache.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    cache.push(0);
    cache.extend_from_slice(&payload);
    let record_len = cache.len() as u64 - record_offset;
    cache.extend_from_slice(&digest_of(&source_sha));
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

#[test]
fn array_count_beyond_remaining_input_is_a_typed_refusal_not_an_abort() {
    let (index_bytes, cache, source_len, source_sha) = hostile_cache(5, 0x1000_0000);
    assert!(
        index_bytes.len() + cache.len() < 1024,
        "the hostile artifact must stay tiny to make the amplification the point"
    );
    let certified =
        CertifiedReplayCache::open(&index_bytes, cache, source_len, &source_sha).unwrap();
    let error = certified
        .reconstruct_json(0)
        .expect_err("a 268M-element array declared by a sub-kilobyte cache must be refused");
    assert!(
        error.message().contains("input bytes remain"),
        "refusal must name the remaining-input bound, got {error}"
    );
}

#[test]
fn object_count_beyond_remaining_input_is_a_typed_refusal() {
    let (index_bytes, cache, source_len, source_sha) = hostile_cache(6, 0x1000_0000);
    let certified =
        CertifiedReplayCache::open(&index_bytes, cache, source_len, &source_sha).unwrap();
    let error = certified.reconstruct_json(0).expect_err("must be refused");
    assert!(
        error.message().contains("input bytes remain"),
        "refusal must name the remaining-input bound, got {error}"
    );
}

#[test]
fn hour_count_beyond_remaining_input_is_a_typed_refusal() {
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
    hostile.extend_from_slice(&2_000_000u64.to_le_bytes());
    assert!(
        hostile.len() < 200,
        "the truncated index must stay tiny, got {} bytes",
        hostile.len()
    );
    let error = ReplayHourIndex::from_bytes(&hostile)
        .expect_err("2,000,000 hours declared by a ~114-byte index must be refused");
    assert!(
        error.message().contains("input bytes remain"),
        "refusal must name the remaining-input bound, got {error}"
    );
}

#[test]
fn ordinary_encoder_output_still_round_trips() {
    use portable_replay_format::replay_cache::{
        CacheBuildHour, CacheIdentityMode, ReplayCacheEncoder,
    };

    let source_sha = hex32(0xab);
    let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha, 3).unwrap();
    for hour in 0i64..5 {
        let date = format!("1936.1.1.{}", hour + 1);
        encoder
            .push(CacheBuildHour {
                hour_index: hour,
                date: date.clone(),
                h4cr_start_offset: (hour as u64) * 100,
                h4cr_end_offset: (hour as u64 + 1) * 100,
                world_json: serde_json::to_vec(&serde_json::json!({
                    "date": date,
                    "hour_index": hour,
                    "world": {"rows": [1, 2, hour], "tag": format!("t{hour}")},
                }))
                .unwrap(),
            })
            .unwrap();
    }
    let (cache, index) = encoder
        .finish("g", "1", CacheIdentityMode::IdentityV1, 500)
        .unwrap();
    let index_bytes = index.to_bytes().unwrap();
    assert_eq!(
        ReplayHourIndex::from_bytes(&index_bytes)
            .unwrap()
            .hours()
            .len(),
        5
    );
    let certified = CertifiedReplayCache::open(&index_bytes, cache, 500, &source_sha).unwrap();
    for hour in 0..5 {
        certified.reconstruct_json(hour).unwrap();
    }
}
