use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};
use store::{Listing, RangeId, SourceKey};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema: u32,
    source: Option<SourceKey>,
    rules: Vec<Rule>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    section: String,
    change: Change,
    ranges: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accounting: Option<[u64; 2]>,
    reason: String,
}

#[derive(Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Change {
    NewPrivate,
    RemovedPrivate,
    CheckpointBytes,
}

// Schema-40 wire types mirror restoration without importing dashboard dependencies.
#[derive(Deserialize, Serialize)]
struct Checkpoint {
    schema: u32,
    cursor: Cursor,
    selected: Vec<u16>,
    header: Vec<u8>,
    index: Vec<Block>,
    sessions: Vec<Vec<u8>>,
    branches: Vec<Vec<u8>>,
    gaps: Vec<Vec<u8>>,
    map: MapProgress,
    content: Option<bool>,
    display_error: Option<String>,
    display_failures: u64,
    refused: BTreeMap<u64, BTreeMap<String, Refusal>>,
    #[serde(default)]
    independent: BTreeMap<String, FamilyProgress>,
    #[serde(default)]
    production: BTreeMap<String, Coverage>,
    bytes: u64,
}

#[derive(Deserialize, Serialize)]
struct Cursor {
    next_block: usize,
    previous: Option<Block>,
}

#[derive(Deserialize, Serialize)]
struct Block {
    branch: u64,
    first: i32,
    last: i32,
    offset: u64,
    length: u64,
    raw_length: u64,
}

#[derive(Deserialize, Serialize)]
struct MapProgress {
    stride: u32,
    selected: Vec<u16>,
    branches: BTreeMap<u64, BranchProgress>,
    resume_range: Option<u64>,
    gaps: BTreeMap<u64, GapPage>,
    ranges: BTreeMap<u64, (i32, i32)>,
}

#[derive(Deserialize, Serialize)]
struct BranchProgress {
    checkpoints: u64,
    since_checkpoint: u32,
    last_hour: Option<i32>,
    last_block: Option<usize>,
}

#[derive(Deserialize, Serialize)]
struct GapPage {
    range: u64,
    intervals: u64,
}

