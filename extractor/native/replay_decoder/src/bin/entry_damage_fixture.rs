
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compact_contract::calendar::DATE_EPOCH_CONST;
use compact_contract::campaign_identity::{
    encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
    COMPACT_RECORD_TYPE_STREAM_HEADER,
};
use compact_contract::stream_validator::StreamIdentityMode;
use compact_contract::{
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION, COMPACT_STATUS_DELTA_DATA,
};
use snapshot::sha256::Sha256;

const DOMAIN_FRAME: u16 = 1000;
const D18_COUNTRY_IDENTITY: u16 = 18;
const D13_TECH_LIVE: u16 = 13;
const CAMPAIGN_UUID: &str = "a4b17fed-ed12-4361-9064-fb6576638110";
const BASE_HOUR: i64 = 16_959_360;
const HOURS: usize = 12;
const GAME_ID: &str = "ha_entry_confinement";
const CHECKPOINT_INTERVAL: u32 = 4;

fn raw_for(hour_index: i64) -> i32 {
    DATE_EPOCH_CONST + hour_index as i32
}

fn hour_of(ordinal: usize) -> i64 {
    BASE_HOUR + ordinal as i64
}


fn write_record(
    out: &mut Vec<u8>,
    seq: u64,
    game_hour_raw: i32,
    domain: u16,
    ty: u16,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain.to_le_bytes());
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
}

fn d18_payload(rows: &[(u32, &str)], status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes());
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    p.extend_from_slice(&status_flags.to_le_bytes());
    for (id, tag) in rows {
        p.extend_from_slice(&id.to_le_bytes());
        p.extend_from_slice(&(tag.len() as u16).to_le_bytes());
        p.extend_from_slice(tag.as_bytes());
        p.push(1);
    }
    p
}

fn d13_payload(countries: &[(u32, u32, u32)], status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes());
    p.extend_from_slice(&(countries.len() as u32).to_le_bytes());
    p.extend_from_slice(&(countries.len() as u32).to_le_bytes());
    p.extend_from_slice(&status_flags.to_le_bytes());
    p.extend_from_slice(
        &recorder_host::domain_data_authority::COMPACT_TECH_PROGRESS_SCALE_RAW.to_le_bytes(),
    );
    p.extend_from_slice(
        &recorder_host::domain_data_authority::COMPACT_TECH_LIVE_PROGRESS_SCALE_RAW.to_le_bytes(),
    );
    p.extend_from_slice(&0u32.to_le_bytes());
    for (tag, completed, slots) in countries {
        p.extend_from_slice(&tag.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&completed.to_le_bytes());
        p.extend_from_slice(&slots.to_le_bytes());
    }
    p
}

fn cid1_record_payload() -> Vec<u8> {
    let observation = Cid1Observation {
        game_unique_id: CAMPAIGN_UUID.to_string(),
        game_unique_seed: 1_026_046_527,
        game_hour_raw: raw_for(BASE_HOUR),
    };
    let fields = Cid1Fields::from_stable_observations(
        observation.clone(),
        observation,
        "vanilla_latest_2026_07_20".to_string(),
        Cid1Witnesses::default(),
    )
    .expect("CID1 fields from stable observations");
    encode_cid1(&fields).expect("encode CID1")
}

fn synthesize() -> Vec<u8> {
    let mut bytes = Vec::new();
    write_record(
        &mut bytes,
        0,
        raw_for(BASE_HOUR),
        COMPACT_DOMAIN_STREAM_HEADER,
        COMPACT_RECORD_TYPE_STREAM_HEADER,
        &cid1_record_payload(),
    );
    let mut seq = 1u64;
    for hour in 0..HOURS {
        let raw = raw_for(hour_of(hour));
        write_record(
            &mut bytes,
            seq,
            raw,
            D18_COUNTRY_IDENTITY,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(&[(47, "GER"), (48, "FRA")], 0),
        );
        let tech = if hour == 0 {
            d13_payload(&[(47, 10, 3), (48, 20, 2)], 0)
        } else {
            d13_payload(&[(47, 10 + hour as u32, 3)], COMPACT_STATUS_DELTA_DATA)
        };
        write_record(
            &mut bytes,
            seq + 1,
            raw,
            D13_TECH_LIVE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &tech,
        );
        write_record(
            &mut bytes,
            seq + 2,
            raw,
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &vec![0u8; 32],
        );
        seq += 3;
    }
    bytes
}


struct BuiltPair {
    index_bytes: Vec<u8>,
    cache_path: PathBuf,
    source_len: u64,
    source_sha256: String,
    records: Vec<(i64, u64, u64)>,
}

