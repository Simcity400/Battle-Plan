//! Lane LL: price the committed-baseline delta ("revive") with THE PRODUCTION
//! ENCODER over the real hours of a sealed recording, and prove the reader
//! reconstructs every one of them byte for byte.
//!
//! WHY AN INSTRUMENT AND NOT A REIMPLEMENTATION. The number this lane owes the
//! owner is a shipped-size number, so it has to come from the encoder that
//! ships: `ReplayCacheEncoder` here is the same type `replay_decoder` drives at
//! finalize, `verify_every_hour` is the same check the seal path runs, and the
//! container level is `pack::MAX_ZSTD_COMPRESSION_LEVEL` -- the ceiling the
//! owner set -- rather than a level chosen to flatter the result.
//!
//! HOW THE HOURS GET HERE. The already-sealed caches on disk are format 4 and
//! this build reads format 5, so the hours arrive on stdin from the read-only
//! Python probe that walks the old records (`h4cp_probe.py`). That is a
//! transport, not a re-encoding: every hour carries the `world_json_sha256` the
//! ORIGINAL recording published, this instrument requires the JSON it receives
//! to hash to it, and the reader afterwards requires each reconstructed hour to
//! hash to it again. A transport that altered a byte could not pass either.
//!
//! Never on by default, never packaged, writes only where it is told.
//!
//! stdin framing, little-endian, repeated until tag 0:
//!   u8 tag (1 = hour, 0 = end)
//!   i64 hour_index | u32 len + date | u32 len + world_json_sha256
//!   u64 h4cr_start | u64 h4cr_end | u64 len + world JSON
use std::io::{Read, Write};

use portable_replay_format::pack::{
    ce_production_zstd_encoder, CompressionProfile, MAX_ZSTD_COMPRESSION_LEVEL,
};
use portable_replay_format::replay_cache::{
    CacheBuildHourMeta, CacheIdentityMode, CanonicalWorldHour, ReplayCacheEncoder,
    ReplayCacheReader,
};

struct In<R: Read> {
    inner: R,
}

