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
const HOURS: usize = 9;
const GAME_ID: &str = "gz_interior_holes";
const CHECKPOINT_INTERVAL: u32 = 168;

fn raw_for(hour_index: i64) -> i32 {
    DATE_EPOCH_CONST + hour_index as i32
}

fn hour_of(ordinal: usize) -> i64 {
    BASE_HOUR + ordinal as i64
}

struct StreamWriter {
    bytes: Vec<u8>,
    seq: u64,
    hour_starts: BTreeMap<usize, usize>,
}

impl StreamWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            seq: 0,
            hour_starts: BTreeMap::new(),
        }
    }

    fn push_with_seq(
        &mut self,
        hour: usize,
        seq: u64,
        game_hour_raw: i32,
        domain: u16,
        ty: u16,
        payload: &[u8],
    ) {
        self.hour_starts.entry(hour).or_insert(self.bytes.len());
        write_record(&mut self.bytes, seq, game_hour_raw, domain, ty, payload);
    }
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

fn frame_end_payload() -> Vec<u8> {
    vec![0u8; 32]
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

struct Synth {
    bytes: Vec<u8>,
    hour_starts: BTreeMap<usize, usize>,
}

fn synthesize(absent: &BTreeSet<usize>) -> Synth {
    let mut w = StreamWriter::new();
    w.push_with_seq(
        0,
        0,
        raw_for(BASE_HOUR),
        COMPACT_DOMAIN_STREAM_HEADER,
        COMPACT_RECORD_TYPE_STREAM_HEADER,
        &cid1_record_payload(),
    );
    w.seq = 1;
    for hour in 0..HOURS {
        let raw = raw_for(hour_of(hour));
        let seq_base = w.seq;
        w.seq = seq_base + 3;
        if absent.contains(&hour) {
            continue;
        }
        w.push_with_seq(
            hour,
            seq_base,
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
        w.push_with_seq(
            hour,
            seq_base + 1,
            raw,
            D13_TECH_LIVE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &tech,
        );
        w.push_with_seq(
            hour,
            seq_base + 2,
            raw,
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
    }
    Synth {
        bytes: w.bytes,
        hour_starts: w.hour_starts,
    }
}

impl Synth {
    fn start_of(&self, hour: usize) -> usize {
        *self
            .hour_starts
            .get(&hour)
            .unwrap_or_else(|| panic!("hour {hour} emitted no records"))
    }

    fn with_framing_damage_at(mut self, hour: usize) -> Vec<u8> {
        let offset = self.start_of(hour);
        self.bytes[offset] ^= 0xff;
        self.bytes
    }

    fn truncated_inside(&self, hour: usize) -> Vec<u8> {
        let offset = self.start_of(hour);
        self.bytes[..offset + COMPACT_RECORD_HEADER_LEN + 4].to_vec()
    }
}

struct Built {
    served: Vec<i64>,
    holes: Vec<(i64, String)>,
    coverage_verified: Vec<i32>,
    coverage_excluded: Vec<(i32, String)>,
    reconstruct: Result<u32, String>,
    refusals: BTreeMap<i64, String>,
    leaked: Vec<i64>,
    range_join: Vec<RangeJoin>,
}

struct RangeJoin {
    hour_index: i64,
    coverage_first_byte: u64,
    coverage_byte_len: u64,
    hole: Option<(u64, u64)>,
}

fn build(scratch: &Path, tag: &str, bytes: &[u8]) -> Result<Built, String> {
    let stream_path = scratch.join(format!("{tag}.h4cr"));
    fs::write(&stream_path, bytes).expect("write injected stream");
    let out_dir = scratch.join(format!("{tag}.artifacts"));
    if out_dir.exists() {
        fs::remove_dir_all(&out_dir).expect("clear artifact spill");
    }
    fs::create_dir_all(&out_dir).expect("create artifact spill");

    let source_sha256 = Sha256::hex(bytes);
    let artifacts = replay_decoder::build_certified_replay_artifacts(
        &stream_path,
        &out_dir,
        GAME_ID,
        StreamIdentityMode::IdentityV1,
        bytes.len() as u64,
        &source_sha256,
        CHECKPOINT_INTERVAL,
    )
    .map_err(|error| error.to_string())?;

    let index_bytes = fs::read(&artifacts.hour_index_path).expect("read packaged H4IX");
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes)
        .map_err(|error| format!("packaged H4IX does not parse: {error}"))?;
    let served: Vec<i64> = index.hours().iter().map(|row| row.hour_index()).collect();
    let holes = read_h4ix_holes(&index);

    let cache_file = fs::File::open(&artifacts.checkpoint_cache_path).expect("open packaged H4CP");
    let reconstruct = snapshot::CertifiedReplayCache::open_file(
        &index_bytes,
        cache_file,
        bytes.len() as u64,
        &source_sha256,
    )
    .and_then(|cache| cache.verify_every_hour())
    .map_err(|error| error.to_string());

    let coverage = &artifacts.hour_coverage;
    let coverage_verified: Vec<i32> = coverage.verified_hours().collect();
    let coverage_excluded: Vec<(i32, String)> = coverage
        .excluded_hours()
        .into_iter()
        .map(|(hour, reason)| (hour, reason.to_string()))
        .collect();

    let mut refusals = BTreeMap::new();
    let mut leaked = Vec::new();
    for (raw, reason) in &coverage_excluded {
        let hour_index = i64::from(*raw - DATE_EPOCH_CONST);
        if index.entry(hour_index).is_some() {
            leaked.push(hour_index);
            continue;
        }
        refusals.insert(
            hour_index,
            match holes.iter().find(|(hour, _)| *hour == hour_index) {
                Some((_, named)) => format!("H4IX hole: {named}"),
                None => format!("absent from H4IX; coverage map: {reason}"),
            },
        );
    }

    let range_join: Vec<RangeJoin> = coverage
        .rows()
        .filter(|row| !row.is_verified())
        .map(|row| {
            let hour_index = i64::from(row.game_hour_raw - DATE_EPOCH_CONST);
            RangeJoin {
                hour_index,
                coverage_first_byte: row.first_byte,
                coverage_byte_len: row.byte_len,
                hole: index
                    .excluded_hours()
                    .iter()
                    .find(|hole| hole.hour_index() == hour_index)
                    .map(|hole| (hole.h4cr_start_offset(), hole.h4cr_end_offset())),
            }
        })
        .collect();

    Ok(Built {
        served,
        holes,
        coverage_verified,
        coverage_excluded,
        reconstruct,
        refusals,
        leaked,
        range_join,
    })
}

fn read_h4ix_holes(index: &snapshot::ReplayHourIndex) -> Vec<(i64, String)> {
    index
        .excluded_hours()
        .iter()
        .map(|row| (row.hour_index(), row.reason().to_string()))
        .collect()
}

struct Row {
    cell: String,
    shape: String,
    injured: Vec<i64>,
    verdict: String,
    served: String,
    holes: String,
    detail: String,
    range_join: Vec<RangeJoin>,
}

fn judge(cell: &str, shape: &str, injured: &[usize], outcome: &Result<Built, String>) -> Row {
    let injured_hours: Vec<i64> = injured.iter().map(|o| hour_of(*o)).collect();
    let built = match outcome {
        Err(error) => {
            return Row {
                cell: cell.to_string(),
                shape: shape.to_string(),
                injured: injured_hours,
                verdict: "FAIL (whole build refused)".to_string(),
                served: "—".to_string(),
                holes: "—".to_string(),
                detail: one_line(error),
                range_join: Vec::new(),
            }
        }
        Ok(built) => built,
    };

    let verified: BTreeSet<i64> = built
        .coverage_verified
        .iter()
        .map(|raw| i64::from(*raw - DATE_EPOCH_CONST))
        .collect();
    let served: BTreeSet<i64> = built.served.iter().copied().collect();
    let lost: Vec<i64> = verified.difference(&served).copied().collect();
    let extra: Vec<i64> = served.difference(&verified).copied().collect();

    let mut failures = Vec::new();
    if !lost.is_empty() {
        failures.push(format!("verified hour(s) {lost:?} are not served"));
    }
    if !extra.is_empty() {
        failures.push(format!(
            "hour(s) {extra:?} served without verified coverage"
        ));
    }
    if !built.leaked.is_empty() {
        failures.push(format!(
            "excluded hour(s) {:?} are served out of the packaged index",
            built.leaked
        ));
    }
    if let Err(error) = &built.reconstruct {
        failures.push(format!("packaged H4CP does not reconstruct: {error}"));
    }
    for (hour, reason) in &built.refusals {
        if reason.trim().is_empty() {
            failures.push(format!("hour {hour} is refused with no reason"));
        }
    }
    for (hour, reason) in &built.holes {
        if reason.trim().is_empty() {
            failures.push(format!("H4IX hole for hour {hour} carries no reason"));
        }
    }

    let verdict = if failures.is_empty() {
        "PASS".to_string()
    } else {
        format!("FAIL ({})", failures.join("; "))
    };
    let detail = if built.coverage_excluded.is_empty() {
        "no hour excluded".to_string()
    } else {
        built
            .coverage_excluded
            .iter()
            .map(|(raw, reason)| {
                format!(
                    "hour {} — {}",
                    i64::from(*raw - DATE_EPOCH_CONST),
                    one_line(reason)
                )
            })
            .collect::<Vec<_>>()
            .join(" · ")
    };
    Row {
        cell: cell.to_string(),
        shape: shape.to_string(),
        injured: injured_hours,
        verdict,
        served: format!("{}/{HOURS}", built.served.len()),
        holes: if built.holes.is_empty() {
            "none".to_string()
        } else {
            built
                .holes
                .iter()
                .map(|(hour, _)| hour.to_string())
                .collect::<Vec<_>>()
                .join(",")
        },
        detail,
        range_join: built
            .range_join
            .iter()
            .map(|join| RangeJoin {
                hour_index: join.hour_index,
                coverage_first_byte: join.coverage_first_byte,
                coverage_byte_len: join.coverage_byte_len,
                hole: join.hole,
            })
            .collect(),
    }
}

fn one_line(text: &str) -> String {
    let flat = text.replace(['\n', '\r'], " ").replace('|', "\\|");
    if flat.chars().count() <= 220 {
        return flat;
    }
    let head: String = flat.chars().take(220).collect();
    format!("{head}…")
}

fn main() {
    let scratch = match std::env::args().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => {
            eprintln!("usage: gz_interior_holes <scratch-dir>");
            std::process::exit(2);
        }
    };
    fs::create_dir_all(&scratch).expect("create scratch dir");
    let scratch = fs::canonicalize(&scratch).expect("canonicalize scratch dir");

    let mut rows = Vec::new();

    let clean = synthesize(&BTreeSet::new());
    rows.push(judge(
        "(g0) clean control",
        "no injury",
        &[],
        &build(&scratch, "g0_clean", &clean.bytes),
    ));

    for (label, hour) in [("early", 1usize), ("middle", 4), ("late", 6)] {
        let synth = synthesize(&BTreeSet::from([hour]));
        rows.push(judge(
            &format!("(h1) interior hole, {label}"),
            &format!("hour {} absent (sequence hole)", hour_of(hour)),
            &[hour, hour + 1],
            &build(&scratch, &format!("h1_{label}"), &synth.bytes),
        ));
    }

    let synth = synthesize(&BTreeSet::from([2usize, 6]));
    rows.push(judge(
        "(h2) multi-hole",
        &format!("hours {} and {} absent", hour_of(2), hour_of(6)),
        &[2, 3, 6, 7],
        &build(&scratch, "h2_multi", &synth.bytes),
    ));

    let synth = synthesize(&BTreeSet::from([0usize]));
    rows.push(judge(
        "(h3) leading hole",
        &format!("hour {} absent (first hour)", hour_of(0)),
        &[0, 1],
        &build(&scratch, "h3_leading", &synth.bytes),
    ));

    let bytes = synthesize(&BTreeSet::new()).with_framing_damage_at(HOURS - 1);
    rows.push(judge(
        "(h4) trailing hole, delivered",
        &format!("framing damage in hour {}", hour_of(HOURS - 1)),
        &[HOURS - 1],
        &build(&scratch, "h4_trailing_delivered", &bytes),
    ));

    let torn = synthesize(&BTreeSet::new()).truncated_inside(HOURS - 1);
    rows.push(judge(
        "(h5) trailing hole, torn tail",
        &format!("file cut inside hour {}", hour_of(HOURS - 1)),
        &[HOURS - 1],
        &build(&scratch, "h5_torn", &torn),
    ));

    let bytes = synthesize(&BTreeSet::new()).with_framing_damage_at(4);
    rows.push(judge(
        "(h6) framing damage, middle",
        &format!("framing damage in hour {}", hour_of(4)),
        &[4, 5, 6, 7, 8],
        &build(&scratch, "h6_framing_middle", &bytes),
    ));

    print_matrix(&rows);
}

fn print_matrix(rows: &[Row]) {
    let passes = rows.iter().filter(|row| row.verdict == "PASS").count();
    println!("# Lane GZ — interior-hole matrix\n");
    println!("**{passes} PASS / {} FAIL**\n", rows.len() - passes);
    println!("| cell | injection | injured hours | verdict | served | H4IX holes |");
    println!("|---|---|---|---|---|---|");
    for row in rows {
        println!(
            "| {} | {} | {:?} | {} | {} | {} |",
            row.cell, row.shape, row.injured, row.verdict, row.served, row.holes
        );
    }
    println!("\n## Per-cell coverage detail\n");
    for row in rows {
        println!("* **{}** — {}", row.cell, row.detail);
    }
    println!(
        "\n## Coverage-map byte attribution vs the packaged H4IX hole\n\n\
         What a replay consumer holding only the two container-authenticated \
         documents can bind. `map` is the excluded hour's own byte attribution in \
         the sealed coverage map; `hole` is the range the packaged H4IX gives it.\n"
    );
    println!("| cell | hour | map [first, first+len) | H4IX hole [start, end) | join |");
    println!("|---|---:|---|---|---|");
    for row in rows {
        for join in &row.range_join {
            let map_range = format!(
                "[{}, {})",
                join.coverage_first_byte,
                join.coverage_first_byte + join.coverage_byte_len
            );
            let (hole_range, verdict) = match join.hole {
                None => (
                    "—".to_string(),
                    if join.coverage_byte_len == 0 {
                        "no bytes; named by the coverage map only"
                    } else {
                        "**BYTES BUT NO HOLE**"
                    }
                    .to_string(),
                ),
                Some((start, end)) => {
                    let map_end = join.coverage_first_byte + join.coverage_byte_len;
                    let contains = start <= join.coverage_first_byte && map_end <= end;
                    let exact = start == join.coverage_first_byte && end == map_end;
                    (
                        format!("[{start}, {end})"),
                        if exact {
                            "exact".to_string()
                        } else if contains {
                            "hole CONTAINS the map's range".to_string()
                        } else {
                            "**DISJOINT**".to_string()
                        },
                    )
                }
            };
            println!(
                "| {} | {} | {} | {} | {} |",
                row.cell, join.hour_index, map_range, hole_range, verdict
            );
        }
    }
}