fn build_clean_pair(scratch: &Path) -> BuiltPair {
    let bytes = synthesize();
    let stream_path = scratch.join("clean.h4cr");
    fs::write(&stream_path, &bytes).expect("write clean stream");
    let out_dir = scratch.join("clean.artifacts");
    if out_dir.exists() {
        fs::remove_dir_all(&out_dir).expect("clear artifact spill");
    }
    fs::create_dir_all(&out_dir).expect("create artifact spill");
    let source_sha256 = Sha256::hex(&bytes);
    let artifacts = replay_decoder::build_certified_replay_artifacts(
        &stream_path,
        &out_dir,
        GAME_ID,
        StreamIdentityMode::IdentityV1,
        bytes.len() as u64,
        &source_sha256,
        CHECKPOINT_INTERVAL,
    )
    .expect("the clean control must build; nothing below measures anything if it does not");

    let index_bytes = fs::read(&artifacts.hour_index_path).expect("read packaged H4IX");
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes).expect("packaged H4IX parses");
    let records = index
        .hours()
        .iter()
        .map(|row| {
            (
                row.hour_index(),
                row.cache_record_offset(),
                row.cache_record_len(),
            )
        })
        .collect();
    BuiltPair {
        index_bytes,
        cache_path: artifacts.checkpoint_cache_path,
        source_len: bytes.len() as u64,
        source_sha256,
        records,
    }
}

enum Injection {
    RecordPayload(usize),
    FileHeader,
}

fn injected_cache(scratch: &Path, tag: &str, pair: &BuiltPair, injections: &[Injection]) -> PathBuf {
    let cell_dir = scratch.join(tag);
    if cell_dir.exists() {
        fs::remove_dir_all(&cell_dir).expect("clear cell dir");
    }
    fs::create_dir_all(&cell_dir).expect("create cell dir");
    let cache_path = cell_dir.join("checkpoints.h4cp");
    let mut cache = fs::read(&pair.cache_path).expect("read clean H4CP");
    for injection in injections {
        let offset = match injection {
            Injection::RecordPayload(position) => {
                let (_, record_offset, record_len) = pair.records[*position];
                (record_offset + record_len - 1) as usize
            }
            Injection::FileHeader => 8,
        };
        cache[offset] ^= 0xff;
    }
    fs::write(&cache_path, &cache).expect("write injected H4CP");
    cache_path
}


struct Observed {
    served: BTreeSet<usize>,
    refused: BTreeMap<usize, String>,
    total_proof: Result<u32, String>,
}

fn observe(pair: &BuiltPair, cache_path: &Path) -> Result<Observed, String> {
    let cache_file = fs::File::open(cache_path).expect("open injected H4CP");
    let cache = snapshot::CertifiedReplayCache::open_file(
        &pair.index_bytes,
        cache_file,
        pair.source_len,
        &pair.source_sha256,
    )
    .map_err(|error| error.to_string())?;

    let mut served = BTreeSet::new();
    let mut refused = BTreeMap::new();
    for ordinal in 0..HOURS {
        match cache.reconstruct_json(hour_of(ordinal)) {
            Ok(_) => {
                served.insert(ordinal);
            }
            Err(error) => {
                refused.insert(ordinal, error.to_string());
            }
        }
    }
    Ok(Observed {
        served,
        refused,
        total_proof: cache.verify_every_hour().map_err(|error| error.to_string()),
    })
}