impl<R: Read> In<R> {
    fn take(&mut self, n: usize) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; n];
        self.inner.read_exact(&mut buf)?;
        Ok(buf)
    }
    fn u8(&mut self) -> std::io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> std::io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn u64(&mut self) -> std::io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }
    fn i64(&mut self) -> std::io::Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }
    fn text(&mut self) -> std::io::Result<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("ll_revive_pricing: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let label = args.next().unwrap_or_else(|| "run".to_string());
    let checkpoint_interval: u32 = args
        .next()
        .unwrap_or_else(|| "168".to_string())
        .parse()
        .map_err(|e| format!("checkpoint interval: {e}"))?;

    let stdin = std::io::stdin();
    let mut input = In {
        inner: std::io::BufReader::with_capacity(1 << 20, stdin.lock()),
    };

    let mut encoder = ReplayCacheEncoder::new(Vec::new(), &"0".repeat(64), checkpoint_interval)
        .map_err(|e| format!("encoder: {e}"))?;
    // Emission is the shipped default now; nothing to turn on.

    let mut hours = 0u64;
    let mut checkpoint_payload = 0u64;
    let mut delta_payload = 0u64;
    let mut source_len = 0u64;
    loop {
        let tag = input.u8().map_err(|e| format!("read tag: {e}"))?;
        if tag == 0 {
            break;
        }
        let hour_index = input.i64().map_err(|e| format!("hour index: {e}"))?;
        let date = input.text().map_err(|e| format!("date: {e}"))?;
        let stated_sha = input.text().map_err(|e| format!("sha: {e}"))?;
        let h4cr_start_offset = input.u64().map_err(|e| format!("h4cr start: {e}"))?;
        let h4cr_end_offset = input.u64().map_err(|e| format!("h4cr end: {e}"))?;
        let json_len = input.u64().map_err(|e| format!("json len: {e}"))? as usize;
        let json = input.take(json_len).map_err(|e| format!("json: {e}"))?;

        let prepared = CanonicalWorldHour::parse(json, hour_index)
            .map_err(|e| format!("hour {hour_index}: {e}"))?;
        // THE TRANSPORT IS CHECKED HERE, before a single byte is encoded: this
        // must be the very world the original recording sealed.
        // An EMPTY digest means the caller is deliberately feeding a modified
        // document -- the wire-deletion comparison -- and says so explicitly
        // rather than by passing a digest that happens not to match.
        if !stated_sha.is_empty() && prepared.world_json_sha256() != stated_sha {
            return Err(format!(
                "hour {hour_index} arrived as different bytes from the ones the recording sealed"
            ));
        }

        encoder
            .push_prepared(
                CacheBuildHourMeta {
                    hour_index,
                    date,
                    h4cr_start_offset,
                    h4cr_end_offset,
                },
                prepared,
            )
            .map_err(|e| format!("hour {hour_index}: {e}"))?;
        source_len = h4cr_end_offset;
        hours += 1;
    }

    let (cache, index) = encoder
        .finish("g", "1", CacheIdentityMode::IdentityV1, source_len)
        .map_err(|e| format!("finish: {e}"))?;

    // Per-record payload bytes, split so the checkpoint cost is never confused
    // with the delta cost the lever actually moves.
    for entry in index.hours() {
        let payload = entry.cache_record_len() - 21;
        if entry.delta_steps() == 0 {
            checkpoint_payload += payload;
        } else {
            delta_payload += payload;
        }
    }

    // THE READER'S OWN PROOF, not a private one: every hour is reconstructed
    // from the finished artifact and required to hash to what the original
    // recording published for it.
    let index_bytes = index.to_bytes().map_err(|e| format!("index: {e}"))?;
    let reader = ReplayCacheReader::open(&index_bytes, cache.as_slice(), source_len, &"0".repeat(64))
        .map_err(|e| format!("reader: {e}"))?;
    let max_seek = reader
        .verify_every_hour()
        .map_err(|e| format!("ROUND-TRIP FAILED: {e}"))?;

    // THIS INSTRUMENT STILL OPENS NO PATH, AND NOW DOES NOT NEED TO. Lane LV
    // wanted the finished artifact on disk so the per-path sealed decomposition
    // could be taken over the format that would actually SHIP rather than over
    // the older records on disk, and briefly added write lines here to get it;
    // that is not kept, because `replay_core`'s
    // `production_sources_have_no_descendant_pathname_read` audits every module
    // under this crate's `src/` and a dev instrument is not a reason to teach a
    // production capability gate four new classified write sites. (Do not name
    // the standard filesystem module even in a comment: the audit's capability
    // detector reads the whole source text, so a mention alone makes this
    // module claim a capability it has no site for.)
    //
    // A THIRD ARGUMENT `dump` instead hands the artifact back the way this
    // instrument already hands back its answer -- down the pipe it was given.
    // The caller writes it wherever it wants; this module names nothing.
    // Framing: u64 index length, index, u64 cache length, cache, then the
    // usual one-line JSON.
    if std::env::args().nth(3).as_deref() == Some("dump") {
        let mut out = std::io::stdout().lock();
        out.write_all(&(index_bytes.len() as u64).to_le_bytes())
            .and_then(|()| out.write_all(&index_bytes))
            .and_then(|()| out.write_all(&(cache.len() as u64).to_le_bytes()))
            .and_then(|()| out.write_all(cache.as_slice()))
            .and_then(|()| out.flush())
            .map_err(|e| format!("dump: {e}"))?;
    }

    let raw = cache.len() as u64;
    let sealed = seal(&cache).map_err(|e| format!("seal: {e}"))?;

    let mut out = std::io::stdout();
    writeln!(
        out,
        "{{\"label\":\"{label}\",\"hours\":{hours},\"raw\":{raw},\"sealed\":{sealed},\
         \"checkpoint_payload\":{checkpoint_payload},\"delta_payload\":{delta_payload},\
         \"zstd_level\":{MAX_ZSTD_COMPRESSION_LEVEL},\"max_seek_deltas\":{max_seek},\
         \"roundtrip\":\"every hour byte-identical to the sealed recording\"}}"
    )
    .map_err(|e| format!("write: {e}"))?;
    Ok(())
}

fn seal(bytes: &[u8]) -> std::io::Result<u64> {
    let profile = CompressionProfile::default();
    let mut encoder = ce_production_zstd_encoder(Vec::new(), profile)?;
    encoder.write_all(bytes)?;
    let sealed = encoder.finish()?;
    Ok(sealed.len() as u64)
}