#[derive(Deserialize, Serialize)]
struct Refusal {
    hour: i32,
    error: String,
    from: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct FamilyProgress {
    next_block: usize,
    #[serde(default)]
    refused: BTreeMap<u64, BTreeMap<String, Refusal>>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Coverage {
    schema: u32,
    first_hour: i32,
    last_hour: i32,
    complete: bool,
}

struct Input {
    manifest: PathBuf,
    directory: PathBuf,
    listing: Listing,
    sections: BTreeMap<String, (usize, u64)>,
    source: SourceKey,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn manifest(path: &Path) -> io::Result<PathBuf> {
    if path.is_file() {
        return Ok(path.to_owned());
    }
    if path.join("manifest").is_file() {
        return Ok(path.join("manifest"));
    }
    let root = if path.join("HOI4Replay/prepared").is_dir() {
        path.join("HOI4Replay/prepared")
    } else {
        path.to_owned()
    };
    let mut found = Vec::new();
    for entry in fs::read_dir(&root)? {
        let candidate = entry?.path().join("manifest");
        if candidate.is_file() {
            found.push(candidate);
        }
    }
    if found.len() != 1 {
        return Err(invalid(format!(
            "{} contains {} stores; supply one store path",
            root.display(),
            found.len()
        )));
    }
    Ok(found.remove(0))
}

impl Input {
    fn open(path: &Path, directory: Option<&PathBuf>) -> io::Result<Self> {
        let manifest = fs::canonicalize(manifest(path)?)?;
        let directory = fs::canonicalize(
            directory
                .map(PathBuf::as_path)
                .unwrap_or(manifest.parent().unwrap()),
        )?;
        let listing =
            Listing::open(&manifest)?.ok_or_else(|| invalid("Missing or unsupported manifest"))?;
        let source = listing.source_key(&directory)?;
        if source != source.recapture()? {
            return Err(invalid("Store source/content stamps are no longer current"));
        }
        let mut sections = BTreeMap::new();
        for (index, (name, count)) in listing.sections().enumerate() {
            if sections.insert(name.to_owned(), (index, count)).is_some() || name.is_empty() {
                return Err(invalid("Empty or duplicate manifest section"));
            }
        }
        Ok(Self {
            manifest,
            directory,
            listing,
            sections,
            source,
        })
    }

    fn single_range(&self, name: &str) -> io::Result<RangeId> {
        let &(section, count) = self
            .sections
            .get(name)
            .ok_or_else(|| invalid(format!("Missing completion section {name}")))?;
        if count != 1 {
            return Err(invalid(format!("Invalid completion range count: {name}")));
        }
        self.listing
            .range(section, 0)?
            .ok_or_else(|| invalid(format!("Completion section is a hole: {name}")))
    }

    fn json(&self, name: &str) -> io::Result<Value> {
        Ok(serde_json::from_reader(io::BufReader::new(
            self.single_range(name)?.open(&self.directory)?,
        ))?)
    }

    fn checkpoint(&self) -> io::Result<Value> {
        let mut packet = Vec::new();
        self.single_range("preparation-checkpoint-v1")?
            .open(&self.directory)?
            .read_to_end(&mut packet)?;
        let state: Checkpoint = serde_json::from_slice(&packet)?;
        if state.schema != 40 {
            return Err(invalid(format!(
                "Unsupported completion schema {}; expected 40",
                state.schema
            )));
        }
        state
            .bytes
            .checked_add(packet.len() as u64)
            .ok_or_else(|| invalid("prepared byte count overflow"))?;
        let mut active = 0u64;
        for (name, &(section, count)) in &self.sections {
            if name == "preparation-checkpoint-v1" {
                continue;
            }
            for ordinal in 0..count {
                let range = self
                    .listing
                    .range(section, ordinal)?
                    .ok_or_else(|| invalid("Checkpoint accounting includes a missing payload"))?;
                active = active
                    .checked_add(range.len())
                    .ok_or_else(|| invalid("prepared byte count overflow"))?;
            }
        }
        if state.bytes != active {
            return Err(invalid(format!(
                "Invalid checkpoint active-byte accounting: claimed {}, stored {active}",
                state.bytes
            )));
        }
        Ok(serde_json::from_slice(&packet)?)
    }

    fn completion(&self) -> io::Result<Value> {
        let checkpoint = self.checkpoint()?;
        let blocks = checkpoint["index"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("Empty/missing completed block index"))?
            .len() as u64;
        if checkpoint["cursor"]["next_block"].as_u64() != Some(blocks) {
            return Err(invalid("The forward walk is incomplete"));
        }
        let families = checkpoint["independent"]
            .as_object()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("Missing family completion inventory"))?;
        for (name, progress) in families {
            if progress["next_block"].as_u64() != Some(blocks) {
                return Err(invalid(format!("Family {name} is incomplete")));
            }
        }
        let side = self.json("preparation-side-work-v1")?;
        if !side["pending"].as_array().is_some_and(Vec::is_empty) {
            return Err(invalid("Side preparation is incomplete"));
        }
        let production = checkpoint["production"]
            .as_object()
            .ok_or_else(|| invalid("Missing production completion inventory"))?;
        let mut last = BTreeMap::new();
        for block in checkpoint["index"].as_array().unwrap() {
            let branch = block["branch"]
                .as_u64()
                .ok_or_else(|| invalid("Invalid completed block branch"))?;
            last.insert(branch, block);
        }
        if production.len() != last.len() {
            return Err(invalid(
                "Production completion inventory differs from recorded branches",
            ));
        }
        for (&branch, block) in &last {
            let coverage = production
                .get(&branch.to_string())
                .ok_or_else(|| invalid(format!("Missing production coverage: {branch}")))?;
            if coverage["schema"] != 40
                || coverage["complete"] != true
                || coverage["lastHour"] != block["last"]
            {
                return Err(invalid(format!("Production scan is incomplete: {branch}")));
            }
            let name = format!("production-lines-directory-v1-{branch}");
            let directory: Value = serde_json::from_reader(zstd::stream::Decoder::new(
                self.single_range(&name)?.open(&self.directory)?,
            )?)?;
            if directory["branch"].as_u64() != Some(branch)
                || directory["coverage"]["complete"] != true
                || directory["coverage"]["lastHour"] != block["last"]
                || directory["last_block"]
                    != json!([block["first"], block["last"], block["offset"]])
            {
                return Err(invalid(format!(
                    "Production directory is incomplete: {branch}"
                )));
            }
            if directory["coverage"] != *coverage {
                return Err(invalid(format!(
                    "Production checkpoint coverage differs from its directory: {branch}"
                )));
            }
            let published = self.json(&format!("preparation-family-production-lines-{branch}"))?;
            if published != directory["coverage"] {
                return Err(invalid(format!(
                    "Production coverage differs from its directory: {branch}"
                )));
            }
        }
        Ok(json!({"blocks": blocks, "families": families, "side": side,
            "header": checkpoint["header"], "content": checkpoint["content"],
            "index": checkpoint["index"], "sessions": checkpoint["sessions"],
            "branches": checkpoint["branches"], "gaps": checkpoint["gaps"], "production": checkpoint["production"]}))
    }
}

#[derive(Serialize, PartialEq, Debug)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Entry {
    Absent,
    Hole,
    Payload { bytes: u64 },
}

fn entry(
    input: &Input,
    section: Option<(usize, u64)>,
    ordinal: u64,
) -> io::Result<(Entry, Option<RangeId>)> {
    let Some((index, count)) = section.filter(|(_, count)| ordinal < *count) else {
        return Ok((Entry::Absent, None));
    };
    let _ = count;
    Ok(match input.listing.range(index, ordinal)? {
        Some(id) => (Entry::Payload { bytes: id.len() }, Some(id)),
        None => (Entry::Hole, None),
    })
}

fn compare_bytes(
    left: Option<store::Range>,
    right: Option<store::Range>,
) -> io::Result<(u64, Option<u64>, String, String)> {
    let left_len = left.as_ref().map_or(0, store::Range::len);
    let right_len = right.as_ref().map_or(0, store::Range::len);
    let mut left = left.map(io::BufReader::new);
    let mut right = right.map(io::BufReader::new);
    let (mut a, mut b) = ([0u8; 65536], [0u8; 65536]);
    let mut at = 0;
    let mut first = None;
    let (mut left_hex, mut right_hex) = (String::new(), String::new());
    while at < left_len.max(right_len) {
        let na = (left_len.saturating_sub(at)).min(a.len() as u64) as usize;
        let nb = (right_len.saturating_sub(at)).min(b.len() as u64) as usize;
        if na > 0 {
            left.as_mut().unwrap().read_exact(&mut a[..na])?;
        }
        if nb > 0 {
            right.as_mut().unwrap().read_exact(&mut b[..nb])?;
        }
        if first.is_none() {
            let mismatch = a[..na.min(nb)]
                .iter()
                .zip(&b[..na.min(nb)])
                .position(|(a, b)| a != b)
                .or_else(|| (na != nb).then_some(na.min(nb)));
            if let Some(offset) = mismatch {
                first = Some(at + offset as u64);
                left_hex = a[offset.min(na)..na.min(offset + 16)]
                    .iter()
                    .map(|v| format!("{v:02x}"))
                    .collect();
                right_hex = b[offset.min(nb)..nb.min(offset + 16)]
                    .iter()
                    .map(|v| format!("{v:02x}"))
                    .collect();
            }
        }
        at += na.max(nb) as u64;
    }
    Ok((left_len.min(right_len), first, left_hex, right_hex))
}

fn private_section(name: &str) -> bool {
    // These identities are continuation-only; unknown sections remain public contracts.
    if name == "map.resume" {
        return true;
    }
    let Some(suffix) = name.strip_prefix("map-pages.state.") else {
        return false;
    };
    let Some((branch, stride)) = suffix.split_once('.') else {
        return false;
    };
    branch
        .parse::<u64>()
        .is_ok_and(|value| value.to_string() == branch)
        && stride
            .parse::<u32>()
            .is_ok_and(|value| value != 0 && value.to_string() == stride)
}

fn compare(left: &Input, right: &Input, policy: &Policy) -> io::Result<Value> {
    if left.source != right.source {
        return Err(invalid(
            "Stores bind different recordings or ordered content stacks",
        ));
    }
    if policy.schema != 1
        || (!policy.rules.is_empty() && policy.source.as_ref() != Some(&left.source))
        || policy
            .source
            .as_ref()
            .is_some_and(|source| source != &left.source)
    {
        return Err(invalid(
            "Unsupported policy or policy source binding does not match both stores",
        ));
    }
    let left_complete = left.completion()?;
    let right_complete = right.completion()?;
    for field in [
        "blocks", "header", "content", "index", "sessions", "branches", "gaps",
    ] {
        if left_complete[field] != right_complete[field] {
            return Err(invalid(format!(
                "Different recording/content completion binding: {field}"
            )));
        }
    }
    let mut rules = BTreeMap::new();
    for (index, rule) in policy.rules.iter().enumerate() {
        if match rule.change {
            Change::CheckpointBytes => {
                rule.section != "preparation-checkpoint-v1"
                    || rule.ranges != 1
                    || !rule.accounting.is_some_and(|[left, right]| left != right)
            }
            _ => {
                if !private_section(&rule.section) {
                    return Err(invalid(format!(
                        "Structural exception is not a known private continuation: {}",
                        rule.section
                    )));
                }
                rule.accounting.is_some()
            }
        } {
            return Err(invalid(
                "Checkpoint accounting requires the exact v1 section, one range and differing left/right byte counts; other rules cannot normalize payloads",
            ));
        }
        if rule.section.is_empty()
            || rule.section.contains(['*', '?', '[', ']'])
            || rule.reason.trim().is_empty()
            || rules.insert(rule.section.as_str(), (index, rule)).is_some()
        {
            return Err(invalid(
                "Policy rules require unique exact section names and a reason; patterns are forbidden",
            ));
        }
    }
    let names: BTreeSet<_> = left.sections.keys().chain(right.sections.keys()).collect();
    let (mut used, mut sections, mut differences, mut compared_bytes) =
        (BTreeSet::new(), Vec::new(), Vec::new(), 0u64);
    for name in names {
        let l = left.sections.get(name).copied();
        let r = right.sections.get(name).copied();
        let allowed = if let Some((index, rule)) = rules.get(name.as_str()) {
            let matches = match (l, r, &rule.change) {
                (None, Some((_, count)), Change::NewPrivate)
                | (Some((_, count)), None, Change::RemovedPrivate) => {
                    private_section(name) && count == rule.ranges
                }
                (Some((_, 1)), Some((_, 1)), Change::CheckpointBytes) => {
                    let mut a = left.checkpoint()?;
                    let b = right.checkpoint()?;
                    let [left_bytes, right_bytes] = rule.accounting.unwrap();
                    let counts_match = a["bytes"].as_u64() == Some(left_bytes)
                        && b["bytes"].as_u64() == Some(right_bytes);
                    a["bytes"] = b["bytes"].clone();
                    counts_match && a == b
                }
                _ => false,
            };
            matches.then(|| {
                used.insert(*index);
                *index
            })
        } else {
            None
        };
        if l.map(|(_, c)| c) != r.map(|(_, c)| c) {
            differences.push(json!({"section": name, "range": null, "kind": if l.is_none() {"extra_section"} else if r.is_none() {"missing_section"} else {"range_count"}, "left_count": l.map(|(_,c)|c), "right_count": r.map(|(_,c)|c), "rule": allowed}));
        }
        let mut ranges = Vec::new();
        for ordinal in 0..l.map_or(0, |(_, c)| c).max(r.map_or(0, |(_, c)| c)) {
            let (ls, li) = entry(left, l, ordinal)?;
            let (rs, ri) = entry(right, r, ordinal)?;
            let (bytes, offset, a, b) = compare_bytes(
                li.map(|id| id.open(&left.directory)).transpose()?,
                ri.map(|id| id.open(&right.directory)).transpose()?,
            )?;
            compared_bytes += bytes;
            if ls != rs || offset.is_some() {
                differences.push(json!({"section": name, "range": ordinal, "kind": if ls != rs {"range_state_or_length"} else {"payload"}, "left": ls, "right": rs, "first_differing_byte": offset, "left_hex": a, "right_hex": b, "rule": allowed}));
            }
            ranges.push(json!({"ordinal": ordinal, "left": ls, "right": rs}));
        }
        sections.push(json!({"section": name, "left_count": l.map(|(_,c)|c), "right_count": r.map(|(_,c)|c), "ranges": ranges}));
    }
    let unused: Vec<_> = (0..policy.rules.len())
        .filter(|index| !used.contains(index))
        .collect();
    if left.source != left.source.recapture()? || right.source != right.source.recapture()? {
        return Err(invalid("Source/content changed during comparison"));
    }
    let unexpected = differences.iter().filter(|d| d["rule"].is_null()).count();
    Ok(
        json!({"schema":1, "passed": unexpected == 0 && unused.is_empty(), "left":left.manifest, "right":right.manifest,
        "source":left.source, "left_completion":left_complete, "right_completion":right_complete,
        "compared_bytes":compared_bytes, "sections":sections, "differences":differences,
        "unexpected_differences":unexpected, "rules":policy.rules, "unused_rules":unused,
        "binding_kind":"Exact stored ordered source stamps, revalidated against the inputs; not historical content hashes"}),
    )
}

fn run() -> io::Result<bool> {
    let mut args = env::args_os().skip(1);
    let mut options = BTreeMap::new();
    while let Some(key) = args.next() {
        let key = key
            .to_str()
            .ok_or_else(|| invalid("Invalid option"))?
            .to_owned();
        if ![
            "--left",
            "--right",
            "--left-store",
            "--right-store",
            "--differences",
            "--report",
        ]
        .contains(&key.as_str())
            || options.contains_key(&key)
        {
            return Err(invalid(
                "Usage: store-compare --left <store/LOCALAPPDATA/manifest> --right <store/LOCALAPPDATA/manifest> --differences <policy.json> --report <report.json> [--left-store <packs>] [--right-store <packs>]",
            ));
        }
        options.insert(
            key,
            PathBuf::from(args.next().ok_or_else(|| invalid("Missing option value"))?),
        );
    }
    let required = |name: &str| {
        options
            .get(name)
            .ok_or_else(|| invalid(format!("Missing {name}")))
    };
    let report_path = required("--report")?;
    let outcome = (|| {
        let left = Input::open(required("--left")?, options.get("--left-store"))?;
        let right = Input::open(required("--right")?, options.get("--right-store"))?;
        let policy = serde_json::from_reader(File::open(required("--differences")?)?)?;
        compare(&left, &right, &policy)
    })();
    let report = match outcome {
        Ok(report) => report,
        Err(error) => json!({"schema":1, "passed":false, "error":error.to_string()}),
    };
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(report_path)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    writeln!(file)?;
    if let Some(error) = report["error"].as_str() {
        eprintln!("REFUSED: {error}");
    }
    if let Some(differences) = report["differences"].as_array() {
        for d in differences {
            println!(
                "{} range={} {} first_byte={} rule={}",
                d["section"].as_str().unwrap(),
                d["range"],
                d["kind"].as_str().unwrap(),
                d["first_differing_byte"],
                d["rule"]
            );
        }
    }
    println!(
        "{}: unexpected={}, unused_rules={}, report={}",
        if report["passed"] == true {
            "PASS"
        } else {
            "FAIL"
        },
        report["unexpected_differences"],
        report["unused_rules"],
        report_path.display()
    );
    Ok(report["passed"] == true)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("store-compare: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    const SIDE: &[u8] = br#"{"pending":[]}"#;
    const PRODUCTION_COVERAGE: &[u8] =
        br#"{"schema":40,"firstHour":1,"lastHour":2,"complete":true}"#;
    fn production_document() -> Value {
        json!({"branch":1,
            "coverage":serde_json::from_slice::<Value>(PRODUCTION_COVERAGE).unwrap(),
            "last_block":[1,2,0]})
    }
    fn compressed(document: &Value) -> Vec<u8> {
        zstd::stream::encode_all(serde_json::to_vec(document).unwrap().as_slice(), 1).unwrap()
    }
    fn production_directory() -> Vec<u8> {
        compressed(&production_document())
    }
    fn base_bytes() -> u64 {
        (SIDE.len() + PRODUCTION_COVERAGE.len() + production_directory().len()) as u64
    }

    struct Fixture {
        root: PathBuf,
        source: SourceKey,
    }

    impl Fixture {
        fn new() -> Self {
            let root = env::temp_dir().join(format!("store-compare-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            fs::write(root.join("recording"), b"synthetic recording identity").unwrap();
            fs::write(root.join("content"), b"synthetic content identity").unwrap();
            let source =
                SourceKey::capture(&root.join("recording"), &[root.join("content")]).unwrap();
            Self { root, source }
        }

        fn store(
            &self,
            name: &str,
            products: Vec<(&str, Vec<Vec<u8>>)>,
            pack: u64,
            complete: bool,
        ) -> Input {
            self.store_with_checkpoint(name, products, pack, complete, |packet| packet)
        }

        fn store_with_checkpoint(
            &self,
            name: &str,
            mut products: Vec<(&str, Vec<Vec<u8>>)>,
            pack: u64,
            complete: bool,
            update: impl FnOnce(Vec<u8>) -> Vec<u8>,
        ) -> Input {
            for (section, ranges) in [
                ("preparation-side-work-v1", vec![SIDE.to_vec()]),
                (
                    "production-lines-directory-v1-1",
                    vec![production_directory()],
                ),
                (
                    "preparation-family-production-lines-1",
                    vec![PRODUCTION_COVERAGE.to_vec()],
                ),
            ] {
                if !products.iter().any(|(name, _)| *name == section) {
                    products.push((section, ranges));
                }
            }
            let bytes: usize = products
                .iter()
                .flat_map(|(_, ranges)| ranges)
                .map(Vec::len)
                .sum();
            let checkpoint = serde_json::to_vec(&json!({"schema":40,
                "index":[{"branch":1,"first":1,"last":2,"offset":0,"length":8,"raw_length":16}],
                "cursor":{"next_block":if complete {1}else{0},"previous":null},
                "independent":{"synthetic":{"next_block":1,"refused":{}}},"production":{"1":{"schema":40,"firstHour":1,"lastHour":2,"complete":true}},"selected":[1,2],
                "header":[1,2,3],"content":false,"sessions":[],"branches":[],"gaps":[],
                "map":{"stride":720,"selected":[1,2],"branches":{},"resume_range":null,"gaps":{},"ranges":{}},
                "display_error":null,"display_failures":0,"refused":{},"bytes":bytes})).unwrap();
            products.push(("preparation-checkpoint-v1", vec![update(checkpoint)]));
            let directory = self.root.join(name);
            fs::create_dir(&directory).unwrap();
            let source = uuid::Uuid::new_v4();
            fs::write(
                directory.join(format!("source_{}.json", source.simple())),
                serde_json::to_vec(&self.source).unwrap(),
            )
            .unwrap();
            let header = serde_json::to_vec(&json!({"version":4,"flat":true,"source":source,
                "sections":products.iter().map(|(name,ranges)|json!({"name":name,"count":ranges.len()})).collect::<Vec<_>>()})).unwrap();
            let mut manifest = File::create(directory.join("manifest")).unwrap();
            manifest
                .write_all(&(header.len() as u64).to_le_bytes())
                .unwrap();
            manifest.write_all(&header).unwrap();
            let mut payload = vec![99; pack as usize];
            for (_, ranges) in products {
                for bytes in ranges {
                    for value in [pack, payload.len() as u64, bytes.len() as u64] {
                        manifest.write_all(&value.to_le_bytes()).unwrap();
                    }
                    payload.extend(bytes);
                }
            }
            fs::write(directory.join(format!("pack_{pack:016x}.bin")), payload).unwrap();
            Input::open(&directory, None).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    fn strict() -> Policy {
        Policy {
            schema: 1,
            source: None,
            rules: Vec::new(),
        }
    }

    #[test]
    fn pending_only_side_progress_still_requires_real_production_completion() {
        let fixture = Fixture::new();
        let current = fixture.store("pending-only-side", vec![], 1, true);
        assert_eq!(
            compare(&current, &current, &strict()).unwrap()["passed"],
            true
        );
        let extra = fixture.store(
            "extra-side-field",
            vec![(
                "preparation-side-work-v1",
                vec![br#"{"pending":[],"production":[]}"#.to_vec()],
            )],
            2,
            true,
        );
        extra.completion().unwrap();
        assert_eq!(
            compare(&current, &extra, &strict()).unwrap()["passed"],
            false
        );
        for (index, pending) in [Value::Null, json!(false), json!({}), json!(["waiting"])]
            .into_iter()
            .enumerate()
        {
            let input = fixture.store(
                &format!("invalid-pending-{index}"),
                vec![(
                    "preparation-side-work-v1",
                    vec![serde_json::to_vec(&json!({"pending":pending})).unwrap()],
                )],
                3,
                true,
            );
            assert!(
                input
                    .completion()
                    .unwrap_err()
                    .to_string()
                    .contains("Side preparation")
            );
        }
    }

    #[test]
    fn production_completion_binds_every_branch_to_its_checkpoint_coverage() {
        let fixture = Fixture::new();
        for (index, production) in [
            json!({}),
            json!({"2":{"schema":40,"firstHour":1,"lastHour":2,"complete":true}}),
            json!({"1":{"schema":40,"firstHour":1,"lastHour":1,"complete":true}}),
            json!({"1":{"schema":40,"firstHour":1,"lastHour":2,"complete":false}}),
            json!({"1":{"schema":40,"firstHour":0,"lastHour":2,"complete":true}}),
            json!({"1":{"schema":41,"firstHour":1,"lastHour":2,"complete":true}}),
        ]
        .into_iter()
        .enumerate()
        {
            let input = fixture.store_with_checkpoint(
                &format!("invalid-production-coverage-{index}"),
                vec![],
                1,
                true,
                |packet| edit_checkpoint(packet, |document| document["production"] = production),
            );
            let expected = match index {
                0 => "Production completion inventory differs from recorded branches",
                1 => "Missing production coverage: 1",
                2 | 3 | 5 => "Production scan is incomplete: 1",
                4 => "Production checkpoint coverage differs from its directory: 1",
                _ => unreachable!(),
            };
            assert_eq!(input.completion().unwrap_err().to_string(), expected);
        }
    }

    #[test]
    fn production_directories_bind_the_final_block_and_published_coverage() {
        let fixture = Fixture::new();
        for (index, (pointer, value)) in [
            ("/branch", json!(2)),
            ("/last_block", Value::Null),
            ("/last_block", json!([1, 1, 0])),
            ("/last_block", json!([1, 2, 1])),
            ("/coverage/complete", json!(false)),
            ("/coverage/lastHour", json!(1)),
            ("/coverage/firstHour", json!(0)),
        ]
        .into_iter()
        .enumerate()
        {
            let mut directory = production_document();
            *directory.pointer_mut(pointer).unwrap() = value;
            let input = fixture.store(
                &format!("invalid-production-directory-{index}"),
                vec![(
                    "production-lines-directory-v1-1",
                    vec![compressed(&directory)],
                )],
                1,
                true,
            );
            assert!(
                input
                    .completion()
                    .unwrap_err()
                    .to_string()
                    .contains("Production")
            );
        }
        let input = fixture.store(
            "invalid-published-production-coverage",
            vec![(
                "preparation-family-production-lines-1",
                vec![br#"{"schema":40,"firstHour":1,"lastHour":2,"complete":false}"#.to_vec()],
            )],
            1,
            true,
        );
        assert!(
            input
                .completion()
                .unwrap_err()
                .to_string()
                .contains("Production coverage differs")
        );
        for (index, ranges) in [vec![], vec![production_directory(), production_directory()]]
            .into_iter()
            .enumerate()
        {
            let input = fixture.store(
                &format!("invalid-production-ranges-{index}"),
                vec![("production-lines-directory-v1-1", ranges)],
                1,
                true,
            );
            assert!(
                input
                    .completion()
                    .unwrap_err()
                    .to_string()
                    .contains("range count")
            );
        }
        let original = fixture.store("public-production-directory-original", vec![], 1, true);
        let mut directory = production_document();
        directory["visual"] = json!("changed public product");
        let changed = fixture.store(
            "public-production-directory-changed",
            vec![(
                "production-lines-directory-v1-1",
                vec![compressed(&directory)],
            )],
            2,
            true,
        );
        assert_eq!(
            compare(&original, &changed, &strict()).unwrap()["passed"],
            false
        );
    }

    #[test]
    fn completion_rejects_obsolete_or_unknown_schema() {
        let fixture = Fixture::new();
        for schema in [37, 38, 39, 41] {
            let input = fixture.store_with_checkpoint(
                &format!("unsupported-schema-{schema}"),
                vec![],
                1,
                true,
                |packet| edit_checkpoint(packet, |document| document["schema"] = json!(schema)),
            );
            assert!(
                input
                    .checkpoint()
                    .unwrap_err()
                    .to_string()
                    .contains(&schema.to_string())
            );
        }
    }

    #[test]
    fn logical_payloads_ignore_physical_layout_and_manifest_order() {
        let fixture = Fixture::new();
        let left = fixture.store(
            "left",
            vec![("a", vec![b"abc".to_vec(), vec![]]), ("b", vec![])],
            1,
            true,
        );
        let right = fixture.store(
            "right",
            vec![("b", vec![]), ("a", vec![b"abc".to_vec(), vec![]])],
            7,
            true,
        );
        let report = compare(&left, &right, &strict()).unwrap();
        assert_eq!(report["passed"], true);
        assert!(
            report["sections"]
                .as_array()
                .unwrap()
                .iter()
                .any(|section| section["section"] == "b" && section["left_count"] == 0)
        );
    }

    #[test]
    fn compares_past_buffer_boundaries_and_distinguishes_missing_empty_ranges() {
        let fixture = Fixture::new();
        let payload = vec![5; 65540];
        let mut changed = payload.clone();
        changed[65537] = 9;
        let left = fixture.store(
            "left",
            vec![("public", vec![payload, vec![]]), ("empty-section", vec![])],
            1,
            true,
        );
        let right = fixture.store("right", vec![("public", vec![changed])], 2, true);
        let report = compare(&left, &right, &strict()).unwrap();
        assert_eq!(report["passed"], false);
        let differences = report["differences"].as_array().unwrap();
        assert!(differences.iter().any(|d| d["section"] == "public"
            && d["range"] == 0
            && d["first_differing_byte"] == 65537));
        assert!(differences.iter().any(|d| d["section"] == "public"
            && d["range"] == 1
            && d["right"]["state"] == "absent"));
        assert!(
            differences
                .iter()
                .any(|d| d["section"] == "empty-section" && d["kind"] == "missing_section")
        );
    }

    #[test]
    fn exceptions_are_exact_source_bound_consumed_structural_rules() {
        let fixture = Fixture::new();
        let left = fixture.store("left", vec![("map.resume", vec![vec![1]])], 1, true);
        let right = fixture.store("right", vec![], 2, true);
        let mut policy = Policy {
            schema: 1,
            source: Some(fixture.source.clone()),
            rules: vec![Rule {
                section: "map.resume".into(),
                change: Change::RemovedPrivate,
                ranges: 1,
                accounting: None,
                reason: "Replaced private continuation".into(),
            }],
        };
        assert_eq!(compare(&left, &right, &policy).unwrap()["passed"], false);
        policy.rules.push(Rule {
            section: "preparation-checkpoint-v1".into(),
            change: Change::CheckpointBytes,
            ranges: 1,
            accounting: Some([base_bytes() + 1, base_bytes()]),
            reason: "Removed private continuation changes only stored-byte accounting".into(),
        });
        assert_eq!(compare(&left, &right, &policy).unwrap()["passed"], true);
        policy.rules[1].accounting = Some([base_bytes() + 2, base_bytes()]);
        assert_eq!(compare(&left, &right, &policy).unwrap()["passed"], false);
        policy.rules[1].accounting = Some([base_bytes() + 1, base_bytes()]);
        assert_eq!(compare(&left, &left, &policy).unwrap()["passed"], false);
        policy.rules[0].section = "private-*".into();
        assert!(compare(&left, &right, &policy).is_err());
        policy.rules[0].section = "map.resume".into();
        policy.source = None;
        assert!(compare(&left, &right, &policy).is_err());
    }

    fn checkpoint_policy(fixture: &Fixture, accounting: [u64; 2]) -> Policy {
        Policy {
            schema: 1,
            source: Some(fixture.source.clone()),
            rules: vec![Rule {
                section: "preparation-checkpoint-v1".into(),
                change: Change::CheckpointBytes,
                ranges: 1,
                accounting: Some(accounting),
                reason: "Exact checkpoint accounting change".into(),
            }],
        }
    }

    fn remove_private_policy(fixture: &Fixture) -> Policy {
        let mut policy = checkpoint_policy(fixture, [base_bytes() + 1, base_bytes()]);
        policy.rules.push(Rule {
            section: "map.resume".into(),
            change: Change::RemovedPrivate,
            ranges: 1,
            accounting: None,
            reason: "Removed private continuation".into(),
        });
        policy
    }

    fn edit_checkpoint(packet: Vec<u8>, update: impl FnOnce(&mut Value)) -> Vec<u8> {
        let mut document = serde_json::from_slice(&packet).unwrap();
        update(&mut document);
        serde_json::to_vec(&document).unwrap()
    }

    #[test]
    fn exact_source_bound_accounting_rules_refuse_overflow_on_either_side() {
        let fixture = Fixture::new();
        let left = fixture.store("left", vec![], 1, true);
        for (index, bytes) in [u64::MAX, u64::MAX - 1].into_iter().enumerate() {
            let right = fixture.store_with_checkpoint(
                &format!("overflow-{index}"),
                vec![],
                2,
                true,
                |packet| edit_checkpoint(packet, |document| document["bytes"] = json!(bytes)),
            );
            for (a, b, accounting) in [
                (&left, &right, [base_bytes(), bytes]),
                (&right, &left, [bytes, base_bytes()]),
            ] {
                let error = compare(a, b, &checkpoint_policy(&fixture, accounting)).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                assert!(error.to_string().contains("prepared byte count overflow"));
            }
        }
    }

    #[test]
    fn exact_source_bound_accounting_rules_require_the_manifest_payload_total() {
        let fixture = Fixture::new();
        let left = fixture.store("left", vec![], 1, true);
        for (index, bytes) in [0, base_bytes() - 1, base_bytes() + 1]
            .into_iter()
            .enumerate()
        {
            let right = fixture.store_with_checkpoint(
                &format!("invalid-{index}"),
                vec![],
                2,
                true,
                |packet| edit_checkpoint(packet, |document| document["bytes"] = json!(bytes)),
            );
            for (a, b, accounting) in [
                (&left, &right, [base_bytes(), bytes]),
                (&right, &left, [bytes, base_bytes()]),
            ] {
                let error = compare(a, b, &checkpoint_policy(&fixture, accounting)).unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("Invalid checkpoint active-byte accounting")
                );
            }
        }
        let mut boundary_bytes = 0;
        let boundary = fixture.store_with_checkpoint("boundary", vec![], 3, true, |packet| {
            let packet = edit_checkpoint(packet, |document| document["bytes"] = json!(u64::MAX));
            boundary_bytes = u64::MAX - packet.len() as u64;
            let length = packet.len();
            let packet =
                edit_checkpoint(packet, |document| document["bytes"] = json!(boundary_bytes));
            assert_eq!(packet.len(), length);
            assert_eq!(
                boundary_bytes.checked_add(packet.len() as u64),
                Some(u64::MAX)
            );
            packet
        });
        let error = compare(
            &left,
            &boundary,
            &checkpoint_policy(&fixture, [base_bytes(), boundary_bytes]),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Invalid checkpoint active-byte accounting")
        );
    }

    #[test]
    fn valid_private_accounting_changes_cannot_hide_duplicate_checkpoint_fields() {
        let fixture = Fixture::new();
        let left = fixture.store("left", vec![("map.resume", vec![vec![1]])], 1, true);
        let bytes = base_bytes();
        for (index, (field, replacement)) in [
            (
                "\"schema\":40".to_owned(),
                "\"schema\":0,\"schema\":40".to_owned(),
            ),
            (
                format!("\"bytes\":{bytes}"),
                format!("\"bytes\":0,\"bytes\":{bytes}"),
            ),
            (
                "\"next_block\":1".to_owned(),
                "\"next_block\":0,\"next_block\":1".to_owned(),
            ),
            (
                "\"stride\":720".to_owned(),
                "\"stride\":0,\"stride\":720".to_owned(),
            ),
            (
                "\"length\":8".to_owned(),
                "\"length\":0,\"length\":8".to_owned(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let right = fixture.store_with_checkpoint(
                &format!("duplicate-{index}"),
                vec![],
                2,
                true,
                |packet| {
                    let packet = String::from_utf8(packet).unwrap();
                    assert!(packet.contains(&field));
                    packet.replacen(&field, &replacement, 1).into_bytes()
                },
            );
            let mut policy = remove_private_policy(&fixture);
            assert!(
                compare(&left, &right, &policy)
                    .unwrap_err()
                    .to_string()
                    .contains("duplicate field")
            );
            policy.rules[0].accounting.as_mut().unwrap().swap(0, 1);
            policy.rules[1].change = Change::NewPrivate;
            assert!(
                compare(&right, &left, &policy)
                    .unwrap_err()
                    .to_string()
                    .contains("duplicate field")
            );
        }
    }

    #[test]
    fn accounting_rules_refuse_checkpoint_shapes_normal_deserialization_rejects() {
        let fixture = Fixture::new();
        for (index, (pointer, value)) in [
            ("/selected", json!([65536])),
            ("/header", json!([256])),
            ("/index/0/first", json!(2147483648u64)),
            ("/index/0/length", json!(-1)),
            ("/map/stride", json!(-1)),
            ("/map/selected", json!("not a table selection")),
            (
                "/map/branches",
                json!({"1":{"checkpoints":0,"since_checkpoint":-1}}),
            ),
            ("/map/gaps", json!({"1":{"range":0}})),
            ("/map/ranges", json!({"1":[1,"not an hour"]})),
            ("/content", json!("false")),
            ("/display_failures", json!(-1)),
            (
                "/refused",
                json!({"1":{"synthetic":{"hour":"not an hour","error":"refused"}}}),
            ),
            ("/independent/synthetic/refused", json!({"not a branch":{}})),
            (
                "/production",
                json!({"1":{"schema":40,"firstHour":1,"lastHour":"not an hour","complete":true}}),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let corrupt = |packet| {
                edit_checkpoint(packet, |document| {
                    *document.pointer_mut(pointer).unwrap() = value.clone();
                })
            };
            let left = fixture.store_with_checkpoint(
                &format!("invalid-left-{index}"),
                vec![("map.resume", vec![vec![1]])],
                1,
                true,
                &corrupt,
            );
            let right = fixture.store_with_checkpoint(
                &format!("invalid-right-{index}"),
                vec![],
                2,
                true,
                &corrupt,
            );
            assert!(
                compare(&left, &right, &remove_private_policy(&fixture)).is_err(),
                "{pointer}"
            );
        }
        for (index, field) in ["selected", "map", "display_failures", "bytes"]
            .into_iter()
            .enumerate()
        {
            let corrupt = |packet| {
                edit_checkpoint(packet, |document| {
                    document.as_object_mut().unwrap().remove(field).unwrap();
                })
            };
            let left = fixture.store_with_checkpoint(
                &format!("missing-left-{index}"),
                vec![("map.resume", vec![vec![1]])],
                1,
                true,
                &corrupt,
            );
            let right = fixture.store_with_checkpoint(
                &format!("missing-right-{index}"),
                vec![],
                2,
                true,
                &corrupt,
            );
            assert!(
                compare(&left, &right, &remove_private_policy(&fixture))
                    .unwrap_err()
                    .to_string()
                    .contains("missing field"),
                "{field}"
            );
        }
    }

    #[test]
    fn checkpoint_validation_accepts_nested_restoration_documents_and_defaults() {
        let fixture = Fixture::new();
        let update = |packet| {
            edit_checkpoint(packet, |document| {
                document["cursor"]["previous"] = document["index"][0].clone();
                document["map"]["branches"] = json!({"1":{
                    "checkpoints":1,"since_checkpoint":0,"last_hour":2,"last_block":0
                }});
                document["map"]["gaps"] = json!({"1":{"range":0,"intervals":0}});
                document["map"]["ranges"] = json!({"1":[1,2]});
                document["refused"] = json!({"1":{"synthetic":{
                    "hour":2,"error":"Synthetic recorded refusal","from":null
                }}});
                document["production"] = json!({"1":{
                    "schema":40,"firstHour":1,"lastHour":2,"complete":true
                }});
            })
        };
        let left = fixture.store_with_checkpoint("nested-left", vec![], 1, true, &update);
        let right = fixture.store_with_checkpoint("nested-right", vec![], 2, true, &update);
        assert_eq!(compare(&left, &right, &strict()).unwrap()["passed"], true);
        let defaults = |packet| {
            edit_checkpoint(packet, |document| {
                document["independent"]["synthetic"]
                    .as_object_mut()
                    .unwrap()
                    .remove("refused");
            })
        };
        let left = fixture.store_with_checkpoint("default-left", vec![], 1, true, &defaults);
        let right = fixture.store_with_checkpoint("default-right", vec![], 2, true, &defaults);
        assert_eq!(compare(&left, &right, &strict()).unwrap()["passed"], true);
    }

    #[test]
    fn validating_checkpoint_types_does_not_drop_other_payload_differences() {
        let fixture = Fixture::new();
        let left = fixture.store_with_checkpoint(
            "left",
            vec![("map.resume", vec![vec![1]])],
            1,
            true,
            |packet| edit_checkpoint(packet, |document| document["future"] = json!(1)),
        );
        let right = fixture.store_with_checkpoint("right", vec![], 2, true, |packet| {
            edit_checkpoint(packet, |document| document["future"] = json!(2))
        });
        let report = compare(&left, &right, &remove_private_policy(&fixture)).unwrap();
        assert_eq!(report["passed"], false);
        assert!(report["differences"].as_array().unwrap().iter().any(
            |difference| difference["section"] == "preparation-checkpoint-v1"
                && difference["rule"].is_null()
        ));
        assert_eq!(report["unused_rules"], json!([0]));
    }

    #[test]
    fn private_contract_defaults_unknown_and_query_visible_sections_to_protected() {
        assert!(private_section("map.resume"));
        assert!(private_section("map-pages.state.1.20"));
        for section in [
            "private-old",
            "private-product",
            "preparation-family-identities-1",
            "preparation-side-work-v1",
            "preparation-checkpoint-v1",
            "country-identities-v1-1",
            "unit-leaders-v1-1",
            "production-lines-scan-v5-1-414141",
            "map-pages.1.20",
            "map-pages.progress.1",
            "map-pages.state.1.20.coverage",
            "map-pages.state.01.20",
            "map-pages.state.1.0",
        ] {
            assert!(!private_section(section), "{section}");
        }
    }

    #[test]
    fn exact_source_bound_rules_cannot_excuse_public_section_removal_or_addition() {
        let fixture = Fixture::new();
        for (index, section) in [
            "preparation-family-identities-1",
            "country-identities-v1-1",
            "unit-leaders-v1-1",
            "production-lines-scan-v5-1-414141",
            "map-pages.1.20",
            "map-pages.progress.1",
            "unknown-catalogue",
        ]
        .into_iter()
        .enumerate()
        {
            let present = fixture.store(
                &format!("present-{index}"),
                vec![(section, vec![vec![1]])],
                1,
                true,
            );
            let absent = fixture.store(&format!("absent-{index}"), vec![], 2, true);
            for (left, right, change, accounting) in [
                (&present, &absent, Change::RemovedPrivate, [1, 0]),
                (&absent, &present, Change::NewPrivate, [0, 1]),
            ] {
                let policy = Policy {
                    schema: 1,
                    source: Some(fixture.source.clone()),
                    rules: vec![
                        Rule {
                            section: section.into(),
                            change,
                            ranges: 1,
                            accounting: None,
                            reason: "Exact structural exception".into(),
                        },
                        Rule {
                            section: "preparation-checkpoint-v1".into(),
                            change: Change::CheckpointBytes,
                            ranges: 1,
                            accounting: Some(accounting.map(|bytes| bytes + base_bytes())),
                            reason: "Exact byte accounting".into(),
                        },
                    ],
                };
                assert!(
                    compare(left, right, &policy)
                        .unwrap_err()
                        .to_string()
                        .contains("not a known private continuation")
                );
            }
        }
    }

    #[test]
    fn changed_public_coverage_fails_even_when_named_by_a_structural_rule() {
        let fixture = Fixture::new();
        let section = "preparation-family-identities-1";
        let left = fixture.store(
            "left",
            vec![(section, vec![br#"{"through":1}"#.to_vec()])],
            1,
            true,
        );
        let right = fixture.store(
            "right",
            vec![(section, vec![br#"{"through":2}"#.to_vec()])],
            2,
            true,
        );
        let report = compare(&left, &right, &strict()).unwrap();
        assert_eq!(report["passed"], false);
        assert!(
            report["differences"]
                .as_array()
                .unwrap()
                .iter()
                .any(|difference| difference["section"] == section && difference["rule"].is_null())
        );
        for change in [Change::NewPrivate, Change::RemovedPrivate] {
            let policy = Policy {
                schema: 1,
                source: Some(fixture.source.clone()),
                rules: vec![Rule {
                    section: section.into(),
                    change,
                    ranges: 1,
                    accounting: None,
                    reason: "Exact coverage name".into(),
                }],
            };
            assert!(compare(&left, &right, &policy).is_err());
        }
    }

    #[test]
    fn valid_private_exceptions_still_compare_every_public_payload() {
        let fixture = Fixture::new();
        let left = fixture.store(
            "left",
            vec![
                ("map-pages.state.1.20", vec![vec![9]]),
                ("preparation-family-identities-1", vec![vec![1]]),
            ],
            1,
            true,
        );
        let right = fixture.store(
            "right",
            vec![("preparation-family-identities-1", vec![vec![2]])],
            2,
            true,
        );
        let policy = Policy {
            schema: 1,
            source: Some(fixture.source.clone()),
            rules: vec![
                Rule {
                    section: "map-pages.state.1.20".into(),
                    change: Change::RemovedPrivate,
                    ranges: 1,
                    accounting: None,
                    reason: "Replaced endpoint continuation".into(),
                },
                Rule {
                    section: "preparation-checkpoint-v1".into(),
                    change: Change::CheckpointBytes,
                    ranges: 1,
                    accounting: Some([base_bytes() + 2, base_bytes() + 1]),
                    reason: "Removed continuation byte".into(),
                },
            ],
        };
        let report = compare(&left, &right, &policy).unwrap();
        assert_eq!(report["passed"], false);
        assert!(report["unused_rules"].as_array().unwrap().is_empty());
        assert!(report["differences"].as_array().unwrap().iter().any(
            |difference| difference["section"] == "preparation-family-identities-1"
                && difference["kind"] == "payload"
                && difference["rule"].is_null()
        ));
    }

    #[test]
    fn refuses_incomplete_differently_bound_and_truncated_inputs() {
        let fixture = Fixture::new();
        let left = fixture.store("left", vec![("payload", vec![vec![8; 70000]])], 1, true);
        let incomplete = fixture.store("incomplete", vec![], 2, false);
        assert!(compare(&left, &incomplete, &strict()).is_err());
        let other = Fixture::new();
        let other_input = other.store("right", vec![], 2, true);
        assert!(compare(&left, &other_input, &strict()).is_err());
        let right = fixture.store("right", vec![("payload", vec![vec![8; 70000]])], 2, true);
        let file = OpenOptions::new()
            .write(true)
            .open(right.directory.join("pack_0000000000000002.bin"))
            .unwrap();
        file.set_len(1).unwrap();
        assert!(compare(&left, &right, &strict()).is_err());
    }
}