enum Expect {
    Confined(BTreeSet<usize>),
    WholeArtifactRefusal(&'static str),
}

fn confined_window(position: usize) -> BTreeSet<usize> {
    let interval = CHECKPOINT_INTERVAL as usize;
    let window_end = (position - position % interval + interval).min(HOURS);
    (position..window_end).collect()
}

struct Row {
    cell: String,
    shape: String,
    verdict: String,
    served: String,
    confined: String,
    detail: String,
}

fn judge(cell: &str, shape: &str, expect: &Expect, outcome: &Result<Observed, String>) -> Row {
    let observed = match (outcome, expect) {
        (Err(error), Expect::WholeArtifactRefusal(must_name)) => {
            let named = error.to_lowercase().contains(&must_name.to_lowercase());
            return Row {
                cell: cell.to_string(),
                shape: shape.to_string(),
                verdict: if named {
                    "PASS".to_string()
                } else {
                    "FAIL (whole-artifact refusal does not name the injury)".to_string()
                },
                served: format!("0/{HOURS}"),
                confined: "whole artifact".to_string(),
                detail: one_line(error),
            };
        }
        (Err(error), Expect::Confined(_)) => {
            return Row {
                cell: cell.to_string(),
                shape: shape.to_string(),
                verdict: "FAIL (whole-artifact refusal)".to_string(),
                served: format!("0/{HOURS}"),
                confined: "whole artifact".to_string(),
                detail: one_line(error),
            }
        }
        (Ok(observed), Expect::WholeArtifactRefusal(must_name)) => {
            return Row {
                cell: cell.to_string(),
                shape: shape.to_string(),
                verdict: format!(
                    "FAIL (admitted an artifact whose damage serves no hour; {} hour(s) served)",
                    observed.served.len()
                ),
                served: format!("{}/{HOURS}", observed.served.len()),
                confined: "—".to_string(),
                detail: format!("this damage serves no single hour; the refusal had to name {must_name:?}"),
            }
        }
        (Ok(observed), Expect::Confined(_)) => observed,
    };
    let Expect::Confined(expected_confined) = expect else {
        unreachable!("the other arms returned");
    };
    let expected_served: BTreeSet<usize> = (0..HOURS)
        .filter(|ordinal| !expected_confined.contains(ordinal))
        .collect();
    let observed_confined: BTreeSet<usize> = observed.refused.keys().copied().collect();

    let mut failures = Vec::new();
    let lost: Vec<usize> = expected_served
        .difference(&observed.served)
        .copied()
        .collect();
    if !lost.is_empty() {
        failures.push(format!(
            "hour(s) {:?} are outside the damaged window but do not serve",
            lost.iter().map(|o| hour_of(*o)).collect::<Vec<_>>()
        ));
    }
    let leaked: Vec<usize> = observed
        .served
        .intersection(expected_confined)
        .copied()
        .collect();
    if !leaked.is_empty() {
        failures.push(format!(
            "hour(s) {:?} replay through a damaged record yet were served",
            leaked.iter().map(|o| hour_of(*o)).collect::<Vec<_>>()
        ));
    }
    if &observed_confined != expected_confined {
        failures.push(format!(
            "refused {:?}, the law says {:?}",
            observed_confined
                .iter()
                .map(|o| hour_of(*o))
                .collect::<Vec<_>>(),
            expected_confined
                .iter()
                .map(|o| hour_of(*o))
                .collect::<Vec<_>>()
        ));
    }
    for (ordinal, reason) in &observed.refused {
        if reason.trim().is_empty() || !reason.contains(&hour_of(*ordinal).to_string()) {
            failures.push(format!(
                "hour {} is refused without naming itself",
                hour_of(*ordinal)
            ));
        }
    }
    match (&observed.total_proof, expected_confined.is_empty()) {
        (Ok(_), true) | (Err(_), false) => {}
        (Err(error), true) => failures.push(format!(
            "the clean control's total proof failed: {}",
            one_line(error)
        )),
        (Ok(_), false) => failures.push(
            "the total proof still claims every indexed hour reconstructs, over a cache that \
             cannot serve them all"
                .to_string(),
        ),
    }

    Row {
        cell: cell.to_string(),
        shape: shape.to_string(),
        verdict: if failures.is_empty() {
            "PASS".to_string()
        } else {
            format!("FAIL ({})", failures.join("; "))
        },
        served: format!("{}/{HOURS}", observed.served.len()),
        confined: if observed_confined.is_empty() {
            "none".to_string()
        } else {
            observed_confined
                .iter()
                .map(|o| hour_of(*o).to_string())
                .collect::<Vec<_>>()
                .join(",")
        },
        detail: observed
            .refused
            .values()
            .next()
            .map(|reason| one_line(reason))
            .unwrap_or_else(|| "no hour refused".to_string()),
    }
}

fn one_line(text: &str) -> String {
    let flat = text.replace(['\n', '\r'], " ").replace('|', "\\|");
    if flat.chars().count() <= 240 {
        return flat;
    }
    let head: String = flat.chars().take(240).collect();
    format!("{head}…")
}



fn truncated_cache(scratch: &Path, tag: &str, pair: &BuiltPair, keep_bytes: u64) -> PathBuf {
    let cell_dir = scratch.join(tag);
    if cell_dir.exists() {
        fs::remove_dir_all(&cell_dir).expect("clear cell dir");
    }
    fs::create_dir_all(&cell_dir).expect("create cell dir");
    let cache_path = cell_dir.join("checkpoints.h4cp");
    let cache = fs::read(&pair.cache_path).expect("read clean H4CP");
    let keep = usize::try_from(keep_bytes).expect("keep length fits usize");
    assert!(
        keep < cache.len(),
        "a truncation cell must actually remove bytes ({keep} of {})",
        cache.len()
    );
    fs::write(&cache_path, &cache[..keep]).expect("write truncated H4CP");
    cache_path
}

fn clean_cache_len(pair: &BuiltPair) -> u64 {
    fs::metadata(&pair.cache_path)
        .expect("stat clean H4CP")
        .len()
}

fn record_start(pair: &BuiltPair, position: usize) -> u64 {
    pair.records[position].1
}

fn inside_record(pair: &BuiltPair, position: usize) -> u64 {
    pair.records[position].1 + 1
}

fn confined_tail(position: usize) -> BTreeSet<usize> {
    (position..HOURS).collect()
}
